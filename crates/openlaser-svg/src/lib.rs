// SPDX-License-Identifier: GPL-3.0-or-later

//! SVG centerlines and text outlines in millimetres. Open subpaths stay open;
//! Bézier curves are flattened after their transforms to a 0.01 mm tolerance.
//! Images, clipping and effects need conversion to plain paths before import.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture tests"))]

mod fonts;
mod paths;
mod text;
mod weld;

pub use fonts::{FontFace, Fonts};
pub use text::{Alignment, Family, Text};

use openlaser_core::geometry::Drawing;
use std::collections::BTreeSet;
use std::sync::Mutex;

/// Maximum input size, shared with DXF uploads.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Maximum deviation of a flattened Bézier chord in millimetres.
pub const TOLERANCE: f64 = 0.01;

/// Why a vector import could not preserve its geometry.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

/// The result of an SVG or text conversion.
pub type Result<T> = std::result::Result<T, Error>;

/// Imported geometry and visible notices, including any font substitution.
#[derive(Clone, Debug)]
pub struct Import {
    /// Geometry in millimetres, with Y pointing up.
    pub drawing: Drawing,
    /// Details the operator should review after import.
    pub warnings: Vec<String>,
}

/// Imports SVG paths and text without executing scripts or loading resources.
pub fn import(bytes: &[u8]) -> Result<Import> {
    import_with_fonts(bytes, &Fonts::default())
}

/// Imports with a snapshot of the bundled and explicitly imported fonts.
pub fn import_with_fonts(bytes: &[u8], fonts: &Fonts) -> Result<Import> {
    if bytes.len() > MAX_BYTES {
        return Err(Error("the SVG exceeds 64 MiB".into()));
    }
    let source = std::str::from_utf8(bytes).map_err(|e| Error(format!("SVG encoding: {e}")))?;
    // Exporters often add a document type declaration, which is harmless on
    // its own. Entity declarations can expand without bound, so they are
    // refused before anything expands them.
    if source.contains("<!ENTITY") {
        return Err(Error(
            "SVG entity declarations are not supported; export the drawing as plain SVG".into(),
        ));
    }
    let options = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let document = usvg::roxmltree::Document::parse_with_options(source, options)
        .map_err(|e| Error(format!("SVG XML: {e}")))?;
    validate_source(&document)?;
    let warnings = Mutex::new(BTreeSet::new());
    let options = fonts.options(&warnings);
    let tree = usvg::Tree::from_str(source, &options).map_err(|e| Error(format!("SVG: {e}")))?;
    let drawing = paths::drawing(&tree)?;
    if drawing.contours.is_empty() {
        return Err(Error("the SVG has no visible paths or text outlines".into()));
    }
    drop(options);
    Ok(Import {
        drawing,
        warnings: warnings
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .into_iter()
            .collect(),
    })
}

fn validate_source(document: &usvg::roxmltree::Document<'_>) -> Result<()> {
    for node in document.descendants().filter(usvg::roxmltree::Node::is_element) {
        let name = node.tag_name().name();
        if matches!(
            name,
            "image"
                | "foreignObject"
                | "script"
                | "animate"
                | "animateMotion"
                | "animateTransform"
                | "set"
        ) {
            return Err(Error(format!(
                "SVG {name} is not a static cutting path; convert the artwork to paths before importing"
            )));
        }
        if name == "path"
            && let Some(data) = node.attribute("d")
        {
            for segment in svgtypes::PathParser::from(data) {
                segment.map_err(|e| Error(format!("SVG path data: {e}")))?;
            }
        }
        for attribute in node.attributes().filter(|a| a.name() == "href") {
            if !attribute.value().starts_with('#') {
                return Err(Error(
                    "SVG external references must be embedded as paths before importing".into(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(body: &str) -> Vec<u8> {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100mm\" height=\"50mm\" viewBox=\"0 0 100 50\">{body}</svg>").into_bytes()
    }

    #[test]
    fn open_subpaths_are_not_closed_and_units_and_transforms_are_applied() {
        let result = import(&svg("<g transform=\"translate(3 4) scale(2)\"><path fill=\"none\" stroke=\"black\" d=\"M1 2L11 2 M20 3L25 8L30 3Z\"/></g>")).unwrap();
        assert_eq!(result.drawing.contours.len(), 2);
        let open = &result.drawing.contours[0];
        assert!(!open.is_closed());
        assert!((open.curves[0].point(0.).x - 5.).abs() < 1e-5);
        assert!((open.curves[0].point(0.).y - 42.).abs() < 1e-5);
        assert!((open.length() - 20.).abs() < 1e-5);
        assert!(result.drawing.contours[1].is_closed());
    }

    #[test]
    fn quadratic_and_cubic_paths_keep_open_endpoints() {
        let result = import(&svg(
            "<path fill=\"none\" stroke=\"black\" d=\"M0 0Q10 20 20 0 C25 -10 35 -10 40 0\"/>",
        ))
        .unwrap();
        let contour = &result.drawing.contours[0];
        assert!(!contour.is_closed());
        assert!(contour.curves.len() > 20);
        assert!((contour.curves.last().unwrap().point(1.).x - 40.).abs() < 1e-5);
        assert!(
            contour
                .curves
                .windows(2)
                .all(|pair| pair[0].point(1.).distance(pair[1].point(0.)) < 1e-8)
        );
    }

    #[test]
    fn text_is_outlined_with_holes_and_xml_characters_are_literal() {
        for family in [Family::Sans, Family::Serif, Family::Mono] {
            let text = Text {
                value: "B & <O>\nCafé".into(),
                family,
                size: 10.,
                bold: true,
                alignment: Alignment::Center,
            };
            let result = import(text.svg().unwrap().as_bytes()).unwrap();
            assert!(result.warnings.is_empty(), "{:?}", result.warnings);
            assert!(result.drawing.contours.len() > 10);
            assert!(
                result.drawing.contours.iter().all(openlaser_core::geometry::Contour::is_closed)
            );
            assert!(result.drawing.bounds().unwrap().height() > 15.);
        }
        let text = Text {
            value: "H".into(),
            family: Family::Sans,
            size: 10.,
            bold: false,
            alignment: Alignment::Left,
        };
        let outlined = text.outlines().unwrap();
        assert!(outlined.drawing.bounds().unwrap().min.y.abs() < 1e-5);
    }

    #[test]
    fn font_substitution_is_reported_and_non_geometry_is_refused() {
        let result = import(&svg(
            "<text x=\"1\" y=\"20\" font-family=\"MissingFont\" font-size=\"10\">AB</text>",
        ))
        .unwrap();
        assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
        for body in [
            "<image href=\"file:///missing.png\"/>",
            "<script>alert(1)</script>",
            "<defs><clipPath id=\"c\"><rect width=\"2\" height=\"2\"/></clipPath></defs><path clip-path=\"url(#c)\" d=\"M0 0L5 0L5 5Z\"/>",
        ] {
            assert!(import(&svg(body)).is_err());
        }
        assert!(import(b"not SVG").is_err());
        assert!(import(&svg("<path d=\"M0 0\"/>")).is_err());
    }
}
