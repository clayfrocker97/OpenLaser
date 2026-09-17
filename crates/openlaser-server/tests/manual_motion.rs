// SPDX-License-Identifier: GPL-3.0-or-later
//! Manual motion remains available before the cutting interlocks are ready.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known loopback fixtures")]
mod common;

use common::{config, seed, until};
use openlaser_controller::Simulator;
use openlaser_controller::lease::Lease;
use openlaser_controller::simulator::{Control, Fault};
use openlaser_server::coordinator::Shared;
use openlaser_server::machine::{JogRequest, OutputRequest};
use openlaser_server::{Coordinator, connect, machine};
use std::time::Duration;

fn lease(sequence: u64) -> Lease {
    Lease { client: "manual-recovery".into(), sequence }
}

fn short_stroke(name: &str, simulator: &Simulator) -> openlaser_server::Config {
    // A 10 mm test bed keeps the actual Home motion inside the test deadline.
    let mut settings = config(name, simulator.config());
    let xml = std::fs::read_to_string(common::fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("SoftLimitMaxLen='1371'", "SoftLimitMaxLen='10'")
        .replace("SoftLimitMaxLen='971'", "SoftLimitMaxLen='10'");
    let backup = settings.data_dir.join("short-stroke.xml");
    std::fs::write(&backup, xml).unwrap();
    settings.files.backup = Some(backup);
    settings
}

async fn press(shared: &Shared, axis: usize, positive: bool, sequence: u64) {
    machine::jog(
        shared,
        JogRequest { axis, positive, step_mm: None, fast: true },
        Some(lease(sequence)),
    )
    .await
    .unwrap();
    for _ in 0..4 {
        machine::heartbeat(shared, lease(sequence)).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    machine::release(shared, lease(sequence)).await.unwrap();
    until(shared, 5, |d| d.machine.operation.is_none()).await;
}

fn assert_recovery_write(control: &Control, axis: usize, positive: bool) {
    let writes = control.writes();
    let jogs: Vec<_> =
        writes.iter().filter(|w| w.address == 101 && w.words.first() == Some(&3)).collect();
    assert_eq!(jogs.len(), 1, "{writes:?}");
    let words = &jogs[0].words;
    assert_eq!(words[1], u32::try_from(axis).unwrap());
    assert!(words[2] > 0 && words[2] <= 1000);
    assert_eq!(words[5].cast_signed(), if positive { 1000 } else { -1000 });
    let view = control.view();
    assert_eq!((view.outputs, view.extended_outputs), (0, 0));
    assert!(view.pwm.iter().all(|p| p[1] == 0));
}

#[tokio::test]
async fn every_xy_limit_at_connection_offers_only_the_away_direction_then_home() {
    for axis in 0..2 {
        for positive in [false, true] {
            for software in [false, true] {
                let simulator = Simulator::start().await.unwrap();
                let shared = Coordinator::start(short_stroke("xy-limit", &simulator)).unwrap();
                seed(&shared, &simulator).await;
                let control = simulator.control();
                control.axis_on_limit(axis, positive, software);
                let start = control.view().position_mm[axis];
                connect::connect(&shared).await.unwrap();
                until(&shared, 5, |d| d.readiness.xy_jog[axis][usize::from(!positive)].ok).await;
                let d = shared.lock().await.document();
                assert!(d.readiness.xy_recovery);
                assert!(!d.readiness.xy_jog[axis][usize::from(positive)].ok);
                assert!(d.readiness.xy_jog[1 - axis].iter().all(|gate| !gate.ok));
                assert!(!d.readiness.home.ok && !d.readiness.position.ok && !d.readiness.run.ok);
                assert!(
                    !d.machine.session.parameters_verified,
                    "this exercises interrupted initialization"
                );
                control.clear_writes();
                assert!(
                    machine::jog(
                        &shared,
                        JogRequest { axis, positive, step_mm: None, fast: true },
                        Some(lease(1))
                    )
                    .await
                    .is_err()
                );
                assert!(control.writes().is_empty());
                press(&shared, axis, !positive, 2).await;
                assert_recovery_write(&control, axis, !positive);
                assert!((0.1..=1.).contains(&(control.view().position_mm[axis] - start).abs()));
                until(&shared, 5, |d| d.readiness.home.ok && !d.readiness.xy_recovery).await;
                machine::home(&shared).await.unwrap();
                until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none())
                    .await;
                assert!(shared.lock().await.acceptance().is_ok());
                until(&shared, 5, |d| d.readiness.xy_jog.iter().flatten().all(|g| g.ok)).await;
                openlaser_server::shutdown(&shared).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn two_corner_limits_can_be_recovered_one_axis_at_a_time() {
    let simulator = Simulator::start().await.unwrap();
    let shared = Coordinator::start(short_stroke("xy-corner", &simulator)).unwrap();
    seed(&shared, &simulator).await;
    let control = simulator.control();
    control.axis_on_limit(0, true, false);
    control.axis_on_limit(1, false, false);
    connect::connect(&shared).await.unwrap();
    until(&shared, 5, |d| d.readiness.xy_jog[0][0].ok && d.readiness.xy_jog[1][1].ok).await;
    press(&shared, 0, false, 1).await;
    until(&shared, 5, |d| d.readiness.xy_jog[1][1].ok).await;
    assert!(shared.lock().await.document().readiness.xy_jog[0].iter().all(|g| !g.ok));
    press(&shared, 1, true, 2).await;
    until(&shared, 5, |d| !d.readiness.xy_recovery && d.readiness.home.ok).await;
    machine::home(&shared).await.unwrap();
    until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn a_stuck_xy_limit_stays_bounded_and_an_emergency_stop_still_blocks() {
    let (simulator, shared) = common::start("xy-stuck").await;
    let control = simulator.control();
    connect::connect(&shared).await.unwrap();
    control.axis_on_limit(0, true, false);
    control.axis_alarm(0, 1);
    until(&shared, 5, |d| d.readiness.xy_recovery && d.readiness.xy_jog[0][0].ok).await;
    let start = control.view().position_mm[0];
    control.clear_writes();
    machine::jog(
        &shared,
        JogRequest { axis: 0, positive: false, step_mm: None, fast: true },
        Some(lease(1)),
    )
    .await
    .unwrap();
    for _ in 0..30 {
        machine::heartbeat(&shared, lease(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    until(&shared, 5, |d| d.machine.operation.is_none()).await;
    assert!((start - control.view().position_mm[0] - 1.).abs() < 0.01);
    assert_recovery_write(&control, 0, false);
    assert!(shared.lock().await.document().readiness.xy_recovery);
    control.fault(Fault::EmergencyStop, true);
    until(&shared, 5, |d| !d.readiness.xy_jog[0][0].ok).await;
    control.clear_writes();
    assert!(
        machine::jog(
            &shared,
            JogRequest { axis: 0, positive: false, step_mm: Some(1.), fast: false },
            None
        )
        .await
        .is_err()
    );
    assert!(control.writes().is_empty());
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn cooling_and_source_alarms_allow_setup_and_axes_but_keep_cutting_closed() {
    let simulator = Simulator::start().await.unwrap();
    let mut settings = config("process-motion", simulator.config());
    let xml = std::fs::read_to_string(common::fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("LaserWarning='0' LaserWarningType='0'", "LaserWarning='5' LaserWarningType='1'");
    let backup = settings.data_dir.join("process-alarms.xml");
    std::fs::write(&backup, xml).unwrap();
    settings.files.backup = Some(backup);
    let shared = Coordinator::start(settings).unwrap();
    seed(&shared, &simulator).await;
    let control = simulator.control();
    control.input(3, Some(false));
    control.input(5, Some(false));
    connect::connect(&shared).await.unwrap();
    until(&shared, 5, |d| {
        d.readiness.home.ok
            && d.readiness.jog.ok
            && d.readiness.head_up.ok
            && d.readiness.head_down.ok
    })
    .await;
    {
        let d = shared.lock().await.document();
        assert!(
            d.machine.alarms.iter().any(|row| row.id == Some(56) && row.active && row.blocking)
        );
        assert!(
            d.machine.alarms.iter().any(|row| row.id == Some(60) && row.active && row.blocking)
        );
        assert!(d.readiness.head_up.ok && d.readiness.head_down.ok);
        assert!(d.readiness.mode.ok);
        assert!(!d.readiness.outputs.ok && !d.readiness.run.ok && !d.readiness.frame.ok);
    }
    for axis in 0..2 {
        machine::jog(
            &shared,
            JogRequest { axis, positive: true, step_mm: Some(1.), fast: false },
            None,
        )
        .await
        .unwrap();
        until(&shared, 5, |d| {
            d.machine.operation.is_none() && control.view().position_mm[axis] > 0.99
        })
        .await;
    }
    machine::outputs(&shared, OutputRequest::HeadJog { up: false, fast: false }, lease(3))
        .await
        .unwrap();
    until(&shared, 5, |_| control.view().position_mm[3] > 0.1).await;
    machine::release(&shared, lease(3)).await.unwrap();
    until(&shared, 5, |d| d.machine.operation.is_none()).await;
    machine::home(&shared).await.unwrap();
    until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    until(&shared, 5, |d| d.readiness.calibrate.ok).await;
    assert!(shared.lock().await.document().readiness.position.ok);
    assert!(shared.lock().await.acceptance().is_ok());
    assert!(!shared.lock().await.document().readiness.run.ok);
    control.clear_writes();
    assert!(machine::pulse(&shared, 100, 10).await.is_err());
    assert!(control.writes().is_empty());
    control.input(4, Some(false));
    until(&shared, 5, |d| !d.readiness.jog.ok && !d.readiness.head_up.ok && !d.readiness.home.ok)
        .await;
    assert!(
        machine::jog(
            &shared,
            JogRequest { axis: 0, positive: true, step_mm: Some(1.), fast: false },
            None
        )
        .await
        .is_err()
    );
    assert!(control.writes().is_empty());
    openlaser_server::shutdown(&shared).await.unwrap();
}
