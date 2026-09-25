// SPDX-License-Identifier: GPL-3.0-or-later
//! The real HTTP boundary, with every machine address bound to a simulator.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "isolated test server")]
mod common;

use openlaser_server::coordinator::Shared;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::SocketAddr;

struct Server {
    shut_down: bool,
    simulator: openlaser_controller::Simulator,
    shared: Shared,
    address: SocketAddr,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start() -> Self {
        Self::start_mode(openlaser_core::LaserMode::Fiber).await
    }

    async fn start_mode(mode: openlaser_core::LaserMode) -> Self {
        let simulator = openlaser_controller::Simulator::start().await.unwrap();
        let mut config = common::config("http-boundary", simulator.config());
        config.mode = Some(mode);
        let shared = openlaser_server::Coordinator::start(config).unwrap();
        common::seed(&shared, &simulator).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let router = openlaser_server::api::router(shared.clone(), "ui/dist".into());
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self { shut_down: false, simulator, shared, address, stop, task }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> (u16, String) {
        self.request_bytes(method, path, body.map(str::as_bytes), headers).await
    }

    async fn request_bytes(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> (u16, String) {
        let host = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("host"))
            .map_or_else(|| self.address.to_string(), |(_, value)| (*value).to_owned());
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        for (key, value) in headers.iter().filter(|(key, _)| !key.eq_ignore_ascii_case("host")) {
            write!(request, "{key}: {value}\r\n").unwrap();
        }
        if let Some(body) = body {
            write!(
                request,
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
        } else {
            request.push_str("Content-Length: 0\r\n\r\n");
        }
        let mut request = request.into_bytes();
        if let Some(body) = body {
            request.extend_from_slice(body);
        }
        let address = self.address;
        tokio::task::spawn_blocking(move || {
            let mut socket = std::net::TcpStream::connect(address).unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            socket.write_all(&request).unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
            (status, response)
        })
        .await
        .unwrap()
    }

    async fn shutdown(&mut self) {
        openlaser_server::shutdown(&self.shared).await.unwrap();
        self.shut_down = true;
    }

    async fn edit(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        let revision = self.shared.lock().await.document().draft_revision;
        let (status, response) = self
            .request(
                "POST",
                &format!("/api/draft/{path}?revision={revision}"),
                Some(&body.to_string()),
                &[],
            )
            .await;
        assert_eq!(status, 200, "{response}");
        response_json(&response)
    }

    async fn close(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        if !self.shut_down {
            openlaser_server::shutdown(&self.shared).await.unwrap();
        }
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one operator journey from a cold CO2 setup through two fiber jobs and a dry run"
)]
async fn editing_builds_automatically_and_repeated_jobs_keep_home_and_calibration() {
    use openlaser_controller::state::ProgramState;
    use openlaser_core::LaserMode;
    use openlaser_protocol::requests;
    use openlaser_server::coordinator::{NewRecipe, Values};
    use openlaser_server::{connect, machine};
    use serde_json::json;

    let server = Server::start_mode(LaserMode::Co2).await;
    connect::connect(&server.shared).await.unwrap();
    let (part, recipe) = {
        let mut c = server.shared.lock().await;
        let part = c
            .import_part(
                "line.dxf",
                &std::fs::read(common::fixture("dxf/minimal-line.dxf")).unwrap(),
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
        (part.id, recipe.id)
    };
    let (status, response) =
        server.request("POST", &format!("/api/draft/part/{part}"), None, &[]).await;
    assert_eq!(status, 200, "{response}");
    let response = server.edit(&format!("recipe/{recipe}"), json!(null)).await;
    assert!(response["draft"]["compiled"].is_object(), "choosing material builds the preview");
    assert_eq!(response["draft"]["dry_run"], false);
    let doc = server.shared.lock().await.document();
    assert_eq!(doc.mode, Some(LaserMode::Fiber));
    assert_eq!(doc.bindings.as_ref().unwrap().outputs.pointer_port, 6);
    assert_eq!(doc.bindings.as_ref().unwrap().outputs.shutter_port, 5);
    machine::home(&server.shared).await.unwrap();
    common::until(&server.shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none())
        .await;
    machine::calibrate(&server.shared).await.unwrap();
    common::until(&server.shared, 10, |d| d.calibration.current && d.machine.operation.is_none())
        .await;

    let control = server.simulator.control();
    for attempt in 0..2 {
        if attempt > 0 {
            let response = server.edit("placement", json!({"kind":"new_run"})).await;
            assert_eq!(response["draft"]["placement"]["captured"], false);
        }
        let response = server.edit("placement", json!({"kind":"set_origin"})).await;
        assert!(response["draft"]["compiled"].is_object());
        assert_eq!(response["draft"]["placement"]["captured"], true);
        control.clear_writes();
        common::run(&server.shared).await.unwrap();
        common::until(&server.shared, 20, |d| {
            d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
                && d.machine.operation.is_none()
        })
        .await;
        let doc = server.shared.lock().await.document();
        assert!(doc.machine.session.homed && doc.calibration.current);
        let plant = control.view();
        assert_eq!(plant.alarms, [0; 3]);
        assert_eq!((plant.outputs, plant.extended_outputs, plant.analog), (0, 0, [0, 0]));
        let writes = control.writes();
        assert!(!writes.contains(&requests::home_xy()), "a new job must not repeat XY homing");
        if attempt > 0 {
            let head_home = writes.iter().position(|w| *w == requests::head_home()).unwrap();
            let upload = writes.iter().position(|w| w.address == 102).unwrap();
            assert!(head_home < upload, "clear the lowered head before any XY program starts");
        }
    }
    server.edit("placement", json!({"kind":"new_run"})).await;
    server.edit("compile", json!({"dry_run":true})).await;
    let response = server.edit("transform", json!({"contours":[0],"matrix":[1,0,0,1,1,0]})).await;
    assert_eq!(
        response["draft"]["compiled"]["dry_run"], true,
        "geometry edits must retain dry run"
    );
    assert_eq!(response["draft"]["dry_run"], true);
    let doc = server.shared.lock().await.document();
    assert!(doc.machine.session.homed && doc.calibration.current);
    common::run(&server.shared).await.unwrap();
    common::until(&server.shared, 20, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
            && d.machine.operation.is_none()
    })
    .await;
    assert_eq!(control.view().alarms, [0; 3]);
    server.close().await;
}

#[tokio::test]
async fn browser_commands_reject_foreign_origins_and_rebound_hostnames() {
    let server = Server::start().await;
    let action = "/api/machine/cancel";
    for headers in [
        vec![("Origin", "https://unrelated.example"), ("Sec-Fetch-Site", "cross-site")],
        vec![
            ("Host", "unrelated.example"),
            ("Origin", "http://unrelated.example"),
            ("Sec-Fetch-Site", "same-origin"),
        ],
        vec![("Origin", "null")],
        vec![("Sec-Fetch-Site", "same-site")],
    ] {
        assert_eq!(server.request("POST", action, None, &headers).await.0, 403);
    }
    let origin = format!("http://{}", server.address);
    let (status, response) = server.request("POST", action, None, &[("Origin", &origin)]).await;
    assert_eq!(status, 200, "{response}");
    assert!(response.to_ascii_lowercase().contains("x-frame-options: deny"));
    assert_eq!(server.request("POST", action, None, &[]).await.0, 200);
    server.close().await;
}

#[tokio::test]
async fn independent_machine_routes_preserve_errors_and_shutdown_admission() {
    let mut server = Server::start().await;
    for action in [
        "jog",
        "table",
        "pulse",
        "gas-test",
        "gas-calibration",
        "release",
        "heartbeat",
        "outputs",
        "mode",
    ] {
        let (status, body) =
            server.request("POST", &format!("/api/machine/{action}"), Some("{}"), &[]).await;
        assert_eq!(status, 400, "{action}: {body}");
        assert!(body.contains("needs"), "{action}: {body}");
    }
    // Home verifies unfinished setup before dispatch, so a disconnected request
    // is refused immediately. Calibration still reports its failure later.
    assert_eq!(server.request("POST", "/api/machine/calibrate", Some("{}"), &[]).await.0, 200);
    for action in ["home", "go-origin", "run", "frame", "resume", "hold"] {
        let (status, body) =
            server.request("POST", &format!("/api/machine/{action}"), Some("{}"), &[]).await;
        assert_eq!(status, 409, "{action}: {body}");
    }
    assert!(server.simulator.control().writes().is_empty(), "offline requests cannot write");
    assert_eq!(server.request("POST", "/api/machine/not-an-action", None, &[]).await.0, 404);
    assert_eq!(server.request("POST", "/api/machine/cancel", Some("{"), &[]).await.0, 400);
    assert_eq!(server.request("POST", "/api/machine/disconnect", Some("{}"), &[]).await.0, 200);
    server.shutdown().await;
    assert_eq!(server.request("POST", "/api/machine/connect", Some("{}"), &[]).await.0, 409);
    assert_eq!(server.request("POST", "/api/machine/cancel", Some("{}"), &[]).await.0, 409);
    let (_, disconnect) = server.request("POST", "/api/machine/disconnect", Some("{}"), &[]).await;
    assert!(!disconnect.contains("shutting down"));
    // Cleanup commands reach the controller path even after its task has exited.
    let (_, stop) = server.request("POST", "/api/machine/stop", Some("{}"), &[]).await;
    assert!(!stop.contains("shutting down"));
    server.close().await;
}

fn response_json(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn a_browser_disconnect_finishes_its_already_saved_draft_preparation() {
    let server = Server::start().await;
    let mut source = String::from("<svg xmlns='http://www.w3.org/2000/svg'>");
    for i in 0..800 {
        write!(source, "<rect x='{}' y='{}' width='2' height='2'/>", (i % 40) * 4, (i / 40) * 4)
            .unwrap();
    }
    source.push_str("</svg>");
    let id = server.shared.lock().await.import_part("reload.svg", source.as_bytes()).unwrap().id;
    let mut state = server.shared.lock().await.subscribe();
    let mut socket = std::net::TcpStream::connect(server.address).unwrap();
    socket
        .write_all(
            format!(
                "POST /api/draft/part/{id} HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\n\r\n",
                server.address
            )
            .as_bytes(),
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if state
                .borrow_and_update()
                .draft
                .as_ref()
                .is_some_and(|d| d.error.as_deref() == Some("preparing geometry"))
            {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    // Keep publication pending until the server observes the closed browser.
    let guard = server.shared.lock().await;
    drop(socket);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    drop(guard);
    common::until(&server.shared, 8, |d| {
        d.draft.as_ref().is_some_and(|draft| {
            draft.preview.is_some() && draft.error.as_deref() != Some("preparing geometry")
        })
    })
    .await;
    assert_eq!(
        server
            .shared
            .lock()
            .await
            .document()
            .draft
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .contours
            .len(),
        800
    );
    server.close().await;
}

#[tokio::test]
async fn a_completed_cut_takes_its_sheet_off_the_rack_once() {
    use openlaser_core::LaserMode;
    use openlaser_core::features::Features;
    use openlaser_server::coordinator::{NewRecipe, Values};
    use openlaser_server::inventory::NewStock;
    use openlaser_server::nesting::StockChoice;
    use openlaser_server::{connect, machine};
    let server = Server::start().await;
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="30mm" viewBox="0 0 30 30"><rect x="5" y="5" width="20" height="20" fill="none" stroke="black"/></svg>"#;
    let rack = {
        let mut c = server.shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Sheet steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        let part = c.import_part("square.svg", source).unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.set_features(Features::default()).unwrap();
        c.add_stock(NewStock {
            material: "sheet steel".into(),
            thickness_mm: 1.,
            laser: LaserMode::Fiber,
            width_mm: 80.,
            height_mm: 120.,
            quantity: 2,
            folder: None,
        })
        .unwrap()
    };
    machine::prepare(&server.shared).await.unwrap();
    let turned = StockChoice::Rectangle { width: 120., height: 80. };
    server.shared.lock().await.set_stock(turned).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    connect::connect(&server.shared).await.unwrap();
    machine::home(&server.shared).await.unwrap();
    common::until(&server.shared, 10, |d| d.machine.session.homed).await;
    let on_hand = |c: &openlaser_server::Coordinator| {
        c.inventory.iter().find(|i| i.id == rack).unwrap().quantity
    };
    machine::compile(&server.shared, false).await.unwrap();
    common::run(&server.shared).await.unwrap();
    common::until(&server.shared, 30, |d| d.completed_sheet.is_some()).await;
    let c = server.shared.lock().await;
    assert_eq!(on_hand(&c), 1, "the turned 120 × 80 sheet came off the 80 × 120 entry");
    assert_eq!(c.document().library.stock[0].quantity, 1);
    drop(c);
    server.close().await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one HTTP journey from real completion through reused inventory"
)]
async fn completed_sheet_survives_edits_and_becomes_stock_with_all_cutouts_excluded() {
    use openlaser_core::LaserMode;
    use openlaser_core::features::Features;
    use openlaser_server::coordinator::{NewRecipe, Values};
    use openlaser_server::nesting::{self, NestRequest, StockChoice};
    use openlaser_server::{connect, machine};
    let server = Server::start().await;
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100"><g fill="none" stroke="black"><rect width="100" height="100"/><rect x="15" y="15" width="20" height="20"/><circle cx="25" cy="25" r="3"/><rect x="65" y="65" width="15" height="15"/></g></svg>"#;
    {
        let mut c = server.shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Sheet steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        let part = c.import_part("cut-sheet.svg", source).unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.set_features(Features::default()).unwrap();
    }
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.set_stock(StockChoice::Outline { contour: 0 }).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    // A single selection group containing two separate manufactured pieces.
    server.shared.lock().await.set_grouped(&[0, 2], true).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.save_job("Two grouped pieces").unwrap();
    connect::connect(&server.shared).await.unwrap();
    machine::home(&server.shared).await.unwrap();
    common::until(&server.shared, 10, |d| d.machine.session.homed).await;
    machine::compile(&server.shared, false).await.unwrap();
    common::run(&server.shared).await.unwrap();
    common::until(&server.shared, 30, |d| d.completed_sheet.is_some()).await;
    let id = server.shared.lock().await.document().completed_sheet.unwrap();
    let path = format!("/api/sheets/{id}");
    let (status, response) = server.request("GET", &path, None, &[]).await;
    assert_eq!(status, 200, "{response}");
    let completed = response_json(&response);
    assert_eq!(completed["state"], "completed");
    assert_eq!(
        completed["cutouts"].as_array().unwrap().len(),
        2,
        "both outer pieces are reserved; their internal hole is already inside the removed area"
    );
    // Changes after the run cannot rewrite the physical sheet record.
    server.shared.lock().await.set_origin([150., 150.]).unwrap();
    let (_, after) = server.request("GET", &path, None, &[]).await;
    assert_eq!(completed, response_json(&after));
    let change = serde_json::json!({"revision":completed["revision"],"name":"Usable sheet remainder","bounds":null}).to_string();
    let (status, response) = server.request("POST", &path, Some(&change), &[]).await;
    assert_eq!(status, 200, "{response}");
    assert_eq!(
        server.request("POST", &path, Some(&change), &[]).await.0,
        409,
        "stale inspection is refused"
    );
    let (_, ready) = server.request("GET", "/api/sheets?remnants=true", None, &[]).await;
    assert_eq!(response_json(&ready)["items"].as_array().unwrap().len(), 1);
    server.shared.lock().await.set_stock(StockChoice::Remnant { id: id.clone() }).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.set_grouped(&[0], false).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.set_grouped(&[0, 1], true).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    let revision = server.shared.lock().await.document().draft_revision;
    let request = NestRequest {
        contours: vec![],
        quantity: 1,
        settings: openlaser_core::nesting::NestSettings::default(),
        seconds: 2,
        stock: vec![],
    };
    let work = nesting::start(&server.shared, revision, request).await.unwrap();
    loop {
        let view = nesting::status(&server.shared, work.id).await.unwrap();
        if !view.running {
            assert!(view.error.is_none(), "{:?}", view.error);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    nesting::apply(&server.shared, work.id).await.unwrap();
    machine::compile(&server.shared, false).await.unwrap();
    let (saved, root) = {
        let mut c = server.shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert_eq!(d.stock_cutouts.len(), 2);
        let n = d.current.nesting.as_ref().unwrap();
        let drawing = d.drawing().unwrap();
        let openlaser_core::nesting::NestStock::Remnant { cutouts, .. } = &n.stock else {
            panic!("expected remnant stock");
        };
        openlaser_nest::check_region(
            &nesting::stock(drawing, n).unwrap(),
            cutouts,
            &openlaser_server::draft::place(drawing, &d.current.placed).contours,
            n.settings.margin,
        )
        .unwrap();
        (c.save_job("Next use of remainder").unwrap(), c.config.data_dir.clone())
    };
    // Recording the next use consumes this stock once and inherits its cutouts.
    let action = format!("/api/jobs/{}/cut-sheet", saved.id);
    let (status, response) = server.request("POST", &action, None, &[]).await;
    assert_eq!(status, 200, "{response}");
    assert_eq!(server.request("POST", &action, None, &[]).await.0, 409);
    let (_, used) = server.request("GET", &path, None, &[]).await;
    assert_eq!(response_json(&used)["used"], true);
    let (_, remaining) = server.request("GET", "/api/sheets?remnants=true", None, &[]).await;
    assert!(response_json(&remaining)["items"].as_array().unwrap().is_empty());
    server.close().await;
    assert_eq!(std::fs::read_dir(root.join("sheets")).unwrap().count(), 2);
}

/// Several parts nest onto an inspected remnant clear of its cut area, what
/// does not fit goes onto the fresh sheet listed after it, the running
/// search shows each sheet with its own stock, and each saved sheet cuts
/// only its parts and keeps its stock.
#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one journey from a used sheet to saved remnant jobs")]
async fn several_parts_nest_on_a_remnant_then_a_chosen_sheet() {
    use openlaser_core::LaserMode;
    use openlaser_core::features::Features;
    use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
    use openlaser_core::nesting::{NestRotation, NestSettings, NestStock};
    use openlaser_server::coordinator::{NewRecipe, Values};
    use openlaser_server::machine;
    use openlaser_server::nesting::{self, NestRequest, StockChoice, StockSource};
    let rect = |x: f64, y: f64, w: f64, h: f64| {
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        Contour {
            layer: "Cut".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: p[i].into(), end: p[(i + 1) % 4].into() })
                .collect(),
        }
    };
    let plain = Features { leads: None, kerf: None, ..Features::default() };
    let server = Server::start().await;
    // A 300 x 200 sheet whose left 180 mm were cut away.
    let (recipe, parts) = {
        let mut c = server.shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Remnant steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap()
            .id;
        let used = Drawing { contours: vec![rect(0., 0., 300., 200.), rect(10., 5., 180., 190.)] };
        let used = c.library.add_part("used.dxf", b"used sheet", used).unwrap().id;
        let disc =
            Contour { layer: "Cut".into(), curves: vec![Curve::circle(Point::new(0., 0.), 15.)] };
        let mut add = |name: &str, contours: Vec<Contour>| {
            c.library.add_part(name, name.as_bytes(), Drawing { contours }).unwrap().id
        };
        let parts = [
            add("plate.dxf", vec![rect(0., 0., 100., 60.), rect(20., 20., 10., 10.)]),
            add("tab.svg", vec![rect(-500., 300., 40., 20.)]),
            add("disc.dxf", vec![disc]),
        ];
        c.open_part(&used).unwrap();
        c.set_recipe(&recipe).unwrap();
        c.set_features(plain.clone()).unwrap();
        (recipe, parts)
    };
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.set_stock(StockChoice::Outline { contour: 0 }).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    let used = server.shared.lock().await.save_job("Used sheet").unwrap();
    let (status, response) =
        server.request("POST", &format!("/api/jobs/{}/cut-sheet", used.id), None, &[]).await;
    assert_eq!(status, 200, "{response}");
    let sheet = response_json(&response)["id"].as_str().unwrap().to_owned();
    let path = format!("/api/sheets/{sheet}");
    let (_, response) = server.request("GET", &path, None, &[]).await;
    let revision = response_json(&response)["revision"].clone();
    let change = serde_json::json!({"revision": revision, "name": "Right strip", "bounds": null});
    let (status, response) = server.request("POST", &path, Some(&change.to_string()), &[]).await;
    assert_eq!(status, 200, "{response}");

    // Three parts on that remnant: only the plate is too wide for the strip.
    {
        let mut c = server.shared.lock().await;
        c.open_parts(&parts).unwrap();
        c.set_recipe(&recipe).unwrap();
        c.set_features(plain).unwrap();
    }
    machine::prepare(&server.shared).await.unwrap();
    server.shared.lock().await.set_stock(StockChoice::Remnant { id: sheet.clone() }).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    let (revision, remnant) = {
        let c = server.shared.lock().await;
        let draft = c.draft.as_ref().unwrap();
        assert_eq!(draft.stock_cutouts.len(), 1, "the cut area is reserved");
        let remnant =
            nesting::stock(draft.drawing().unwrap(), draft.current.nesting.as_ref().unwrap())
                .unwrap()
                .bounds()
                .unwrap();
        (c.document().draft_revision, remnant)
    };
    let settings = NestSettings {
        spacing: 3.,
        margin: 3.,
        remnant_clearance: 5.,
        rotation: NestRotation::Fixed,
    };
    // The remnant first, then one fresh sheet of its size, as the operator
    // lists them.
    let fresh = StockSource::Sheet { width: remnant.width(), height: remnant.height(), count: 1 };
    let stock = vec![StockSource::Remnant { id: sheet.clone() }, fresh];
    let request = NestRequest { contours: vec![], quantity: 1, settings, seconds: 2, stock };
    let work = nesting::start(&server.shared, revision, request).await.unwrap();
    let result = loop {
        let view = nesting::status(&server.shared, work.id).await.unwrap();
        if let Some(live) = &view.live {
            assert!(!live.stock_outline.is_empty());
        }
        if !view.running {
            break view;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(result.error.is_none(), "{:?}", result.error);
    let sheets: Vec<_> = result.sheets.iter().map(|s| (s.parts, s.source)).collect();
    assert_eq!(sheets, [(2, 0), (1, 1)]);
    let first = nesting::sheet_preview(&server.shared, work.id, 0).await.unwrap();
    let second = nesting::sheet_preview(&server.shared, work.id, 1).await.unwrap();
    assert_eq!((first.stock_cutouts.len(), second.stock_cutouts.len()), (1, 0));
    nesting::apply(&server.shared, work.id).await.unwrap();
    machine::prepare(&server.shared).await.unwrap();
    {
        let c = server.shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        let n = d.current.nesting.as_ref().unwrap();
        let NestStock::Remnant { cutouts, .. } = &n.stock else { panic!("remnant stock") };
        let drawing = d.drawing().unwrap();
        let placed = openlaser_server::draft::place(drawing, &d.current.placed);
        openlaser_nest::check_region(
            &nesting::stock(drawing, n).unwrap(),
            cutouts,
            &placed.contours,
            n.margin(),
        )
        .unwrap();
        assert!(placed.bounds().unwrap().min.x > remnant.min.x + 180., "clear of the cut area");
    }
    server.shared.lock().await.select_sheet(1).unwrap();
    machine::prepare(&server.shared).await.unwrap();
    {
        let c = server.shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        let NestStock::Rectangle { bounds } = &d.current.nesting.as_ref().unwrap().stock else {
            panic!("fresh rectangular stock")
        };
        assert!((bounds.width() - remnant.width()).abs() < 1e-9);
        assert!((bounds.height() - remnant.height()).abs() < 1e-9);
        assert!(d.stock_cutouts.is_empty());
    }
    let mut c = server.shared.lock().await;
    c.save_job("Remnant batch").unwrap();
    let names = |job: &openlaser_library::Job| -> Vec<String> {
        job.parts.iter().map(|id| c.library.part(id).unwrap().name.clone()).collect()
    };
    let mut jobs: Vec<_> = c.library.jobs().filter(|j| j.sheet.is_some()).cloned().collect();
    jobs.sort_by_key(|j| j.sheet.as_ref().unwrap().number);
    assert_eq!(jobs.len(), 2);
    assert_eq!(names(&jobs[0]), ["tab", "disc"]);
    assert!(matches!(jobs[0].nesting.as_ref().unwrap().stock, NestStock::Remnant { .. }));
    assert_eq!(names(&jobs[1]), ["plate"]);
    assert!(matches!(jobs[1].nesting.as_ref().unwrap().stock, NestStock::Rectangle { .. }));
    drop(c);
    server.close().await;
}

/// Simplifying a drawing shows what it would do, saves the result as a new
/// part once, and leaves the part and its jobs as they are.
#[tokio::test]
async fn simplifying_a_drawing_saves_a_new_part_and_keeps_the_original() {
    use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
    let server = Server::start().await;
    let circle: Vec<Curve> = (0..120)
        .map(|k| {
            let at = |k: i32| {
                Point::new(50., 50.)
                    + Point::direction(std::f64::consts::TAU * f64::from(k) / 120.) * 20.
            };
            Curve::Line { start: at(k), end: at(k + 1) }
        })
        .collect();
    let square = |x: f64| {
        let p = [[x, 0.], [x + 10., 0.], [x + 10., 10.], [x, 10.]];
        Contour {
            layer: "Cut".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: p[i].into(), end: p[(i + 1) % 4].into() })
                .collect(),
        }
    };
    let drawing = Drawing {
        contours: vec![Contour { layer: "Cut".into(), curves: circle }, square(100.), square(100.)],
    };
    let original = server
        .shared
        .lock()
        .await
        .library
        .add_part("flattened.dxf", b"flat", drawing.clone())
        .unwrap();
    let path = format!("/api/parts/{}/simplify", original.id);
    let ask = |save: bool| serde_json::json!({ "tolerance": 0.02, "save": save }).to_string();
    let (status, response) = server.request("POST", &path, Some(&ask(false)), &[]).await;
    assert_eq!(status, 200, "{response}");
    let preview = response_json(&response);
    assert_eq!(
        (preview["curves_before"].as_u64(), preview["curves_after"].as_u64()),
        (Some(128), Some(6))
    );
    assert_eq!(
        (preview["contours_after"].as_u64(), preview["repeats"].as_u64()),
        (Some(2), Some(1))
    );
    assert!(preview["part"].is_null());
    let count = server.shared.lock().await.library.parts().count();
    assert_eq!(count, 1, "a preview saves nothing");
    let (status, response) = server.request("POST", &path, Some(&ask(true)), &[]).await;
    assert_eq!(status, 200, "{response}");
    let made = response_json(&response)["part"].as_str().unwrap().to_owned();
    let (_, again) = server.request("POST", &path, Some(&ask(true)), &[]).await;
    assert_eq!(response_json(&again)["part"], made.as_str(), "the same result is the same part");
    {
        let c = server.shared.lock().await;
        let simplified = c.library.part(&openlaser_library::Id::from(made.as_str())).unwrap();
        assert_eq!(simplified.name, "flattened simplified");
        assert_eq!(simplified.drawing.contours.len(), 2);
        assert_eq!(*c.library.part(&original.id).unwrap().drawing, drawing, "the original is kept");
    }
    // A wave simplifies further at a coarser tolerance: a second, numbered part.
    let wave: Vec<Curve> = (0..200)
        .map(|k| {
            let at = |k: i32| Point::new(f64::from(k) * 0.5, 10. * (f64::from(k) / 30.).sin());
            Curve::Line { start: at(k), end: at(k + 1) }
        })
        .collect();
    let wave = Drawing { contours: vec![Contour { layer: "Cut".into(), curves: wave }] };
    let wave = server.shared.lock().await.library.add_part("wave.dxf", b"wave", wave).unwrap();
    let path = format!("/api/parts/{}/simplify", wave.id);
    let mut names = Vec::new();
    for tolerance in [0.01, 0.1] {
        let ask = serde_json::json!({ "tolerance": tolerance, "save": true }).to_string();
        let (status, response) = server.request("POST", &path, Some(&ask), &[]).await;
        assert_eq!(status, 200, "{response}");
        let id = openlaser_library::Id::from(response_json(&response)["part"].as_str().unwrap());
        names.push(server.shared.lock().await.library.part(&id).unwrap().name.clone());
    }
    assert_eq!(names, ["wave simplified", "wave simplified 2"]);
    let bad = serde_json::json!({ "tolerance": 5., "save": false }).to_string();
    assert_eq!(server.request("POST", &path, Some(&bad), &[]).await.0, 409);
    server.close().await;
}

#[tokio::test]
async fn a_job_with_only_an_open_cut_can_be_saved_as_a_remnant() {
    use openlaser_server::coordinator::{NewRecipe, Values};
    let server = Server::start().await;
    {
        let mut c = server.shared.lock().await;
        let part = c.import_part("slit.svg", b"<svg xmlns='http://www.w3.org/2000/svg' width='100mm' height='100mm' viewBox='0 0 100 100'><path d='M10 50L90 50' fill='none' stroke='black'/></svg>").unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Steel".into(),
                laser: openlaser_core::LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
    }
    openlaser_server::machine::prepare(&server.shared).await.unwrap();
    let job = server.shared.lock().await.save_job("Open slit").unwrap();
    let (status, response) =
        server.request("POST", &format!("/api/jobs/{}/cut-sheet", job.id), None, &[]).await;
    assert_eq!(status, 200, "{response}");
    let id = response_json(&response)["id"].as_str().unwrap().to_owned();
    let path = format!("/api/sheets/{id}");
    let (_, response) = server.request("GET", &path, None, &[]).await;
    let sheet = response_json(&response);
    assert_eq!(sheet["cutouts"].as_array().unwrap().len(), 1);
    let change = serde_json::json!({ "revision":sheet["revision"], "name":"Slit remainder", "bounds":{"min":{"x":0,"y":0},"max":{"x":100,"y":100}} });
    let (status, response) = server.request("POST", &path, Some(&change.to_string()), &[]).await;
    assert_eq!(status, 200, "{response}");
    let (_, response) = server.request("GET", "/api/sheets?remnants=true", None, &[]).await;
    assert_eq!(response_json(&response)["items"][0]["id"], id);
    server.close().await;
}

#[tokio::test]
async fn text_preview_creation_and_vector_uploads_reach_real_compilation() {
    use openlaser_core::LaserMode;
    use openlaser_server::coordinator::{NewRecipe, Values};
    let server = Server::start().await;
    let text = serde_json::json!({"value":"BO\nCafé & <H>", "size":8, "family":"sans", "bold":true, "alignment":"center"});
    let count = server.shared.lock().await.library.parts().count();
    let (status, preview) =
        server.request("POST", "/api/text/preview", Some(&text.to_string()), &[]).await;
    assert_eq!(status, 200, "{preview}");
    assert!(response_json(&preview)["contours"].as_u64().unwrap() > 10);
    assert_eq!(server.shared.lock().await.library.parts().count(), count);
    let (status, bad) = server
        .request("POST", "/api/text/preview", Some(r#"{"value":"A🚧B","size":10}"#), &[])
        .await;
    assert_eq!(status, 400, "{bad}");
    assert_eq!(server.shared.lock().await.library.parts().count(), count);

    let request = serde_json::json!({"name":"Pasted lettering","text":text}).to_string();
    let (status, response) = server.request("POST", "/api/parts/text", Some(&request), &[]).await;
    assert_eq!(status, 200, "{response}");
    let created = response_json(&response);
    let mut ids: Vec<openlaser_library::Id> =
        vec![serde_json::from_value(created["id"].clone()).unwrap()];
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path d="M0 0L20 0L20 10" fill="none" stroke="black"/><text x="0" y="25" font-size="8">B</text></svg>"#;
    let dxf = "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n20\n21\n0\n0\nTEXT\n10\n0\n20\n10\n40\n6\n1\nB\n0\nENDSEC\n0\nEOF\n";
    for (name, source) in [("mixed.SVG", svg), ("mixed.DXF", dxf)] {
        let (status, response) =
            server.request("POST", &format!("/api/parts?name={name}"), Some(source), &[]).await;
        assert_eq!(status, 200, "{response}");
        let id: openlaser_library::Id =
            serde_json::from_value(response_json(&response)["id"].clone()).unwrap();
        let c = server.shared.lock().await;
        let drawing = &c.library.part(&id).unwrap().drawing;
        assert_eq!(drawing.contours.iter().filter(|c| !c.is_closed()).count(), 1);
        assert_eq!(drawing.contours.iter().filter(|c| c.is_closed()).count(), 3);
        ids.push(id);
    }
    let recipe = server
        .shared
        .lock()
        .await
        .add_recipe(&NewRecipe {
            name: "Import test".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 1.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    for id in ids {
        {
            let mut c = server.shared.lock().await;
            c.open_part(&id).unwrap();
            c.set_recipe(&recipe.id).unwrap();
        }
        openlaser_server::machine::prepare(&server.shared).await.unwrap();
        openlaser_server::machine::compile(&server.shared, false).await.unwrap();
        let c = server.shared.lock().await;
        let draft = c.draft.as_ref().unwrap();
        assert!(draft.error.is_none());
        assert!(draft.compiled.is_some());
    }
    server.close().await;
}

#[tokio::test]
async fn font_uploads_drive_svg_import_and_text_creation_with_explicit_face_selection() {
    let server = Server::start().await;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="30mm" viewBox="0 0 100 30"><text y="20" font-family="Lobster Two" font-style="italic" font-size="10">Bo &amp; Café</text></svg>"#;
    let (_, before) = server.request("POST", "/api/parts?name=lettering.svg", Some(svg), &[]).await;
    assert!(
        response_json(&before)["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("unavailable"))
    );
    let (status, invalid) =
        server.request("POST", "/api/fonts?name=invalid.ttf", Some("invalid"), &[]).await;
    assert_eq!(status, 400, "{invalid}");
    let (status, response) = server
        .request_bytes(
            "POST",
            "/api/fonts?name=Lobster.ttf",
            Some(include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf")),
            &[],
        )
        .await;
    assert_eq!(status, 200, "{response}");
    let uploaded = response_json(&response);
    assert_eq!(uploaded["existing"], false);
    let font = uploaded["fonts"][0]["id"].as_str().unwrap();
    let (_, duplicate) = server
        .request_bytes(
            "POST",
            "/api/fonts?name=AnotherName.ttf",
            Some(include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf")),
            &[],
        )
        .await;
    assert_eq!(response_json(&duplicate)["existing"], true);
    let (_, list) = server.request("GET", "/api/fonts", None, &[]).await;
    assert_eq!(response_json(&list)["fonts"].as_array().unwrap().len(), 1);
    let (status, after) =
        server.request("POST", "/api/parts?name=lettering.svg", Some(svg), &[]).await;
    assert_eq!(status, 200, "{after}");
    assert!(response_json(&after)["warnings"].as_array().unwrap().is_empty());
    // Reimport after resolving a fallback must preserve the corrected geometry.
    assert_ne!(response_json(&before)["id"], response_json(&after)["id"]);
    let text = serde_json::json!({"value":"Lobster\nB & <O>", "size":20, "font":font});
    let (status, preview) =
        server.request("POST", "/api/text/preview", Some(&text.to_string()), &[]).await;
    assert_eq!(status, 200, "{preview}");
    assert!(response_json(&preview)["warnings"].as_array().unwrap().is_empty());
    let request = serde_json::json!({"name":"Imported font", "text":text});
    let (status, created) =
        server.request("POST", "/api/parts/text", Some(&request.to_string()), &[]).await;
    assert_eq!(status, 200, "{created}");
    assert!(response_json(&created)["warnings"].as_array().unwrap().is_empty());
    {
        use openlaser_server::coordinator::{NewRecipe, Values};
        let id: openlaser_library::Id =
            serde_json::from_value(response_json(&created)["id"].clone()).unwrap();
        let mut c = server.shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Welded lettering".into(),
                laser: openlaser_core::LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.open_part(&id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
    }
    openlaser_server::machine::prepare(&server.shared).await.unwrap();
    openlaser_server::machine::compile(&server.shared, false).await.unwrap();
    assert!(server.shared.lock().await.draft.as_ref().unwrap().compiled.is_some());
    let (status, bad) = server
        .request(
            "POST",
            "/api/text/preview",
            Some(r#"{"value":"Bo","size":10,"font":"unknown"}"#),
            &[],
        )
        .await;
    assert_eq!(status, 400, "{bad}");
    assert!(bad.contains("selected font is unavailable"));
    server.close().await;
}

/// Hold times are one setting for every screen: saved only from the times the
/// caller last read, never short enough for a tap, and published to all.
#[tokio::test]
async fn hold_times_are_shared_checked_and_published() {
    let server = Server::start().await;
    let change = |hold: [u32; 2], expected: [u32; 2]| {
        serde_json::json!({
            "hold": { "move_ms": hold[0], "zero_ms": hold[1] },
            "expected": { "move_ms": expected[0], "zero_ms": expected[1] },
        })
        .to_string()
    };
    let (status, response) =
        server.request("POST", "/api/touch", Some(&change([1500, 900], [1000, 750])), &[]).await;
    assert_eq!(status, 200, "{response}");
    let hold = server.shared.lock().await.document().hold;
    assert_eq!([hold.move_ms, hold.zero_ms], [1500, 900]);
    let stale = change([2000, 900], [1000, 750]);
    assert_eq!(server.request("POST", "/api/touch", Some(&stale), &[]).await.0, 409);
    let tap = change([100, 900], [1500, 900]);
    assert_eq!(server.request("POST", "/api/touch", Some(&tap), &[]).await.0, 400);
    assert_eq!(server.shared.lock().await.document().hold.move_ms, 1500);
    server.close().await;
}
