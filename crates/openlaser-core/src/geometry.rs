// SPDX-License-Identifier: GPL-3.0-or-later

//! Planar geometry in millimetres: points, lines and arcs, the contours they
//! chain into, and a drawing of contours.
//!
//! Arcs are exact. An arc stays an arc through import, preparation and
//! compilation, so a circle is cut as a circle. Angles are radians,
//! counterclockwise positive; a negative sweep runs clockwise.

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;
use std::ops::{Add, Mul, Neg, Sub};

/// The distance within which two endpoints count as the same point.
pub const CONTINUITY_TOLERANCE: f64 = 1e-6;

/// The largest retained placement. Editing and persistence share this limit;
/// preparation may impose a smaller budget on the active cutting geometry.
pub const MAX_PLACED_CONTOURS: usize = 100_000;

/// A point, or a displacement, in millimetres.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// The X coordinate.
    pub x: f64,
    /// The Y coordinate.
    pub y: f64,
}

impl Point {
    /// The origin.
    pub const ORIGIN: Self = Self { x: 0., y: 0. };

    /// A point at `x`, `y`.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The unit vector at `angle` radians from the X axis.
    #[must_use]
    pub fn direction(angle: f64) -> Self {
        Self { x: angle.cos(), y: angle.sin() }
    }

    /// The distance to `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    /// The point `t` of the way to `other`.
    #[must_use]
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self { x: self.x + (other.x - self.x) * t, y: self.y + (other.y - self.y) * t }
    }

    /// The length as a displacement.
    #[must_use]
    pub fn norm(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The Z component of the cross product.
    #[must_use]
    pub fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// The angle from the X axis, in radians.
    #[must_use]
    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }

    /// This displacement turned a quarter turn counterclockwise.
    #[must_use]
    pub fn perpendicular(self) -> Self {
        Self { x: -self.y, y: self.x }
    }

    /// This displacement turned by `angle` radians.
    #[must_use]
    pub fn rotated(self, angle: f64) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self { x: self.x * cos - self.y * sin, y: self.x * sin + self.y * cos }
    }

    /// Whether both coordinates are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self { x: self.x + other.x, y: self.y + other.y }
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self { x: self.x - other.x, y: self.y - other.y }
    }
}

impl Mul<f64> for Point {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self { x: self.x * factor, y: self.y * factor }
    }
}

impl Neg for Point {
    type Output = Self;
    fn neg(self) -> Self {
        Self { x: -self.x, y: -self.y }
    }
}

impl From<[f64; 2]> for Point {
    fn from([x, y]: [f64; 2]) -> Self {
        Self { x, y }
    }
}

impl From<Point> for [f64; 2] {
    fn from(point: Point) -> Self {
        [point.x, point.y]
    }
}

/// A uniform similarity transform as an affine matrix `[a, b, c, d, e, f]`:
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`. Rotation, mirroring and
/// translation and uniform scale, so circles remain circles.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform(pub [f64; 6]);

/// A contour of a drawing on the sheet: which one, which copy of the
/// drawing it belongs to, and where it lies.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    /// The drawing contour.
    pub source: usize,
    /// Which copy of the drawing; the drawing itself is 0.
    pub copy: u32,
    /// Where it lies.
    pub transform: Transform,
}

impl Placed {
    /// Checks placement size, source references, transforms and unique physical
    /// instance identities before editing or restoring a retained layout.
    pub fn validate_all(placed: &[Self], sources: usize) -> Result<(), String> {
        if !(1..=MAX_PLACED_CONTOURS).contains(&placed.len()) {
            return Err(format!("a placement must contain 1 to {MAX_PLACED_CONTOURS} contours"));
        }
        let mut instances = std::collections::BTreeSet::new();
        for p in placed {
            if !p.is_valid(sources) || !instances.insert((p.source, p.copy)) {
                return Err(
                    "the placement has an invalid source, transform or duplicate instance".into()
                );
            }
        }
        Ok(())
    }

    fn is_valid(&self, sources: usize) -> bool {
        self.source < sources && self.transform.is_similarity()
    }

    /// The contour where the drawing has it.
    #[must_use]
    pub const fn drawn(source: usize) -> Self {
        Self { source, copy: 0, transform: Transform::IDENTITY }
    }

    /// Every contour of a drawing of `count` contours, as drawn.
    #[must_use]
    pub fn all(count: usize) -> Vec<Self> {
        (0..count).map(Self::drawn).collect()
    }
}

#[allow(clippy::many_single_char_names, reason = "the affine matrix's conventional names")]
impl Transform {
    /// Leaves everything where it is.
    pub const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);

    /// A move by `delta`.
    #[must_use]
    pub const fn translation(delta: Point) -> Self {
        Self([1., 0., 0., 1., delta.x, delta.y])
    }

    /// Whether the matrix is finite and rigid: its axes are unit length and
    /// perpendicular.
    #[must_use]
    pub fn is_rigid(&self) -> bool {
        let [a, b, c, d, e, f] = self.0;
        [a, b, c, d, e, f].iter().all(|v| v.is_finite())
            && (a * a + b * b - 1.).abs() < 1e-9
            && (c * c + d * d - 1.).abs() < 1e-9
            && (a * c + b * d).abs() < 1e-9
    }

    /// Uniform scale; valid similarities have equal perpendicular axes.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.0[0].hypot(self.0[1])
    }

    /// Whether this finite matrix preserves angles and has a usable scale.
    /// Nonuniform resizing and shear cannot preserve analytic circular arcs.
    #[must_use]
    pub fn is_similarity(&self) -> bool {
        let [a, b, c, d, e, f] = self.0;
        let squared = a * a + b * b;
        [a, b, c, d, e, f].iter().all(|v| v.is_finite())
            && (1e-12..=1e12).contains(&squared)
            && (c * c + d * d - squared).abs() <= squared * 1e-9
            && (a * c + b * d).abs() <= squared * 1e-9
    }

    /// Whether it mirrors, so arcs sweep the other way through it.
    #[must_use]
    pub fn mirrors(&self) -> bool {
        let [a, b, c, d, ..] = self.0;
        a * d - b * c < 0.
    }

    /// The point under the transform.
    #[must_use]
    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f }
    }

    /// This transform applied after `first`.
    #[must_use]
    pub fn after(&self, first: &Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = first.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }

    /// The inverse of a valid similarity, including its uniform scale.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let squared = a * a + b * b;
        Self([
            a / squared,
            c / squared,
            b / squared,
            d / squared,
            -(a * e + b * f) / squared,
            -(c * e + d * f) / squared,
        ])
    }

    /// The curve under the transform: an arc scales its radius, and its
    /// sweep reverses when the transform mirrors.
    #[must_use]
    pub fn curve(&self, curve: &Curve) -> Curve {
        match *curve {
            Curve::Line { start, end } => {
                Curve::Line { start: self.apply(start), end: self.apply(end) }
            }
            Curve::Arc { center, radius, start_angle, sweep } => {
                let [a, b, c, d, ..] = self.0;
                let (sin, cos) = start_angle.sin_cos();
                let start_angle = (b * cos + d * sin).atan2(a * cos + c * sin);
                let sweep = if self.mirrors() { -sweep } else { sweep };
                Curve::Arc {
                    center: self.apply(center),
                    radius: radius * self.scale(),
                    start_angle,
                    sweep,
                }
            }
        }
    }

    /// The contour under the transform.
    #[must_use]
    pub fn contour(&self, contour: &Contour) -> Contour {
        Contour {
            layer: contour.layer.clone(),
            curves: contour.curves.iter().map(|c| self.curve(c)).collect(),
        }
    }
}

/// An axis-aligned box.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// The lowest X and Y.
    pub min: Point,
    /// The highest X and Y.
    pub max: Point,
}

impl Bounds {
    /// The box around one point.
    #[must_use]
    pub const fn of(point: Point) -> Self {
        Self { min: point, max: point }
    }

    /// Grows the box to hold `point`.
    pub fn include(&mut self, point: Point) {
        self.min = Point::new(self.min.x.min(point.x), self.min.y.min(point.y));
        self.max = Point::new(self.max.x.max(point.x), self.max.y.max(point.y));
    }

    /// The box around both.
    #[must_use]
    pub fn union(mut self, other: Self) -> Self {
        self.include(other.min);
        self.include(other.max);
        self
    }

    /// The box moved by `delta`.
    #[must_use]
    pub fn translated(&self, delta: Point) -> Self {
        Self {
            min: Point::new(self.min.x + delta.x, self.min.y + delta.y),
            max: Point::new(self.max.x + delta.x, self.max.y + delta.y),
        }
    }

    /// The width.
    #[must_use]
    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    /// The height.
    #[must_use]
    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    /// The larger of width and height.
    #[must_use]
    pub fn extent(&self) -> f64 {
        self.width().max(self.height())
    }

    /// The middle of the box.
    #[must_use]
    pub fn center(&self) -> Point {
        self.min.lerp(self.max, 0.5)
    }
}

/// A line or an arc.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Curve {
    /// A straight line.
    Line {
        /// Where it starts.
        start: Point,
        /// Where it ends.
        end: Point,
    },
    /// An arc about `center`, from `start_angle` through `sweep`.
    Arc {
        /// The centre.
        center: Point,
        /// The radius.
        radius: f64,
        /// The angle of the start point, in radians.
        start_angle: f64,
        /// The signed angle swept, in radians; negative is clockwise.
        sweep: f64,
    },
}

impl Curve {
    /// A full circle, starting at its rightmost point and running
    /// counterclockwise.
    #[must_use]
    pub const fn circle(center: Point, radius: f64) -> Self {
        Self::Arc { center, radius, start_angle: 0., sweep: TAU }
    }

    /// The point at parameter `t`, `0..=1` along the curve.
    #[must_use]
    pub fn point(&self, t: f64) -> Point {
        match *self {
            Self::Line { start, end } => start.lerp(end, t),
            Self::Arc { center, radius, start_angle, sweep } => {
                center + Point::direction(start_angle + sweep * t) * radius
            }
        }
    }

    /// Where the curve starts.
    #[must_use]
    pub fn start(&self) -> Point {
        self.point(0.)
    }

    /// Where the curve ends.
    #[must_use]
    pub fn end(&self) -> Point {
        self.point(1.)
    }

    /// The unit direction of travel at parameter `t`.
    #[must_use]
    pub fn tangent(&self, t: f64) -> Point {
        match *self {
            Self::Line { start, end } => (end - start) * (1. / start.distance(end)),
            Self::Arc { start_angle, sweep, .. } => {
                Point::direction(start_angle + sweep * t).perpendicular() * sweep.signum()
            }
        }
    }

    /// The length.
    #[must_use]
    pub fn length(&self) -> f64 {
        match *self {
            Self::Line { start, end } => start.distance(end),
            Self::Arc { radius, sweep, .. } => radius * sweep.abs(),
        }
    }

    /// The exact extent, including the cardinal points an arc passes.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        let mut bounds = Bounds::of(self.start());
        bounds.include(self.end());
        if let Self::Arc { center, radius, start_angle, sweep } = *self {
            for quarter in 0..4 {
                let angle = f64::from(quarter) * TAU / 4.;
                let travelled = if sweep >= 0. { angle - start_angle } else { start_angle - angle }
                    .rem_euclid(TAU);
                if sweep.abs() >= TAU || travelled <= sweep.abs() {
                    bounds.include(center + Point::direction(angle) * radius);
                }
            }
        }
        bounds
    }

    /// The same curve travelled the other way.
    #[must_use]
    pub fn reversed(&self) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line { start: end, end: start },
            Self::Arc { center, radius, start_angle, sweep } => {
                Self::Arc { center, radius, start_angle: start_angle + sweep, sweep: -sweep }
            }
        }
    }

    /// The part between parameters `from` and `to`.
    #[must_use]
    pub fn slice(&self, from: f64, to: f64) -> Self {
        match *self {
            Self::Line { .. } => Self::Line { start: self.point(from), end: self.point(to) },
            Self::Arc { center, radius, start_angle, sweep } => Self::Arc {
                center,
                radius,
                start_angle: start_angle + sweep * from,
                sweep: sweep * (to - from),
            },
        }
    }

    /// The curve moved by `delta`.
    #[must_use]
    pub fn translated(&self, delta: Point) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line { start: start + delta, end: end + delta },
            Self::Arc { center, radius, start_angle, sweep } => {
                Self::Arc { center: center + delta, radius, start_angle, sweep }
            }
        }
    }

    /// The curve scaled about the origin by a positive `factor`.
    #[must_use]
    pub fn scaled(&self, factor: f64) -> Self {
        match *self {
            Self::Line { start, end } => Self::Line { start: start * factor, end: end * factor },
            Self::Arc { center, radius, start_angle, sweep } => {
                Self::Arc { center: center * factor, radius: radius * factor, start_angle, sweep }
            }
        }
    }

    /// Whether the curve is finite, has length, and, for an arc, a positive
    /// radius and at most one full turn.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let finite = self.start().is_finite() && self.end().is_finite();
        let length = self.length();
        let shape = match *self {
            Self::Line { .. } => true,
            Self::Arc { radius, sweep, .. } => radius > 0. && sweep.abs() <= TAU + 1e-9,
        };
        finite && length.is_finite() && length > 1e-9 && shape
    }
}

/// A chain of curves on one drawing layer.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contour {
    /// The drawing layer the contour came from.
    pub layer: String,
    /// The curves, each starting where the previous one ends.
    pub curves: Vec<Curve>,
}

impl Contour {
    /// Where the contour starts.
    #[must_use]
    pub fn start(&self) -> Option<Point> {
        self.curves.first().map(Curve::start)
    }

    /// Where the contour ends.
    #[must_use]
    pub fn end(&self) -> Option<Point> {
        self.curves.last().map(Curve::end)
    }

    /// The total length.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.curves.iter().map(Curve::length).sum()
    }

    /// The extent, `None` for an empty contour.
    #[must_use]
    pub fn bounds(&self) -> Option<Bounds> {
        self.curves.iter().map(Curve::bounds).reduce(Bounds::union)
    }

    /// Whether every curve starts where the previous one ends.
    #[must_use]
    pub fn is_continuous(&self) -> bool {
        self.curves
            .windows(2)
            .all(|pair| pair[0].end().distance(pair[1].start()) <= CONTINUITY_TOLERANCE)
    }

    /// Whether the contour is continuous and ends where it starts.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match (self.start(), self.end()) {
            (Some(start), Some(end)) => {
                self.is_continuous() && start.distance(end) <= CONTINUITY_TOLERANCE
            }
            _ => false,
        }
    }

    /// The enclosed area of a closed contour, positive when it runs
    /// counterclockwise.
    #[must_use]
    pub fn signed_area(&self) -> f64 {
        self.curves
            .iter()
            .map(|curve| match *curve {
                Curve::Line { start, end } => 0.5 * start.cross(end),
                Curve::Arc { center, radius, start_angle, sweep } => {
                    let end_angle = start_angle + sweep;
                    0.5 * radius
                        * (center.x * (end_angle.sin() - start_angle.sin())
                            - center.y * (end_angle.cos() - start_angle.cos())
                            + radius * sweep)
                }
            })
            .sum()
    }

    /// Whether the contour is a single full circle, whatever arcs it is
    /// drawn with.
    #[must_use]
    pub fn is_circle(&self) -> bool {
        let Some(&Curve::Arc { center, radius, sweep, .. }) = self.curves.first() else {
            return false;
        };
        if !self.is_closed() {
            return false;
        }
        let mut total = 0.;
        for curve in &self.curves {
            let Curve::Arc { center: c, radius: r, sweep: s, .. } = *curve else { return false };
            if c.distance(center) > CONTINUITY_TOLERANCE
                || (r - radius).abs() > CONTINUITY_TOLERANCE
                || s.signum() != sweep.signum()
            {
                return false;
            }
            total += s;
        }
        (total.abs() - TAU).abs() <= 1e-9
    }

    /// The contour travelled the other way.
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            layer: self.layer.clone(),
            curves: self.curves.iter().rev().map(Curve::reversed).collect(),
        }
    }

    /// The contour moved by `delta`.
    #[must_use]
    pub fn translated(&self, delta: Point) -> Self {
        Self {
            layer: self.layer.clone(),
            curves: self.curves.iter().map(|c| c.translated(delta)).collect(),
        }
    }

    /// The point `distance` along the contour, `None` beyond its end.
    #[must_use]
    pub fn point_at(&self, distance: f64) -> Option<Point> {
        let mut remaining = distance;
        for curve in &self.curves {
            let length = curve.length();
            if remaining <= length {
                return Some(curve.point(remaining / length));
            }
            remaining -= length;
        }
        None
    }

    /// A closed contour re-chained to start `distance` along it, splitting
    /// the curve that point falls in. An open contour is returned as is.
    #[must_use]
    pub fn started_at(&self, distance: f64) -> Self {
        if !self.is_closed() {
            return self.clone();
        }
        let mut remaining = distance.rem_euclid(self.length());
        for (index, curve) in self.curves.iter().enumerate() {
            let length = curve.length();
            if remaining < length {
                let fraction = remaining / length;
                let mut curves = Vec::with_capacity(self.curves.len() + 1);
                if fraction > 0. {
                    curves.push(curve.slice(fraction, 1.));
                } else {
                    curves.push(*curve);
                }
                curves.extend_from_slice(&self.curves[index + 1..]);
                curves.extend_from_slice(&self.curves[..index]);
                if fraction > 0. {
                    curves.push(curve.slice(0., fraction));
                }
                return Self { layer: self.layer.clone(), curves };
            }
            remaining -= length;
        }
        self.clone()
    }
}

/// The contours of one drawing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    /// The contours, in drawing order.
    pub contours: Vec<Contour>,
}

impl Drawing {
    /// The extent of every contour, `None` when there is no geometry.
    #[must_use]
    pub fn bounds(&self) -> Option<Bounds> {
        self.contours.iter().filter_map(Contour::bounds).reduce(Bounds::union)
    }

    /// The total length of every contour.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.contours.iter().map(Contour::length).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter turn about the origin then a mirror keep lengths, compose
    /// in order, invert back, and turn an arc's sweep the other way.
    #[test]
    fn a_rigid_transform_keeps_lengths_and_inverts() {
        let turn = Transform([0., 1., -1., 0., 10., 0.]);
        let mirror = Transform([-1., 0., 0., 1., 0., 0.]);
        assert!(turn.is_rigid() && mirror.is_rigid() && !turn.mirrors() && mirror.mirrors());
        assert!(!Transform([2., 0., 0., 2., 0., 0.]).is_rigid());
        let p = Point { x: 3., y: 4. };
        let both = mirror.after(&turn);
        assert!(both.apply(p).distance(mirror.apply(turn.apply(p))) < 1e-12);
        assert!(both.inverse().apply(both.apply(p)).distance(p) < 1e-12);
        let arc = Curve::Arc {
            center: Point { x: 5., y: 0. },
            radius: 5.,
            start_angle: 0.,
            sweep: TAU / 2.,
        };
        let mirrored = mirror.curve(&arc);
        assert!(mirrored.start().distance(Point { x: -10., y: 0. }) < 1e-9);
        assert!(mirrored.end().distance(Point { x: 0., y: 0. }) < 1e-9);
        assert!((mirrored.length() - arc.length()).abs() < 1e-9);
        let turned = turn.curve(&arc);
        assert!(turned.start().distance(turn.apply(arc.start())) < 1e-9);
    }
    use proptest::prelude::*;

    fn square() -> Contour {
        let corners =
            [Point::new(0., 0.), Point::new(10., 0.), Point::new(10., 10.), Point::new(0., 10.)];
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: corners[i], end: corners[(i + 1) % 4] })
                .collect(),
        }
    }

    /// An arc's box includes the cardinal points it passes, for both
    /// directions and across the zero angle; a full circle includes all four.
    #[test]
    fn arc_bounds_include_the_cardinal_points_passed() {
        for sweep in [TAU / 3., -TAU / 3.] {
            let arc = Curve::Arc {
                center: Point::new(5., 8.),
                radius: 10.,
                start_angle: -sweep / 2.,
                sweep,
            };
            let bounds = arc.bounds();
            assert_eq!(bounds.max.x, 15.);
            assert!((bounds.min.x - 10.).abs() < 1e-12);
            assert!(bounds.min.y < 0. && bounds.max.y > 16.);
            assert_eq!(arc.reversed().bounds(), bounds);
        }
        let circle =
            Curve::Arc { center: Point::new(5., 8.), radius: 10., start_angle: 0.17, sweep: -TAU };
        assert_eq!(
            circle.bounds(),
            Bounds { min: Point::new(-5., -2.), max: Point::new(15., 18.) }
        );
    }

    /// The signed area is positive counterclockwise, negative when
    /// reversed, and exact for a circle drawn as two arcs.
    #[test]
    fn signed_area_follows_the_winding() {
        assert_eq!(square().signed_area(), 100.);
        assert_eq!(square().reversed().signed_area(), -100.);
        let circle = Contour {
            layer: "0".into(),
            curves: vec![
                Curve::Arc {
                    center: Point::new(3., 4.),
                    radius: 2.,
                    start_angle: 0.,
                    sweep: TAU / 2.,
                },
                Curve::Arc {
                    center: Point::new(3., 4.),
                    radius: 2.,
                    start_angle: TAU / 2.,
                    sweep: TAU / 2.,
                },
            ],
        };
        assert!((circle.signed_area() - TAU * 2.).abs() < 1e-12);
        assert!(circle.is_circle() && circle.is_closed());
        assert!(!square().is_circle());
    }

    /// Restarting a closed contour keeps its length and geometry and puts
    /// the new start where asked; an open contour is left alone.
    #[test]
    fn restarting_a_closed_contour_keeps_its_geometry() {
        let restarted = square().started_at(15.);
        assert_eq!(restarted.curves.len(), 5);
        assert_eq!(restarted.start(), Some(Point::new(10., 5.)));
        assert!((restarted.length() - 40.).abs() < 1e-12);
        assert!(restarted.is_closed());
        assert_eq!(square().started_at(10.).curves.len(), 4);
        assert_eq!(square().started_at(10.).start(), Some(Point::new(10., 0.)));
        let open = Contour {
            layer: "0".into(),
            curves: vec![Curve::Line { start: Point::ORIGIN, end: Point::new(4., 0.) }],
        };
        assert_eq!(open.started_at(2.), open);
        assert_eq!(open.point_at(1.), Some(Point::new(1., 0.)));
        assert_eq!(open.point_at(5.), None);
    }

    /// Tangents point along the direction of travel: along a line, and
    /// perpendicular to the radius on an arc, flipping with the sweep sign.
    #[test]
    fn tangents_follow_the_direction_of_travel() {
        let line = Curve::Line { start: Point::ORIGIN, end: Point::new(0., 3.) };
        assert_eq!(line.tangent(0.5), Point::new(0., 1.));
        let arc =
            Curve::Arc { center: Point::ORIGIN, radius: 2., start_angle: 0., sweep: TAU / 4. };
        let t = arc.tangent(0.);
        assert!((t.x).abs() < 1e-12 && (t.y - 1.).abs() < 1e-12);
        let back = arc.reversed().tangent(1.);
        assert!((back.x).abs() < 1e-12 && (back.y + 1.).abs() < 1e-12);
        assert!(!Curve::Line { start: Point::ORIGIN, end: Point::ORIGIN }.is_valid());
        assert!(
            !Curve::Arc { center: Point::ORIGIN, radius: -1., start_angle: 0., sweep: 1. }
                .is_valid()
        );
    }

    /// Curves serialise with a type tag the UI can switch on.
    #[test]
    fn curves_serialise_with_a_type_tag() {
        let json = serde_json::to_string(&Curve::circle(Point::new(1., 2.), 3.)).unwrap();
        assert!(json.starts_with("{\"type\":\"arc\""));
        let back: Curve = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Curve::circle(Point::new(1., 2.), 3.));
    }

    fn arcs() -> impl Strategy<Value = Curve> {
        (-50f64..50., -50f64..50., 0.01f64..40., -TAU..TAU, -TAU..TAU).prop_filter_map(
            "an arc needs a sweep",
            |(x, y, radius, start_angle, sweep)| {
                (sweep.abs() > 1e-3).then_some(Curve::Arc {
                    center: Point::new(x, y),
                    radius,
                    start_angle,
                    sweep,
                })
            },
        )
    }

    proptest! {
        /// Reversing twice gives the curve back, reversal keeps length and
        /// bounds, and a slice covers the distance its parameters span.
        #[test]
        fn reversal_and_slicing_are_consistent(arc in arcs(), from in 0f64..1., to in 0f64..1.) {
            let back = arc.reversed().reversed();
            prop_assert!(back.start().distance(arc.start()) < 1e-9);
            prop_assert!(back.end().distance(arc.end()) < 1e-9);
            prop_assert!((arc.reversed().length() - arc.length()).abs() < 1e-9);
            let (a, b) = (from.min(to), from.max(to));
            let slice = arc.slice(a, b);
            prop_assert!((slice.length() - arc.length() * (b - a)).abs() < 1e-9);
            prop_assert!(slice.start().distance(arc.point(a)) < 1e-9);
            let bounds = arc.bounds();
            prop_assert!(bounds.min.x <= arc.point(from).x + 1e-9 && arc.point(from).x <= bounds.max.x + 1e-9);
            prop_assert!(bounds.min.y <= arc.point(from).y + 1e-9 && arc.point(from).y <= bounds.max.y + 1e-9);
        }
    }
}
