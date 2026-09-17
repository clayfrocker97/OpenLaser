// SPDX-License-Identifier: GPL-3.0-or-later

//! The local HTTP interface must not accept commands from unrelated websites.
//! Native windows and the development proxy both use an ordinary HTTP origin.

use axum::Json;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header, uri::Authority};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub(crate) async fn guard(request: Request, next: Next) -> Response {
    if !allowed(request.headers()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "use OpenLaser from its own address" })),
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    response.headers_mut().insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    response
}

fn authority(headers: &HeaderMap) -> Option<Authority> {
    let host: Authority = headers.get(header::HOST)?.to_str().ok()?.parse().ok()?;
    if host.as_str().contains('@') {
        return None;
    }
    let name = host.host().trim_matches(['[', ']']);
    // Literal addresses support both loopback and explicitly bound LAN use.
    // Arbitrary DNS names would allow a rebinding site to pass the origin test.
    let known = name.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || name.trim_end_matches('.').eq_ignore_ascii_case("openlaser.local")
        || name.parse::<std::net::IpAddr>().is_ok();
    known.then_some(host)
}

fn allowed(headers: &HeaderMap) -> bool {
    let Some(host) = authority(headers) else { return false };
    match headers.get(header::ORIGIN) {
        Some(origin) => origin.to_str().ok().is_some_and(|origin| same_origin(origin, &host)),
        None => same_site(headers),
    }
}

fn same_origin(origin: &str, host: &Authority) -> bool {
    let Ok(origin) = origin.parse::<Uri>() else { return false };
    origin.scheme_str() == Some("http")
        && origin.authority().is_some_and(|other| {
            other.host().eq_ignore_ascii_case(host.host())
                && other.port_u16().unwrap_or(80) == host.port_u16().unwrap_or(80)
                && !other.as_str().contains('@')
        })
        && origin.path_and_query().is_none_or(|path| path.as_str() == "/")
}

fn same_site(headers: &HeaderMap) -> bool {
    // Nonbrowser tools may omit these headers. Browsers cannot forge them.
    headers.get("sec-fetch-site").is_none_or(|site| site == "same-origin" || site == "none")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, origin: Option<&str>, site: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        if let Some(site) = site {
            headers.insert("sec-fetch-site", site.parse().unwrap());
        }
        headers
    }

    #[test]
    fn native_development_proxy_ipv6_and_nonbrowser_clients_are_supported() {
        for host in [
            "127.0.0.1:8080",
            "localhost:5173",
            "[::1]:8080",
            "192.168.2.10:8080",
            "openlaser.local",
            "OPENLASER.local:80",
        ] {
            assert!(allowed(&headers(host, Some(&format!("http://{host}")), Some("same-origin"))));
            assert!(allowed(&headers(host, None, None)));
            assert!(allowed(&headers(host, None, Some("none"))));
        }
        assert!(allowed(&headers("localhost:80", Some("http://localhost"), Some("same-origin"))));
    }

    #[test]
    fn cross_site_null_malformed_and_rebound_origins_are_refused() {
        for origin in [
            "https://unrelated.example",
            "null",
            "http://127.0.0.1:9000",
            "https://127.0.0.1:8080",
            "http://127.0.0.1:8080/path",
            "http://127.0.0.1:8080/?x=1",
        ] {
            assert!(!allowed(&headers("127.0.0.1:8080", Some(origin), Some("cross-site"))));
        }
        assert!(!allowed(&headers(
            "unrelated.example",
            Some("http://unrelated.example"),
            Some("same-origin")
        )));
        assert!(!allowed(&headers("user@127.0.0.1:8080", None, None)));
        assert!(!allowed(&headers("127.0.0.1:8080", None, Some("same-site"))));
        assert!(!allowed(&headers("127.0.0.1:8080", None, Some("cross-site"))));
        assert!(!allowed(&HeaderMap::new()));
    }
}
