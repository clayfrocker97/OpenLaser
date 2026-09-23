// SPDX-License-Identifier: GPL-3.0-or-later

//! The SVG drawings under `fixtures/import`, written by
//! `scripts/import_fixtures.py` in the manner of Inkscape, Illustrator and
//! Fusion 360 exports: each is read at its real size, circles stay exact,
//! closed shapes stay closed, and fitted curves stay within the tolerance.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known vector fixtures")]

use openlaser_core::fit::deviation;
use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_svg::{Import, Scale, Units, import};
use std::f64::consts::TAU;

fn read(name: &str) -> Import {
    let path = format!("{}/../../fixtures/import/{name}", env!("CARGO_MANIFEST_DIR"));
    import(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

struct Expected {
    name: &'static str,
    units: Units,
    scale: Option<Scale>,
    contours: usize,
    closed: usize,
    circles: usize,
    size: [f64; 2],
    /// Gaps closed and curves dropped.
    repairs: [usize; 2],
}

const EXPECTED: &[Expected] = &[
    Expected {
        name: "fusion360-sketch-cm.svg",
        units: Units::Physical,
        scale: None,
        contours: 3,
        closed: 2,
        circles: 1,
        size: [100., 60.],
        repairs: [0, 0],
    },
    Expected {
        name: "generic-export-unitless.svg",
        units: Units::Ambiguous,
        scale: Some(Scale::Dpi96),
        contours: 3,
        closed: 3,
        circles: 2,
        size: [100.542, 47.625],
        repairs: [0, 0],
    },
    Expected {
        name: "illustrator-cc-logo-72dpi.svg",
        units: Units::Illustrator,
        scale: Some(Scale::Dpi72),
        contours: 4,
        closed: 3,
        circles: 1,
        size: [101.6, 50.8],
        repairs: [0, 0],
    },
    Expected {
        name: "illustrator-cs6-gaps-duplicates.svg",
        units: Units::Physical,
        scale: None,
        contours: 2,
        closed: 2,
        circles: 1,
        size: [63.5, 28.222],
        repairs: [2, 2],
    },
    Expected {
        name: "inkscape-0.92-bracket-px.svg",
        units: Units::Inkscape,
        scale: Some(Scale::Dpi96),
        contours: 2,
        closed: 2,
        circles: 1,
        size: [100.0125, 50.006],
        repairs: [0, 0],
    },
    Expected {
        name: "inkscape-1.2-transforms.svg",
        units: Units::Physical,
        scale: None,
        contours: 3,
        closed: 2,
        circles: 0,
        size: [85.856, 75.857],
        repairs: [0, 0],
    },
    Expected {
        name: "inkscape-1.3-plate-mm.svg",
        units: Units::Physical,
        scale: None,
        contours: 4,
        closed: 3,
        circles: 1,
        size: [110., 70.],
        repairs: [0, 0],
    },
];

#[test]
fn each_export_imports_at_its_real_size() {
    let dir = format!("{}/../../fixtures/import", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| {
            std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        })
        .collect();
    files.sort();
    let listed: Vec<&str> = EXPECTED.iter().map(|e| e.name).collect();
    assert_eq!(files, listed, "every fixture has expectations");
    for expected in EXPECTED {
        let name = expected.name;
        let import = read(name);
        let contours = &import.drawing.contours;
        assert_eq!((import.units, import.scale), (expected.units, expected.scale), "{name}");
        assert_eq!(contours.len(), expected.contours, "{name}");
        assert_eq!(contours.iter().filter(|c| c.is_closed()).count(), expected.closed, "{name}");
        assert_eq!(contours.iter().filter(|c| c.is_circle()).count(), expected.circles, "{name}");
        assert!(contours.iter().all(Contour::is_continuous), "{name}");
        let bounds = import.drawing.bounds().unwrap();
        assert!((bounds.width() - expected.size[0]).abs() < 0.01, "{name}: {bounds:?}");
        assert!((bounds.height() - expected.size[1]).abs() < 0.01, "{name}: {bounds:?}");
        assert_eq!(
            [import.repairs.gaps.len(), import.repairs.duplicates.len()],
            expected.repairs,
            "{name}"
        );
    }
}

/// Circles are one exact arc of their drawn radius, rounded corners and arc
/// commands are exact arcs, and the ellipse follows its formula.
#[test]
fn circles_stay_exact_and_ellipses_stay_within_the_tolerance() {
    let import = read("inkscape-1.3-plate-mm.svg");
    let circle = import.drawing.contours.iter().find(|c| c.is_circle()).unwrap();
    let Curve::Arc { center, radius, .. } = circle.curves[0] else { unreachable!() };
    assert!(center.distance(Point::new(30., 40.)) < 1e-4 && (radius - 12.).abs() < 1e-4);
    let plate =
        import.drawing.contours.iter().find(|c| c.bounds().unwrap().width() > 100.).unwrap();
    let corners: Vec<&Curve> =
        plate.curves.iter().filter(|c| matches!(c, Curve::Arc { .. })).collect();
    assert_eq!(corners.len(), 4);
    let ellipse = import
        .drawing
        .contours
        .iter()
        .find(|c| (c.bounds().unwrap().width() - 40.).abs() < 0.01)
        .unwrap();
    let at = |t: f64| Point::new(80. + 20. * t.cos(), 40. + 8. * t.sin());
    // The SVG ellipse is four cubic approximations, each within 0.03 % of the
    // radius of the true ellipse, then fitted within 0.01 mm.
    assert!(deviation(at, 0., TAU, 4000, &ellipse.curves) <= 0.01 + 20. * 3e-4);
    let half = import.drawing.contours.iter().find(|c| !c.is_closed()).unwrap();
    assert_eq!(half.curves.len(), 1, "two quarter arcs join: {:?}", half.curves);
    let Curve::Arc { radius, sweep, .. } = half.curves[0] else { unreachable!() };
    assert!((radius - 10.).abs() < 1e-4 && (sweep.abs() - TAU / 2.).abs() < 1e-6);
}

/// A free Bézier stays within the tolerance of its formula.
#[test]
fn free_curves_stay_within_the_tolerance() {
    let import = read("fusion360-sketch-cm.svg");
    let open = import.drawing.contours.iter().find(|c| !c.is_closed()).unwrap();
    // In millimetres with Y up: the SVG's 6 cm height flips it.
    let p = |x: f64, y: f64| Point::new(x * 10., 60. - y * 10.);
    let cubic = |a: Point, b: Point, c: Point, d: Point, t: f64| {
        let u = 1. - t;
        a * (u * u * u) + b * (3. * u * u * t) + c * (3. * u * t * t) + d * (t * t * t)
    };
    let first = |t: f64| cubic(p(6., 1.), p(7., 0.), p(9., 2.), p(8., 3.), t);
    let second = |t: f64| cubic(p(8., 3.), p(7., 4.), p(9., 5.), p(7., 5.), t);
    let fitted = &open.curves;
    assert!(deviation(first, 0., 1., 2000, fitted) <= 0.01);
    assert!(deviation(second, 0., 1., 2000, fitted) <= 0.01);
    assert!(fitted.len() < 40, "{}", fitted.len());
}
