// SPDX-License-Identifier: GPL-3.0-or-later

//! Match the same contour within translated repetitions of an original part.
//! Placement transforms and operator grouping never redefine part identity.

use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Placed, Point};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContourKey {
    layer: String,
    closed: bool,
    curves: Vec<Vec<i64>>,
}

/// The representative original contour for each placed contour. Matching the
/// complete part layout prevents equal-size holes in different positions from
/// being mistaken for one another. Geometry within one nanometre is equivalent.
pub(crate) fn matching(
    drawing: &Drawing,
    placed: &[Placed],
    groups: &[Vec<usize>],
) -> Vec<Option<usize>> {
    let mut families = BTreeMap::<Vec<ContourKey>, Vec<usize>>::new();
    let mut result = vec![None; placed.len()];
    for group in groups {
        let contours: Vec<_> = group.iter().map(|&i| &drawing.contours[placed[i].source]).collect();
        let Some(bounds) = contours.iter().filter_map(|c| c.bounds()).reduce(Bounds::union) else {
            continue;
        };
        let Some(mut keys) = contours
            .iter()
            .zip(group)
            .map(|(c, &i)| contour_key(c, bounds.min).map(|key| (key, i)))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        keys.sort_unstable();
        // Coincident duplicates have no unique geometric identity.
        if keys.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            continue;
        }
        let family = keys.iter().map(|(key, _)| key.clone()).collect();
        let representatives = families
            .entry(family)
            .or_insert_with(|| keys.iter().map(|(_, i)| placed[*i].source).collect());
        for ((_, i), &representative) in keys.iter().zip(representatives.iter()) {
            result[*i] = Some(representative);
        }
    }
    result
}

fn contour_key(contour: &Contour, origin: Point) -> Option<ContourKey> {
    let mut curves =
        contour.curves.iter().map(|c| curve_key(c, origin)).collect::<Option<Vec<_>>>()?;
    curves.sort_unstable();
    Some(ContourKey { layer: contour.layer.clone(), closed: contour.is_closed(), curves })
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "rounded and bounded below the exact-integer range"
)]
fn quantized(value: f64) -> Option<i64> {
    // Bound the conversion explicitly; saturated casts must not create matches.
    (value.is_finite() && value.abs() < 1e9).then(|| (value * 1e6).round() as i64)
}

fn point_key(point: Point, origin: Point) -> Option<[i64; 2]> {
    Some([quantized(point.x - origin.x)?, quantized(point.y - origin.y)?])
}

fn curve_key(curve: &Curve, origin: Point) -> Option<Vec<i64>> {
    let mut ends = [point_key(curve.point(0.), origin)?, point_key(curve.point(1.), origin)?];
    ends.sort_unstable();
    match *curve {
        Curve::Line { .. } => Some(vec![0, ends[0][0], ends[0][1], ends[1][0], ends[1][1]]),
        Curve::Arc { center, radius, sweep, .. } => {
            let center = point_key(center, origin)?;
            let mut key =
                vec![1, center[0], center[1], quantized(radius)?, quantized(sweep.abs())?];
            if (sweep.abs() - std::f64::consts::TAU).abs() > 1e-9 {
                // Midpoint distinguishes complementary semicircles sharing endpoints.
                key.extend(ends.into_iter().flatten());
                key.extend(point_key(curve.point(0.5), origin)?);
            }
            Some(key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::Transform;

    fn hole(x: f64, y: f64) -> Contour {
        Contour { layer: "cut".into(), curves: vec![Curve::circle(Point::new(x, y), 2.)] }
    }

    #[test]
    fn repeated_part_slots_survive_reordering_rotation_and_manual_grouping() {
        let drawing = Drawing {
            contours: vec![hole(0., 0.), hole(10., 0.), hole(110., 40.), hole(100., 40.)],
        };
        let mut placed = Placed::all(4);
        placed.extend([
            Placed { source: 0, copy: 1, transform: Transform([0., 1., -1., 0., 50., 60.]) },
            Placed { source: 1, copy: 1, transform: Transform([-1., 0., 0., 1., 50., 60.]) },
        ]);
        let matches = matching(&drawing, &placed, &[vec![0, 1], vec![2, 3], vec![4, 5]]);
        assert_eq!(matches, vec![Some(0), Some(1), Some(1), Some(0), Some(0), Some(1)]);
        assert_ne!(matches[0], matches[1]);
    }

    #[test]
    fn different_layouts_and_coincident_contours_do_not_match() {
        let drawing =
            Drawing { contours: vec![hole(0., 0.), hole(10., 0.), hole(100., 0.), hole(120., 0.)] };
        let placed = Placed::all(4);
        let matches = matching(&drawing, &placed, &[vec![0, 1], vec![2, 3]]);
        assert_ne!(matches[0], matches[2]);
        assert_eq!(
            matching(&drawing, &[Placed::drawn(0), Placed::drawn(0)], &[vec![0, 1]]),
            vec![None, None]
        );
    }
}
