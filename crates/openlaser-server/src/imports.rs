// SPDX-License-Identifier: GPL-3.0-or-later

//! One persistence boundary for supported drawing formats, and the review a
//! file gets before it becomes a part: its layers, its scale, what import
//! repaired, whether it fits the bed, and which shapes are open or cross
//! themselves.

use crate::{Error, Result};
use openlaser_core::geometry::{Bounds, Drawing};
use openlaser_core::repair::Repairs;
use serde::{Deserialize, Serialize};

/// How the operator asks for a file to be imported.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportOptions {
    /// The layers to import, by name; absent imports every layer the file
    /// does not turn off or freeze.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub layers: Option<Vec<String>>,
    /// The scale of an SVG sized in pixels; absent uses what its author
    /// implies, or 96 pixels per inch.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub scale: Option<SvgScale>,
}

/// How many pixels of a pixel-sized SVG make an inch, or a millimetre.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SvgScale {
    /// 96 pixels per inch, the CSS pixel.
    Dpi96,
    /// 72 pixels per inch, as Adobe Illustrator writes.
    Dpi72,
    /// One pixel, or unitless unit, is one millimetre.
    Millimetre,
}

impl From<SvgScale> for openlaser_svg::Scale {
    fn from(scale: SvgScale) -> Self {
        match scale {
            SvgScale::Dpi96 => Self::Dpi96,
            SvgScale::Dpi72 => Self::Dpi72,
            SvgScale::Millimetre => Self::Millimetre,
        }
    }
}

impl From<openlaser_svg::Scale> for SvgScale {
    fn from(scale: openlaser_svg::Scale) -> Self {
        match scale {
            openlaser_svg::Scale::Dpi96 => Self::Dpi96,
            openlaser_svg::Scale::Dpi72 => Self::Dpi72,
            openlaser_svg::Scale::Millimetre => Self::Millimetre,
        }
    }
}

/// Who sized an SVG, and so how its pixels were read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SvgUnits {
    /// Real units: mm, cm, in, pt or pc.
    Physical,
    /// Pixels written by Adobe Illustrator, 72 per inch.
    Illustrator,
    /// Pixels written by Inkscape, 96 per inch.
    Inkscape,
    /// Pixels, or no size, from an unknown program: the scale is a guess.
    Ambiguous,
}

/// How an SVG's size was read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ScaleReview {
    /// How the document declares its size.
    pub units: SvgUnits,
    /// The scale applied to its pixels; absent when it is in real units.
    pub scale: Option<SvgScale>,
}

/// A layer of the file that holds geometry.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportLayer {
    /// The name, as the file writes it.
    pub name: String,
    /// Whether the file turns it off or freezes it.
    pub hidden: bool,
    /// Whether its geometry is imported.
    pub imported: bool,
    /// How many entities it holds.
    pub entities: usize,
}

/// Whether the drawing's size is plausible for this machine.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SizeCheck {
    /// The width and height of the drawing, in millimetres.
    pub size: [f64; 2],
    /// The width and height of the bed, when the machine is known.
    pub bed: Option<[f64; 2]>,
    /// Larger than the bed, turned either way, or than any bed when none is
    /// known.
    pub exceeds_bed: bool,
    /// So small that its units are probably wrong.
    pub tiny: bool,
}

/// What importing a file would make, for review before it is kept.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportReview {
    /// The file's name.
    pub name: String,
    /// The layers with geometry, and which are imported.
    pub layers: Vec<ImportLayer>,
    /// How an SVG's size was read; absent for DXF.
    pub scale: Option<ScaleReview>,
    /// What import repaired, and where.
    pub repairs: Repairs,
    /// Whether the size is plausible; absent when nothing is imported.
    pub size: Option<SizeCheck>,
    /// The extent of the imported geometry.
    pub bounds: Option<Bounds>,
    /// How many contours would be imported.
    pub contours: usize,
    /// The contours that are open.
    pub open: Vec<usize>,
    /// The closed contours that cross themselves.
    pub crossing: Vec<usize>,
    /// Every contour as a polyline, for the preview.
    pub outline: Vec<Vec<[f64; 2]>>,
    /// Notices to read before keeping the part.
    pub warnings: Vec<String>,
}

/// Smaller than this in both directions, a part is probably drawn in inches
/// and read as millimetres, or scaled down by mistake.
const TINY_MM: f64 = 3.;

/// Millimetres in an inch, exactly.
const MM_PER_INCH: f64 = 25.4;

/// Larger than this, with no bed to compare, a part is probably scaled up
/// by mistake.
const HUGE_MM: f64 = 5_000.;

/// A parsed drawing and what the review needs.
#[derive(Clone, Debug, Default)]
pub(crate) struct Import {
    pub drawing: Drawing,
    pub warnings: Vec<String>,
    pub layers: Vec<ImportLayer>,
    pub scale: Option<ScaleReview>,
    pub repairs: Repairs,
    /// The colours the file gives its layers.
    pub colors: Vec<openlaser_library::LayerColor>,
}

/// The colours an importer found, as the library keeps them.
fn colors(found: Vec<(String, [u8; 3])>) -> Vec<openlaser_library::LayerColor> {
    found.into_iter().map(|(layer, color)| openlaser_library::LayerColor { layer, color }).collect()
}

impl Import {
    /// The notices to show once the part is kept: the warnings, then what
    /// was repaired. A review lists the repairs on their own.
    pub(crate) fn notices(&self) -> Vec<String> {
        let mut notices = self.warnings.clone();
        notices.extend(repaired(&self.repairs));
        notices
    }
}

pub(crate) fn part(
    name: &str,
    bytes: &[u8],
    fonts: &openlaser_svg::Fonts,
    options: &ImportOptions,
) -> Result<Import> {
    if std::path::Path::new(name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("svg")) {
        svg(bytes, fonts, options)
    } else {
        dxf(bytes, options)
    }
}

fn svg(bytes: &[u8], fonts: &openlaser_svg::Fonts, options: &ImportOptions) -> Result<Import> {
    let svg = openlaser_svg::Options {
        scale: options.scale.map(Into::into),
        ..openlaser_svg::Options::default()
    };
    let imported = openlaser_svg::import_with(bytes, fonts, &svg)
        .map_err(|e| Error::Request(e.to_string()))?;
    let units = match imported.units {
        openlaser_svg::Units::Physical => SvgUnits::Physical,
        openlaser_svg::Units::Illustrator => SvgUnits::Illustrator,
        openlaser_svg::Units::Inkscape => SvgUnits::Inkscape,
        openlaser_svg::Units::Ambiguous => SvgUnits::Ambiguous,
    };
    let mut warnings = imported.warnings;
    if units == SvgUnits::Ambiguous && options.scale.is_none() {
        warnings.push(AMBIGUOUS.to_owned());
    }
    Ok(Import {
        drawing: imported.drawing,
        warnings,
        layers: Vec::new(),
        scale: Some(ScaleReview { units, scale: imported.scale.map(Into::into) }),
        repairs: imported.repairs,
        colors: colors(imported.colors),
    })
}

fn dxf(bytes: &[u8], options: &ImportOptions) -> Result<Import> {
    let dxf = openlaser_dxf::Options {
        layers: options.layers.clone(),
        ..openlaser_dxf::Options::default()
    };
    let imported =
        openlaser_dxf::import_with(bytes, &dxf).map_err(|e| Error::Request(e.to_string()))?;
    let mut warnings = Vec::new();
    if imported.units == openlaser_dxf::Units::Unspecified {
        warnings.push(UNITLESS.to_owned());
    }
    warnings.extend(imported.warnings);
    let hidden = imported.layers.iter().filter(|l| l.hidden && !l.imported).count();
    if hidden > 0 {
        warnings.push(format!(
            "{hidden} {} turned off or frozen in the file {} left out.",
            if hidden == 1 { "layer" } else { "layers" },
            if hidden == 1 { "was" } else { "were" }
        ));
    }
    warnings.extend(
        imported
            .skipped
            .iter()
            .map(|item| format!("Skipped {} at line {}.", item.entity, item.line)),
    );
    Ok(Import {
        drawing: imported.drawing,
        warnings,
        layers: imported
            .layers
            .into_iter()
            .map(|l| ImportLayer {
                name: l.name,
                hidden: l.hidden,
                imported: l.imported,
                entities: l.entities,
            })
            .collect(),
        scale: None,
        repairs: imported.repairs,
        colors: colors(imported.colors),
    })
}

/// A notice for each kind of repair made.
fn repaired(repairs: &Repairs) -> Vec<String> {
    let plural = |n: usize, one: &'static str, many: &'static str| if n == 1 { one } else { many };
    let mut notes = Vec::new();
    if let Some(widest) = repairs.gaps.iter().map(|g| g.size).reduce(f64::max) {
        notes.push(format!(
            "Closed {} {} of up to {widest:.3} mm.",
            repairs.gaps.len(),
            plural(repairs.gaps.len(), "gap", "gaps")
        ));
    }
    if !repairs.duplicates.is_empty() {
        notes.push(format!(
            "Removed {} repeated {}.",
            repairs.duplicates.len(),
            plural(repairs.duplicates.len(), "line or arc", "lines and arcs")
        ));
    }
    if !repairs.mirrored.is_empty() {
        notes.push(format!(
            "Mirrored {} upside-down {} into place.",
            repairs.mirrored.len(),
            plural(repairs.mirrored.len(), "entity", "entities")
        ));
    }
    notes
}

/// The review of a parsed file for a bed of `bed` millimetres.
pub(crate) fn review(name: &str, import: &Import, bed: Option<[f64; 2]>) -> ImportReview {
    let drawing = &import.drawing;
    let bounds = drawing.bounds();
    let size = bounds.map(|b| size_check([b.width(), b.height()], bed));
    let mut warnings = import.warnings.clone();
    if let Some(check) = size {
        warnings.extend(size_warnings(&check));
    }
    if drawing.contours.is_empty() {
        warnings.push(NOTHING.to_owned());
    }
    let open: Vec<usize> =
        (0..drawing.contours.len()).filter(|&i| !drawing.contours[i].is_closed()).collect();
    let crossing: Vec<usize> = (0..drawing.contours.len())
        .filter(|&i| openlaser_prep::crosses_itself(&drawing.contours[i]))
        .collect();
    if !crossing.is_empty() {
        warnings.push(format!(
            "{} closed {} itself; inside and outside are ambiguous there.",
            crossing.len(),
            if crossing.len() == 1 { "shape crosses" } else { "shapes cross" }
        ));
    }
    ImportReview {
        name: name.to_owned(),
        layers: import.layers.clone(),
        scale: import.scale,
        repairs: import.repairs.clone(),
        size,
        bounds,
        contours: drawing.contours.len(),
        open,
        crossing,
        outline: crate::draft::outline(drawing, 48),
        warnings,
    }
}

fn size_warnings(check: &SizeCheck) -> Vec<String> {
    let [w, h] = check.size;
    let mut warnings = Vec::new();
    if check.tiny {
        warnings.push(format!(
            "The drawing is only {w:.2} × {h:.2} mm. Drawn in inches it would be {:.1} × {:.1} mm; check its units.",
            w * MM_PER_INCH,
            h * MM_PER_INCH
        ));
    }
    if check.exceeds_bed {
        warnings.push(match check.bed {
            Some([bw, bh]) => {
                format!(
                    "The drawing is {w:.1} × {h:.1} mm, larger than the {bw:.0} × {bh:.0} mm bed."
                )
            }
            None => format!("The drawing is {w:.0} × {h:.0} mm; check its units."),
        });
    }
    warnings
}

fn size_check(size: [f64; 2], bed: Option<[f64; 2]>) -> SizeCheck {
    let (small, large) = (size[0].min(size[1]), size[0].max(size[1]));
    let exceeds_bed = match bed {
        Some([w, h]) => small > w.min(h) + 1e-6 || large > w.max(h) + 1e-6,
        None => large > HUGE_MM,
    };
    SizeCheck { size, bed, exceeds_bed, tiny: large < TINY_MM }
}

/// Refuses a drawing with nothing in it before it is kept.
pub(crate) fn require_geometry(import: &Import) -> Result<()> {
    if import.drawing.contours.is_empty() { Err(Error::Request(NOTHING.into())) } else { Ok(()) }
}

/// Parses the `options` query value, JSON, or the defaults when absent.
pub(crate) fn options(query: Option<&str>) -> Result<ImportOptions> {
    query.map_or_else(
        || Ok(ImportOptions::default()),
        |text| {
            serde_json::from_str(text).map_err(|e| Error::Request(format!("import options: {e}")))
        },
    )
}

/// A DXF without `$INSUNITS` is read in millimetres; say so, because an
/// inch drawing read that way is 25.4 times too small.
const UNITLESS: &str = "The drawing does not declare its units, so it was read in millimetres. \
     If it was drawn in inches, scale it to 2540 %.";

/// An SVG sized in pixels by an unknown program.
const AMBIGUOUS: &str = "The SVG is sized in pixels without saying how large a pixel is, so it \
     was read at 96 pixels per inch. Check its size, or choose another scale.";

/// Every layer with geometry was left out.
const NOTHING: &str = "Nothing to import: every layer with geometry is turned off or left out.";

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str =
        "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDSEC\n0\nEOF\n";

    #[test]
    fn a_unitless_dxf_says_it_was_read_in_millimetres() {
        let fonts = openlaser_svg::Fonts::default();
        let defaults = ImportOptions::default();
        let unitless =
            part("plate.dxf", LINE.as_bytes(), &fonts, &defaults).map_err(|e| e.to_string());
        assert_eq!(unitless.map(|i| i.warnings), Ok(vec![UNITLESS.to_owned()]));
        let declared = format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n{LINE}");
        let declared =
            part("plate.dxf", declared.as_bytes(), &fonts, &defaults).map_err(|e| e.to_string());
        assert_eq!(declared.map(|i| i.warnings), Ok(Vec::new()));
    }

    /// A one-millimetre line is flagged as probably tiny and open; a drawing
    /// larger than the bed either way round is flagged too.
    #[test]
    fn the_review_checks_size_against_the_bed_and_finds_open_shapes() {
        let fonts = openlaser_svg::Fonts::default();
        let import = part("plate.dxf", LINE.as_bytes(), &fonts, &ImportOptions::default()).unwrap();
        let review = review("plate.dxf", &import, Some([300., 200.]));
        let size = review.size.unwrap();
        assert!(size.tiny && !size.exceeds_bed);
        assert_eq!(review.open, [0]);
        assert!(review.warnings.iter().any(|w| w.contains("inches")));
        assert!(size_check([250., 250.], Some([300., 200.])).exceeds_bed);
        assert!(!size_check([190., 290.], Some([300., 200.])).exceeds_bed);
        assert!(size_check([6000., 10.], None).exceeds_bed);
    }

    /// A bow tie crosses itself; an ambiguous SVG says so and takes the
    /// chosen scale.
    #[test]
    fn crossing_shapes_and_ambiguous_scale_are_reported() {
        let fonts = openlaser_svg::Fonts::default();
        let bow = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path d="M0 0L100 100L100 0L0 100Z" fill="none" stroke="black"/></svg>"#;
        let import = part("bow.svg", bow.as_bytes(), &fonts, &ImportOptions::default()).unwrap();
        let review = review("bow.svg", &import, None);
        assert_eq!(review.crossing, [0]);
        assert_eq!(review.scale.unwrap().units, SvgUnits::Ambiguous);
        assert!(review.warnings.iter().any(|w| w.contains("96 pixels")));
        let chosen = ImportOptions { scale: Some(SvgScale::Millimetre), layers: None };
        let import = part("bow.svg", bow.as_bytes(), &fonts, &chosen).unwrap();
        assert!((import.drawing.bounds().unwrap().width() - 100.).abs() < 1e-6);
        assert!(!import.warnings.iter().any(|w| w.contains("96 pixels")));
        assert_eq!(options(Some(r#"{"scale":"dpi72"}"#)).unwrap().scale, Some(SvgScale::Dpi72));
        assert!(options(Some("{\"unknown\":1}")).is_err());
    }
}
