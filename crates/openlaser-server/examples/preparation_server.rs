// SPDX-License-Identifier: GPL-3.0-or-later

//! Loopback-only simulator for preparation benchmarks; never contacts hardware.
//! Arguments: private-data-directory backup.xml ui-directory port.

use openlaser_controller::Simulator;
use openlaser_core::LaserMode;
use openlaser_server::bindings::Files;
use openlaser_server::{Config, Coordinator};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = |i| args.get(i).map(PathBuf::from).ok_or("missing path argument");
    let port = args.get(4).ok_or("port is required")?.parse()?;
    let listen = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let simulator = Simulator::start().await?;
    let shared = Coordinator::start(Config {
        listen,
        data_dir: path(1)?,
        machine: simulator.config(),
        files: Files { backup: Some(path(2)?) },
        mode: Some(LaserMode::Co2),
        co2_manual_focus: true,
        ui_dir: path(3)?,
        network: None,
    })?;
    shared.lock().await.seed_defaults();
    let control = simulator.control();
    let (rules, banks) = shared.lock().await.simulated_plant(control.parameters())?;
    control.rules(&rules);
    control.set_parameters(banks);
    openlaser_server::serve_ready(shared, listen, |address| println!("http://{address}")).await?;
    drop(simulator);
    Ok(())
}
