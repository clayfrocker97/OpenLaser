// SPDX-License-Identifier: GPL-3.0-or-later

//! Which contours lie inside which, and kerf compensation. Both go through
//! `cavalier_contours` polylines with bulges, so arcs stay arcs.

use crate::{Error, Result, float};
use cavalier_contours::polyline::{
    PlineContainsOptions, PlineContainsResult, PlineSource, PlineSourceMut, PlineVertex, Polyline,
    seg_arc_radius_and_center,
};
use cavalier_contours::static_aabb2d_index::{StaticAABB2DIndex, StaticAABB2DIndexBuilder};
use openlaser_core::features::{Kerf, Side};
use openlaser_core::geometry::{Contour, Curve, Point};
use std::f64::consts::PI;

/// A closed contour as a polyline. Arcs of more than a half turn are split
/// so every bulge stays finite.
pub(crate) fn polyline(contour: &Contour) -> std::result::Result<Polyline, String> {
    if !contour.is_closed() {
        return Err("is not closed".into());
    }
    let mut polyline = Polyline::new_closed();
    for curve in &contour.curves {
        let (pieces, bulge) = match *curve {
            Curve::Arc { sweep, .. } => {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a sweep is at most one turn, so at most two pieces"
                )]
                let pieces = (sweep.abs() / PI).ceil().max(1.) as usize;
                (pieces, (sweep / float(pieces) / 4.).tan())
            }
            Curve::Line { .. } => (1, 0.),
        };
        for piece in 0..pieces {
            let point = curve.point(float(piece) / float(pieces));
            polyline.add(point.x, point.y, bulge);
        }
    }
    if polyline.scan_for_self_intersect() {
        return Err("crosses itself".into());
    }
    Ok(polyline)
}

/// One segment index per contour, shared by every containment query in this
/// analysis. The outer index rejects distant contours before exact line/arc
/// predicates run; it never substitutes bounding boxes for containment.
struct Topology {
    contours: Vec<Option<(Polyline, StaticAABB2DIndex<f64>)>>,
    spatial: Option<StaticAABB2DIndex<f64>>,
    indices: Vec<usize>,
}

impl Topology {
    fn new(polylines: Vec<Option<Polyline>>) -> Self {
        let contours: Vec<_> = polylines
            .into_iter()
            .map(|p| {
                p.map(|p| {
                    let segments = p.create_approx_aabb_index();
                    (p, segments)
                })
            })
            .collect();
        let indices: Vec<_> =
            contours.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|_| i)).collect();
        let mut builder = StaticAABB2DIndexBuilder::new(indices.len());
        for &i in &indices {
            if let Some(bounds) = contours[i].as_ref().and_then(|(_, s)| s.bounds()) {
                builder.add(bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y);
            }
        }
        // If a degenerate grouping input has no bounds, retain the exhaustive
        // behavior. Preparation still validates every source contour first.
        let spatial = builder.build().ok();
        Self { contours, spatial, indices }
    }

    fn candidates(&self, i: usize) -> Vec<usize> {
        let Some((_, segments)) = &self.contours[i] else { return Vec::new() };
        let (Some(spatial), Some(bounds)) = (&self.spatial, segments.bounds()) else {
            return self.indices.clone();
        };
        // Match the exact predicate's tolerance, including near-touching arcs
        // and lines whose unexpanded bounding boxes do not quite overlap.
        let eps = PlineContainsOptions::<f64>::default().pos_equal_eps;
        let mut found: Vec<_> = spatial
            .query_iter(
                bounds.min_x - eps,
                bounds.min_y - eps,
                bounds.max_x + eps,
                bounds.max_y + eps,
            )
            .map(|slot| self.indices[slot])
            .collect();
        // Keep source order for stable groups and the same first topology error.
        found.sort_unstable();
        found
    }

    fn contains(&self, i: usize, j: usize) -> PlineContainsResult {
        let (Some((a, segments)), Some((b, _))) = (&self.contours[i], &self.contours[j]) else {
            return PlineContainsResult::InvalidInput;
        };
        a.contains_opt(
            b,
            &PlineContainsOptions {
                pline1_aabb_index: Some(segments),
                ..PlineContainsOptions::default()
            },
        )
    }
}

/// How many closed contours enclose each contour. An open contour counts
/// as enclosed by nothing. Closed contours that cross are refused.
pub(crate) fn depths(contours: &[&Contour]) -> Result<Vec<usize>> {
    depths_with(contours, |_, _| false)
}

pub(crate) fn depths_with(
    contours: &[&Contour],
    shared: impl Fn(usize, usize) -> bool,
) -> Result<Vec<usize>> {
    let polylines = contours
        .iter()
        .enumerate()
        .map(|(index, contour)| {
            if contour.is_closed() {
                polyline(contour).map(Some).map_err(|reason| Error::Contour { index, reason })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let topology = Topology::new(polylines);
    let mut depth = vec![0; contours.len()];
    for i in 0..contours.len() {
        for j in topology.candidates(i).into_iter().filter(|&j| j > i) {
            let (Some((a, _)), Some((b, _))) = (&topology.contours[i], &topology.contours[j])
            else {
                continue;
            };
            match topology.contains(i, j) {
                PlineContainsResult::Pline1InsidePline2 => depth[i] += 1,
                PlineContainsResult::Pline2InsidePline1 => depth[j] += 1,
                PlineContainsResult::Disjoint => {}
                _ if shared(i, j)
                    && a.boolean(b, cavalier_contours::polyline::BooleanOp::And)
                        .pos_plines
                        .iter()
                        .all(|p| p.pline.area().abs() <= crate::EPS * crate::EPS) => {}
                _ => {
                    return Err(Error::Topology(format!(
                        "contours {i} and {j} cross or touch, so which is inside is ambiguous"
                    )));
                }
            }
        }
    }
    Ok(depth)
}

/// The contours that move together: each closed contour inside no other,
/// with everything it encloses; every other contour on its own. A pair
/// that crosses counts as apart, so the drawing stays selectable.
#[must_use]
pub fn groups(contours: &[Contour]) -> Vec<Vec<usize>> {
    let polylines: Vec<Option<Polyline>> = contours.iter().map(|c| polyline(c).ok()).collect();
    let topology = Topology::new(polylines);
    let inside = |i, j| matches!(topology.contains(i, j), PlineContainsResult::Pline1InsidePline2);
    let mut groups = Vec::new();
    let mut taken = vec![false; contours.len()];
    for outer in 0..contours.len() {
        if taken[outer] {
            continue;
        }
        let candidates = topology.candidates(outer);
        if candidates.iter().any(|&j| j != outer && inside(outer, j)) {
            continue;
        }
        let mut members = vec![outer];
        members.extend(
            candidates.into_iter().filter(|&i| i != outer && !taken[i] && inside(i, outer)),
        );
        members.sort_unstable();
        for &i in &members {
            taken[i] = true;
        }
        groups.push(members);
    }
    groups
}

/// The contour moved half the kerf into its waste: into a hole, out from
/// an outline, when the side is automatic.
pub(crate) fn compensated(
    contour: &Contour,
    kerf: &Kerf,
    hole: bool,
) -> std::result::Result<Contour, String> {
    let inside = match kerf.side {
        Side::Inside => true,
        Side::Outside => false,
        Side::Auto => hole,
    };
    offset(contour, kerf.width.0 / 2., inside)
}

/// The closed contour offset by `distance`, inward or outward. The result
/// must be one closed contour that does not cross itself.
pub(crate) fn offset(
    contour: &Contour,
    distance: f64,
    inside: bool,
) -> std::result::Result<Contour, String> {
    let source = polyline(contour)?;
    if distance == 0. {
        return Ok(contour.clone());
    }
    // A positive offset moves to the left of travel, which is inward for a
    // counterclockwise polyline.
    let signed = if (source.area() > 0.) == inside { distance } else { -distance };
    let parts = source.parallel_offset(signed);
    if parts.len() != 1 {
        return Err(format!(
            "kerf compensation leaves {} pieces; reduce the kerf or revise the geometry",
            parts.len()
        ));
    }
    let curves = parts[0].iter_segments().map(|(a, b)| curve(a, b)).collect();
    let result = Contour { layer: contour.layer.clone(), curves };
    polyline(&result)?;
    Ok(result)
}

/// The line or arc between two polyline vertices.
fn curve(a: PlineVertex<f64>, b: PlineVertex<f64>) -> Curve {
    let start = Point::new(a.x, a.y);
    if a.bulge.abs() < 1e-12 {
        return Curve::Line { start, end: Point::new(b.x, b.y) };
    }
    let (radius, center) = seg_arc_radius_and_center(a, b);
    let center = Point::new(center.x, center.y);
    Curve::Arc { center, radius, start_angle: (start - center).angle(), sweep: 4. * a.bulge.atan() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::units::Millimeters;
    use std::f64::consts::TAU;

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

    fn circle(center: Point, radius: f64) -> Contour {
        Contour { layer: "0".into(), curves: vec![Curve::circle(center, radius)] }
    }

    /// The former exhaustive implementation is an independent oracle for
    /// candidate pruning, predicate orientation, source order and error order.
    fn exhaustive(contours: &[Contour]) -> (Vec<Vec<usize>>, Result<Vec<usize>>) {
        let polylines: Vec<_> = contours.iter().map(|c| polyline(c).ok()).collect();
        let inside = |i: usize, j: usize| match (&polylines[i], &polylines[j]) {
            (Some(a), Some(b)) => matches!(a.contains(b), PlineContainsResult::Pline1InsidePline2),
            _ => false,
        };
        let mut groups = Vec::new();
        let mut taken = vec![false; contours.len()];
        for outer in 0..contours.len() {
            if taken[outer] || (0..contours.len()).any(|j| j != outer && inside(outer, j)) {
                continue;
            }
            let members: Vec<_> = (0..contours.len())
                .filter(|&i| i == outer || (!taken[i] && inside(i, outer)))
                .collect();
            for &i in &members {
                taken[i] = true;
            }
            groups.push(members);
        }
        let depth = (|| {
            for (index, contour) in contours.iter().enumerate() {
                if contour.is_closed() {
                    polyline(contour).map_err(|reason| Error::Contour { index, reason })?;
                }
            }
            let mut depth = vec![0; contours.len()];
            for i in 0..contours.len() {
                for j in i + 1..contours.len() {
                    let (Some(a), Some(b)) = (&polylines[i], &polylines[j]) else { continue };
                    match a.contains(b) {
                        PlineContainsResult::Pline1InsidePline2 => depth[i] += 1,
                        PlineContainsResult::Pline2InsidePline1 => depth[j] += 1,
                        PlineContainsResult::Disjoint => {}
                        _ => {
                            return Err(Error::Topology(format!(
                                "contours {i} and {j} cross or touch, so which is inside is ambiguous"
                            )));
                        }
                    }
                }
            }
            Ok(depth)
        })();
        (groups, depth)
    }

    fn agrees_with_exhaustive(contours: &[Contour]) {
        let (expected_groups, expected_depths) = exhaustive(contours);
        assert_eq!(groups(contours), expected_groups);
        assert_eq!(depths(&contours.iter().collect::<Vec<_>>()), expected_depths);
    }

    #[test]
    fn indexed_topology_matches_exhaustive_for_reordered_nested_and_crossing_shapes() {
        use openlaser_core::geometry::Transform;
        let mut contours = Vec::new();
        for i in 0..24 {
            let x = f64::from(i % 6) * 80. - 200.;
            let y = f64::from(i / 6) * 80. - 100.;
            let angle = f64::from(i) * 0.13;
            let transform = Transform([angle.cos(), angle.sin(), -angle.sin(), angle.cos(), x, y]);
            contours.extend(
                [
                    square(40., Point::ORIGIN),
                    circle(Point::new(12., 12.), 8.),
                    square(2., Point::new(11., 11.)),
                    circle(Point::new(30., 30.), 4.),
                ]
                .iter()
                .map(|c| transform.contour(c)),
            );
        }
        contours.push(Contour {
            layer: "0".into(),
            curves: vec![Curve::Line { start: Point::ORIGIN, end: Point::new(500., 500.) }],
        });
        for _ in 0..3 {
            agrees_with_exhaustive(&contours);
            contours.reverse();
            contours.rotate_left(7);
        }
        contours.extend([square(40., Point::ORIGIN), square(40., Point::new(20., 20.))]);
        agrees_with_exhaustive(&contours);
        contours.reverse();
        agrees_with_exhaustive(&contours);
    }

    #[test]
    fn indexed_topology_preserves_touches_gaps_and_arc_extrema() {
        let eps = PlineContainsOptions::<f64>::default().pos_equal_eps;
        for gap in [-2., -0.5, 0., 0.5, 2., 10.] {
            for offset in [0., -100_000., 100_000.] {
                let shapes = [
                    square(10., Point::new(offset, offset)),
                    square(10., Point::new(offset + 10. + gap * eps, offset)),
                    circle(Point::new(offset + 30. + gap * eps, offset + 5.), 10.),
                    circle(Point::new(offset + 50. + 2. * gap * eps, offset + 5.), 10.),
                ];
                agrees_with_exhaustive(&shapes);
                agrees_with_exhaustive(&shapes.into_iter().rev().collect::<Vec<_>>());
            }
        }
        let major_arc = Contour {
            layer: "0".into(),
            curves: vec![
                Curve::Arc { center: Point::ORIGIN, radius: 20., start_angle: 0., sweep: 1.5 * PI },
                Curve::Line { start: Point::new(0., -20.), end: Point::new(20., 0.) },
            ],
        };
        agrees_with_exhaustive(&[
            major_arc,
            circle(Point::new(-17., 0.), 2.),
            square(5., Point::new(-22., -1.)),
        ]);
    }

    #[test]
    fn full_sheet_only_checks_nearby_contours() {
        let mut contours = Vec::new();
        for i in 0..1666 {
            let at = Point::new(f64::from(i % 49) * 30., f64::from(i / 49) * 30.);
            contours.extend([
                square(20., at),
                circle(at + Point::new(5., 10.), 2.),
                circle(at + Point::new(15., 10.), 2.),
            ]);
        }
        let topology = Topology::new(contours.iter().map(|c| polyline(c).ok()).collect());
        let pairs: usize = (0..contours.len())
            .map(|i| topology.candidates(i).into_iter().filter(|&j| j > i).count())
            .sum();
        assert_eq!(pairs, 3332, "only each plate's two holes need exact pair tests");
        assert_eq!(
            groups(&contours),
            (0..1666).map(|i| vec![i * 3, i * 3 + 1, i * 3 + 2]).collect::<Vec<_>>()
        );
        assert_eq!(depths(&contours.iter().collect::<Vec<_>>()).unwrap(), [0, 1, 1].repeat(1666));
    }

    /// An outline takes its holes along; a second outline and an open
    /// contour stand alone.
    #[test]
    fn groups_join_an_outline_with_its_holes() {
        let open = Contour {
            layer: "0".into(),
            curves: vec![Curve::Line { start: Point::new(0., -50.), end: Point::new(300., -50.) }],
        };
        let contours = [
            square(100., Point::new(0., 0.)),
            square(20., Point::new(10., 10.)),
            square(50., Point::new(200., 0.)),
            open,
        ];
        assert_eq!(groups(&contours), vec![vec![0, 1], vec![2], vec![3]]);
    }

    /// A hole inside a plate is one deep, a boss inside that hole two deep,
    /// an open contour and a disjoint one stay at zero, and crossing
    /// contours are refused.
    #[test]
    fn depths_count_the_enclosing_contours() {
        let plate = square(100., Point::ORIGIN);
        let hole = circle(Point::new(50., 50.), 20.);
        let boss = square(4., Point::new(48., 48.));
        let away = circle(Point::new(200., 0.), 5.);
        let open = Contour {
            layer: "0".into(),
            curves: vec![Curve::Line { start: Point::new(10., 10.), end: Point::new(20., 10.) }],
        };
        assert_eq!(depths(&[&plate, &hole, &boss, &away, &open]).unwrap(), vec![0, 1, 2, 0, 0]);
        let crossing = square(100., Point::new(50., 50.));
        assert!(matches!(depths(&[&plate, &crossing]), Err(Error::Topology(_))));
    }

    /// Offsetting inward shrinks a square and outward grows it, whichever
    /// way it is drawn; kerf compensation picks the side from the depth.
    #[test]
    fn offsets_go_the_right_way() {
        for contour in [square(10., Point::ORIGIN), square(10., Point::ORIGIN).reversed()] {
            let inner = offset(&contour, 1., true).unwrap();
            let outer = offset(&contour, 1., false).unwrap();
            assert!((inner.signed_area().abs() - 64.).abs() < 1e-9, "{inner:?}");
            assert!(outer.signed_area().abs() > 100., "{outer:?}");
            assert!(inner.is_closed() && outer.is_closed());
        }
        let kerf = Kerf { width: Millimeters(2.), side: Side::Auto };
        let outline = compensated(&square(10., Point::ORIGIN), &kerf, false).unwrap();
        let hole = compensated(&square(10., Point::ORIGIN), &kerf, true).unwrap();
        assert!(outline.signed_area() > 100. && hole.signed_area() < 100.);
        assert_eq!(
            offset(&square(10., Point::ORIGIN), 0., true).unwrap(),
            square(10., Point::ORIGIN)
        );
    }

    /// A circle survives compensation as one arc-only contour of the new
    /// radius, and an offset too large for the shape is refused.
    #[test]
    fn circles_stay_round_and_collapses_are_refused() {
        let shrunk = offset(&circle(Point::new(3., 4.), 5.), 1., true).unwrap();
        assert!(
            shrunk
                .curves
                .iter()
                .all(|c| matches!(c, Curve::Arc { radius, .. } if (radius - 4.).abs() < 1e-9))
        );
        assert!((shrunk.length() - 4. * TAU).abs() < 1e-9);
        assert!(offset(&circle(Point::ORIGIN, 5.), 6., true).is_err());
    }
}
