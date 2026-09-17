// SPDX-License-Identifier: GPL-3.0-or-later

//! The computer's side of the machine link: which Ethernet adapter faces
//! the controller, whether it already carries an address on the
//! controller's subnet, and the one change needed when it does not.
//!
//! The vendor's host does this with an elevated `IPSet.exe` that sets an
//! adapter's static address; the prototype pinned one adapter by name and
//! MAC. Here the adapter is found each time: wired, link up, carrying no
//! default route and no address of another network, so it is the
//! machine's own cable. A remembered adapter is preferred, matched by MAC
//! then by name, never required. The controller has no verified discovery
//! mechanism, so its address is the factory default until the operator
//! gives another.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "tests read known inventories")
)]

pub mod os;

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4};

/// One IPv4 address with its prefix length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// The address.
    pub ip: Ipv4Addr,
    /// The prefix length, 0 to 32.
    pub prefix: u8,
}

impl Address {
    /// Whether `ip` lies on this address's network.
    #[must_use]
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        self.mask_bits().is_some_and(|mask| u32::from(self.ip) & mask == u32::from(ip) & mask)
    }

    /// The netmask, or `None` when the prefix is outside 0–32.
    #[must_use]
    pub fn netmask(&self) -> Option<Ipv4Addr> {
        self.mask_bits().map(Ipv4Addr::from)
    }

    fn mask_bits(self) -> Option<u32> {
        match self.prefix {
            0 => Some(0),
            1..=32 => Some(u32::MAX << (32 - u32::from(self.prefix))),
            _ => None,
        }
    }

    /// Whether the address is self-assigned, which says nothing about the
    /// network the adapter is on.
    #[must_use]
    pub const fn is_link_local(&self) -> bool {
        self.ip.is_link_local()
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.ip, self.prefix)
    }
}

/// A network adapter as the computer reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Adapter {
    /// The interface name: `en9`, `eth0`, `Ethernet 2`.
    pub name: String,
    /// What the computer calls the hardware, for the operator.
    pub description: String,
    /// The hardware address, lower case with colons.
    pub mac: Option<String>,
    /// An Ethernet port: not Wi-Fi, loopback, a tunnel or a bridge.
    pub wired: bool,
    /// Whether the link is up.
    pub up: bool,
    /// Its IPv4 addresses.
    pub addresses: Vec<Address>,
    /// Whether the computer's default route leaves through it.
    pub default_route: bool,
}

impl Adapter {
    fn eligible(&self) -> bool {
        self.wired && self.up && !self.default_route
    }

    /// Whether nothing but the machine could be on the other end: link
    /// up, no default route, and no address of another network.
    fn dedicated(&self) -> bool {
        self.eligible() && self.addresses.iter().all(Address::is_link_local)
    }
}

/// Where the controller is and the address the computer speaks from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The controller's endpoint.
    pub controller: SocketAddrV4,
    /// The computer's address on the controller's subnet.
    pub host: Address,
}

impl Target {
    /// The vendor's factory setting: the card at 10.1.1.168:502, the
    /// computer at 10.1.1.10/24.
    #[must_use]
    pub const fn factory() -> Self {
        Self {
            controller: SocketAddrV4::new(Ipv4Addr::new(10, 1, 1, 168), 502),
            host: Address { ip: Ipv4Addr::new(10, 1, 1, 10), prefix: 24 },
        }
    }
}

/// What worked last time, kept between runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    /// The adapter's name.
    pub adapter: String,
    /// Its hardware address, which outlives a renaming.
    pub mac: Option<String>,
    /// The target it connected to.
    pub target: Target,
    /// The address this program added to the adapter, when it did.
    pub owned: Option<Address>,
}

/// What to do before opening the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The adapter already carries `host` on the controller's subnet.
    Ready {
        /// The adapter.
        adapter: Adapter,
        /// The address to speak from.
        host: Ipv4Addr,
    },
    /// Add `address` to `adapter`.
    Configure {
        /// The adapter.
        adapter: Adapter,
        /// The address to add.
        address: Address,
    },
    /// The controller's network is already in use elsewhere.
    Conflict {
        /// The adapter carrying it.
        adapter: String,
        /// What it carries.
        address: Address,
        /// What prevents this adapter/address from serving the machine.
        reason: &'static str,
    },
    /// The requested or observed address/prefix is invalid.
    Invalid(&'static str),
    /// More than one cable could be the machine's.
    Ambiguous(Vec<Adapter>),
    /// No wired adapter has its link up.
    NoAdapter,
}

/// The plan for `adapters`: use the one already on the controller's
/// subnet; else configure the remembered one, or the only dedicated one.
#[must_use]
pub fn plan(adapters: &[Adapter], target: &Target, remembered: Option<&Remembered>) -> Plan {
    let controller = *target.controller.ip();
    if target.host.netmask().is_none()
        || adapters.iter().flat_map(|a| &a.addresses).any(|a| a.netmask().is_none())
    {
        return Plan::Invalid("an IPv4 prefix is outside 0–32");
    }
    if !target.host.contains(controller) {
        return Plan::Invalid("the requested host address does not reach the controller");
    }
    existing_network(adapters, target)
        .unwrap_or_else(|| choose_adapter(adapters, target, remembered))
}

fn existing_network(adapters: &[Adapter], target: &Target) -> Option<Plan> {
    let controller = *target.controller.ip();
    let on_subnet: Vec<(&Adapter, &Address)> = adapters
        .iter()
        .filter_map(|a| {
            a.addresses
                .iter()
                .find(|x| x.contains(controller) || target.host.contains(x.ip))
                .map(|x| (a, x))
        })
        .collect();
    match on_subnet.as_slice() {
        // Loopback is the one network where both ends share an address;
        // anywhere else the computer has taken the controller's own.
        [(adapter, address)]
            if adapter.eligible()
                && address.contains(controller)
                && (address.ip != controller || controller.is_loopback()) =>
        {
            return Some(Plan::Ready { adapter: (*adapter).clone(), host: address.ip });
        }
        [first, ..] => {
            let (adapter, address) =
                on_subnet.iter().find(|(a, _)| !a.wired || a.default_route).unwrap_or(first);
            return Some(Plan::Conflict {
                adapter: adapter.name.clone(),
                address: **address,
                reason: conflict_reason(adapter, **address, controller),
            });
        }
        [] => {}
    }
    None
}

fn conflict_reason(adapter: &Adapter, address: Address, controller: Ipv4Addr) -> &'static str {
    if !adapter.up {
        "its link is down"
    } else if !address.contains(controller) {
        "its configured prefix does not reach the controller"
    } else if !adapter.eligible() {
        "it is not a dedicated wired interface"
    } else {
        "the controller address or network is already in use"
    }
}

fn choose_adapter(adapters: &[Adapter], target: &Target, remembered: Option<&Remembered>) -> Plan {
    let candidates: Vec<&Adapter> = adapters.iter().filter(|a| a.dedicated()).collect();
    let preferred = remembered.and_then(|r| {
        candidates
            .iter()
            .find(|a| r.mac.is_some() && a.mac == r.mac)
            .or_else(|| candidates.iter().find(|a| a.name == r.adapter))
            .copied()
    });
    let only = match candidates.as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    match preferred.or(only) {
        Some(adapter) => Plan::Configure { adapter: adapter.clone(), address: target.host },
        None if candidates.is_empty() => Plan::NoAdapter,
        None => Plan::Ambiguous(candidates.into_iter().cloned().collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(
        name: &str,
        mac: &str,
        wired: bool,
        up: bool,
        addresses: &[(&str, u8)],
        gateway: bool,
    ) -> Adapter {
        Adapter {
            name: name.into(),
            description: String::new(),
            mac: Some(mac.into()),
            wired,
            up,
            addresses: addresses
                .iter()
                .map(|(ip, prefix)| Address { ip: ip.parse().unwrap(), prefix: *prefix })
                .collect(),
            default_route: gateway,
        }
    }

    /// A fresh computer with Wi-Fi and one cable configures the cable; a
    /// cable already on the subnet is used as it is; a second cable makes
    /// the choice ambiguous unless one is remembered, by MAC before name;
    /// the subnet on Wi-Fi or a stale cable is a conflict; no cable is no
    /// adapter.
    #[test]
    fn the_plan_follows_the_cables() {
        let target = Target::factory();
        let wifi = adapter("en0", "aa:00", false, true, &[("192.168.1.20", 24)], true);
        let cable = adapter("en9", "aa:09", true, true, &[("169.254.3.4", 16)], false);
        let ready = adapter("en9", "aa:09", true, true, &[("10.1.1.10", 24)], false);
        assert_eq!(
            plan(&[wifi.clone(), cable.clone()], &target, None),
            Plan::Configure { adapter: cable.clone(), address: target.host }
        );
        assert_eq!(
            plan(&[wifi.clone(), ready.clone()], &target, None),
            Plan::Ready { adapter: ready.clone(), host: "10.1.1.10".parse().unwrap() }
        );
        let second = adapter("en5", "aa:05", true, true, &[], false);
        assert!(
            matches!(plan(&[cable.clone(), second.clone()], &target, None), Plan::Ambiguous(ref a) if a.len() == 2)
        );
        let remembered =
            Remembered { adapter: "en7".into(), mac: Some("aa:05".into()), target, owned: None };
        assert_eq!(
            plan(&[cable.clone(), second.clone()], &target, Some(&remembered)),
            Plan::Configure { adapter: second.clone(), address: target.host }
        );
        let by_name = Remembered { adapter: "en9".into(), mac: None, ..remembered };
        assert_eq!(
            plan(&[cable.clone(), second], &target, Some(&by_name)),
            Plan::Configure { adapter: cable.clone(), address: target.host }
        );
        let office = adapter("en0", "aa:00", false, true, &[("10.1.1.50", 24)], true);
        assert!(
            matches!(plan(&[office, cable.clone()], &target, None), Plan::Conflict { ref adapter, .. } if adapter == "en0")
        );
        let own = adapter("en9", "aa:09", true, true, &[("10.1.1.168", 24)], false);
        assert!(matches!(plan(&[own], &target, None), Plan::Conflict { .. }));
        let loopback = Target {
            controller: "127.0.0.1:502".parse().unwrap(),
            host: Address { ip: Ipv4Addr::LOCALHOST, prefix: 24 },
        };
        let lo = adapter("lo0", "00:00", true, true, &[("127.0.0.1", 8)], false);
        assert!(matches!(plan(&[lo], &loopback, None), Plan::Ready { .. }));
        let down = adapter("en9", "aa:09", true, false, &[], false);
        assert_eq!(plan(&[wifi, down], &target, None), Plan::NoAdapter);
        assert!(target.host.contains("10.1.1.168".parse().unwrap()));
        assert!(!target.host.contains("10.1.2.1".parse().unwrap()));
        assert_eq!(target.host.netmask(), Some(Ipv4Addr::new(255, 255, 255, 0)));
    }

    #[test]
    fn readiness_uses_link_state_and_the_actual_configured_prefix() {
        let target = Target::factory();
        let down = adapter("en9", "aa:09", true, false, &[("10.1.1.10", 24)], false);
        assert!(matches!(
            plan(&[down], &target, None),
            Plan::Conflict { reason: "its link is down", .. }
        ));
        let wrong = adapter("en9", "aa:09", true, true, &[("10.1.1.10", 32)], false);
        assert!(matches!(
            plan(&[wrong], &target, None),
            Plan::Conflict { reason: "its configured prefix does not reach the controller", .. }
        ));
        let routed = adapter("en9", "aa:09", true, true, &[("10.1.1.10", 24)], true);
        assert!(matches!(plan(&[routed], &target, None), Plan::Conflict { .. }));
        let invalid = Address { prefix: 33, ..target.host };
        assert_eq!(invalid.netmask(), None);
        assert!(!invalid.contains(*target.controller.ip()));
        assert!(matches!(plan(&[], &Target { host: invalid, ..target }, None), Plan::Invalid(_)));
        let wide = adapter("en9", "aa:09", true, true, &[("10.1.2.10", 16)], false);
        assert!(matches!(plan(&[wide], &target, None), Plan::Ready { .. }));
    }
}
