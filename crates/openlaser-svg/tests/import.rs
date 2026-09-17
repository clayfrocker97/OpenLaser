// SPDX-License-Identifier: GPL-3.0-or-later
//! Imported geometry stays in physical units and never silently loses letters.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known vector fixtures")]

use openlaser_svg::{Alignment, Family, Text, import};

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}

#[test]
fn primitives_curves_and_references_keep_their_open_or_closed_geometry() {
    let source = svg(
        r##"<defs><path id="open" d="M1 1L11 1"/></defs><g fill="none" stroke="black"><use href="#open"/><polyline points="1,3 6,8 11,3"/><polygon points="20,1 30,1 30,10"/><path d="M1 20A5 5 0 0 1 11 20"/><circle cx="30" cy="25" r="4"/><ellipse cx="50" cy="25" rx="6" ry="3"/><rect x="65" y="20" width="10" height="10" rx="2"/></g>"##,
    );
    let drawing = import(source.as_bytes()).unwrap().drawing;
    assert_eq!(drawing.contours.len(), 7);
    assert_eq!(drawing.contours.iter().filter(|c| !c.is_closed()).count(), 3);
    assert!((drawing.contours[0].length() - 10.).abs() < 1e-4);
    assert!((drawing.contours[3].length() - std::f64::consts::PI * 5.).abs() < 0.03);
    let pixels = r#"<svg xmlns="http://www.w3.org/2000/svg" width="192" height="96"><path d="M0 0L96 0" stroke="black"/></svg>"#;
    assert!((import(pixels.as_bytes()).unwrap().drawing.length() - 25.4).abs() < 1e-6);
}

#[test]
fn hidden_artwork_is_excluded_and_malformed_partial_artwork_is_refused() {
    let source = svg(
        r#"<path d="M0 0L10 0" stroke="black"/><g opacity="0"><path d="M0 0L30 30"/></g><path d="M0 0L20 0" stroke="black" stroke-opacity="0" fill="none"/><path d="M0 0L50 50" visibility="hidden"/>"#,
    );
    assert_eq!(import(source.as_bytes()).unwrap().drawing.contours.len(), 1);
    for extra in [
        r#"<path d="M0 0L20 20 BAD"/>"#,
        r#"<use href="external.svg#shape"/>"#,
        r#"<animate attributeName="x"/>"#,
    ] {
        let source = svg(&format!(r#"<path d="M0 0L10 0" stroke="black"/>{extra}"#));
        assert!(import(source.as_bytes()).is_err(), "{extra}");
    }
}

#[test]
fn missing_glyphs_are_errors_and_style_substitution_is_visible() {
    let source =
        svg(r#"<text x="0" y="20" font-size="10">A🚧B</text><path d="M0 0L10 0" stroke="black"/>"#);
    let error = import(source.as_bytes()).unwrap_err().to_string();
    assert!(error.contains("cannot draw"), "{error}");
    let source = svg(
        r#"<text x="0" y="20" font-family="Noto Sans" font-style="italic" font-size="10">AB</text>"#,
    );
    assert!(import(source.as_bytes()).unwrap().warnings.iter().any(|w| w.contains("upright")));
}

#[test]
fn text_alignment_line_breaks_and_transforms_survive_outlining() {
    for alignment in [Alignment::Left, Alignment::Center, Alignment::Right] {
        let text = Text {
            value: "HH\r\nH".into(),
            family: Family::Sans,
            size: 10.,
            bold: false,
            alignment,
        };
        let drawing = text.outlines().unwrap().drawing;
        assert_eq!(drawing.contours.len(), 3);
        let first = drawing.contours[0].bounds().unwrap();
        let last = drawing.contours[2].bounds().unwrap();
        assert!((first.min.y - last.min.y - 13.).abs() < 1e-4);
        match alignment {
            Alignment::Left => assert!((first.min.x - last.min.x).abs() < 1e-4),
            Alignment::Center => assert!(last.min.x > first.min.x),
            Alignment::Right => {
                assert!((drawing.contours[1].bounds().unwrap().max.x - last.max.x).abs() < 1e-4);
            }
        }
    }
    let plain = svg(r#"<text x="0" y="20" font-family="Noto Sans" font-size="10">H</text>"#);
    let rotated = svg(
        r#"<g transform="translate(60 10) rotate(90) scale(2)"><text x="0" y="20" font-family="Noto Sans" font-size="10">H</text></g>"#,
    );
    let a = import(plain.as_bytes()).unwrap().drawing.bounds().unwrap();
    let b = import(rotated.as_bytes()).unwrap().drawing.bounds().unwrap();
    assert!((b.width() - 2. * a.height()).abs() < 1e-4);
    assert!((b.height() - 2. * a.width()).abs() < 1e-4);
}

#[test]
fn imported_italic_and_bold_faces_replace_fallback_without_changing_prior_snapshots() {
    use openlaser_svg::{Fonts, import_with_fonts};
    let source = svg(
        r#"<text y="20" font-family="Lobster Two" font-style="italic" font-size="10">Bo &amp; Café</text>"#,
    );
    let initial = Fonts::default();
    let fallback = import_with_fonts(source.as_bytes(), &initial).unwrap();
    assert!(fallback.warnings.iter().any(|w| w.contains("unavailable")));
    let fonts = initial
        .with_font(
            "italic",
            include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf").to_vec(),
        )
        .unwrap();
    assert_eq!(fonts.faces().len(), 1);
    assert_eq!(fonts.faces()[0].style, "italic");
    let imported = import_with_fonts(source.as_bytes(), &fonts).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    assert!(
        (fallback.drawing.bounds().unwrap().width() - imported.drawing.bounds().unwrap().width())
            .abs()
            > 1.
    );
    assert!(initial.faces().is_empty());
    assert!(!import_with_fonts(source.as_bytes(), &initial).unwrap().warnings.is_empty());
    let fonts = fonts
        .with_font("bold", include_bytes!("../../../fixtures/fonts/LobsterTwo-Bold.ttf").to_vec())
        .unwrap();
    for face in fonts.faces() {
        let text = Text {
            value: "B & <O>".into(),
            family: Family::Sans,
            size: 10.,
            bold: false,
            alignment: Alignment::Left,
        };
        let result =
            import_with_fonts(text.svg_with_font(face).unwrap().as_bytes(), &fonts).unwrap();
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        // Both fixture faces use one connected outline for each visible glyph.
        assert_eq!(result.drawing.contours.len(), 5, "{}", face.name);
        assert!(result.drawing.contours.iter().all(openlaser_core::geometry::Contour::is_closed));
    }
    assert!(fonts.face("unknown").is_err());
    assert!(fonts.with_font("invalid", b"not a font".to_vec()).is_err());
    // Identical uploads do not alter selection or add another face.
    let duplicate = fonts
        .with_font(
            "italic",
            include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf").to_vec(),
        )
        .unwrap();
    assert_eq!(duplicate.faces().len(), 2);
    // A different font version cannot silently shadow the same family/style.
    let mut changed = include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf").to_vec();
    changed.push(0);
    assert!(fonts.with_font("other-version", changed).is_err());
}
