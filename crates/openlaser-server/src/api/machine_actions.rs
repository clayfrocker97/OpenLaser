// SPDX-License-Identifier: GPL-3.0-or-later

//! Individually routed machine commands. Controller admission remains in the
//! machine workflow; this module only decodes requests and reports outcomes.

use super::{Reply, ok};
use crate::coordinator::Shared;
use crate::machine::{self, JogRequest, OutputRequest};
use crate::{Error, connect};
use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use openlaser_core::LaserMode;
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct MachineRequest {
    table: Option<machine::TableRequest>,
    pulse: Option<PulseRequest>,
    gas_test: Option<GasTestRequest>,
    gas_calibration: Option<GasCalibrationRequest>,
    preflight: Option<crate::preflight::PreflightConfirmation>,
    jog: Option<JogRequest>,
    lease: Option<openlaser_controller::lease::Lease>,
    output: Option<OutputRequest>,
    mode: Option<LaserMode>,
    alarm: Option<u32>,
    #[serde(default)]
    fast: bool,
}

#[derive(Deserialize)]
struct PulseRequest {
    duration_ms: u32,
    power: u8,
}
#[derive(Deserialize)]
struct GasTestRequest {
    selector: u8,
    pressure: f64,
    duration_ms: u64,
}

#[derive(Deserialize)]
struct GasCalibrationRequest {
    selector: u8,
    pressure: f64,
}

type Body = Option<Json<MachineRequest>>;

fn input(shared: &Shared, body: Body) -> crate::Result<MachineRequest> {
    shared.epoch()?;
    Ok(optional(body))
}

fn optional(body: Body) -> MachineRequest {
    body.map(|Json(request)| request).unwrap_or_default()
}

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/machine/connect", post(connect))
        .route("/api/machine/cancel", post(cancel))
        .route("/api/machine/disconnect", post(disconnect))
        .route("/api/machine/home", post(home))
        .route("/api/machine/calibrate", post(calibrate))
        .route("/api/machine/jog", post(jog))
        .route("/api/machine/table", post(table))
        .route("/api/machine/pulse", post(pulse))
        .route("/api/machine/gas-test", post(gas_test))
        .route("/api/machine/gas-calibration", post(gas_calibration))
        .route("/api/machine/go-origin", post(go_origin))
        .route("/api/machine/release", post(release))
        .route("/api/machine/heartbeat", post(heartbeat))
        .route("/api/machine/outputs", post(outputs))
        .route("/api/machine/mode", post(mode))
        .route("/api/machine/relieve", post(relieve))
        .route("/api/machine/run", post(run))
        .route("/api/machine/frame", post(frame))
        .route("/api/machine/resume", post(resume))
        .route("/api/machine/hold", post(hold))
        .route("/api/machine/stop", post(stop))
        .route("/api/machine/{action}", post(unknown))
}

async fn connect(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    // The attempt survives a dropped HTTP request and publishes its outcome.
    tokio::spawn(async move { connect::connect(&shared).await.ok() });
    Ok(ok())
}

async fn cancel(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    connect::cancel(&shared).await;
    Ok(ok())
}

async fn disconnect(State(shared): State<Shared>, body: Body) -> Reply {
    let _ = optional(body);
    machine::disconnect(&shared).await?;
    Ok(ok())
}

async fn home(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    machine::home(&shared).await?;
    Ok(ok())
}

async fn calibrate(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    machine::calibrate(&shared).await?;
    Ok(ok())
}

async fn jog(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let jog =
        request.jog.ok_or_else(|| Error::Request("a jog needs axis, direction and step".into()))?;
    machine::jog(&shared, jog, request.lease).await?;
    Ok(ok())
}

async fn table(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let table = request
        .table
        .ok_or_else(|| Error::Request("a table jog needs direction and speed".into()))?;
    let lease =
        request.lease.ok_or_else(|| Error::Request("a table jog needs a press identity".into()))?;
    machine::table(&shared, table, lease).await?;
    Ok(ok())
}

async fn pulse(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let pulse =
        request.pulse.ok_or_else(|| Error::Request("a pulse needs duration and power".into()))?;
    machine::pulse(&shared, pulse.duration_ms, pulse.power).await?;
    Ok(ok())
}

async fn gas_test(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let test = request
        .gas_test
        .ok_or_else(|| Error::Request("a gas test needs selector, pressure and duration".into()))?;
    machine::gas_test(&shared, test.selector, test.pressure, test.duration_ms).await?;
    Ok(ok())
}

async fn gas_calibration(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let test = request
        .gas_calibration
        .ok_or_else(|| Error::Request("a gas flow test needs selector and pressure".into()))?;
    machine::gas_calibration(&shared, test.selector, test.pressure).await?;
    Ok(ok())
}

async fn go_origin(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    machine::go_origin(&shared, request.fast).await?;
    Ok(ok())
}

async fn release(State(shared): State<Shared>, body: Body) -> Reply {
    let request = optional(body);
    let lease =
        request.lease.ok_or_else(|| Error::Request("release needs a press identity".into()))?;
    machine::release(&shared, lease).await?;
    Ok(ok())
}

async fn heartbeat(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let lease =
        request.lease.ok_or_else(|| Error::Request("heartbeat needs a press identity".into()))?;
    machine::heartbeat(&shared, lease)?;
    Ok(ok())
}

async fn outputs(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let output = request.output.ok_or_else(|| Error::Request("an output needs a kind".into()))?;
    let lease =
        request.lease.ok_or_else(|| Error::Request("an output needs a press identity".into()))?;
    machine::outputs(&shared, output, lease).await?;
    Ok(ok())
}

async fn mode(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    let mode = request.mode.ok_or_else(|| Error::Request("a mode switch needs a mode".into()))?;
    machine::switch_mode(&shared, mode).await?;
    Ok(ok())
}

async fn relieve(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    machine::relieve(&shared, request.alarm)?;
    Ok(ok())
}

async fn run(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    machine::run_reviewed(&shared, request.preflight.as_ref()).await?;
    Ok(ok())
}

async fn frame(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    machine::frame(&shared).await?;
    Ok(ok())
}

async fn resume(State(shared): State<Shared>, body: Body) -> Reply {
    let request = input(&shared, body)?;
    machine::resume_reviewed(&shared, request.preflight.as_ref()).await?;
    Ok(ok())
}

async fn hold(State(shared): State<Shared>, body: Body) -> Reply {
    input(&shared, body)?;
    machine::hold(&shared).await?;
    Ok(ok())
}

async fn stop(State(shared): State<Shared>, body: Body) -> Reply {
    let _ = optional(body);
    machine::stop(&shared).await?;
    Ok(ok())
}

async fn unknown(State(shared): State<Shared>, Path(action): Path<String>, body: Body) -> Reply {
    input(&shared, body)?;
    Err(Error::Missing(format!("no machine action {action}")).into())
}
