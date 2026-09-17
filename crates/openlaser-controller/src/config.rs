// SPDX-License-Identifier: GPL-3.0-or-later

//! Where the controller is and how patiently it is polled.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

/// The vendor's socket timeout per exchange.
pub const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
/// How often the task polls the controller.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The connection settings, from `machine.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The controller card's endpoint; the vendor default is 10.1.1.168:502.
    pub endpoint: SocketAddrV4,
    /// The host address to send from. The machine's dedicated segment gives
    /// the host 10.1.1.10; the simulator runs on loopback.
    pub host: Ipv4Addr,
    /// How long to wait for a reply to one request.
    pub reply_timeout: Duration,
    /// How often feedback is read while connected.
    pub poll_interval: Duration,
}

impl Config {
    /// The vendor's machine: the card at 10.1.1.168:502 from host 10.1.1.10.
    #[must_use]
    pub fn machine() -> Self {
        Self {
            endpoint: SocketAddrV4::new(Ipv4Addr::new(10, 1, 1, 168), 502),
            host: Ipv4Addr::new(10, 1, 1, 10),
            reply_timeout: REPLY_TIMEOUT,
            poll_interval: POLL_INTERVAL,
        }
    }

    /// A simulator on loopback at `endpoint`.
    #[must_use]
    pub fn loopback(endpoint: SocketAddrV4) -> Self {
        Self {
            endpoint,
            host: Ipv4Addr::LOCALHOST,
            reply_timeout: REPLY_TIMEOUT,
            poll_interval: POLL_INTERVAL,
        }
    }
}
