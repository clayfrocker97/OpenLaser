// SPDX-License-Identifier: GPL-3.0-or-later

//! One persistence boundary for supported drawing formats.

use crate::{Error, Result};
use openlaser_core::geometry::Drawing;

pub(crate) struct Import {
    pub drawing: Drawing,
    pub warnings: Vec<String>,
}

pub(crate) fn part(name: &str, bytes: &[u8], fonts: &openlaser_svg::Fonts) -> Result<Import> {
    if std::path::Path::new(name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("svg")) {
        let imported = openlaser_svg::import_with_fonts(bytes, fonts)
            .map_err(|e| Error::Request(e.to_string()))?;
        Ok(Import { drawing: imported.drawing, warnings: imported.warnings })
    } else {
        let imported = openlaser_dxf::import(bytes).map_err(|e| Error::Request(e.to_string()))?;
        let mut warnings = Vec::new();
        if imported.units == openlaser_dxf::Units::Unspecified {
            warnings.push(UNITLESS.to_owned());
        }
        warnings.extend(imported.warnings);
        warnings.extend(
            imported
                .skipped
                .iter()
                .map(|item| format!("Skipped {} at line {}.", item.entity, item.line)),
        );
        Ok(Import { drawing: imported.drawing, warnings })
    }
}

/// A DXF without `$INSUNITS` is read in millimetres; say so, because an
/// inch drawing read that way is 25.4 times too small.
const UNITLESS: &str = "The drawing does not declare its units, so it was read in millimetres. \
     If it was drawn in inches, scale it to 2540 %.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unitless_dxf_says_it_was_read_in_millimetres() {
        let line = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDSEC\n0\nEOF\n";
        let fonts = openlaser_svg::Fonts::default();
        let unitless = part("plate.dxf", line.as_bytes(), &fonts).map_err(|e| e.to_string());
        assert_eq!(unitless.map(|i| i.warnings), Ok(vec![UNITLESS.to_owned()]));
        let declared = format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n{line}");
        let declared = part("plate.dxf", declared.as_bytes(), &fonts).map_err(|e| e.to_string());
        assert_eq!(declared.map(|i| i.warnings), Ok(Vec::new()));
    }
}
