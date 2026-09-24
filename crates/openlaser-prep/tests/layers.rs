// SPDX-License-Identifier: GPL-3.0-or-later

//! Layers: the ones cut through decide parts and holes together, a marked
//! layer never makes a hole and has no machining of its own, and layers
//! run in their order.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests build known drawings")]

use openlaser_core::features::{Features, Kerf, Layer, LayerMode, Machining, Side};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use openlaser_core::units::Millimeters;

fn circle(layer: &str, x: f64, y: f64, radius: f64) -> Contour {
    Contour {
        layer: layer.into(),
        curves: vec![Curve::Arc {
            center: Point::new(x, y),
            radius,
            start_angle: 0.,
            sweep: std::f64::consts::TAU,
        }],
    }
}

fn square(layer: &str, size: f64) -> Contour {
    let p = [[0., 0.], [size, 0.], [size, size], [0., size]];
    Contour {
        layer: layer.into(),
        curves: (0..4)
            .map(|i| Curve::Line { start: p[i].into(), end: p[(i + 1) % 4].into() })
            .collect(),
    }
}

fn layer(name: &str, mode: LayerMode) -> Layer {
    Layer { name: name.into(), mode, color: None, machining: None }
}

/// A countersink: a drilled hole with the countersink's rim marked around
/// it. The rim is a mark, so the hole stays a hole.
#[test]
fn a_marked_rim_never_turns_the_hole_inside_it_into_a_part() {
    let drawing = Drawing {
        contours: vec![
            square("Cut", 60.),
            circle("Cut", 20., 20., 4.),
            circle("Rim", 20., 20., 7.),
        ],
    };
    let kerf = Kerf { width: Millimeters(0.4), side: Side::Auto };
    let unmarked = Features { kerf: Some(kerf), ..Features::default() };
    let before = openlaser_prep::prepare(&drawing, &unmarked).unwrap();
    let depth = |t: &openlaser_core::toolpath::Toolpath, source: usize| {
        t.contours.iter().find(|c| c.sources == [source]).unwrap().depth
    };
    assert_eq!(depth(&before, 1), 2, "cut through, the rim makes the hole an island");

    let marked = Features { layers: vec![layer("Rim", LayerMode::Mark)], ..unmarked.clone() };
    let after = openlaser_prep::prepare(&drawing, &marked).unwrap();
    assert_eq!(depth(&after, 1), 1, "the hole is a hole again");
    assert_eq!(depth(&after, 2), 0);
    let rim = after.contours.iter().find(|c| c.layer == "Rim").unwrap();
    assert!((rim.length() - std::f64::consts::TAU * 7.).abs() < 1e-6, "no kerf on the mark");
    let layers: Vec<_> = after.contours.iter().map(|c| c.layer.as_str()).collect();
    assert_eq!(layers, ["Rim", "Cut", "Cut"], "the smaller layer first, by default");

    // A mark with machining of its own gets it.
    let own = Machining { kerf: Some(kerf), ..Machining::default() };
    let rim_kerf = Layer { machining: Some(own), ..layer("Rim", LayerMode::Mark) };
    let kerfed = Features { layers: vec![rim_kerf], ..unmarked };
    let rim = openlaser_prep::prepare(&drawing, &kerfed).unwrap();
    let rim = rim.contours.iter().find(|c| c.layer == "Rim").unwrap();
    assert!((rim.length() - std::f64::consts::TAU * 7.2).abs() < 1e-6);
}

#[test]
fn layers_run_in_the_tables_order_and_holes_on_their_own_layer_stay_holes() {
    let drawing = Drawing {
        contours: vec![
            square("Outer", 60.),
            circle("Inner", 20., 20., 4.),
            circle("Etch", 40., 40., 3.),
        ],
    };
    let features = Features::default();
    let order = openlaser_prep::layer_order(&drawing, &features);
    assert_eq!(order, ["Etch", "Inner", "Outer"], "smaller shapes first");
    let toolpath = openlaser_prep::prepare(&drawing, &features).unwrap();
    let inner = toolpath.contours.iter().find(|c| c.layer == "Inner").unwrap();
    assert_eq!(inner.depth, 1, "a hole on its own layer is still a hole");

    let dragged = Features {
        layers: vec![
            layer("Outer", LayerMode::Cut),
            layer("Etch", LayerMode::Mark),
            layer("Inner", LayerMode::Cut),
        ],
        ..Features::default()
    };
    let toolpath = openlaser_prep::prepare(&drawing, &dragged).unwrap();
    let layers: Vec<_> = toolpath.contours.iter().map(|c| c.layer.as_str()).collect();
    assert_eq!(layers, ["Outer", "Etch", "Inner"]);
}
