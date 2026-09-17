// SPDX-License-Identifier: GPL-3.0-or-later

//! User-data paths and platform launch helpers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

pub fn default_config() -> PathBuf {
    if cfg!(windows) || cfg!(feature = "desktop") {
        if let Ok(executable) = std::env::current_exe()
            && let Some(directory) = executable.parent()
        {
            let portable = directory.join("machine.toml");
            if portable.is_file() {
                return portable;
            }
        }
        if let Some(directory) = user_data() {
            return directory.join("machine.toml");
        }
    }
    PathBuf::from("machine.toml")
}

pub fn default_data(config: &Path, explicit: bool) -> PathBuf {
    let beside = config.parent().unwrap_or_else(|| Path::new(""));
    if !explicit
        && (cfg!(windows) || cfg!(feature = "desktop"))
        && let Some(directory) = user_data()
        && config == directory.join("machine.toml")
    {
        return directory;
    }
    beside.join("data")
}

fn user_data() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let directory = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("OpenLaser"))
    } else if cfg!(target_os = "macos") {
        home.map(|p| p.join("Library/Application Support/OpenLaser"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.map(|p| p.join(".local/share")))
            .map(|p| p.join("openlaser"))
    };
    directory.filter(|p| p.is_absolute())
}

pub fn browser_url(mut listen: SocketAddr) -> String {
    if listen.ip().is_unspecified() {
        listen.set_ip(match listen.ip() {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        });
    }
    if listen.port() == 80 {
        match listen.ip() {
            IpAddr::V4(ip) => format!("http://{ip}/"),
            IpAddr::V6(ip) => format!("http://[{ip}]/"),
        }
    } else {
        format!("http://{listen}/")
    }
}

pub fn ready(listen: SocketAddr, open: bool) {
    let url = browser_url(listen);
    tracing::info!(%url, "OpenLaser is ready");
    #[cfg(windows)]
    if open {
        // The URL comes only from a parsed IP address and numeric port.
        // Windows `start` opens URLs using the default browser association.
        let launched = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "start", "", &url])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Err(error) = launched {
            tracing::warn!(%error, %url, "open this address in your browser");
        }
    }
    #[cfg(not(windows))]
    let _ = open;
}

pub fn report_error(error: &str) {
    #[cfg(all(windows, feature = "desktop"))]
    {
        let _ = powershell()
            .args(["-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show($env:OPENLASER_ERROR, 'OpenLaser', 'OK', 'Error') | Out-Null"])
            .env("OPENLASER_ERROR", error)
            .status();
    }
    #[cfg(not(all(windows, feature = "desktop")))]
    let _ = error;
}

#[cfg(all(windows, feature = "desktop"))]
pub fn powershell() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive"]);
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_config_keeps_its_portable_data_location() {
        let base = std::env::temp_dir().join("portable");
        assert_eq!(default_data(&base.join("machine.toml"), true), base.join("data"));
        if (cfg!(windows) || cfg!(feature = "desktop"))
            && let Some(directory) = user_data()
        {
            assert_eq!(default_data(&directory.join("machine.toml"), false), directory);
        }
    }

    #[test]
    fn browser_uses_a_connectable_address_and_retains_ipv6_brackets() {
        assert_eq!(browser_url(SocketAddr::from(([0, 0, 0, 0], 8080))), "http://127.0.0.1:8080/");
        assert_eq!(browser_url(SocketAddr::from(([127, 0, 0, 1], 9876))), "http://127.0.0.1:9876/");
        assert_eq!(
            browser_url(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 8080))),
            "http://[::1]:8080/"
        );
        assert_eq!(browser_url(SocketAddr::from(([0, 0, 0, 0], 80))), "http://127.0.0.1/");
        assert_eq!(browser_url(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 80))), "http://[::1]/");
    }
}
