// SPDX-License-Identifier: GPL-3.0-or-later

//! Fitting lines and arcs to points and curves, within a tolerance.
//!
//! One fitter serves the Simplify tool, which turns runs of short lines back
//! into the arcs they follow, and import, which turns splines, ellipses and
//! stretched arcs into the lines and arcs a machine cuts. A run of points is
//! covered greedily by the longest line or arc that keeps every point, and
//! every chord's middle, within the tolerance. Corners stay sharp, the first
//! and last points are kept exactly, and consecutive arcs on one circle are
//! joined.
//!
//! ```
//! use openlaser_core::fit;
//! use openlaser_core::geometry::{Curve, Point};
//!
//! // A quarter of the unit circle, as a parametric curve.
//! let curves = fit::approximate(|t| Point::direction(t) * 10., 0., 1.5, 0.01, false).unwrap();
//! assert!(curves.iter().all(|c| matches!(c, Curve::Arc { .. })));
//! ```

use crate::geometry::{Curve, Point};
use std::f64::consts::{PI, TAU};

/// The finest tolerance fitting takes, in millimetres.
pub const MIN_TOLERANCE: f64 = 0.001;

/// The coarsest tolerance fitting takes, in millimetres.
pub const MAX_TOLERANCE: f64 = 0.5;

/// The most original lines one fitted curve replaces, which bounds the
/// search for each curve.
const MAX_SPAN: usize = 256;

/// Radii beyond this are straight for any tolerance taken here.
const MAX_RADIUS: f64 = 100_000.;

/// Points closer than this are the same point.
const SAME: f64 = 1e-9;

/// The share of the tolerance a sampled curve may leave between its samples
/// and their chords; the fit takes the rest.
const SAMPLE_SHARE: f64 = 0.25;

/// The share of the tolerance the fitted lines and arcs may take from the
/// samples.
const FIT_SHARE: f64 = 0.7;

/// The most samples one curve may need before approximation gives up.
const MAX_SAMPLES: usize = 1_000_000;

/// Appends the fewest lines and arcs found greedily through `points`, each
/// the longest that keeps every point and every chord's middle within
/// `tolerance`. Repeated points are ignored; fewer than two distinct points
/// add nothing.
pub fn fit_points(points: &[Point], tolerance: f64, curves: &mut Vec<Curve>) {
    let mut points = points.to_vec();
    points.dedup_by(|next, kept| next.distance(*kept) <= SAME);
    let mut from = 0;
    while from + 1 < points.len() {
        let line = longest_line(&points, from, tolerance);
        match longest_arc(&points, from, tolerance) {
            Some((to, arc)) if to > line => {
                push_arc(curves, arc);
                from = to;
            }
            _ => {
                curves.push(Curve::Line { start: points[from], end: points[line] });
                from = line;
            }
        }
    }
}

/// Lines and arcs within `tolerance` of the curve `at` traces from parameter
/// `from` to `to`. The curve is sampled until every chord stays within a
/// quarter of the tolerance, then fitted. A `closed` curve ends exactly where
/// it starts. `None` when the curve is not finite, has no extent, or would
/// need more than a million samples.
pub fn approximate(
    at: impl Fn(f64) -> Point,
    from: f64,
    to: f64,
    tolerance: f64,
    closed: bool,
) -> Option<Vec<Curve>> {
    let mut points = sample(&at, from, to, tolerance * SAMPLE_SHARE)?;
    if closed && let Some(&first) = points.first() {
        let last = points.len() - 1;
        points[last] = first;
    }
    let mut curves = Vec::new();
    fit_points(&points, tolerance * FIT_SHARE, &mut curves);
    (!curves.is_empty()).then_some(curves)
}

/// Points along the curve from `from` to `to`, subdivided until the curve's
/// quarter, middle and three-quarter points between two neighbours lie
/// within `tolerance` of their chord.
fn sample(at: &impl Fn(f64) -> Point, from: f64, to: f64, tolerance: f64) -> Option<Vec<Point>> {
    const START: u32 = 32;
    const DEEPEST: u32 = 40;
    if !(from.is_finite() && to.is_finite() && to > from && tolerance > 0.) {
        return None;
    }
    let step = (to - from) / f64::from(START);
    let mut points = vec![at(from)];
    // Depth-first, so the points come out in order.
    let mut pending: Vec<(f64, f64, Point, u32)> = (0..START)
        .rev()
        .map(|k| {
            let b = if k + 1 == START { to } else { from + step * f64::from(k + 1) };
            (from + step * f64::from(k), b, at(b), 0)
        })
        .collect();
    while let Some((a, b, end, depth)) = pending.pop() {
        let start = *points.last()?;
        let quarter = |k: f64| at(a + (b - a) * k);
        let (q1, middle, q3) = (quarter(0.25), quarter(0.5), quarter(0.75));
        if ![start, end, q1, middle, q3].iter().all(|p| p.is_finite()) {
            return None;
        }
        let flat = [q1, middle, q3].iter().all(|&p| segment_distance(p, start, end) <= tolerance);
        if flat || depth >= DEEPEST {
            points.push(end);
            if points.len() > MAX_SAMPLES {
                return None;
            }
        } else {
            let m = a + (b - a) * 0.5;
            pending.push((m, b, end, depth + 1));
            pending.push((a, m, middle, depth + 1));
        }
    }
    Some(points)
}

/// The furthest point a straight line from `from` can reach with every
/// point between within the tolerance.
fn longest_line(points: &[Point], from: usize, tolerance: f64) -> usize {
    let last = (from + MAX_SPAN).min(points.len() - 1);
    let mut best = from + 1;
    for to in from + 2..=last {
        let (a, b) = (points[from], points[to]);
        if points[from + 1..to].iter().all(|&p| segment_distance(p, a, b) <= tolerance) {
            best = to;
        } else {
            break;
        }
    }
    best
}

/// The furthest point, at least three lines on, an arc from `from` can
/// reach within the tolerance, and that arc.
fn longest_arc(points: &[Point], from: usize, tolerance: f64) -> Option<(usize, Curve)> {
    let last = (from + MAX_SPAN).min(points.len() - 1);
    let mut best = None;
    for to in from + 3..=last {
        match fit_arc(&points[from..=to], tolerance) {
            Some(arc) => best = Some((to, arc)),
            None => break,
        }
    }
    best
}

/// The arc through the first, middle and last points, when every point and
/// every chord's middle lies within the tolerance of it, it turns one way
/// throughout and sweeps at most half a turn.
fn fit_arc(points: &[Point], tolerance: f64) -> Option<Curve> {
    let (first, last) = (*points.first()?, *points.last()?);
    let (center, radius) = circle(first, points[points.len() / 2], last)?;
    if !(radius > tolerance && radius < MAX_RADIUS) {
        return None;
    }
    let mut sweep = 0.;
    let mut turn: Option<bool> = None;
    for pair in points.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        let step = (p - center).cross(q - center).atan2((p - center).dot(q - center));
        let forward = step > 0.;
        if step.abs() <= f64::EPSILON || turn.is_some_and(|t| t != forward) {
            return None;
        }
        turn = Some(forward);
        sweep += step;
        let off = |at: Point| (at.distance(center) - radius).abs() > tolerance;
        if off(q) || off(p.lerp(q, 0.5)) || sweep.abs() > PI + 1e-9 {
            return None;
        }
    }
    let start_angle = (first - center).angle();
    let arc = Curve::Arc { center, radius, start_angle, sweep };
    (arc.end().distance(last) <= SAME * 1e3 && arc.start().distance(first) <= SAME * 1e3)
        .then_some(arc)
}

/// The circle through three points, computed about the first for accuracy
/// far from the origin; none when they are in line.
#[must_use]
pub fn circle(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let (b, c) = (b - a, c - a);
    let d = 2. * b.cross(c);
    if d.abs() <= SAME * (b.norm() * c.norm()).max(SAME) {
        return None;
    }
    let (bb, cc) = (b.dot(b), c.dot(c));
    let center = Point::new((c.y * bb - b.y * cc) / d, (b.x * cc - c.x * bb) / d);
    let radius = center.norm();
    radius.is_finite().then_some((a + center, radius))
}

/// Adds an arc, joining it to the arc before when both lie on one circle and
/// turn the same way.
pub fn push_arc(curves: &mut Vec<Curve>, arc: Curve) {
    if let (
        Some(Curve::Arc { center, radius, start_angle, sweep }),
        Curve::Arc { center: c, radius: r, sweep: s, .. },
    ) = (curves.last().copied(), arc)
        && center.distance(c) <= SAME * 100.
        && (radius - r).abs() <= SAME * 100.
        && sweep.signum() == s.signum()
        && (sweep + s).abs() <= TAU
    {
        let joined = Curve::Arc { center, radius, start_angle, sweep: sweep + s };
        if joined.end().distance(arc.end()) <= SAME * 1e3 {
            curves.pop();
            curves.push(joined);
            return;
        }
    }
    curves.push(arc);
}

/// The distance from `p` to the segment from `a` to `b`.
#[must_use]
pub fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let length = ab.dot(ab);
    if length <= SAME * SAME {
        return p.distance(a);
    }
    p.distance(a + ab * ((p - a).dot(ab) / length).clamp(0., 1.))
}

/// The distance from `p` to the nearest point of `curve`.
#[must_use]
pub fn curve_distance(p: Point, curve: &Curve) -> f64 {
    match *curve {
        Curve::Line { start, end } => segment_distance(p, start, end),
        Curve::Arc { center, radius, start_angle, sweep } => {
            let angle = (p - center).angle();
            let along = (angle - start_angle) * sweep.signum();
            let within = along.rem_euclid(TAU) <= sweep.abs() + 1e-12;
            if within {
                (p.distance(center) - radius).abs()
            } else {
                p.distance(curve.start()).min(p.distance(curve.end()))
            }
        }
    }
}

/// The largest distance from `samples` points along the curve `at` traces
/// between `from` and `to` to the nearest of `curves`: how far a fit strays
/// from what it replaces.
#[must_use]
pub fn deviation(
    at: impl Fn(f64) -> Point,
    from: f64,
    to: f64,
    samples: u32,
    curves: &[Curve],
) -> f64 {
    (0..=samples)
        .map(|k| at(from + (to - from) * f64::from(k) / f64::from(samples.max(1))))
        .map(|p| curves.iter().map(|c| curve_distance(p, c)).fold(f64::MAX, f64::min))
        .fold(0., f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Contour;

    /// The distance both ways between the fit and a dense sampling of the
    /// curve it replaces.
    fn both_ways(at: &impl Fn(f64) -> Point, from: f64, to: f64, curves: &[Curve]) -> f64 {
        let there = deviation(at, from, to, 20_000, curves);
        let dense: Vec<Point> =
            (0..=4000).map(|k| at(from + (to - from) * f64::from(k) / 4000.)).collect();
        let back = curves
            .iter()
            .flat_map(|c| (0..=64).map(move |k| c.point(f64::from(k) / 64.)))
            .map(|p| {
                dense.windows(2).map(|w| segment_distance(p, w[0], w[1])).fold(f64::MAX, f64::min)
            })
            .fold(0., f64::max);
        there.max(back)
    }

    #[test]
    fn an_ellipse_becomes_arcs_within_the_tolerance_and_stays_closed() {
        let ellipse = |t: f64| Point::new(40. * t.cos() + 3., 15. * t.sin() - 7.);
        for tolerance in [0.001, 0.01, 0.1] {
            let curves = approximate(ellipse, 0., TAU, tolerance, true).unwrap();
            let contour = Contour { layer: "0".into(), curves: curves.clone() };
            assert!(contour.is_closed(), "{tolerance}");
            assert!(curves.iter().all(Curve::is_valid));
            assert!(curves.iter().filter(|c| matches!(c, Curve::Arc { .. })).count() > 3);
            let off = both_ways(&ellipse, 0., TAU, &curves);
            assert!(off <= tolerance, "{tolerance}: {off}");
        }
    }

    #[test]
    fn a_circle_becomes_one_full_arc() {
        let circle = |t: f64| Point::new(5., 5.) + Point::direction(t) * 12.;
        let curves = approximate(circle, 0., TAU, 0.01, true).unwrap();
        let contour = Contour { layer: "0".into(), curves };
        assert!(contour.is_circle(), "{:?}", contour.curves);
    }

    #[test]
    fn straight_curves_become_one_line_and_corners_stay() {
        let line = |t: f64| Point::new(t, 2. * t);
        assert_eq!(approximate(line, 0., 10., 0.01, false).unwrap().len(), 1);
        let mut curves = Vec::new();
        fit_points(
            &[Point::new(0., 0.), Point::new(5., 0.), Point::new(10., 0.), Point::new(10., 10.)],
            0.01,
            &mut curves,
        );
        assert_eq!(curves.len(), 2);
        assert_eq!(curves[0].end(), Point::new(10., 0.));
    }

    #[test]
    fn degenerate_curves_are_refused() {
        assert!(approximate(|_| Point::ORIGIN, 0., 1., 0.01, false).is_none());
        assert!(approximate(|t| Point::new(t, 0.), 1., 1., 0.01, false).is_none());
        assert!(approximate(|_| Point::new(f64::NAN, 0.), 0., 1., 0.01, false).is_none());
    }
}
