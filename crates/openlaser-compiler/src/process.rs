// SPDX-License-Identifier: GPL-3.0-or-later

//! Joint, cooling and small-circle refinement of a contour's source records
//! (CADModule `0x1010_3780`, `0x1010_4640`), and the speed-dependent power
//! and frequency curves (MainApp `0x0045_2733`, CADModule `0x1010_49C0`).

use crate::planner::Source;
use crate::{Error, Result, to_u16};

/// A range of the contour, as fractions of its length, cut as a joint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRange {
    /// Start fraction.
    pub start: f64,
    /// End fraction.
    pub end: f64,
    /// A slower speed inside the joint, if requested.
    pub speed: Option<f64>,
    /// Whether the cut re-pierces after the joint.
    pub repierce: bool,
}

/// Records closer than this along the path merge into one.
const MERGE_DISTANCE: f64 = 0.05;

fn plain([radius, distance, scale]: [f64; 3], cap: f64) -> Source {
    Source { distance, radius, span_cap: cap, point_cap: 30_000., scale }
}

fn check(triples: &[[f64; 3]], cap: f64, joints: &[JointRange], cooling: &[f64]) -> Result<f64> {
    if triples.len() < 2
        || triples.len() > 1_000_000
        || !cap.is_finite()
        || cap <= 0.
        || triples.iter().flatten().any(|v| !v.is_finite())
        || triples.windows(2).any(|w| w[0][1] > w[1][1])
    {
        return Err(Error::Invalid("source triples must be finite, ordered and capped"));
    }
    let total = triples[triples.len() - 1][1];
    check_positions(total, joints, cooling)?;
    Ok(total)
}

fn check_positions(total: f64, joints: &[JointRange], cooling: &[f64]) -> Result<()> {
    let joint_ok = |j: &JointRange| {
        j.start.is_finite()
            && j.end.is_finite()
            && j.start >= 0.
            && j.end <= 1.
            && j.start < j.end
            && j.speed.is_none_or(|s| s.is_finite() && s > 0.)
    };
    if total <= 0.
        || joints.len() > 10_000
        || cooling.len() > 10_000
        || !joints.iter().all(joint_ok)
        || joints.windows(2).any(|w| w[0].end > w[1].start)
        || cooling.iter().any(|c| !c.is_finite() || !(0. ..=1.).contains(c))
        || cooling.windows(2).any(|w| w[0] > w[1])
    {
        return Err(Error::Invalid("joint and cooling positions must be ordered fractions"));
    }
    Ok(())
}

/// Both speed-dependent process curves, built from the same recipe in cutting and continuation.
pub(crate) struct Curves {
    pub power: Curve,
    pub frequency: Curve,
}

impl Curves {
    pub fn new(settings: &crate::settings::Settings) -> Result<Self> {
        Ok(Self {
            power: Curve::new(
                settings.speed,
                u16::from(settings.power),
                settings.power_curve.as_deref(),
            )?,
            frequency: Curve::new(
                settings.speed,
                settings.frequency,
                settings.frequency_curve.as_deref(),
            )?,
        })
    }
}

/// Refines `triples` with joints, then the small-circle limit, then cooling
/// stops, in the vendor's order. Fractions refer to the whole prepared
/// contour. `circle` is the eligible arc radius and the limit ratio. Returns
/// the source records and the distances of the cooling stops that applied.
pub fn refine(
    triples: &[[f64; 3]],
    cap: f64,
    joints: &[JointRange],
    cooling: &[f64],
    circle: Option<(f64, f64)>,
) -> Result<(Vec<Source>, Vec<f64>)> {
    if circle
        .is_some_and(|(r, ratio)| !r.is_finite() || r <= 0. || !ratio.is_finite() || ratio < 0.)
    {
        return Err(Error::Invalid("the small-circle limit needs a positive radius and ratio"));
    }
    let total = check(triples, cap, joints, cooling)?;
    // Without joints the direct producer keeps close extrema; the joint
    // producer's merge applies only when it ran.
    let mut rows = if joints.is_empty() {
        triples.iter().map(|&t| plain(t, cap)).collect()
    } else {
        joint_rows(triples, cap, joints, total)
    };
    let at = rows.len() - 1;
    rows[at].distance = total;
    if let Some((radius, ratio)) = circle {
        limit_small_circle(&mut rows, radius, ratio);
    }
    let events = cooling_stops(&mut rows, cooling, total, cap)?;
    Ok((rows, events))
}

/// The joint producer: triples inside a joint become records capped at the
/// joint speed, triples outside merge with their close neighbours.
fn joint_rows(triples: &[[f64; 3]], cap: f64, joints: &[JointRange], total: f64) -> Vec<Source> {
    let mut rows = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < triples.len() {
        if let Some(joint) = joints.get(j).filter(|joint| joint.start * total < triples[i][1]) {
            i = joint_records(&mut rows, triples, joint, i, total, cap);
            j += 1;
        } else {
            plain_record(&mut rows, triples[i], cap);
            i += 1;
        }
    }
    rows
}

/// One triple outside a joint: merged into the last record when close,
/// keeping the smaller radius, otherwise appended.
fn plain_record(rows: &mut Vec<Source>, triple: [f64; 3], cap: f64) {
    let [radius, distance, scale] = triple;
    match rows.last_mut() {
        Some(last) if last.distance + MERGE_DISTANCE >= distance => {
            if radius < last.radius {
                last.radius = radius;
                last.distance = distance;
                last.scale = scale;
            }
        }
        _ => rows.push(plain(triple, cap)),
    }
}

/// The records of one joint starting at triple `i`: its start, the triples
/// inside it and its end, all capped at the joint speed, and a stop at the
/// end when it re-pierces. Returns the index of the first triple after it.
fn joint_records(
    rows: &mut Vec<Source>,
    triples: &[[f64; 3]],
    joint: &JointRange,
    i: usize,
    total: f64,
    cap: f64,
) -> usize {
    let (begin, end) = (joint.start * total, joint.end * total);
    let speed = joint.speed.map_or(cap, |s| s.max(1.).min(cap));
    let mut end_index = i;
    while end_index < triples.len() && triples[end_index][1] <= end {
        end_index += 1;
    }
    match rows.last_mut() {
        Some(last) if begin - last.distance <= MERGE_DISTANCE => {
            last.distance = begin;
            last.point_cap = speed;
        }
        last => {
            let radius = last.map_or(triples[i][0], |r| r.radius.max(triples[i][0]));
            rows.push(Source {
                distance: begin,
                radius,
                span_cap: cap,
                point_cap: speed,
                scale: 1.,
            });
        }
    }
    for &[radius, distance, _] in &triples[i..end_index] {
        let at = rows.len() - 1;
        if distance - rows[at].distance <= MERGE_DISTANCE {
            if radius < rows[at].radius {
                rows[at].radius = radius;
                rows[at].distance = distance;
            }
        } else {
            rows.push(Source { distance, radius, span_cap: speed, point_cap: speed, scale: 1. });
        }
    }
    let at = rows.len() - 1;
    if end - rows[at].distance <= MERGE_DISTANCE {
        rows[at].distance = end;
        rows[at].point_cap = speed;
    } else {
        let radius = rows[at].radius.max(triples[end_index.min(triples.len() - 1)][0]);
        rows.push(Source { distance: end, radius, span_cap: speed, point_cap: speed, scale: 1. });
    }
    if joint.repierce {
        let at = rows.len() - 1;
        rows[at].point_cap = 0.;
        rows[at].span_cap = speed;
    }
    end_index
}

/// Radii matching the small circle's, within five percent of it or half a
/// millimetre, are scaled by the limit ratio.
fn limit_small_circle(rows: &mut [Source], radius: f64, ratio: f64) {
    let threshold = (radius * 0.05).min(0.5);
    for row in rows {
        if (row.radius - radius).abs() <= threshold {
            row.radius *= ratio;
        }
    }
}

/// Each cooling stop, walked from the end, zeroes the point cap of the
/// record nearest to it or inserts a stopping record. Stops within 0.2 mm
/// of either end are ignored. Returns the distances of the stops applied.
fn cooling_stops(
    rows: &mut Vec<Source>,
    cooling: &[f64],
    total: f64,
    cap: f64,
) -> Result<Vec<f64>> {
    let mut events = Vec::new();
    let mut at = rows.len().saturating_sub(2);
    for &fraction in cooling.iter().rev() {
        let d = fraction * total;
        if !cooling_interior(d, total) {
            continue;
        }
        while rows[at].distance > d {
            if at == 0 {
                return Err(Error::Invalid("a cooling point precedes the first source record"));
            }
            at -= 1;
        }
        let threshold = ((rows[at + 1].distance - rows[at].distance) * 0.25).min(0.1);
        if rows[at + 1].distance - d < threshold {
            rows[at + 1].point_cap = 0.;
        } else if d - rows[at].distance >= threshold {
            let mut row = rows[at];
            row.distance = d;
            row.radius = row.radius.max(rows[at + 1].radius);
            row.span_cap = cap;
            row.point_cap = 0.;
            rows.insert(at + 1, row);
        } else {
            rows[at].point_cap = 0.;
        }
        events.push(d);
    }
    events.reverse();
    Ok(events)
}

/// The native cooling producer excludes the open 0.2 mm endpoint regions.
pub(crate) fn cooling_interior(distance: f64, total: f64) -> bool {
    distance >= 0.2 && distance <= total - 0.2
}

/// The first sample at or beyond `distance`, as MainApp `0x0046_2840`
/// attaches events to samples. `None` outside the sampled range.
#[must_use]
pub fn sample_index(distances: &[f64], distance: f64) -> Option<usize> {
    if !distance.is_finite()
        || distances.first().is_none_or(|d| distance < *d)
        || distances.last().is_none_or(|d| distance > *d)
    {
        return None;
    }
    let at = distances.partition_point(|d| *d < distance);
    (at < distances.len()).then_some(at)
}

/// A linear power or frequency curve over speed: nodes are integer
/// percentages of the nominal speed and of the base output. Near nominal
/// speed the curve is bypassed; outputs round half up.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    nominal_speed: f64,
    base: u16,
    segments: Vec<[f64; 4]>,
}

impl Curve {
    /// A curve over `nodes` of `(speed percent, output percent)` spanning
    /// 0 to 100, or a constant when `nodes` is `None`.
    pub fn new(nominal_speed: f64, base: u16, nodes: Option<&[(f64, f64)]>) -> Result<Self> {
        if !nominal_speed.is_finite() || nominal_speed <= 0. {
            return Err(Error::Invalid("a process curve needs a positive nominal speed"));
        }
        let mut segments = Vec::new();
        if let Some(nodes) = nodes {
            let percent = |v: f64| v.is_finite() && (0. ..=100.).contains(&v) && v.fract() == 0.;
            if nodes.len() < 2
                || nodes.len() > 1000
                || nodes[0].0 != 0.
                || nodes[nodes.len() - 1].0 != 100.
                || nodes.iter().any(|(x, y)| !percent(*x) || !percent(*y))
                || nodes.windows(2).any(|w| w[0].0 >= w[1].0)
            {
                return Err(Error::Invalid(
                    "a process curve needs ordered integer percentages from 0 to 100",
                ));
            }
            for w in nodes.windows(2) {
                let lo = w[0].0 * 0.01 * nominal_speed;
                let hi = w[1].0 * 0.01 * nominal_speed;
                let value = w[0].1 * 0.01 * f64::from(base);
                let slope = (w[1].1 - w[0].1) * 0.01 * f64::from(base) / (hi - lo);
                segments.push([lo, hi, value, slope]);
            }
        }
        Ok(Self { nominal_speed, base, segments })
    }

    /// The output at `speed`.
    pub fn evaluate(&self, speed: f64) -> Result<u16> {
        if !speed.is_finite() || speed < 0. {
            return Err(Error::Invalid("a process sample speed must be finite and nonnegative"));
        }
        let threshold = self.nominal_speed - (self.nominal_speed * 0.02).min(0.1);
        if self.segments.is_empty() || speed > threshold {
            return Ok(self.base);
        }
        let segment = self
            .segments
            .iter()
            .find(|s| speed <= s[1])
            .unwrap_or(&self.segments[self.segments.len() - 1]);
        let output = (segment[2] + 0.5 + (speed - segment[0]) * segment[3]).trunc();
        if !(0. ..=65535.).contains(&output) {
            return Err(Error::Invalid("a process curve output exceeds its field"));
        }
        to_u16(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triples() -> Vec<[f64; 3]> {
        vec![
            [10000., 0., 1.],
            [10000., 5., 1.],
            [10000., 10., 1.],
            [10000., 15., 1.],
            [10000., 20., 1.],
        ]
    }

    /// Without joints or cooling the records are the triples with the span
    /// cap and unconstrained point cap.
    #[test]
    fn plain_records_mirror_the_triples() {
        let (rows, events) = refine(&triples(), 100., &[], &[], None).unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.span_cap == 100. && r.point_cap == 30_000. && r.scale == 1.));
        assert!(events.is_empty());
    }

    /// A slow joint caps the span and points inside it, and a re-pierce
    /// zeroes the point cap at its end.
    #[test]
    fn a_joint_caps_its_range_and_a_repierce_stops() {
        let joint = JointRange { start: 0.25, end: 0.5, speed: Some(20.), repierce: true };
        let (rows, _) = refine(&triples(), 100., &[joint], &[], None).unwrap();
        let inside: Vec<_> = rows.iter().filter(|r| r.distance > 5. && r.distance <= 10.).collect();
        assert!(inside.iter().all(|r| r.span_cap == 20.));
        assert_eq!(rows.iter().find(|r| r.distance == 10.).unwrap().point_cap, 0.);
    }

    /// A cooling stop away from any record inserts a stopping record at its
    /// distance and reports it.
    #[test]
    fn a_cooling_stop_inserts_a_record() {
        let (rows, events) = refine(&triples(), 100., &[], &[0.4], None).unwrap();
        assert_eq!(events, vec![8.]);
        let stop = rows.iter().find(|r| r.distance == 8.).unwrap();
        assert_eq!(stop.point_cap, 0.);
    }

    /// The small-circle ratio scales radii matching the circle's radius.
    #[test]
    fn small_circle_ratio_scales_matching_radii() {
        let rows = [[2., 0., 1.], [2., 5., 1.], [10000., 10., 1.]];
        let (out, _) = refine(&rows, 100., &[], &[], Some((2., 0.5))).unwrap();
        assert_eq!(out[0].radius, 1.);
        assert_eq!(out[2].radius, 10000.);
    }

    /// The sample index is the first distance at or beyond the request and
    /// none outside the range.
    #[test]
    fn sample_index_is_inclusive_upward() {
        let distances = [0., 1., 2., 3.];
        assert_eq!(sample_index(&distances, 1.5), Some(2));
        assert_eq!(sample_index(&distances, 2.), Some(2));
        assert_eq!(sample_index(&distances, 3.5), None);
    }

    /// Percentages interpolate with half-up rounding and the curve is
    /// bypassed within two percent of nominal speed.
    #[test]
    fn curve_rounds_and_bypasses_near_nominal() {
        let curve = Curve::new(50., 35, Some(&[(0., 10.), (100., 80.)])).unwrap();
        assert_eq!(curve.evaluate(0.).unwrap(), 4);
        assert_eq!(curve.evaluate(25.).unwrap(), 16);
        assert_eq!(curve.evaluate(49.9).unwrap(), 28);
        assert_eq!(curve.evaluate(49.90001).unwrap(), 35);
        assert_eq!(curve.evaluate(70.).unwrap(), 35);
        let slow = Curve::new(1., 100, Some(&[(0., 0.), (100., 50.)])).unwrap();
        assert_eq!(slow.evaluate(0.98).unwrap(), 49);
        assert_eq!(slow.evaluate(0.981).unwrap(), 100);
    }

    /// A frequency curve interpolates its 16-bit base, a disabled curve is
    /// constant, and malformed nodes are refused.
    #[test]
    fn frequency_and_disabled_curves_preserve_the_base() {
        let curve = Curve::new(50., 12345, Some(&[(0., 10.), (50., 40.), (100., 100.)])).unwrap();
        assert_eq!(curve.evaluate(25.).unwrap(), 4938);
        assert_eq!(Curve::new(50., 5000, None).unwrap().evaluate(0.).unwrap(), 5000);
        assert!(curve.evaluate(f64::NAN).is_err());
        assert!(Curve::new(50., 35, Some(&[(0., 0.), (0., 100.)])).is_err());
    }
}
