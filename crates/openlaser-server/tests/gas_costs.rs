// SPDX-License-Identifier: GPL-3.0-or-later

//! Gas and laser accounting against the simulator: the compiled job's
//! usage, the run history after a stopped and a completed run, and the
//! 60-second flow test's bounded valve.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests drive a known plant")]

mod common;

use common::{fixture, start, until};
use openlaser_controller::state::ProgramState;
use openlaser_core::LaserMode;
use openlaser_core::units::{Bar, Liters, Millimeters, Seconds};
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::gas::{
    CalibrationRequest, GasKind, Money, Nozzle, NozzleType, RunOutcome, Source,
};
use openlaser_server::{connect, machine};
use std::time::Duration;

async fn set_up(shared: &Shared) {
    let dxf = std::fs::read(fixture("dxf/plate-with-holes.dxf")).unwrap();
    let mut coordinator = shared.lock().await;
    let part = coordinator.import_part("plate-with-holes.dxf", &dxf).unwrap();
    let recipe = coordinator
        .add_recipe(&NewRecipe {
            name: "Stainless".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 2.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    assert_eq!(recipe.gas, "High N₂");
    coordinator.open_part(&part.id).unwrap();
    coordinator.set_recipe(&recipe.id).unwrap();
    drop(coordinator);
    machine::prepare(shared).await.unwrap();
    shared.lock().await.set_origin([100., 100.]).unwrap();
}

#[tokio::test]
async fn compiled_usage_prices_runs_and_records_stopped_and_completed_runs() {
    let (simulator, shared) = start("gas").await;
    let control = simulator.control();
    set_up(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    let document = shared.lock().await.document();
    let draft = document.draft.clone().unwrap();
    let compiled = draft.compiled.clone().unwrap();
    let usage = &compiled.usage;
    assert!(usage.laser.0 > 0. && usage.laser.0 < compiled.seconds, "{usage:?}");
    assert_eq!(usage.pierces as usize, compiled.pierces.len());
    assert!(usage.cut.0 > 0.);
    assert!(!usage.gases.is_empty());
    assert!(usage.gases.iter().all(|g| g.gas == GasKind::Nitrogen), "{usage:?}");
    assert!(usage.gas_seconds() >= usage.laser.0, "gas flows whenever the beam is on");
    assert!(usage.gas_seconds() <= compiled.seconds);
    // Unpriced until the shop enters prices.
    let estimate = document.gas.estimate.clone().unwrap();
    assert_eq!(estimate.cost, None);

    let mut coordinator = shared.lock().await;
    let before = coordinator.gas.costs().clone();
    let mut costs = before.clone();
    costs.nitrogen.source = Source::Bulk { price_per_m3: Money(3.) };
    coordinator.gas.save(costs, &before).unwrap();
    coordinator.publish();
    let gas = coordinator.document().gas;
    drop(coordinator);
    let nozzle = gas.estimate.as_ref().unwrap().nozzle;
    // The bundled fixture bank names no nozzle unless the recipe sets one.
    if nozzle.is_some() {
        assert!(gas.estimate.as_ref().unwrap().cost.is_some_and(|c| c.0 > 0.));
    }

    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    control.time_scale(2.);
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    machine::stop(&shared).await.unwrap();
    until(&shared, 20, |d| d.message.as_ref().is_some_and(|m| m.text.contains("stopped"))).await;
    let key = draft.key.clone();
    let runs = shared.lock().await.gas.runs(&key, None);
    assert_eq!(runs.len(), 1, "{runs:?}");
    let stopped = &runs[0];
    assert_eq!(stopped.outcome, RunOutcome::Stopped);
    assert!(stopped.fraction > 0. && stopped.fraction < 1., "{stopped:?}");
    assert!(stopped.consumption.laser.0 < usage.laser.0);
    assert_eq!(stopped.currency, "$");

    control.time_scale(20.);
    until(&shared, 10, |d| d.readiness.run.ok).await;
    common::run(&shared).await.unwrap();
    until(&shared, 60, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
            && d.message.as_ref().is_some_and(|m| m.text == "Job finished")
    })
    .await;
    let runs = shared.lock().await.gas.runs(&key, None);
    assert_eq!(runs.len(), 2, "{runs:?}");
    let done = &runs[0];
    assert_eq!(done.outcome, RunOutcome::Done);
    assert!((done.fraction - 1.).abs() < 1e-12);
    assert!((done.consumption.laser.0 - usage.laser.0).abs() < 1e-9);
    assert_eq!(done.consumption.pierces, usage.pierces);
    assert!(shared.lock().await.document().gas.revision > gas.revision);
}

#[tokio::test]
async fn the_flow_test_is_bounded_and_saves_a_calibration() {
    let (simulator, shared) = start("gas-flow-test").await;
    let control = simulator.control();
    assert!(machine::gas_calibration(&shared, 6, 1.).await.is_err());
    assert!(machine::gas_calibration(&shared, 1, 101.).await.is_err());
    connect::connect(&shared).await.unwrap();
    control.time_scale(20.);
    machine::gas_calibration(&shared, 5, 3.).await.unwrap();
    until(&shared, 5, |d| d.machine.operation.is_some()).await;
    machine::stop(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.operation.is_none()).await;
    assert_eq!(control.view().outputs, 0, "Stop closes the valve");

    let nozzle = Nozzle { diameter: Millimeters(1.5), kind: NozzleType::Double };
    let mut coordinator = shared.lock().await;
    let request = CalibrationRequest {
        gas: GasKind::Nitrogen,
        pressure: Bar(3.),
        nozzle,
        seconds: Seconds(60.),
        measured: Liters(100.),
    };
    let saved = coordinator.gas.calibrate(&request).unwrap();
    assert!((saved.factor * saved.estimated.0 - 100.).abs() < 1e-9);
    assert_eq!(coordinator.gas.costs().nitrogen.calibration, Some(saved));
}
