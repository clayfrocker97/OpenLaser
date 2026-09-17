// SPDX-License-Identifier: GPL-3.0-or-later

//! DXF import into [`openlaser_core`] geometry: lines, arcs, circles and
//! polylines with bulges, and plain TEXT/MTEXT outlines, in millimetres.
//!
//! Our own rules, snapshot-tested against fixture drawings. Geometry is read
//! exactly, so an arc stays an arc. Entities that carry nothing to cut are
//! skipped and reported; geometry this beta cannot cut, such as splines and
//! block references, is refused rather than dropped. Entities on one layer
//! whose ends meet are chained into contours, and a junction where more than
//! two ends meet is left alone rather than guessed at.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests read known drawings and compare exact values"
    )
)]

mod assemble;
mod entities;
mod pairs;

use openlaser_core::geometry::Drawing;

/// Largest file accepted.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Why a file could not be imported.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is a binary DXF; only ASCII is read.
    #[error("binary DXF is not supported; save as ASCII DXF")]
    Binary,
    /// Legacy non-UTF-8 strings need conversion, never replacement glyphs.
    #[error(
        "DXF text encoding is not UTF-8; save as AutoCAD 2007 or later ASCII DXF, or escape Unicode characters"
    )]
    Encoding,
    /// The file is larger than [`MAX_BYTES`].
    #[error("the DXF is larger than {MAX_BYTES} bytes")]
    TooLarge,
    /// The file is not a sequence of group code and value pairs.
    #[error("line {line}: {reason}")]
    Syntax {
        /// The line of the problem, counted from one.
        line: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// An entity has a value that cannot be cut as drawn.
    #[error("line {line}: {entity}: {reason}")]
    Entity {
        /// The line the entity starts on.
        line: usize,
        /// The entity type.
        entity: String,
        /// What is wrong.
        reason: String,
    },
    /// An entity type whose geometry this beta cannot cut.
    #[error("line {line}: {entity} entities are not supported; explode or redraw them")]
    Unsupported {
        /// The line the entity starts on.
        line: usize,
        /// The entity type.
        entity: String,
    },
    /// A `$INSUNITS` code other than millimetres or inches.
    #[error("drawing units code {0} is not supported; save in millimetres or inches")]
    Units(u32),
    /// Nothing to cut.
    #[error("the drawing has no lines, arcs, circles, polylines or text outlines")]
    Empty,
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// The drawing units the file declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    /// `$INSUNITS` 4.
    Millimeters,
    /// `$INSUNITS` 1, converted to millimetres.
    Inches,
    /// No usable declaration; the coordinates were taken as millimetres.
    Unspecified,
}

/// An entity that was read past because it carries nothing to cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The entity type.
    pub entity: String,
    /// The line it starts on, counted from one.
    pub line: usize,
}

/// An imported drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    /// The geometry, in millimetres, chained into contours.
    pub drawing: Drawing,
    /// The units the file declared.
    pub units: Units,
    /// How many entities carried geometry.
    pub entities: usize,
    /// The entities that were skipped.
    pub skipped: Vec<Skipped>,
    /// Font substitutions and other import details to review.
    pub warnings: Vec<String>,
}

/// Imports an ASCII DXF file.
pub fn import(bytes: &[u8]) -> Result<Import> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err(Error::Binary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Encoding)?;
    let pairs = pairs::tokenize(text.trim_start_matches('\u{feff}'))?;
    let (mut read, units) = Sections::read(&pairs)?;
    if units == Units::Inches {
        scale_to_mm(&mut read);
    }
    let warnings = read.outline_text(if units == Units::Inches { 25.4 } else { 1. })?;
    if read.contours.is_empty() {
        return Err(Error::Empty);
    }
    Ok(Import {
        drawing: Drawing { contours: assemble::chain(&read.contours) },
        units,
        entities: read.entities,
        skipped: read.skipped,
        warnings,
    })
}

fn scale_to_mm(read: &mut entities::Read) {
    for contour in &mut read.contours {
        for curve in &mut contour.curves {
            *curve = curve.scaled(25.4);
        }
    }
}

struct Sections<'a> {
    pairs: &'a [pairs::Pair<'a>],
    at: usize,
    section: &'a str,
    units: Units,
    entities: entities::Read,
}

impl<'a> Sections<'a> {
    fn read(pairs: &'a [pairs::Pair<'a>]) -> Result<(entities::Read, Units)> {
        let mut reader = Self {
            pairs,
            at: 0,
            section: "",
            units: Units::Unspecified,
            entities: entities::Read::default(),
        };
        while reader.at < pairs.len() {
            if reader.control()? || reader.header()? {
                continue;
            }
            reader.at += 1;
        }
        Ok((reader.entities, reader.units))
    }

    /// Section boundaries and entity records advance the same input cursor.
    fn control(&mut self) -> Result<bool> {
        let pair = self.pairs[self.at];
        if pair.code != 0 {
            return Ok(false);
        }
        match pair.value.to_ascii_uppercase().as_str() {
            "SECTION" => {
                let name =
                    self.pairs.get(self.at + 1).filter(|p| p.code == 2).ok_or(Error::Syntax {
                        line: pair.line,
                        reason: "SECTION must be followed by its name",
                    })?;
                self.section = name.value;
                self.at += 2;
            }
            "ENDSEC" => {
                self.section = "";
                self.at += 1;
            }
            "EOF" => self.at = self.pairs.len(),
            _ if self.section.eq_ignore_ascii_case("ENTITIES") => {
                self.at = entities::read(self.pairs, self.at, &mut self.entities)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn header(&mut self) -> Result<bool> {
        let pair = self.pairs[self.at];
        if self.section.eq_ignore_ascii_case("HEADER")
            && pair.code == 9
            && pair.value.eq_ignore_ascii_case("$INSUNITS")
            && let Some(code) = self.pairs.get(self.at + 1).filter(|p| p.code == 70)
        {
            self.units = declared_units(code.value)?;
            self.at += 2;
            return Ok(true);
        }
        Ok(false)
    }
}

fn declared_units(value: &str) -> Result<Units> {
    match value.trim().parse::<u32>() {
        Ok(0) | Err(_) => Ok(Units::Unspecified),
        Ok(1) => Ok(Units::Inches),
        Ok(4) => Ok(Units::Millimeters),
        Ok(other) => Err(Error::Units(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(entities: &str, units: &str) -> Vec<u8> {
        format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
        .into_bytes()
    }

    /// Geometry we cannot cut is refused by name, and files that are not
    /// ASCII pairs, have no geometry, or use other units are refused too.
    #[test]
    fn unsupported_files_are_refused_with_the_reason() {
        for (entity, name) in [
            ("0\nSPLINE\n8\n0\n", "SPLINE"),
            ("0\nINSERT\n8\n0\n2\nBLOCK\n10\n0\n20\n0\n", "INSERT"),
            ("0\nELLIPSE\n8\n0\n", "ELLIPSE"),
        ] {
            let error = import(&file(entity, "4")).unwrap_err();
            assert_eq!(error, Error::Unsupported { line: 15, entity: name.into() }, "{entity}");
        }
        assert_eq!(import(b"AutoCAD Binary DXF\r\n\x1a\0").unwrap_err(), Error::Binary);
        assert_eq!(
            import(b"0\nSECTION\n2\n").unwrap_err(),
            Error::Syntax { line: 3, reason: "a group code needs a value line" }
        );
        assert_eq!(import(&file("0\nPOINT\n8\n0\n10\n0\n20\n0\n", "4")).unwrap_err(), Error::Empty);
        assert_eq!(
            import(&file("0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n", "2")).unwrap_err(),
            Error::Units(2)
        );
    }

    /// Values that make an entity uncuttable as drawn are refused: a line
    /// off the XY plane, a polyline with the wrong vertex count, an arc
    /// with no extent, a wide polyline.
    #[test]
    fn bad_entity_values_are_refused() {
        for entity in [
            "0\nLINE\n10\n0\n20\n0\n30\n1\n11\n1\n21\n1\n",
            "0\nLWPOLYLINE\n90\n3\n70\n0\n10\n0\n20\n0\n10\n1\n20\n1\n",
            "0\nARC\n10\n0\n20\n0\n40\n1\n50\n30\n51\n30\n",
            "0\nLWPOLYLINE\n90\n2\n70\n0\n43\n0.5\n10\n0\n20\n0\n10\n1\n20\n1\n",
            "0\nCIRCLE\n10\n0\n20\n0\n40\n0\n",
            "0\nLINE\n10\n0\n20\n0\n11\n0\n21\n0\n",
        ] {
            let result = import(&file(entity, "4"));
            assert!(matches!(result, Err(Error::Entity { line: 15, .. })), "{entity}: {result:?}");
        }
    }

    /// A byte order mark, Windows line endings, lower-case names and
    /// indented group codes are all read.
    #[test]
    fn tolerant_of_encodings_and_case() {
        let text = "\u{feff}  0\r\nsection\r\n  2\r\nentities\r\n  0\r\nline\r\n 10\r\n0\r\n 20\r\n0\r\n 11\r\n5\r\n 21\r\n0\r\n  0\r\nendsec\r\n  0\r\neof\r\n";
        let import = import(text.as_bytes()).unwrap();
        assert_eq!(import.units, Units::Unspecified);
        assert_eq!(import.entities, 1);
        assert!((import.drawing.length() - 5.).abs() < 1e-12);
    }

    #[test]
    fn polyline_vertices_share_planar_zero_width_admission() {
        let line = "0\nLINE\n10\n0\n20\n0\n11\n10\n21\n0\n";
        let light =
            "0\nLWPOLYLINE\n90\n2\n10\n0\n20\n0\n40\n0\n41\n0\n10\n10\n20\n0\n40\n0\n41\n0\n";
        let heavy = |header: &str, vertex: &str| {
            format!(
                "0\nPOLYLINE\n{header}0\nVERTEX\n10\n0\n20\n0\n{vertex}0\nVERTEX\n10\n10\n20\n0\n0\nSEQEND\n"
            )
        };
        let expected = import(&file(line, "4")).unwrap().drawing;
        assert_eq!(import(&file(light, "4")).unwrap().drawing, expected);
        assert_eq!(import(&file(&heavy("", "40\n0\n41\n0\n"), "4")).unwrap().drawing, expected);
        for header in
            ["30\n1\n", "39\n1\n", "230\n-1\n", "40\n1\n", "70\n2\n", "70\n4\n", "70\n8\n"]
        {
            assert!(
                matches!(import(&file(&heavy(header, ""), "4")), Err(Error::Entity { entity, .. }) if entity == "POLYLINE")
            );
        }
        for vertex in ["30\n1\n", "39\n1\n", "40\n1\n", "41\n1\n", "70\n1\n", "70\n8\n"] {
            assert!(import(&file(&heavy("", vertex), "4")).is_err(), "{vertex}");
        }
        assert!(import(&file(&light.replacen("40\n0", "40\n1", 1), "4")).is_err());
        assert!(import(&file(&light.replacen("20\n0", "20\n0\n20\n1", 1), "4")).is_err());
    }
}
