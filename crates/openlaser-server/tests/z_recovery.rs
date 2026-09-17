// SPDX-License-Identifier: GPL-3.0-or-later
//! The directional Z controls recover a limit already active at connection.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known simulator fixtures")]
mod common;

use common::{config, seed, until};
use openlaser_controller::Simulator;
use openlaser_controller::lease::Lease;
use openlaser_controller::simulator::Fault;
use openlaser_protocol::requests;
use openlaser_server::{Coordinator, connect, machine};
use std::time::Duration;

fn lease(sequence: u64) -> Lease {
    Lease { client: "z-recovery".into(), sequence }
}

#[tokio::test]
async fn a_limit_at_connection_keeps_only_the_away_direction_available() {
    let simulator = Simulator::start().await.unwrap();
    let control = simulator.control();
    control.head_on_limit(true);
    let shared = Coordinator::start(config("z-recovery", simulator.config())).unwrap();
    seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    until(&shared, 5, |d| d.readiness.head_down.ok).await;
    {
        let c = shared.lock().await;
        let d = c.document();
        assert!(d.readiness.head_recovery);
        assert!(!d.readiness.head_up.ok);
        assert!(!d.readiness.home.ok);
        assert!(!d.readiness.jog.ok);
        assert!(!d.readiness.outputs.ok);
        assert!(!d.readiness.calibrate.ok);
        assert!(!d.readiness.run.ok);
    }
    control.clear_writes();
    assert!(
        machine::outputs(
            &shared,
            machine::OutputRequest::HeadJog { up: true, fast: true },
            lease(1)
        )
        .await
        .is_err()
    );
    assert!(control.writes().is_empty());
    machine::outputs(&shared, machine::OutputRequest::HeadJog { up: false, fast: true }, lease(2))
        .await
        .unwrap();
    for _ in 0..4 {
        machine::heartbeat(&shared, lease(2)).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    machine::release(&shared, lease(2)).await.unwrap();
    until(&shared, 5, |d| d.machine.operation.is_none() && d.readiness.home.ok).await;
    assert!(control.writes().contains(&requests::head_move(10, 1000)));
    assert!((0.1..=1.).contains(&control.view().position_mm[3]));
    assert!(!control.view().head_referenced);
    assert_eq!(control.view().outputs, 0);
    machine::home(&shared).await.unwrap();
    until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    assert!(control.view().head_referenced);
    assert!(
        shared.lock().await.acceptance().is_ok(),
        "Home must finish interrupted initialization"
    );
    assert!(shared.lock().await.document().readiness.jog.ok);

    // A stuck lower switch permits one bounded up press, then remains an
    // alarm. Even continuous renewals cannot prolong the two-second press.
    control.head_on_limit(false);
    control.fault(Fault::HeadLowerLimit, true);
    until(&shared, 5, |d| d.readiness.head_up.ok && !d.readiness.head_down.ok).await;
    control.clear_writes();
    machine::outputs(&shared, machine::OutputRequest::HeadJog { up: true, fast: true }, lease(3))
        .await
        .unwrap();
    for _ in 0..30 {
        machine::heartbeat(&shared, lease(3)).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    until(&shared, 5, |d| d.machine.operation.is_none()).await;
    assert!(control.writes().contains(&requests::head_move(10, -1000)));
    assert!(control.writes().contains(&requests::head_cancel()));
    assert!((119. ..=119.01).contains(&control.view().position_mm[3]));
    assert_ne!(control.view().alarms[2] & 2, 0);
    control.fault(Fault::EmergencyStop, true);
    until(&shared, 5, |d| !d.readiness.head_up.ok && !d.readiness.head_down.ok).await;
    control.clear_writes();
    assert!(
        machine::outputs(
            &shared,
            machine::OutputRequest::HeadJog { up: true, fast: false },
            lease(4)
        )
        .await
        .is_err()
    );
    assert!(control.writes().is_empty());
    openlaser_server::shutdown(&shared).await.unwrap();
}
