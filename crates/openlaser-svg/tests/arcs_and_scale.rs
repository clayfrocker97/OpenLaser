// SPDX-License-Identifier: GPL-3.0-or-later

//! Circles and arcs stay exact arcs, other curves are fitted within the
//! tolerance, and a document is read at the scale its units or author imply.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known vector fixtures")]

use openlaser_core::fit::deviation;
use openlaser_core::geometry::{Curve, Point};
use openlaser_svg::{Fonts, Options, Scale, Units, import, import_with};
use std::f64::consts::{PI, TAU};

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}

#[test]
fn circles_arcs_and_rounded_corners_stay_exact_arcs() {
    let drawing =
        import(svg(r#"<circle cx="30" cy="40" r="12.5" fill="none" stroke="black"/>"#).as_bytes())
            .unwrap()
            .drawing;
    let circle = &drawing.contours[0];
    assert!(circle.is_circle(), "{:?}", circle.curves);
    assert_eq!(circle.curves.len(), 1);
    let Curve::Arc { center, radius, .. } = circle.curves[0] else { unreachable!() };
    assert!(center.distance(Point::new(30., 60.)) < 1e-4, "{center:?}");
    assert!((radius - 12.5).abs() < 1e-4);

    let arc = import(
        svg(r#"<path d="M10 50 A20 20 0 0 1 50 50" fill="none" stroke="black"/>"#).as_bytes(),
    )
    .unwrap()
    .drawing;
    assert_eq!(arc.contours[0].curves.len(), 1, "{:?}", arc.contours[0].curves);
    let Curve::Arc { radius, sweep, .. } = arc.contours[0].curves[0] else { unreachable!() };
    assert!((radius - 20.).abs() < 1e-4 && (sweep.abs() - PI).abs() < 1e-6);
    assert!(arc.contours[0].start().unwrap().distance(Point::new(10., 50.)) < 1e-4);

    let rounded = import(
        svg(r#"<rect x="10" y="10" width="40" height="20" rx="5" fill="none" stroke="black"/>"#)
            .as_bytes(),
    )
    .unwrap()
    .drawing;
    let contour = &rounded.contours[0];
    assert!(contour.is_closed());
    let corners: Vec<_> =
        contour.curves.iter().filter(|c| matches!(c, Curve::Arc { .. })).collect();
    assert_eq!(corners.len(), 4, "{:?}", contour.curves);
    assert!(corners.iter().all(|c| (c.length() - 5. * PI / 2.).abs() < 1e-3));
}

#[test]
fn ellipses_and_free_curves_are_fitted_within_the_tolerance() {
    for tolerance in [0.002, 0.01, 0.05] {
        let options = Options { tolerance, ..Options::default() };
        let source =
            svg(r#"<ellipse cx="50" cy="50" rx="40" ry="15" fill="none" stroke="black"/>"#);
        let drawing = import_with(source.as_bytes(), &Fonts::default(), &options).unwrap().drawing;
        let contour = &drawing.contours[0];
        assert!(contour.is_closed());
        let arcs = contour.curves.iter().filter(|c| matches!(c, Curve::Arc { .. })).count();
        assert!(arcs >= 4 && arcs * 2 > contour.curves.len(), "{:?}", contour.curves);
        // The SVG ellipse is itself four cubic approximations, each within
        // about 0.03 % of the true ellipse.
        let at = |t: f64| Point::new(50. + 40. * t.cos(), 50. + 15. * t.sin());
        let off = deviation(at, 0., TAU, 4000, &contour.curves);
        assert!(off <= tolerance + 40. * 3e-4, "{tolerance}: {off}");
    }
    // A circle stretched unequally becomes an ellipse too.
    let stretched = svg(
        r#"<g transform="scale(2 1)"><circle cx="20" cy="50" r="10" fill="none" stroke="black"/></g>"#,
    );
    let contour = &import(stretched.as_bytes()).unwrap().drawing.contours[0];
    assert!(contour.is_closed() && !contour.is_circle());
}

#[test]
fn real_units_illustrator_inkscape_and_ambiguous_pixels() {
    let line = r#"<path d="M0 0L72 0" stroke="black"/>"#;
    let length = |source: &str, scale: Option<Scale>| {
        let options = Options { scale, ..Options::default() };
        let import = import_with(source.as_bytes(), &Fonts::default(), &options).unwrap();
        (import.drawing.length(), import.units, import.scale)
    };
    let real = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in" viewBox="0 0 144 72">{line}</svg>"#
    );
    let (mm, units, scale) = length(&real, Some(Scale::Millimetre));
    assert!((mm - 25.4).abs() < 1e-6);
    assert_eq!((units, scale), (Units::Physical, None));

    let illustrator = format!(
        r#"<?xml version="1.0"?><!-- Generator: Adobe Illustrator 27.0.0, SVG Export Plug-In --><svg xmlns="http://www.w3.org/2000/svg" width="144px" height="72px" viewBox="0 0 144 72">{line}</svg>"#
    );
    let (mm, units, scale) = length(&illustrator, None);
    assert!((mm - 25.4).abs() < 1e-6, "{mm}");
    assert_eq!((units, scale), (Units::Illustrator, Some(Scale::Dpi72)));

    let inkscape = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="144" height="72">{line}</svg>"#
    );
    let (mm, units, _) = length(&inkscape, None);
    assert!((mm - 19.05).abs() < 1e-6);
    assert_eq!(units, Units::Inkscape);

    let unknown =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 72">{line}</svg>"#);
    for (scale, expected) in
        [(None, 19.05), (Some(Scale::Dpi72), 25.4), (Some(Scale::Millimetre), 72.)]
    {
        let (mm, units, used) = length(&unknown, scale);
        assert!((mm - expected).abs() < 1e-6, "{scale:?}: {mm}");
        assert_eq!(units, Units::Ambiguous);
        assert_eq!(used, Some(scale.unwrap_or(Scale::Dpi96)));
    }
}

#[test]
fn repeated_paths_and_small_gaps_are_repaired() {
    let source = svg(
        r#"<g fill="none" stroke="black"><path d="M0 0L40 0L40 20"/><path d="M40 20.02L0 20L0 0.01"/><path d="M0 0L40 0"/></g>"#,
    );
    let import = import(source.as_bytes()).unwrap();
    assert_eq!(import.drawing.contours.len(), 1);
    assert!(import.drawing.contours[0].is_closed());
    assert_eq!(import.repairs.duplicates.len(), 1);
    assert_eq!(import.repairs.gaps.len(), 2);
}
