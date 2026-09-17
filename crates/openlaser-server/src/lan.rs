// SPDX-License-Identifier: GPL-3.0-or-later

//! Advertises the embedded web server on the shop LAN, outside the controller link.

use mdns_sd::{DaemonEvent, IfKind, ServiceDaemon, ServiceInfo};
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr};
use tokio::sync::watch;

#[derive(Clone, Serialize)]
pub(crate) struct Status {
    pub enabled: bool,
    pub ready: bool,
    pub error: Option<String>,
}

pub(crate) struct Discovery {
    daemon: Option<ServiceDaemon>,
    fullname: String,
    pub status: watch::Receiver<Status>,
}

impl Discovery {
    pub fn start(listen: SocketAddr, controller_host: Ipv4Addr) -> Self {
        let enabled = !listen.ip().is_loopback();
        let (status, receiver) = watch::channel(Status { enabled, ready: false, error: None });
        let mut discovery = Self { daemon: None, fullname: String::new(), status: receiver };
        if enabled && let Err(error) = discovery.register(listen, controller_host, status.clone()) {
            status.send_modify(|s| s.error = Some(format!("Local discovery: {error}")));
        }
        discovery
    }

    fn register(
        &mut self,
        listen: SocketAddr,
        controller_host: Ipv4Addr,
        status: watch::Sender<Status>,
    ) -> Result<(), String> {
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        self.daemon = Some(daemon.clone());
        if listen.ip().is_unspecified() {
            daemon
                .disable_interface(if listen.is_ipv4() { IfKind::IPv6 } else { IfKind::IPv4 })
                .map_err(|e| e.to_string())?;
            if !controller_host.is_loopback() && !controller_host.is_unspecified() {
                daemon
                    .disable_interface(IfKind::Addr(controller_host.into()))
                    .map_err(|e| e.to_string())?;
            }
        } else {
            daemon.disable_interface(IfKind::All).map_err(|e| e.to_string())?;
            daemon.enable_interface(IfKind::Addr(listen.ip())).map_err(|e| e.to_string())?;
        }
        daemon
            .disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
            .map_err(|e| e.to_string())?;
        let info = ServiceInfo::new(
            "_http._tcp.local.",
            "OpenLaser",
            "openlaser.local.",
            "",
            listen.port(),
            &[("path", "/"), ("mobile", "/mobile")][..],
        )
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
        info.get_fullname().clone_into(&mut self.fullname);
        let monitor = daemon.monitor().map_err(|e| e.to_string())?;
        daemon.register(info).map_err(|e| e.to_string())?;
        let fullname = self.fullname.clone();
        tokio::spawn(async move {
            while let Ok(event) = monitor.recv_async().await {
                match event {
                    DaemonEvent::Announce(_, _) => {
                        status.send_modify(|s| s.ready = s.error.is_none());
                    }
                    DaemonEvent::NameChange(change)
                        if change.original.eq_ignore_ascii_case("openlaser.local.") =>
                    {
                        status.send_modify(|s| {
                            s.ready = false;
                            s.error =
                                Some("openlaser.local is already used on this network.".into());
                        });
                        let _ = daemon.unregister(&fullname);
                    }
                    DaemonEvent::Error(error) => status.send_modify(|s| {
                        s.ready = false;
                        s.error = Some(error.to_string());
                    }),
                    _ => {}
                }
            }
        });
        Ok(())
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        if let Some(daemon) = &self.daemon {
            let _ = daemon.unregister(&self.fullname);
            let _ = daemon.shutdown();
        }
    }
}
