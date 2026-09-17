// SPDX-License-Identifier: GPL-3.0-or-later

//! The same local interface hosted by each operating system's webview.

use std::cell::RefCell;
use std::rc::Rc;
use tao::window::Window;
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

pub fn build(
    window: &Window,
    context: &mut WebContext,
    origin: Rc<RefCell<String>>,
) -> Result<WebView, String> {
    let builder = WebViewBuilder::new_with_web_context(context)
        .with_html(page("Opening OpenLaser", "Your workspace is loading."))
        .with_background_color((245, 246, 242, 255))
        .with_clipboard(true)
        .with_devtools(false)
        .with_navigation_handler(move |url| {
            if internal(&url, &origin.borrow()) {
                true
            } else {
                external(&url);
                false
            }
        })
        .with_new_window_req_handler(|url, _| {
            external(&url);
            NewWindowResponse::Deny
        });
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let container = window.default_vbox().ok_or("the window has no GTK container")?;
        builder.build_gtk(container)
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(window);
    webview.map_err(|e| e.to_string())
}

fn internal(url: &str, origin: &str) -> bool {
    url == "about:blank" || (!origin.is_empty() && url.starts_with(origin))
}

fn external(url: &str) {
    let Ok(uri) = url.parse::<wry::http::Uri>() else { return };
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
        return;
    }
    #[cfg(windows)]
    let launched = crate::desktop::powershell()
        .args(["-Command", "Start-Process -FilePath $env:OPENLASER_OPEN_URL"])
        .env("OPENLASER_OPEN_URL", url)
        .spawn();
    #[cfg(target_os = "macos")]
    let launched = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let launched = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = launched {
        tracing::warn!(%error, %url, "could not open the link");
    }
}

pub fn page(title: &str, message: &str) -> String {
    let logo = include_str!("../../../../ui/static/logo.svg");
    format!(
        "<!doctype html><html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>OpenLaser</title><style>html{{background:#f5f6f2;color:#1c211b;font:16px system-ui}}body{{min-height:90vh;display:grid;place-items:center;margin:0}}main{{width:min(560px,80vw)}}svg{{width:200px;height:auto;margin-bottom:48px}}h1{{font-size:24px;font-weight:600}}p{{line-height:1.7;white-space:pre-wrap;overflow-wrap:anywhere;color:#5b6258}}</style><main>{logo}<h1>{}</h1><p>{}</p></main></html>",
        escape(title),
        escape(message)
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_keeps_the_exact_app_origin() {
        let origin = "http://127.0.0.1:8080/";
        assert!(internal("about:blank", ""));
        assert!(internal("http://127.0.0.1:8080/setup", origin));
        assert!(!internal("https://example.com", origin));
        assert!(!internal("http://127.0.0.1:8080.evil.example/", origin));
        assert!(!internal("http://127.0.0.1:8081/", origin));
        assert!(!internal("https://example.com", ""));
        assert!(!page("Error", "<script>run()</script>").contains("<script>"));
    }
}
