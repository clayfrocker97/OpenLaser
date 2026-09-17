// SPDX-License-Identifier: GPL-3.0-or-later

//! A vendor recipe file: one layer bank, `PLayerParam{n}` or
//! `PCO2LayerParam{n}`, as M-Laser exports a material and as its process
//! library ships them. Reading takes the file as is; writing produces the
//! same shape from an attribute map, so a bank can be handed back to the
//! binder as a layer file.

use crate::document::{Attributes, Document, Kind, escape};
use crate::{Error, Result};
use openlaser_core::LaserMode;

/// One layer bank read from a recipe file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerFile {
    /// The laser the bank is for.
    pub laser: LaserMode,
    /// The bank number, 1 to 11.
    pub bank: u8,
    /// The `GP` attributes.
    pub attributes: Attributes,
}

/// Reads a recipe file holding exactly one bank.
pub fn parse(bytes: &[u8]) -> Result<LayerFile> {
    let document = Document::parse(Kind::Layer, bytes)?;
    let mut banks = document.paths().into_iter().filter_map(bank_of);
    let (laser, bank, path) = banks.next().ok_or_else(|| Error::Missing("a layer bank".into()))?;
    if banks.next().is_some() {
        return Err(Error::Unsupported("a recipe file holds one layer bank".into()));
    }
    Ok(LayerFile { laser, bank, attributes: document.attributes(path)?.clone() })
}

/// The bank a `GP` path names.
fn bank_of(path: &str) -> Option<(LaserMode, u8, &str)> {
    let group = path.strip_prefix("/ParameterRoot/P")?.strip_suffix("/GP")?;
    let (laser, number) = match group.strip_prefix("CO2LayerParam") {
        Some(number) => (LaserMode::Co2, number),
        None => (LaserMode::Fiber, group.strip_prefix("LayerParam")?),
    };
    number.parse().ok().filter(|n| (1..=11).contains(n)).map(|n| (laser, n, path))
}

/// The bank's parameter group: `LayerParam3` or `CO2LayerParam3`.
#[must_use]
pub fn group(laser: LaserMode, bank: u8) -> String {
    format!("{}{bank}", if laser == LaserMode::Co2 { "CO2LayerParam" } else { "LayerParam" })
}

/// A recipe file for one bank, as the vendor writes it.
pub fn write(laser: LaserMode, bank: u8, attributes: &Attributes) -> Result<Document> {
    let group = group(laser, bank);
    let fields: Vec<String> =
        attributes.iter().map(|(key, value)| format!("{key}=\"{}\"", escape(value))).collect();
    let text = format!(
        "<ParameterRoot>\n<P{group}>\n<GP {}/>\n</P{group}>\n</ParameterRoot>\n",
        fields.join(" ")
    );
    Document::parse(Kind::Layer, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vendor recipe file reads as its bank, a note with raw line breaks
    /// survives, a file holding two banks or none is refused, and a bank
    /// written from the attributes reads back the same.
    #[test]
    fn recipe_files_read_and_write() {
        let bytes = b"<ParameterRoot>\n<PCO2LayerParam11>\n<GP CutSpeed=\"5\" Note=\"flat\nboards\" LayerFileName=\"\"/>\n</PCO2LayerParam11>\n</ParameterRoot>\n";
        let file = parse(bytes).unwrap();
        assert_eq!((file.laser, file.bank), (LaserMode::Co2, 11));
        assert_eq!(file.attributes["Note"], "flat\nboards");
        let written = write(file.laser, file.bank, &file.attributes).unwrap();
        assert_eq!(parse(written.original()).unwrap(), file);
        assert_eq!(
            written.attributes("/ParameterRoot/PCO2LayerParam11/GP").unwrap()["Note"],
            "flat\nboards"
        );
        assert!(parse(b"<ParameterRoot><PLayerParam1><GP a=\"1\"/></PLayerParam1><PLayerParam2><GP a=\"1\"/></PLayerParam2></ParameterRoot>").is_err());
        assert!(
            parse(b"<ParameterRoot><PManuParam><MC a=\"1\"/></PManuParam></ParameterRoot>")
                .is_err()
        );
        assert_eq!(group(LaserMode::Fiber, 3), "LayerParam3");
    }
}
