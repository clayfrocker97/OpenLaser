// SPDX-License-Identifier: GPL-3.0-or-later

//! Competing work is accepted by input identity, never completion order.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests use known fixtures")]

mod common;

use common::{fixture, start};
use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_server::coordinator::{CompileInputs, NewRecipe, Shared, Values};
use openlaser_server::{draft, machine};
use std::sync::Arc;

async fn setup(shared: &Shared) {
    let mut c = shared.lock().await;
    let part = c
        .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
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
    machine::prepare(shared).await.unwrap();
}

fn compile(input: &CompileInputs) -> draft::CompiledJob {
    draft::compile(
        &input.prepared,
        &input.settings,
        input.film.as_ref(),
        &input.layered,
        false,
        input.scale,
    )
    .unwrap()
}

#[tokio::test]
async fn a_compile_cannot_replace_newer_work_or_changed_inputs() {
    let (_simulator, shared) = start("revisions").await;
    setup(&shared).await;
    let (older, newer) = {
        let mut c = shared.lock().await;
        (c.compile_inputs(false).unwrap(), c.compile_inputs(false).unwrap())
    };
    let old_result = compile(&older);
    let new_result = compile(&newer);
    let mut c = shared.lock().await;
    c.compiled(newer.stamp, Ok(new_result)).unwrap();
    let accepted = c.document().draft.unwrap().compiled.clone().unwrap();
    assert!(
        c.compiled(older.stamp, Ok(old_result)).unwrap_err().to_string().contains("superseded")
    );
    assert!(Arc::ptr_eq(&accepted, c.document().draft.unwrap().compiled.as_ref().unwrap()));
    let stale = c.compile_inputs(false).unwrap();
    c.set_features(Features::default()).unwrap();
    assert!(c.compiled(stale.stamp, Ok(compile(&stale))).is_err());
    assert!(c.document().draft.unwrap().compiled.is_none());
    drop(c);
    machine::prepare(&shared).await.unwrap();
    let stale = shared.lock().await.compile_inputs(false).unwrap();
    let backup = std::fs::read(fixture("xml/harness-backup.xml")).unwrap();
    machine::import_file(&shared, "replacement.xml", &backup).await.unwrap();
    assert!(shared.lock().await.compiled(stale.stamp, Ok(compile(&stale))).is_err());
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn machine_ticks_share_views_and_deleting_the_open_part_publishes_its_clear() {
    let (_simulator, shared) = start("shared-views").await;
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    let mut c = shared.lock().await;
    let before = c.document();
    for _ in 0..100 {
        c.publish();
        let tick = c.document();
        assert!(Arc::ptr_eq(before.draft.as_ref().unwrap(), tick.draft.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&before.library, &tick.library));
        assert!(Arc::ptr_eq(&before.files, &tick.files));
    }
    let mut subscriber = c.subscribe();
    subscriber.borrow_and_update();
    c.note("first failure", true);
    let first = c.document().message.unwrap();
    c.note("second failure", true);
    let second = c.document().message.unwrap();
    assert_ne!(first.id, second.id);
    c.remove_part(&before.draft.as_ref().unwrap().parts[0].id).unwrap();
    assert!(subscriber.has_changed().unwrap());
    let deleted = subscriber.borrow_and_update().clone();
    assert!(deleted.draft.is_none());
    assert!(deleted.draft_revision > before.draft_revision);
    assert!(deleted.revisions.library > before.revisions.library);
    assert!(
        c.subscribe().borrow().draft.is_none(),
        "reconnect starts with the complete current document"
    );
    drop(c);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn stopped_position_projection_is_strict_and_refuses_ambiguous_crossings() {
    use openlaser_compiler::cut::{Contour, Segment};
    use openlaser_compiler::program::Job;
    use openlaser_compiler::settings::Overrides;
    use openlaser_core::toolpath::{PreparedSegment, SegmentProcess};
    use openlaser_server::resume;
    let (_simulator, shared) = start("projection").await;
    setup(&shared).await;
    let settings = shared.lock().await.compile_inputs(false).unwrap().settings;
    let contour = |points: &[[f64; 2]]| Contour {
        segments: points
            .windows(2)
            .map(|p| {
                PreparedSegment::new(
                    Segment::Line { start: p[0], end: p[1] },
                    SegmentProcess::default(),
                )
            })
            .collect(),
        overrides: Overrides::default(),
    };
    let job = Job::compile(&settings, &[contour(&[[50., 0.], [70., 0.]])]).unwrap();
    let checkpoint = resume::locate(&job, 0, [60., 0.199_999]).unwrap();
    assert!((checkpoint.fraction - 0.5).abs() < 1e-7);
    assert!(resume::locate(&job, 0, [60., 0.2]).is_err());
    assert!(resume::locate(&job, 0, [60., 0.200_001]).is_err());
    assert!(resume::locate(&job, u32::MAX, [60., 0.]).is_err());
    assert_eq!(
        resume::locate(&job, 0x8000_0000, [10., 10.]).unwrap(),
        resume::Checkpoint { pass: 0, fraction: 0. }
    );
    let crossing =
        Job::compile(&settings, &[contour(&[[-10., -10.], [10., 10.], [-10., 10.], [10., -10.]])])
            .unwrap();
    assert!(
        resume::locate(&crossing, 0, [0., 0.])
            .unwrap_err()
            .to_string()
            .contains("more than one place")
    );
    let mut point = job;
    point.passes[0].pass.kind = openlaser_compiler::pass::PassKind::PrePierce;
    point.passes[0].pieces.clear();
    point.passes[0].events = vec![None];
    assert_eq!(
        resume::locate(&point, 0, [50., 0.]).unwrap(),
        resume::Checkpoint { pass: 0, fraction: 1. }
    );
    openlaser_server::shutdown(&shared).await.unwrap();
}

async fn request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    use std::io::{Read as _, Write as _};
    let body = serde_json::to_string(&body).unwrap();
    let message = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tokio::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        socket.write_all(message.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (header, body) = response.split_once("\r\n\r\n").unwrap();
        let status = header.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn http_edits_are_revision_qualified_and_the_live_stream_clears_and_closes() {
    use serde_json::json;
    use std::io::{BufRead as _, Write as _};
    use std::time::Duration;
    let (_simulator, shared) = start("http-revisions").await;
    setup(&shared).await;
    let initial = shared.lock().await.document().draft.unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(openlaser_server::serve_until(shared.clone(), address, async {
        stopped.await.unwrap();
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::net::TcpStream::connect(address).await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
    let stream = tokio::task::spawn_blocking(move || {
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        socket
            .write_all(
                b"GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
            )
            .unwrap();
        for line in std::io::BufReader::new(socket).lines() {
            let line = line.unwrap();
            if let Some(data) = line.strip_prefix("data:") {
                send.send(serde_json::from_str(data.trim()).unwrap()).unwrap();
            }
        }
    });
    let first = tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
    assert!(
        first.get("draft").is_some()
            && first.get("library").is_some()
            && first.get("bindings").is_some()
    );
    let path = format!("/api/draft/features?revision={}", initial.revision);
    let (status, reply) = request(address, "POST", &path, json!(initial.features)).await;
    assert_eq!(status, 200);
    assert!(reply["draft_revision"].as_u64().unwrap() > initial.revision);
    assert_eq!(reply["draft"]["revision"], reply["draft_revision"]);
    let (status, refused) =
        request(address, "POST", &path, json!({"skip_layers": ["stale edit"]})).await;
    assert_eq!(status, 409);
    assert!(refused["error"].as_str().unwrap().contains("draft changed"));
    assert!(shared.lock().await.document().draft.unwrap().features.skip_layers.is_empty());
    assert_eq!(request(address, "POST", "/api/machine/heartbeat", json!({})).await.0, 400);
    assert_eq!(
        request(address, "DELETE", &format!("/api/parts/{}", initial.parts[0].id), json!(null))
            .await
            .0,
        200
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.recv().await {
            if event.get("draft") == Some(&serde_json::Value::Null) {
                break;
            }
        }
    })
    .await
    .unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), serving).await.unwrap().unwrap().unwrap();
    stream.await.unwrap();
}
