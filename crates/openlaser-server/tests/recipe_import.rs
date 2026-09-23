// SPDX-License-Identifier: GPL-3.0-or-later

//! Reviewed recipe import: a preview saves nothing, the operator's material,
//! thickness and head setup are kept, and a duplicate is replaced, kept
//! beside the original, or answered with the recipe already there.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "isolated material library fixtures")]
mod common;

use openlaser_server::Coordinator;
use openlaser_server::coordinator::RecipeImport;
use openlaser_server::recipes::setup::NozzleKind;

const FILE: &[u8] = b"<ParameterRoot><PLayerParam3><GP CutSpeed=\"60\" CutPower=\"80\" CutPeakCurrent=\"90\" CutGasType=\"5\" ManuType=\"3\" Note=\"NOZZLE---DOUBLE-1.5   FOCAL LENGTH---(+1)\" LayerFileName=\"SS 2mm\"/></PLayerParam3></ParameterRoot>";
const NAME: &str = "SS2.0mm 2.0S F-3 F150 N2.xml";

#[tokio::test]
async fn previews_then_imports_as_reviewed() {
    let shared = Coordinator::start(common::config(
        "recipe-import",
        openlaser_controller::Config::machine(),
    ))
    .unwrap();
    let mut c = shared.lock().await;

    let preview = c.preview_recipe(NAME, FILE).unwrap();
    assert!(c.document().library.recipes.is_empty(), "a preview saves nothing");
    assert_eq!(
        (preview.name.as_str(), preview.thickness_mm, preview.gas.as_str()),
        ("SS", 2., "N2")
    );
    assert_eq!(preview.setup.nozzle_diameter_mm.as_deref(), Some("1.5"), "the note wins");
    assert_eq!(preview.setup.nozzle, Some(NozzleKind::Double));
    assert_eq!(preview.setup.focus_mm.as_deref(), Some("1"));
    assert_eq!(preview.setup.lens_mm.as_deref(), Some("150"), "the name fills the rest");
    assert_eq!(preview.summary.duty.as_deref(), Some("80"));
    assert_eq!(preview.summary.peak.as_deref(), Some("90"));
    assert_eq!(preview.summary.pierce_stages, 2);
    assert!(preview.existing.is_none());

    let reviewed = RecipeImport {
        name: NAME.into(),
        material: Some("Stainless steel".into()),
        setup: true,
        nozzle_diameter_mm: Some("2.0".into()),
        nozzle: Some(NozzleKind::Single),
        focus_mm: Some("-3".into()),
        ..RecipeImport::default()
    };
    let (first, existing) = c.import_recipe_as(&reviewed, FILE).unwrap();
    assert!(!existing);
    assert_eq!(first.name, "Stainless steel");
    assert_eq!(first.attributes["OpenLaserNozzleDiameter"], "2.0");
    assert_eq!(first.attributes["OpenLaserNozzleType"], "single");
    assert_eq!(first.attributes["OpenLaserManualFocus"], "-3");
    assert!(!first.attributes.contains_key("OpenLaserLens"), "a cleared value stays cleared");
    assert_eq!(first.attributes["CutSpeed"], "60");

    // The same file again under the same identity answers with the first.
    assert_eq!(c.preview_recipe(NAME, FILE).unwrap().existing, Some(first.id.clone()));
    let (again, existing) = c.import_recipe_as(&reviewed, FILE).unwrap();
    assert!(existing);
    assert_eq!(again.id, first.id);

    // Keep both adds a second recipe.
    let (second, _) =
        c.import_recipe_as(&RecipeImport { keep_both: true, ..reviewed.clone() }, FILE).unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(c.document().library.recipes.len(), 2);

    // Replace keeps the id and star, and takes the new values.
    c.update_recipe(
        &first.id,
        openlaser_server::coordinator::RecipeChange { favourite: Some(true), ..Default::default() },
    )
    .unwrap();
    let other =
        String::from_utf8(FILE.to_vec()).unwrap().replace("CutSpeed=\"60\"", "CutSpeed=\"75\"");
    let (replaced, _) = c
        .import_recipe_as(
            &RecipeImport { replace: Some(first.id.clone()), ..reviewed.clone() },
            other.as_bytes(),
        )
        .unwrap();
    assert_eq!(replaced.id, first.id);
    assert!(replaced.favourite);
    assert_eq!(replaced.attributes["CutSpeed"], "75");
    assert_eq!(c.document().library.recipes.len(), 2);

    // A setup value out of range is refused before anything is saved.
    let bad = RecipeImport { lens_mm: Some("abc".into()), ..reviewed };
    assert!(c.import_recipe_as(&bad, FILE).is_err());
    assert_eq!(c.document().library.recipes.len(), 2);
}
