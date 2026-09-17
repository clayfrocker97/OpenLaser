// SPDX-License-Identifier: GPL-3.0-or-later

//! Uses the shared Windows webview; its small bootstrapper runs only if absent.

use std::path::Path;

pub fn ensure(directory: &Path) -> Result<(), String> {
    if wry::webview_version().is_ok() {
        return Ok(());
    }
    let installer = directory.join("MicrosoftEdgeWebview2Setup.exe");
    std::fs::write(&installer, include_bytes!("../../assets/MicrosoftEdgeWebview2Setup.exe"))
        .map_err(|e| format!("prepare the browser runtime: {e}"))?;
    let result = crate::desktop::powershell()
        .args(["-Command", include_str!("runtime.ps1")])
        .env("OPENLASER_RUNTIME_INSTALLER", &installer)
        .status()
        .map_err(|e| format!("start browser setup: {e}"));
    let _ = std::fs::remove_file(installer);
    let status = result?;
    if !status.success() || wry::webview_version().is_err() {
        return Err("OpenLaser needs Microsoft's WebView2 Runtime. Connect to the internet and reopen OpenLaser to finish its one-time setup.".to_owned());
    }
    Ok(())
}
