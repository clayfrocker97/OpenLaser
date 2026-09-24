// SPDX-License-Identifier: GPL-3.0-or-later

//! SVG centerlines and text outlines in millimetres. Open subpaths stay open.
//! Circles, circular arcs and rounded corners stay exact arcs; other curves
//! are fitted with lines and arcs after their transforms, within a tolerance.
//! Images, clipping and effects need conversion to plain paths before import.
//!
//! A document sized in real units (mm, cm, in, pt, pc) is read at that size.
//! One sized in pixels, or not at all, is read at 72 pixels per inch when
//! Adobe Illustrator wrote it and at 96 when Inkscape did; otherwise its
//! scale is ambiguous, read at 96 pixels per inch unless [`Options::scale`]
//! says otherwise, and reported in [`Import::units`] for review.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture tests"))]

mod fonts;
mod paths;
mod text;
mod weld;

pub use fonts::{FontFace, Fonts};
pub use text::{Alignment, Family, Text};

use openlaser_core::geometry::Drawing;
use openlaser_core::repair::{self, Repairs};
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

/// How many pixels of a pixel-sized document make an inch, or a millimetre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// 96 pixels per inch, the CSS pixel most programs use.
    Dpi96,
    /// 72 pixels per inch, one pixel a point, as Adobe Illustrator writes.
    Dpi72,
    /// One pixel, or unitless user unit, is one millimetre.
    Millimetre,
}

impl Scale {
    /// Millimetres per CSS pixel under this scale.
    #[must_use]
    pub fn mm_per_px(self) -> f64 {
        match self {
            Self::Dpi96 => 25.4 / 96.,
            Self::Dpi72 => 25.4 / 72.,
            Self::Millimetre => 1.,
        }
    }
}

/// How the document declares its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    /// In real units: mm, cm, in, pt or pc.
    Physical,
    /// In pixels, written by Adobe Illustrator: 72 per inch.
    Illustrator,
    /// In pixels, written by Inkscape: 96 per inch.
    Inkscape,
    /// In pixels or not at all, by an unknown program: the scale is a guess.
    Ambiguous,
}

/// How to import.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// How far fitted lines and arcs may stray from the curves they replace,
    /// in millimetres; also how close two curves must lie to count as one
    /// drawn twice.
    pub tolerance: f64,
    /// The widest gap between two open ends that is closed, in millimetres.
    pub gap: f64,
    /// The scale of a document sized in pixels; `None` uses what the
    /// document's author implies, or 96 pixels per inch.
    pub scale: Option<Scale>,
}

impl Default for Options {
    fn default() -> Self {
        Self { tolerance: TOLERANCE, gap: 0.05, scale: None }
    }
}

/// Imported geometry and visible notices, including any font substitution.
#[derive(Clone, Debug)]
pub struct Import {
    /// Geometry in millimetres, with Y pointing up.
    pub drawing: Drawing,
    /// Details the operator should review after import.
    pub warnings: Vec<String>,
    /// How the document declared its size.
    pub units: Units,
    /// The scale applied to a pixel-sized document; `None` when it is sized
    /// in real units.
    pub scale: Option<Scale>,
    /// What was repaired, and where.
    pub repairs: Repairs,
    /// The colour of each layer that has one; paths outside a named group
    /// are on a layer of their colour.
    pub colors: Vec<(String, [u8; 3])>,
}

/// Imports SVG paths and text without executing scripts or loading resources.
pub fn import(bytes: &[u8]) -> Result<Import> {
    import_with_fonts(bytes, &Fonts::default())
}

/// Imports with a snapshot of the bundled and explicitly imported fonts.
pub fn import_with_fonts(bytes: &[u8], fonts: &Fonts) -> Result<Import> {
    import_with(bytes, fonts, &Options::default())
}

/// Imports with fonts and options.
pub fn import_with(bytes: &[u8], fonts: &Fonts, options: &Options) -> Result<Import> {
    if !(openlaser_core::fit::MIN_TOLERANCE..=openlaser_core::fit::MAX_TOLERANCE)
        .contains(&options.tolerance)
        || !(0. ..=1.).contains(&options.gap)
    {
        return Err(Error("the tolerance must be 0.001 to 0.5 mm and the gap 0 to 1 mm".into()));
    }
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
    let parsing = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let document = usvg::roxmltree::Document::parse_with_options(source, parsing)
        .map_err(|e| Error(format!("SVG XML: {e}")))?;
    validate_source(&document)?;
    let units = units(source, &document);
    let scale = (units != Units::Physical).then(|| {
        options.scale.unwrap_or(if units == Units::Illustrator {
            Scale::Dpi72
        } else {
            Scale::Dpi96
        })
    });
    let mm_per_px = scale.map_or(25.4 / 96., Scale::mm_per_px);
    let warnings = Mutex::new(BTreeSet::new());
    let fonts = fonts.options(&warnings);
    let tree = usvg::Tree::from_str(source, &fonts).map_err(|e| Error(format!("SVG: {e}")))?;
    let (drawing, colors) = paths::drawing(&tree, mm_per_px, options.tolerance)?;
    if drawing.contours.is_empty() {
        return Err(Error("the SVG has no visible paths or text outlines".into()));
    }
    drop(fonts);
    let (contours, duplicates) = repair::without_duplicates(drawing.contours, options.tolerance);
    let (contours, gaps) = repair::chain(&contours, options.gap);
    Ok(Import {
        drawing: Drawing { contours },
        warnings: warnings
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .into_iter()
            .collect(),
        units,
        scale,
        repairs: Repairs { gaps, duplicates, mirrored: Vec::new() },
        colors,
    })
}

/// How the root element sizes the document, and who wrote it.
fn units(source: &str, document: &usvg::roxmltree::Document<'_>) -> Units {
    let root = document.root_element();
    let physical = |name: &str| {
        root.attribute(name).is_some_and(|value| {
            let unit = value.trim().trim_start_matches(|c: char| {
                c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
            });
            matches!(unit.trim(), "mm" | "cm" | "in" | "pt" | "pc" | "q" | "Q")
        })
    };
    if physical("width") || physical("height") {
        return Units::Physical;
    }
    let head = &source[..source.len().min(4096)];
    let namespaces = root.namespaces().map(usvg::roxmltree::Namespace::uri).collect::<Vec<_>>();
    if head.contains("Adobe Illustrator")
        || namespaces.iter().any(|uri| uri.starts_with("http://ns.adobe.com/AdobeIllustrator"))
    {
        Units::Illustrator
    } else if namespaces
        .iter()
        .any(|uri| uri.starts_with("http://www.inkscape.org/namespaces/inkscape"))
    {
        Units::Inkscape
    } else {
        Units::Ambiguous
    }
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

    /// Paths outside a named group are on a layer of their colour; black
    /// stays on the default layer, and a named group keeps its name.
    #[test]
    fn colours_make_layers_and_named_groups_keep_theirs() {
        let result = import(&svg(concat!(
            "<path fill=\"none\" stroke=\"black\" d=\"M0 0L10 0\"/>",
            "<path fill=\"none\" stroke=\"#ff0000\" d=\"M0 5L10 5\"/>",
            "<g id=\"Etch\"><path fill=\"none\" stroke=\"blue\" d=\"M0 9L10 9\"/></g>",
        )))
        .unwrap();
        let layers: Vec<_> = result.drawing.contours.iter().map(|c| c.layer.as_str()).collect();
        assert_eq!(layers, ["0", "#FF0000", "Etch"]);
        assert_eq!(
            result.colors,
            [("#FF0000".to_owned(), [255, 0, 0]), ("Etch".to_owned(), [0, 0, 255])]
        );
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
        assert!((2..20).contains(&contour.curves.len()), "{:?}", contour.curves);
        assert!(
            contour.curves.iter().any(|c| matches!(c, openlaser_core::geometry::Curve::Arc { .. }))
        );
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
