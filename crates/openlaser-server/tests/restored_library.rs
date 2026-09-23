// SPDX-License-Identifier: GPL-3.0-or-later

//! Explicit acceptance of an isolated copy of a desktop library over loopback.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "operator-supplied offline fixture")]
mod common;

use common::until;
use openlaser_controller::{Simulator, state::ProgramState};
use openlaser_server::{Coordinator, connect, machine};
use serde_json::json;

#[tokio::test]
#[ignore = "requires OPENLASER_TEST_LIBRARY (isolated writable copy), BACKUP and REPORT paths"]
async fn restored_library_compiles_and_executes() {
    let simulator = Simulator::start().await.unwrap();
    let control = simulator.control();
    let mut config = common::config("restored-library", simulator.config());
    config.data_dir =
        std::env::var("OPENLASER_TEST_LIBRARY").expect("isolated library copy").into();
    config.files.backup =
        Some(std::env::var("OPENLASER_TEST_BACKUP").expect("fixture backup").into());
    let report = std::env::var("OPENLASER_TEST_REPORT").expect("report destination");
    let shared = Coordinator::start(config).unwrap();
    common::seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed && d.readiness.home.ok).await;
    let mut jobs = vec![None];
    jobs.extend(shared.lock().await.library.jobs().map(|j| Some(j.id.clone())));
    let mut results = Vec::new();
    for id in jobs {
        if let Some(id) = &id {
            // Test the saved job; its separate pending working copy may be invalid.
            openlaser_server::workspace::discard(&shared, &format!("job-{id}")).await.unwrap();
            let mut c = shared.lock().await;
            c.open_job(id).unwrap();
            drop(c);
            machine::prepare(&shared).await.unwrap();
        }
        let (placed, recipe, before) = {
            let c = shared.lock().await;
            let d = c.draft.as_ref().unwrap();
            assert!(d.prepared.is_some(), "restored draft was not prepared: {:?}", d.error);
            (
                d.current.placed.len(),
                d.current.recipe.as_ref().map(|r| (r.id.clone(), r.name.clone())),
                serde_json::to_value(d).unwrap(),
            )
        };
        machine::compile(&shared, false).await.unwrap();
        let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
        let start = std::time::Instant::now();
        if let Err(error) = common::run(&shared).await {
            // A saved job at the exact bed edge can exceed travel after native
            // interpolation. Record that refusal without moving its saved origin.
            assert!(id.is_some() && error.to_string().contains("travel"), "{error}");
            assert!(!control.view().running);
            let row = json!({"job_id":id,"recipe_id":recipe,"passes":compiled.plan.len(),
                "placed_contours":placed,"result":"refused_at_saved_origin", "reason":error.to_string(),
                "source":"saved job; separate pending working copy excluded"});
            println!("RESTORED_LIBRARY {row}");
            results.push(row);
            std::fs::write(&report, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
            continue;
        }
        until(&shared, 240, |d| {
            d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
                && d.machine.operation.is_none()
                && d.readiness.home.ok
        })
        .await;
        let plant = control.view();
        assert!(!plant.running && plant.queued_bytes == 0);
        assert_eq!((plant.outputs, plant.extended_outputs, plant.analog), (0, 0, [0, 0]));
        assert!(plant.pwm.iter().all(|p| p[1] == 0));
        assert!(plant.last_error.is_none(), "{:?}", plant.last_error);
        let c = shared.lock().await;
        assert_eq!(c.document().progress.unwrap().completed, compiled.plan.len());
        assert_eq!(serde_json::to_value(c.draft.as_ref().unwrap()).unwrap(), before);
        let row = json!({"job_id":id,"recipe_id":recipe,"placed_contours":placed,
            "passes":compiled.plan.len(),"blocks":compiled.blocks,"records_executed":plant.executed,
            "program_seconds":compiled.seconds,"elapsed_seconds":start.elapsed().as_secs_f64(),
            "simulator_time_scale":20,"synthetic_short_transfer_mm":12.5,
            "result":"completed","outputs_off":true,"authoring_unchanged":true});
        println!("RESTORED_LIBRARY {row}");
        results.push(row);
        std::fs::write(&report, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}
