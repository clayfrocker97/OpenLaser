// SPDX-License-Identifier: GPL-3.0-or-later

//! Bridges: a channel cut between two picked points joins two closed
//! contours into one, or splits one contour in two, so the parts cut in
//! one pass without a second pierce.

use crate::{EPS, Error, Result, topology};
use openlaser_core::features::{Bridge, Bridges, Pick};
use openlaser_core::geometry::{Contour, Curve, Point};
use std::f64::consts::TAU;

/// The most bridges one drawing may hold.
const MAX_BRIDGES: usize = 1000;

/// A contour and the drawing contours it was made from.
pub(crate) struct Sourced {
    pub sources: Vec<usize>,
    pub contour: Contour,
}

/// Applies the bridges in order to the contours, given with their drawing
/// indices. Each pick names a contour as the list stood after the bridges
/// before it; the result takes the place of the earlier of the two.
pub(crate) fn apply(
    contours: &[(usize, &Contour)],
    bridges: Option<&Bridges>,
) -> Result<Vec<Sourced>> {
    resolve(contours, bridges, |_, point| Ok(point)).map(|(list, _)| list)
}

/// Resolve local picks while the contour list evolves. Every result names
/// the original placed contours, including after joins and splits.
pub(crate) fn resolve(
    contours: &[(usize, &Contour)],
    bridges: Option<&Bridges>,
    transform: impl Fn(usize, Point) -> Result<Point>,
) -> Result<(Vec<Sourced>, Vec<Bridge>)> {
    let mut list: Vec<Sourced> = contours
        .iter()
        .map(|(index, contour)| Sourced { sources: vec![*index], contour: (*contour).clone() })
        .collect();
    let mut placed = Vec::new();
    let Some(bridges) = bridges else { return Ok((list, placed)) };
    if bridges.connections.len() > MAX_BRIDGES {
        return Err(Error::Budget("bridge count"));
    }
    for (number, bridge) in bridges.connections.iter().enumerate() {
        let index = number + 1;
        let fail = |reason: String| Error::Bridge { index, reason };
        let (mut a, mut b) = (bridge.first.contour, bridge.second.contour);
        if a >= list.len() || b >= list.len() {
            return Err(fail("refers to a contour that is not there".into()));
        }
        let point = |pick: Pick| transform(list[pick.contour].sources[0], pick.point);
        let (mut pick_a, mut pick_b) = (point(bridge.first)?, point(bridge.second)?);
        placed.push(Bridge {
            first: Pick { point: pick_a, ..bridge.first },
            second: Pick { point: pick_b, ..bridge.second },
        });
        if a > b {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut pick_a, &mut pick_b);
        }
        let second = (a != b).then(|| &list[b].contour);
        let pieces =
            splice(&list[a].contour, second, [pick_a, pick_b], bridges.width.0).map_err(fail)?;
        let mut sources = list[a].sources.clone();
        if a != b {
            sources.extend(list[b].sources.iter().copied());
            list.remove(b);
        }
        sources.sort_unstable();
        sources.dedup();
        let results =
            pieces.into_iter().map(|contour| Sourced { sources: sources.clone(), contour });
        list.splice(a..=a, results);
    }
    Ok((list, placed))
}

/// The nearest point on `contour` to `point`, and how far along the
/// contour it lies as a fraction of the length. What a pick on the drawing
/// snaps to.
#[must_use]
pub fn project(contour: &Contour, point: Point) -> Option<(Point, f64)> {
    let total = contour.length();
    if !total.is_finite() || total <= 0. {
        return None;
    }
    let mut best: Option<(f64, Point, f64)> = None;
    let mut offset = 0.;
    for curve in &contour.curves {
        let t = nearest_parameter(curve, point);
        let candidate = curve.point(t);
        let distance = candidate.distance(point);
        if best.is_none_or(|(closest, _, _)| distance < closest) {
            best = Some((distance, candidate, offset + t * curve.length()));
        }
        offset += curve.length();
    }
    best.map(|(_, candidate, along)| (candidate, along / total))
}

/// The parameter of the point on `curve` nearest to `point`.
pub(crate) fn nearest_parameter(curve: &Curve, point: Point) -> f64 {
    match *curve {
        Curve::Line { start, end } => {
            let direction = end - start;
            ((point - start).dot(direction) / direction.dot(direction)).clamp(0., 1.)
        }
        Curve::Arc { center, radius, .. } => {
            let radial = point - center;
            let on_circle = if radial.norm() > EPS {
                center + radial * (radius / radial.norm())
            } else {
                curve.start()
            };
            arc_parameter(curve, on_circle).unwrap_or_else(|| {
                if point.distance(curve.start()) <= point.distance(curve.end()) { 0. } else { 1. }
            })
        }
    }
}

/// The parameter of `point`, which lies on the arc's circle, if it lies
/// within the arc.
fn arc_parameter(curve: &Curve, point: Point) -> Option<f64> {
    let Curve::Arc { center, start_angle, sweep, .. } = *curve else { return None };
    let angle = (point - center).angle();
    let base = ((start_angle - angle) / TAU).floor();
    (-2..=2)
        .filter_map(|n| {
            let t = (angle + (base + f64::from(n)) * TAU - start_angle) / sweep;
            (-EPS..=1. + EPS).contains(&t).then_some(t.clamp(0., 1.))
        })
        .min_by(f64::total_cmp)
}

/// A place where a channel edge crosses a contour.
#[derive(Clone, Copy)]
struct Hit {
    point: Point,
    fraction: f64,
}

/// A closed contour of sound, connected curves.
fn checked(contour: &Contour) -> std::result::Result<(), String> {
    if contour.curves.is_empty() || !contour.is_closed() {
        return Err("needs closed contours".into());
    }
    if contour.curves.iter().any(|c| !c.is_valid()) {
        return Err("a contour has degenerate geometry".into());
    }
    Ok(())
}

/// How far along `contour` the pick lies; it must lie on the contour.
fn picked_fraction(contour: &Contour, pick: Point) -> std::result::Result<f64, String> {
    if !pick.is_finite() {
        return Err("a pick is not a point".into());
    }
    let total = contour.length();
    let mut offset = 0.;
    for curve in &contour.curves {
        let t = nearest_parameter(curve, pick);
        if curve.point(t).distance(pick) <= EPS {
            return Ok(((offset + t * curve.length()) / total).rem_euclid(1.));
        }
        offset += curve.length();
    }
    Err("a pick is not on its contour".into())
}

/// Where the segment `line` crosses `contour`, in contour order.
fn hits(
    contour: &Contour,
    line: [Point; 2],
    tolerance: f64,
) -> std::result::Result<Vec<Hit>, String> {
    let intersection = Intersection::new(line);
    let total = contour.length();
    let mut result: Vec<Hit> = Vec::new();
    let mut offset = 0.;
    for curve in &contour.curves {
        let mut candidates = intersection.parameters(curve)?;
        candidates.sort_by(f64::total_cmp);
        for t in candidates {
            let point = curve.point(t);
            if result.last().is_none_or(|h| {
                (h.point.x - point.x).abs() >= tolerance || (h.point.y - point.y).abs() >= tolerance
            }) {
                result.push(Hit {
                    point,
                    fraction: ((offset + t * curve.length()) / total).rem_euclid(1.),
                });
            }
        }
        offset += curve.length();
    }
    if result.len() > 1 && result[0].point.distance(result[result.len() - 1].point) < tolerance {
        result.pop();
    }
    Ok(result)
}

struct Intersection {
    start: Point,
    direction: Point,
    squared: f64,
}

impl Intersection {
    fn new(line: [Point; 2]) -> Self {
        let direction = line[1] - line[0];
        Self { start: line[0], direction, squared: direction.dot(direction) }
    }

    fn parameters(&self, curve: &Curve) -> std::result::Result<Vec<f64>, String> {
        let mut candidates = Vec::new();
        match *curve {
            Curve::Line { start, end } => self.line(start, end, &mut candidates)?,
            Curve::Arc { center, radius, .. } => self.arc(curve, center, radius, &mut candidates),
        }
        Ok(candidates)
    }

    fn line(
        &self,
        start: Point,
        end: Point,
        candidates: &mut Vec<f64>,
    ) -> std::result::Result<(), String> {
        let edge = end - start;
        let denominator = self.direction.cross(edge);
        let delta = start - self.start;
        if denominator.abs() <= EPS * self.direction.norm() * edge.norm() {
            if delta.cross(self.direction).abs() <= EPS * self.direction.norm() {
                let a = delta.dot(self.direction) / self.squared;
                let b = (end - self.start).dot(self.direction) / self.squared;
                if a.min(b) <= 1. && a.max(b) >= 0. {
                    return Err("a channel edge runs along a contour edge".into());
                }
            }
        } else {
            let along = delta.cross(edge) / denominator;
            let t = delta.cross(self.direction) / denominator;
            if (-EPS..=1. + EPS).contains(&along) && (-EPS..=1. + EPS).contains(&t) {
                candidates.push(t.clamp(0., 1.));
            }
        }
        Ok(())
    }

    fn arc(&self, curve: &Curve, center: Point, radius: f64, candidates: &mut Vec<f64>) {
        let v = self.start - center;
        let b = 2. * v.dot(self.direction);
        let c = v.dot(v) - radius * radius;
        let discriminant = b * b - 4. * self.squared * c;
        if discriminant >= 0. {
            let root = discriminant.sqrt();
            for along in [(-b - root) / (2. * self.squared), (-b + root) / (2. * self.squared)] {
                if (-EPS..=1. + EPS).contains(&along)
                    && let Some(t) =
                        arc_parameter(curve, self.start + self.direction * along.clamp(0., 1.))
                {
                    candidates.push(t);
                }
            }
        }
    }
}

/// The part of `contour` from fraction `begin` forward to fraction `end`,
/// wrapping past the start.
fn interval(contour: &Contour, begin: f64, end: f64) -> Vec<Curve> {
    let total = contour.length();
    let start = begin.rem_euclid(1.) * total;
    let finish = start + (end - begin).rem_euclid(1.) * total;
    let mut curves = Vec::new();
    for cycle in 0..2 {
        let mut offset = f64::from(cycle) * total;
        for curve in &contour.curves {
            let length = curve.length();
            let a = start.max(offset);
            let b = finish.min(offset + length);
            if b - a > EPS {
                curves.push(curve.slice((a - offset) / length, (b - offset) / length));
            }
            offset += length;
        }
    }
    curves
}

fn reverse(curves: &mut [Curve]) {
    curves.reverse();
    for curve in curves.iter_mut() {
        *curve = curve.reversed();
    }
}

fn connector(start: Point, end: Point) -> std::result::Result<Curve, String> {
    if start.distance(end) <= EPS {
        return Err("a channel edge collapses to a point".into());
    }
    Ok(Curve::Line { start, end })
}

/// Cuts a channel `width` wide along the line between the picks. With a
/// `second` contour the two become one; without, the first splits in two.
pub(crate) fn splice(
    first: &Contour,
    second: Option<&Contour>,
    picks: [Point; 2],
    width: f64,
) -> std::result::Result<Vec<Contour>, String> {
    checked(first)?;
    if let Some(second) = second {
        checked(second)?;
    }
    let first_fraction = picked_fraction(first, picks[0])?;
    picked_fraction(second.unwrap_or(first), picks[1])?;
    let axis = picks[1] - picks[0];
    let distance = axis.norm();
    if distance < EPS {
        return Err("the picks must be two different points".into());
    }
    let unit = axis * (1. / distance);
    let normal = unit.perpendicular();
    // The channel's edges run parallel to the pick axis, half a width to
    // each side, and reach one width past each pick.
    let edges = [1., -1.].map(|side| {
        [
            picks[0] - unit * width + normal * (side * width / 2.),
            picks[1] + unit * width + normal * (side * width / 2.),
        ]
    });
    let extent = |c: &Contour| c.bounds().map_or(0., |b| b.extent());
    let tolerance = (extent(first).min(extent(second.unwrap_or(first))) * 0.1).clamp(EPS, 0.03);
    let result = match second {
        Some(second) => vec![joined(first, second, edges, tolerance)?],
        None => halved(first, first_fraction, edges, tolerance, width)?,
    };
    for contour in &result {
        checked(contour)?;
    }
    topology::depths(&result.iter().collect::<Vec<_>>())
        .map_err(|e| format!("the bridged geometry is invalid: {e}"))?;
    Ok(result)
}

/// Two contours joined through the channel between `edges`: what stays of
/// each, the long way round between its two crossings, connected end to
/// end.
fn joined(
    first: &Contour,
    second: &Contour,
    edges: [[Point; 2]; 2],
    tolerance: f64,
) -> std::result::Result<Contour, String> {
    let choose = |contour: &Contour, edge: [Point; 2]| -> std::result::Result<Hit, String> {
        let middle = edge[0].lerp(edge[1], 0.5);
        let nearness = |h: &Hit| (h.point.x - middle.x).abs() + (h.point.y - middle.y).abs();
        hits(contour, edge, tolerance)?
            .into_iter()
            .min_by(|a, b| nearness(a).total_cmp(&nearness(b)))
            .ok_or_else(|| "a channel edge misses its contour".into())
    };
    let retained = |contour: &Contour| -> std::result::Result<(Vec<Curve>, bool), String> {
        let plus = choose(contour, edges[0])?;
        let minus = choose(contour, edges[1])?;
        let delta = (minus.fraction - plus.fraction).rem_euclid(1.);
        if delta > 0.4 && delta < 0.6 {
            return Err("the channel would remove too much of a contour".into());
        }
        if plus.point.distance(minus.point) <= EPS {
            return Err("the channel opening collapses to a point".into());
        }
        Ok(if delta >= 0.6 {
            (interval(contour, plus.fraction, minus.fraction), false)
        } else {
            (interval(contour, minus.fraction, plus.fraction), true)
        })
    };
    let (mut a, a_flipped) = retained(first)?;
    let (mut b, b_flipped) = retained(second)?;
    if a.is_empty() || b.is_empty() {
        return Err("the channel leaves nothing of a contour".into());
    }
    if a_flipped == b_flipped {
        reverse(&mut b);
    }
    let join = connector(a[a.len() - 1].end(), b[0].start())?;
    let close = connector(b[b.len() - 1].end(), a[0].start())?;
    a.push(join);
    a.extend(b);
    a.push(close);
    Ok(Contour { layer: first.layer.clone(), curves: a })
}

/// One contour split by the channel between `edges` into the two closed
/// pieces on either side of it.
fn halved(
    contour: &Contour,
    first_fraction: f64,
    edges: [[Point; 2]; 2],
    tolerance: f64,
    width: f64,
) -> std::result::Result<Vec<Contour>, String> {
    let mut result = Vec::with_capacity(2);
    for edge in edges {
        let crossings = hits(contour, edge, tolerance)?;
        if crossings.len() < 2 {
            return Err("a channel edge must cross the contour twice".into());
        }
        let nearest = |end: Point| {
            crossings
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.point.distance(end).total_cmp(&b.1.point.distance(end)))
                .map_or(0, |(i, _)| i)
        };
        let (a, b) = (nearest(edge[0]), nearest(edge[1]));
        if a == b {
            return Err("the picks resolve to the same crossing".into());
        }
        let relative = |hit: &Hit| {
            let d = hit.fraction - first_fraction;
            if d < -1e-5 { d + 1. } else { d }
        };
        let mut fractions = [relative(&crossings[a]), relative(&crossings[b])];
        fractions.sort_by(f64::total_cmp);
        if (fractions[1] - fractions[0]) * contour.length() < width {
            return Err("a channel edge spans less than the width".into());
        }
        let mut curves = interval(
            contour,
            (fractions[0] + first_fraction).rem_euclid(1.),
            (fractions[1] + first_fraction).rem_euclid(1.),
        );
        if curves.is_empty() {
            return Err("the channel leaves nothing of the contour".into());
        }
        curves.push(connector(curves[curves.len() - 1].end(), curves[0].start())?);
        result.push(Contour { layer: contour.layer.clone(), curves });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::features::{Bridge, Pick};
    use openlaser_core::units::Millimeters;

    fn square(size: f64, at: Point) -> Contour {
        let corners =
            [at, at + Point::new(size, 0.), at + Point::new(size, size), at + Point::new(0., size)];
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: corners[i], end: corners[(i + 1) % 4] })
                .collect(),
        }
    }

    /// Two squares bridged across the gap between them become one closed
    /// contour whose area is both squares plus the channel; the result
    /// takes the earlier square's place and carries both sources.
    #[test]
    fn two_contours_bridge_into_one() {
        let left = square(10., Point::ORIGIN);
        let right = square(10., Point::new(14., 0.));
        let joined =
            splice(&left, Some(&right), [Point::new(10., 5.), Point::new(14., 5.)], 2.).unwrap();
        assert_eq!(joined.len(), 1);
        assert!(joined[0].is_closed());
        assert!((joined[0].signed_area().abs() - 208.).abs() < 1e-9, "{}", joined[0].signed_area());
        let bridges = Bridges {
            width: Millimeters(2.),
            connections: vec![Bridge {
                first: Pick { contour: 2, point: Point::new(14., 5.) },
                second: Pick { contour: 0, point: Point::new(10., 5.) },
            }],
        };
        let small = square(1., Point::new(50., 50.));
        let applied = apply(&[(0, &left), (1, &small), (2, &right)], Some(&bridges)).unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].sources, vec![0, 2]);
        assert_eq!(applied[1].sources, vec![1]);
    }

    /// A channel across one square splits it into two closed halves.
    #[test]
    fn one_contour_splits_in_two() {
        let halves = splice(
            &square(10., Point::ORIGIN),
            None,
            [Point::new(5., 0.), Point::new(5., 10.)],
            2.,
        )
        .unwrap();
        assert_eq!(halves.len(), 2);
        for half in &halves {
            assert!(half.is_closed());
            assert!((half.signed_area().abs() - 40.).abs() < 1e-9, "{}", half.signed_area());
        }
    }

    /// Picks off the contour, coincident picks and a channel that misses
    /// the other contour are refused with their reasons.
    #[test]
    fn bad_bridges_are_refused() {
        let left = square(10., Point::ORIGIN);
        let right = square(10., Point::new(14., 0.));
        assert!(
            splice(&left, Some(&right), [Point::new(10., 5.), Point::new(15., 5.)], 2.).is_err()
        );
        assert!(
            splice(&left, Some(&right), [Point::new(10., 5.), Point::new(10., 5.)], 2.).is_err()
        );
        assert!(
            splice(&left, Some(&right), [Point::new(10., 5.), Point::new(14., 5.)], 0.).is_err()
        );
        let far = square(10., Point::new(100., 100.));
        assert!(
            splice(&left, Some(&far), [Point::new(10., 5.), Point::new(105., 105.)], 2.).is_err()
        );
    }

    /// Projection snaps a point to the nearest place on the contour and
    /// reports how far around it is.
    #[test]
    fn projection_snaps_to_the_contour() {
        let (point, fraction) = project(&square(10., Point::ORIGIN), Point::new(12., 4.)).unwrap();
        assert_eq!(point, Point::new(10., 4.));
        assert!((fraction - 0.35).abs() < 1e-12);
        let circle = Contour { layer: "0".into(), curves: vec![Curve::circle(Point::ORIGIN, 5.)] };
        let (point, fraction) = project(&circle, Point::new(0., 9.)).unwrap();
        assert!(point.distance(Point::new(0., 5.)) < 1e-9);
        assert!((fraction - 0.25).abs() < 1e-9);
    }
}
