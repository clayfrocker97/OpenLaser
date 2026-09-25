// SPDX-License-Identifier: GPL-3.0-or-later
//! Layers with their own recipes: cuts on the job's material, the recipe a
//! mark among other layers asks for, shapes moved and layers renamed, a
//! marked layer prepared bare under its own recipe, the order dragged, and
//! all of it saved.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use common::start;
use openlaser_core::LaserMode;
use openlaser_core::features::{Features, Kerf, LayerMode, Side};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use openlaser_core::units::Millimeters;
use openlaser_server::coordinator::{NewRecipe, RecipeChange, Values};
use openlaser_server::layers::LayerChange;
use openlaser_server::machine;

fn rect(layer: &str, corner: [f64; 2], size: [f64; 2]) -> Contour {
    let [x, y] = corner;
    let [width, height] = size;
    let points = [[x, y], [x + width, y], [x + width, y + height], [x, y + height]];
    Contour {
        layer: layer.into(),
        curves: (0..4)
            .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
            .collect(),
    }
}

#[tokio::test]
async fn cuts_run_on_the_material_and_a_mark_runs_bare_under_its_own_recipe_in_order() {
    let (_sim, shared) = start("layers").await;
    let bend = Contour {
        layer: "Bend".into(),
        curves: vec![Curve::Line { start: Point::new(10., 30.), end: Point::new(90., 30.) }],
    };
    let drawing = Drawing {
        contours: vec![
            rect("Cut", [0., 0.], [100., 60.]),
            bend,
            rect("Cut", [40., 40.], [10., 10.]),
        ],
    };
    let (etch, job_steel) = {
        let mut c = shared.lock().await;
        let part = c.library.add_part("bracket.dxf", b"layered fixture", drawing).unwrap();
        let recipe = |name: &str, values| NewRecipe {
            name: name.into(),
            laser: LaserMode::Fiber,
            thickness_mm: 2.,
            values,
            gas: None,
        };
        let steel = c.add_recipe(&recipe("Steel", Values::Bank(1))).unwrap();
        // An engraving recipe: the cutting one with the beam turned down.
        let etch = c.add_recipe(&recipe("Etch", Values::Recipe(steel.id.clone()))).unwrap();
        let lower = RecipeChange {
            attributes: [("CutPeakCurrent".to_owned(), "9".to_owned())].into(),
            ..RecipeChange::default()
        };
        c.update_recipe(&etch.id, lower).unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&steel.id).unwrap();
        let kerf = Kerf { width: Millimeters(0.4), side: Side::Auto };
        c.set_features(Features { kerf: Some(kerf), ..Features::default() }).unwrap();
        (etch.id, steel)
    };
    // Two cuts run on the job's material without asking.
    machine::prepare(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    // A mark would be cut through on the material's recipe, so it asks.
    shared
        .lock()
        .await
        .change_layers(LayerChange::Mode { layer: "Bend".into(), mode: LayerMode::Mark })
        .unwrap();
    machine::prepare(&shared).await.unwrap();
    let refused = machine::compile(&shared, false).await.unwrap_err().to_string();
    assert!(refused.contains("Bend") && !refused.contains("Cut"), "{refused}");

    {
        let mut c = shared.lock().await;
        // The small square goes to its own layer, named twice over.
        c.change_layers(LayerChange::Assign { contours: vec![2], layer: "Holes".into() }).unwrap();
        c.change_layers(LayerChange::Rename { from: "Holes".into(), to: "Slots".into() }).unwrap();
        let names: Vec<_> =
            c.document().draft.unwrap().layers.iter().map(|l| l.name.clone()).collect();
        assert_eq!(names, ["Bend", "Cut", "Slots"], "a new layer after those already listed");
        c.change_layers(LayerChange::Output { layer: "Slots".into(), on: false }).unwrap();
        c.change_layers(LayerChange::Recipe { layer: "Bend".into(), recipe: Some(etch) }).unwrap();
        let layers = c.document().draft.unwrap().layers.clone();
        assert!(layers.iter().all(|l| l.chosen));
        let bend = layers.iter().find(|l| l.name == "Bend").unwrap();
        assert_eq!(bend.recipe.as_ref().unwrap().name, "Etch");
        assert_eq!(bend.mode, LayerMode::Mark);
        assert!(!layers.iter().find(|l| l.name == "Slots").unwrap().output);
        // Dragged: the cut first, the mark after it.
        let order = vec!["Cut".to_owned(), "Slots".to_owned(), "Bend".to_owned()];
        c.change_layers(LayerChange::Order { layers: order }).unwrap();
        let names: Vec<_> =
            c.document().draft.unwrap().layers.iter().map(|l| l.name.clone()).collect();
        assert_eq!(names, ["Cut", "Slots", "Bend"]);
        let order = vec!["Bend".to_owned(), "Cut".to_owned(), "Slots".to_owned()];
        c.change_layers(LayerChange::Order { layers: order }).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    let c = shared.lock().await;
    let draft = c.draft.as_ref().unwrap();
    let preview = draft.preview.as_ref().unwrap();
    let order: Vec<_> = preview.contours.iter().map(|c| c.layer.as_str()).collect();
    assert_eq!(order, ["Bend", "Cut"], "in the layers' order, the slot not at all");
    let compiled = draft.compiled.as_ref().unwrap();
    let passes = &compiled.job.passes;
    assert_ne!(passes[0].settings, compiled.job.settings, "the bend runs under Etch");
    assert_eq!(passes.last().unwrap().settings, compiled.job.settings);
    // Bare: the bend line keeps its drawn ends, where kerf would move them.
    let bend = &preview.contours[0];
    let ends = bend
        .paths
        .iter()
        .flat_map(|p| p.points.iter())
        .fold((f64::MAX, f64::MIN), |(lo, hi), point| (lo.min(point[0]), hi.max(point[0])));
    assert!((ends.0 - 10.).abs() < 1e-6 && (ends.1 - 90.).abs() < 1e-6, "{ends:?}");
    drop(c);

    // A saved job keeps the layers, and opens with them again.
    let mut c = shared.lock().await;
    let saved = c.save_job("Layered bracket").unwrap();
    let job = c.library.job(&saved.id).unwrap().clone();
    assert_eq!(job.layers.len(), 1, "only the mark's recipe is a choice");
    assert_eq!(job.features.layers.len(), 3);
    assert_eq!(job.recipe.name, job_steel.name);
    assert_eq!(job.features.layer_edits.len(), 1);
    c.open_job(&saved.id).unwrap();
    assert!(c.document().draft.unwrap().layers.iter().all(|l| l.chosen));
}
