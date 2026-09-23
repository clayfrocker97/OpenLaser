// SPDX-License-Identifier: GPL-3.0-or-later

//! The analytic curves DXF draws that a machine cannot cut directly:
//! NURBS splines, splines through fit points, elliptical arcs, and circular
//! arcs stretched by a block's unequal scale. Each is evaluated here and
//! fitted into lines and arcs by [`openlaser_core::fit`] once the drawing
//! units, and so the tolerance in drawing units, are known.

use openlaser_core::geometry::{Curve, Point};
use std::f64::consts::TAU;

/// The highest spline degree read; DXF writers use 1 to 3, and rarely 5.
pub(crate) const MAX_DEGREE: usize = 11;

/// A parametric curve and the parameter range drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Parametric {
    /// A non-uniform rational B-spline.
    Nurbs(Nurbs),
    /// A cubic through fit points.
    Cubic(Cubic),
    /// An elliptical arc.
    Ellipse(Ellipse),
    /// A circular arc, before a transform that stretches it.
    Arc(Curve),
}

impl Parametric {
    /// The point at parameter `t`.
    pub(crate) fn at(&self, t: f64) -> Point {
        match self {
            Self::Nurbs(n) => n.at(t),
            Self::Cubic(c) => c.at(t),
            Self::Ellipse(e) => e.at(t),
            Self::Arc(a) => a.point(t),
        }
    }

    /// The parameter range drawn.
    pub(crate) fn domain(&self) -> (f64, f64) {
        match self {
            Self::Nurbs(n) => (n.knots[n.degree], n.knots[n.controls.len()]),
            Self::Cubic(c) => (c.from, c.to),
            Self::Ellipse(e) => (e.start, e.end),
            Self::Arc(_) => (0., 1.),
        }
    }

    /// Whether the curve ends where it starts.
    pub(crate) fn closed(&self) -> bool {
        let (from, to) = self.domain();
        let (a, b) = (self.at(from), self.at(to));
        let size = match self {
            Self::Nurbs(n) => extent(&n.controls),
            Self::Cubic(c) => extent(&c.points),
            Self::Ellipse(e) => e.major.norm(),
            Self::Arc(a) => a.length(),
        };
        a.distance(b) <= 1e-9 * size.max(1.)
    }
}

fn extent(points: &[Point]) -> f64 {
    points.iter().filter_map(|p| points.first().map(|q| p.distance(*q))).fold(0., f64::max)
}

/// A non-uniform rational B-spline: `degree`, one weight per control point,
/// and `controls + degree + 1` knots.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Nurbs {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub controls: Vec<Point>,
    pub weights: Vec<f64>,
}

impl Nurbs {
    /// Checks the definition, or says what is wrong with it.
    pub(crate) fn new(
        degree: usize,
        knots: Vec<f64>,
        controls: Vec<Point>,
        weights: Vec<f64>,
    ) -> Result<Self, String> {
        let n = controls.len();
        if !(1..=MAX_DEGREE).contains(&degree) {
            return Err(format!("spline degree {degree} is not 1 to {MAX_DEGREE}"));
        }
        if n <= degree {
            return Err("a spline needs more control points than its degree".into());
        }
        if knots.len() != n + degree + 1 {
            return Err(format!(
                "{} knots where {} control points of degree {degree} need {}",
                knots.len(),
                n,
                n + degree + 1
            ));
        }
        if weights.len() != n || weights.iter().any(|w| !(w.is_finite() && *w > 0.)) {
            return Err("spline weights must be positive, one per control point".into());
        }
        if knots.windows(2).any(|k| k[1] < k[0]) || knots.iter().any(|k| !k.is_finite()) {
            return Err("spline knots must not decrease".into());
        }
        if knots[degree] >= knots[n] {
            return Err("the spline's knots leave it no extent".into());
        }
        Ok(Self { degree, knots, controls, weights })
    }

    /// A clamped, uniform B-spline over `controls`, as a spline-fit
    /// polyline's frame describes one.
    pub(crate) fn uniform(controls: Vec<Point>, degree: usize) -> Result<Self, String> {
        let n = controls.len();
        let degree = degree.min(n.saturating_sub(1)).max(1);
        let spans = n - degree;
        let mut knots = vec![0.; degree + 1];
        knots.extend((1..spans).map(crate::float));
        knots.extend(vec![crate::float(spans); degree + 1]);
        let weights = vec![1.; n];
        Self::new(degree, knots, controls, weights)
    }

    /// The point at `u`, by de Boor's algorithm in homogeneous coordinates.
    #[allow(clippy::many_single_char_names, reason = "the algorithm's conventional names")]
    pub(crate) fn at(&self, u: f64) -> Point {
        let (p, n) = (self.degree, self.controls.len());
        let span = if u >= self.knots[n] {
            n - 1
        } else {
            self.knots.partition_point(|&k| k <= u).saturating_sub(1).clamp(p, n - 1)
        };
        let mut d = [[0.; 3]; MAX_DEGREE + 1];
        for (j, slot) in d.iter_mut().enumerate().take(p + 1) {
            let i = span - p + j;
            let (c, w) = (self.controls[i], self.weights[i]);
            *slot = [c.x * w, c.y * w, w];
        }
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let denominator = self.knots[i + p + 1 - r] - self.knots[i];
                let alpha = if denominator == 0. { 0. } else { (u - self.knots[i]) / denominator };
                let previous = d[j - 1];
                for (value, before) in d[j].iter_mut().zip(previous) {
                    *value = before * (1. - alpha) + *value * alpha;
                }
            }
        }
        Point::new(d[p][0] / d[p][2], d[p][1] / d[p][2])
    }
}

/// A cubic through fit points, parameterised by chord length, with its
/// second derivatives at the points.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cubic {
    params: Vec<f64>,
    points: Vec<Point>,
    second: Vec<Point>,
    from: f64,
    to: f64,
}

impl Cubic {
    /// The curve through `points`. With tangents, the ends run along them;
    /// otherwise they are free. A closed curve runs on past its first point
    /// so its join is smooth.
    pub(crate) fn through(
        points: &[Point],
        tangents: (Option<Point>, Option<Point>),
        closed: bool,
    ) -> Result<Self, String> {
        let mut points: Vec<Point> = points.to_vec();
        points.dedup_by(|a, b| a.distance(*b) <= 1e-9);
        if closed && points.len() > 2 && points[0].distance(points[points.len() - 1]) > 1e-9 {
            points.push(points[0]);
        }
        if points.len() < 2 {
            return Err("a spline needs at least two distinct fit points".into());
        }
        if closed && points.len() > 3 {
            // Three points of overlap either side make the join smooth.
            let n = points.len() - 1;
            let mut wrapped: Vec<Point> = points[n - 3..n].to_vec();
            wrapped.extend_from_slice(&points);
            wrapped.extend_from_slice(&points[1..4.min(points.len())]);
            let mut cubic = Self::solve(wrapped, (None, None))?;
            cubic.from = cubic.params[3];
            cubic.to = cubic.params[3 + n];
            return Ok(cubic);
        }
        Self::solve(points, tangents)
    }

    fn solve(points: Vec<Point>, tangents: (Option<Point>, Option<Point>)) -> Result<Self, String> {
        let n = points.len();
        let mut params = vec![0.];
        for pair in points.windows(2) {
            params.push(params[params.len() - 1] + pair[0].distance(pair[1]));
        }
        let h: Vec<f64> = params.windows(2).map(|p| p[1] - p[0]).collect();
        // The tridiagonal system for the second derivatives.
        let (mut lower, mut diagonal, mut upper) = (vec![0.; n], vec![1.; n], vec![0.; n]);
        let mut rhs = vec![Point::ORIGIN; n];
        let slope = |i: usize| (points[i + 1] - points[i]) * (1. / h[i]);
        if let Some(t) = tangents.0.filter(|t| t.norm() > 0.) {
            let t = t * (1. / t.norm());
            diagonal[0] = h[0] / 3.;
            upper[0] = h[0] / 6.;
            rhs[0] = slope(0) - t;
        }
        if let Some(t) = tangents.1.filter(|t| t.norm() > 0.) {
            let t = t * (1. / t.norm());
            lower[n - 1] = h[n - 2] / 6.;
            diagonal[n - 1] = h[n - 2] / 3.;
            rhs[n - 1] = t - slope(n - 2);
        }
        for i in 1..n - 1 {
            lower[i] = h[i - 1] / 6.;
            diagonal[i] = (h[i - 1] + h[i]) / 3.;
            upper[i] = h[i] / 6.;
            rhs[i] = slope(i) - slope(i - 1);
        }
        for i in 1..n {
            let m = lower[i] / diagonal[i - 1];
            diagonal[i] -= m * upper[i - 1];
            rhs[i] = rhs[i] - rhs[i - 1] * m;
        }
        let mut second = vec![Point::ORIGIN; n];
        second[n - 1] = rhs[n - 1] * (1. / diagonal[n - 1]);
        for i in (0..n - 1).rev() {
            second[i] = (rhs[i] - second[i + 1] * upper[i]) * (1. / diagonal[i]);
        }
        if second.iter().any(|p| !p.is_finite()) {
            return Err("the spline's fit points cannot be interpolated".into());
        }
        let to = params[n - 1];
        Ok(Self { params, points, second, from: 0., to })
    }

    #[allow(clippy::many_single_char_names, reason = "the textbook spline formula's names")]
    fn at(&self, t: f64) -> Point {
        let n = self.points.len();
        let i = self.params.partition_point(|&p| p <= t).saturating_sub(1).min(n - 2);
        let h = self.params[i + 1] - self.params[i];
        let a = (self.params[i + 1] - t) / h;
        let b = (t - self.params[i]) / h;
        self.points[i] * a
            + self.points[i + 1] * b
            + (self.second[i] * (a * a * a - a) + self.second[i + 1] * (b * b * b - b))
                * (h * h / 6.)
    }
}

/// An elliptical arc: `center + major·cos t + minor·sin t` from `start` to
/// `end`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ellipse {
    pub center: Point,
    pub major: Point,
    pub minor: Point,
    pub start: f64,
    pub end: f64,
}

impl Ellipse {
    fn at(&self, t: f64) -> Point {
        let (sin, cos) = t.sin_cos();
        self.center + self.major * cos + self.minor * sin
    }

    /// The exact arc when the ellipse is a circle.
    pub(crate) fn as_arc(&self) -> Option<Curve> {
        let radius = self.major.norm();
        let turn = self.major.cross(self.minor).signum();
        let circular = (self.minor.norm() - radius).abs() <= 1e-12 * radius
            && self.major.dot(self.minor).abs() <= 1e-12 * radius * radius;
        circular.then(|| Curve::Arc {
            center: self.center,
            radius,
            start_angle: self.major.angle() + turn * self.start,
            sweep: turn * (self.end - self.start).min(TAU),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    /// A rational quadratic with the classic weights draws an exact quarter
    /// circle.
    #[test]
    fn a_rational_quadratic_draws_a_circle() {
        let spline = Nurbs::new(
            2,
            vec![0., 0., 0., 1., 1., 1.],
            vec![Point::new(10., 0.), Point::new(10., 10.), Point::new(0., 10.)],
            vec![1., FRAC_1_SQRT_2, 1.],
        )
        .unwrap();
        for k in 0..=100 {
            let p = spline.at(f64::from(k) / 100.);
            assert!((p.norm() - 10.).abs() < 1e-12, "{p:?}");
        }
        assert_eq!(spline.at(1.), Point::new(0., 10.));
    }

    #[test]
    fn a_cubic_passes_through_its_fit_points_and_tangents() {
        let points =
            [Point::new(0., 0.), Point::new(10., 5.), Point::new(20., 0.), Point::new(30., 5.)];
        let cubic = Cubic::through(&points, (Some(Point::new(0., 1.)), None), false).unwrap();
        for (p, &t) in points.iter().zip(&cubic.params) {
            assert!(cubic.at(t).distance(*p) < 1e-9);
        }
        let d = cubic.at(1e-6) - cubic.at(0.);
        assert!(d.x.abs() < 1e-6 * 1e-3, "{d:?}");
        let closed = Cubic::through(&points, (None, None), true).unwrap();
        assert!(closed.at(closed.from).distance(closed.at(closed.to)) < 1e-9);
    }

    #[test]
    fn bad_splines_say_why() {
        let controls = vec![Point::ORIGIN, Point::new(1., 0.), Point::new(2., 1.)];
        assert!(Nurbs::new(3, vec![0.; 7], controls.clone(), vec![1.; 3]).is_err());
        assert!(
            Nurbs::new(2, vec![0., 0., 0., 0., 0., 0.], controls.clone(), vec![1.; 3]).is_err()
        );
        assert!(Nurbs::new(2, vec![0., 0., 0., 1., 1., 1.], controls, vec![1., 0., 1.]).is_err());
    }
}
