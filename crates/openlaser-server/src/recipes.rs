// SPDX-License-Identifier: GPL-3.0-or-later

//! Recipes from the vendor's files: a material library recipe file, or a
//! layer bank of the machine's own backup.
//!
//! The vendor names library files `<material>-<thickness>MM_<process>`,
//! with spaces or a lower-case `mm` here and there, and writes the
//! operator's note into the bank. The prototype kept the file stem as the
//! material and left the thickness unset; this reads the vendor's pattern
//! where it is followed and leaves the rest for the operator to edit.

use openlaser_core::LaserMode;
use openlaser_library::{Id, Recipe};
use openlaser_xml::document::Attributes;
use openlaser_xml::{Bundle, layer_file};

use crate::{Error, Result};

pub mod setup;

use setup::HeadSetup;

/// A recipe from a vendor recipe file.
pub fn from_file(file_name: &str, bytes: &[u8]) -> Result<Recipe> {
    let file = layer_file::parse(bytes).map_err(|e| Error::Request(format!("{file_name}: {e}")))?;
    let stem = file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem);
    let (short, process) =
        stem.split_once('_').map_or((stem, ""), |(short, process)| (short, process));
    let (name, thickness_mm, rest) = material(short);
    let tags = [process, rest].into_iter().map(clean).filter(|s| !s.is_empty()).collect();
    let gas = gas(stem, &file.attributes);
    let mut recipe = recipe(
        name,
        thickness_mm,
        gas,
        file.laser,
        file.bank,
        file.attributes,
        Some(file_name),
        tags,
    );
    read_setup(&mut recipe.attributes, stem);
    Ok(recipe)
}

/// A recipe from a layer bank of the machine files, named by the operator.
pub fn from_bank(
    bundle: &Bundle,
    laser: LaserMode,
    bank: u8,
    name: &str,
    thickness_mm: f64,
) -> Result<Recipe> {
    let attributes = bundle.group(&layer_file::group(laser, bank), "GP")?.clone();
    let words = attributes.get("LayerFileName").cloned().unwrap_or_default();
    let gas = gas(&words, &attributes);
    let mut recipe =
        recipe(name.to_owned(), thickness_mm, gas, laser, bank, attributes, None, Vec::new());
    read_setup(&mut recipe.attributes, "");
    Ok(recipe)
}

#[allow(clippy::too_many_arguments, reason = "the two sources fill the same record")]
fn recipe(
    name: String,
    thickness_mm: f64,
    gas: String,
    laser: LaserMode,
    bank: u8,
    attributes: Attributes,
    file_name: Option<&str>,
    tags: Vec<String>,
) -> Recipe {
    Recipe {
        id: Id::from(""),
        name,
        laser,
        thickness_mm,
        gas: if laser == LaserMode::Co2 { GAS[3].into() } else { gas },
        layer: bank,
        note: attributes.get("Note").map(|note| note.trim().to_owned()).unwrap_or_default(),
        attributes,
        film: None,
        source_sha256: None,
        tags,
        file_name: file_name.map(str::to_owned),
        photo: None,
        favourite: false,
        created: 0,
        updated: 0,
    }
}

/// The head setup the note, the layer name and the file name state, added
/// where the attributes do not already hold it. Every controller field,
/// `CutFocusPos` included, stays as the file has it.
fn read_setup(attributes: &mut Attributes, stem: &str) {
    let note = attributes.get("Note").cloned().unwrap_or_default();
    let layer = attributes.get("LayerFileName").cloned().unwrap_or_default();
    HeadSetup::read(&[&note, &layer, stem]).apply(attributes, false);
}

/// The material name, the thickness and whatever follows, from the
/// vendor's `<material>-<thickness>MM` pattern; the whole text as the name
/// and no thickness when the pattern is not there.
fn material(short: &str) -> (String, f64, &str) {
    let lower = short.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("mm") {
        let end = from + at;
        let start = lower[..end].trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').len();
        if let Ok(value) = lower[start..end].parse::<f64>()
            && value > 0.
        {
            let name = clean(&short[..start]);
            return (if name.is_empty() { clean(short) } else { name }, value, &short[end + 2..]);
        }
        from = end + 2;
    }
    (clean(short), 0., "")
}

/// Text without the separators the vendor pads names with.
fn clean(text: &str) -> String {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '_')).to_owned()
}

/// The controller's six gas selections, low pressure then high, as the
/// vendor orders them.
pub const GAS: [&str; 6] = ["Low Air", "Low O₂", "Low N₂", "High Air", "High O₂", "High N₂"];

/// The gas selection's name: `selector` 0 to 5.
#[must_use]
pub fn gas_name(selector: &str) -> String {
    selector
        .parse::<usize>()
        .ok()
        .and_then(|i| GAS.get(i))
        .map_or_else(|| format!("Controller gas {selector}"), |name| (*name).to_owned())
}

/// The assist gas by name. The vendor's file names say AIR, N2 or O2;
/// otherwise the bank's gas selector names it.
fn gas(words: &str, attributes: &Attributes) -> String {
    let upper = words.to_ascii_uppercase();
    let word = |w: &str| upper.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == w);
    if word("AIR") {
        "Air".into()
    } else if word("N2") || word("NITROGEN") {
        "N2".into()
    } else if word("O2") || word("OXYGEN") {
        "O2".into()
    } else {
        attributes.get("CutGasType").map_or_else(|| "Unassigned".into(), |s| gas_name(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn co2_imports_use_high_air_and_preserve_original_attributes() {
        let source = br#"<ParameterRoot><PCO2LayerParam1><GP CutSpeed="50" CutGasType="5" /></PCO2LayerParam1></ParameterRoot>"#;
        let imported = from_file("Basswood-3MM_CO2 CUTTING.xml", source).unwrap();
        assert_eq!(imported.gas, "High Air");
        assert_eq!(imported.attributes["CutGasType"], "5");
    }

    #[test]
    fn original_mlaser_xml_imports_without_optional_manual_setup() {
        let source = br#"<ParameterRoot><PLayerParam1><GP CutSpeed="50" CutFocusPos="0" CutGasType="5" Note="Original vendor note" /></PLayerParam1></ParameterRoot>"#;
        let imported = from_file("SS-2MM_N2 CUTTING.xml", source).unwrap();
        assert!((imported.thickness_mm - 2.).abs() < f64::EPSILON);
        assert_eq!(imported.gas, "N2");
        assert_eq!(imported.attributes["CutSpeed"], "50");
        assert_eq!(imported.attributes["CutFocusPos"], "0");
        assert!(!imported.attributes.contains_key("OpenLaserNozzleDiameter"));
        assert!(!imported.attributes.contains_key("OpenLaserManualFocus"));
    }

    /// The vendor's naming patterns: material and thickness before `MM`,
    /// the process after the underscore, the gas from the words, and a
    /// name without the pattern kept whole.
    #[test]
    fn names_follow_the_vendors_pattern() {
        assert_eq!(material("SS-2MM"), ("SS".into(), 2., ""));
        assert_eq!(material("AL 1MM"), ("AL".into(), 1., ""));
        assert_eq!(material("SS1.0mm 1.5S F-3 N2"), ("SS".into(), 1., " 1.5S F-3 N2"));
        assert_eq!(material("AL-3MM-MARKING"), ("AL".into(), 3., "-MARKING"));
        assert_eq!(material("1mm"), ("1mm".into(), 1., ""));
        assert_eq!(material("SIX-IN-ONE"), ("SIX-IN-ONE".into(), 0., ""));
        let selector = Attributes::from([("CutGasType".to_owned(), "5".to_owned())]);
        assert_eq!(gas("SS-2MM_AIR CUTTING", &selector), "Air");
        assert_eq!(gas("SS-2MM_N2 CUTTING", &selector), "N2");
        assert_eq!(gas("CS-3MM_O2 CUTTING", &selector), "O2");
        assert_eq!(gas("ACRYLIC-5MM_CO2-CUTTING", &selector), "High N₂");
        assert_eq!(gas("SIX-IN-ONE", &Attributes::new()), "Unassigned");
        assert_eq!(gas_name("1"), "Low O₂");
        assert_eq!(gas_name("7"), "Controller gas 7");
    }

    /// A vendor recipe file becomes a recipe with its bank, note and tags.
    #[test]
    fn a_recipe_file_reads_as_a_recipe() {
        let bytes = b"<ParameterRoot>\n<PLayerParam11>\n<GP CutSpeed=\"100\" CutGasType=\"5\" Note=\"NOZZLE---SINGLE-2.0\n\" LayerFileName=\"x\"/>\n</PLayerParam11>\n</ParameterRoot>\n";
        let recipe = from_file("SS-2MM_AIR CUTTING.xml", bytes).unwrap();
        assert_eq!(
            (recipe.name.as_str(), recipe.thickness_mm, recipe.gas.as_str()),
            ("SS", 2., "Air")
        );
        assert_eq!((recipe.laser, recipe.layer), (LaserMode::Fiber, 11));
        assert_eq!(recipe.tags, vec!["AIR CUTTING".to_owned()]);
        assert_eq!(recipe.note, "NOZZLE---SINGLE-2.0");
        assert_eq!(recipe.file_name.as_deref(), Some("SS-2MM_AIR CUTTING.xml"));
        assert!(from_file("plain.xml", b"<ParameterRoot/>").is_err());
    }
}
