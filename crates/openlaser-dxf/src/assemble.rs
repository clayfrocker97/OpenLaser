// SPDX-License-Identifier: GPL-3.0-or-later

//! Chaining entities into contours where their ends meet.

use openlaser_core::geometry::{CONTINUITY_TOLERANCE, Contour, Curve, Point};
use std::collections::{BTreeMap, BTreeSet};

/// One end of an entity: its start or its tail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct End {
    entity: usize,
    tail: bool,
}

/// Chains entities on one layer whose ends meet exactly one other end.
/// Entities closed on their own stay as they are, and an end that meets
/// two or more others is left unjoined.
pub(crate) fn chain(entities: &[Contour]) -> Vec<Contour> {
    let links = Endpoints::new(entities).links();
    let mut used = vec![false; entities.len()];
    let mut contours = Vec::new();
    for seed in 0..entities.len() {
        if used[seed] {
            continue;
        }
        let start = first(&links, seed);
        let curves = collect_chain(entities, &links, &mut used, start);
        contours.push(Contour { layer: entities[seed].layer.clone(), curves });
    }
    contours
}

type Links = BTreeMap<End, End>;
type Bucket = (String, i64, i64);

struct Endpoints<'a> {
    entities: &'a [Contour],
    ends: Vec<Option<[Point; 2]>>,
    buckets: BTreeMap<Bucket, Vec<End>>,
}

fn open_ends(contour: &Contour) -> Option<[Point; 2]> {
    let (start, end) = (contour.start()?, contour.end()?);
    (start.distance(end) > CONTINUITY_TOLERANCE).then_some([start, end])
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "coordinates are bounded to 1e9, so the bucket fits"
)]
fn bucket(layer: &str, point: Point) -> Bucket {
    (
        layer.to_owned(),
        (point.x / CONTINUITY_TOLERANCE).floor() as i64,
        (point.y / CONTINUITY_TOLERANCE).floor() as i64,
    )
}

impl<'a> Endpoints<'a> {
    fn new(entities: &'a [Contour]) -> Self {
        let ends: Vec<_> = entities.iter().map(open_ends).collect();
        let mut buckets: BTreeMap<Bucket, Vec<End>> = BTreeMap::new();
        for (entity, points) in ends.iter().enumerate() {
            if let Some([start, end]) = points {
                for (point, tail) in [(*start, false), (*end, true)] {
                    buckets
                        .entry(bucket(&entities[entity].layer, point))
                        .or_default()
                        .push(End { entity, tail });
                }
            }
        }
        Self { entities, ends, buckets }
    }

    fn point(&self, end: End) -> Option<Point> {
        self.ends[end.entity].map(|p| p[usize::from(end.tail)])
    }

    /// Exactly one neighbouring end qualifies; junctions remain disconnected.
    fn partner(&self, end: End) -> Option<End> {
        let p = self.point(end)?;
        let (layer, x, y) = bucket(&self.entities[end.entity].layer, p);
        let mut found = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &other in
                    self.buckets.get(&(layer.clone(), x + dx, y + dy)).into_iter().flatten()
                {
                    if other.entity != end.entity
                        && self.point(other).is_some_and(|q| q.distance(p) <= CONTINUITY_TOLERANCE)
                    {
                        if found.is_some() {
                            return None;
                        }
                        found = Some(other);
                    }
                }
            }
        }
        found
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

fn collect_chain(
    entities: &[Contour],
    links: &Links,
    used: &mut [bool],
    mut cursor: End,
) -> Vec<Curve> {
    let mut curves = Vec::new();
    loop {
        let entity = cursor.entity;
        if used[entity] {
            break;
        }
        used[entity] = true;
        if cursor.tail {
            curves.extend(entities[entity].curves.iter().rev().map(Curve::reversed));
        } else {
            curves.extend_from_slice(&entities[entity].curves);
        }
        match links.get(&End { entity, tail: !cursor.tail }) {
            Some(&next) if !used[next.entity] => cursor = next,
            _ => break,
        }
    }
    curves
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
        let joined = chain(&[line("0", [10., 0.], [0., 0.]), line("0", [10., 0.], [20., 0.])]);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].curves.len(), 2);
        assert!(joined[0].is_continuous());
        let junction = chain(&[
            line("0", [0., 0.], [5., 0.]),
            line("0", [5., 0.], [10., 0.]),
            line("0", [5., 0.], [5., 5.]),
        ]);
        assert_eq!(junction.len(), 3);
        let layers = chain(&[line("A", [0., 0.], [5., 0.]), line("B", [5., 0.], [10., 0.])]);
        assert_eq!(layers.len(), 2);
    }

    /// A loop of entities becomes one closed contour starting at the first
    /// entity, and an entity closed on its own is kept whole.
    #[test]
    fn loops_close_and_closed_entities_stay_whole() {
        let square = chain(&[
            line("0", [0., 0.], [1., 0.]),
            line("0", [1., 1.], [0., 1.]),
            line("0", [0., 1.], [0., 0.]),
            line("0", [1., 0.], [1., 1.]),
        ]);
        assert_eq!(square.len(), 1);
        assert!(square[0].is_closed());
        assert_eq!(square[0].start(), Some(Point::ORIGIN));
        let circle = Contour { layer: "0".into(), curves: vec![Curve::circle(Point::ORIGIN, 1.)] };
        assert_eq!(chain(&[circle, line("0", [1., 0.], [2., 0.])]).len(), 2);
    }
}
