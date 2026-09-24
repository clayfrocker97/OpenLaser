// SPDX-License-Identifier: GPL-3.0-or-later

//! The coordinator, HTTP commands and server-sent event stream.
//!
//! Parts, Setup and Run as one workflow: a draft is prepared and compiled
//! from its parts, a recipe and machining features; running binds the compiled
//! job to the head's live position and streams it; a hold keeps the
//! checkpoint a continuation resumes from. Every enabling action is
//! admitted here against the controller's live state; the UI is advisory.
//! The static UI is served from the same process.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "tests build known inputs")
)]

mod access;
pub mod alarm_history;
pub mod api;
pub mod bindings;
pub mod connect;
pub mod coordinator;
pub mod correction;
mod correspondence;
pub mod defaults;
mod display_path;
pub mod document;
pub mod draft;
mod envelope;
mod font_store;
pub mod gas;
mod imports;
pub mod inventory;
pub mod job_parts;
mod lan;
pub mod machine;
mod machine_files;
pub mod nesting;
pub mod parameter_settings;
pub mod placement;
pub mod postflight;
pub mod preflight;
pub mod recipes;
pub mod resume;
pub mod setup;
pub mod sheet_sizes;
pub mod sheets;
pub mod soft_settings;
pub mod stock_store;
pub mod touch;
pub mod ui;
pub mod workspace;

pub use coordinator::{Config, Coordinator};

/// Serves until Ctrl-C, SIGTERM, or a listener failure, then awaits cleanup.
pub async fn serve(
    shared: coordinator::Shared,
    listen: std::net::SocketAddr,
) -> std::result::Result<(), String> {
    serve_until(shared, listen, shutdown_signal()).await
}

/// Serves with the normal shutdown lifecycle, notifying the caller after bind.
pub async fn serve_ready(
    shared: coordinator::Shared,
    listen: std::net::SocketAddr,
    ready: impl FnOnce(std::net::SocketAddr),
) -> std::result::Result<(), String> {
    serve_until_ready(shared, listen, shutdown_signal(), ready).await
}

/// The same lifecycle with an injected shutdown signal, used by loopback tests.
pub async fn serve_until(
    shared: coordinator::Shared,
    listen: std::net::SocketAddr,
    signal: impl std::future::Future<Output = ()>,
) -> std::result::Result<(), String> {
    serve_until_ready(shared, listen, signal, |_| {}).await
}

/// Serves with a caller's shutdown signal and reports the bound address.
/// A specific LAN binding also gets a loopback listener for the machine console.
pub async fn serve_until_ready(
    shared: coordinator::Shared,
    listen: std::net::SocketAddr,
    signal: impl std::future::Future<Output = ()>,
    ready: impl FnOnce(std::net::SocketAddr),
) -> std::result::Result<(), String> {
    let listener = match tokio::net::TcpListener::bind(listen).await {
        Ok(listener) => listener,
        Err(error) => {
            let cleanup = shutdown(&shared).await;
            return Err(format!(
                "listen on {listen}: {error}{}",
                cleanup.err().map_or(String::new(), |e| format!("; {e}"))
            ));
        }
    };
    let address = listener.local_addr().unwrap_or(listen);
    let console = if !address.ip().is_loopback() && !address.ip().is_unspecified() {
        let ip = if address.is_ipv4() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        };
        match tokio::net::TcpListener::bind(std::net::SocketAddr::new(ip, address.port())).await {
            Ok(listener) => Some(listener),
            Err(error) => {
                let _ = shutdown(&shared).await;
                return Err(format!("listen for the local console: {error}"));
            }
        }
    } else {
        None
    };
    let config = shared.lock().await.config.clone();
    let discovery = lan::Discovery::start(address, config.machine.host);
    let ui_dir = config.ui_dir;
    let app = access::protect(
        api::router(shared.clone(), ui_dir.clone()),
        shared.clone(),
        address,
        discovery.status.clone(),
    );
    ready(address);
    tracing::info!(%listen, ui = %ui_dir.display(), "serving");
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let serving = async {
        let primary = serve_listener(listener, app.clone(), stopped.clone());
        let local = async {
            match console {
                Some(listener) => serve_listener(listener, app, stopped).await,
                None => Ok(()),
            }
        };
        tokio::try_join!(primary, local).map(|_| ())
    };
    tokio::pin!(serving);
    let mut closing = shared.closing.subscribe();
    let result = tokio::select! {
        result = &mut serving => Some(result),
        () = signal => None,
        () = shutdown_signal() => None,
        _ = closing.wait_for(|closing| *closing) => None,
    };
    let cleanup = shutdown(&shared).await;
    stop.send_replace(true);
    let served = match result {
        Some(result) => result,
        None => match tokio::time::timeout(HTTP_DRAIN_TIMEOUT, serving).await {
            Ok(result) => result,
            Err(_) => Err("HTTP requests did not drain after machine shutdown".to_owned()),
        },
    };
    match (served, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; {cleanup}")),
    }
}

async fn serve_listener(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    mut stopped: tokio::sync::watch::Receiver<bool>,
) -> std::result::Result<(), String> {
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        })
        .await
        .map_err(|error| error.to_string())
}

/// How long HTTP requests may take to finish once the machine has shut
/// down, before the server gives up on them.
const HTTP_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long the controller task may take to switch outputs off and close
/// the link at shutdown before its output state is reported uncertain.
const MACHINE_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Stops admission before asking the socket task to clean up and exit.
pub async fn shutdown(shared: &coordinator::Shared) -> std::result::Result<(), String> {
    use std::sync::atomic::Ordering;
    shared.closing.send_replace(true);
    shared.stop_epoch.fetch_add(1, Ordering::AcqRel);
    let result =
        match tokio::time::timeout(MACHINE_SHUTDOWN_TIMEOUT, shared.machine.shutdown()).await {
            Ok(result) => result.map_err(|e| format!("machine cleanup is uncertain: {e}")),
            Err(_) => Err("machine cleanup timed out; output state is uncertain".to_owned()),
        };
    if let Err(error) = &result {
        tracing::error!(%error);
    }
    let history = shared.lock().await.history.clone();
    let recorded = history.flush().await.map_err(|e| e.to_string());
    let drafts = workspace::flush(shared).await.map_err(|e| e.to_string());
    let errors: Vec<_> =
        [result, recorded, drafts].into_iter().filter_map(std::result::Result::err).collect();
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        let Ok(mut terminate) = terminate else {
            tracing::error!("could not install the shutdown signal handler");
            return;
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => if let Err(error) = result { tracing::error!(%error, "Ctrl-C handler"); },
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "Ctrl-C handler");
    }
}

/// Why the server refused a request.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The request itself is wrong.
    #[error("{0}")]
    Request(String),
    /// The machine or the library refused.
    #[error("{0}")]
    Refused(String),
    /// Nothing to act on.
    #[error("{0}")]
    Missing(String),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

impl From<openlaser_library::Error> for Error {
    fn from(error: openlaser_library::Error) -> Self {
        match error {
            openlaser_library::Error::Missing { .. } => Self::Missing(error.to_string()),
            openlaser_library::Error::Invalid(_) => Self::Request(error.to_string()),
            _ => Self::Refused(error.to_string()),
        }
    }
}

impl From<openlaser_controller::Error> for Error {
    fn from(error: openlaser_controller::Error) -> Self {
        Self::Refused(error.to_string())
    }
}

impl From<openlaser_xml::Error> for Error {
    fn from(error: openlaser_xml::Error) -> Self {
        match error {
            openlaser_xml::Error::Missing(field) => Self::Refused(format!(
                "the machine files have no {field}; import the full machine backup on the Settings page"
            )),
            other => Self::Refused(other.to_string()),
        }
    }
}

impl From<openlaser_compiler::Error> for Error {
    fn from(error: openlaser_compiler::Error) -> Self {
        Self::Refused(error.to_string())
    }
}

impl From<openlaser_prep::Error> for Error {
    fn from(error: openlaser_prep::Error) -> Self {
        Self::Refused(error.to_string())
    }
}
