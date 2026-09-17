// SPDX-License-Identifier: GPL-3.0-or-later

//! Normal OS termination runs the real server lifecycle against loopback.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests drive an isolated child and simulator"
)]

mod common;

use common::{config, fixture, scratch, start, until};
use openlaser_controller::lease::Lease;
use openlaser_core::LaserMode;
use openlaser_protocol::sequences;
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::{Coordinator, connect, machine};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sigterm_finishes_cleanup_when_idle_holding_an_output_or_uploading() {
    for case in ["idle", "output", "upload", "lost-ack"] {
        let (simulator, parent) = start("shutdown-parent").await;
        let control = simulator.control();
        let bundle = parent.lock().await.bundle.clone().unwrap();
        let bound = openlaser_xml::bindings::bind(&bundle, LaserMode::Fiber, 1000).unwrap();
        let expected =
            sequences::abort(&openlaser_server::bindings::controller(&bound).shutdown, true)
                .unwrap();
        let dir = scratch(case);
        let log = std::fs::File::create(dir.join("child.log")).unwrap();
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "shutdown_child", "--nocapture"])
                .env("OPENLASER_SHUTDOWN_ENDPOINT", simulator.config().endpoint.to_string())
                .env("OPENLASER_SHUTDOWN_CASE", case)
                .env("OPENLASER_SHUTDOWN_DIR", &dir)
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        );
        wait_file(&dir.join("prepared")).await;
        control.clear_writes();
        if case == "upload" {
            control.reply_delay(Duration::from_millis(50));
        }
        std::fs::write(dir.join("proceed"), b"").unwrap();
        wait_file(&dir.join("ready")).await;
        if case == "output" || case == "upload" {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let started = if case == "upload" {
                        control.writes().iter().any(|write| write.address == 102)
                    } else {
                        control.view().outputs != 0
                    };
                    if started {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        if case == "lost-ack" {
            control.drop_next_ack();
        }
        assert!(
            Command::new("kill")
                .args(["-TERM", &child.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let status = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let log = std::fs::read_to_string(dir.join("child.log")).unwrap();
        assert_eq!(status.success(), case != "lost-ack", "{case}: {log}");
        if case == "lost-ack" {
            assert!(log.contains("cleanup is uncertain"), "{log}");
        }
        let writes = control.writes();
        assert!(writes.ends_with(&expected), "{case}: {writes:?}");
        let view = control.view();
        assert!(!view.running, "{case}");
        assert_eq!(&view.pwm[0][1..], &[0; 2], "{case}");
        assert_eq!(&view.pwm[1][1..], &[0; 2], "{case}");
        openlaser_server::shutdown(&parent).await.unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Invoked only by the parent test; ordinary test discovery does no work.
#[test]
fn shutdown_child() {
    let Ok(endpoint) = std::env::var("OPENLASER_SHUTDOWN_ENDPOINT") else { return };
    let dir = std::path::PathBuf::from(std::env::var("OPENLASER_SHUTDOWN_DIR").unwrap());
    let case = std::env::var("OPENLASER_SHUTDOWN_CASE").unwrap();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let shared = Coordinator::start(config(
                "shutdown-child",
                openlaser_controller::Config::loopback(endpoint.parse().unwrap()),
            ))
            .unwrap();
            connect::connect(&shared).await.unwrap();
            if case == "upload" {
                let mut c = shared.lock().await;
                let part = c
                    .import_part(
                        "plate.dxf",
                        &std::fs::read(fixture("dxf/plate-with-holes.dxf")).unwrap(),
                    )
                    .unwrap();
                let recipe = c
                    .add_recipe(&NewRecipe {
                        name: "Steel".into(),
                        laser: LaserMode::Fiber,
                        thickness_mm: 1.,
                        values: Values::Bank(1),
                        gas: None,
                    })
                    .unwrap();
                c.open_part(&part.id).unwrap();
                c.set_recipe(&recipe.id).unwrap();
                drop(c);
                machine::prepare(&shared).await.unwrap();
                shared.lock().await.set_origin([100., 100.]).unwrap();
                machine::compile(&shared, false).await.unwrap();
            }
            if case == "upload" || case == "output" {
                machine::home(&shared).await.unwrap();
                until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none())
                    .await;
            }
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            let serving = tokio::spawn(openlaser_server::serve(shared.clone(), address));
            tokio::time::timeout(Duration::from_secs(5), async {
                while tokio::net::TcpStream::connect(address).await.is_err() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            std::fs::write(dir.join("prepared"), b"").unwrap();
            wait_file(&dir.join("proceed")).await;
            match case.as_str() {
                "output" => machine::outputs(
                    &shared,
                    machine::OutputRequest::Pointer,
                    Lease { client: "shutdown-test".into(), sequence: 1 },
                )
                .await
                .unwrap(),
                "upload" => common::run(&shared).await.unwrap(),
                _ => {}
            }
            std::fs::write(dir.join("ready"), b"").unwrap();
            serving.await.unwrap().unwrap();
        });
}
