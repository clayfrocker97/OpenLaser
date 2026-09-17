// SPDX-License-Identifier: GPL-3.0-or-later

//! Host continuation policy: retain the executed geometry, and retime its
//! remainder without invoking preparation or the smoothing path. Each
//! retained process piece starts and ends at rest. Rescheduled chords must
//! stay within the original corner accuracy of the execution polyline.
//! A cooling dwell at the
//! stopped position is restarted in full because feedback does not identify
//! how much of it elapsed. Later complete passes keep their original plan.

use crate::cut::Piece;
use crate::motion::{Plan, Sample};
use crate::pass::{self, PassKind};
use crate::planner;
use crate::process::Curves;
use crate::program::{Compiled, Job};
use crate::{Error, MAX_SAMPLES, Point, Result, settings};
use std::sync::Arc;

pub(crate) fn remaining(
    job: &Job,
    at: usize,
    fraction: f64,
    pierce: bool,
    position: Option<Point>,
) -> Result<Job> {
    let plan: Vec<_> = job.passes.iter().map(|c| c.pass).collect();
    let remaining = pass::remaining(&plan, at, fraction)?;
    let start = at + usize::from(fraction == 1.);
    let count = |passes: &[Compiled]| {
        passes
            .iter()
            .flat_map(|pass| &pass.pieces)
            .try_fold(0usize, |n, piece| n.checked_add(piece.samples.len()))
            .filter(|&count| count <= MAX_SAMPLES)
            .ok_or(Error::Budget("continuation samples"))
    };
    count(&job.passes[start..])?;
    let later = count(job.passes.get(start + 1..).unwrap_or_default())?;
    let mut passes = Vec::with_capacity(remaining.len());
    for (i, (_, fraction)) in remaining.into_iter().enumerate() {
        let compiled = &job.passes[start + i];
        let position = if i == 0 { position } else { None };
        passes.push(if fraction == 0. && position.is_none() {
            compiled.clone()
        } else {
            trim(compiled, fraction, pierce, position, MAX_SAMPLES - later)?
        });
    }
    if passes.is_empty() {
        return Err(Error::Invalid("the program had finished; nothing remains to resume"));
    }
    Ok(Job { settings: job.settings.clone(), passes })
}

fn length(piece: &Piece) -> f64 {
    piece.plan.points.windows(2).map(|p| distance(p[0], p[1])).sum()
}

fn distance(a: Point, b: Point) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn trim(
    compiled: &Compiled,
    fraction: f64,
    pierce: bool,
    position: Option<Point>,
    mut budget: usize,
) -> Result<Compiled> {
    if compiled.events.len() != compiled.pieces.len() + 1
        || compiled.pieces.iter().any(|p| p.plan.points.len() != p.samples.len() + 1)
    {
        return Err(Error::Invalid("the held path and its process samples do not agree"));
    }
    let total: f64 = compiled.pieces.iter().map(length).sum();
    let consumed = total * fraction;
    let mut left = consumed;
    let mut first = 0;
    while let Some(piece) = compiled.pieces.get(first) {
        let length = length(piece);
        if left < length - 1e-10 || (length == 0. && left <= 1e-10) {
            break;
        }
        left = (left - length).max(0.);
        first += 1;
    }
    let mut rest = Compiled {
        pass: compiled.pass,
        contour: compiled.contour.clone(),
        settings: compiled.settings.clone(),
        overrides: compiled.overrides.clone(),
        pieces: compiled.pieces[first..].to_vec(),
        events: compiled.events[first..].to_vec(),
        start: compiled.start,
        end: compiled.end,
        initial_pierce: compiled.initial_pierce,
        consumed: compiled.consumed + consumed,
        omitted_cooling: compiled.omitted_cooling,
    };
    if rest.pass.kind == PassKind::Cut {
        rest.initial_pierce = pierce;
    }
    let initial =
        rest.pieces.first_mut().ok_or(Error::Invalid("the pass has no remaining motion"))?;
    if left > 0. {
        trim_start(initial, left)?;
        rest.events[0] = None;
    }
    if let Some(position) = position {
        anchor_start(&mut rest, position)?;
    }
    rest.start = rest.pieces[0].plan.points[0];
    let mut offset = rest.consumed;
    for piece in &mut rest.pieces {
        let length = length(piece);
        retime(piece, &compiled.settings, offset, compiled.consumed + total, budget)?;
        budget =
            budget.checked_sub(piece.samples.len()).ok_or(Error::Budget("continuation samples"))?;
        offset += length;
    }
    rest.end = rest
        .pieces
        .last()
        .and_then(|piece| piece.plan.points.last())
        .copied()
        .ok_or(Error::Invalid("the pass has no remaining endpoint"))?;
    Ok(rest)
}

/// Feedback is quantized, and a decelerated stop can lie slightly off the
/// planned chord. Start at that exact saved coordinate rather than move to
/// its projection with the laser off. Only the first retained chord changes;
/// all subsequent endpoints and process pieces belong to the original path.
fn anchor_start(rest: &mut Compiled, position: Point) -> Result<()> {
    let start = rest.pieces[0].plan.points[0];
    if position.iter().any(|v| !v.is_finite()) || distance(position, start) >= 0.2 {
        return Err(Error::Invalid("the saved position is not within 0.2 mm of the restart path"));
    }
    // Include any leading rest samples or cooling dwell at the same point.
    for piece in &mut rest.pieces {
        let points = Arc::make_mut(&mut piece.plan.points);
        let mut changed = 0;
        for point in points.iter_mut().take_while(|p| distance(**p, start) <= 1e-12) {
            *point = position;
            changed += 1;
        }
        if changed != points.len() {
            break;
        }
    }
    Ok(())
}

fn trim_start(initial: &mut Piece, mut left: f64) -> Result<()> {
    let mut step = 0;
    while step < initial.samples.len() {
        let length = distance(initial.plan.points[step], initial.plan.points[step + 1]);
        if length > left {
            break;
        }
        left -= length;
        step += 1;
    }
    let a = *initial.plan.points.get(step).ok_or(Error::Invalid("checkpoint exceeds the path"))?;
    let b =
        *initial.plan.points.get(step + 1).ok_or(Error::Invalid("checkpoint exceeds the path"))?;
    let t = left / distance(a, b);
    let position = std::array::from_fn(|axis| a[axis] + t * (b[axis] - a[axis]));
    initial.plan.points = initial.plan.points[step..].into();
    Arc::make_mut(&mut initial.plan.points)[0] = position;
    initial.samples = initial.samples[step..].into();
    Ok(())
}

/// Reuses the existing velocity planner over polyline distances and the
/// original local speed ceilings; no geometry enters the smoothing code.
fn retime(
    piece: &mut Piece,
    s: &settings::Settings,
    offset: f64,
    total: f64,
    budget: usize,
) -> Result<()> {
    if piece.process.cool_ms > 0 || length(piece) <= 1e-12 {
        if piece.samples.len() > budget {
            return Err(Error::Budget("continuation dwell samples"));
        }
        return Ok(());
    }
    let path = RetimingPath::new(&piece.plan, s.planner_speed());
    let scheduled = path.schedule(s, budget)?;
    let curves = Curves::new(s)?;
    let mut points = Vec::with_capacity(scheduled.len() + 1);
    let mut samples = Vec::with_capacity(scheduled.len());
    points.push(piece.plan.points[0]);
    let mut previous = 0.;
    let mut vertex = 0;
    for (i, &d) in scheduled.iter().enumerate() {
        let next = path.point(d);
        let prior = points.last().copied().ok_or(Error::Invalid("continuation start"))?;
        path.check_chord(
            &mut vertex,
            [previous, d],
            [prior, next],
            s.corner_accuracy.min(0.2) + 1e-9,
        )?;
        points.push(next);
        let step = if i == 0 { scheduled.get(1).map_or(0., |next| next - d) } else { d - previous };
        let speed = step.max(0.) / piece.plan.interval;
        samples.push(retimed_sample(s, &piece.process, &curves, speed, offset + d, total)?);
        previous = d;
    }
    let mut measured = Vec::with_capacity(scheduled.len() + 1);
    measured.push(0.);
    measured.extend(scheduled);
    piece.plan =
        Plan { interval: piece.plan.interval, distances: measured.into(), points: points.into() };
    piece.samples = samples.into();
    Ok(())
}

struct RetimingPath<'a> {
    points: &'a [Point],
    distances: Vec<f64>,
    sources: Vec<(f64, f64)>,
}

impl<'a> RetimingPath<'a> {
    fn new(plan: &'a Plan, speed: f64) -> Self {
        let mut distances = Vec::with_capacity(plan.points.len());
        let mut sources = Vec::with_capacity(plan.points.len());
        distances.push(0.);
        sources.push((0., speed));
        for pair in plan.points.windows(2) {
            let step = distance(pair[0], pair[1]);
            let end = distances.last().copied().unwrap_or(0.) + step;
            distances.push(end);
            if step > 1e-12 {
                sources.push((end, step / plan.interval));
            }
        }
        Self { points: &plan.points, distances, sources }
    }

    fn schedule(&self, s: &settings::Settings, budget: usize) -> Result<Vec<f64>> {
        let planned = planner::plan_speeds(
            &self.sources,
            planner::Settings {
                acceleration: s.acceleration,
                acceleration_time: s.acceleration_time,
                spline_accuracy: s.spline_accuracy,
                speed: s.planner_speed(),
                corner_speed_floor: 0.,
                interval_ms: s.cadence_ms(),
                slow_start_length: 0.,
                slow_start_speed: 0.,
                slow_end_length: 0.,
                slow_end_speed: 0.,
            },
        )?;
        planner::sample(&planned.profiles, planned.settings.interval_ms, budget)
    }

    fn point(&self, d: f64) -> Point {
        let at = self
            .distances
            .partition_point(|v| *v <= d)
            .saturating_sub(1)
            .min(self.distances.len() - 2);
        let span = self.distances[at + 1] - self.distances[at];
        let t = if span > 0. { ((d - self.distances[at]) / span).clamp(0., 1.) } else { 0. };
        let [a, b] = [self.points[at], self.points[at + 1]];
        std::array::from_fn(|axis| a[axis] + t * (b[axis] - a[axis]))
    }

    fn check_chord(
        &self,
        vertex: &mut usize,
        range: [f64; 2],
        chord: [Point; 2],
        tolerance: f64,
    ) -> Result<()> {
        while *vertex < self.distances.len() && self.distances[*vertex] <= range[1] {
            if self.distances[*vertex] >= range[0]
                && chord_error(self.points[*vertex], chord[0], chord[1]) > tolerance
            {
                return Err(Error::Invalid("continuation exceeds the original path accuracy"));
            }
            *vertex += 1;
        }
        Ok(())
    }
}

fn retimed_sample(
    s: &settings::Settings,
    process: &settings::SegmentProcess,
    curves: &Curves,
    speed: f64,
    distance: f64,
    total: f64,
) -> Result<Sample> {
    let analog = u8::try_from(curves.power.evaluate(speed)?)
        .map_err(|_| Error::Invalid("continuation power"))?;
    let (base, frequency) = s.edge_pwm(distance, total, analog, curves.frequency.evaluate(speed)?);
    Ok(Sample {
        analog: settings::sample_power(process, analog)?,
        power: if s.dry_run { 0 } else { settings::sample_power(process, base)? },
        frequency,
        tail: if s.dry_run { 0 } else { 0xffff },
    })
}

fn chord_error(p: Point, a: Point, b: Point) -> f64 {
    let squared = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
    let t = if squared > 0. {
        (((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / squared).clamp(0., 1.)
    } else {
        0.
    };
    distance(p, std::array::from_fn(|axis| a[axis] + t * (b[axis] - a[axis])))
}
