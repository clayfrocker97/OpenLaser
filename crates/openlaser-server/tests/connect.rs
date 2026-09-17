// SPDX-License-Identifier: GPL-3.0-or-later

//! The connect workflow against scripted computers and the simulator: the
//! adapter is found and configured with one prompt, refusals and conflicts
//! are named, the route is remembered, and a lost or wrong controller is
//! reported without a write.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests drive a known plant")]

mod common;

use common::{config, seed, start, until};
use openlaser_controller::Simulator;
use openlaser_controller::state::Connection;
use openlaser_network::os::{Fake, Os};
use openlaser_network::{Adapter, Address, Remembered, Target};
use openlaser_server::coordinator::Shared;
use openlaser_server::document::LinkPhase;
use openlaser_server::{Coordinator, connect, machine};
use std::sync::Arc;

fn adapter(
    name: &str,
    mac: &str,
    wired: bool,
    up: bool,
    addresses: &[&str],
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
            .map(|a| {
                let (ip, prefix) = a.split_once('/').unwrap();
                Address { ip: ip.parse().unwrap(), prefix: prefix.parse().unwrap() }
            })
            .collect(),
        default_route: gateway,
    }
}

/// The office Wi-Fi with the default route.
fn wifi() -> Adapter {
    adapter("en0", "aa:00", false, true, &["192.168.1.20/24"], true)
}

/// A cable with nothing but a self-assigned address: the machine's.
fn cable(name: &str, mac: &str) -> Adapter {
    adapter(name, mac, true, true, &["169.254.3.4/16"], false)
}

/// A coordinator over `computer`, towards the simulator, seeded for it.
async fn start_on(name: &str, simulator: &Simulator, computer: &Arc<Fake>) -> Shared {
    let mut config = config(name, simulator.config());
    config.network = Some(Arc::clone(computer) as Arc<dyn Os>);
    let shared = Coordinator::start(config).unwrap();
    seed(&shared, simulator).await;
    shared
}

/// The last attempt failed and left the link down; its reason.
async fn failure(shared: &Shared) -> String {
    let document = shared.lock().await.document();
    assert_eq!(document.link.phase, LinkPhase::Failed);
    assert!(matches!(document.machine.connection, Connection::Disconnected));
    document.link.detail
}

/// A fresh computer with Wi-Fi, an office cable and the machine's cable:
/// the machine's cable alone is dedicated, it gets the address with one
/// prompt, the controller answers, saved settings initialize and verify, and
/// the route is remembered. A second click while connected does nothing;
/// the next connection finds the address in place and asks for nothing.
#[tokio::test]
async fn a_fresh_computer_connects_with_one_prompt() {
    let simulator = Simulator::start().await.unwrap();
    let office = adapter("en8", "aa:08", true, true, &["172.16.0.5/24"], false);
    let computer = Arc::new(Fake::with(vec![wifi(), office, cable("en9", "aa:09")]));
    let shared = start_on("fresh", &simulator, &computer).await;
    connect::connect(&shared).await.unwrap();
    let document = shared.lock().await.document();
    assert!(matches!(document.machine.connection, Connection::Connected { epoch: 1, .. }));
    assert_eq!(document.link.phase, LinkPhase::Idle);
    assert_eq!(document.link.adapter.as_ref().map(|a| a.name.as_str()), Some("en9"));
    assert_eq!(document.link.host.as_deref(), Some("127.0.0.1"));
    assert!(document.bindings.is_some(), "{:?}", document.bindings_error);
    assert!(document.readiness.home.ok, "{:?}", document.readiness.home);
    assert!(!document.readiness.connect.ok);
    assert_initialized(&simulator);
    let added = computer.added();
    assert_eq!(added.len(), 1);
    assert_eq!((added[0].0.as_str(), added[0].1.to_string().as_str()), ("en9", "127.0.0.1/24"));
    let data_dir = shared.lock().await.config.data_dir.clone();
    let remembered = connect::read_remembered(&data_dir).unwrap();
    assert_eq!((remembered.adapter.as_str(), remembered.mac.as_deref()), ("en9", Some("aa:09")));
    assert_eq!(remembered.owned, Some(added[0].1));

    connect::connect(&shared).await.unwrap();
    assert!(matches!(
        shared.lock().await.document().machine.connection,
        Connection::Connected { epoch: 1, .. }
    ));
    assert!(simulator.control().take_writes().is_empty(), "a repeated click reinitialized");
    machine::disconnect(&shared).await.unwrap();
    connect::connect(&shared).await.unwrap();
    assert!(matches!(
        shared.lock().await.document().machine.connection,
        Connection::Connected { epoch: 2, .. }
    ));
    assert_eq!(computer.added().len(), 1, "the address in place is used as it is");
}

/// A refused authorization prompt ends the attempt with the reason and
/// leaves the computer as it was.
#[tokio::test]
async fn a_refused_prompt_changes_nothing() {
    let simulator = Simulator::start().await.unwrap();
    let computer = Arc::new(Fake::with(vec![wifi(), cable("en9", "aa:09")]));
    computer.deny();
    let shared = start_on("denied", &simulator, &computer).await;
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert!(
        error.starts_with("permission to give en9 the address 127.0.0.1/24 was refused"),
        "{error}"
    );
    assert_eq!(failure(&shared).await, error);
    assert!(computer.added().is_empty());
}

/// Two cables that could each be the machine's are offered as a choice;
/// the operator's choice is kept and used, and a renamed adapter is still
/// known by its hardware address. A route typed wrongly is refused.
#[tokio::test]
async fn two_cables_need_a_choice_and_the_choice_is_kept() {
    let simulator = Simulator::start().await.unwrap();
    let computer = Arc::new(Fake::with(vec![cable("en9", "aa:09"), cable("en5", "aa:05")]));
    let shared = start_on("choice", &simulator, &computer).await;
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert!(
        error.starts_with("more than one Ethernet cable could be the machine's (en9, en5)"),
        "{error}"
    );
    let document = shared.lock().await.document();
    assert_eq!(
        document.link.choices.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["en9", "en5"]
    );
    let bad = connect::RouteChange { controller: Some("10.1.1.168".into()), ..Default::default() };
    assert!(matches!(
        connect::set_route(&shared, bad).await,
        Err(openlaser_server::Error::Request(_))
    ));
    let choice = connect::RouteChange { adapter: Some("en5".into()), ..Default::default() };
    connect::set_route(&shared, choice).await.unwrap();
    connect::connect(&shared).await.unwrap();
    assert_eq!(computer.added()[0].0, "en5");
    let data_dir = shared.lock().await.config.data_dir.clone();
    let remembered = connect::read_remembered(&data_dir).unwrap();
    assert_eq!((remembered.adapter.as_str(), remembered.mac.as_deref()), ("en5", Some("aa:05")));

    // The same computer names the adapter differently next time.
    let renamed = Arc::new(Fake::with(vec![cable("en9", "aa:09"), cable("en7", "aa:05")]));
    let mut config = config("renamed", simulator.config());
    std::fs::write(
        config.data_dir.join("connection.json"),
        serde_json::to_vec(&Remembered { target: Target::factory(), ..remembered }).unwrap(),
    )
    .unwrap();
    config.network = Some(Arc::clone(&renamed) as Arc<dyn Os>);
    let shared = Coordinator::start(config).unwrap();
    assert_eq!(shared.lock().await.document().link.controller, "10.1.1.168:502", "remembered");
    let route = connect::RouteChange {
        controller: Some(simulator.endpoint().to_string()),
        host: Some("127.0.0.1/24".into()),
        ..Default::default()
    };
    connect::set_route(&shared, route).await.unwrap();
    connect::connect(&shared).await.unwrap();
    assert_eq!(renamed.added()[0].0, "en7");
    assert_eq!(
        connect::read_remembered(&shared.lock().await.config.data_dir).unwrap().adapter,
        "en7"
    );
}

/// The controller's network already reached another way, and no cable at
/// all, are each named without touching anything.
#[tokio::test]
async fn conflicts_and_missing_cables_are_named() {
    let simulator = Simulator::start().await.unwrap();
    let office = adapter("en0", "aa:00", false, true, &["127.0.0.5/24"], true);
    let computer = Arc::new(Fake::with(vec![office, cable("en9", "aa:09")]));
    let shared = start_on("conflict", &simulator, &computer).await;
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert!(
        error.starts_with("en0 carries 127.0.0.5/24, but it is not a dedicated wired interface"),
        "{error}"
    );
    assert!(computer.added().is_empty());

    let unplugged =
        Arc::new(Fake::with(vec![wifi(), adapter("en9", "aa:09", true, false, &[], false)]));
    let shared = start_on("unplugged", &simulator, &unplugged).await;
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert!(error.starts_with("no Ethernet adapter has its link up"), "{error}");
    assert_eq!(failure(&shared).await, error);
}

/// A device that is not an MCC100 is refused by its identity; an address
/// that never answers is reported with both addresses; a cable pulled
/// while connected faults the link and closes every gate; when the machine
/// is back the reconnection starts a new epoch with the reference and the
/// checkpoint gone, and saved settings initialize again without motion.
#[tokio::test]
async fn wrong_silent_and_lost_controllers_are_reported() {
    let (simulator, shared) = start("wrong").await;
    let control = simulator.control();
    control.product(7);
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert_eq!(error, "the controller reports product 7, not the MCC100");
    assert_eq!(failure(&shared).await, error);
    control.product(103);

    let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let endpoint = match silent.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        std::net::SocketAddr::V6(_) => unreachable!(),
    };
    let deaf =
        Coordinator::start(config("silent", openlaser_controller::Config::loopback(endpoint)))
            .unwrap();
    let error = connect::connect(&deaf).await.unwrap_err().to_string();
    assert!(
        error.starts_with(&format!("no controller answered at {endpoint} from 127.0.0.1")),
        "{error}"
    );
    assert!(error.ends_with("Check the cable and that the machine is switched on"), "{error}");

    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed).await;
    assert!(shared.lock().await.document().readiness.jog.ok);
    let endpoint = simulator.endpoint();
    drop(simulator);
    until(&shared, 5, |d| matches!(d.machine.connection, Connection::Faulted { .. })).await;
    let document = shared.lock().await.document();
    assert!(!document.readiness.jog.ok && !document.readiness.home.ok);
    assert!(document.readiness.connect.ok);
    let error = connect::connect(&shared).await.unwrap_err().to_string();
    assert!(error.starts_with("no controller answered"), "{error}");

    let simulator = Simulator::start_at(endpoint).await.unwrap();
    seed(&shared, &simulator).await;
    connect::connect(&shared).await.unwrap();
    let document = shared.lock().await.document();
    assert!(matches!(document.machine.connection, Connection::Connected { epoch: 2, .. }));
    assert!(!document.machine.session.homed, "the reference belongs to the old epoch");
    assert!(!document.readiness.run.ok && !document.can_resume);
    assert_initialized(&simulator);
}

/// Exact initialization requests exclude homing, jogging, and job replay.
fn assert_initialized(simulator: &Simulator) {
    let source = std::fs::read(common::fixture("xml/harness-backup.xml")).unwrap();
    let doc = openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, &source).unwrap();
    let expected = openlaser_xml::initialization::Plan::from_document(
        &doc,
        1000,
        openlaser_core::LaserMode::Fiber,
    )
    .unwrap();
    assert_eq!(simulator.control().take_writes(), expected.writes);
}

/// A second click while an attempt runs does nothing, and a cancel ends
/// the attempt at its next step without a failure.
#[tokio::test]
async fn repeated_clicks_and_a_cancel() {
    let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let endpoint = match silent.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        std::net::SocketAddr::V6(_) => unreachable!(),
    };
    let shared =
        Coordinator::start(config("cancel", openlaser_controller::Config::loopback(endpoint)))
            .unwrap();
    let first = tokio::spawn({
        let shared = Arc::clone(&shared);
        async move { connect::connect(&shared).await }
    });
    until(&shared, 5, |d| d.link.phase == LinkPhase::Connecting).await;
    assert!(!shared.lock().await.document().readiness.connect.ok, "busy");
    let started = tokio::time::Instant::now();
    connect::connect(&shared).await.unwrap();
    assert!(started.elapsed().as_millis() < 100, "a second click returns at once");
    connect::cancel(&shared).await;
    assert!(first.await.unwrap().is_err());
    let document = shared.lock().await.document();
    assert_eq!(
        (document.link.phase, document.link.detail.as_str()),
        (LinkPhase::Idle, "Cancelled")
    );
    assert_eq!(
        document.message.as_ref().map(|m| (m.text.as_str(), m.error)),
        Some(("Connection cancelled", false))
    );
    assert!(document.readiness.connect.ok);
}
