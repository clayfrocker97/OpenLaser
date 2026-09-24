// SPDX-License-Identifier: GPL-3.0-or-later

//! Compile and execute varied sheets over the real loopback transport.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture acceptance tests")]

mod common;

use common::{fixture, start, until};
use openlaser_controller::state::ProgramState;
use openlaser_core::LaserMode;
use openlaser_core::features::{Cooling, CoolingPlacement, Features, Lead, LeadShape, Leads, Side};
use openlaser_core::geometry::Transform;
use openlaser_core::units::{Degrees, Millimeters, Milliseconds};
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::{connect, machine};
use serde_json::json;

async fn sheet(shared: &Shared, name: &str, variant: u32, mode: LaserMode) -> usize {
    let bytes = std::fs::read(fixture(&format!("dxf/{name}.dxf"))).unwrap();
    let mut c = shared.lock().await;
    let original = c.import_part(&format!("{name}.dxf"), &bytes).unwrap();
    let part = c.duplicate_part(&original.id).unwrap();
    let recipe = c
        .add_recipe(&NewRecipe {
            name: format!("{name}-{variant}"),
            laser: mode,
            thickness_mm: 1.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    c.open_part(&part.id).unwrap();
    c.set_recipe(&recipe.id).unwrap();
    c.set_features(if variant == 1 {
        Features {
            leads: Some(Leads {
                entry: Some(Lead {
                    shape: LeadShape::Line,
                    length: Millimeters(1.),
                    radius: Millimeters(1.),
                    angle: Degrees(45.),
                }),
                exit: None,
                side: Side::Auto,
                closed_only: true,
                overrides: Vec::new(),
            }),
            cooling: Some(Cooling {
                dwell: Milliseconds(20),
                placement: CoolingPlacement::Automatic { at_start: true, corners_below: None },
            }),
            ..Features::default()
        }
    } else {
        Features::default()
    })
    .unwrap();
    let count = c.library.part(&part.id).unwrap().drawing.contours.len();
    if variant == 1 {
        let bounds = c.library.part(&part.id).unwrap().drawing.bounds().unwrap();
        for (x, y) in [(1., 0.), (0., 1.), (1., 1.)] {
            let transform = Transform([
                1.,
                0.,
                0.,
                1.,
                x * (bounds.max.x - bounds.min.x + 10.),
                y * (bounds.max.y - bounds.min.y + 10.),
            ]);
            c.add(&(0..count).map(|source| (source, transform)).collect::<Vec<_>>()).unwrap();
        }
    } else if variant >= 2 {
        let transform = if variant == 2 {
            Transform([-1.25, 0., 0., 1.25, 0., 0.])
        } else {
            Transform([0., 1., -1., 0., 0., 0.])
        };
        c.transform(&(0..count).collect::<Vec<_>>(), Some(transform)).unwrap();
    }
    // A drawing of several layers cuts each with the job's recipe.
    let layers = c.document().draft.unwrap().layers.clone();
    if layers.len() > 1 {
        for layer in layers {
            let change = openlaser_server::layers::LayerChange::Cut {
                layer: layer.name,
                recipe: None,
                engrave: false,
            };
            c.change_layers(change).unwrap();
        }
    }
    drop(c);
    machine::prepare(shared).await.unwrap();
    let mut c = shared.lock().await;
    c.set_origin([100., 100.]).unwrap();
    let draft = c.document().draft.unwrap();
    assert!(draft.error.is_none(), "{name}/{variant}: {:?}", draft.error);
    draft.placed.len()
}

#[tokio::test]
async fn varied_parts_compile_and_execute_to_completion() {
    let (simulator, shared) = start("parts-batch").await;
    let control = simulator.control();
    connect::connect(&shared).await.unwrap();
    let mut results = Vec::new();
    // The first two are the parts loaded in the desktop preview. The rest
    // exercise open paths, separate layers, inch conversion and bulge arcs.
    for variant in 0..4 {
        let mode = if variant == 3 { LaserMode::Co2 } else { LaserMode::Fiber };
        for name in [
            "plate-with-holes",
            "minimal-closed-polyline",
            "minimal-line",
            "layers",
            "inches",
            "polylines",
        ] {
            let placed = sheet(&shared, name, variant, mode).await;
            machine::compile(&shared, false).await.unwrap();
            // Select the job's source before establishing its references.
            // Further sheets in the same source retain that setup.
            if !shared.lock().await.document().machine.session.homed {
                machine::home(&shared).await.unwrap();
                until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none())
                    .await;
            }
            let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
            assert!(!compiled.plan.is_empty(), "{name}/{variant}");
            common::run(&shared).await.unwrap();
            until(&shared, 45, |d| {
                d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
                    && d.machine.operation.is_none()
                    && d.readiness.home.ok
            })
            .await;
            let plant = control.view();
            assert!(
                !plant.running && plant.queued_bytes == 0,
                "{name}/{variant}: FIFO did not drain"
            );
            assert_eq!(
                (plant.outputs, plant.extended_outputs, plant.analog),
                (0, 0, [0, 0]),
                "{name}/{variant}: outputs"
            );
            assert!(plant.pwm.iter().all(|p| p[1] == 0), "{name}/{variant}: laser remained on");
            assert!(plant.last_error.is_none(), "{name}/{variant}: {:?}", plant.last_error);
            let row = json!({"part":name,"variant":variant,"mode":mode,"placed_contours":placed,"passes":compiled.plan.len(),"blocks":compiled.blocks,"records_executed":plant.executed,"result":"completed","outputs_off":true});
            println!("SIMULATION_BATCH {row}");
            results.push(row);
        }
    }
    assert_eq!(results.len(), 24);
    if let Ok(path) = std::env::var("OPENLASER_SIMULATION_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}
