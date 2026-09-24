// SPDX-License-Identifier: GPL-3.0-or-later

//! The harness the server tests share: a simulator, a coordinator over the
//! fixture backup, and a wait on the document.

#![allow(dead_code, reason = "each test file uses part of the harness")]

use openlaser_controller::Simulator;
use openlaser_core::LaserMode;
use openlaser_server::Coordinator;
use openlaser_server::bindings::Files;
use openlaser_server::coordinator::{Config, Shared};
use openlaser_server::document::Document;
use std::path::PathBuf;
use std::time::Duration;

pub fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(relative)
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("openlaser-server-{name}-{}", openlaser_library::Id::generate()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The configuration over the fixture backup, towards `machine`.
pub fn config(name: &str, machine: openlaser_controller::Config) -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        data_dir: scratch(name),
        machine,
        files: Files { backup: Some(fixture("xml/harness-backup.xml")) },
        mode: Some(LaserMode::Fiber),
        co2_manual_focus: true,
        ui_dir: PathBuf::from("ui/dist"),
        network: None,
    }
}

/// The plant answers the fixture's inputs and parameters.
pub async fn seed(shared: &Shared, simulator: &Simulator) {
    let control = simulator.control();
    control.time_scale(20.);
    let (rules, banks) = shared.lock().await.simulated_plant(control.parameters()).unwrap();
    control.rules(&rules);
    control.set_parameters(banks);
}

/// A simulator and a coordinator seeded for it.
pub async fn start(name: &str) -> (Simulator, Shared) {
    let simulator = Simulator::start().await.unwrap();
    let shared = Coordinator::start(config(name, simulator.config())).unwrap();
    seed(&shared, &simulator).await;
    (simulator, shared)
}

/// Waits until the document satisfies `done`, within `seconds`.
pub async fn until(shared: &Shared, seconds: u64, done: impl Fn(&Document) -> bool) {
    let mut documents = shared.lock().await.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        if done(&documents.borrow_and_update()) {
            return;
        }
        tokio::time::timeout_at(deadline, documents.changed())
            .await
            .unwrap_or_else(|_| {
                let d = documents.borrow();
                panic!(
                    "state timeout: program={:?}, recovery={:?}, operation={:?}, message={:?}",
                    d.machine.program,
                    d.recovery.as_ref().map(|r| (&r.state, r.ready, r.selected, &r.problem)),
                    d.machine.operation,
                    d.message
                );
            })
            .expect("coordinator alive");
    }
}

/// Explicit fixture operator responses. Production defaults remain enabled.
pub async fn confirmation(
    shared: &Shared,
    intent: openlaser_server::preflight::PreflightIntent,
) -> openlaser_server::preflight::PreflightConfirmation {
    let review = openlaser_server::preflight::review(shared, intent).await.unwrap();
    openlaser_server::preflight::PreflightConfirmation {
        token: review.token,
        checked: (0..review.steps.len()).collect(),
        gas_ready: review.confirm_gas,
    }
}

/// Calibrates Z for the current material when Start would ask for it, as an
/// operator does before cutting.
pub async fn calibrate(shared: &Shared) {
    let needed = {
        let c = shared.lock().await;
        let doc = c.document();
        doc.bindings.as_ref().is_some_and(|b| b.head_enabled) && !doc.calibration.current
    };
    if !needed {
        return;
    }
    openlaser_server::machine::calibrate(shared).await.unwrap();
    until(shared, 20, |d| d.calibration.current && d.machine.operation.is_none()).await;
}

pub async fn run(shared: &Shared) -> openlaser_server::Result<()> {
    calibrate(shared).await;
    openlaser_server::placement::prepare(shared).await?;
    let confirmation =
        confirmation(shared, openlaser_server::preflight::PreflightIntent::Run).await;
    openlaser_server::machine::run_reviewed(shared, Some(&confirmation)).await
}

pub async fn resume(shared: &Shared) -> openlaser_server::Result<()> {
    let confirmation =
        confirmation(shared, openlaser_server::preflight::PreflightIntent::Resume).await;
    openlaser_server::machine::resume_reviewed(shared, Some(&confirmation)).await
}
