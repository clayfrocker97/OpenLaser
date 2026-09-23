// SPDX-License-Identifier: GPL-3.0-or-later

//! Curves, block references, flipped extrusions, hidden layers and repairs,
//! each checked against the geometry the drawing describes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::format_collect,
    reason = "tests read known drawings built from short strings"
)]

use openlaser_core::fit::deviation;
use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_dxf::{Options, import, import_with};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn file(entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn with_blocks(blocks: &str, entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn curves(contours: &[Contour]) -> Vec<Curve> {
    contours.iter().flat_map(|c| c.curves.iter().copied()).collect()
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

#[test]
fn ellipses_become_arcs_within_the_tolerance() {
    let ellipse = "0\nELLIPSE\n8\n0\n10\n10\n20\n5\n30\n0\n11\n20\n21\n0\n31\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n";
    for tolerance in [0.002, 0.01, 0.05] {
        let options = Options { tolerance, ..Options::default() };
        let import = import_with(&file(ellipse), &options).unwrap();
        let contours = &import.drawing.contours;
        assert_eq!(contours.len(), 1);
        assert!(contours[0].is_closed());
        assert!(contours[0].curves.iter().all(|c| matches!(c, Curve::Arc { .. })));
        let at = |t: f64| Point::new(10. + 20. * t.cos(), 5. + 10. * t.sin());
        let off = both_ways(&at, 0., TAU, &contours[0].curves);
        assert!(off <= tolerance, "{tolerance}: {off}");
        assert!((contours[0].signed_area() - PI * 200.).abs() < 200. * tolerance);
    }
    // A half ellipse drawn with a flipped extrusion runs clockwise.
    let flipped = "0\nELLIPSE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n40\n0.5\n41\n0\n42\n3.141592653589793\n210\n0\n220\n0\n230\n-1\n";
    let contour = &import(&file(flipped)).unwrap().drawing.contours[0];
    assert!(contour.start().unwrap().distance(Point::new(10., 0.)) < 1e-9);
    assert!(contour.end().unwrap().distance(Point::new(-10., 0.)) < 1e-9);
    assert!(contour.point_at(contour.length() / 2.).unwrap().y < -4.99);
    // A circular ellipse is an exact arc.
    let round = "0\nELLIPSE\n10\n0\n20\n0\n11\n0\n21\n4\n40\n1\n41\n0\n42\n1.5707963267948966\n";
    let arc = import(&file(round)).unwrap().drawing.contours[0].curves.clone();
    assert_eq!(arc.len(), 1);
    let Curve::Arc { radius, sweep, .. } = arc[0] else { panic!("{arc:?}") };
    assert!((radius - 4.).abs() < 1e-12 && (sweep - FRAC_PI_2).abs() < 1e-12);
    assert!(arc[0].end().distance(Point::new(-4., 0.)) < 1e-9);
}

/// A clamped cubic spline with one span is a Bézier curve; its fit stays
/// within the tolerance of the Bézier formula, and its ends are exact.
#[test]
fn splines_become_arcs_and_lines_within_the_tolerance() {
    let bezier =
        [Point::new(0., 0.), Point::new(10., 30.), Point::new(40., -20.), Point::new(50., 10.)];
    let spline = format!(
        "0\nSPLINE\n8\nCUT\n210\n0\n220\n0\n230\n1\n70\n8\n71\n3\n72\n8\n73\n4\n74\n0\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n{}",
        bezier.iter().map(|p| format!("10\n{}\n20\n{}\n30\n0\n", p.x, p.y)).collect::<String>()
    );
    let at = |t: f64| {
        let u = 1. - t;
        bezier[0] * (u * u * u)
            + bezier[1] * (3. * u * u * t)
            + bezier[2] * (3. * u * t * t)
            + bezier[3] * (t * t * t)
    };
    for tolerance in [0.001, 0.01, 0.1] {
        let options = Options { tolerance, ..Options::default() };
        let drawing = import_with(&file(&spline), &options).unwrap().drawing;
        let fitted = curves(&drawing.contours);
        assert!(both_ways(&at, 0., 1., &fitted) <= tolerance, "{tolerance}");
        assert!(drawing.contours[0].start().unwrap().distance(bezier[0]) < 1e-9);
        assert!(drawing.contours[0].end().unwrap().distance(bezier[3]) < 1e-9);
        assert!(fitted.iter().any(|c| matches!(c, Curve::Arc { .. })));
    }
    // A rational quadratic spline draws an exact quarter circle, which fits
    // as one arc.
    let quarter = "0\nSPLINE\n8\n0\n70\n12\n71\n2\n72\n6\n73\n3\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n41\n1\n41\n0.7071067811865476\n41\n1\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n";
    let arc = curves(&import(&file(quarter)).unwrap().drawing.contours);
    assert_eq!(arc.len(), 1, "{arc:?}");
    let Curve::Arc { center, radius, .. } = arc[0] else { panic!("{arc:?}") };
    assert!(center.distance(Point::ORIGIN) < 1e-3 && (radius - 10.).abs() < 1e-3);
}

/// A spline with only fit points passes through them; a closed one stays
/// closed.
#[test]
fn fit_point_splines_pass_through_their_points() {
    let points = [[0., 0.], [20., 10.], [40., 0.], [60., 15.]];
    let fit: String = points.iter().map(|[x, y]| format!("11\n{x}\n21\n{y}\n31\n0\n")).collect();
    let open = format!("0\nSPLINE\n8\n0\n70\n8\n71\n3\n74\n4\n{fit}");
    let drawing = import(&file(&open)).unwrap().drawing;
    let fitted = curves(&drawing.contours);
    for [x, y] in points {
        let p = Point::new(x, y);
        assert!(deviation(|_| p, 0., 1., 0, &fitted) < 0.01, "{p:?}");
    }
    let closed = format!("0\nSPLINE\n8\n0\n70\n9\n71\n3\n74\n4\n{fit}");
    let drawing = import(&file(&closed)).unwrap().drawing;
    assert_eq!(drawing.contours.len(), 1);
    assert!(drawing.contours[0].is_closed());
}

const SQUARE_BLOCK: &str = "0\nBLOCK\n8\n0\n2\nTAB\n70\n0\n10\n5\n20\n0\n0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n5\n10\n0\n20\n5\n0\nARC\n8\nHOLES\n10\n7\n20\n2.5\n40\n1\n50\n0\n51\n180\n0\nENDBLK\n8\n0\n";

fn arc_of(contours: &[Contour]) -> Curve {
    *curves(contours).iter().find(|c| matches!(c, Curve::Arc { .. })).unwrap()
}

/// References place their block through rotation, scale and mirroring,
/// entities on layer 0 take the reference's layer, and a mirrored arc turns
/// the other way.
#[test]
fn block_references_rotate_scale_and_mirror() {
    let plain =
        import(&with_blocks(SQUARE_BLOCK, "0\nINSERT\n8\nPARTS\n2\nTAB\n10\n100\n20\n50\n"))
            .unwrap();
    let bounds = plain.drawing.bounds().unwrap();
    assert_eq!((bounds.min, bounds.max), (Point::new(95., 50.), Point::new(105., 55.)));
    let layers: Vec<&str> = plain.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(layers, ["PARTS", "HOLES"]);
    let Curve::Arc { sweep, .. } = arc_of(&plain.drawing.contours) else { unreachable!() };
    assert!(sweep > 0.);

    let turned = "0\nINSERT\n8\n0\n2\nTAB\n10\n0\n20\n0\n41\n2\n42\n2\n50\n90\n";
    let turned = import(&with_blocks(SQUARE_BLOCK, turned)).unwrap().drawing;
    let bounds = turned.bounds().unwrap();
    assert!(bounds.min.distance(Point::new(-10., -10.)) < 1e-9, "{bounds:?}");
    assert!(bounds.max.distance(Point::new(0., 10.)) < 1e-9, "{bounds:?}");
    let Curve::Arc { center, radius, .. } = arc_of(&turned.contours) else { unreachable!() };
    assert!(center.distance(Point::new(-5., 4.)) < 1e-9 && (radius - 2.).abs() < 1e-12);

    let mirrored = "0\nINSERT\n8\n0\n2\nTAB\n10\n0\n20\n0\n41\n-1\n";
    let mirrored = import(&with_blocks(SQUARE_BLOCK, mirrored)).unwrap().drawing;
    let bounds = mirrored.bounds().unwrap();
    assert!(
        bounds.min.distance(Point::new(-5., 0.)) < 1e-9
            && bounds.max.distance(Point::new(5., 5.)) < 1e-9
    );
    let arc = arc_of(&mirrored.contours);
    let Curve::Arc { center, sweep, .. } = arc else { unreachable!() };
    assert!(center.distance(Point::new(-2., 2.5)) < 1e-9 && sweep < 0.);
    assert!(arc.start().distance(Point::new(-3., 2.5)) < 1e-9, "{arc:?}");
    assert!(arc.point(0.5).y > 3.49, "the half circle still bulges up");
}

/// Arrays repeat along the reference's rotated axes, nested blocks compose,
/// and an unequal scale stretches circles into fitted ellipses.
#[test]
fn arrays_nesting_and_unequal_scale() {
    let array = "0\nINSERT\n8\n0\n2\nTAB\n10\n0\n20\n0\n70\n3\n71\n2\n44\n20\n45\n10\n";
    let drawing = import(&with_blocks(SQUARE_BLOCK, array)).unwrap().drawing;
    assert_eq!(drawing.contours.len(), 12);
    let bounds = drawing.bounds().unwrap();
    assert!(
        bounds.min.distance(Point::new(-5., 0.)) < 1e-9
            && bounds.max.distance(Point::new(45., 15.)) < 1e-9
    );

    let nested = format!(
        "{SQUARE_BLOCK}0\nBLOCK\n8\n0\n2\nPAIR\n10\n0\n20\n0\n0\nINSERT\n8\n0\n2\nTAB\n10\n0\n20\n0\n0\nINSERT\n8\n0\n2\nTAB\n10\n0\n20\n20\n50\n180\n0\nENDBLK\n"
    );
    let drawing =
        import(&with_blocks(&nested, "0\nINSERT\n8\nOUTER\n2\nPAIR\n10\n100\n20\n0\n")).unwrap();
    assert_eq!(drawing.drawing.contours.len(), 4);
    let bounds = drawing.drawing.bounds().unwrap();
    assert!(
        bounds.min.distance(Point::new(95., 0.)) < 1e-9
            && bounds.max.distance(Point::new(105., 20.)) < 1e-9
    );
    assert!(drawing.layers.iter().any(|l| l.name == "OUTER"));

    let circle =
        "0\nBLOCK\n2\nDOT\n10\n0\n20\n0\n0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n5\n0\nENDBLK\n";
    let stretched =
        import(&with_blocks(circle, "0\nINSERT\n2\nDOT\n10\n0\n20\n0\n41\n2\n42\n1\n")).unwrap();
    let contour = &stretched.drawing.contours[0];
    assert!(contour.is_closed() && contour.curves.len() > 2);
    let at = |t: f64| Point::new(10. * t.cos(), 5. * t.sin());
    assert!(deviation(at, 0., TAU, 2000, &contour.curves) <= 0.01);

    let looping = "0\nBLOCK\n2\nSELF\n0\nINSERT\n2\nSELF\n0\nENDBLK\n";
    assert!(import(&with_blocks(looping, "0\nINSERT\n2\nSELF\n")).is_err());
}

/// Entities drawn with extrusion (0, 0, -1) are mirrored into place and
/// reported.
#[test]
fn flipped_extrusions_are_mirrored() {
    let arc = "0\nARC\n8\n0\n10\n10\n20\n0\n40\n5\n50\n0\n51\n90\n210\n0\n220\n0\n230\n-1\n";
    let import = import(&file(arc)).unwrap();
    let arc = import.drawing.contours[0].curves[0];
    let Curve::Arc { center, sweep, .. } = arc else { panic!("{arc:?}") };
    assert!(center.distance(Point::new(-10., 0.)) < 1e-9);
    assert!(sweep < 0.);
    assert!(arc.start().distance(Point::new(-15., 0.)) < 1e-9);
    assert!(arc.end().distance(Point::new(-10., 5.)) < 1e-9);
    assert_eq!(import.repairs.mirrored.len(), 1);
    let polyline = "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n1\n20\n2\n10\n5\n20\n2\n230\n-1\n";
    let line =
        import_with(&file(polyline), &Options::default()).unwrap().drawing.contours[0].curves[0];
    assert_eq!(line, Curve::Line { start: Point::new(-1., 2.), end: Point::new(-5., 2.) });
}

/// Layers turned off or frozen are left out unless chosen, and every layer
/// with geometry is listed.
#[test]
fn hidden_layers_are_left_out_unless_chosen() {
    let tables = "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n3\n0\nLAYER\n2\nCUT\n70\n0\n62\n7\n0\nLAYER\n2\nNOTES\n70\n0\n62\n-2\n0\nLAYER\n2\nOLD\n70\n1\n62\n3\n0\nENDTAB\n0\nENDSEC\n";
    let entities = "0\nLINE\n8\nCUT\n10\n0\n20\n0\n11\n10\n21\n0\n0\nLINE\n8\nnotes\n10\n0\n20\n5\n11\n10\n21\n5\n0\nLINE\n8\nOLD\n10\n0\n20\n9\n11\n10\n21\n9\n0\nLINE\n8\nCUT\n10\n0\n20\n1\n11\n10\n21\n1\n60\n1\n";
    let bytes = format!("{tables}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n");
    let visible = import(bytes.as_bytes()).unwrap();
    assert_eq!(visible.drawing.contours.len(), 1);
    let summary: Vec<(&str, bool, bool)> =
        visible.layers.iter().map(|l| (l.name.as_str(), l.hidden, l.imported)).collect();
    assert_eq!(summary, [("CUT", false, true), ("notes", true, false), ("OLD", true, false)]);
    assert!(visible.warnings.iter().any(|w| w.contains("invisible")));
    let options = Options { layers: Some(vec!["NOTES".into()]), ..Options::default() };
    let chosen = import_with(bytes.as_bytes(), &options).unwrap();
    assert_eq!(chosen.drawing.contours.len(), 1);
    assert_eq!(chosen.drawing.contours[0].layer, "notes");
    let none = Options { layers: Some(Vec::new()), ..Options::default() };
    assert!(import_with(bytes.as_bytes(), &none).unwrap().drawing.contours.is_empty());
}

/// Small gaps close into shapes, duplicate lines go, and both are reported
/// with where they were.
#[test]
fn gaps_close_and_duplicates_go() {
    let lines = [
        [0., 0., 50., 0.],
        [50.03, 0., 50., 30.],
        [50., 30., 0., 30.],
        [0., 30., 0., 0.],
        [0., 30., 50., 30.],
        [10., 30., 20., 30.],
    ];
    let entities: String = lines
        .iter()
        .map(|[a, b, c, d]| format!("0\nLINE\n8\n0\n10\n{a}\n20\n{b}\n11\n{c}\n21\n{d}\n"))
        .collect();
    let import = import(&file(&entities)).unwrap();
    assert_eq!(import.drawing.contours.len(), 1);
    assert!(import.drawing.contours[0].is_closed());
    assert_eq!(import.repairs.gaps.len(), 1);
    assert!(import.repairs.gaps[0].at.distance(Point::new(50.015, 0.)) < 1e-9);
    assert_eq!(import.repairs.duplicates.len(), 2);
    let strict = Options { gap: 0., ..Options::default() };
    let open = import_with(&file(&entities), &strict).unwrap();
    assert!(open.repairs.gaps.is_empty());
    assert!(open.drawing.contours.iter().all(|c| !c.is_closed()));
}

/// Paper space is left out, and an external reference is skipped by name.
#[test]
fn paper_space_and_external_references_are_left_out() {
    let entities = "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n0\nLINE\n67\n1\n8\n0\n10\n0\n20\n0\n11\n500\n21\n0\n0\nINSERT\n2\nXREF\n10\n0\n20\n0\n";
    let blocks = "0\nBLOCK\n2\nXREF\n70\n4\n0\nENDBLK\n";
    let import = import(&with_blocks(blocks, entities)).unwrap();
    assert_eq!(import.drawing.contours.len(), 1);
    assert!((import.drawing.length() - 10.).abs() < 1e-12);
    assert!(import.warnings.iter().any(|w| w.contains("paper-space")));
    assert!(import.skipped.iter().any(|s| s.entity.contains("XREF")));
}
