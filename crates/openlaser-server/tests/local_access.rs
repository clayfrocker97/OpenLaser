// SPDX-License-Identifier: GPL-3.0-or-later
//! Every network screen opens directly; command handoff preserves independent Stop.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "isolated HTTP fixture")]
mod common;

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    host: &str,
    page: &str,
    body: Value,
) -> (u16, String, Value) {
    let body = if method == "GET" { String::new() } else { body.to_string() };
    let wire = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nX-OpenLaser-Client: {page}\r\nX-OpenLaser-Name: Test device\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tokio::task::spawn_blocking(move || {
        let mut socket = TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        socket.write_all(wire.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let code = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let value = serde_json::from_str(body).unwrap_or(Value::Null);
        (code, headers.into(), value)
    })
    .await
    .unwrap()
}

async fn serving(
    shared: openlaser_server::coordinator::Shared,
) -> (SocketAddr, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<Result<(), String>>) {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let (ready, address) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(openlaser_server::serve_until_ready(
        shared.clone(),
        "127.0.0.1:0".parse().unwrap(),
        async {
            let _ = stopped.await;
        },
        |address| {
            ready.send(address).unwrap();
        },
    ));
    let address = address.await.unwrap();
    (address, stop, task)
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "One anonymous phone follows direct access, control handoff and pending-command Stop"
)]
async fn lan_access_needs_no_pairing_and_stop_bypasses_pending_commands() {
    let (simulator, shared) = common::start("lan-access").await;
    let (address, stop, task) = serving(shared.clone()).await;
    let local = address.to_string();
    let remote = "openlaser.local";
    assert_eq!(request(address, "GET", "/api/state", remote, "", Value::Null).await.0, 200);
    let (code, headers, state) =
        request(address, "GET", "/api/access", remote, "phone", Value::Null).await;
    assert_eq!(code, 200);
    assert_eq!(state["can_control"], true, "the first network screen opens with control");
    assert!(!headers.to_ascii_lowercase().contains("set-cookie"));
    assert!(state.get("pairing").is_none());
    assert_eq!(state["owner"], "Test device");
    assert_eq!(
        request(address, "POST", "/api/folders", remote, "phone", json!({"name":"From phone"}))
            .await
            .0,
        200
    );
    assert_eq!(
        request(address, "POST", "/api/folders", &local, "desk", json!({"name":"Blocked"})).await.0,
        409
    );
    assert_eq!(
        request(address, "POST", "/api/access/control", &local, "desk", json!({})).await.0,
        200
    );
    assert_eq!(
        request(address, "POST", "/api/folders", remote, "phone", json!({"name":"Blocked"}))
            .await
            .0,
        409
    );
    assert_eq!(
        request(address, "POST", "/api/access/control", remote, "phone", json!({})).await.0,
        200
    );
    assert_eq!(
        request(address, "POST", "/api/access/release", remote, "phone", json!({})).await.0,
        200
    );
    let (_, _, available) =
        request(address, "GET", "/api/access", &local, "desk", Value::Null).await;
    assert_eq!(available["can_control"], true, "an unclaimed idle workspace opens directly");
    assert_eq!(
        request(address, "POST", "/api/access/control", remote, "phone", json!({})).await.0,
        200
    );
    openlaser_server::connect::connect(&shared).await.unwrap();
    openlaser_server::machine::home(&shared).await.unwrap();
    common::until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    openlaser_server::machine::outputs(
        &shared,
        openlaser_server::machine::OutputRequest::Pointer,
        openlaser_controller::lease::Lease { client: "phone".into(), sequence: 1 },
    )
    .await
    .unwrap();
    assert_eq!(
        request(address, "POST", "/api/access/control", &local, "desk", json!({})).await.0,
        409,
        "control must not transfer during motion or held outputs"
    );
    assert_eq!(
        request(address, "POST", "/api/machine/stop", remote, "", json!({})).await.0,
        200,
        "every network screen can stop, even without a page identity"
    );
    common::until(&shared, 5, |d| d.machine.operation.is_none()).await;
    assert_eq!(simulator.control().view().outputs, 0);

    let coordinator = shared.lock().await;
    let pending = tokio::spawn(async move {
        request(address, "POST", "/api/folders", remote, "phone", json!({"name":"Pending command"}))
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!pending.is_finished());
    let queued = tokio::spawn(async move {
        request(
            address,
            "POST",
            "/api/folders",
            remote,
            "phone",
            json!({"name":"Cancelled command"}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!queued.is_finished());
    let status = tokio::time::timeout(
        Duration::from_secs(1),
        request(address, "GET", "/api/access", &local, "desk", Value::Null),
    )
    .await
    .unwrap();
    assert_eq!(status.0, 200);
    let stopped = tokio::time::timeout(
        Duration::from_secs(1),
        request(address, "POST", "/api/machine/stop", remote, "", json!({})),
    )
    .await
    .unwrap();
    assert_eq!(stopped.0, 200, "Stop bypasses an unfinished owner command");
    drop(coordinator);
    assert_eq!(pending.await.unwrap().0, 200);
    assert_eq!(queued.await.unwrap().0, 409, "Stop cancels commands still waiting at admission");
    assert!(!shared.lock().await.config.data_dir.join("paired-devices.json").exists());
    assert_eq!(
        request(address, "POST", "/api/access/shutdown", remote, "phone", json!({})).await.0,
        403
    );

    let (listening, listener_ready) = tokio::sync::oneshot::channel();
    let events = tokio::task::spawn_blocking(move || {
        let mut socket = TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        socket
            .write_all(
                b"GET /api/events HTTP/1.1\r\nHost: openlaser.local\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let mut buffer = [0; 4096];
        let read = socket.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..read]).starts_with("HTTP/1.1 200"));
        listening.send(()).unwrap();
        socket.read_to_end(&mut Vec::new()).unwrap();
    });
    listener_ready.await.unwrap();
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), events).await.unwrap().unwrap();
}

fn upload(address: SocketAddr, path: &str, body: &[u8], length: usize) -> TcpStream {
    let mut socket = TcpStream::connect(address).unwrap();
    socket.write_all(format!("POST {path} HTTP/1.1\r\nHost: {address}\r\nX-OpenLaser-Client: phone\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n").as_bytes()).unwrap();
    socket.write_all(body).unwrap();
    socket
}

// Does not use the blocking pool occupied by the deliberately queued import.
async fn take_control(address: SocketAddr) -> String {
    let (done, response) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut socket = TcpStream::connect(address).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        socket.write_all(format!("POST /api/access/control HTTP/1.1\r\nHost: {address}\r\nX-OpenLaser-Client: desk\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let _ = done.send(response);
    });
    response.await.unwrap()
}

#[test]
fn disconnected_import_finishes_or_cancels_and_keeps_handoff_ordered() {
    // Alarm history owns one blocking thread; occupy the second to queue XML
    // parsing after reservation, without adding test hooks to production code.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
        .unwrap();
    runtime.block_on(async {
        for cancel in [false, true] {
            let (_simulator, shared) =
                common::start(if cancel { "disconnect-cancel" } else { "disconnect-finish" }).await;
            openlaser_server::connect::connect(&shared).await.unwrap();
            let (address, stop, task) = serving(shared.clone()).await;
            let (unblock, block) = std::sync::mpsc::channel();
            let (entered, started) = tokio::sync::oneshot::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                let _ = block.recv();
            });
            started.await.unwrap();
            let bytes = std::fs::read(common::fixture("xml/harness-backup.xml")).unwrap();
            let socket =
                upload(address, "/api/machine/files?name=disconnect.xml", &bytes, bytes.len());
            common::until(&shared, 5, |d| {
                d.readiness.mode.reason.as_deref() == Some("an operation is being prepared")
            })
            .await;
            drop(socket);
            let handoff = tokio::spawn(take_control(address));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !handoff.is_finished(),
                "disconnect cannot release the workflow's command gate"
            );
            if cancel {
                openlaser_server::machine::stop(&shared).await.unwrap();
            }
            unblock.send(()).unwrap();
            blocker.await.unwrap();
            common::until(&shared, 5, |d| {
                d.readiness.mode.reason.as_deref() != Some("an operation is being prepared")
            })
            .await;
            let response =
                tokio::time::timeout(Duration::from_secs(5), handoff).await.unwrap().unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            openlaser_server::machine::import_file(&shared, "retry.xml", &bytes).await.unwrap();
            stop.send(()).unwrap();
            task.await.unwrap().unwrap();
        }
    });
}

#[tokio::test]
async fn an_incomplete_upload_does_not_block_owner_heartbeat_or_stop() {
    let (simulator, shared) = common::start("upload-heartbeat").await;
    openlaser_server::connect::connect(&shared).await.unwrap();
    openlaser_server::machine::home(&shared).await.unwrap();
    common::until(&shared, 5, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    let (address, stop, task) = serving(shared.clone()).await;
    openlaser_server::machine::outputs(
        &shared,
        openlaser_server::machine::OutputRequest::Pointer,
        openlaser_controller::lease::Lease { client: "phone".into(), sequence: 1 },
    )
    .await
    .unwrap();
    common::until(&shared, 5, |_| simulator.control().view().outputs != 0).await;
    let upload = upload(address, "/api/folders", b"{", 10000);
    tokio::time::sleep(Duration::from_millis(60)).await;
    let body = json!({"lease":{"client":"phone","sequence":1}});
    assert_eq!(
        request(address, "POST", "/api/machine/heartbeat", "openlaser.local", "desk", body.clone())
            .await
            .0,
        409
    );
    // Renew beyond the pointer's two-second lease, while the upload stays open.
    for _ in 0..10 {
        let reply = tokio::time::timeout(
            Duration::from_millis(500),
            request(
                address,
                "POST",
                "/api/machine/heartbeat",
                "openlaser.local",
                "phone",
                body.clone(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(reply.0, 200);
        assert_ne!(simulator.control().view().outputs, 0);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        request(address, "POST", "/api/machine/stop", "openlaser.local", "", json!({})).await.0,
        200
    );
    common::until(&shared, 5, |_| simulator.control().view().outputs == 0).await;
    drop(upload);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
