// SPDX-License-Identifier: GPL-3.0-or-later

//! Import repairs: chaining pieces whose ends meet into contours, closing
//! small gaps between ends that nearly meet, and dropping curves another
//! curve on the same layer already cuts. Every repair is reported with where
//! it was made, so the operator can review it before cutting.

use crate::fit::curve_distance;
use crate::geometry::{CONTINUITY_TOLERANCE, Contour, Curve, Point};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One repair and where it was made.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Repair {
    /// Where, in millimetres.
    pub at: Point,
    /// How large: the width of a closed gap or the length of a dropped
    /// curve, in millimetres; zero for a mirrored entity.
    pub size: f64,
    /// The drawing layer.
    pub layer: String,
}

/// What import repaired in a drawing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Repairs {
    /// Gaps closed between ends that nearly met.
    pub gaps: Vec<Repair>,
    /// Curves dropped because another on the same layer already cuts them.
    pub duplicates: Vec<Repair>,
    /// Entities drawn upside down, with a flipped extrusion direction, and
    /// mirrored into place.
    pub mirrored: Vec<Repair>,
}

impl Repairs {
    /// Whether nothing was repaired.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.gaps.is_empty() && self.duplicates.is_empty() && self.mirrored.is_empty()
    }

    /// Adds the repairs of `other`.
    pub fn extend(&mut self, other: Self) {
        self.gaps.extend(other.gaps);
        self.duplicates.extend(other.duplicates);
        self.mirrored.extend(other.mirrored);
    }
}

// Chaining -----------------------------------------------------------------------

/// One end of a piece: its start or its tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct End {
    entity: usize,
    tail: bool,
}

type Links = BTreeMap<End, End>;
type Bucket = (usize, i64, i64);

/// Chains pieces on one layer whose ends meet exactly one other end into
/// contours. Pieces closed on their own stay as they are, and an end that
/// meets two or more others is left unjoined. Ends that miss each other by
/// no more than `gap` millimetres, with no other end as near, are joined
/// too, and so is a chain whose own ends are that close; each such join is
/// reported. A gap of zero joins only ends that meet.
#[must_use]
pub fn chain(entities: &[Contour], gap: f64) -> (Vec<Contour>, Vec<Repair>) {
    let gap = gap.max(CONTINUITY_TOLERANCE);
    let endpoints = Endpoints::new(entities, gap);
    let links = endpoints.links();
    let mut used = vec![false; entities.len()];
    let mut contours = Vec::new();
    let mut repairs = Vec::new();
    for seed in 0..entities.len() {
        if used[seed] {
            continue;
        }
        let start = first(&links, seed);
        let (mut curves, exit) = collect_chain(entities, &links, &mut used, start, &mut repairs);
        let layer = &entities[seed].layer;
        if exit.is_some_and(|exit| links.get(&exit) == Some(&start))
            && let (Some(first), Some(last)) = (curves.first(), curves.last())
        {
            let (from, to) = (last.end(), first.start());
            if from.distance(to) > CONTINUITY_TOLERANCE {
                repairs.push(Repair {
                    at: from.lerp(to, 0.5),
                    size: from.distance(to),
                    layer: layer.clone(),
                });
                close(&mut curves);
            }
        }
        contours.push(Contour { layer: layer.clone(), curves });
    }
    (contours, repairs)
}

struct Endpoints<'a> {
    entities: &'a [Contour],
    layers: Vec<usize>,
    ends: Vec<Option<[Point; 2]>>,
    buckets: HashMap<Bucket, Vec<End>>,
    gap: f64,
}

fn open_ends(contour: &Contour) -> Option<[Point; 2]> {
    let (start, end) = (contour.start()?, contour.end()?);
    (start.distance(end) > CONTINUITY_TOLERANCE).then_some([start, end])
}

impl<'a> Endpoints<'a> {
    fn new(entities: &'a [Contour], gap: f64) -> Self {
        let mut names = BTreeMap::new();
        let layers: Vec<usize> = entities
            .iter()
            .map(|e| {
                let next = names.len();
                *names.entry(e.layer.as_str()).or_insert(next)
            })
            .collect();
        let ends: Vec<_> = entities.iter().map(open_ends).collect();
        let mut endpoints = Self { entities, layers, ends, buckets: HashMap::new(), gap };
        for entity in 0..entities.len() {
            if let Some([start, end]) = endpoints.ends[entity] {
                for (point, tail) in [(start, false), (end, true)] {
                    let bucket = endpoints.bucket(entity, point);
                    endpoints.buckets.entry(bucket).or_default().push(End { entity, tail });
                }
            }
        }
        endpoints
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "coordinates are bounded well below 1e12, so the bucket fits"
    )]
    fn bucket(&self, entity: usize, point: Point) -> Bucket {
        (
            self.layers[entity],
            (point.x / self.gap).floor() as i64,
            (point.y / self.gap).floor() as i64,
        )
    }

    fn point(&self, end: End) -> Option<Point> {
        self.ends[end.entity].map(|p| p[usize::from(end.tail)])
    }

    /// Exactly one end meets this one, or, when none meets it, exactly one
    /// lies within the gap; junctions remain disconnected. A piece's own
    /// other end counts only when the piece is long enough to close.
    fn partner(&self, end: End) -> Option<End> {
        let p = self.point(end)?;
        let (layer, x, y) = self.bucket(end.entity, p);
        let (mut meeting, mut near) = (Vec::new(), Vec::new());
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &other in self.buckets.get(&(layer, x + dx, y + dy)).into_iter().flatten() {
                    if other == end
                        || (other.entity == end.entity
                            && self.entities[end.entity].length() <= 4. * self.gap)
                    {
                        continue;
                    }
                    let Some(distance) = self.point(other).map(|q| q.distance(p)) else {
                        continue;
                    };
                    if distance <= CONTINUITY_TOLERANCE {
                        meeting.push(other);
                    } else if distance <= self.gap {
                        near.push(other);
                    }
                }
            }
        }
        match (meeting.as_slice(), near.as_slice()) {
            ([one], _) | ([], [one]) => Some(*one),
            _ => None,
        }
    }

    fn links(&self) -> Links {
        let mut links = Links::new();
        for entity in 0..self.entities.len() {
            for tail in [false, true] {
                let end = End { entity, tail };
                if let Some(other) = self.partner(end)
                    && self.partner(other) == Some(end)
                {
                    links.insert(end, other);
                }
            }
        }
        links
    }
}

/// Walk back to an open start; a loop starts at the seed.
fn first(links: &Links, seed: usize) -> End {
    let mut first = End { entity: seed, tail: false };
    let mut visited = BTreeSet::from([seed]);
    while let Some(&previous) = links.get(&first) {
        if previous.entity == seed {
            return End { entity: seed, tail: false };
        }
        if !visited.insert(previous.entity) {
            break;
        }
        first = End { entity: previous.entity, tail: !previous.tail };
    }
    first
}

/// The curves of the chain from `cursor`, and the end it leaves by.
fn collect_chain(
    entities: &[Contour],
    links: &Links,
    used: &mut [bool],
    mut cursor: End,
    repairs: &mut Vec<Repair>,
) -> (Vec<Curve>, Option<End>) {
    let mut curves: Vec<Curve> = Vec::new();
    let mut exit = None;
    loop {
        let entity = cursor.entity;
        if used[entity] {
            break;
        }
        used[entity] = true;
        let piece: Vec<Curve> = if cursor.tail {
            entities[entity].curves.iter().rev().map(Curve::reversed).collect()
        } else {
            entities[entity].curves.clone()
        };
        if let (Some(last), Some(next)) = (curves.last(), piece.first()) {
            let (from, to) = (last.end(), next.start());
            if from.distance(to) > CONTINUITY_TOLERANCE {
                repairs.push(Repair {
                    at: from.lerp(to, 0.5),
                    size: from.distance(to),
                    layer: entities[entity].layer.clone(),
                });
                let at = curves.len();
                curves.extend(piece);
                bridge(&mut curves, at);
            } else {
                curves.extend(piece);
            }
        } else {
            curves.extend(piece);
        }
        let out = End { entity, tail: !cursor.tail };
        exit = Some(out);
        match links.get(&out) {
            Some(&next) if !used[next.entity] => cursor = next,
            _ => break,
        }
    }
    (curves, exit)
}

/// Joins the curve before `at` to the curve at `at`: a line end moves onto
/// the other curve's end, or a short line bridges two arcs.
fn bridge(curves: &mut Vec<Curve>, at: usize) {
    let (from, to) = (curves[at - 1].end(), curves[at].start());
    if let Curve::Line { end, .. } = &mut curves[at] {
        let moved = Curve::Line { start: from, end: *end };
        if moved.is_valid() {
            curves[at] = moved;
            return;
        }
    }
    if let Curve::Line { start, .. } = curves[at - 1] {
        let moved = Curve::Line { start, end: to };
        if moved.is_valid() {
            curves[at - 1] = moved;
            return;
        }
    }
    curves.insert(at, Curve::Line { start: from, end: to });
}

/// Closes a chain whose ends nearly meet.
fn close(curves: &mut Vec<Curve>) {
    let (Some(first), Some(last)) = (curves.first().copied(), curves.last().copied()) else {
        return;
    };
    let (from, to) = (last.end(), first.start());
    if let Curve::Line { end, .. } = first {
        let moved = Curve::Line { start: from, end };
        if moved.is_valid() && curves.len() > 1 {
            curves[0] = moved;
            return;
        }
    }
    if let Curve::Line { start, .. } = last {
        let moved = Curve::Line { start, end: to };
        if moved.is_valid() && curves.len() > 1 {
            let at = curves.len() - 1;
            curves[at] = moved;
            return;
        }
    }
    curves.push(Curve::Line { start: from, end: to });
}

// Duplicates ---------------------------------------------------------------------

/// The pieces without curves that another kept curve on the same layer
/// already covers to within `tolerance`, and where each dropped curve was.
///
/// An open piece loses the covered curves and splits where they were. A
/// closed piece is dropped only whole, when every curve of it lies on one
/// other closed piece of about the same length, so shapes that share an edge
/// keep it. Curves no longer than twice the tolerance are never dropped.
#[must_use]
pub fn without_duplicates(entities: Vec<Contour>, tolerance: f64) -> (Vec<Contour>, Vec<Repair>) {
    let index = Index::new(&entities, tolerance);
    let mut removed: Vec<Vec<bool>> =
        entities.iter().map(|e| vec![false; e.curves.len()]).collect();
    let mut repairs = Vec::new();
    for entity in (0..entities.len()).rev() {
        let contour = &entities[entity];
        if contour.is_closed() {
            if let Some(repair) = index.closed_repeat(&entities, &removed, entity) {
                removed[entity].iter_mut().for_each(|r| *r = true);
                repairs.push(repair);
            }
            continue;
        }
        for (k, curve) in contour.curves.iter().enumerate() {
            if curve.length() <= 2. * tolerance {
                continue;
            }
            if index.covered(&entities, &removed, (entity, k)) {
                removed[entity][k] = true;
                repairs.push(Repair {
                    at: curve.point(0.5),
                    size: curve.length(),
                    layer: contour.layer.clone(),
                });
            }
        }
    }
    repairs.reverse();
    let mut kept = Vec::with_capacity(entities.len());
    for (contour, removed) in entities.into_iter().zip(removed) {
        if !removed.contains(&true) {
            kept.push(contour);
            continue;
        }
        let mut run = Vec::new();
        for (curve, gone) in contour.curves.into_iter().zip(removed) {
            if gone {
                if !run.is_empty() {
                    kept.push(Contour {
                        layer: contour.layer.clone(),
                        curves: std::mem::take(&mut run),
                    });
                }
            } else {
                run.push(curve);
            }
        }
        if !run.is_empty() {
            kept.push(Contour { layer: contour.layer, curves: run });
        }
    }
    (kept, repairs)
}

/// A count as a float.
#[allow(clippy::cast_precision_loss, reason = "counts stay far below 2^53")]
fn float(count: usize) -> f64 {
    count as f64
}

/// A grid of the cells each curve passes, per layer.
struct Index {
    cell: f64,
    tolerance: f64,
    layers: Vec<usize>,
    cells: HashMap<(usize, i64, i64), Vec<(usize, usize)>>,
}

impl Index {
    fn new(entities: &[Contour], tolerance: f64) -> Self {
        let mut names = BTreeMap::new();
        let layers: Vec<usize> = entities
            .iter()
            .map(|e| {
                let next = names.len();
                *names.entry(e.layer.as_str()).or_insert(next)
            })
            .collect();
        let extent = entities
            .iter()
            .filter_map(Contour::bounds)
            .reduce(crate::geometry::Bounds::union)
            .map_or(1., |b| b.extent());
        let length: f64 = entities.iter().map(Contour::length).sum();
        // About four million cell visits at most, and never finer than the
        // tolerance can resolve.
        let cell = (extent / 2048.).max(length / 2e6).max(tolerance * 8.).max(1e-3);
        let mut index = Self { cell, tolerance, layers, cells: HashMap::new() };
        for (entity, contour) in entities.iter().enumerate() {
            for (k, curve) in contour.curves.iter().enumerate() {
                let mut seen = BTreeSet::new();
                for key in index.keys(entity, curve) {
                    if seen.insert(key) {
                        index.cells.entry(key).or_default().push((entity, k));
                    }
                }
            }
        }
        index
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "cells are bounded by the drawing's extent"
    )]
    fn keys(&self, entity: usize, curve: &Curve) -> Vec<(usize, i64, i64)> {
        let steps = ((curve.length() / (self.cell * 0.5)).ceil() as usize).clamp(1, 1 << 20);
        (0..=steps)
            .map(|k| {
                let p = curve.point(float(k) / float(steps));
                (
                    self.layers[entity],
                    (p.x / self.cell).floor() as i64,
                    (p.y / self.cell).floor() as i64,
                )
            })
            .collect()
    }

    /// The kept curves near `curve`, not in `skip`.
    fn near(
        &self,
        entity: usize,
        curve: &Curve,
        removed: &[Vec<bool>],
        skip: usize,
    ) -> BTreeSet<(usize, usize)> {
        let mut found = BTreeSet::new();
        for (layer, x, y) in self.keys(entity, curve) {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for &(e, k) in self.cells.get(&(layer, x + dx, y + dy)).into_iter().flatten() {
                        if e != skip && !removed[e][k] {
                            found.insert((e, k));
                        }
                    }
                }
            }
        }
        found
    }

    fn lies_on(&self, curve: &Curve, other: &Curve) -> bool {
        other.length() + self.tolerance >= curve.length()
            && [0., 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.]
                .iter()
                .all(|&t| curve_distance(curve.point(t), other) <= self.tolerance)
    }

    /// Whether another kept curve covers this open piece's curve.
    fn covered(&self, entities: &[Contour], removed: &[Vec<bool>], (e, k): (usize, usize)) -> bool {
        let curve = &entities[e].curves[k];
        let mut near = self.near(e, curve, removed, usize::MAX);
        near.remove(&(e, k));
        near.into_iter().any(|(oe, ok)| self.lies_on(curve, &entities[oe].curves[ok]))
    }

    /// Whether a whole closed piece repeats another kept closed piece.
    fn closed_repeat(
        &self,
        entities: &[Contour],
        removed: &[Vec<bool>],
        entity: usize,
    ) -> Option<Repair> {
        let contour = &entities[entity];
        let first = contour.curves.first()?;
        let candidates: BTreeSet<usize> =
            self.near(entity, first, removed, entity).into_iter().map(|(e, _)| e).collect();
        for other in candidates {
            let target = &entities[other];
            if !target.is_closed()
                || removed[other].contains(&true)
                || (target.length() - contour.length()).abs()
                    > self.tolerance * 2. * float(contour.curves.len() + target.curves.len())
            {
                continue;
            }
            let on = |from: &Contour, to: &Contour| {
                from.curves.iter().all(|c| {
                    [0., 0.25, 0.5, 0.75].iter().all(|&t| {
                        to.curves.iter().any(|o| curve_distance(c.point(t), o) <= self.tolerance)
                    })
                })
            };
            if on(contour, target) && on(target, contour) {
                return Some(Repair {
                    at: first.start(),
                    size: contour.length(),
                    layer: contour.layer.clone(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(layer: &str, a: [f64; 2], b: [f64; 2]) -> Contour {
        Contour {
            layer: layer.into(),
            curves: vec![Curve::Line { start: a.into(), end: b.into() }],
        }
    }

    /// Entities chain regardless of their order and direction, a three-way
    /// junction is never chained through, and layers never join.
    #[test]
    fn chains_join_unique_ends_only() {
        let (joined, gaps) =
            chain(&[line("0", [10., 0.], [0., 0.]), line("0", [10., 0.], [20., 0.])], 0.);
        assert_eq!(joined.len(), 1);
        assert!(gaps.is_empty());
        assert_eq!(joined[0].curves.len(), 2);
        assert!(joined[0].is_continuous());
        let (junction, _) = chain(
            &[
                line("0", [0., 0.], [5., 0.]),
                line("0", [5., 0.], [10., 0.]),
                line("0", [5., 0.], [5., 5.]),
            ],
            0.,
        );
        assert_eq!(junction.len(), 3);
        let (layers, _) =
            chain(&[line("A", [0., 0.], [5., 0.]), line("B", [5., 0.], [10., 0.])], 0.1);
        assert_eq!(layers.len(), 2);
    }

    /// A loop of entities becomes one closed contour starting at the first
    /// entity, and an entity closed on its own is kept whole.
    #[test]
    fn loops_close_and_closed_entities_stay_whole() {
        let (square, _) = chain(
            &[
                line("0", [0., 0.], [1., 0.]),
                line("0", [1., 1.], [0., 1.]),
                line("0", [0., 1.], [0., 0.]),
                line("0", [1., 0.], [1., 1.]),
            ],
            0.,
        );
        assert_eq!(square.len(), 1);
        assert!(square[0].is_closed());
        assert_eq!(square[0].start(), Some(Point::ORIGIN));
        let circle = Contour { layer: "0".into(), curves: vec![Curve::circle(Point::ORIGIN, 1.)] };
        assert_eq!(chain(&[circle, line("0", [1., 0.], [2., 0.])], 0.).0.len(), 2);
    }

    /// Ends that miss by less than the gap join and are reported, the
    /// chain closes when its own ends nearly meet, and an end with two
    /// candidates within the gap is left alone.
    #[test]
    fn small_gaps_join_and_are_reported() {
        let pieces = [
            line("0", [0., 0.], [10., 0.]),
            line("0", [10.02, 0.], [10., 10.]),
            line("0", [10., 10.], [0., 10.]),
            line("0", [0., 10.], [0., 0.03]),
        ];
        let (closed, gaps) = chain(&pieces, 0.05);
        assert_eq!(closed.len(), 1);
        assert!(closed[0].is_closed(), "{:?}", closed[0].curves);
        assert_eq!(gaps.len(), 2);
        assert!((gaps[0].size - 0.02).abs() < 1e-9);
        assert!(gaps.iter().any(|g| g.at.distance(Point::new(0., 0.015)) < 1e-9));
        let (unjoined, none) = chain(&pieces, 0.01);
        assert_eq!(unjoined.len(), 2);
        assert!(none.is_empty());
        let (ambiguous, none) = chain(
            &[
                line("0", [0., 0.], [10., 0.]),
                line("0", [10.02, 0.], [20., 0.]),
                line("0", [10.02, 0.01], [10., 10.]),
            ],
            0.05,
        );
        assert_eq!(ambiguous.len(), 3);
        assert!(none.is_empty());
        // Two arcs that miss are bridged by a short line.
        let arcs = [
            Contour {
                layer: "0".into(),
                curves: vec![Curve::Arc {
                    center: Point::ORIGIN,
                    radius: 5.,
                    start_angle: 0.,
                    sweep: std::f64::consts::PI,
                }],
            },
            Contour {
                layer: "0".into(),
                curves: vec![Curve::Arc {
                    center: Point::new(0.01, 0.),
                    radius: 5.,
                    start_angle: std::f64::consts::PI,
                    sweep: std::f64::consts::PI,
                }],
            },
        ];
        let (circle, gaps) = chain(&arcs, 0.05);
        assert_eq!(circle.len(), 1);
        assert!(circle[0].is_closed());
        assert_eq!((circle[0].curves.len(), gaps.len()), (4, 2));
    }

    /// Repeated and covered curves go, closed shapes that share an edge keep
    /// it, and a closed shape drawn twice keeps one copy.
    #[test]
    fn duplicates_and_covered_curves_are_dropped() {
        let pieces = vec![
            line("0", [0., 0.], [10., 0.]),
            line("0", [10., 0.], [0., 0.]),
            line("0", [2., 0.], [6., 0.]),
            line("1", [0., 0.], [10., 0.]),
            line("0", [0., 5.], [10., 5.00001]),
        ];
        let (kept, dropped) = without_duplicates(pieces, 0.001);
        assert_eq!(kept.len(), 3, "{kept:?}");
        assert_eq!(dropped.len(), 2);
        assert!(dropped.iter().any(|d| (d.size - 4.).abs() < 1e-9));
        let square = |x: f64| Contour {
            layer: "0".into(),
            curves: vec![
                Curve::Line { start: Point::new(x, 0.), end: Point::new(x + 10., 0.) },
                Curve::Line { start: Point::new(x + 10., 0.), end: Point::new(x + 10., 10.) },
                Curve::Line { start: Point::new(x + 10., 10.), end: Point::new(x, 10.) },
                Curve::Line { start: Point::new(x, 10.), end: Point::new(x, 0.) },
            ],
        };
        let (kept, dropped) = without_duplicates(vec![square(0.), square(10.)], 0.001);
        assert_eq!((kept.len(), dropped.len()), (2, 0));
        let (kept, dropped) = without_duplicates(vec![square(0.), square(0.).reversed()], 0.001);
        assert_eq!((kept.len(), dropped.len()), (1, 1));
        let circle = Contour { layer: "0".into(), curves: vec![Curve::circle(Point::ORIGIN, 3.)] };
        let (kept, dropped) = without_duplicates(vec![circle.clone(), circle], 0.001);
        assert_eq!((kept.len(), dropped.len()), (1, 1));
    }

    /// A polyline that loses a covered curve in its middle splits there.
    #[test]
    fn a_covered_curve_splits_its_polyline() {
        let polyline = Contour {
            layer: "0".into(),
            curves: vec![
                Curve::Line { start: Point::new(0., 5.), end: Point::new(0., 0.) },
                Curve::Line { start: Point::new(0., 0.), end: Point::new(10., 0.) },
                Curve::Line { start: Point::new(10., 0.), end: Point::new(10., 5.) },
            ],
        };
        let (kept, dropped) =
            without_duplicates(vec![line("0", [0., 0.], [10., 0.]), polyline], 0.001);
        assert_eq!(dropped.len(), 1);
        assert_eq!(kept.len(), 3);
    }
}
