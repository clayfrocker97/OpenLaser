// SPDX-License-Identifier: GPL-3.0-or-later
//! Startup source selection and setup lifetimes over the loopback controller.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known simulator fixtures")]
mod common;

use common::{config, fixture, seed, until};
use openlaser_controller::Simulator;
use openlaser_core::LaserMode;
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::placement::{self, PlacementChange};
use openlaser_server::{Coordinator, connect, machine};

async fn check_manual_outputs(
    shared: &openlaser_server::coordinator::Shared,
    simulator: &Simulator,
) {
    let control = simulator.control();
    for (request, port, sequence) in
        [(machine::OutputRequest::Pointer, 6, 1), (machine::OutputRequest::Shutter, 5, 2)]
    {
        let lease = openlaser_controller::lease::Lease { client: "fiber-outputs".into(), sequence };
        control.clear_writes();
        machine::outputs(shared, request, lease.clone()).await.unwrap();
        until(shared, 5, |_| control.view().outputs != 0).await;
        assert!(
            control
                .writes()
                .contains(&openlaser_protocol::requests::digital_output(port, true).unwrap())
        );
        machine::release(shared, lease).await.unwrap();
        until(shared, 5, |d| d.machine.operation.is_none() && control.view().outputs == 0).await;
        assert!(
            control
                .writes()
                .contains(&openlaser_protocol::requests::digital_output(port, false).unwrap())
        );
    }
}

#[tokio::test]
async fn home_uses_the_job_source_and_compile_keeps_machine_setup() {
    let simulator = Simulator::start().await.unwrap();
    let mut config = config("first-fiber-job", simulator.config());
    config.mode = Some(LaserMode::Co2);
    let shared = Coordinator::start(config).unwrap();
    seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    let recipe_id = {
        let mut c = shared.lock().await;
        let part = c
            .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
            .unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Fiber steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        recipe.id
    };
    machine::prepare(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    assert_eq!(
        shared.lock().await.document().machine.session.mode,
        Some(LaserMode::Fiber),
        "Home must select the job source before establishing references"
    );
    machine::calibrate(&shared).await.unwrap();
    until(&shared, 10, |d| d.calibration.current && d.machine.operation.is_none()).await;
    simulator.control().clear_writes();
    machine::compile(&shared, false).await.unwrap();
    let doc = shared.lock().await.document();
    assert!(doc.machine.session.homed);
    assert!(doc.machine.session.calibration.is_some());
    assert!(doc.calibration.current);
    assert_eq!(doc.bindings.as_ref().unwrap().outputs.pointer_port, 6);
    assert_eq!(doc.bindings.as_ref().unwrap().outputs.shutter_port, 5);
    assert!(doc.draft.as_ref().unwrap().compiled.is_some());
    assert!(
        simulator.control().writes().is_empty(),
        "recompiling the same source must not reinitialize the machine"
    );
    check_manual_outputs(&shared, &simulator).await;
    placement::change(&shared, PlacementChange::SetOrigin {}, doc.draft_revision).await.unwrap();
    let doc = shared.lock().await.document();
    assert!(doc.calibration.current && doc.machine.session.homed);
    assert!(doc.draft.as_ref().unwrap().placement.captured);
    assert!(doc.draft.as_ref().unwrap().compiled.is_some());
    placement::change(&shared, PlacementChange::NewRun {}, doc.draft_revision).await.unwrap();
    let doc = shared.lock().await.document();
    assert!(doc.calibration.current && doc.machine.session.homed);
    assert!(!doc.draft.as_ref().unwrap().placement.captured);
    assert!(doc.draft.as_ref().unwrap().compiled.is_some());
    {
        let mut c = shared.lock().await;
        let alternative = c
            .add_recipe(&NewRecipe {
                name: "Fiber steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: Some(2),
            })
            .unwrap();
        c.set_recipe(&alternative.id).unwrap();
        assert!(
            c.document().calibration.current,
            "another recipe for the same material and thickness retains calibration"
        );
        let other = c
            .add_recipe(&NewRecipe {
                name: "Other material".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 2.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.set_recipe(&other.id).unwrap();
        assert!(!c.document().calibration.current);
        assert!(c.document().machine.session.homed, "a material change does not lose XY home");
        c.set_recipe(&recipe_id).unwrap();
        assert!(
            !c.document().calibration.current,
            "switching back must not resurrect a superseded calibration"
        );
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}
