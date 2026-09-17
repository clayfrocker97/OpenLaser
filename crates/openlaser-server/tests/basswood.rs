// SPDX-License-Identifier: GPL-3.0-or-later

//! The shipped basswood recipe must compile with its saved graph style.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known recipe and loopback plant")]
mod common;

use common::{fixture, until};
use openlaser_controller::{Simulator, state::ProgramState};
use openlaser_core::LaserMode;
use openlaser_server::{Coordinator, connect, machine};

#[tokio::test]
async fn bundled_basswood_compiles_and_runs_with_its_original_curve() {
    let simulator = Simulator::start().await.unwrap();
    let mut config = common::config("basswood", simulator.config());
    config.mode = Some(LaserMode::Co2);
    let shared = Coordinator::start(config).unwrap();
    common::seed(&shared, &simulator).await;
    let original = {
        let mut c = shared.lock().await;
        c.seed_defaults();
        let recipe = c
            .document()
            .library
            .recipes
            .iter()
            .find(|r| r.name == "Basswood" && r.laser == LaserMode::Co2)
            .unwrap()
            .clone();
        assert_eq!(recipe.layer, 11);
        assert_eq!(recipe.attributes["PowerCurveSmoothType"], "2");
        assert_eq!(recipe.attributes["PowerAdjustWithSpeed"], "1");
        assert_eq!(recipe.attributes["PWMCurveNodes"], "0,75,27,89,57,97,100,100");
        let bytes = std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap();
        let part = c.import_part("basswood-line.dxf", &bytes).unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        recipe
    };
    machine::prepare(&shared).await.unwrap();
    shared.lock().await.set_origin([100., 100.]).unwrap();
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    machine::compile(&shared, false).await.unwrap();
    let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
    assert!(!compiled.dry_run && compiled.blocks > 0 && !compiled.plan.is_empty());
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
            && d.machine.operation.is_none()
    })
    .await;
    let plant = simulator.control().view();
    assert!(plant.last_error.is_none(), "{:?}", plant.last_error);
    assert!(plant.executed > 0 && plant.pwm.iter().all(|p| p[1] == 0));
    let c = shared.lock().await;
    assert_eq!(c.library.recipe(&original.id).unwrap().attributes, original.attributes);
    assert_eq!(c.draft.as_ref().unwrap().recipe.as_ref().unwrap().attributes, original.attributes);
    drop(c);
    openlaser_server::shutdown(&shared).await.unwrap();
}
