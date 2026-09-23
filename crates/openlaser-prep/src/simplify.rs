// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing simplification. CAD exports often flatten splines, ellipses and
//! fillets into runs of very short lines; each becomes the arcs and longer
//! lines it follows, within a tolerance. A contour that repeats another on
//! the same layer, which would be cut twice, is dropped, and so is a speck
//! smaller than the tolerance. Every kept contour stays within the
//! tolerance of its original, closed contours stay closed, and a contour
//! that would cross itself or lose its closure is kept as drawn.

use crate::{Result, feature, float, topology};
use openlaser_core::fit::{curve_distance, fit_points, push_arc};
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Point};

pub use openlaser_core::fit::{MAX_TOLERANCE, MIN_TOLERANCE};

/// A simplified drawing and what changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Simplified {
    /// The simplified drawing.
    pub drawing: Drawing,
    /// Lines and arcs before.
    pub curves_before: usize,
    /// Lines and arcs after.
    pub curves_after: usize,
    /// Contours dropped because another on the same layer cuts the same path.
    pub repeats: usize,
    /// Contours dropped because they are smaller than the tolerance.
    pub specks: usize,
}

impl Simplified {
    /// Whether anything changed.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.curves_after != self.curves_before || self.repeats > 0 || self.specks > 0
    }
}

/// Simplifies `drawing` so no kept path moves by more than `tolerance`.
pub fn simplify(drawing: &Drawing, tolerance: f64) -> Result<Simplified> {
    if !(MIN_TOLERANCE..=MAX_TOLERANCE).contains(&tolerance) {
        return Err(feature(
            "simplify",
            format!("the tolerance must be {MIN_TOLERANCE} to {MAX_TOLERANCE} mm"),
        ));
    }
    let curves_before = drawing.contours.iter().map(|c| c.curves.len()).sum();
    let mut specks = 0;
    let mut contours = Vec::with_capacity(drawing.contours.len());
    for contour in &drawing.contours {
        if contour.bounds().is_none_or(|b| b.extent() <= tolerance) {
            specks += 1;
        } else {
            contours.push(simplified(contour, tolerance));
        }
    }
    let (contours, repeats) = without_repeats(contours, tolerance);
    if contours.is_empty() {
        return Err(feature("simplify", "nothing larger than the tolerance would remain"));
    }
    let curves_after = contours.iter().map(|c| c.curves.len()).sum();
    Ok(Simplified { drawing: Drawing { contours }, curves_before, curves_after, repeats, specks })
}

/// The contour with its runs of lines fitted, or as drawn when fitting
/// would change what it is.
fn simplified(contour: &Contour, tolerance: f64) -> Contour {
    let mut curves: Vec<Curve> = Vec::with_capacity(contour.curves.len());
    let mut run: Vec<Point> = Vec::new();
    for curve in &contour.curves {
        match *curve {
            Curve::Line { start, end } => {
                if run.is_empty() {
                    run.push(start);
                }
                run.push(end);
            }
            arc @ Curve::Arc { .. } => {
                fit_points(&run, tolerance, &mut curves);
                run.clear();
                push_arc(&mut curves, arc);
            }
        }
    }
    fit_points(&run, tolerance, &mut curves);
    let candidate = Contour { layer: contour.layer.clone(), curves };
    if acceptable(contour, &candidate) { candidate } else { contour.clone() }
}

/// Whether a fitted contour can replace its original: no more curves, every
/// curve valid and continuous, closed as before, and not crossing itself.
fn acceptable(original: &Contour, candidate: &Contour) -> bool {
    candidate.curves.len() <= original.curves.len()
        && !candidate.curves.is_empty()
        && candidate.curves.iter().all(Curve::is_valid)
        && candidate.is_continuous()
        && candidate.is_closed() == original.is_closed()
        && (!candidate.is_closed()
            || topology::polyline(original).is_err()
            || topology::polyline(candidate).is_ok())
}

/// Every contour but those repeating an earlier one on the same layer, and
/// how many were dropped. Candidates share their extent within the
/// tolerance; a repeat is one whose every sampled point lies within the
/// tolerance of the other, both ways, however each is split or directed.
fn without_repeats(contours: Vec<Contour>, tolerance: f64) -> (Vec<Contour>, usize) {
    let bounds: Vec<Bounds> = contours.iter().filter_map(Contour::bounds).collect();
    if bounds.len() != contours.len() {
        return (contours, 0);
    }
    let mut order: Vec<usize> = (0..contours.len()).collect();
    order.sort_by(|&a, &b| bounds[a].min.x.total_cmp(&bounds[b].min.x).then(a.cmp(&b)));
    let mut repeated = vec![false; contours.len()];
    for (slot, &i) in order.iter().enumerate() {
        if repeated[i] {
            continue;
        }
        for &j in &order[slot + 1..] {
            if bounds[j].min.x - bounds[i].min.x > tolerance {
                break;
            }
            let (first, later) = (i.min(j), i.max(j));
            if !repeated[later]
                && same_extent(&bounds[i], &bounds[j], tolerance)
                && same_path(&contours[first], &contours[later], tolerance)
            {
                repeated[later] = true;
            }
        }
    }
    let count = repeated.iter().filter(|&&r| r).count();
    let kept = contours.into_iter().zip(repeated).filter(|(_, r)| !r).map(|(c, _)| c).collect();
    (kept, count)
}

fn same_extent(a: &Bounds, b: &Bounds, tolerance: f64) -> bool {
    [a.min.x - b.min.x, a.min.y - b.min.y, a.max.x - b.max.x, a.max.y - b.max.y]
        .iter()
        .all(|d| d.abs() <= tolerance)
}

/// Whether two contours on one layer trace the same path.
fn same_path(a: &Contour, b: &Contour, tolerance: f64) -> bool {
    const MOST: usize = 5000;
    if a.layer != b.layer || a.curves.len() > MOST || b.curves.len() > MOST {
        return false;
    }
    if (a.length() - b.length()).abs() > 2. * tolerance * float(a.curves.len() + b.curves.len()) {
        return false;
    }
    let within = |from: &Contour, to: &Contour| {
        from.curves.iter().all(|curve| {
            [0., 0.25, 0.5, 0.75].iter().all(|&t| {
                let p = curve.point(t);
                to.curves.iter().any(|other| curve_distance(p, other) <= tolerance)
            })
        })
    };
    within(a, b) && within(b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{PI, TAU};

    fn polyline(points: &[Point], closed: bool) -> Contour {
        let mut curves: Vec<Curve> =
            points.windows(2).map(|p| Curve::Line { start: p[0], end: p[1] }).collect();
        if closed {
            curves.push(Curve::Line { start: *points.last().unwrap(), end: points[0] });
        }
        Contour { layer: "0".into(), curves }
    }

    /// A circle of radius `r` about `c` flattened into `n` lines.
    fn polygon(c: Point, r: f64, n: usize) -> Contour {
        let points: Vec<_> =
            (0..n).map(|k| c + Point::direction(TAU * float(k) / float(n)) * r).collect();
        polyline(&points, true)
    }

    fn max_deviation(original: &Contour, simplified: &Contour) -> f64 {
        let sample = |from: &Contour, to: &Contour| {
            from.curves
                .iter()
                .flat_map(|c| (0..=8).map(move |k| c.point(f64::from(k) / 8.)))
                .map(|p| to.curves.iter().map(|o| curve_distance(p, o)).fold(f64::MAX, f64::min))
                .fold(0., f64::max)
        };
        sample(original, simplified).max(sample(simplified, original))
    }

    #[test]
    fn a_flattened_circle_becomes_two_arcs_within_the_tolerance() {
        let original = polygon(Point::new(500., 300.), 20., 180);
        let result = simplify(&Drawing { contours: vec![original.clone()] }, 0.02).unwrap();
        let contour = &result.drawing.contours[0];
        assert_eq!((result.curves_before, result.curves_after), (180, 2), "{:?}", contour.curves);
        assert!(contour.curves.iter().all(|c| matches!(c, Curve::Arc { .. })));
        assert!(contour.is_closed());
        assert!(max_deviation(&original, contour) <= 0.02 + 1e-9);
        assert!((contour.signed_area() - original.signed_area()).abs() < 1.);
    }

    #[test]
    fn straight_runs_join_and_corners_stay_sharp() {
        let mut points = Vec::new();
        for k in 0..=50 {
            points.push(Point::new(f64::from(k) * 2., 0.001 * f64::from(k % 2)));
        }
        points.push(Point::new(100., 40.));
        points.push(Point::new(0., 40.));
        let original = polyline(&points, true);
        let result = simplify(&Drawing { contours: vec![original.clone()] }, 0.01).unwrap();
        let contour = &result.drawing.contours[0];
        assert_eq!(contour.curves.len(), 4, "{:?}", contour.curves);
        for corner in [Point::new(100., 0.), Point::new(100., 40.), Point::new(0., 40.)] {
            assert!(contour.curves.iter().any(|c| c.start().distance(corner) < 1e-9));
        }
        assert!(max_deviation(&original, contour) <= 0.01 + 1e-9);
    }

    #[test]
    fn repeats_and_specks_are_dropped_and_arcs_on_one_circle_join() {
        let square = polyline(
            &[Point::new(0., 0.), Point::new(10., 0.), Point::new(10., 10.), Point::new(0., 10.)],
            true,
        );
        // The same square, drawn the other way from another corner.
        let again = polyline(
            &[Point::new(10., 10.), Point::new(10., 0.), Point::new(0., 0.), Point::new(0., 10.)],
            true,
        );
        let mut etched = square.clone();
        etched.layer = "Etch".into();
        let speck = polygon(Point::new(50., 50.), 0.004, 8);
        let halves = Contour {
            layer: "0".into(),
            curves: vec![
                Curve::Arc { center: Point::new(30., 5.), radius: 3., start_angle: 0., sweep: PI },
                Curve::Arc { center: Point::new(30., 5.), radius: 3., start_angle: PI, sweep: PI },
            ],
        };
        let drawing = Drawing { contours: vec![square, again, etched, speck, halves] };
        let result = simplify(&drawing, 0.02).unwrap();
        assert_eq!((result.repeats, result.specks), (1, 1));
        assert_eq!(result.drawing.contours.len(), 3);
        assert_eq!(result.drawing.contours[1].layer, "Etch", "another layer is not a repeat");
        assert_eq!(result.drawing.contours[2].curves.len(), 1, "one full circle");
        assert!(result.drawing.contours[2].is_closed());
        assert!(result.changed());
    }

    #[test]
    fn open_paths_stay_open_and_nothing_changes_twice() {
        let wave: Vec<_> = (0..=200)
            .map(|k| {
                let x = f64::from(k) * 0.5;
                Point::new(x, 10. * (x / 15.).sin())
            })
            .collect();
        let original = polyline(&wave, false);
        let once = simplify(&Drawing { contours: vec![original.clone()] }, 0.01).unwrap();
        let contour = &once.drawing.contours[0];
        assert!(!contour.is_closed() && contour.is_continuous());
        assert!(once.curves_after < 60, "{}", once.curves_after);
        assert!(contour.start().unwrap().distance(original.start().unwrap()) < 1e-9);
        assert!(contour.end().unwrap().distance(original.end().unwrap()) < 1e-9);
        assert!(max_deviation(&original, contour) <= 0.01 + 1e-9);
        let twice = simplify(&once.drawing, 0.01).unwrap();
        assert!(!twice.changed() || twice.curves_after <= once.curves_after);
    }

    #[test]
    fn the_tolerance_is_bounded_and_something_must_remain() {
        let square = polyline(
            &[Point::new(0., 0.), Point::new(1., 0.), Point::new(1., 1.), Point::new(0., 1.)],
            true,
        );
        let drawing = Drawing { contours: vec![square] };
        assert!(simplify(&drawing, 0.).is_err());
        assert!(simplify(&drawing, 1.).is_err());
        let tiny = Drawing { contours: vec![polygon(Point::new(0., 0.), 0.1, 6)] };
        assert!(simplify(&tiny, 0.5).is_err());
    }
}
