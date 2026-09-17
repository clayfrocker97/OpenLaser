// SPDX-License-Identifier: GPL-3.0-or-later

//! Common-edge acceptance uses physical lengths and contiguous output paths.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known geometry fixtures")]

use openlaser_core::features::{CommonEdges, Features, OrderStrategy};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use openlaser_core::toolpath::Toolpath;

fn polygon(points: &[[f64; 2]]) -> Contour {
    Contour {
        layer: "cut".into(),
        curves: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(a, b)| Curve::Line { start: Point::from(*a), end: Point::from(*b) })
            .collect(),
    }
}
fn square(x: f64, y: f64, size: f64) -> Contour {
    polygon(&[[x, y], [x + size, y], [x + size, y + size], [x, y + size]])
}
fn features(overcut: bool) -> Features {
    let mut features = Features {
        common: Some(CommonEdges { contours: vec![0, 1], tolerance: 0.01, allow_overcut: overcut }),
        ..Features::default()
    };
    features.order.strategy = OrderStrategy::AsDrawn;
    features
}
fn length(path: &Toolpath) -> f64 {
    path.contours.iter().map(openlaser_core::toolpath::PreparedContour::length).sum()
}

#[test]
fn adjacent_parts_cut_shared_spans_once_and_retain_continuity_and_provenance() {
    let drawing = Drawing { contours: vec![square(0., 0., 10.), square(10., 0., 10.)] };
    assert!(openlaser_prep::prepare(&drawing, &Features::default()).is_err());
    let path = openlaser_prep::prepare(&drawing, &features(false)).unwrap();
    assert!((length(&path) - 70.).abs() < 1e-8);
    assert!(
        (path.contours.iter().flat_map(|c| &c.film).map(Curve::length).sum::<f64>() - 70.).abs()
            < 1e-8
    );
    for contour in &path.contours {
        assert_eq!(contour.sources, [0, 1]);
        for pair in contour.segments.windows(2) {
            assert!(pair[0].curve.end().distance(pair[1].curve.start()) < 1e-8);
        }
        for segment in &contour.segments {
            let source = segment.source.unwrap();
            assert!((0. ..=1.).contains(&source.start) && (0. ..=1.).contains(&source.end));
        }
    }
}

#[test]
fn partial_common_edges_are_split_and_overcut_is_explicit() {
    let drawing = Drawing { contours: vec![square(0., 0., 10.), square(10., 2., 4.)] };
    let path = openlaser_prep::prepare(&drawing, &features(false)).unwrap();
    assert!((length(&path) - 52.).abs() < 1e-8);
    // Put the shared edge in the middle of the second path: retracing joins
    // that path only when explicitly permitted.
    let drawing = Drawing {
        contours: vec![square(0., 0., 10.), polygon(&[[14., 2.], [14., 6.], [10., 6.], [10., 2.]])],
    };
    let without = openlaser_prep::prepare(&drawing, &features(false)).unwrap();
    let with = openlaser_prep::prepare(&drawing, &features(true)).unwrap();
    assert!((length(&without) - 52.).abs() < 1e-8);
    assert!((length(&with) - 56.).abs() < 1e-8);
    assert!(with.contours.len() < without.contours.len());
}

#[test]
fn only_selected_nonoverlapping_compatible_contours_can_share_edges() {
    for contours in [
        vec![square(0., 0., 10.), square(9., 0., 10.)],
        vec![square(0., 0., 10.), square(10., 0., 10.), square(20., 0., 10.)],
    ] {
        assert!(openlaser_prep::prepare(&Drawing { contours }, &features(false)).is_err());
    }
    let mut drawing = Drawing { contours: vec![square(0., 0., 10.), square(10., 0., 10.)] };
    drawing.contours[1].layer = "different".into();
    assert!(openlaser_prep::prepare(&drawing, &features(false)).is_err());
    let drawing = Drawing { contours: vec![square(0., 0., 10.), square(10.005, 0., 10.)] };
    let path = openlaser_prep::prepare(&drawing, &features(false)).unwrap();
    assert!((length(&path) - 70.).abs() < 0.02);
    // Tolerance removes the second span; it cannot insert a link across the gap.
    for contour in path.contours {
        for segment in contour.segments {
            assert!(segment.curve.length() > 0.01);
        }
    }
}

#[test]
fn partial_arcs_share_without_becoming_chords() {
    let arc = |start_angle, sweep| Contour {
        layer: "cut".into(),
        curves: vec![Curve::Arc { center: Point::new(0., 0.), radius: 10., start_angle, sweep }],
    };
    let drawing = Drawing {
        contours: vec![
            arc(0., std::f64::consts::PI),
            arc(std::f64::consts::FRAC_PI_2, std::f64::consts::PI),
        ],
    };
    let path = openlaser_prep::prepare(&drawing, &features(false)).unwrap();
    assert!((length(&path) - 15. * std::f64::consts::PI).abs() < 1e-8);
    assert!(
        path.contours
            .iter()
            .flat_map(|c| &c.segments)
            .all(|s| matches!(s.curve, Curve::Arc { .. }))
    );
}
