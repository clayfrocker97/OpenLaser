// SPDX-License-Identifier: GPL-3.0-or-later

//! The drawings under `fixtures/import`, written by
//! `scripts/import_fixtures.py` in the manner of the exports of `AutoCAD`,
//! Fusion 360, `SolidWorks`, `LibreCAD`, QCAD, Rhino and `DraftSight`: each imports to
//! the shapes it draws, closed shapes stay closed, and every fitted curve
//! stays within the tolerance of the analytic curve it replaces.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests read known drawings")]

use openlaser_core::fit::deviation;
use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_dxf::{Import, Options, Units, import, import_with};
use std::f64::consts::TAU;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/import/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn read(name: &str) -> Import {
    import(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// What a fixture must import to.
struct Expected {
    name: &'static str,
    units: Units,
    contours: usize,
    closed: usize,
    /// Width and height, in millimetres.
    size: [f64; 2],
    /// Gaps closed, curves dropped, entities mirrored.
    repairs: [usize; 3],
    /// Layers left out.
    hidden: usize,
}

const EXPECTED: &[Expected] = &[
    Expected {
        name: "autocad-2000-bracket-mm.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 3,
        size: [80., 40.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2000-hidden-layers.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 3,
        size: [100., 60.],
        repairs: [0, 0, 0],
        hidden: 2,
    },
    Expected {
        name: "autocad-2000-open-and-crossing.dxf",
        units: Units::Millimeters,
        contours: 2,
        closed: 1,
        size: [100., 30.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2000-oversize-sheet.dxf",
        units: Units::Millimeters,
        contours: 2,
        closed: 2,
        size: [4000., 2000.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2000-stretched-block.dxf",
        units: Units::Millimeters,
        contours: 4,
        closed: 2,
        size: [100., 89.115],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2004-nested-blocks.dxf",
        units: Units::Millimeters,
        contours: 8,
        closed: 8,
        size: [50., 30.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2007-paper-space.dxf",
        units: Units::Millimeters,
        contours: 1,
        closed: 1,
        size: [50., 50.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2013-minsert-array.dxf",
        units: Units::Millimeters,
        contours: 24,
        closed: 24,
        size: [102.272, 82.141],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-2018-blocks-mirrored.dxf",
        units: Units::Millimeters,
        contours: 10,
        closed: 10,
        size: [130., 42.5],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-r12-flange-inch.dxf",
        units: Units::Unspecified,
        contours: 6,
        closed: 6,
        size: [2.5, 2.5],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "autocad-r12-spline-fit-polyline.dxf",
        units: Units::Millimeters,
        contours: 1,
        closed: 0,
        size: [80., 30.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "draftsight-mixed-layers.dxf",
        units: Units::Millimeters,
        contours: 6,
        closed: 5,
        size: [120., 80.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "fusion360-ellipses.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 3,
        size: [160.879, 96.883],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "fusion360-spline-sketch.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 1,
        size: [100., 225.149],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "librecad-gasket.dxf",
        units: Units::Millimeters,
        contours: 4,
        closed: 4,
        size: [90., 30.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "qcad-duplicates.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 2,
        size: [150., 50.],
        repairs: [0, 4, 0],
        hidden: 0,
    },
    Expected {
        name: "qcad-fit-point-splines.dxf",
        units: Units::Millimeters,
        contours: 2,
        closed: 1,
        size: [104.153, 114.996],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "rhino-rational-circle.dxf",
        units: Units::Millimeters,
        contours: 1,
        closed: 1,
        size: [50., 50.],
        repairs: [0, 0, 0],
        hidden: 0,
    },
    Expected {
        name: "solidworks-flat-pattern-inch.dxf",
        units: Units::Inches,
        contours: 3,
        closed: 2,
        size: [101.6, 50.8],
        repairs: [2, 2, 0],
        hidden: 0,
    },
    Expected {
        name: "solidworks-flipped-extrusion.dxf",
        units: Units::Millimeters,
        contours: 3,
        closed: 3,
        size: [50., 40.],
        repairs: [0, 0, 3],
        hidden: 0,
    },
];

/// Every DXF fixture is listed, and each imports to its shapes: the right
/// number of contours, the closed ones closed and continuous, its size, its
/// repairs and its hidden layers.
#[test]
fn each_export_imports_to_its_shapes() {
    let dir = format!("{}/../../fixtures/import", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| {
            std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
        })
        .collect();
    files.sort();
    let listed: Vec<&str> = EXPECTED.iter().map(|e| e.name).collect();
    assert_eq!(files, listed, "every fixture has expectations");
    for expected in EXPECTED {
        let name = expected.name;
        let import = read(name);
        let contours = &import.drawing.contours;
        assert_eq!(import.units, expected.units, "{name}");
        assert_eq!(contours.len(), expected.contours, "{name}");
        assert_eq!(contours.iter().filter(|c| c.is_closed()).count(), expected.closed, "{name}");
        assert!(contours.iter().all(Contour::is_continuous), "{name}");
        assert!(contours.iter().flat_map(|c| &c.curves).all(Curve::is_valid), "{name}");
        let bounds = import.drawing.bounds().unwrap();
        assert!((bounds.width() - expected.size[0]).abs() < 0.01, "{name}: {bounds:?}");
        assert!((bounds.height() - expected.size[1]).abs() < 0.01, "{name}: {bounds:?}");
        let repairs = &import.repairs;
        assert_eq!(
            [repairs.gaps.len(), repairs.duplicates.len(), repairs.mirrored.len()],
            expected.repairs,
            "{name}"
        );
        assert_eq!(import.layers.iter().filter(|l| !l.imported).count(), expected.hidden, "{name}");
    }
}

/// The largest distance from the analytic curve to the fit, and from the
/// fit back to a dense polyline of the curve.
fn both_ways(at: &impl Fn(f64) -> Point, from: f64, to: f64, fitted: &[Curve]) -> f64 {
    let there = deviation(at, from, to, 4000, fitted);
    let dense: Vec<Curve> = (0..4000)
        .map(|k| Curve::Line {
            start: at(from + (to - from) * f64::from(k) / 4000.),
            end: at(from + (to - from) * f64::from(k + 1) / 4000.),
        })
        .filter(Curve::is_valid)
        .collect();
    let back = fitted
        .iter()
        .flat_map(|c| (0..=32).map(move |k| c.point(f64::from(k) / 32.)))
        .map(|p| deviation(|_| p, 0., 1., 0, &dense))
        .fold(0., f64::max);
    there.max(back)
}

/// A B-spline point by the Cox–de Boor basis recursion, independent of the
/// importer's own evaluation.
#[allow(clippy::many_single_char_names, reason = "the recursion's textbook names")]
fn basis(knots: &[f64], i: usize, p: usize, t: f64) -> f64 {
    if p == 0 {
        let last = knots[knots.len() - 1];
        let inside = knots[i] <= t
            && (t < knots[i + 1] || (t >= last && knots[i + 1] >= last && knots[i] < last));
        return if inside { 1. } else { 0. };
    }
    let left = knots[i + p] - knots[i];
    let right = knots[i + p + 1] - knots[i + 1];
    let a = if left > 0. { (t - knots[i]) / left * basis(knots, i, p - 1, t) } else { 0. };
    let b = if right > 0. {
        (knots[i + p + 1] - t) / right * basis(knots, i + 1, p - 1, t)
    } else {
        0.
    };
    a + b
}

fn b_spline(controls: &[Point], knots: &[f64], degree: usize, t: f64) -> Point {
    controls
        .iter()
        .enumerate()
        .fold(Point::ORIGIN, |sum, (i, c)| sum + *c * basis(knots, i, degree, t))
}

const FUSION_CONTROLS: [(f64, f64); 7] =
    [(0., 0.), (10., 25.), (30., 30.), (50., 0.), (70., -30.), (90., -25.), (100., 0.)];
const FUSION_KNOTS: [f64; 11] = [0., 0., 0., 0., 0.25, 0.5, 0.75, 1., 1., 1., 1.];

fn contour_near(import: &Import, point: Point) -> &Contour {
    import
        .drawing
        .contours
        .iter()
        .find(|c| c.curves.iter().any(|curve| curve.start().distance(point) < 1e-6))
        .unwrap_or_else(|| panic!("no contour starts near {point:?}"))
}

#[test]
fn fitted_splines_stay_within_the_tolerance() {
    for tolerance in [0.001, 0.01, 0.05] {
        let options = Options { tolerance, ..Options::default() };
        let import = import_with(&fixture("fusion360-spline-sketch.dxf"), &options).unwrap();
        let bezier = [
            Point::new(0., 0.),
            Point::new(20., 60.),
            Point::new(80., -40.),
            Point::new(100., 20.),
        ];
        let at = |t: f64| b_spline(&bezier, &[0., 0., 0., 0., 1., 1., 1., 1.], 3, t);
        let fitted = &contour_near(&import, Point::new(0., 0.)).curves;
        assert!(both_ways(&at, 0., 1., fitted) <= tolerance, "Bézier at {tolerance}");
        for shift in [100., 200.] {
            let controls: Vec<Point> =
                FUSION_CONTROLS.iter().map(|&(x, y)| Point::new(x, y + shift)).collect();
            let at = |t: f64| b_spline(&controls, &FUSION_KNOTS, 3, t);
            let contour = contour_near(&import, Point::new(0., shift));
            let spline: Vec<Curve> =
                contour.curves.iter().copied().filter(|c| c.start().y > shift - 40.).collect();
            let off = deviation(at, 0., 1., 4000, &spline);
            assert!(off <= tolerance, "spline at {shift}, {tolerance}: {off}");
        }
    }
    // The closed wave stays closed at any tolerance.
    let coarse = Options { tolerance: 0.5, ..Options::default() };
    let import = import_with(&fixture("fusion360-spline-sketch.dxf"), &coarse).unwrap();
    assert_eq!(import.drawing.contours.iter().filter(|c| c.is_closed()).count(), 1);
}

#[test]
fn a_rational_spline_circle_stays_round() {
    for tolerance in [0.001, 0.01] {
        let options = Options { tolerance, ..Options::default() };
        let import = import_with(&fixture("rhino-rational-circle.dxf"), &options).unwrap();
        let contour = &import.drawing.contours[0];
        assert!(contour.is_closed());
        let circle = |t: f64| Point::new(40., 40.) + Point::direction(t) * 25.;
        assert!(both_ways(&circle, 0., TAU, &contour.curves) <= tolerance, "{tolerance}");
        assert!(contour.curves.iter().all(|c| matches!(c, Curve::Arc { .. })));
    }
}

#[test]
fn ellipses_stay_within_the_tolerance() {
    let import = read("fusion360-ellipses.dxf");
    let full = contour_near(&import, Point::new(90., 30.));
    let at = |t: f64| Point::new(50. + 40. * t.cos(), 30. + 20. * t.sin());
    assert!(both_ways(&at, 0., TAU, &full.curves) <= 0.01);
    // The tilted ellipse: semi-major 20√2 along 45°, semi-minor 0.3 of it.
    let (major, minor) = (Point::new(20., 20.), Point::new(-20., 20.) * 0.3);
    let tilted = |t: f64| Point::new(150., 30.) + major * t.cos() + minor * t.sin();
    let contour = contour_near(&import, Point::new(170., 50.));
    assert!(both_ways(&tilted, 0., TAU, &contour.curves) <= 0.01);
    // The slot's half ellipses meet its lines, so it closes.
    let slot = contour_near(&import, Point::new(20., 94.));
    assert!(slot.is_closed() && slot.curves.len() > 3);
}

#[test]
fn stretched_block_circles_become_ellipses_within_the_tolerance() {
    let import = read("autocad-2000-stretched-block.dxf");
    let stretched = contour_near(&import, Point::new(20., 0.));
    let at = |t: f64| Point::new(20. * t.cos(), 10. * t.sin());
    assert!(stretched.is_closed());
    assert!(both_ways(&at, 0., TAU, &stretched.curves) <= 0.01);
}

#[test]
fn fit_point_splines_pass_through_their_points() {
    let import = read("qcad-fit-point-splines.dxf");
    let open = contour_near(&import, Point::new(0., 0.));
    for (x, y) in [(0., 0.), (25., 15.), (50., 0.), (75., -15.), (100., 0.)] {
        assert!(deviation(|_| Point::new(x, y), 0., 1., 0, &open.curves) < 0.01, "({x}, {y})");
    }
}

/// Mirrored and rotated references keep their arcs on the right side, and
/// an arc drawn with a flipped extrusion still meets its lines.
#[test]
fn mirrored_references_and_flipped_arcs_land_where_drawn() {
    let import = read("autocad-2018-blocks-mirrored.dxf");
    // The mirrored copy at x = 120 spans x 100..120, its slot centred at
    // 120 - 12 = 108, so its round ends are at 106 and 110 ± 2.
    let slots: Vec<&Contour> =
        import.drawing.contours.iter().filter(|c| c.layer == "HOLES").collect();
    let mirrored = slots
        .iter()
        .find(|c| c.bounds().unwrap().min.x > 99. && c.bounds().unwrap().max.x < 121.)
        .unwrap();
    let bounds = mirrored.bounds().unwrap();
    assert!((bounds.min.x - 104.).abs() < 1e-9 && (bounds.max.x - 112.).abs() < 1e-9, "{bounds:?}");
    assert!(mirrored.is_closed());
    let flipped = read("solidworks-flipped-extrusion.dxf");
    let tab = contour_near(&flipped, Point::new(0., 0.));
    assert!(tab.is_closed());
    assert!((tab.bounds().unwrap().max.x - 50.).abs() < 1e-9);
}

/// Hidden layers can be chosen back.
#[test]
fn hidden_layers_can_be_chosen() {
    let options =
        Options { layers: Some(vec!["CUT".into(), "CONSTRUCTION".into()]), ..Options::default() };
    let import = import_with(&fixture("autocad-2000-hidden-layers.dxf"), &options).unwrap();
    assert_eq!(import.drawing.contours.len(), 4);
    assert!(import.drawing.contours.iter().any(|c| c.layer == "CONSTRUCTION"));
    assert!(!import.drawing.contours.iter().any(|c| c.layer == "ETCH"));
}
