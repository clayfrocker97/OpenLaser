// SPDX-License-Identifier: GPL-3.0-or-later

//! The OpenLaser executable.
//!
//! ```text
//! openlaser [--config machine.toml] [--data DIR] [--listen ADDR] [--lan] [--simulate] [--no-browser] [--shutdown]
//! ```
//!
//! `machine.toml` beside the executable says where the controller is and
//! which vendor files describe the machine. `--simulate` starts the
//! loopback simulator and points the controller at it. Machine setup and
//! movement use the same admitted server workflow.

#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]

mod cli;
mod desktop;
mod service;
use cli::{Arguments, CommandLine};
#[cfg(feature = "desktop")]
mod native;

use openlaser_controller::{Config as MachineConfig, Simulator};
use openlaser_core::LaserMode;
use openlaser_server::bindings::Files;
use openlaser_server::coordinator::Config;
use serde::Deserialize;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The file beside the executable.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MachineFile {
    /// The controller card, `10.1.1.168:502` by default.
    controller: Option<String>,
    /// The host address on the machine's segment, `10.1.1.10` by default.
    host: Option<String>,
    /// Explicit UI listener; the desktop defaults to LAN HTTP on port 80.
    listen: Option<String>,
    /// Publish openlaser.local on the shop LAN, using HTTP port 80 by default.
    lan: bool,
    /// The library directory, `data` beside the file by default.
    data: Option<PathBuf>,
    /// The machine backup.
    backup: Option<PathBuf>,
    /// `fiber` or `co2`; the files' saved mode by default.
    mode: Option<String>,
    /// Whether the CO2 height controller is bypassed, independently of optical focus.
    co2_manual_focus: Option<bool>,
    /// The development UI directory. Ignored when the UI is embedded.
    ui: Option<PathBuf>,
}

fn read_machine_file(path: &Path, required: bool) -> Result<MachineFile, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
            Ok(MachineFile::default())
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// The server configuration from the file and the command line, and the
/// simulator when one was asked for.
async fn configure(arguments: &Arguments) -> Result<(Config, Option<Simulator>), String> {
    let file = read_machine_file(&arguments.config, arguments.config_explicit)?;
    let beside = arguments.config.parent().map(Path::to_path_buf).unwrap_or_default();
    let relative =
        |path: &PathBuf| if path.is_absolute() { path.clone() } else { beside.join(path) };
    let (machine, simulator) = if arguments.simulate {
        let simulator = Simulator::start().await.map_err(|e| format!("simulator: {e}"))?;
        (simulator.config(), Some(simulator))
    } else {
        let endpoint: SocketAddrV4 = file
            .controller
            .as_deref()
            .unwrap_or("10.1.1.168:502")
            .parse()
            .map_err(|_| "controller must be an IPv4 address and port")?;
        let host: Ipv4Addr = file
            .host
            .as_deref()
            .unwrap_or("10.1.1.10")
            .parse()
            .map_err(|_| "host must be an IPv4 address")?;
        let mut config = MachineConfig::machine();
        config.endpoint = endpoint;
        config.host = host;
        (config, None)
    };
    let listen: SocketAddr = arguments
        .listen
        .as_deref()
        .or_else(|| arguments.lan.then_some("0.0.0.0:80"))
        .or(file.listen.as_deref())
        .unwrap_or(
            if arguments.lan
                || file.lan
                || (cfg!(feature = "desktop") && !arguments.no_browser && !arguments.simulate)
            {
                "0.0.0.0:80"
            } else {
                "127.0.0.1:8080"
            },
        )
        .parse()
        .map_err(|_| "listen must be an address and port")?;
    let mode = match file.mode.as_deref() {
        None => None,
        Some("fiber") => Some(LaserMode::Fiber),
        Some("co2") => Some(LaserMode::Co2),
        Some(other) => return Err(format!("mode must be fiber or co2, not {other}")),
    };
    let config = Config {
        listen,
        data_dir: arguments
            .data
            .clone()
            .or(file.data.as_ref().map(relative))
            .unwrap_or_else(|| desktop::default_data(&arguments.config, arguments.config_explicit)),
        machine,
        files: Files { backup: file.backup.as_ref().map(relative) },
        mode,
        co2_manual_focus: file.co2_manual_focus.unwrap_or(true),
        ui_dir: file.ui.as_ref().map_or_else(|| beside.join("ui/dist"), relative),
        network: (!arguments.simulate)
            .then(|| Arc::new(openlaser_network::os::Live) as Arc<dyn openlaser_network::os::Os>),
    };
    Ok((config, simulator))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let outcome = (|| {
        let arguments = match cli::parse(std::env::args_os().skip(1))? {
            CommandLine::Help => {
                println!("{}", cli::HELP);
                return Ok(());
            }
            CommandLine::Version => {
                println!("OpenLaser {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            CommandLine::Run(arguments) => arguments,
        };
        if arguments.shutdown {
            let file = read_machine_file(&arguments.config, arguments.config_explicit)?;
            let beside = arguments.config.parent().unwrap_or_else(|| Path::new(""));
            let directory = arguments
                .data
                .clone()
                .or(file.data.map(|p| if p.is_absolute() { p } else { beside.join(p) }))
                .unwrap_or_else(|| {
                    desktop::default_data(&arguments.config, arguments.config_explicit)
                });
            return service::shutdown(&directory);
        }
        let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        #[cfg(feature = "desktop")]
        if !arguments.no_browser {
            return native::run(&arguments, &runtime);
        }
        runtime.block_on(async {
            let (config, simulator) = configure(&arguments).await?;
            let Some(instance) = service::Instance::acquire(&config.data_dir)? else {
                desktop::ready(service::wait(&config.data_dir)?, !arguments.no_browser);
                return Ok(());
            };
            run_server(config, simulator, std::future::pending(), |address| {
                if let Err(error) = instance.ready(address) {
                    tracing::error!(%error);
                }
                desktop::ready(address, !arguments.no_browser);
            })
            .await
        })
    })();
    if let Err(error) = outcome {
        eprintln!("openlaser: {error}");
        desktop::report_error(&error);
        std::process::exit(1);
    }
}

async fn run_server(
    config: Config,
    simulator: Option<Simulator>,
    stop: impl std::future::Future<Output = ()>,
    ready: impl FnOnce(SocketAddr),
) -> Result<(), String> {
    let listen = config.listen;
    let shared = openlaser_server::Coordinator::start(config)?;
    shared.lock().await.seed_defaults();
    if let Some(simulator) = &simulator {
        // The plant answers the machine files' inputs and parameters.
        let coordinator = shared.lock().await;
        let control = simulator.control();
        match coordinator.simulated_plant(control.parameters()) {
            Ok((rules, banks)) => {
                control.rules(&rules);
                control.set_parameters(banks);
            }
            Err(error) => tracing::warn!(%error, "the simulator keeps its defaults"),
        }
    }
    let served = openlaser_server::serve_until_ready(shared, listen, stop, ready).await;
    drop(simulator);
    served
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests use known configuration fixtures"
)]
mod tests {
    use super::*;

    struct Configuration {
        directory: PathBuf,
        arguments: Arguments,
    }

    impl Configuration {
        fn new(contents: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = std::env::temp_dir()
                .join(format!("openlaser-config-{}-{sequence}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("machine.toml");
            std::fs::write(&path, contents).unwrap();
            let Ok(CommandLine::Run(arguments)) =
                cli::parse(["--config".into(), path.into_os_string()])
            else {
                panic!("valid configuration path")
            };
            Self { directory, arguments }
        }
    }

    impl Drop for Configuration {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[tokio::test]
    async fn file_paths_are_relative_to_the_configuration_and_cli_paths_are_not() {
        let mut fixture = Configuration::new(
            r#"
controller = "192.0.2.7:1502"
host = "192.0.2.8"
listen = "127.0.0.1:9000"
data = "library"
backup = "backup.xml"
ui = "public"
mode = "fiber"
co2_manual_focus = false
"#,
        );
        let (config, simulator) = configure(&fixture.arguments).await.unwrap();
        assert!(simulator.is_none() && config.network.is_some());
        assert_eq!(config.machine.endpoint, "192.0.2.7:1502".parse().unwrap());
        assert_eq!(config.machine.host, Ipv4Addr::new(192, 0, 2, 8));
        assert_eq!(config.listen, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(config.data_dir, fixture.directory.join("library"));
        assert_eq!(config.files.backup, Some(fixture.directory.join("backup.xml")));
        assert_eq!(config.ui_dir, fixture.directory.join("public"));
        assert_eq!(config.mode, Some(LaserMode::Fiber));
        assert!(!config.co2_manual_focus);

        fixture.arguments.data = Some(PathBuf::from("cli-library"));
        fixture.arguments.listen = Some("[::1]:9001".to_owned());
        let (config, _) = configure(&fixture.arguments).await.unwrap();
        assert_eq!(config.data_dir, PathBuf::from("cli-library"));
        assert_eq!(config.listen, "[::1]:9001".parse().unwrap());
    }

    #[tokio::test]
    async fn defaults_and_simulation_keep_their_separate_network_configuration() {
        let fixture = Configuration::new("");
        let (config, simulator) = configure(&fixture.arguments).await.unwrap();
        assert!(simulator.is_none() && config.network.is_some());
        assert_eq!(config.machine, MachineConfig::machine());
        assert_eq!(config.data_dir, fixture.directory.join("data"));
        assert_eq!(config.ui_dir, fixture.directory.join("ui/dist"));
        assert_eq!(
            config.listen,
            if cfg!(feature = "desktop") { "0.0.0.0:80" } else { "127.0.0.1:8080" }
                .parse()
                .unwrap()
        );
        assert!(config.mode.is_none() && config.files.backup.is_none());
        assert!(config.co2_manual_focus);

        let mut simulated =
            Configuration::new("controller = 'ignored'\nhost = 'ignored'\nmode = 'co2'\n");
        simulated.arguments.simulate = true;
        let (config, simulator) = configure(&simulated.arguments).await.unwrap();
        assert!(config.network.is_none());
        assert_eq!(config.machine, simulator.unwrap().config());
        assert!(config.machine.endpoint.ip().is_loopback());
        assert_eq!(config.mode, Some(LaserMode::Co2));
        assert_eq!(config.listen, "127.0.0.1:8080".parse().unwrap());
    }

    #[tokio::test]
    async fn explicit_lan_and_headless_listener_defaults_are_distinct() {
        let mut fixture = Configuration::new("listen = '127.0.0.1:8080'");
        fixture.arguments.no_browser = true;
        assert_eq!(
            configure(&fixture.arguments).await.unwrap().0.listen,
            "127.0.0.1:8080".parse().unwrap()
        );
        fixture.arguments.lan = true;
        assert_eq!(
            configure(&fixture.arguments).await.unwrap().0.listen,
            "0.0.0.0:80".parse().unwrap()
        );
        fixture.arguments.listen = Some("127.0.0.1:9000".into());
        assert_eq!(
            configure(&fixture.arguments).await.unwrap().0.listen,
            "127.0.0.1:9000".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn malformed_configuration_is_rejected_before_starting_a_machine_session() {
        for (contents, message) in [
            ("controller = 'controller.local:502'", "controller must be an IPv4"),
            ("host = 'host.local'", "host must be an IPv4"),
            ("listen = 'localhost:8080'", "listen must be an address"),
            ("mode = 'unknown'", "mode must be fiber or co2"),
            ("misspelled = true", "unknown field"),
        ] {
            let fixture = Configuration::new(contents);
            let error = configure(&fixture.arguments).await.err().expect("invalid configuration");
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn an_explicit_missing_configuration_is_not_silently_replaced_by_defaults() {
        let dir = std::env::temp_dir().join(format!("openlaser-config-{}", std::process::id()));
        let path = dir.join("missing-machine.toml");
        assert!(read_machine_file(&path, false).is_ok());
        assert!(read_machine_file(&path, true).is_err());
    }
}
