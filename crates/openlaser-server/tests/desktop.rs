// SPDX-License-Identifier: GPL-3.0-or-later

//! The desktop window's close signal uses the admitted server cleanup path.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "isolated simulator assertions")]

mod common;

use openlaser_controller::lease::Lease;
use openlaser_server::{connect, machine};
use std::time::Duration;

#[tokio::test]
async fn window_ready_reports_its_listener_and_close_clears_active_outputs() {
    let (simulator, shared) = common::start("desktop-close").await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    common::until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let (ready, address) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(openlaser_server::serve_until_ready(
        shared.clone(),
        "127.0.0.1:0".parse().unwrap(),
        async {
            let _ = stopped.await;
        },
        |bound| {
            ready.send(bound).unwrap();
        },
    ));
    let address = tokio::time::timeout(Duration::from_secs(5), address).await.unwrap().unwrap();
    assert!(address.ip().is_loopback() && address.port() != 0);
    drop(tokio::net::TcpStream::connect(address).await.unwrap());
    machine::outputs(
        &shared,
        machine::OutputRequest::Pointer,
        Lease { client: "desktop-close".to_owned(), sequence: 1 },
    )
    .await
    .unwrap();
    let control = simulator.control();
    tokio::time::timeout(Duration::from_secs(2), async {
        while control.view().outputs == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(control.view().outputs, 0);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), serving).await.unwrap().unwrap().unwrap();
    let view = control.view();
    assert_eq!(view.outputs, 0);
    assert!(!view.running);
    assert!(
        machine::outputs(
            &shared,
            machine::OutputRequest::Pointer,
            Lease { client: "desktop-close".to_owned(), sequence: 2 },
        )
        .await
        .is_err()
    );
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}
