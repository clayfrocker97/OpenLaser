// SPDX-License-Identifier: GPL-3.0-or-later

//! Bundled source preservation, distinct nozzle variants and editable setup.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "isolated material library fixtures")]
mod common;

use openlaser_core::LaserMode;
use openlaser_server::{Coordinator, coordinator::RecipeChange};

#[tokio::test]
async fn bundled_metals_preserve_source_values_and_setup_edits_survive_restart() {
    let shared = Coordinator::start(common::config(
        "bundled-metals",
        openlaser_controller::Config::machine(),
    ))
    .unwrap();
    let (config, edited_id, removed_id) = {
        let mut c = shared.lock().await;
        c.seed_defaults();
        let doc = c.document();
        let metals: Vec<_> =
            doc.library.recipes.iter().filter(|r| r.laser == LaserMode::Fiber).collect();
        assert_eq!(metals.len(), 50);
        assert_eq!(doc.library.recipes.len(), 51);
        for recipe in &metals {
            assert!(recipe.photo.is_none(), "bundled recipes use the default material artwork");
            assert!(recipe.attributes.contains_key("OpenLaserNozzleDiameter"));
            assert!(recipe.attributes.contains_key("OpenLaserNozzleType"));
            assert!(recipe.attributes.contains_key("OpenLaserManualFocus"));
            let saved = c.library.recipe(&recipe.id).unwrap();
            let original = std::fs::read(
                c.config.data_dir.join("originals").join(saved.source_sha256.as_ref().unwrap()),
            )
            .unwrap();
            let source = openlaser_xml::layer_file::parse(&original).unwrap();
            for (key, value) in &source.attributes {
                assert_eq!(recipe.attributes.get(key), Some(value), "{}: {key}", recipe.name);
            }
        }
        let variants: Vec<_> = metals
            .iter()
            .filter(|r| {
                r.name == "Stainless Steel"
                    && (r.thickness_mm - 3.).abs() < f64::EPSILON
                    && r.gas == "N2"
            })
            .collect();
        assert_eq!(variants.len(), 2, "both nozzle variants remain selectable");
        let chosen =
            variants.iter().find(|r| r.attributes["OpenLaserNozzleDiameter"] == "2.0").unwrap();
        assert_eq!(chosen.attributes["OpenLaserManualFocus"], "-5");
        assert_eq!(chosen.attributes["CutFocusPos"], "0");
        let id = chosen.id.clone();
        let removed = metals.iter().find(|r| r.id != id).unwrap().id.clone();
        c.update_recipe(
            &id,
            RecipeChange {
                attributes: [
                    ("OpenLaserNozzleDiameter".into(), "1.5".into()),
                    ("OpenLaserNozzleType".into(), "double".into()),
                    ("OpenLaserManualFocus".into(), "-4".into()),
                ]
                .into(),
                ..RecipeChange::default()
            },
        )
        .unwrap();
        c.remove_recipe(&removed).unwrap();
        c.seed_defaults();
        assert_eq!(c.document().library.recipes.len(), 50);
        (c.config.clone(), id, removed)
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    {
        let mut c = reopened.lock().await;
        c.seed_defaults();
        let doc = c.document();
        let edited = doc.library.recipes.iter().find(|r| r.id == edited_id).unwrap();
        assert_eq!(edited.attributes["OpenLaserNozzleDiameter"], "1.5");
        assert_eq!(edited.attributes["OpenLaserNozzleType"], "double");
        assert_eq!(edited.attributes["OpenLaserManualFocus"], "-4");
        assert_eq!(edited.attributes["CutFocusPos"], "0");
        assert!(!doc.library.recipes.iter().any(|r| r.id == removed_id));
    }
    openlaser_server::shutdown(&reopened).await.unwrap();
}
