// SPDX-License-Identifier: GPL-3.0-or-later

//! Connect: one path from the button to a verified live state.
//!
//! The Ethernet adapter towards the machine is found and, when it carries
//! no address on the controller's network, given one with the system's
//! own authorization prompt; the socket opens; the controller's identity
//! and a complete snapshot are read before the state says connected; the
//! machine files bind and initialize controller settings, then verify their
//! readback. No homing or job resume occurs. Each step publishes its progress,
//! a click while an attempt runs does nothing, and a cancel ends the
//! attempt at its next step. The route that worked is remembered beside
//! the library and tried first next time; its adapter is matched by
//! hardware address, then by name, and never required.

use crate::coordinator::{Config, Shared};
use crate::document::{AdapterView, LinkPhase, LinkView};
use crate::{Error, Result};
use openlaser_network::os::{self, Os};
use openlaser_network::{Adapter, Address, Plan, Remembered, Target, plan};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The computer's side of the link, as the coordinator keeps it.
#[derive(Debug, Default)]
pub struct Link {
    /// The step in progress, or how the last attempt ended.
    pub phase: LinkPhase,
    /// What the step is doing, or why the attempt failed.
    pub detail: String,
    /// The adapter in use, once found.
    pub adapter: Option<Adapter>,
    /// The address spoken from, once known.
    pub host: Option<Ipv4Addr>,
    /// The adapters an ambiguous inventory offers.
    pub choices: Vec<Adapter>,
    /// What worked last time, or what the operator chose.
    pub remembered: Option<Remembered>,
    /// Ends the attempt in progress at its next step.
    cancel: Option<Arc<AtomicBool>>,
}

impl Link {
    /// The link at start: the remembered route, when there is a network
    /// to remember it for.
    pub(crate) fn open(config: &Config) -> Self {
        let remembered = config.network.as_ref().and_then(|_| read_remembered(&config.data_dir));
        Self { remembered, ..Self::default() }
    }

    /// Whether an attempt is in progress.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.cancel.is_some()
    }

    /// The route to try: the remembered one, else the configured one.
    fn target(&self, config: &Config) -> Target {
        self.remembered.as_ref().map_or_else(|| configured(config), |r| r.target)
    }

    pub(crate) fn view(&self, config: &Config) -> LinkView {
        LinkView {
            phase: self.phase,
            detail: self.detail.clone(),
            adapter: self.adapter.as_ref().map(AdapterView::from),
            host: self.host.map(|h| h.to_string()),
            computer: self.target(config).host.to_string(),
            controller: self.target(config).controller.to_string(),
            remembered_adapter: self
                .remembered
                .as_ref()
                .map_or_else(String::new, |r| r.adapter.clone()),
            choices: self.choices.iter().map(AdapterView::from).collect(),
        }
    }

    pub(crate) fn set(&mut self, phase: LinkPhase, detail: impl Into<String>) {
        self.phase = phase;
        self.detail = detail.into();
    }
}

impl From<&Adapter> for AdapterView {
    fn from(adapter: &Adapter) -> Self {
        Self {
            name: adapter.name.clone(),
            description: adapter.description.clone(),
            addresses: adapter.addresses.iter().map(ToString::to_string).collect(),
        }
    }
}

/// The route the configuration gives: the controller's endpoint and the
/// computer's address on its /24, the mask the vendor's IPSet.exe sets.
fn configured(config: &Config) -> Target {
    Target {
        controller: config.machine.endpoint,
        host: Address { ip: config.machine.host, prefix: 24 },
    }
}

fn remembered_path(data_dir: &Path) -> PathBuf {
    data_dir.join("connection.json")
}

/// The route that worked last time, if any.
#[must_use]
pub fn read_remembered(data_dir: &Path) -> Option<Remembered> {
    serde_json::from_slice(&std::fs::read(remembered_path(data_dir)).ok()?).ok()
}

fn remember(data_dir: &Path, remembered: &Remembered) -> Result<()> {
    let path = remembered_path(data_dir);
    let json = serde_json::to_vec_pretty(remembered).map_err(|e| Error::Refused(e.to_string()))?;
    openlaser_library::atomic_write(&path, &json).map_err(Error::from)
}

/// Connects. A click while an attempt runs, or while connected, does
/// nothing.
pub async fn connect(shared: &Shared) -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut coordinator = shared.lock().await;
        if coordinator.link.busy() || coordinator.connected() {
            return Ok(());
        }
        // Whatever the last connection established is stale now.
        coordinator.bound = None;
        coordinator.bindings_changed();
        coordinator.accepted = None;
        coordinator.parameter_problem = None;
        coordinator.held = None;
        coordinator.link.cancel = Some(Arc::clone(&cancel));
        coordinator.link.choices.clear();
        coordinator.link.set(LinkPhase::Inspecting, "Looking for the machine's Ethernet cable");
        coordinator.publish();
    }
    let outcome = attempt(shared, &cancel).await;
    let mut coordinator = shared.lock().await;
    coordinator.link.cancel = None;
    match outcome {
        Ok(None) => {
            coordinator.link.set(LinkPhase::Idle, "");
            coordinator.note("Connected", false);
        }
        Ok(Some(problem)) => {
            coordinator.link.set(LinkPhase::Idle, "");
            coordinator.note(format!("Connected; {problem}"), true);
        }
        Err(error) if cancel.load(Ordering::Relaxed) => {
            coordinator.link.set(LinkPhase::Idle, "Cancelled");
            coordinator.note("Connection cancelled", false);
            return Err(error);
        }
        Err(error) => {
            coordinator.link.set(LinkPhase::Failed, error.to_string());
            coordinator.note(error.to_string(), true);
            return Err(error);
        }
    }
    Ok(())
}

/// Ends the attempt in progress at its next step.
pub async fn cancel(shared: &Shared) {
    if let Some(flag) = &shared.lock().await.link.cancel {
        flag.store(true, Ordering::Relaxed);
    }
}

/// The way to the controller once the computer is ready.
struct Route {
    /// The adapter, when there is a network; none for the simulator.
    adapter: Option<Adapter>,
    /// The address to speak from.
    host: Ipv4Addr,
    /// The address this attempt added.
    owned: Option<Address>,
}

/// The steps; the parameter problem, if any, once connected.
async fn attempt(shared: &Shared, cancel: &AtomicBool) -> Result<Option<String>> {
    let (machine, network, remembered, target, data_dir) = {
        let coordinator = shared.lock().await;
        (
            coordinator.machine.clone(),
            coordinator.config.network.clone(),
            coordinator.link.remembered.clone(),
            coordinator.link.target(&coordinator.config),
            coordinator.config.data_dir.clone(),
        )
    };
    let route = match &network {
        // The simulator: loopback, nothing to prepare.
        None => Route { adapter: None, host: target.host.ip, owned: None },
        Some(os) => prepare(shared, os, &target, remembered.as_ref(), cancel).await?,
    };
    {
        let mut coordinator = shared.lock().await;
        coordinator.link.adapter.clone_from(&route.adapter);
        coordinator.link.host = Some(route.host);
        let detail = format!("Reaching the controller at {}", target.controller);
        coordinator.link.set(LinkPhase::Connecting, detail);
        coordinator.publish();
    }
    let opened = machine.connect_to(target.controller, route.host).await;
    if cancel.load(Ordering::Relaxed) {
        machine.disconnect().await.ok();
        return Err(Error::Refused("cancelled".into()));
    }
    opened.map_err(|error| unreachable(error, target.controller, route.host))?;
    {
        let mut coordinator = shared.lock().await;
        let detail = "Applying machine settings and verifying controller readback";
        coordinator.link.set(LinkPhase::Reading, detail);
        coordinator.publish();
    }
    let problem = crate::machine::after_connect(shared).await;
    if let Some(adapter) = route.adapter {
        let owned = route
            .owned
            .or_else(|| remembered.filter(|r| r.mac == adapter.mac).and_then(|r| r.owned));
        let remembered = Remembered { adapter: adapter.name, mac: adapter.mac, target, owned };
        if let Err(error) = remember(&data_dir, &remembered) {
            tracing::warn!(%error, "the route could not be remembered");
        }
        shared.lock().await.link.remembered = Some(remembered);
    }
    Ok(problem)
}

/// Why the controller was not reached, for the operator.
fn unreachable(
    error: openlaser_controller::Error,
    endpoint: SocketAddrV4,
    host: Ipv4Addr,
) -> Error {
    match error {
        openlaser_controller::Error::Link(reason) => Error::Refused(format!(
            "no controller answered at {endpoint} from {host}: {reason}. Check the cable and that the machine is switched on"
        )),
        other => Error::Refused(other.to_string()),
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) { Err(Error::Refused("cancelled".into())) } else { Ok(()) }
}

/// One system call, off the async threads.
async fn blocking<T: Send + 'static>(
    os: &Arc<dyn Os>,
    call: impl FnOnce(&dyn Os) -> std::result::Result<T, os::Error> + Send + 'static,
) -> std::result::Result<T, os::Error> {
    let os = Arc::clone(os);
    tokio::task::spawn_blocking(move || call(&*os))
        .await
        .unwrap_or_else(|error| Err(os::Error::Unavailable(error.to_string())))
}

/// The adapter and the address to speak from, configuring the adapter
/// when it needs it: the system asks the operator for authorization.
async fn prepare(
    shared: &Shared,
    os: &Arc<dyn Os>,
    target: &Target,
    remembered: Option<&Remembered>,
    cancel: &AtomicBool,
) -> Result<Route> {
    let adapters = blocking(os, |os| os.adapters())
        .await
        .map_err(|e| Error::Refused(format!("the computer's adapters could not be read: {e}")))?;
    let controller = *target.controller.ip();
    let (adapter, host, owned) = match plan(&adapters, target, remembered) {
        Plan::Ready { adapter, host } => (adapter, host, None),
        Plan::Configure { adapter, address } => {
            cancelled(cancel)?;
            let name = describe(&adapter);
            {
                let mut coordinator = shared.lock().await;
                coordinator.link.adapter = Some(adapter.clone());
                let detail = format!(
                    "Giving {name} the address {address}; the computer asks for permission"
                );
                coordinator.link.set(LinkPhase::Configuring, detail);
                coordinator.publish();
            }
            let interface = adapter.name.clone();
            blocking(os, move |os| os.add_address(&interface, &address)).await.map_err(|error| {
                Error::Refused(match error {
                    os::Error::Denied => format!(
                        "permission to give {name} the address {address} was refused; nothing was changed"
                    ),
                    other => format!("{name} could not be configured: {other}"),
                })
            })?;
            let adapters = blocking(os, |os| os.adapters())
                .await
                .map_err(|e| Error::Refused(e.to_string()))?;
            match plan(&adapters, target, remembered) {
                Plan::Ready { adapter, host } => (adapter, host, Some(address)),
                _ => {
                    return Err(Error::Refused(format!(
                        "{} did not take the address {address}",
                        describe(&adapter)
                    )));
                }
            }
        }
        Plan::Conflict { adapter, address, .. } if address.ip == controller => {
            return Err(Error::Refused(format!(
                "this computer has the controller's own address {address} on {adapter}; remove it, or give the controller another address on the Settings page"
            )));
        }
        Plan::Conflict { adapter, address, reason } => {
            return Err(Error::Refused(format!(
                "{adapter} carries {address}, but {reason}; check the cable and its address before connecting"
            )));
        }
        Plan::Invalid(reason) => return Err(Error::Request(reason.into())),
        Plan::Ambiguous(choices) => {
            let names: Vec<String> = choices.iter().map(describe).collect();
            shared.lock().await.link.choices = choices;
            return Err(Error::Refused(format!(
                "more than one Ethernet cable could be the machine's ({}); choose one on the Settings page",
                names.join(", ")
            )));
        }
        Plan::NoAdapter => {
            return Err(Error::Refused(
                "no Ethernet adapter has its link up; plug the machine's cable into this computer and switch the machine on".into(),
            ));
        }
    };
    let interface = adapter.name.clone();
    let reaches = blocking(os, move |os| os.routes_through(controller, &interface))
        .await
        .map_err(|e| Error::Refused(e.to_string()))?;
    if !reaches {
        return Err(Error::Refused(format!(
            "{} is not the way to {controller}; another adapter carries that network",
            describe(&adapter)
        )));
    }
    Ok(Route { adapter: Some(adapter), host, owned })
}

/// How the operator knows the adapter.
fn describe(adapter: &Adapter) -> String {
    if adapter.description.is_empty() {
        adapter.name.clone()
    } else {
        format!("{} ({})", adapter.description, adapter.name)
    }
}

/// A change to the route from the Settings page: an adapter chosen among
/// several, or another address for the controller or the computer. Kept
/// for the next Connect.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, optional_fields))]
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct RouteChange {
    /// Saved field values from which these edits started.
    pub expected: Option<std::collections::BTreeMap<String, String>>,
    /// The adapter to use, by name.
    pub adapter: Option<String>,
    /// The controller's endpoint, `ip:port`.
    pub controller: Option<String>,
    /// The computer's address on the controller's network, `ip/prefix`.
    pub host: Option<String>,
}

/// Applies a route change.
pub async fn set_route(shared: &Shared, change: RouteChange) -> Result<()> {
    let mut coordinator = shared.lock().await;
    if coordinator.config.network.is_none() {
        return Err(Error::Refused("the simulator's route is fixed".into()));
    }
    let mut remembered = coordinator.link.remembered.clone().unwrap_or(Remembered {
        adapter: String::new(),
        mac: None,
        target: configured(&coordinator.config),
        owned: None,
    });
    check_route_base(&change, &remembered)?;
    if let Some(name) = change.adapter {
        remembered.mac =
            coordinator.link.choices.iter().find(|a| a.name == name).and_then(|a| a.mac.clone());
        remembered.adapter = name;
    }
    if let Some(controller) = change.controller {
        remembered.target.controller = controller.parse().map_err(|_| {
            Error::Request("the controller address is ip:port, such as 10.1.1.168:502".into())
        })?;
    }
    if let Some(host) = change.host {
        remembered.target.host = host_address(&host)?;
    }
    remember(&coordinator.config.data_dir, &remembered)?;
    coordinator.link.remembered = Some(remembered);
    coordinator.link.choices.clear();
    coordinator.link.set(LinkPhase::Idle, "");
    coordinator.publish();
    Ok(())
}

fn check_route_base(change: &RouteChange, remembered: &Remembered) -> Result<()> {
    for (key, base) in change.expected.as_ref().into_iter().flatten() {
        let (saved, next) = match key.as_str() {
            "adapter" => (remembered.adapter.clone(), change.adapter.as_ref()),
            "controller" => (remembered.target.controller.to_string(), change.controller.as_ref()),
            "host" => (remembered.target.host.to_string(), change.host.as_ref()),
            _ => return Err(Error::Request("unknown route field".into())),
        };
        if &saved != base && next != Some(&saved) {
            return Err(Error::Refused(format!("{key} changed; review pending changes again")));
        }
    }
    Ok(())
}

fn host_address(host: &str) -> Result<Address> {
    let invalid =
        || Error::Request("the computer's address is ip/prefix, such as 10.1.1.10/24".into());
    let (ip, prefix) = host.split_once('/').ok_or_else(invalid)?;
    Ok(Address {
        ip: ip.parse().map_err(|_| invalid())?,
        prefix: prefix.parse().ok().filter(|p| *p <= 32).ok_or_else(invalid)?,
    })
}
