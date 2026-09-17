// SPDX-License-Identifier: GPL-3.0-or-later

//! Resolve text fills and union their silhouettes before producing cut lines.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{IntOverlayOptions, Overlay};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::i_float::int::point::IntPoint;
use openlaser_core::geometry::{Contour, Curve, Point};
use std::collections::BTreeMap;
use std::ops::Range;

// One nanometre in drawing units, far below the 0.01 mm curve tolerance.
// Reader validates finite coordinates within +/- 1e6 mm before this conversion.
const SCALE: f64 = 1_000_000.;
type Ring = Vec<(i64, i64)>;

/// Each range is one SVG paint operation: normalize its own fill rule before
/// unioning it with the other letters, whose original windings may differ.
pub(crate) fn text(
    contours: &[Contour],
    paints: &[(Range<usize>, usvg::FillRule)],
) -> Vec<Contour> {
    let rings: Vec<_> = contours.iter().map(ring).collect();
    let mut normalized = Vec::new();
    for (range, rule) in paints {
        let rule = match rule {
            usvg::FillRule::NonZero => FillRule::NonZero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        normalized.extend(resolve(&rings[range.clone()], rule));
    }
    let merged = resolve(&normalized, FillRule::NonZero);
    restore(contours, &rings, merged)
}

fn resolve(rings: &[Ring], rule: FillRule) -> Vec<Ring> {
    let paths: Vec<Vec<IntPoint<i64>>> =
        rings.iter().map(|ring| ring.iter().map(|&(x, y)| IntPoint::new(x, y)).collect()).collect();
    Overlay::with_contours_custom(
        &paths,
        &[],
        IntOverlayOptions {
            preserve_input_collinear: true,
            preserve_output_collinear: true,
            ogc: true,
            ..IntOverlayOptions::default()
        },
        Solver::default(),
    )
    .overlay(OverlayRule::Subject, rule)
    .into_iter()
    .flatten()
    .map(|ring| ring.into_iter().map(|point| (point.x, point.y)).collect())
    .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "finite coordinates are bounded to 1e6 mm by Reader"
)]
fn ring(contour: &Contour) -> Ring {
    contour
        .curves
        .iter()
        .map(|curve| {
            let point = curve.point(0.);
            ((point.x * SCALE).round() as i64, (point.y * SCALE).round() as i64)
        })
        .collect()
}

/// Equivalent rings retain their exact coordinates, seam and source order.
/// Welded boundaries take the earliest contributing contour's order and layer.
fn restore(original: &[Contour], rings: &[Ring], merged: Vec<Ring>) -> Vec<Contour> {
    let mut exact = BTreeMap::new();
    let mut vertices = BTreeMap::new();
    for (index, ring) in rings.iter().enumerate() {
        exact.entry(key(ring)).or_insert(index);
        for &point in ring {
            vertices.entry(point).or_insert(index);
        }
    }
    let mut result: Vec<_> = merged
        .into_iter()
        .filter(|ring| ring.len() >= 3)
        .map(|ring| {
            if let Some(&index) = exact.get(&key(&ring)) {
                return (index, original[index].clone());
            }
            let index =
                ring.iter().filter_map(|point| vertices.get(point)).copied().min().unwrap_or(0);
            let points: Vec<_> = ring.iter().map(|&(x, y)| from_grid(x, y)).collect();
            let curves = points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(&start, &end)| Curve::Line { start, end })
                .collect();
            (
                index,
                Contour {
                    layer: original.get(index).map_or_else(|| "0".into(), |c| c.layer.clone()),
                    curves,
                },
            )
        })
        .collect();
    result.sort_by_key(|(index, _)| *index);
    result.into_iter().map(|(_, contour)| contour).collect()
}

fn key(ring: &Ring) -> Ring {
    if ring.is_empty() {
        return vec![];
    }
    let first = ring.iter().enumerate().min_by_key(|(_, point)| *point).map_or(0, |(i, _)| i);
    let forward: Ring = (0..ring.len()).map(|i| ring[(first + i) % ring.len()]).collect();
    let reverse: Ring =
        (0..ring.len()).map(|i| ring[(first + ring.len() - i) % ring.len()]).collect();
    forward.min(reverse)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "grid coordinates are bounded to 1e12, exactly representable by f64"
)]
fn from_grid(x: i64, y: i64) -> Point {
    Point::new(x as f64 / SCALE, y as f64 / SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(x: f64, y: f64, width: f64, height: f64) -> Contour {
        let p = [
            Point::new(x, y),
            Point::new(x + width, y),
            Point::new(x + width, y + height),
            Point::new(x, y + height),
        ];
        Contour {
            layer: "letters".into(),
            curves: p
                .iter()
                .zip(p.iter().cycle().skip(1))
                .map(|(&start, &end)| Curve::Line { start, end })
                .collect(),
        }
    }

    #[test]
    fn union_preserves_holes_and_separately_painted_opposite_windings() {
        let contours = vec![
            rectangle(0., 0., 10., 10.),
            rectangle(2., 2., 6., 6.).reversed(),
            rectangle(7., -1., 5., 12.).reversed(),
        ];
        let welded =
            text(&contours, &[(0..2, usvg::FillRule::NonZero), (2..3, usvg::FillRule::NonZero)]);
        assert_eq!(welded.len(), 2);
        assert!(welded.iter().all(Contour::is_closed));
        assert!((welded.iter().map(Contour::signed_area).sum::<f64>() - 100.).abs() < 1e-6);
        assert!(welded.iter().any(|c| (c.signed_area() + 30.).abs() < 1e-6));
        // Even-odd input resolves the same hole when both raw rings run CCW.
        let mut even_odd = contours.clone();
        even_odd[1] = even_odd[1].reversed();
        assert_eq!(
            text(&even_odd, &[(0..2, usvg::FillRule::EvenOdd), (2..3, usvg::FillRule::NonZero)]),
            welded
        );
    }

    #[test]
    fn separated_letters_keep_exact_coordinates_order_and_seams() {
        let contours =
            vec![rectangle(20.123_456_789, 8., 4., 6.), rectangle(2., 2., 5., 3.).reversed()];
        assert_eq!(
            text(&contours, &[(0..1, usvg::FillRule::NonZero), (1..2, usvg::FillRule::NonZero)]),
            contours
        );
    }
}
