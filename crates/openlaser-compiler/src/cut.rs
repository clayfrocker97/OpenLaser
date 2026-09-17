// SPDX-License-Identifier: GPL-3.0-or-later

//! Planning one contour's cutting motion: geometry preprocessing, joints,
//! cooling, velocity planning, cadence sampling and the process fields of
//! every step (MainApp `0x0045_B520`, `0x0045_D253`).
//!
//! The result is a list of pieces, each a sampled path with one process
//! sample per step and the segment process it belongs to. Pieces split
//! where the process changes, at re-pierces and around cooling stops. They
//! are quantised into records later, once the residuals carried into the
//! contour are known.

use crate::geometry::{Node, Path};
use crate::motion::{Plan, Sample};
use crate::planner;
use crate::process::{self, JointRange};
use crate::settings::{self, LeadRole, Overrides, SegmentProcess, Settings};
use crate::{Error, Point, Result, float};
use openlaser_core::toolpath::PreparedSegment;
use std::collections::BTreeMap;
use std::sync::Arc;

/// One segment of a prepared contour, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    /// A straight line.
    Line {
        /// Start point.
        start: Point,
        /// End point.
        end: Point,
    },
    /// An arc about `center` from `start_angle` through `sweep`; a negative
    /// sweep runs clockwise.
    Arc {
        /// Centre.
        center: Point,
        /// Radius.
        radius: f64,
        /// Start angle in radians.
        start_angle: f64,
        /// Signed sweep in radians.
        sweep: f64,
    },
}

impl Segment {
    /// The segment's length.
    #[must_use]
    pub fn length(&self) -> f64 {
        match *self {
            Self::Line { start, end } => (end[0] - start[0]).hypot(end[1] - start[1]),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The point at parameter `t` in `0..=1`.
    #[must_use]
    pub fn point(&self, t: f64) -> Point {
        match *self {
            Self::Line { start, end } => {
                [start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t]
            }
            Self::Arc { center, radius, start_angle, sweep } => {
                let angle = start_angle + sweep * t;
                [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
            }
        }
    }

    /// The segment moved by `delta`.
    #[must_use]
    pub fn translated(&self, delta: Point) -> Self {
        let moved = |p: Point| [p[0] + delta[0], p[1] + delta[1]];
        match *self {
            Self::Line { start, end } => Self::Line { start: moved(start), end: moved(end) },
            Self::Arc { center, radius, start_angle, sweep } => {
                Self::Arc { center: moved(center), radius, start_angle, sweep }
            }
        }
    }

    /// The part of the segment from parameter `t` to its end.
    #[must_use]
    pub fn after(&self, t: f64) -> Self {
        match *self {
            Self::Line { end, .. } => Self::Line { start: self.point(t), end },
            Self::Arc { center, radius, start_angle, sweep } => Self::Arc {
                center,
                radius,
                start_angle: start_angle + sweep * t,
                sweep: sweep * (1. - t),
            },
        }
    }

    fn node(&self) -> Node {
        match *self {
            Self::Line { start, end } => Node::Line { start, end },
            Self::Arc { center, radius, start_angle, sweep } => {
                Node::Arc { center, radius, start_angle, end_angle: start_angle + sweep }
            }
        }
    }
}

impl From<openlaser_core::geometry::Curve> for Segment {
    fn from(curve: openlaser_core::geometry::Curve) -> Self {
        use openlaser_core::geometry::Curve;
        match curve {
            Curve::Line { start, end } => Self::Line { start: start.into(), end: end.into() },
            Curve::Arc { center, radius, start_angle, sweep } => {
                Self::Arc { center: center.into(), radius, start_angle, sweep }
            }
        }
    }
}

/// A prepared contour: its segments, how each is cut, and the overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    /// The segments in cutting order, leads included.
    pub segments: Vec<PreparedSegment<Segment>>,
    /// Per-contour overrides of the recipe.
    pub overrides: Overrides,
}

impl Contour {
    /// The length of the segments.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.segments.iter().map(|s| s.curve.length()).sum()
    }

    /// The contour moved by `delta`: its geometry, with its processes and
    /// overrides as they are.
    #[must_use]
    pub fn translated(&self, delta: Point) -> Self {
        Self {
            segments: self
                .segments
                .iter()
                .map(|s| s.map_curve(|curve| curve.translated(delta)))
                .collect(),
            overrides: self.overrides.clone(),
        }
    }

    /// The part of the contour after `distance` along it, with the slow
    /// start of the `resolved` overrides shortened by the same distance
    /// and the slow end kept within what is left; nothing when the
    /// distance reaches its end. A cut resumed inside a segment does not
    /// repeat its cooling stop.
    #[must_use]
    pub fn after(&self, distance: f64, resolved: &Overrides) -> Option<Self> {
        let mut remaining = distance;
        let mut segments = Vec::new();
        for segment in &self.segments {
            let length = segment.curve.length();
            if !segments.is_empty() {
                segments.push(segment.clone());
                continue;
            }
            if remaining >= length - 1e-9 {
                remaining = (remaining - length).max(0.);
                continue;
            }
            let mut segment = segment.clone();
            if remaining > 0. {
                let fraction = remaining / length;
                segment.curve = segment.curve.after(fraction);
                segment.process.cool_ms = 0;
                if let Some(source) = &mut segment.source {
                    source.start += (source.end - source.start) * fraction;
                }
                remaining = 0.;
            }
            segments.push(segment);
        }
        if segments.is_empty() {
            return None;
        }
        let left = self.length() - distance;
        let overrides = Overrides {
            slow_start: Some(resolved.slow_start.flatten().and_then(|(length, speed)| {
                (length > distance).then_some((length - distance, speed))
            })),
            slow_end: resolved.slow_end.map(|(length, speed)| (length.min(left), speed)),
            on_delay_ms: resolved.on_delay_ms,
            off_delay_ms: resolved.off_delay_ms,
        };
        Some(Self { segments, overrides })
    }
}

/// A stop within a contour, between two pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StopEvent {
    /// Distance along the sampled path.
    pub distance: f64,
    /// A cooling dwell in milliseconds.
    pub cool_ms: u32,
    /// Whether the cut re-pierces here.
    pub repierce: bool,
}

/// A run of samples cut with one process.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The sampled path; its first point repeats the previous piece's end.
    pub plan: Plan,
    /// One process sample per step.
    pub samples: Arc<[Sample]>,
    /// The process the piece is cut with.
    pub process: SegmentProcess,
}

impl Piece {
    /// The piece's duration.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.plan.interval * float(self.samples.len())
    }
}

/// A planned contour.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    /// The pieces in order.
    pub pieces: Vec<Piece>,
    /// One entry per piece boundary, `pieces.len() + 1` in all: the event
    /// that ends the piece before, if any.
    pub events: Vec<Option<StopEvent>>,
    /// The overrides as resolved for this contour.
    pub overrides: Overrides,
    /// The preprocessed path length.
    pub length: f64,
    /// Requested cooling points excluded by the native endpoint tolerance.
    pub omitted_cooling: usize,
}

/// A joint over a range of the sampled path.
#[derive(Clone)]
struct Joint {
    start: f64,
    end: f64,
    process: SegmentProcess,
    repierce: bool,
}

/// The sampled contour with the process of every step.
struct Sampled {
    interval: f64,
    points: Vec<Point>,
    distances: Vec<f64>,
    tags: Vec<SegmentProcess>,
    samples: Vec<Sample>,
}

/// The vendor's writer flushes a piece before it reaches this many samples.
const PIECE_SAMPLES: usize = 999_998;

/// Plans `contour` under `settings`, with at most `max_samples` samples.
pub fn plan(contour: &Contour, settings: &Settings, max_samples: usize) -> Result<Motion> {
    plan_geometry(contour, settings, max_samples, false)
}

pub(crate) fn plan_prepared(
    contour: &Contour,
    settings: &Settings,
    max_samples: usize,
) -> Result<Motion> {
    plan_geometry(contour, settings, max_samples, true)
}

fn plan_geometry(
    contour: &Contour,
    settings: &Settings,
    max_samples: usize,
    prepared: bool,
) -> Result<Motion> {
    let segments = &contour.segments;
    if segments.is_empty() {
        return Err(Error::Invalid("a contour needs geometry"));
    }
    let speed = settings.planner_speed();
    let path = if prepared {
        Path::prepared(nodes(segments), settings.corner_accuracy)?
    } else {
        Path::new(nodes(segments), true, settings.corner_accuracy)?
    };
    let length = path.length();
    let boundaries = boundaries(segments);
    let (joints, cooling) = joints_and_cooling(segments, &boundaries, settings.dry_run);
    let cooling_fractions: Vec<f64> = cooling.iter().map(|p| p.0).collect();
    let circle = path.small_circle_radius.zip(settings.small_circle_limit_ratio);
    let (sources, cooling_distances) = process::refine(
        &path.triples(),
        speed,
        &joint_ranges(&joints),
        &cooling_fractions,
        circle,
    )?;
    let overrides = contour.overrides.resolved(settings, length);
    let planned = planner::plan(&sources, planner_settings(settings, speed, &overrides))?;
    let distances = planner::sample(&planned.profiles, planned.settings.interval_ms, max_samples)?;
    let points = path.map_distances(&distances)?;
    if points.is_empty() {
        return Err(Error::Invalid("the contour produced no samples"));
    }
    let mut sampled = Sampled {
        interval: planned.settings.interval_ms * 0.001,
        points,
        distances,
        tags: Vec::new(),
        samples: Vec::new(),
    };
    tag_samples(&mut sampled, length, &boundaries, segments, settings)?;
    let repierce = joint_samples(&joints, &mut sampled, length, settings.dry_run)?;
    let dwell = dwells(&cooling_distances, &sampled.distances, &cooling, length);
    let cycle_us = settings.interpolation_cycle_us;
    let extra = dwell.values().try_fold(0usize, |count, ms| {
        usize::try_from(dwell_samples(*ms, cycle_us))
            .ok()
            .and_then(|n| count.checked_add(n))
            .ok_or(Error::Budget("cooling samples overflow"))
    })?;
    if sampled.points.len().saturating_add(extra).saturating_add(repierce.len()) > max_samples {
        return Err(Error::Budget("the contour's samples, cooling included, exceed the budget"));
    }
    let (pieces, events) = write_pieces(&sampled, &repierce, &dwell, cycle_us);
    let omitted_cooling = cooling_fractions
        .iter()
        .filter(|&&fraction| !process::cooling_interior(fraction * length, length))
        .count();
    Ok(Motion { pieces, events, overrides, length, omitted_cooling })
}

/// The preprocessing input: one node per segment, plus a marker after a
/// pure straight entry lead so the prepass keeps the join and selects the
/// lead accuracy there.
fn nodes(segments: &[PreparedSegment<Segment>]) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(segments.len() + 1);
    for (i, segment) in segments.iter().enumerate() {
        nodes.push(segment.curve.node());
        if i == 0
            && matches!(segment.curve, Segment::Line { .. })
            && segment.process.lead == Some(LeadRole::Entry)
            && segments.get(i + 1).is_some_and(|s| s.process.lead != Some(LeadRole::Entry))
        {
            nodes.push(Node::Marker);
        }
    }
    nodes
}

/// Where each segment starts and ends, as fractions of the original length.
fn boundaries(segments: &[PreparedSegment<Segment>]) -> Vec<f64> {
    let original_length: f64 = segments.iter().map(|s| s.curve.length()).sum();
    let mut boundaries = vec![0.];
    let mut distance = 0.;
    for segment in segments {
        distance += segment.curve.length();
        boundaries.push(distance / original_length);
    }
    boundaries
}

/// The joints, adjacent equal joints merged, and the cooling stops with
/// their delays; a dry run has neither.
fn joints_and_cooling(
    segments: &[PreparedSegment<Segment>],
    boundaries: &[f64],
    dry_run: bool,
) -> (Vec<Joint>, Vec<(f64, u32)>) {
    let mut joints: Vec<Joint> = Vec::new();
    let mut cooling = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let p = &segment.process;
        if p.cool_ms > 0 && !dry_run {
            cooling.push((boundaries[i], p.cool_ms));
        }
        if !p.joint {
            continue;
        }
        let mut process = p.clone();
        process.cool_ms = 0;
        process.repierce = false;
        let repierce =
            segments.get(i + 1).is_some_and(|next| !next.process.joint && next.process.repierce)
                && !dry_run;
        match joints.last_mut() {
            Some(last) if last.end == boundaries[i] && last.process == process => {
                last.end = boundaries[i + 1];
                last.repierce = repierce;
            }
            _ => joints.push(Joint {
                start: boundaries[i],
                end: boundaries[i + 1],
                process,
                repierce,
            }),
        }
    }
    (joints, cooling)
}

/// The source refinement joints ask for: none unless some joint slows down
/// or re-pierces, as the vendor's direct producer is used otherwise.
fn joint_ranges(joints: &[Joint]) -> Vec<JointRange> {
    if !joints.iter().any(|j| j.process.speed.is_some() || j.repierce) {
        return Vec::new();
    }
    joints
        .iter()
        .map(|j| JointRange {
            start: j.start,
            end: j.end,
            speed: j.process.speed,
            repierce: j.repierce,
        })
        .collect()
}

/// The planner's settings for a cut at `speed` with the resolved slow regions.
fn planner_settings(settings: &Settings, speed: f64, overrides: &Overrides) -> planner::Settings {
    let (slow_length, slow_speed) = overrides.slow_start.flatten().unwrap_or((0., speed));
    let (end_length, end_speed) = overrides.slow_end.unwrap_or((0., speed));
    planner::Settings {
        acceleration: settings.acceleration,
        acceleration_time: settings.acceleration_time,
        spline_accuracy: settings.spline_accuracy,
        speed,
        corner_speed_floor: 0.,
        interval_ms: settings.cadence_ms(),
        slow_start_length: slow_length,
        slow_start_speed: slow_speed.min(speed),
        slow_end_length: end_length,
        slow_end_speed: end_speed.min(speed),
    }
}

/// The process of every step: the segment it falls in, the power and
/// frequency curves at its speed, and the edge overrides.
fn tag_samples(
    sampled: &mut Sampled,
    length: f64,
    boundaries: &[f64],
    segments: &[PreparedSegment<Segment>],
    settings: &Settings,
) -> Result<()> {
    let process::Curves { power, frequency } = process::Curves::new(settings)?;
    let distances = &sampled.distances;
    let interval = sampled.interval;
    for (i, &distance) in distances.iter().enumerate() {
        let v = if i > 0 {
            (distance - distances[i - 1]) / interval
        } else {
            distances.get(1).map_or(0., |d| (d - distance) / interval)
        };
        let analog =
            u8::try_from(power.evaluate(v)?).map_err(|_| Error::Invalid("power exceeds a byte"))?;
        let (base, f) = settings.edge_pwm(distance, length, analog, frequency.evaluate(v)?);
        let source = boundaries
            .partition_point(|fraction| *fraction * length <= distance)
            .saturating_sub(1)
            .min(segments.len() - 1);
        let mut tag = segments[source].process.clone();
        tag.joint = false;
        tag.repierce = false;
        tag.cool_ms = 0;
        if segments[source].process.joint {
            tag.power = None;
            tag.speed = None;
        }
        let applied = settings::sample_power(&tag, base)?;
        // The writer stores the analog level before the edge overrides.
        let analog = settings::sample_power(&tag, analog)?;
        sampled.tags.push(tag);
        sampled.samples.push(Sample {
            analog,
            power: if settings.dry_run { 0 } else { applied },
            frequency: f,
            tail: if settings.dry_run { 0 } else { 0xffff },
        });
    }
    Ok(())
}

/// Retags the steps inside each joint with the joint's process and scaled
/// power. Returns the re-pierce sample to insert after each re-piercing
/// joint, keyed by the step it follows.
fn joint_samples(
    joints: &[Joint],
    sampled: &mut Sampled,
    length: f64,
    dry_run: bool,
) -> Result<BTreeMap<usize, Sample>> {
    let mut repierce = BTreeMap::new();
    let distances = &sampled.distances;
    for joint in joints {
        let Some(begin) = process::sample_index(distances, joint.start * length) else { continue };
        let end =
            process::sample_index(distances, joint.end * length).unwrap_or(distances.len() - 1);
        for i in begin..=end {
            sampled.tags[i] = joint.process.clone();
            sampled.samples[i].power = if dry_run {
                0
            } else {
                settings::sample_power(&joint.process, sampled.samples[i].power)?
            };
        }
        if joint.repierce && end + 1 < distances.len() {
            repierce.insert(end, sampled.samples[begin]);
        }
    }
    Ok(repierce)
}

/// The cooling dwell at each stopping step. The writer inserts them after
/// the inclusive position lookup; equal steps keep the longest delay.
fn dwells(
    cooling_distances: &[f64],
    distances: &[f64],
    cooling: &[(f64, u32)],
    length: f64,
) -> BTreeMap<usize, u32> {
    let mut dwell = BTreeMap::<usize, u32>::new();
    for &d in cooling_distances {
        if let Some(at) = process::sample_index(distances, d) {
            let delay = cooling
                .iter()
                .filter(|(fraction, _)| (*fraction * length - d).abs() < 1e-8)
                .map(|p| p.1)
                .max()
                .unwrap_or(0);
            dwell.entry(at).and_modify(|v| *v = (*v).max(delay)).or_insert(delay);
        }
    }
    dwell
}

/// How many stationary records a dwell of `ms` takes: floor of the delay in
/// interpolation cycles.
fn dwell_samples(ms: u32, cycle_us: u32) -> u64 {
    u64::from(ms) * 1000 / u64::from(cycle_us)
}

/// Splits the sampled contour into pieces where the process changes, after
/// re-pierces and around cooling dwells, as the vendor's writer flushes.
fn write_pieces(
    sampled: &Sampled,
    repierce: &BTreeMap<usize, Sample>,
    dwell: &BTreeMap<usize, u32>,
    cycle_us: u32,
) -> (Vec<Piece>, Vec<Option<StopEvent>>) {
    let Sampled { interval, points, distances, tags, samples } = sampled;
    let mut out = Writer::new(*interval, points[0]);
    let mut current = tags[0].clone();
    for i in 0..points.len() {
        if tags[i] != current || out.samples.len() >= PIECE_SAMPLES {
            out.flush(&current, None);
            current = tags[i].clone();
        }
        out.push(points[i], distances[i], samples[i]);
        if let Some(sample) = repierce.get(&i) {
            out.push(points[i], distances[i], *sample);
            out.flush(
                &current,
                Some(StopEvent { distance: distances[i], cool_ms: 0, repierce: true }),
            );
        }
        if let Some(&ms) = dwell.get(&i) {
            out.flush(&current, None);
            let cool = SegmentProcess { cool_ms: ms, power: Some(0.), ..Default::default() };
            for _ in 0..dwell_samples(ms, cycle_us) {
                if out.samples.len() >= PIECE_SAMPLES {
                    out.flush(&cool, None);
                }
                out.points.push(points[i]);
                out.distances.push(0.);
                out.samples.push(Sample { analog: 0, power: 0, frequency: 0, tail: 0 });
            }
            out.flush(&cool, None);
        }
    }
    out.flush(&current, None);
    (out.pieces, out.events)
}

/// Collects samples into pieces.
struct Writer {
    interval: f64,
    pieces: Vec<Piece>,
    events: Vec<Option<StopEvent>>,
    points: Vec<Point>,
    distances: Vec<f64>,
    samples: Vec<Sample>,
    base: f64,
}

impl Writer {
    fn new(interval: f64, start: Point) -> Self {
        Self {
            interval,
            pieces: Vec::new(),
            events: vec![None],
            points: vec![start],
            distances: vec![0.],
            samples: Vec::new(),
            base: 0.,
        }
    }

    /// Closes the current piece under `process`. The next piece starts at
    /// this one's last point, so its first step is the zero increment the
    /// vendor emits for every sample.
    fn flush(&mut self, process: &SegmentProcess, event: Option<StopEvent>) {
        if self.samples.is_empty() {
            return;
        }
        let (Some(&last), Some(&end)) = (self.points.last(), self.distances.last()) else { return };
        let plan = Plan {
            interval: self.interval,
            distances: std::mem::take(&mut self.distances).into(),
            points: std::mem::take(&mut self.points).into(),
        };
        self.pieces.push(Piece {
            plan,
            samples: std::mem::take(&mut self.samples).into(),
            process: process.clone(),
        });
        self.events.push(event);
        self.points.push(last);
        self.distances.push(0.);
        self.base += end;
    }

    fn push(&mut self, point: Point, distance: f64, sample: Sample) {
        self.points.push(point);
        self.distances.push(distance - self.base);
        self.samples.push(sample);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn tagged(
        processes: Vec<SegmentProcess>,
        curves: Vec<Segment>,
    ) -> Vec<openlaser_core::toolpath::PreparedSegment<Segment>> {
        assert_eq!(curves.len(), processes.len());
        curves
            .into_iter()
            .zip(processes)
            .map(|(c, p)| openlaser_core::toolpath::PreparedSegment::new(c, p))
            .collect()
    }

    use crate::settings::{HeadTravel, Timeouts};
    use openlaser_core::LaserMode;

    pub(crate) fn settings() -> Settings {
        Settings {
            mode: LaserMode::Fiber,
            layer: 1,
            dry_run: false,
            counts_per_mm: [1000.; 2],
            acceleration: 6000.,
            acceleration_time: 0.25,
            interpolation_cycle_us: 250,
            speed: 250.,
            maximum_cut_speed: 3000.,
            native_speed_cap: None,
            corner_accuracy: 0.01,
            spline_accuracy: 0.,
            small_circle_limit_ratio: None,
            slow_start: None,
            edge_start: None,
            edge_end: None,
            power: 45,
            frequency: 2000,
            peak_current: 100.,
            gas: 1,
            pressure: 2.5,
            cut_height: 1.,
            retract_height: 15.,
            z_up_speed: 50.,
            z_follow_speed: 50.,
            fixed_height: false,
            absolute_cut_height: None,
            no_follow: false,
            pierce: Vec::new(),
            residue: None,
            on_delay_ms: 9,
            off_before_ms: 0,
            off_after_ms: 0,
            power_curve: None,
            frequency_curve: None,
            vibration_word: 0,
            hardware: None,
            travel_speed: 500.,
            travel_acceleration: 5000.,
            travel_acceleration_time: 0.1,
            machining_kind: 0,
            smooth_pierce: false,
            head_travel: HeadTravel {
                frog_jump: false,
                minimum_height: None,
                protect_insertions: false,
                keep_height_below: None,
                jump_pierce_height: None,
                short_transfer: None,
            },
            timeouts: Timeouts::default(),
            pre_pierce: None,
            with_film: false,
            film_batch: 0,
            keep_gas_after: false,
            short_gas_keep: false,
            contour_shift: [0.; 2],
        }
    }

    fn line(start: Point, end: Point) -> Segment {
        Segment::Line { start, end }
    }

    /// A single line yields one piece whose first step is a zero increment,
    /// one sample per step, cutting power throughout and the tail set.
    #[test]
    fn a_line_is_one_piece_with_the_vendor_zero_first_step() {
        let contour = Contour {
            segments: tagged(vec![SegmentProcess::default()], vec![line([0., 0.], [20., 0.])]),
            overrides: Overrides::default(),
        };
        let motion = plan(&contour, &settings(), 100_000).unwrap();
        assert_eq!(motion.pieces.len(), 1);
        let piece = &motion.pieces[0];
        assert_eq!(piece.plan.points[0], piece.plan.points[1]);
        assert_eq!(piece.plan.points.len(), piece.samples.len() + 1);
        assert!(
            piece.samples.iter().all(|s| s.power == 45 && s.frequency == 2000 && s.tail == 0xffff)
        );
        assert_eq!(motion.events.len(), 2);
        assert!((motion.length - 20.).abs() < 1e-9);
    }

    /// A joint segment with a power override splits the contour into pieces
    /// around it, with the joint's scaled power inside.
    #[test]
    fn a_joint_splits_pieces_and_scales_power() {
        let joint = SegmentProcess { joint: true, power: Some(20.), ..Default::default() };
        let contour = Contour {
            segments: tagged(
                vec![SegmentProcess::default(), joint.clone(), SegmentProcess::default()],
                vec![
                    line([0., 0.], [10., 0.]),
                    line([10., 0.], [12., 0.]),
                    line([12., 0.], [22., 0.]),
                ],
            ),
            overrides: Overrides::default(),
        };
        let motion = plan(&contour, &settings(), 100_000).unwrap();
        assert_eq!(motion.pieces.len(), 3);
        assert!(motion.pieces[1].process.joint);
        assert!(motion.pieces[1].samples.iter().all(|s| s.power == 9));
        assert!(motion.pieces[0].samples.iter().all(|s| s.power == 45));
    }

    /// A cooling stop inserts a stationary piece of zero-power samples whose
    /// count is the delay in interpolation cycles.
    #[test]
    fn a_cooling_stop_inserts_stationary_samples() {
        let cool = SegmentProcess { cool_ms: 2, ..Default::default() };
        let contour = Contour {
            segments: tagged(
                vec![SegmentProcess::default(), cool],
                vec![line([0., 0.], [10., 0.]), line([10., 0.], [20., 0.])],
            ),
            overrides: Overrides::default(),
        };
        let motion = plan(&contour, &settings(), 100_000).unwrap();
        let dwell = motion.pieces.iter().find(|p| p.process.cool_ms == 2).unwrap();
        assert_eq!(dwell.samples.len(), 8);
        assert!(dwell.samples.iter().all(|s| s.power == 0 && s.frequency == 0));
        assert!(dwell.plan.points.iter().all(|p| *p == dwell.plan.points[0]));
    }

    /// A dry run zeroes every power and tail and drops the process events.
    #[test]
    fn a_dry_run_moves_without_firing() {
        let mut dry = settings();
        dry.dry_run = true;
        let contour = Contour {
            segments: tagged(
                vec![SegmentProcess { cool_ms: 5, ..Default::default() }],
                vec![line([0., 0.], [20., 0.])],
            ),
            overrides: Overrides::default(),
        };
        let motion = plan(&contour, &dry, 100_000).unwrap();
        assert_eq!(motion.pieces.len(), 1);
        assert!(motion.pieces[0].samples.iter().all(|s| s.power == 0 && s.tail == 0));
    }

    /// A contour with a cooled arc between two lines.
    fn cooled_contour() -> Contour {
        let segments = vec![
            line([0., 0.], [10., 0.]),
            Segment::Arc {
                center: [10., 5.],
                radius: 5.,
                start_angle: -std::f64::consts::FRAC_PI_2,
                sweep: std::f64::consts::PI,
            },
            line([10., 10.], [0., 10.]),
        ];
        let cooled = SegmentProcess { cool_ms: 300, ..SegmentProcess::default() };
        Contour {
            segments: tagged(
                vec![SegmentProcess::default(), cooled, SegmentProcess::default()],
                segments,
            ),
            overrides: Overrides::default(),
        }
    }

    /// The part after a distance inside the arc keeps the rest of the arc
    /// and the last line, drops the cooling stop of the cut segment, and
    /// shortens the slow start by the distance; the part after the end is
    /// nothing, and a translation moves every segment.
    #[test]
    fn a_contour_continues_after_a_distance() {
        let contour = cooled_contour();
        let resolved = Overrides {
            slow_start: Some(Some((12., 5.))),
            slow_end: Some((30., 5.)),
            on_delay_ms: None,
            off_delay_ms: None,
        };
        let quarter = 10. + std::f64::consts::PI * 5. / 2.;
        let rest = contour.after(quarter, &resolved).unwrap();
        assert_eq!(rest.segments.len(), 2);
        assert!(
            matches!(rest.segments[0].curve, Segment::Arc { sweep, .. } if (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9)
        );
        let start = rest.segments[0].curve.point(0.);
        assert!((start[0] - 15.).abs() < 1e-9 && (start[1] - 5.).abs() < 1e-9);
        assert_eq!(rest.segments[0].process.cool_ms, 0);
        assert_eq!(rest.overrides.slow_start, Some(None), "the slow start was shorter");
        assert_eq!(rest.overrides.slow_end, Some((contour.length() - quarter, 5.)));
        let at_start = contour.after(0., &resolved).unwrap();
        assert_eq!(at_start.segments.len(), 3);
        assert_eq!(at_start.segments[1].process.cool_ms, 300);
        assert_eq!(at_start.overrides.slow_start, Some(Some((12., 5.))));
        assert!(contour.after(contour.length(), &resolved).is_none());
        let moved = contour.translated([10., -3.]);
        assert_eq!(moved.segments[0].curve.point(0.), [10., -3.]);
        assert!(matches!(moved.segments[1].curve, Segment::Arc { center: [20., 2.], .. }));
        assert_eq!(
            moved.segments.iter().map(|s| &s.process).collect::<Vec<_>>(),
            contour.segments.iter().map(|s| &s.process).collect::<Vec<_>>()
        );
    }
}
