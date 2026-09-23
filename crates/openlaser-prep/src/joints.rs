// SPDX-License-Identifier: GPL-3.0-or-later

//! Micro-joints and cooling stops: the contour is cut into segments at
//! every joint end and every stop, and each segment is tagged with how it
//! is cut.

use crate::{EPS, Result, feature, float};
use openlaser_core::features::{
    Cooling, CoolingPlacement, JointBehaviour, JointPlacement, Joints, Spot,
};
use openlaser_core::geometry::{Contour, Curve};
use openlaser_core::toolpath::{PreparedSegment, SegmentProcess};
use std::f64::consts::{PI, TAU};

/// The most joints or stops one contour may carry.
const MAX_POSITIONS: usize = 10_000;

/// Contours smaller than this, in millimetres, never get automatic joints,
/// whatever the recipe's minimum size says.
const MIN_JOINTED_SIZE_MM: f64 = 2.;
/// Nor do contours smaller than this many joint widths.
const MIN_JOINTED_WIDTHS: f64 = 5.;
/// On an open contour jointed across X or Y, a joint also goes at the start
/// when the first one lies further along than this, in millimetres.
const OPEN_START_JOINT_MM: f64 = 5.;
/// Joints together may cover at most this share of the contour's span; a
/// wider joint is narrowed to fit.
const MAX_JOINTED_SHARE: f64 = 0.5;

/// What the split needs to know about the contour it is given.
pub(crate) struct Frame {
    /// Whether the contour is closed.
    pub closed: bool,
    /// Whether the contour is a hole.
    pub hole: bool,
    /// Whether it was reversed from how it was drawn.
    pub reversed: bool,
    /// Where it now starts, as a fraction of the drawn length.
    pub start: f64,
    /// The bridge result whose manual locations have already been mapped.
    pub contour: usize,
}

impl Frame {
    /// The fractions of the spots that lie on this contour.
    fn own(&self, spots: &[Spot]) -> Vec<f64> {
        spots.iter().filter(|s| self.contour == s.contour).map(|s| s.fraction).collect()
    }

    /// A fraction along the contour as drawn, as a fraction along the
    /// contour as it now runs.
    fn remap(&self, fraction: f64) -> f64 {
        let along = if self.reversed { 1. - fraction } else { fraction } - self.start;
        if self.closed { along.rem_euclid(1.) } else { along.clamp(0., 1.) }
    }
}

pub(crate) fn check(joints: &Joints) -> Result<()> {
    let positive = |value: f64| value.is_finite() && value > 0.;
    if !positive(joints.width.0) {
        return Err(feature("joints", "the width must be positive"));
    }
    if !(joints.minimum_size.0.is_finite() && joints.minimum_size.0 >= 0.) {
        return Err(feature("joints", "the minimum size must not be negative"));
    }
    check_placement(joints)?;
    if let JointBehaviour::Power(percent) = joints.behaviour
        && !(0. ..=100.).contains(&percent.0)
    {
        return Err(feature("joints", "the power must be 0 to 100 percent"));
    }
    if joints.slow_speed.is_some_and(|speed| !positive(speed.0)) {
        return Err(feature("joints", "the slow speed must be positive"));
    }
    Ok(())
}

fn check_placement(joints: &Joints) -> Result<()> {
    let positive = |value: f64| value.is_finite() && value > 0.;
    match &joints.placement {
        JointPlacement::Count(count)
        | JointPlacement::AcrossX(count)
        | JointPlacement::AcrossY(count) => {
            if *count == 0 || usize::try_from(*count).is_ok_and(|n| n > MAX_POSITIONS) {
                return Err(feature("joints", format!("the count must be 1 to {MAX_POSITIONS}")));
            }
        }
        JointPlacement::Spacing(spacing) => {
            if !positive(spacing.0) || spacing.0 <= joints.width.0 {
                return Err(feature("joints", "the spacing must be more than the width"));
            }
        }
        JointPlacement::Manual(spots) => spots_ok("joints", spots)?,
    }
    Ok(())
}

pub(crate) fn check_cooling(cooling: &Cooling) -> Result<()> {
    if cooling.dwell.0 == 0 {
        return Err(feature("cooling", "the dwell must be positive"));
    }
    match &cooling.placement {
        CoolingPlacement::Automatic { corners_below: Some(angle), .. }
            if !(0. ..=180.).contains(&angle.0) =>
        {
            Err(feature("cooling", "the corner angle must be 0 to 180 degrees"))
        }
        CoolingPlacement::Manual(spots) => spots_ok("cooling", spots),
        CoolingPlacement::Automatic { .. } => Ok(()),
    }
}

/// Places picked on the drawing: none yet is allowed, so a feature can be
/// switched to picking before the first tap.
pub(crate) fn spots_ok(name: &'static str, spots: &[Spot]) -> Result<()> {
    if spots.len() > MAX_POSITIONS {
        return Err(feature(name, format!("at most {MAX_POSITIONS} places")));
    }
    if spots.iter().any(|s| !(0. ..=1.).contains(&s.fraction)) {
        return Err(feature(name, "a place lies along its contour, 0 to 1"));
    }
    Ok(())
}

/// The contour cut into segments at every joint end and cooling stop, with
/// each segment's process.
pub(crate) fn split(
    contour: &Contour,
    joints: Option<&Joints>,
    cooling: Option<&Cooling>,
    frame: &Frame,
) -> Result<Vec<PreparedSegment>> {
    let length = contour.length();
    let ranges =
        joints.map(|j| joint_ranges(contour, j, frame, length)).transpose()?.unwrap_or_default();
    let stops = cooling.map_or_else(Vec::new, |c| cooling_stops(contour, c, frame, length));
    let (power, speed, repierce) = joints.map_or((None, None, false), |j| {
        let power = match j.behaviour {
            JointBehaviour::LaserOff => 0.,
            JointBehaviour::Power(percent) => percent.0,
        };
        (Some(power), j.slow_speed.map(|s| s.0), j.repierce)
    });
    if ranges.len() + stops.len() > MAX_POSITIONS {
        return Err(crate::Error::Budget("generated joint and cooling positions"));
    }
    let mut segments = Vec::new();
    let mut offset = 0.;
    let mut after_joint = false;
    for curve in &contour.curves {
        let end = offset + curve.length();
        let inside = |x: &f64| *x > offset + EPS && *x < end - EPS;
        let mut cuts = vec![offset, end];
        cuts.extend(ranges.iter().flat_map(|&(a, b)| [a, b]).filter(inside));
        cuts.extend(stops.iter().map(|s| s.0).filter(inside));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
        for pair in cuts.windows(2) {
            let middle = f64::midpoint(pair[0], pair[1]);
            let joint = ranges.iter().any(|&(a, b)| a <= middle && middle <= b);
            let geometry = curve
                .slice((pair[0] - offset) / curve.length(), (pair[1] - offset) / curve.length());
            segments.push(PreparedSegment::new(
                geometry,
                SegmentProcess {
                    lead: None,
                    joint,
                    repierce: repierce && after_joint && !joint,
                    power: if joint { power } else { None },
                    speed: if joint { speed } else { None },
                    cool_ms: stops
                        .iter()
                        .filter(|(at, _)| (at - pair[0]).abs() < EPS)
                        .map(|(_, dwell)| *dwell)
                        .max()
                        .unwrap_or(0),
                },
            ));
            after_joint = joint;
        }
        offset = end;
    }
    Ok(segments)
}

/// The joints as distance ranges along the contour as it runs.
fn joint_ranges(
    contour: &Contour,
    joints: &Joints,
    frame: &Frame,
    length: f64,
) -> Result<Vec<(f64, f64)>> {
    if joints.outer_only && frame.hole {
        return Ok(Vec::new());
    }
    let Some(bounds) = contour.bounds() else { return Ok(Vec::new()) };
    let manual = matches!(joints.placement, JointPlacement::Manual(_));
    if !manual
        && bounds.extent()
            < joints
                .minimum_size
                .0
                .max(MIN_JOINTED_SIZE_MM)
                .max(joints.width.0 * MIN_JOINTED_WIDTHS)
    {
        return Ok(Vec::new());
    }
    let mut width = joints.width.0;
    // An open contour may carry a joint at its start as well.
    let from_start = joints.open_start && !frame.closed;
    let (mut centers, span) = joint_centers(contour, joints, frame, length, from_start, bounds)?;
    if from_start
        && matches!(joints.placement, JointPlacement::AcrossX(_) | JointPlacement::AcrossY(_))
        && centers.first().is_none_or(|first| *first > OPEN_START_JOINT_MM)
    {
        centers.push(width);
        centers.sort_by(f64::total_cmp);
    }
    if !centers.is_empty() {
        width = width.min(span * MAX_JOINTED_SHARE / float(centers.len()));
    }
    Ok(centers
        .iter()
        .map(|&center| ((center - width / 2.).max(0.), (center + width / 2.).min(length)))
        .filter(|(a, b)| b - a > EPS)
        .collect())
}

fn joint_centers(
    contour: &Contour,
    joints: &Joints,
    frame: &Frame,
    length: f64,
    from_start: bool,
    bounds: openlaser_core::geometry::Bounds,
) -> Result<(Vec<f64>, f64)> {
    let centers = match &joints.placement {
        JointPlacement::Count(count) => (spread(length, *count, from_start), length),
        JointPlacement::Spacing(spacing) => {
            (spread(length, spaced_count(length, spacing.0)?, from_start), length)
        }
        JointPlacement::AcrossX(count) => {
            (grid_centers(contour, 0, *count, joints.width.0)?, bounds.width())
        }
        JointPlacement::AcrossY(count) => {
            (grid_centers(contour, 1, *count, joints.width.0)?, bounds.height())
        }
        JointPlacement::Manual(spots) => {
            (frame.own(spots).iter().map(|f| frame.remap(*f) * length).collect(), length)
        }
    };
    Ok(centers)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to the position budget"
)]
fn spaced_count(length: f64, spacing: f64) -> Result<u32> {
    let count = ((length / spacing).round() - 1.).max(0.);
    if count > float(MAX_POSITIONS) {
        return Err(crate::Error::Budget("generated joint positions"));
    }
    Ok(count as u32)
}

/// `count` centres spread evenly: none at the start or end, or the first
/// at the start when `from_start`.
fn spread(length: f64, count: u32, from_start: bool) -> Vec<f64> {
    let count = usize::try_from(count).unwrap_or(MAX_POSITIONS);
    if from_start {
        (0..count).map(|i| length * float(i) / float(count)).collect()
    } else {
        (1..=count).map(|i| length * float(i) / float(count + 1)).collect()
    }
}

/// Where grid lines across `axis`, `count` of them between the contour's
/// extremes, cross the contour steeply enough for a joint to hold.
fn grid_centers(contour: &Contour, axis: usize, count: u32, width: f64) -> Result<Vec<f64>> {
    let Some(bounds) = contour.bounds() else { return Ok(Vec::new()) };
    let (low, high) = ([bounds.min.x, bounds.min.y][axis], [bounds.max.x, bounds.max.y][axis]);
    let count = usize::try_from(count).unwrap_or(MAX_POSITIONS);
    if contour.curves.len().saturating_mul(count) > 1_000_000 {
        return Err(crate::Error::Budget("joint grid intersections"));
    }
    let mut centers = Vec::new();
    let mut offset = 0.;
    for curve in &contour.curves {
        let extent = curve.bounds();
        let (lo, hi) = ([extent.min.x, extent.min.y][axis], [extent.max.x, extent.max.y][axis]);
        for index in 0..count {
            let position = low + (high - low) * float(index + 1) / float(count + 1);
            if position < lo + width || position > hi - width {
                continue;
            }
            for t in crossings(curve, axis, position) {
                let tangent = curve.tangent(t);
                if [tangent.y, tangent.x][axis].abs() < 0.5 {
                    if centers.len() >= MAX_POSITIONS {
                        return Err(crate::Error::Budget("generated joint positions"));
                    }
                    centers.push(offset + t * curve.length());
                }
            }
        }
        offset += curve.length();
    }
    centers.sort_by(f64::total_cmp);
    centers.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    Ok(centers)
}

/// The parameters where `curve` crosses the line `axis = position`.
fn crossings(curve: &Curve, axis: usize, position: f64) -> Vec<f64> {
    let mut parameters = Vec::new();
    match *curve {
        Curve::Line { start, end } => {
            let (a, b) = if axis == 0 { (start.x, end.x) } else { (start.y, end.y) };
            if (b - a).abs() > 1e-12 {
                parameters.push((position - a) / (b - a));
            }
        }
        Curve::Arc { center, radius, start_angle, sweep } => {
            let v = (position - if axis == 0 { center.x } else { center.y }) / radius;
            if v.abs() <= 1. {
                let angles =
                    if axis == 0 { [v.acos(), -v.acos()] } else { [v.asin(), PI - v.asin()] };
                for angle in angles {
                    let base = ((start_angle - angle) / TAU).floor();
                    for turn in -1..=2 {
                        let t = (angle + (base + f64::from(turn)) * TAU - start_angle) / sweep;
                        if (0. ..=1.).contains(&t) {
                            parameters.push(t);
                        }
                    }
                }
            }
        }
    }
    parameters.retain(|t| (0. ..=1.).contains(t));
    parameters
}

/// The cooling stops as distances along the contour as it runs, with
/// their dwell.
fn cooling_stops(
    contour: &Contour,
    cooling: &Cooling,
    frame: &Frame,
    length: f64,
) -> Vec<(f64, u32)> {
    let dwell = cooling.dwell.0;
    match &cooling.placement {
        CoolingPlacement::Manual(spots) => {
            frame.own(spots).iter().map(|f| (frame.remap(*f) * length, dwell)).collect()
        }
        CoolingPlacement::Automatic { at_start, corners_below } => {
            let mut stops = Vec::new();
            if *at_start {
                stops.push((0., dwell));
            }
            if let Some(limit) = corners_below {
                let count = contour.curves.len();
                let mut distance = 0.;
                for (i, curve) in contour.curves.iter().enumerate() {
                    if i > 0 || frame.closed {
                        let incoming = contour.curves[(i + count - 1) % count].tangent(1.);
                        let interior = 180.
                            - incoming.dot(curve.tangent(0.)).clamp(-1., 1.).acos().to_degrees();
                        if interior <= limit.0 + 1e-8 {
                            stops.push((distance, dwell));
                        }
                    }
                    distance += curve.length();
                }
            }
            stops
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(
        contour: &Contour,
        joints: Option<&Joints>,
        cooling: Option<&Cooling>,
        frame: &Frame,
    ) -> (Vec<Curve>, Vec<SegmentProcess>) {
        super::split(contour, joints, cooling, frame)
            .unwrap()
            .into_iter()
            .map(|s| (s.curve, s.process))
            .unzip()
    }

    use openlaser_core::geometry::Point;
    use openlaser_core::units::{Degrees, Millimeters, Milliseconds, MmPerSecond, Percent};

    fn square() -> Contour {
        let corners =
            [Point::ORIGIN, Point::new(40., 0.), Point::new(40., 40.), Point::new(0., 40.)];
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: corners[i], end: corners[(i + 1) % 4] })
                .collect(),
        }
    }

    fn frame() -> Frame {
        Frame { closed: true, hole: false, reversed: false, start: 0., contour: 0 }
    }

    fn spot(fraction: f64) -> Spot {
        Spot { contour: 0, fraction }
    }

    fn joints(placement: JointPlacement) -> Joints {
        Joints {
            placement,
            width: Millimeters(2.),
            minimum_size: Millimeters(10.),
            outer_only: false,
            open_start: false,
            behaviour: JointBehaviour::Power(Percent(20.)),
            slow_speed: Some(MmPerSecond(5.)),
            repierce: true,
        }
    }

    /// Two joints by count sit a third and two thirds of the way around,
    /// each two millimetres wide, cut at joint power and speed, and the
    /// segment after each re-pierces.
    #[test]
    fn joints_by_count_split_and_tag_the_contour() {
        let (curves, processes) =
            split(&square(), Some(&joints(JointPlacement::Count(2))), None, &frame());
        assert_eq!(curves.len(), 8);
        let joints: Vec<_> =
            processes.iter().enumerate().filter(|(_, p)| p.joint).map(|(i, _)| i).collect();
        assert_eq!(joints, vec![2, 5]);
        assert!((curves[2].length() - 2.).abs() < 1e-9);
        assert!(curves[2].start().distance(Point::new(40., 12.333_333_333_333_334)) < 1e-9);
        assert_eq!(processes[2].power, Some(20.));
        assert_eq!(processes[2].speed, Some(5.));
        assert!(processes[3].repierce && !processes[2].repierce && !processes[0].repierce);
        assert!((curves.iter().map(Curve::length).sum::<f64>() - 160.).abs() < 1e-9);
    }

    /// Valid input counts can expand into too many grid intersections or
    /// spacing positions. Refuse the whole feature instead of truncating it.
    #[test]
    fn generated_positions_are_bounded() {
        let mut dense = joints(JointPlacement::Spacing(Millimeters(0.001)));
        dense.width = Millimeters(0.0001);
        assert!(matches!(
            super::split(&square(), Some(&dense), None, &frame()),
            Err(crate::Error::Budget(_))
        ));
        dense.placement = JointPlacement::AcrossX(10_000);
        assert!(matches!(
            super::split(&square(), Some(&dense), None, &frame()),
            Err(crate::Error::Budget(_))
        ));
    }

    /// A small contour gets no joints, a hole gets none when only outlines
    /// are jointed, and manual positions map through reversal and a moved
    /// start.
    #[test]
    fn joints_are_skipped_or_remapped_as_configured() {
        let small = Contour {
            layer: "0".into(),
            curves: square().curves.iter().map(|c| c.scaled(0.1)).collect(),
        };
        assert!(
            split(&small, Some(&joints(JointPlacement::Count(2))), None, &frame())
                .1
                .iter()
                .all(|p| !p.joint)
        );
        let outer_only = Joints { outer_only: true, ..joints(JointPlacement::Count(2)) };
        let hole = Frame { hole: true, ..frame() };
        assert!(split(&square(), Some(&outer_only), None, &hole).1.iter().all(|p| !p.joint));
        let moved = Frame { reversed: true, start: 0.25, ..frame() };
        let (curves, processes) = split(
            &square().reversed().started_at(40.),
            Some(&joints(JointPlacement::Manual(vec![
                spot(0.375),
                Spot { contour: 3, fraction: 0.5 },
            ]))),
            None,
            &moved,
        );
        let joint = processes.iter().position(|p| p.joint).unwrap();
        // As drawn, three eighths of the way round is (40, 20); reversed and
        // restarted a quarter turn later, it is three eighths along again.
        // The spot on another contour is not this contour's.
        assert_eq!(processes.iter().filter(|p| p.joint).count(), 1);
        assert!(curves[joint].point(0.5).distance(Point::new(40., 20.)) < 1e-9);
    }

    /// On an open contour with the start joint on, joints by count begin
    /// at the start, and a grid joint is added at the start.
    #[test]
    fn open_contours_can_start_in_a_joint() {
        let open = Contour { layer: "0".into(), curves: square().curves[..3].to_vec() };
        let frame = Frame { closed: false, ..frame() };
        let from_start = Joints { open_start: true, ..joints(JointPlacement::Count(2)) };
        let (curves, processes) = split(&open, Some(&from_start), None, &frame);
        assert!(processes[0].joint, "the first segment is a joint");
        assert!((curves[0].length() - 1.).abs() < 1e-9, "half the width, from the start");
        let grid = Joints { open_start: true, ..joints(JointPlacement::AcrossY(1)) };
        let (curves, processes) = split(&open, Some(&grid), None, &frame);
        let first = processes.iter().position(|p| p.joint).unwrap();
        assert!(curves[first].start().distance(Point::new(1., 0.)) < 1e-9, "centred one width in");
        assert_eq!(processes.iter().filter(|p| p.joint).count(), 2);
    }

    /// Grid joints across X sit where vertical grid lines cross the
    /// horizontal sides, never on the vertical sides they run along.
    #[test]
    fn grid_joints_cross_the_contour_steeply() {
        let (curves, processes) =
            split(&square(), Some(&joints(JointPlacement::AcrossX(1))), None, &frame());
        let joints: Vec<_> = curves
            .iter()
            .zip(&processes)
            .filter(|(_, p)| p.joint)
            .map(|(c, _)| c.point(0.5))
            .collect();
        assert_eq!(joints.len(), 2);
        assert!(joints.iter().all(|p| (p.x - 20.).abs() < 1e-9 && (p.y == 0. || p.y == 40.)));
    }

    /// Cooling stops land at the start and at every right angle of a
    /// square, each carrying the dwell, and manual stops at their fraction.
    #[test]
    fn cooling_stops_tag_the_segment_they_start() {
        let cooling = Cooling {
            dwell: Milliseconds(300),
            placement: CoolingPlacement::Automatic {
                at_start: true,
                corners_below: Some(Degrees(90.)),
            },
        };
        let (curves, processes) = split(&square(), None, Some(&cooling), &frame());
        assert_eq!(curves.len(), 4);
        assert!(processes.iter().all(|p| p.cool_ms == 300));
        let gentle = Cooling {
            dwell: Milliseconds(300),
            placement: CoolingPlacement::Automatic {
                at_start: false,
                corners_below: Some(Degrees(60.)),
            },
        };
        assert!(split(&square(), None, Some(&gentle), &frame()).1.iter().all(|p| p.cool_ms == 0));
        let manual = Cooling {
            dwell: Milliseconds(50),
            placement: CoolingPlacement::Manual(vec![spot(0.125)]),
        };
        let (curves, processes) = split(&square(), None, Some(&manual), &frame());
        assert_eq!(curves.len(), 5);
        assert_eq!(processes[1].cool_ms, 50);
        assert_eq!(curves[1].start(), Point::new(20., 0.));
    }

    /// Feature values that make no sense are refused before any geometry
    /// is touched.
    #[test]
    fn bad_values_are_refused() {
        assert!(
            check(&Joints { width: Millimeters(0.), ..joints(JointPlacement::Count(1)) }).is_err()
        );
        assert!(check(&joints(JointPlacement::Count(0))).is_err());
        assert!(check(&joints(JointPlacement::Spacing(Millimeters(1.)))).is_err());
        assert!(check(&joints(JointPlacement::Manual(vec![spot(1.5)]))).is_err());
        assert!(check(&joints(JointPlacement::Manual(vec![]))).is_ok(), "none picked yet");
        assert!(
            check(&Joints {
                behaviour: JointBehaviour::Power(Percent(120.)),
                ..joints(JointPlacement::Count(1))
            })
            .is_err()
        );
        assert!(check(&joints(JointPlacement::Count(3))).is_ok());
        assert!(
            check_cooling(&Cooling {
                dwell: Milliseconds(0),
                placement: CoolingPlacement::Manual(vec![spot(0.5)])
            })
            .is_err()
        );
        assert!(
            check_cooling(&Cooling {
                dwell: Milliseconds(1),
                placement: CoolingPlacement::Automatic {
                    at_start: true,
                    corners_below: Some(Degrees(200.))
                }
            })
            .is_err()
        );
    }
}
