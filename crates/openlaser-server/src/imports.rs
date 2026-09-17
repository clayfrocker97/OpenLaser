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
        let mut warnings = imported.warnings;
        warnings.extend(
            imported
                .skipped
                .iter()
                .map(|item| format!("Skipped {} at line {}.", item.entity, item.line)),
        );
        Ok(Import { drawing: imported.drawing, warnings })
    }
}
