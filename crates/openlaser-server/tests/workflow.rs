// SPDX-License-Identifier: GPL-3.0-or-later

//! The whole workflow against the simulator: import a part, make a recipe
//! from the machine files, set up and compile a job, connect, home, run it
//! to completion, then hold one and resume it from the checkpoint.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests drive a known plant")]

mod common;

use common::{fixture, start, until};
use openlaser_controller::state::ProgramState;
use openlaser_core::LaserMode;
use openlaser_core::features::{Features, Lead, LeadShape, Leads, Side};
use openlaser_core::geometry::{Point, Transform};
use openlaser_core::units::{Degrees, Millimeters};
use openlaser_library::Id;
use openlaser_server::coordinator::{NewRecipe, RecipeChange, Shared, Values};
use openlaser_server::document::PathKind;
use openlaser_server::{connect, machine};
use std::time::Duration;

#[tokio::test]
async fn original_recovery_selection_survives_stop_and_reconnect() {
    use openlaser_server::resume::RecoveryChange;
    let (simulator, shared) = start("original-recovery").await;
    let control = simulator.control();
    control.time_scale(2.);
    set_up(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    machine::compile(&shared, false).await.unwrap();
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1)).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    let original_id = {
        let mut c = shared.lock().await;
        let mut revision = c.document().recovery.unwrap().revision;
        for change in [
            RecoveryChange::Select { pass: 0, fraction: 0.4 },
            RecoveryChange::MarkGood,
            RecoveryChange::Next,
            RecoveryChange::Previous,
            RecoveryChange::LastGood,
            RecoveryChange::Forward { distance: 1. },
            RecoveryChange::Backward { distance: 1. },
        ] {
            c.change_recovery(revision, change).unwrap();
            revision = c.document().recovery.unwrap().revision;
        }
        let view = c.document().recovery.unwrap();
        assert!((view.selected.unwrap().fraction - 0.4).abs() < 1e-6);
        assert!(c.change_recovery(revision - 1, RecoveryChange::Skip).is_err());
        c.change_recovery(revision, RecoveryChange::Skip).unwrap();
        let view = c.document().recovery.unwrap();
        assert!(view.steps.iter().any(|s| s.status == "skipped"));
        c.change_recovery(view.revision, RecoveryChange::IncludeAll).unwrap();
        let view = c.document().recovery.unwrap();
        c.change_recovery(view.revision, RecoveryChange::LastGood).unwrap();
        view.id
    };
    machine::stop(&shared).await.unwrap();
    until(&shared, 10, |d| d.recovery.as_ref().is_some_and(|r| r.state == ProgramState::Stopped))
        .await;
    {
        let mut c = shared.lock().await;
        let revision = c.document().recovery.unwrap().revision;
        assert!(c.prepare_recovery(revision, false).is_err());
        assert!(!c.document().can_resume);
    }
    machine::disconnect(&shared).await.unwrap();
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    let revision = {
        let mut c = shared.lock().await;
        let recovery = c.document().recovery.unwrap();
        assert_eq!(recovery.id, original_id);
        c.change_recovery(recovery.revision, RecoveryChange::LastGood).unwrap();
        let revision = c.document().recovery.unwrap().revision;
        c.prepare_recovery(revision, true).unwrap();
        assert!(c.document().can_resume, "prepared recovery: {:?}", c.document().recovery);
        c.document().recovery.unwrap().revision
    };
    machine::move_restart(&shared, revision).await.unwrap();
    until(&shared, 10, |d| {
        d.machine.operation.is_none() && d.machine.feedback.as_ref().is_some_and(|f| f.stationary)
    })
    .await;
    let position = control.view().position_mm;
    let marker = shared.lock().await.document().recovery.unwrap().position.unwrap();
    assert!((position[0] - marker[0]).abs() < 0.01 && (position[1] - marker[1]).abs() < 0.01);
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    control.time_scale(20.);
    common::resume(&shared).await.unwrap();
    until(&shared, 60, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
            && d.machine.operation.is_none()
    })
    .await;
    let recovery = shared.lock().await.document().recovery.unwrap();
    assert_eq!(recovery.id, original_id);
    assert!(recovery.steps.iter().all(|s| s.status == "completed" || s.status == "partial"));
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn manual_pulses_gas_and_table_are_bounded_and_leave_outputs_off() {
    let (simulator, shared) = start("manual-controls").await;
    let control = simulator.control();
    let backup = std::fs::read_to_string(fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("<PLaserParam>", "<PLaserParam><LPF LiftingPlatformType='1'/>")
        .replace(
            "<MP LaserDAKeepOutput",
            "<MP ExchangePlatformType='0' RollSheetType='0' LaserDAKeepOutput",
        )
        .replace("</ParameterRoot>", "<PHomeParam><HPA3 Acc='1000'/></PHomeParam></ParameterRoot>");
    let backup = openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, backup.as_bytes())
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "SoftLimitMaxLen", "100")
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "GoOriginalDirection", "0")
        .unwrap();
    machine::import_file(&shared, "table.xml", backup.original()).await.unwrap();
    common::seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    let before = control.view().position_mm;
    let lease = openlaser_controller::lease::Lease { client: "table-test".into(), sequence: 1 };
    machine::table(&shared, machine::TableRequest { positive: true, speed_mm_s: 10. }, lease)
        .await
        .unwrap();
    until(&shared, 5, |d| {
        d.machine.feedback.as_ref().is_some_and(|f| f.table_mm > 0.)
            && d.machine.operation.is_none()
    })
    .await;
    let moved = control.view().position_mm;
    assert!(moved[4] > 0. && moved[4] < 10., "unrenewed table lease: {moved:?}");
    assert_eq!(&moved[..4], &before[..4], "W must not move XY or the head");
    // The fixture low-O2 path has a 300 ms valve setup, longer than this test.
    machine::gas_test(&shared, 1, 2., 100).await.unwrap();
    until(&shared, 5, |d| d.machine.operation.is_none()).await;
    assert_eq!(control.view().analog, [0, 0]);
    for (mode, interrupt) in [(LaserMode::Fiber, false), (LaserMode::Co2, true)] {
        if mode == LaserMode::Co2 {
            machine::switch_mode(&shared, mode).await.unwrap();
            machine::home(&shared).await.unwrap();
            until(&shared, 10, |d| d.machine.session.homed).await;
        }
        control.time_scale(1.);
        let position = control.view().position_mm;
        assert!(machine::pulse(&shared, 1001, 10).await.is_err());
        machine::pulse(&shared, 1000, 7).await.unwrap();
        let channel = usize::from(mode == LaserMode::Co2);
        until(&shared, 10, |_| control.view().pwm[channel][1] == 7).await;
        if interrupt {
            machine::stop(&shared).await.unwrap();
        }
        until(&shared, 10, |d| {
            d.machine.operation.is_none()
                && d.machine.program.as_ref().is_some_and(|p| {
                    matches!(p.state, ProgramState::Completed | ProgramState::Stopped)
                })
        })
        .await;
        let view = control.view();
        assert!(view.position_mm.iter().zip(position).all(|(a, b)| (a - b).abs() < 1e-12));
        assert_eq!(view.outputs, 0);
        assert_eq!(view.extended_outputs, 0);
        assert_eq!(view.analog, [0, 0]);
        assert!(view.pwm.iter().all(|p| p[1] == 0));
        control.time_scale(20.);
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

/// A plate with leads on, set up on the fixture recipe.
async fn set_up(shared: &Shared) -> Id {
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
    assert_eq!(recipe.summary.speed.as_deref(), Some("50"));
    assert_eq!(recipe.gas, "High N₂", "the fixture bank's selector names the gas");
    coordinator.open_part(&part.id).unwrap();
    coordinator.set_recipe(&recipe.id).unwrap();
    let features = Features {
        leads: Some(Leads {
            entry: Some(Lead {
                shape: LeadShape::Line,
                length: Millimeters(2.),
                radius: Millimeters(1.),
                angle: Degrees(45.),
            }),
            exit: None,
            side: Side::Auto,
            closed_only: true,
            overrides: Vec::new(),
        }),
        ..Features::default()
    };
    coordinator.set_features(features).unwrap();
    drop(coordinator);
    machine::prepare(shared).await.unwrap();
    let mut coordinator = shared.lock().await;
    coordinator.set_origin([100., 100.]).unwrap();
    let draft = coordinator.document().draft.unwrap();
    assert!(draft.preview.is_some(), "{:?}", draft.error);
    part.id
}

/// Setup, compile, connect, home and run: the job finishes, the plant ends
/// where the drawing ends relative to the origin, and the saved job
/// remembers its features for the next job on the recipe.
#[tokio::test]
async fn a_job_runs_to_completion() {
    let (simulator, shared) = start("run").await;
    let control = simulator.control();
    set_up(&shared).await;
    // Planned offline against the files' geometry; connecting reveals the
    // controller's own scale, so the plan is made again.
    machine::compile(&shared, false).await.unwrap();
    let job = shared.lock().await.save_job("plate").unwrap();
    assert!(shared.lock().await.document().library.jobs.iter().any(|j| j.id == job.id));
    assert!(
        matches!(common::run(&shared).await, Err(openlaser_server::Error::Refused(_))),
        "not connected"
    );
    connect::connect(&shared).await.unwrap();
    let document = shared.lock().await.document();
    assert!(document.bindings.is_some(), "{:?}", document.bindings_error);
    assert!(document.message.as_ref().is_some_and(|m| !m.error), "{:?}", document.message);
    assert!(document.draft.unwrap().compiled.is_some(), "connection rebuilds the offline plan");
    {
        let c = shared.lock().await;
        let configuration = c.draft.as_ref().unwrap().compiled.as_ref().unwrap().configuration;
        assert!(configuration.is_some());
        assert_eq!(configuration, c.accepted);
    }
    let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
    assert!(compiled.seconds > 0. && compiled.blocks > 0 && compiled.plan.len() > 1);
    assert!(compiled.moves.iter().any(|m| m.kind == PathKind::Cut));
    assert!(compiled.moves.iter().any(|m| m.kind == PathKind::Travel));
    assert_eq!(compiled.pierces.len(), compiled.plan.len());
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    // The frame walks the part's bounds with the laser off and comes back.
    let before = control.view().position_mm;
    machine::frame(&shared).await.unwrap();
    let framing = shared.lock().await.document();
    assert!(framing.execution.as_ref().is_some_and(|e| e.frame));
    assert_eq!(framing.execution.as_ref().unwrap().name, "plate");
    assert!(
        std::sync::Arc::ptr_eq(&framing.execution.as_ref().unwrap().compiled, &compiled,),
        "the cut geometry must stay visible throughout framing"
    );
    assert!(std::sync::Arc::ptr_eq(
        framing.draft.as_ref().unwrap().compiled.as_ref().unwrap(),
        &compiled,
    ));
    until(&shared, 60, |d| d.message.as_ref().is_some_and(|m| m.text == "Frame finished")).await;
    let framed = shared.lock().await.document();
    assert!(framed.execution.is_none(), "the temporary frame must uncover the compiled job");
    assert!(framed.progress.is_none(), "frame completion is not cut progress");
    assert!(framed.readiness.run.ok, "{:?}", framed.readiness.run.reason);
    assert!(std::sync::Arc::ptr_eq(
        framed.draft.as_ref().unwrap().compiled.as_ref().unwrap(),
        &compiled,
    ));
    let after = control.view().position_mm;
    assert!(
        (before[0] - after[0]).abs() < 0.01 && (before[1] - after[1]).abs() < 0.01,
        "{before:?} {after:?}"
    );
    common::run(&shared).await.unwrap();
    until(&shared, 60, |d| d.message.as_ref().is_some_and(|m| m.text == "Job finished")).await;
    let view = control.view();
    assert!(
        view.position_mm[0] > 100. && view.position_mm[1] > 100.,
        "ended inside the placed part: {:?}",
        view.position_mm
    );
    assert!(!view.running);
    let document = shared.lock().await.document();
    assert_eq!(document.message.as_ref().map(|m| m.text.as_str()), Some("Job finished"));
    assert!(!document.can_resume);

    // A new draft on the same recipe starts from the saved job's features.
    let mut coordinator = shared.lock().await;
    let part = coordinator.document().library.parts[0].id.clone();
    let recipe = coordinator.document().library.recipes[0].id.clone();
    coordinator.open_part(&part).unwrap();
    coordinator.set_recipe(&recipe).unwrap();
    let draft = coordinator.document().draft.unwrap();
    assert!(draft.features.leads.is_some());
    assert_eq!(draft.feature_source.as_ref().map(|s| s.name.clone()), Some("plate".into()));
}

/// A hold captures a checkpoint inside a contour; resuming compiles the
/// continuation from it and runs the rest to completion.
#[tokio::test]
async fn a_held_job_resumes_from_its_checkpoint() {
    let (simulator, shared) = start("resume").await;
    let control = simulator.control();
    control.time_scale(2.);
    set_up(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    let document = shared.lock().await.document();
    assert_eq!(document.machine.program.as_ref().map(|p| p.state), Some(ProgramState::Held));
    let checkpoint = document.machine.program.unwrap().checkpoint.unwrap();
    assert!(checkpoint.item < 0x1000, "held inside a contour: {checkpoint:?}");
    assert_eq!(document.recovery.unwrap().pause_position, Some(checkpoint.position_mm));
    assert!(document.postflight.is_none(), "Pause is not job completion");
    let configuration = document.machine.configuration;
    let original = shared.lock().await.held.clone().unwrap();
    for refused in [
        machine::run_reviewed(&shared, None).await.expect_err("a new run would replace the hold"),
        machine::frame(&shared).await.expect_err("framing would replace the hold"),
    ] {
        assert!(refused.to_string().contains("resume or stop"), "{refused}");
    }
    let readiness = shared.lock().await.document().readiness;
    assert!(!readiness.run.ok && !readiness.frame.ok, "{readiness:?}");
    assert!(readiness.resume.ok, "{readiness:?}");
    assert!(std::sync::Arc::ptr_eq(&shared.lock().await.held.clone().unwrap().job, &original.job));
    let id = document.execution.unwrap().id;
    let parked = [checkpoint.position_mm[0] + 20., checkpoint.position_mm[1] + 15.];
    machine::go_xy(&shared, parked).await.unwrap();
    until(&shared, 15, |d| {
        d.machine.operation.is_none()
            && d.machine
                .feedback
                .as_ref()
                .is_some_and(|f| (f.position_mm[0] - parked[0]).abs() < 0.01)
    })
    .await;
    let moved = shared.lock().await.document();
    assert_eq!(
        moved.machine.configuration, configuration,
        "Pause and manual movement retain the session"
    );
    assert!(matches!(
        moved.machine.connection,
        openlaser_controller::state::Connection::Connected { .. }
    ));
    assert_eq!(moved.machine.program.unwrap().checkpoint, Some(checkpoint));
    assert_eq!(moved.recovery.unwrap().pause_position, Some(checkpoint.position_mm));
    assert!(moved.can_resume);
    assert_eq!(control.view().outputs, 0);
    assert!(machine::resume(&shared).await.is_err(), "Resume needs fresh gas confirmation");
    common::resume(&shared).await.unwrap();
    until(&shared, 20, |d| {
        d.execution.as_ref().is_some_and(|e| e.id != id)
            && d.machine.program.as_ref().is_some_and(|p| p.started)
            && d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1)
    })
    .await;
    let remainder = shared.lock().await.held.clone().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&original.job, &remainder.job));
    let restart = remainder.job.passes[0].start;
    for (axis, coordinate) in restart.iter().enumerate() {
        assert!(
            (coordinate + remainder.sheet_offset[axis] - checkpoint.position_mm[axis]).abs()
                < 1e-10
        );
    }
    let return_move = remainder.view.moves.iter().find(|m| m.kind == PathKind::Travel).unwrap();
    let returned = return_move.points.last().unwrap();
    for (axis, coordinate) in returned.iter().enumerate() {
        assert!(
            (coordinate + remainder.sheet_offset[axis] - checkpoint.position_mm[axis]).abs()
                < 1e-10
        );
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    control.time_scale(20.);
    common::resume(&shared).await.unwrap();
    until(&shared, 60, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
            && d.message.as_ref().is_some_and(|m| m.text == "Job finished")
    })
    .await;
    assert_eq!(
        shared.lock().await.document().message.as_ref().map(|m| m.text.as_str()),
        Some("Job finished")
    );
    assert!(!control.view().running);
    assert!(matches!(
        shared.machine.state().connection,
        openlaser_controller::state::Connection::Connected { .. }
    ));
}

/// A temporary framing operation must also release its display on Stop or
/// an alarm, without discarding the cut program or masking the alarm gate.
#[tokio::test]
async fn interrupted_framing_preserves_the_compiled_job() {
    use openlaser_controller::simulator::Fault;

    for alarm in [false, true] {
        let (simulator, shared) = start(if alarm { "frame-alarm" } else { "frame-stop" }).await;
        let control = simulator.control();
        set_up(&shared).await;
        connect::connect(&shared).await.unwrap();
        machine::home(&shared).await.unwrap();
        until(&shared, 10, |d| d.machine.session.homed).await;
        machine::compile(&shared, false).await.unwrap();
        let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
        control.time_scale(1.);
        machine::frame(&shared).await.unwrap();
        until(&shared, 10, |d| d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1))
            .await;
        assert!(shared.lock().await.document().execution.as_ref().is_some_and(|e| e.frame));
        if alarm {
            control.fault(Fault::EmergencyStop, true);
        } else {
            machine::stop(&shared).await.unwrap();
        }
        until(&shared, 10, |d| d.message.as_ref().is_some_and(|m| m.text == "Frame stopped")).await;
        let document = shared.lock().await.document();
        assert!(document.execution.is_none());
        assert!(document.progress.is_none());
        assert!(std::sync::Arc::ptr_eq(
            document.draft.as_ref().unwrap().compiled.as_ref().unwrap(),
            &compiled,
        ));
        assert_eq!(document.readiness.run.ok, !alarm, "{:?}", document.readiness.run.reason);
        assert!(!control.view().running);
        openlaser_server::shutdown(&shared).await.unwrap();
    }
}

#[tokio::test]
async fn one_run_owns_admission_and_stop_bypasses_the_document_lock() {
    let (simulator, shared) = start("run-ownership").await;
    let control = simulator.control();
    control.time_scale(1.);
    set_up(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    let (first, second) = tokio::join!(common::run(&shared), common::run(&shared));
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let mut c = shared.lock().await;
    let execution = c.document().execution.unwrap();
    let part = c.document().draft.unwrap().parts[0].id.clone();
    c.open_part(&part).unwrap();
    assert!(std::sync::Arc::ptr_eq(&execution, c.document().execution.as_ref().unwrap()));
    tokio::time::timeout(Duration::from_secs(2), machine::stop(&shared)).await.unwrap().unwrap();
    drop(c);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn an_outward_lead_is_refused_before_run_dry_run_or_frame_writes() {
    let (simulator, shared) = start("envelope").await;
    let control = simulator.control();
    set_up(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    {
        let mut c = shared.lock().await;
        let extent = c.bound().unwrap().jog.extent;
        c.set_anchor(openlaser_library::Anchor::BackRight).unwrap();
        c.set_origin([extent[0][1] - 0.1, extent[1][1] - 0.1]).unwrap();
    }
    for dry in [false, true] {
        machine::compile(&shared, dry).await.unwrap();
        control.clear_writes();
        let refused = common::run(&shared).await.unwrap_err();
        assert!(refused.to_string().contains("leaves machine travel"), "{refused}");
        assert!(control.writes().is_empty());
        let refused = machine::frame(&shared).await.unwrap_err();
        assert!(refused.to_string().contains("leaves machine travel"), "{refused}");
        assert!(control.writes().is_empty());
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

/// A vendor recipe file imports with its name, gas, note and photo, only
/// once, compiles through the merged layer file although the backup lacks
/// its slow-start attributes, and an imported backup replaces the
/// configured one and rebinds a connected machine.
#[tokio::test]
async fn imported_recipes_and_machine_files() {
    let (_simulator, shared) = start("import").await;
    let part = set_up(&shared).await;
    let recipe_file = std::fs::read(fixture("xml/recipe-fiber.xml")).unwrap();
    let photo = std::fs::read(fixture("xml/photo.png")).unwrap();
    let (recipe, existing) =
        shared.lock().await.import_recipe("SS-2MM_AIR CUTTING.xml", &recipe_file).unwrap();
    assert!(!existing);
    assert_eq!((recipe.name.as_str(), recipe.thickness_mm, recipe.gas.as_str()), ("SS", 2., "Air"));
    assert_eq!(recipe.layer, 11);
    assert_eq!(recipe.tags, vec!["AIR CUTTING".to_owned()]);
    assert!(recipe.note.starts_with("NOZZLE---SINGLE-2.0\n"));
    assert_eq!(recipe.summary.speed.as_deref(), Some("60"));
    let (again, existing) = shared.lock().await.import_recipe("copy.xml", &recipe_file).unwrap();
    assert!(existing && again.id == recipe.id, "the same file imports once");
    assert!(shared.lock().await.set_photo(&recipe.id, b"not a png").is_err());
    shared.lock().await.set_photo(&recipe.id, &photo).unwrap();
    let view = shared
        .lock()
        .await
        .document()
        .library
        .recipes
        .iter()
        .find(|r| r.id == recipe.id)
        .unwrap()
        .clone();
    let sha256 = view.photo.clone().unwrap();
    assert_eq!(shared.lock().await.photo(&sha256).unwrap(), ("image/png", photo));

    // The imported recipe binds and compiles against the harness backup,
    // whose only fiber bank is layer 1 without the slow-start attributes.
    let mut coordinator = shared.lock().await;
    coordinator.open_part(&part).unwrap();
    coordinator.set_recipe(&recipe.id).unwrap();
    drop(coordinator);
    machine::prepare(&shared).await.unwrap();
    let mut coordinator = shared.lock().await;
    coordinator.set_origin([100., 100.]).unwrap();
    drop(coordinator);
    connect::connect(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    let compiled = shared.lock().await.document().draft.unwrap().compiled.clone().unwrap();
    assert!(compiled.plan.len() > 1);

    // Importing the backup replaces the configured file under its own
    // name, keeps the machine bound, and asks for a fresh verification. A
    // backup that parses but lacks a group is refused and changes nothing.
    let backup = std::fs::read(fixture("xml/harness-backup.xml")).unwrap();
    assert!(matches!(
        machine::import_file(&shared, "notes.txt", b"x").await,
        Err(openlaser_server::Error::Request(_))
    ));
    let text = String::from_utf8(backup.clone()).unwrap();
    let (from, to) = (text.find("<PAxisParam>").unwrap(), text.find("</PAxisParam>").unwrap() + 13);
    let trimmed = format!("{}{}", &text[..from], &text[to..]);
    let refused =
        machine::import_file(&shared, "1390backup.xml", trimmed.as_bytes()).await.unwrap_err();
    assert!(
        matches!(&refused, openlaser_server::Error::Request(text) if text.starts_with("1390backup.xml has no PAxisParam")),
        "{refused}"
    );
    assert_eq!(
        shared.lock().await.document().files.backup.as_ref().map(|f| f.name.as_str()),
        Some("harness-backup.xml"),
        "the configured backup stays"
    );
    machine::import_file(&shared, "1390backup.xml", &backup).await.unwrap();
    let document = shared.lock().await.document();
    assert_eq!(document.files.backup.as_ref().map(|f| f.name.as_str()), Some("1390backup.xml"));
    assert!(document.files.banks.iter().any(|b| b.laser == LaserMode::Fiber && b.bank == 1));
    assert!(document.bindings.is_some(), "{:?}", document.bindings_error);
    assert!(
        document.message.as_ref().is_some_and(|m| m.text.starts_with("Imported")),
        "{:?}",
        document.message
    );
    let rebuilt = document.draft.unwrap().compiled.clone().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&rebuilt, &compiled), "new files rebuild the job");
    {
        let c = shared.lock().await;
        let configuration = c.draft.as_ref().unwrap().compiled.as_ref().unwrap().configuration;
        assert!(configuration.is_some());
        assert_eq!(configuration, c.accepted);
    }
    assert!(document.readiness.jog.reason.is_none() || !document.readiness.jog.ok);
}

/// A fresh library seeds the bundled materials once, under their full
/// names with the gas each file names; a thickness copies a recipe's
/// values, and a gas selection renames the gas.
#[tokio::test]
async fn the_defaults_seed_once_and_a_thickness_copies() {
    let (_simulator, shared) = start("defaults").await;
    let mut coordinator = shared.lock().await;
    coordinator.seed_defaults();
    let recipes = coordinator.document().library.recipes.clone();
    assert_eq!(recipes.len(), 51);
    let names: std::collections::BTreeSet<&str> = recipes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        ["Aluminum", "Basswood", "Brass", "Carbon Steel", "Galvanized Steel", "Stainless Steel"]
    );
    let steel = recipes
        .iter()
        .find(|r| r.name == "Carbon Steel" && (r.thickness_mm - 3.).abs() < 1e-9 && r.gas == "O2")
        .unwrap();
    let copy = coordinator.duplicate_recipe(&steel.id, 3.5).unwrap();
    assert_eq!(
        (copy.name.as_str(), copy.thickness_mm, copy.gas.as_str()),
        ("Carbon Steel", 3.5, "O2")
    );
    assert_eq!(copy.attributes, steel.attributes);
    let gas = RecipeChange {
        attributes: [("CutGasType".to_owned(), "3".to_owned())].into(),
        ..RecipeChange::default()
    };
    coordinator.update_recipe(&copy.id, gas).unwrap();
    let renamed = coordinator
        .document()
        .library
        .recipes
        .clone()
        .into_iter()
        .find(|r| r.id == copy.id)
        .unwrap();
    assert_eq!(renamed.gas, "High Air");
    let custom = coordinator
        .add_recipe(&NewRecipe {
            name: "Weathering steel".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 4.,
            values: Values::Recipe(steel.id.clone()),
            gas: Some(1),
        })
        .unwrap();
    assert_eq!((custom.gas.as_str(), custom.attributes["CutGasType"].as_str()), ("Low O₂", "1"));
    assert_eq!(custom.attributes["CutSpeed"], steel.attributes["CutSpeed"]);
    assert_eq!(coordinator.document().files.gases, vec![1, 3, 5], "the valves the backup assigns");
    for r in &recipes {
        coordinator.remove_recipe(&r.id).unwrap();
    }
    coordinator.seed_defaults();
    assert_eq!(
        coordinator.document().library.recipes.len(),
        2,
        "seeded once: the copy and the custom recipe stay"
    );
}

/// Copies, removal, undo and redo go through the coordinator with the
/// sheet prepared again each time, and the refusals are the right ones:
/// the sheet keeps at least one contour, and only the drawing's own
/// contours can be copied.
#[tokio::test]
async fn the_sheet_edits_are_checked_and_undone() {
    let (_simulator, shared) = start("edits").await;
    set_up(&shared).await;
    let mut coordinator = shared.lock().await;
    let count = coordinator.document().draft.unwrap().placed.len();
    let initial_history = coordinator.document().draft.unwrap().past;
    let added = coordinator.add(&[(0, Transform::translation(Point::new(200., 0.)))]).unwrap();
    assert_eq!(added, vec![count]);
    drop(coordinator);
    machine::prepare(&shared).await.unwrap();
    let mut coordinator = shared.lock().await;
    let draft = coordinator.document().draft.unwrap();
    assert_eq!(draft.placed.len(), count + 1);
    assert_eq!(draft.groups.len(), 2, "the copy is its own shape");
    let everything: Vec<usize> = (0..=count).collect();
    assert!(matches!(coordinator.remove(&everything), Err(openlaser_server::Error::Refused(_))));
    assert!(matches!(
        coordinator.add(&[(99, Transform::IDENTITY)]),
        Err(openlaser_server::Error::Request(_))
    ));
    coordinator.remove(&[count]).unwrap();
    assert_eq!(coordinator.document().draft.unwrap().placed.len(), count);
    coordinator.undo().unwrap();
    assert_eq!(coordinator.document().draft.unwrap().placed.len(), count + 1);
    coordinator.redo().unwrap();
    let draft = coordinator.document().draft.unwrap();
    assert_eq!((draft.placed.len(), draft.past, draft.future), (count, initial_history + 2, 0));
    assert!(matches!(coordinator.redo(), Err(openlaser_server::Error::Refused(_))));
}
