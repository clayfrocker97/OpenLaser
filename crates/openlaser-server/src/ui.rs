// SPDX-License-Identifier: GPL-3.0-or-later

//! The built UI, embedded in standalone builds or read from `ui/dist` for
//! development. Unknown paths return the app page so it owns its routes.

use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use std::path::{Component, Path, PathBuf};

#[cfg(feature = "embedded-ui")]
include!(concat!(env!("OUT_DIR"), "/ui-assets.rs"));

/// Where development builds read the UI. Standalone builds use embedded assets.
#[derive(Clone, Debug)]
pub struct UiDir(pub PathBuf);

/// Serves a file of the UI, or its page for an unknown path.
pub async fn serve(State(dir): State<UiDir>, uri: Uri) -> Response {
    let Some(relative) = safe_relative(uri.path()) else {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    };
    #[cfg(feature = "embedded-ui")]
    {
        let _ = dir;
        let key = relative.to_string_lossy().replace('\\', "/");
        let asset = ASSETS
            .iter()
            .find(|(name, _)| *name == key)
            .or_else(|| ASSETS.iter().find(|(name, _)| *name == "index.html"));
        let Some((name, bytes)) = asset else {
            return (StatusCode::SERVICE_UNAVAILABLE, "the embedded UI is incomplete")
                .into_response();
        };
        ([(header::CONTENT_TYPE, content_type(Path::new(name)))], Body::from(*bytes))
            .into_response()
    }
    #[cfg(not(feature = "embedded-ui"))]
    disk(&dir.0, &relative).await
}

#[cfg(not(feature = "embedded-ui"))]
async fn disk(directory: &Path, relative: &Path) -> Response {
    let candidate = directory.join(relative);
    let path = if candidate.is_file() { candidate } else { directory.join("index.html") };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            ([(header::CONTENT_TYPE, content_type(&path))], Body::from(bytes)).into_response()
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "the UI is not built: {} is missing (run `npm run build` in ui/)",
                path.display()
            ),
        )
            .into_response(),
    }
}

/// The request path as a relative file path, refusing anything that could
/// leave the directory.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return Some(PathBuf::from("index.html"));
    }
    let relative = Path::new(trimmed);
    relative.components().all(|c| matches!(c, Component::Normal(_))).then(|| relative.to_path_buf())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("json") => "application/json",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only plain relative paths reach the disk; the root is the page.
    #[test]
    fn paths_stay_inside_the_directory() {
        assert_eq!(safe_relative("/"), Some(PathBuf::from("index.html")));
        assert_eq!(safe_relative("/assets/app.js"), Some(PathBuf::from("assets/app.js")));
        assert_eq!(safe_relative("/../secret"), None);
        assert_eq!(safe_relative("/a/../../b"), None);
        assert_eq!(content_type(Path::new("x.woff2")), "font/woff2");
    }

    #[cfg(feature = "embedded-ui")]
    #[tokio::test]
    async fn standalone_serves_its_page_assets_and_licenses_without_a_ui_directory() {
        let directory = UiDir(PathBuf::from("/no-ui-directory-needed"));
        let page = serve(State(directory.clone()), Uri::from_static("/")).await;
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        let bytes = axum::body::to_bytes(page.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(html.contains("<html") && html.contains("/assets/"));
        for path in html.split('"').filter(|path| path.starts_with('/')).chain([
            "/licenses/sparrow-MIT.txt",
            "/licenses/jagua-rs-MPL-2.0.txt",
            "/licenses/OpenLaser-GPL-3.0.txt",
        ]) {
            let response = serve(State(directory.clone()), path.parse().unwrap()).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_ne!(response.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
            assert!(
                !axum::body::to_bytes(response.into_body(), 2_000_000).await.unwrap().is_empty()
            );
        }
        let route = serve(State(directory), Uri::from_static("/setup")).await;
        assert_eq!(axum::body::to_bytes(route.into_body(), 1_000_000).await.unwrap(), bytes);
    }
}
