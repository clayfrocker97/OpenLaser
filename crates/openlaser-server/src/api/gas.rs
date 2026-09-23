// SPDX-License-Identifier: GPL-3.0-or-later

//! Gas costs: saving prices, pricing a flow test, saving its calibration
//! and reading a job's run history. The prices themselves are published in
//! the state document.

use super::{Reply, ok};
use crate::Error;
use crate::coordinator::Shared;
use crate::gas::{CalibrationRequest, FlowQuery, GasCosts};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/gas/costs", post(save_costs))
        .route("/api/gas/flow", post(flow))
        .route("/api/gas/calibrate", post(calibrate))
        .route("/api/gas/runs", get(runs))
}

fn value(value: impl serde::Serialize) -> Reply {
    Ok(Json(serde_json::to_value(value).map_err(|e| Error::Refused(e.to_string()))?))
}

/// New prices, from the ones the caller last read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CostsChange {
    costs: GasCosts,
    expected: GasCosts,
}

async fn save_costs(State(shared): State<Shared>, Json(change): Json<CostsChange>) -> Reply {
    let mut coordinator = shared.lock().await;
    coordinator.gas.save(change.costs, &change.expected)?;
    coordinator.publish();
    Ok(ok())
}

async fn flow(State(shared): State<Shared>, Json(query): Json<FlowQuery>) -> Reply {
    value(shared.lock().await.gas.flow(&query)?)
}

async fn calibrate(State(shared): State<Shared>, Json(request): Json<CalibrationRequest>) -> Reply {
    let mut coordinator = shared.lock().await;
    let calibration = coordinator.gas.calibrate(&request)?;
    coordinator.publish();
    value(calibration)
}

#[derive(Deserialize)]
struct RunsQuery {
    key: String,
    job: Option<String>,
}

async fn runs(State(shared): State<Shared>, Query(query): Query<RunsQuery>) -> Reply {
    value(shared.lock().await.gas.runs(&query.key, query.job.as_deref()))
}
