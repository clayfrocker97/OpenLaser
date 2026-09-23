// SPDX-License-Identifier: GPL-3.0-or-later

//! Every LAN screen can open the workspace; one page at a time owns commands.

use crate::coordinator::Shared;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, watch};

const CONTROL_TTL: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct Access {
    state: Arc<Mutex<Session>>,
    commands: Arc<Mutex<()>>,
    shared: Shared,
    address: String,
    discovery: watch::Receiver<crate::lan::Status>,
}

struct Session {
    owner: Option<Owner>,
    initialized: bool,
}

struct Owner {
    peer: IpAddr,
    page: String,
    name: String,
    until: Instant,
}

struct Identity {
    peer: IpAddr,
    page: String,
    name: String,
    previous: Option<String>,
}

fn identity(headers: &HeaderMap, peer: SocketAddr) -> Option<Identity> {
    let local = local(headers, peer);
    let page = headers
        .get("x-openlaser-client")
        .and_then(|h| h.to_str().ok())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
        .or(if local { Some("local-tool") } else { None })?;
    let name = headers
        .get("x-openlaser-name")
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|s| {
            !s.is_empty() && s.len() <= 48 && s.bytes().all(|c| c.is_ascii_graphic() || c == b' ')
        })
        .unwrap_or(if local { "Machine screen" } else { "Network screen" });
    Some(Identity {
        peer: peer.ip(),
        page: page.into(),
        name: name.into(),
        previous: headers
            .get("x-openlaser-previous")
            .and_then(|h| h.to_str().ok())
            .filter(|s| s.len() <= 64)
            .map(str::to_owned),
    })
}

impl Session {
    fn owns(&self, identity: &Identity) -> bool {
        self.owner.as_ref().is_some_and(|owner| {
            owner.peer == identity.peer
                && owner.page == identity.page
                && owner.until > Instant::now()
        })
    }

    fn assign(&mut self, identity: &Identity) {
        self.owner = Some(Owner {
            peer: identity.peer,
            page: identity.page.clone(),
            name: identity.name.clone(),
            until: Instant::now() + CONTROL_TTL,
        });
        self.initialized = true;
    }

    fn refresh(&mut self, identity: &Identity) {
        if let Some(owner) = &mut self.owner
            && owner.peer == identity.peer
            && owner.page == identity.page
        {
            owner.until = Instant::now() + CONTROL_TTL;
            owner.name.clone_from(&identity.name);
        }
    }

    fn available(&self) -> bool {
        self.owner.as_ref().is_none_or(|owner| owner.until <= Instant::now())
    }

    fn returns(&self, identity: &Identity) -> bool {
        self.owner.as_ref().is_some_and(|owner| {
            owner.peer == identity.peer && identity.previous.as_deref() == Some(owner.page.as_str())
        })
    }
}

/// Coordinates operator handoff without accounts, pairing or cookies.
pub(crate) fn protect(
    app: Router,
    shared: Shared,
    listen: SocketAddr,
    discovery: watch::Receiver<crate::lan::Status>,
) -> Router {
    let address = if listen.ip().is_loopback() {
        format!("http://{listen}")
    } else if listen.port() == 80 {
        "http://openlaser.local".into()
    } else {
        format!("http://openlaser.local:{}", listen.port())
    };
    let access = Access {
        state: Arc::new(Mutex::new(Session { owner: None, initialized: false })),
        shared,
        commands: Arc::new(Mutex::new(())),
        discovery,
        address,
    };
    let routes = Router::new()
        .route("/api/access", get(status))
        .route("/api/access/control", post(control))
        .route("/api/access/release", post(release))
        .route("/api/access/shutdown", post(shutdown))
        .with_state(access.clone())
        .layer(axum::middleware::from_fn(crate::api::browser::guard));
    app.merge(routes).layer(axum::middleware::from_fn_with_state(access, guard))
}

async fn guard(State(access): State<Access>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let stopping =
        matches!(path, "/api/machine/stop" | "/api/machine/release" | "/api/machine/hold");
    if !path.starts_with("/api/")
        || path == "/api/access"
        || path.starts_with("/api/access/")
        || request.method() == Method::GET
        || stopping
    {
        return next.run(request).await;
    }
    let Some(peer) = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|p| p.0) else {
        return refused(StatusCode::BAD_REQUEST, "the server could not identify this connection");
    };
    let Some(identity) = identity(request.headers(), peer) else {
        return refused(StatusCode::BAD_REQUEST, "this page needs a control identity");
    };
    let epoch = match access.shared.ensure_running() {
        Ok(epoch) => epoch,
        Err(error) => return refused(StatusCode::CONFLICT, &error.to_string()),
    };
    // Renewal validates the same owner, but must never wait for an upload or
    // another long request before reaching the controller's lease channel.
    let command = if path == "/api/machine/heartbeat" {
        None
    } else {
        Some(access.commands.clone().lock_owned().await)
    };
    if access.shared.ensure_running().ok() != Some(epoch) {
        return refused(StatusCode::CONFLICT, "Stop cancelled this pending command.");
    }
    let mut state = access.state.lock().await;
    if !state.initialized {
        state.assign(&identity);
    }
    if !state.owns(&identity) {
        return refused(
            StatusCode::CONFLICT,
            "This screen is watching. Take control before making changes.",
        );
    }
    state.refresh(&identity);
    drop(state);
    // The admitted request owns both its workflow cleanup and the command
    // gate. Losing the browser cannot abandon a reservation or permit handoff
    // while that work is still finishing. Stop keeps its independent path.
    match tokio::spawn(async move {
        let response = next.run(request).await;
        drop(command);
        response
    })
    .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, "command task failed");
            refused(StatusCode::INTERNAL_SERVER_ERROR, "The command could not finish.")
        }
    }
}

async fn status(
    State(access): State<Access>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let mut state = access.state.lock().await;
    let identity = headers.get("x-openlaser-client").and_then(|_| identity(&headers, peer));
    if let Some(identity) = &identity {
        if !state.initialized {
            state.assign(identity);
        }
        if state.returns(identity)
            && let Ok(_command) = access.commands.try_lock()
        {
            state.assign(identity);
        }
        state.refresh(identity);
    }
    let available = state.available();
    drop(state);
    // An idle, unclaimed server opens directly without a takeover popup.
    if available
        && let Some(identity) = &identity
        && let Ok(_command) = access.commands.try_lock()
    {
        let coordinator = access.shared.lock().await;
        let mut state = access.state.lock().await;
        if state.available()
            && coordinator.operation.is_none()
            && coordinator.machine.state().operation.is_none()
        {
            state.assign(identity);
        }
    }
    let state = access.state.lock().await;
    let owner = state.owner.as_ref().filter(|o| o.until > Instant::now());
    let body = json!({
        "can_control": identity.as_ref().is_some_and(|i| state.owns(i)),
        "owner": owner.map(|o| &o.name), "address": access.address,
        "discovery": *access.discovery.borrow(),
    });
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

async fn control(
    State(access): State<Access>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let Some(identity) = identity(&headers, peer) else {
        return refused(StatusCode::BAD_REQUEST, "this page needs a control identity");
    };
    let _command = access.commands.lock().await;
    let coordinator = access.shared.lock().await;
    if coordinator.operation.is_some() || coordinator.machine.state().operation.is_some() {
        return refused(
            StatusCode::CONFLICT,
            "Wait for motion to finish or stop it before taking control.",
        );
    }
    access.state.lock().await.assign(&identity);
    Json(json!({"ok":true})).into_response()
}

async fn release(
    State(access): State<Access>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let mut state = access.state.lock().await;
    if let Some(identity) = identity(&headers, peer)
        && state.owns(&identity)
    {
        state.owner = None;
    }
    Json(json!({"ok":true})).into_response()
}

async fn shutdown(
    State(access): State<Access>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !local(&headers, peer) {
        return refused(StatusCode::FORBIDDEN, "Stop the server from the machine computer.");
    }
    access.shared.closing.send_replace(true);
    Json(json!({"ok":true})).into_response()
}

fn local(headers: &HeaderMap, peer: SocketAddr) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|host| {
            let name = host.host().trim_matches(['[', ']']);
            name.trim_end_matches('.').eq_ignore_ascii_case("localhost")
                || name.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        })
}

fn refused(status: StatusCode, message: &str) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(json!({"error":message}))).into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "literal HTTP peer fixtures")]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn returning_to_the_owning_page_renews_an_expired_claim() {
        let identity = Identity {
            peer: "192.0.2.1".parse().unwrap(),
            page: "phone".into(),
            name: "My phone".into(),
            previous: None,
        };
        let mut session = Session { owner: None, initialized: false };
        session.assign(&identity);
        session.owner.as_mut().unwrap().until =
            Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        assert!(session.available());
        session.refresh(&identity);
        assert!(session.owns(&identity));
        assert!(!session.available());
        let mut returned = Identity {
            peer: "192.0.2.2".parse().unwrap(),
            page: "reloaded".into(),
            name: "My phone".into(),
            previous: Some(identity.page.clone()),
        };
        assert!(!session.returns(&returned), "another computer cannot continue this page");
        returned.peer = identity.peer;
        assert!(session.returns(&returned));
        session.assign(&returned);
        assert!(session.owns(&returned));
        assert!(!session.owns(&identity));
        assert!(!session.returns(&returned), "a previous page is consumed only once");
    }

    #[test]
    fn server_shutdown_requires_both_loopback_peer_and_loopback_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:80"));
        assert!(local(&headers, "127.0.0.1:4000".parse().unwrap()));
        assert!(!local(&headers, "192.0.2.1:4000".parse().unwrap()));
        headers.insert(header::HOST, HeaderValue::from_static("openlaser.local"));
        assert!(!local(&headers, "127.0.0.1:4000".parse().unwrap()));
        headers.insert(header::HOST, HeaderValue::from_static("[::1]:80"));
        assert!(local(&headers, "[::1]:4000".parse().unwrap()));
    }
}
