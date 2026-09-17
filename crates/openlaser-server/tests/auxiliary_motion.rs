// SPDX-License-Identifier: GPL-3.0-or-later

//! Auxiliary motion and reference search through the loopback controller.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture acceptance tests")]

mod common;

use common::{start, until};
use openlaser_controller::lease::Lease;
use openlaser_controller::simulator::{AlarmBank, Control, Fault};
use openlaser_server::coordinator::Shared;
use openlaser_server::machine::{JogRequest, OutputRequest, TableRequest};
use openlaser_server::{connect, machine};
use std::time::Duration;

fn lease(sequence: u64) -> Lease {
    Lease { client: "axis-acceptance".into(), sequence }
}

async fn settled(shared: &Shared) {
    until(shared, 10, |d| {
        d.machine.operation.is_none()
            && d.machine.feedback.as_ref().is_some_and(|f| f.stationary && f.table_stationary)
    })
    .await;
}

async fn lower_fixture_head(shared: &Shared, control: &Control) -> f64 {
    // Keep direction/release checks inside the stroke: even a whole
    // unrenewed jog lease must not reach the physical upper switch.
    shared
        .machine
        .outputs(openlaser_controller::operations::outputs::Plan {
            name: "position fixture head".into(),
            on: vec![openlaser_protocol::sequences::Step::Write(
                openlaser_protocol::requests::head_move(1000, 20_000),
            )],
            off: vec![openlaser_protocol::requests::head_cancel()],
            lease: Duration::from_secs(2),
            duration: Some(Duration::from_millis(300)),
        })
        .await
        .unwrap();
    control.view().position_mm[3]
}

fn table_backup() -> String {
    let xml = std::fs::read_to_string(common::fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("<PLaserParam>", "<PLaserParam><LPF LiftingPlatformType='1'/>")
        .replace(
            "<MP LaserDAKeepOutput",
            "<MP ExchangePlatformType='0' RollSheetType='0' LaserDAKeepOutput",
        )
        .replace("</ParameterRoot>", "<PHomeParam><HPA3 Acc='1000'/></PHomeParam></ParameterRoot>");
    let doc = openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, xml.as_bytes())
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "SoftLimitMaxLen", "4")
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "GoOriginalDirection", "0")
        .unwrap();
    String::from_utf8(doc.original().to_vec()).unwrap()
}

/// Connection replaces stale axis settings and verifies them before an
/// unreferenced manual jog uses the newly configured limits.
#[tokio::test]
async fn connection_initializes_mismatched_parameters_then_allows_jogging() {
    let (simulator, shared) = start("mismatched-jog").await;
    let control = simulator.control();
    let backup =
        openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, table_backup().as_bytes())
            .unwrap()
            .with_attribute("/ParameterRoot/PMachineAxisConfig_0/MAC", "SoftLimitMaxLen", "1.5")
            .unwrap();
    machine::import_file(&shared, "older-backup.xml", backup.original()).await.unwrap();
    let mut old = control.parameters();
    old[0][9] = 62_080;
    old[0][10] = 0;
    old[4][2] = 2000;
    control.set_parameters(old);
    connect::connect(&shared).await.unwrap();
    let document = shared.lock().await.document();
    assert!(document.machine.session.parameters_verified);
    assert!(!document.machine.session.homed);
    assert!(document.readiness.jog.ok, "{:?}", document.readiness.jog);
    let banks = control.parameters();
    assert_eq!(banks[0][9], 31_040);
    assert_eq!(banks[0][10], 8000);
    assert_eq!(banks[0][2], 1500);
    assert_eq!(banks[4][2], 4000);
    let writes = control.writes();
    assert_eq!(writes.iter().filter(|w| w.words.len() == 14).count(), 5);
    assert!(writes.iter().any(|w| w.address == 100 && w.words == [9999]));
    assert_eq!(control.view().outputs, 0);
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    machine::jog(
        &shared,
        JogRequest { axis: 0, positive: true, step_mm: Some(10.), fast: false },
        None,
    )
    .await
    .unwrap();
    until(&shared, 5, |d| d.machine.operation.is_none() && control.view().position_mm[0] > 1.49)
        .await;
    assert!(control.view().position_mm[0] <= 1.5);
    assert_eq!(control.parameters(), banks);
    machine::home(&shared).await.unwrap();
    until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    assert!(shared.lock().await.document().readiness.position.ok);
    openlaser_server::shutdown(&shared).await.unwrap();
}

/// Losing valid controller parameters after initialization invalidates
/// the connection before any further manual move can be admitted.
#[tokio::test]
async fn unreadable_live_travel_or_resolution_still_refuses_jogging() {
    for missing_resolution in [true, false] {
        let (simulator, shared) = start("invalid-live-jog").await;
        let control = simulator.control();
        connect::connect(&shared).await.unwrap();
        let mut banks = control.parameters();
        if missing_resolution {
            banks[0][10] = 0;
        } else {
            banks[0][2] = banks[0][1];
        }
        control.clear_writes();
        control.set_parameters(banks);
        until(&shared, 5, |d| !d.machine.session.parameters_verified).await;
        assert!(
            machine::jog(
                &shared,
                JogRequest { axis: 0, positive: true, step_mm: Some(1.), fast: false },
                None
            )
            .await
            .is_err()
        );
        assert!(
            !control.writes().iter().any(|w| w.address == 101 && w.words.first() == Some(&3)),
            "invalid live parameters emitted a jog"
        );
        assert_eq!(control.parameters(), banks);
        openlaser_server::shutdown(&shared).await.unwrap();
    }
}

/// Missing-reference notices must not disable the actions that establish a
/// reference, or the manual moves an operator uses to position the machine.
#[tokio::test]
async fn an_unreferenced_machine_can_jog_xy_z_and_w_then_home() {
    let (simulator, shared) = start("unreferenced-positioning").await;
    let control = simulator.control();
    machine::import_file(&shared, "table-test.xml", table_backup().as_bytes()).await.unwrap();
    common::seed(&shared, &simulator).await;
    control.head_reference(false);
    control.alarm_word(AlarmBank::Head, 1 << 12);
    control.alarm_word(AlarmBank::Controller1, 1 << 24);
    connect::connect(&shared).await.unwrap();
    let parameters = control.parameters();
    let before = shared.lock().await.document();
    assert!(!before.machine.session.homed);
    assert!(!before.machine.feedback.as_ref().unwrap().head.referenced);
    assert!(before.readiness.home.ok, "{:?}", before.readiness.home);
    assert!(before.readiness.jog.ok, "{:?}", before.readiness.jog);
    assert!(!before.readiness.outputs.ok, "the concession must not enable laser outputs");
    assert!(!before.readiness.run.ok && !before.readiness.frame.ok);
    for axis in 0..2 {
        machine::jog(
            &shared,
            JogRequest { axis, positive: true, step_mm: Some(2.), fast: false },
            None,
        )
        .await
        .unwrap();
        until(&shared, 5, |d| {
            d.machine.operation.is_none()
                && d.machine
                    .feedback
                    .as_ref()
                    .is_some_and(|f| f.stationary && (f.position_mm[axis] - 2.).abs() < 0.01)
        })
        .await;
    }
    machine::outputs(&shared, OutputRequest::HeadJog { up: false, fast: false }, lease(101))
        .await
        .unwrap();
    until(&shared, 5, |_| control.view().position_mm[3] > 0.5).await;
    machine::release(&shared, lease(101)).await.unwrap();
    settled(&shared).await;
    machine::table(&shared, TableRequest { positive: true, speed_mm_s: 10. }, lease(102))
        .await
        .unwrap();
    until(&shared, 5, |_| control.view().position_mm[4] > 0.5).await;
    machine::release(&shared, lease(102)).await.unwrap();
    settled(&shared).await;
    let positioned = control.view();
    assert!(!shared.lock().await.document().machine.session.homed);
    assert_eq!(positioned.referenced[..2], [false, false]);
    assert!(!shared.lock().await.document().machine.feedback.as_ref().unwrap().head.referenced);
    assert_eq!(control.parameters(), parameters, "jogging changed the configured limits");
    assert_eq!(
        (positioned.outputs, positioned.extended_outputs, positioned.analog),
        (0, 0, [0, 0])
    );
    assert!(positioned.pwm.iter().all(|p| p[1] == 0));
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    let homed = control.view();
    assert!(homed.referenced[0] && homed.referenced[1] && homed.referenced[3]);
    assert_eq!(control.parameters(), parameters, "homing changed the configured limits");
    assert!((homed.position_mm[4] - positioned.position_mm[4]).abs() < 0.01);
    openlaser_server::shutdown(&shared).await.unwrap();
}

/// Unrelated head/controller faults still prevent unreferenced motion.
#[tokio::test]
async fn unrelated_faults_still_block_manual_positioning_and_z_recovery() {
    let (simulator, shared) = start("unreferenced-faults").await;
    let control = simulator.control();
    control.head_reference(false);
    connect::connect(&shared).await.unwrap();
    for (index, (bank, word)) in
        [(AlarmBank::Head, 1 << 8), (AlarmBank::Controller1, 1 << 30), (AlarmBank::Controller2, 1)]
            .into_iter()
            .enumerate()
    {
        control.alarm_word(bank, word);
        until(&shared, 5, |d| !d.readiness.home.ok && !d.readiness.jog.ok).await;
        control.clear_writes();
        let previous = shared.lock().await.document().message.map_or(0, |m| m.id);
        machine::home(&shared).await.unwrap();
        until(&shared, 5, |d| d.message.as_ref().is_some_and(|m| m.error && m.id > previous)).await;
        assert!(control.writes().is_empty(), "a faulted home emitted motion");
        let refused = machine::jog(
            &shared,
            JogRequest { axis: 0, positive: true, step_mm: Some(1.), fast: false },
            None,
        )
        .await
        .unwrap_err();
        assert!(refused.to_string().contains("alarms are active"));
        assert!(
            machine::outputs(
                &shared,
                OutputRequest::HeadJog { up: false, fast: false },
                lease(103 + index as u64),
            )
            .await
            .is_err()
        );
        assert!(control.writes().is_empty(), "faulted positioning emitted motion");
        control.alarm_word(bank, 0);
        until(&shared, 5, |d| d.readiness.home.ok && d.readiness.jog.ok).await;
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn homing_sequences_head_before_xy_and_auxiliary_axes_remain_independent() {
    let (simulator, shared) = start("homing-auxiliary").await;
    let control = simulator.control();
    machine::import_file(&shared, "auxiliary-test.xml", table_backup().as_bytes()).await.unwrap();
    common::seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    for axis in 0..2 {
        machine::jog(
            &shared,
            JogRequest { axis, positive: true, step_mm: Some(5.), fast: false },
            None,
        )
        .await
        .unwrap();
        settled(&shared).await;
    }
    let before = control.view().position_mm;
    assert!((before[0] - 5.).abs() < 0.01 && (before[1] - 5.).abs() < 0.01);
    machine::table(&shared, TableRequest { positive: true, speed_mm_s: 10. }, lease(1))
        .await
        .unwrap();
    until(&shared, 5, |d| d.machine.feedback.as_ref().is_some_and(|f| f.table_mm > 0.5)).await;
    machine::release(&shared, lease(1)).await.unwrap();
    settled(&shared).await;
    let released = control.view().position_mm;
    assert!(released[4] > 0.5 && released[4] < 4.);
    assert_eq!(&released[..4], &before[..4], "W moved another axis");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        control.view().position_mm.iter().zip(released).all(|(a, b)| (a - b).abs() < 1e-9),
        "released W kept moving"
    );
    for (sequence, positive, target) in [(2, false, 0.), (3, true, 4.)] {
        machine::table(&shared, TableRequest { positive, speed_mm_s: 100. }, lease(sequence))
            .await
            .unwrap();
        settled(&shared).await;
        assert!((control.view().position_mm[4] - target).abs() < 0.01);
        assert!(
            machine::table(
                &shared,
                TableRequest { positive, speed_mm_s: 100. },
                lease(sequence + 10)
            )
            .await
            .is_err(),
            "table crossed its configured limit"
        );
    }
    let table = control.view().position_mm[4];
    let start_z = lower_fixture_head(&shared, &control).await;
    machine::outputs(&shared, OutputRequest::HeadJog { up: false, fast: false }, lease(20))
        .await
        .unwrap();
    until(&shared, 5, |d| {
        d.machine.feedback.as_ref().is_some_and(|f| f.position_mm[2] > start_z + 0.5)
    })
    .await;
    machine::release(&shared, lease(20)).await.unwrap();
    settled(&shared).await;
    let down = control.view().position_mm;
    assert!(down[3] > start_z + 0.5 && (down[4] - table).abs() < 0.01, "Z changed W");
    assert_eq!(&down[..2], &before[..2], "Z changed XY");
    machine::outputs(&shared, OutputRequest::HeadJog { up: true, fast: false }, lease(21))
        .await
        .unwrap();
    until(&shared, 5, |_| control.view().position_mm[3] < down[3] - 0.5).await;
    machine::release(&shared, lease(21)).await.unwrap();
    settled(&shared).await;

    // A stalled head must prevent XY homing and never grant a reference.
    control.head_reference(false);
    control.axis_reference(0, false);
    control.axis_reference(1, false);
    control.fault(Fault::HeadStall, true);
    machine::home(&shared).await.unwrap();
    until(&shared, 5, |_| control.view().head_command == 102).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!shared.lock().await.document().machine.session.homed);
    assert_eq!(&control.view().position_mm[..2], &before[..2]);
    machine::stop(&shared).await.unwrap();
    settled(&shared).await;
    assert!(
        !shared.lock().await.document().machine.session.homed,
        "cancelled home granted a reference"
    );
    control.fault(Fault::HeadStall, false);
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    let homed = control.view();
    assert!(homed.position_mm[..2].iter().all(|v| v.abs() < 0.01));
    assert!(homed.position_mm[3].abs() < 0.01);
    assert!((homed.position_mm[4] - table).abs() < 0.01, "home unexpectedly moved W");
    assert!(homed.referenced[0] && homed.referenced[1] && homed.referenced[3]);
    assert_eq!((homed.outputs, homed.extended_outputs, homed.analog), (0, 0, [0, 0]));
    assert!(homed.pwm.iter().all(|p| p[1] == 0));
    assert!(homed.last_error.is_none(), "{:?}", homed.last_error);
    println!(
        "AXIS_ACCEPTANCE head-before-XY; stalled/cancelled home refused; XY home; Z up/down/release; W up/down/release/limits; axes independent; outputs off"
    );
    openlaser_server::shutdown(&shared).await.unwrap();
}
