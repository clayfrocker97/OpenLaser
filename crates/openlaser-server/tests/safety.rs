// SPDX-License-Identifier: GPL-3.0-or-later

//! Operator checks and alarm recording, entirely on local fixture data and UDP loopback.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture and simulator assertions")]

mod common;

use common::{confirmation, fixture, start, until};
use openlaser_controller::alarms::Observation;
use openlaser_controller::simulator::Fault;
use openlaser_controller::state::{AlarmView, ProgramState, ReliefView};
use openlaser_core::LaserMode;
use openlaser_library::preflight::{Check, CheckAction, JobPreflight};
use openlaser_server::alarm_history::History;
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::preflight::{self, PreflightIntent};
use openlaser_server::{Coordinator, connect, machine};
use std::time::Duration;

fn check(text: &str) -> Check {
    Check { text: text.into(), action: None, auto_check: false }
}

fn automatic(action: CheckAction) -> Check {
    Check { text: "Fixture setup action".into(), action: Some(action), auto_check: true }
}

async fn invoke_check(shared: &Shared, step: usize) {
    let review = preflight::review(shared, PreflightIntent::Run).await.unwrap();
    preflight::action(shared, &review.token, step).await.unwrap();
}

async fn setup(shared: &Shared) {
    let bytes = std::fs::read(fixture("dxf/plate-with-holes.dxf")).unwrap();
    {
        let mut c = shared.lock().await;
        let part = c.import_part("plate.dxf", &bytes).unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 2.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
    }
    machine::prepare(shared).await.unwrap();
    shared.lock().await.set_origin([100., 100.]).unwrap();
}

async fn ready(shared: &Shared) {
    setup(shared).await;
    connect::connect(shared).await.unwrap();
    machine::home(shared).await.unwrap();
    until(shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    machine::compile(shared, false).await.unwrap();
}

async fn ready_small(shared: &Shared, simulator: &openlaser_controller::Simulator) {
    let backup = std::fs::read_to_string(fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("SoftLimitMaxLen='1371'", "SoftLimitMaxLen='50'")
        .replace("SoftLimitMaxLen='971'", "SoftLimitMaxLen='40'");
    machine::import_file(shared, "small-bed.xml", backup.as_bytes()).await.unwrap();
    common::seed(shared, simulator).await;
    setup(shared).await;
    shared.lock().await.set_origin([5., 5.]).unwrap();
    connect::connect(shared).await.unwrap();
    machine::home(shared).await.unwrap();
    until(shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    machine::compile(shared, false).await.unwrap();
}

#[tokio::test]
async fn preflight_blocks_missing_partial_stale_and_reused_responses_before_writes() {
    let (simulator, shared) = start("preflight-admission").await;
    ready(&shared).await;
    let control = simulator.control();
    control.clear_writes();
    assert!(machine::run(&shared).await.unwrap_err().to_string().contains("preflight"));
    let accepted = confirmation(&shared, PreflightIntent::Run).await;
    assert!(!accepted.checked.is_empty() && !accepted.gas_ready);
    let mut incomplete = accepted.clone();
    incomplete.checked.pop();
    assert!(
        machine::run_reviewed(&shared, Some(&incomplete))
            .await
            .unwrap_err()
            .to_string()
            .contains("every preflight")
    );
    incomplete = accepted.clone();
    incomplete.checked.push(0);
    assert!(machine::run_reviewed(&shared, Some(&incomplete)).await.is_err());
    incomplete = accepted.clone();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    let gas_check = review.steps.iter().position(Check::checks_gas).unwrap();
    incomplete.checked.retain(|&step| step != gas_check);
    assert!(
        machine::run_reviewed(&shared, Some(&incomplete))
            .await
            .unwrap_err()
            .to_string()
            .contains("every preflight")
    );
    assert!(control.writes().is_empty(), "no enabling writes before confirmation");

    shared.lock().await.set_origin([101., 100.]).unwrap();
    assert!(
        machine::run_reviewed(&shared, Some(&accepted))
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(control.writes().is_empty());
    let fresh = confirmation(&shared, PreflightIntent::Run).await;
    machine::stop(&shared).await.unwrap();
    control.clear_writes();
    assert!(
        machine::run_reviewed(&shared, Some(&fresh))
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(control.writes().is_empty(), "Stop invalidates an already-open checklist");

    let fresh = confirmation(&shared, PreflightIntent::Run).await;
    machine::run_reviewed(&shared, Some(&fresh)).await.unwrap();
    until(&shared, 15, |d| {
        d.machine.operation.is_none()
            && d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
    })
    .await;
    assert!(
        machine::run_reviewed(&shared, Some(&fresh))
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn defaults_and_job_policies_survive_restart_without_completed_checkboxes() {
    let (_simulator, shared) = start("preflight-persistence").await;
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    let job;
    let config;
    {
        let mut c = shared.lock().await;
        let mut preferences = c.preflight.clone();
        preferences.fiber.steps = vec![check("Fiber setup")];
        preferences.co2.steps = vec![check("CO2 setup"), check("Lens checked")];
        let stale = preferences.clone();
        c.save_preflight_preferences(preferences).unwrap();
        assert!(c.save_preflight_preferences(stale).is_err());
        assert_eq!(
            c.preflight_review(PreflightIntent::Run, 0).unwrap().steps[0].text,
            "Fiber setup"
        );
        c.set_preflight(JobPreflight::Custom { steps: vec![check("Clamps are clear")] }).unwrap();
        job = c.save_job("Custom checks").unwrap().id;
        config = c.config.clone();
        let bytes =
            std::fs::read(config.data_dir.join("jobs").join(format!("{job}.json"))).unwrap();
        let saved = String::from_utf8(bytes).unwrap();
        assert!(
            !saved.contains("gas_ready") && !saved.contains("token") && !saved.contains("checked")
        );
    }
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    {
        let mut c = reopened.lock().await;
        assert_eq!(c.preflight.fiber.steps.len(), 1);
        assert_eq!(c.preflight.co2.steps.len(), 2);
        c.open_job(&job).unwrap();
        assert_eq!(
            c.draft.as_ref().unwrap().current.preflight,
            JobPreflight::Custom { steps: vec![check("Clamps are clear")] }
        );
    }
    machine::prepare(&reopened).await.unwrap();
    machine::compile(&reopened, false).await.unwrap();
    let custom = preflight::review(&reopened, PreflightIntent::Run).await.unwrap();
    assert_eq!(custom.steps.len(), 1);
    reopened.lock().await.set_preflight(JobPreflight::Off {}).unwrap();
    let off = preflight::review(&reopened, PreflightIntent::Run).await.unwrap();
    assert!(off.steps.is_empty() && !off.confirm_gas, "gas is a normal editable checklist step");
    openlaser_server::shutdown(&reopened).await.unwrap();
}

#[tokio::test]
async fn process_import_rebinds_cut_and_film_and_rejects_bad_or_stale_replacements() {
    let (_simulator, shared) = start("soft-import").await;
    setup(&shared).await;
    {
        let mut c = shared.lock().await;
        let draft = c.draft.as_mut().unwrap();
        let mut film = draft.current.recipe.clone().unwrap();
        film.attributes.remove("WithFilm");
        draft.current.film = Some(film);
        draft.current.recipe.as_mut().unwrap().attributes.insert("WithFilm".into(), "1".into());
    }
    machine::compile(&shared, false).await.unwrap();
    let stale = shared.lock().await.compile_inputs(false).unwrap();
    let source = b"[Network]\nIP=10.1.1.168\n[Soft]\nFollowOvertime=9000\nSectionDrillOvertime=7000\nDAMinVal=75\n";
    machine::import_soft(&shared, "ipAdd.ini", source).await.unwrap();
    let config;
    {
        let mut c = shared.lock().await;
        let rebuilt = c.draft.as_ref().unwrap().compiled.as_ref().unwrap();
        assert_eq!(rebuilt.job.settings.timeouts.follow_ms, 9000);
        assert_eq!(rebuilt.job.settings.timeouts.section_drill_ms, 7000);
        assert_eq!(rebuilt.job.settings.timeouts.analog_minimum, 75);
        let input = c.compile_inputs(false).unwrap();
        assert_eq!(input.settings.timeouts.follow_ms, 9000);
        assert_eq!(input.settings.timeouts.section_drill_ms, 7000);
        assert_eq!(input.settings.timeouts.analog_minimum, 75);
        assert_eq!(input.film.as_ref().unwrap().timeouts, input.settings.timeouts);
        assert!(
            c.compiled(stale.stamp, Err(openlaser_server::Error::Refused("stale".into())))
                .unwrap_err()
                .to_string()
                .contains("superseded")
        );
        config = c.config.clone();
    }
    assert!(machine::import_soft(&shared, "bad.ini", b"[Soft]\nFollowOvertime=0\n").await.is_err());
    assert_eq!(shared.lock().await.soft.bytes().unwrap(), source);
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    assert_eq!(reopened.lock().await.soft.timeouts.follow_ms, 9000);
    assert_eq!(reopened.lock().await.soft.bytes().unwrap(), source);
    openlaser_server::shutdown(&reopened).await.unwrap();
}

fn alarm(active: bool) -> AlarmView {
    AlarmView {
        active,
        relief: ReliefView { label: "Reset".into(), moves_axes: false },
        id: Some(60),
        source: "source input".into(),
        label: "Laser source warning".into(),
        title: "Laser source alarm".into(),
        fix: "Read the alarm on the laser source.".into(),
        blocking: true,
        latched: false,
        age_seconds: 0,
    }
}

#[tokio::test]
async fn historical_alarms_are_counted_durable_and_separated_by_restart() {
    let root = common::scratch("alarm-history");
    let (sender, observations) = tokio::sync::mpsc::unbounded_channel();
    let history = History::start(&root, observations).unwrap();
    // A reset result may reach the worker before its observation relay.
    history.reset(
        &[alarm(true)],
        Some(60),
        &Err(openlaser_controller::Error::Refused("still active".into())),
    );
    for (at, alarms) in [
        (10, vec![alarm(true)]),
        (11, vec![alarm(true)]),
        (12, vec![alarm(false)]),
        (13, vec![alarm(true)]),
    ] {
        sender.send(Observation { at, alarms, fault: None }).unwrap();
    }
    drop(sender);
    history.flush().await.unwrap();
    let first = history.page(None).await.unwrap();
    let row = &first.sessions[0].alarms[0];
    assert_eq!((row.first, row.last, row.count), (10, 13, 2));
    assert!(row.active && row.recovered.is_none());
    assert!(!row.reset.as_ref().unwrap().ok);
    assert!(history.page(Some("../../preflight".into())).await.is_err());
    let previous_bytes =
        std::fs::read(root.join("alarm-history").join(format!("{}.json", first.current))).unwrap();
    let (sender, observations) = tokio::sync::mpsc::unbounded_channel();
    let restarted = History::start(&root, observations).unwrap();
    let page = restarted.page(None).await.unwrap();
    assert_ne!(page.current, first.current);
    assert_eq!(page.sessions.len(), 2);
    assert!(page.sessions[0].alarms.is_empty());
    assert_eq!(page.sessions[1].id, first.current);
    assert!(page.sessions[1].alarms[0].active, "restart must not fabricate a recovery");
    assert_eq!(
        std::fs::read(root.join("alarm-history").join(format!("{}.json", first.current))).unwrap(),
        previous_bytes
    );
    drop(sender);
    restarted.flush().await.unwrap();
}

#[tokio::test]
async fn alarm_edges_survive_a_blocked_document_publisher_without_becoming_live_history() {
    let (simulator, shared) = start("alarm-transitions").await;
    connect::connect(&shared).await.unwrap();
    let control = simulator.control();
    let mut states = shared.machine.watch();
    let coordinator = shared.lock().await;
    for present in [true, false, true, false] {
        control.fault(Fault::EmergencyStop, present);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let found = states
                    .borrow_and_update()
                    .alarms
                    .iter()
                    .any(|a| a.source == "controller group 1 bit 30");
                if found == present {
                    break;
                }
                states.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    drop(coordinator);
    openlaser_server::shutdown(&shared).await.unwrap();
    let history = shared.lock().await.history.clone();
    let page = history.page(None).await.unwrap();
    let row =
        page.sessions[0].alarms.iter().find(|a| a.source == "controller group 1 bit 30").unwrap();
    assert_eq!(row.count, 2);
    assert!(!row.active);
    assert!(!shared.machine.state().alarms.iter().any(|a| a.source == row.source));
}

#[tokio::test]
async fn typed_checklist_actions_are_explicit_bounded_and_do_not_check_the_box() {
    let (simulator, shared) = start("checklist-actions").await;
    ready(&shared).await;
    let control = simulator.control();
    let policy = JobPreflight::Custom {
        steps: vec![Check {
            text: "Gas flow checked".into(),
            action: Some(CheckAction::GasTest { selector: 5, pressure: 3., duration_ms: 500 }),
            auto_check: false,
        }],
    };
    shared.lock().await.set_preflight(policy).unwrap();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    control.clear_writes();
    assert!(preflight::action(&shared, "stale", 0).await.is_err());
    assert!(preflight::action(&shared, &review.token, 1).await.is_err());
    assert!(control.writes().is_empty());
    preflight::action(&shared, &review.token, 0).await.unwrap();
    until(&shared, 5, |d| d.message.as_ref().is_some_and(|m| m.text == "Gas test done")).await;
    assert_eq!(control.view().outputs, 0, "the timer closes the valve without a browser release");
    assert!(machine::run(&shared).await.unwrap_err().to_string().contains("preflight"));
    assert!(machine::gas_test(&shared, 5, 3., 2001).await.is_err());
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn automatic_checks_follow_current_facts_and_origin_edits_do_not_move_axes() {
    use openlaser_controller::session::Quality;
    use openlaser_library::Anchor;
    let (simulator, shared) = start("automatic-preflight").await;
    ready_small(&shared, &simulator).await;
    let control = simulator.control();
    shared
        .lock()
        .await
        .set_preflight(JobPreflight::Custom {
            steps: vec![
                automatic(CheckAction::Home {}),
                automatic(CheckAction::Origin {}),
                automatic(CheckAction::SetOrigin {}),
                automatic(CheckAction::SetOriginAt { point: Anchor::BackLeft }),
                automatic(CheckAction::MoveTo { point: Anchor::FrontLeft }),
                automatic(CheckAction::Calibrate {}),
            ],
        })
        .unwrap();
    control.clear_writes();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(review.satisfied, vec![0, 4]);
    assert!(control.writes().is_empty(), "a review does not execute actions");
    for index in [2, 3] {
        assert!(review.steps[index].action.is_none());
        assert!(!review.steps[index].auto_check);
        assert!(preflight::action(&shared, &review.token, index).await.is_err());
    }
    shared.lock().await.set_origin([0., 0.]).unwrap();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(review.satisfied, vec![0, 1, 4]);
    assert!(control.writes().is_empty(), "setting an origin only edits the draft");
    let old = confirmation(&shared, PreflightIntent::Run).await;
    {
        let mut c = shared.lock().await;
        let origin = c.bed_point(Anchor::BackLeft).unwrap();
        c.set_origin_at(origin, Some(Anchor::BackLeft)).unwrap();
    }
    assert!(control.writes().is_empty());
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(review.satisfied, vec![0, 4]);
    {
        let c = shared.lock().await;
        let draft = c.draft.as_ref().unwrap();
        assert_eq!(draft.current.anchor, Anchor::BackLeft);
        assert_eq!(draft.origin(), Some(c.bed_point(Anchor::BackLeft).unwrap()));
    }
    assert!(machine::run_reviewed(&shared, Some(&old)).await.is_err());
    assert!(control.writes().is_empty(), "the old confirmation cannot start this placement");
    invoke_check(&shared, 1).await;
    until(&shared, 10, |d| d.readiness.home.ok).await;
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(review.satisfied, vec![0, 1]);
    control.calibration_quality(Quality::Excellent);
    invoke_check(&shared, 5).await;
    until(&shared, 10, |d| {
        d.machine.operation.is_none() && d.machine.session.calibration == Some(Quality::Excellent)
    })
    .await;
    assert!(preflight::review(&shared, PreflightIntent::Run).await.unwrap().satisfied.contains(&5));
    control.calibration_quality(Quality::Bad);
    invoke_check(&shared, 5).await;
    until(&shared, 10, |d| {
        d.machine.operation.is_none()
            && d.message.as_ref().is_some_and(|m| m.error && m.text.contains("bad calibration"))
    })
    .await;
    let document = shared.lock().await.document();
    assert!(
        matches!(
            document.machine.connection,
            openlaser_controller::state::Connection::Connected { .. }
        ),
        "a bad result is a finished calibration, not a lost connection"
    );
    assert_eq!(document.machine.session.calibration, Some(Quality::Bad));
    assert!(document.machine.session.homed);
    assert!(
        !preflight::review(&shared, PreflightIntent::Run).await.unwrap().satisfied.contains(&5)
    );
    machine::disconnect(&shared).await.unwrap();
    assert!(preflight::review(&shared, PreflightIntent::Run).await.unwrap().satisfied.is_empty());
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn every_bed_point_uses_bounded_laser_off_travel_and_live_auto_checks() {
    use openlaser_library::Anchor;
    let (simulator, shared) = start("preflight-bed-points").await;
    ready_small(&shared, &simulator).await;
    let control = simulator.control();
    let points = [
        Anchor::FrontLeft,
        Anchor::FrontCenter,
        Anchor::FrontRight,
        Anchor::Right,
        Anchor::BackRight,
        Anchor::BackCenter,
        Anchor::BackLeft,
        Anchor::Left,
        Anchor::Center,
    ];
    shared
        .lock()
        .await
        .set_preflight(JobPreflight::Custom {
            steps: points
                .into_iter()
                .map(|point| automatic(CheckAction::MoveTo { point }))
                .collect(),
        })
        .unwrap();
    let initial_z = shared.machine.state().feedback.unwrap().position_mm[2];
    for (index, point) in points.into_iter().enumerate() {
        let target = shared.lock().await.bed_point(point).unwrap();
        invoke_check(&shared, index).await;
        until(&shared, 10, |d| {
            d.machine.operation.is_none()
                && d.machine.feedback.as_ref().is_some_and(|f| {
                    (f.position_mm[0] - target[0]).abs() < 0.01
                        && (f.position_mm[1] - target[1]).abs() < 0.01
                })
        })
        .await;
        let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
        assert_eq!(
            review.satisfied,
            vec![index],
            "target={target:?}, machine={:?}",
            shared.machine.state()
        );
        let feedback = shared.machine.state().feedback.unwrap();
        assert!((feedback.position_mm[2] - initial_z).abs() < 0.001);
        let plant = control.view();
        assert_eq!(plant.outputs, 0);
        assert_eq!(plant.extended_outputs, 0);
        assert!(plant.pwm.iter().all(|pwm| pwm[1] == 0));
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn gas_confirmation_uses_every_compiled_process_and_skips_dry_runs() {
    let (_simulator, shared) = start("preflight-gases").await;
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    let mut job = {
        let c = shared.lock().await;
        (*c.draft.as_ref().unwrap().compiled.as_ref().unwrap().job).clone()
    };
    job.settings.gas = 0;
    job.settings.pierce = vec![openlaser_compiler::settings::PierceStage {
        index: 1,
        height: 1.,
        power: 1,
        frequency: 100,
        peak_current: 1.,
        gas: 1,
        pressure: 1.,
        duration_ms: 50,
        gradual: false,
        gradual_ms: 0,
        before_off_ms: 0,
        after_off_ms: 0,
        bolt: None,
    }];
    job.settings.residue = Some(openlaser_compiler::settings::Residue {
        height: 1.,
        speed: 1.,
        gas: 2,
        pressure: 1.,
        peak_current: 1.,
        power: 1,
        frequency: 100,
        radius: 1.,
        turns: 1,
    });
    job.passes[0].settings.gas = 3;
    let gases = preflight::required_gases(&job);
    for name in ["Low Air", "Low O₂", "Low N₂", "High Air"] {
        assert!(gases.iter().any(|gas| gas == name), "{name}: {gases:?}");
    }
    machine::compile(&shared, true).await.unwrap();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert!(review.dry_run && review.gases.is_empty() && !review.confirm_gas);
    assert!(!review.steps.is_empty(), "dry motion still shows the operator checklist");
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn postflight_belongs_to_completed_job_and_uses_shared_parking_actions() {
    use openlaser_server::postflight;
    let (simulator, shared) = start("postflight-parking").await;
    ready(&shared).await;
    {
        let mut c = shared.lock().await;
        let mut defaults = c.preflight.clone();
        defaults.postflight.fiber.steps =
            vec![automatic(CheckAction::MoveXy { x: 7., y: 8. }), check("Inspect the parts")];
        c.save_preflight_preferences(defaults).unwrap();
    }
    assert!(postflight::review(&shared).await.is_none());
    common::run(&shared).await.unwrap();
    until(&shared, 15, |d| d.postflight.is_some()).await;
    let control = simulator.control();
    control.clear_writes();
    let review = postflight::review(&shared).await.unwrap();
    assert_eq!(review.notice.name, "plate");
    assert_eq!(review.steps.len(), 2);
    assert!(review.satisfied.is_empty());
    assert!(control.writes().is_empty(), "reading a checklist must never move the head");
    assert!(postflight::action(&shared, "old-completion", 0).await.is_err());
    assert!(postflight::action(&shared, &review.notice.id, 10).await.is_err());
    assert!(control.writes().is_empty());
    postflight::action(&shared, &review.notice.id, 0).await.unwrap();
    until(&shared, 20, |d| {
        d.machine.operation.is_none()
            && d.machine.feedback.as_ref().is_some_and(|f| {
                (f.position_mm[0] - 7.).abs() < 0.01 && (f.position_mm[1] - 8.).abs() < 0.01
            })
    })
    .await;
    assert_eq!(postflight::review(&shared).await.unwrap().satisfied, vec![0]);
    assert_eq!(control.view().outputs, 0);
    postflight::dismiss(&shared, &review.notice.id).await.unwrap();
    assert!(postflight::review(&shared).await.is_none());
    assert!(postflight::action(&shared, &review.notice.id, 0).await.is_err());
    assert!(machine::go_xy(&shared, [2000., 8.]).await.is_err());
    assert!(machine::go_xy(&shared, [f64::NAN, 8.]).await.is_err());
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn postflight_follows_stop_running_paused_or_settling_but_never_pause_or_frame() {
    use openlaser_server::postflight;
    let (simulator, shared) = start("postflight-stop").await;
    ready(&shared).await;
    machine::frame(&shared).await.unwrap();
    until(&shared, 15, |d| d.message.as_ref().is_some_and(|m| m.text == "Frame finished")).await;
    assert!(openlaser_server::postflight::review(&shared).await.is_none());
    let control = simulator.control();
    control.time_scale(0.2);
    machine::frame(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.program.as_ref().is_some_and(|p| p.started)).await;
    machine::stop(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.operation.is_none()).await;
    assert!(postflight::review(&shared).await.is_none(), "stopped frames do not open postflight");
    for phase in ["running", "paused", "settling"] {
        common::run(&shared).await.unwrap();
        until(&shared, 10, |d| d.machine.program.as_ref().is_some_and(|p| p.started)).await;
        if phase != "running" {
            control.fault(Fault::StopFeedbackBusy, phase == "settling");
            machine::hold(&shared).await.unwrap();
            if phase == "paused" {
                until(&shared, 10, |d| d.can_resume).await;
            }
            assert!(postflight::review(&shared).await.is_none(), "Pause does not end the job");
        }
        machine::stop(&shared).await.unwrap();
        control.fault(Fault::StopFeedbackBusy, false);
        assert!(!control.view().running);
        assert_eq!(control.view().outputs, 0);
        until(&shared, 10, |d| d.postflight.is_some() && d.machine.operation.is_none()).await;
        let doc = shared.lock().await.document();
        assert_eq!(doc.machine.program.unwrap().state, ProgramState::Stopped);
        assert!(matches!(
            doc.machine.connection,
            openlaser_controller::state::Connection::Connected { .. }
        ));
        assert!(doc.machine.session.homed);
        assert!(!doc.can_resume);
        assert!(doc.completed_sheet.is_none(), "a stopped sheet is not marked fully cut");
        assert!(shared.lock().await.held.is_none());
        let notice = postflight::review(&shared).await.unwrap().notice;
        assert_eq!(notice.name, "plate");
        postflight::dismiss(&shared, &notice.id).await.unwrap();
        machine::stop(&shared).await.unwrap();
        assert!(postflight::review(&shared).await.is_none(), "Stop is idempotent");
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn compiling_selects_the_recipe_mode_offline_and_on_a_connected_simulator() {
    let (simulator, shared) = start("automatic-job-mode").await;
    setup(&shared).await;
    let (fiber, co2) = {
        let mut c = shared.lock().await;
        let fiber = c.draft.as_ref().unwrap().current.recipe.as_ref().unwrap().id.clone();
        let co2 = c
            .add_recipe(&NewRecipe {
                name: "Plywood".into(),
                laser: LaserMode::Co2,
                thickness_mm: 3.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.set_recipe(&co2.id).unwrap();
        (fiber, co2.id)
    };
    machine::compile(&shared, false).await.unwrap();
    assert_eq!(shared.lock().await.document().mode, Some(LaserMode::Co2));
    assert!(
        shared
            .lock()
            .await
            .draft
            .as_ref()
            .unwrap()
            .compiled
            .as_ref()
            .unwrap()
            .configuration
            .is_none()
    );
    common::seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    shared.lock().await.set_recipe(&fiber).unwrap();
    machine::compile(&shared, false).await.unwrap();
    let document = shared.lock().await.document();
    assert_eq!(document.mode, Some(LaserMode::Fiber));
    assert_eq!(document.machine.session.mode, Some(LaserMode::Fiber));
    assert!(document.machine.session.homed, "a mode switch keeps the XY reference");
    assert!(
        shared
            .lock()
            .await
            .draft
            .as_ref()
            .unwrap()
            .compiled
            .as_ref()
            .unwrap()
            .configuration
            .is_some()
    );
    shared.lock().await.set_recipe(&co2).unwrap();
    machine::compile(&shared, false).await.unwrap();
    let document = shared.lock().await.document();
    assert_eq!(document.machine.session.mode, Some(LaserMode::Co2));
    assert!(document.machine.session.homed, "switching back keeps it too");
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn gas_tests_resolve_from_the_compiled_job_and_legacy_preferences_migrate() {
    let (_simulator, shared) = start("automatic-job-gas").await;
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    let gas =
        shared.lock().await.draft.as_ref().unwrap().compiled.as_ref().unwrap().job.settings.gas;
    // Even an older editor's fixed selector is replaced with the job selection.
    shared
        .lock()
        .await
        .set_preflight(JobPreflight::Custom {
            steps: vec![Check {
                text: "Gas flow".into(),
                action: Some(CheckAction::GasTest { selector: 0, pressure: 1., duration_ms: 300 }),
                auto_check: false,
            }],
        })
        .unwrap();
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert!(review.steps.iter().any(|s| matches!(s.action, Some(CheckAction::GasTest { selector, duration_ms: 300, .. }) if selector == gas)));
    assert!(review.steps.iter().all(|s| !s.auto_check));
    machine::compile(&shared, true).await.unwrap();
    assert!(preflight::review(&shared, PreflightIntent::Run).await.unwrap().steps.is_empty());
    let (root, mut value) = {
        let c = shared.lock().await;
        (c.config.data_dir.clone(), serde_json::to_value(&c.preflight).unwrap())
    };
    value.as_object_mut().unwrap().remove("postflight");
    value["version"] = 1.into();
    value["confirm_gas"] = true.into();
    value["fiber"]["steps"]
        .as_array_mut()
        .unwrap()
        .retain(|step| step["action"]["kind"] != "job_gas_test");
    std::fs::write(root.join("preflight.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    let migrated = preflight::PreflightPreferences::open(&root).unwrap();
    assert!(migrated.postflight.fiber.enabled && !migrated.postflight.fiber.steps.is_empty());
    assert_eq!(migrated.fiber.steps, shared.lock().await.preflight.fiber.steps);
    assert_eq!(
        migrated.co2.steps.iter().filter(|check| check.checks_gas()).count(),
        1,
        "an existing gas check is not duplicated"
    );
    assert_eq!(migrated.version, 2);
    assert!(!migrated.confirm_gas);
    let mut edited = migrated;
    edited.fiber.steps.retain(|check| !check.checks_gas());
    shared.lock().await.save_preflight_preferences(edited).unwrap();
    let reopened = preflight::PreflightPreferences::open(&root).unwrap();
    assert!(
        !reopened.fiber.steps.iter().any(Check::checks_gas),
        "a deleted gas step stays deleted"
    );
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn co2_preflight_tests_high_air_and_closes_the_valve() {
    let (simulator, shared) = start("co2-high-air").await;
    setup(&shared).await;
    {
        let mut c = shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Basswood".into(),
                laser: LaserMode::Co2,
                thickness_mm: 3.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        assert_eq!(recipe.gas, "High Air");
        assert_eq!(recipe.summary.gas.as_deref(), Some("3"));
        c.set_recipe(&recipe.id).unwrap();
    }
    machine::compile(&shared, false).await.unwrap();
    common::seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    let review = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(review.gases, ["High Air"]);
    let step = review.steps.iter().position(Check::checks_gas).unwrap();
    assert!(matches!(
        review.steps[step].action,
        Some(CheckAction::GasTest { selector: 3, pressure: 0., .. })
    ));
    let control = simulator.control();
    control.time_scale(1.);
    preflight::action(&shared, &review.token, step).await.unwrap();
    until(&shared, 5, |_| control.view().outputs != 0).await;
    assert_eq!(control.view().outputs, 1 << 2, "only the configured High Air valve opens");
    assert_eq!(control.view().analog, [0, 0]);
    until(&shared, 5, |d| d.machine.operation.is_none() && control.view().outputs == 0).await;
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    assert_eq!(preflight::review(&shared, PreflightIntent::Run).await.unwrap().token, review.token);
    common::run(&shared).await.unwrap();
    until(&shared, 10, |_| control.view().pwm[1][1] > 0).await;
    assert_ne!(control.view().outputs & (1 << 2), 0, "High Air stays on during the CO2 cut");
    machine::stop(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.operation.is_none() && control.view().outputs == 0).await;
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn testing_gas_keeps_the_preflight_token_and_requires_manual_confirmation() {
    let (_simulator, shared) = start("preflight-gas-refresh").await;
    ready(&shared).await;
    let before = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    let gas = before.steps.iter().position(Check::checks_gas).unwrap();
    preflight::action(&shared, &before.token, gas).await.unwrap();
    until(&shared, 10, |d| d.machine.operation.is_none()).await;
    let after = preflight::review(&shared, PreflightIntent::Run).await.unwrap();
    assert_eq!(
        before.token, after.token,
        "gas testing does not invalidate unrelated manual checks"
    );
    assert!(!after.satisfied.contains(&gas), "opening a valve does not confirm flow");
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn pause_checklist_tests_gas_without_losing_position_checks_or_cut_history() {
    let (simulator, shared) = start("pause-checklist-gas").await;
    ready(&shared).await;
    let control = simulator.control();
    control.time_scale(0.5);
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| d.progress.as_ref().is_some_and(|p| p.completed >= 1)).await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    let retained = shared.lock().await.document().recovery.clone().unwrap();
    assert!(retained.steps.iter().any(|s| !s.executed.is_empty()));
    let review = preflight::review(&shared, PreflightIntent::Resume).await.unwrap();
    assert_eq!(review.steps.len(), 3);
    let gas = review.steps.iter().position(Check::checks_gas).unwrap();
    assert_eq!(review.steps[gas].text.matches("High N₂").count(), 1);
    control.clear_writes();
    assert!(preflight::action(&shared, "stale-pause", gas).await.is_err());
    assert!(control.writes().is_empty());
    preflight::action(&shared, &review.token, gas).await.unwrap();
    until(&shared, 5, |d| d.machine.operation.is_none()).await;
    assert_eq!(control.view().outputs, 0);
    let refreshed = preflight::review(&shared, PreflightIntent::Resume).await.unwrap();
    assert_eq!(review.token, refreshed.token);
    assert!(refreshed.satisfied.is_empty());
    let after = shared.lock().await.document().recovery.clone().unwrap();
    assert_eq!(retained.pause_position, after.pause_position);
    assert_eq!(retained.selected, after.selected);
    assert_eq!(
        serde_json::to_value(&retained.steps).unwrap(),
        serde_json::to_value(&after.steps).unwrap()
    );
    let mut partial = confirmation(&shared, PreflightIntent::Resume).await;
    partial.checked.pop();
    assert!(machine::resume_reviewed(&shared, Some(&partial)).await.is_err());
    common::resume(&shared).await.unwrap();
    until(&shared, 10, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Running)
    })
    .await;
    let resumed = shared.lock().await.document().recovery.clone().unwrap();
    for (before, now) in retained.steps.iter().zip(&resumed.steps) {
        for range in &before.executed {
            assert!(now.executed.iter().any(|r| r[0] <= range[0] && r[1] >= range[1]));
        }
    }
    machine::stop(&shared).await.unwrap();
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn a_new_job_does_not_inherit_the_previous_jobs_cut_history() {
    let (simulator, shared) = start("fresh-job-history").await;
    ready(&shared).await;
    let control = simulator.control();
    control.time_scale(20.);
    common::run(&shared).await.unwrap();
    until(&shared, 20, |d| {
        d.machine.operation.is_none()
            && d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
    })
    .await;
    assert!(
        shared
            .lock()
            .await
            .document()
            .recovery
            .as_ref()
            .unwrap()
            .steps
            .iter()
            .all(|s| s.status == "completed")
    );
    control.time_scale(0.25);
    common::run(&shared).await.unwrap();
    until(&shared, 15, |d| {
        d.progress.as_ref().is_some_and(|p| p.pass == Some(0) && !p.approaching)
    })
    .await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    let recovery = shared.lock().await.document().recovery.clone().unwrap();
    assert_eq!(recovery.selected.unwrap().pass, 0);
    assert!(recovery.steps[1..].iter().all(|s| s.executed.is_empty()));
    assert!(recovery.steps[0].executed.iter().all(|range| range[1] < 1.));
    machine::stop(&shared).await.unwrap();
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn pause_defaults_migrate_and_save_independently() {
    let (_simulator, shared) = start("pause-defaults").await;
    let mut c = shared.lock().await;
    let root = c.config.data_dir.clone();
    let mut old = serde_json::to_value(&c.preflight).unwrap();
    old.as_object_mut().unwrap().remove("pause");
    std::fs::write(root.join("preflight.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    let mut preferences = preflight::PreflightPreferences::open(&root).unwrap();
    assert_eq!(preferences.pause.fiber.steps.len(), 3);
    let preflight = preferences.fiber.steps.clone();
    let postflight = preferences.postflight.fiber.steps.clone();
    preferences.pause.fiber.steps = vec![check("Custom pause check"), Check::gas()];
    preferences.pause.co2.enabled = false;
    c.save_preflight_preferences(preferences).unwrap();
    let saved = preflight::PreflightPreferences::open(&root).unwrap();
    assert_eq!(saved.pause.fiber.steps[0].text, "Custom pause check");
    assert!(!saved.pause.co2.enabled);
    assert_eq!(saved.fiber.steps, preflight);
    assert_eq!(saved.postflight.fiber.steps, postflight);
    let mut invalid = saved;
    invalid.pause.fiber.steps[0].action = Some(CheckAction::Home {});
    assert!(c.save_preflight_preferences(invalid).is_err());
    drop(c);
    openlaser_server::shutdown(&shared).await.unwrap();
}
