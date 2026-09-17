// SPDX-License-Identifier: GPL-3.0-or-later

//! The operating system's view of the adapters, and the one change made
//! through it: adding an address to an adapter with the system's own
//! authorization prompt. Nothing else on the computer is touched.
//!
//! Each system is read through its own tools: `ifconfig`, `networksetup`
//! and `route` on macOS; `ip` on Linux; PowerShell on Windows. The
//! parsers are pure so the samples in the tests pin what each tool says.

use crate::{Adapter, Address};
use std::net::Ipv4Addr;
use std::process::Command;

/// Why the system could not be read or changed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A system tool failed or answered something unexpected.
    #[error("{0}")]
    Unavailable(String),
    /// The authorization prompt was refused or dismissed.
    #[error("authorization was refused")]
    Denied,
    /// This operating system is not supported.
    #[error("network setup is not supported on this operating system")]
    Unsupported,
}

/// The computer's network, read and changed.
pub trait Os: Send + Sync + std::fmt::Debug {
    /// Every adapter, as the system reports it.
    fn adapters(&self) -> Result<Vec<Adapter>, Error>;
    /// Adds `address` to `adapter`, asking the system for authorization.
    fn add_address(&self, adapter: &str, address: &Address) -> Result<(), Error>;
    /// Whether `ip` would be reached through `adapter`.
    fn routes_through(&self, ip: Ipv4Addr, adapter: &str) -> Result<bool, Error>;
}

/// The running system.
#[derive(Clone, Copy, Debug, Default)]
pub struct Live;

fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn run(program: &str, args: &[&str]) -> Result<String, Error> {
    let output = command(program)
        .args(args)
        .output()
        .map_err(|e| Error::Unavailable(format!("{program}: {e}")))?;
    checked_output(program, &output)
}

fn checked_output(program: &str, output: &std::process::Output) -> Result<String, Error> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Unavailable(format!("{program} failed: {}", stderr.trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
impl Os for Live {
    fn adapters(&self) -> Result<Vec<Adapter>, Error> {
        let ports =
            macos::hardware_ports(&run("/usr/sbin/networksetup", &["-listallhardwareports"])?);
        let route = macos::route_interface(
            &run("/sbin/route", &["-n", "get", "default"]).unwrap_or_default(),
        );
        Ok(macos::adapters(&run("/sbin/ifconfig", &["-a"])?, &ports, route.as_deref()))
    }

    fn add_address(&self, adapter: &str, address: &Address) -> Result<(), Error> {
        // The system's own password prompt, as the vendor's IPSet.exe
        // asks for administrator rights; the alias leaves whatever the
        // adapter already has.
        let script = macos::add_address_script(adapter, address)?;
        match run("/usr/bin/osascript", &["-e", &script]) {
            Ok(_) => Ok(()),
            Err(Error::Unavailable(text)) if text.contains("-128") => Err(Error::Denied),
            Err(error) => Err(error),
        }
    }

    fn routes_through(&self, ip: Ipv4Addr, adapter: &str) -> Result<bool, Error> {
        let route = run("/sbin/route", &["-n", "get", &ip.to_string()])?;
        Ok(macos::route_interface(&route).as_deref() == Some(adapter))
    }
}

#[cfg(target_os = "linux")]
impl Os for Live {
    fn adapters(&self) -> Result<Vec<Adapter>, Error> {
        let route = linux::route_device(
            &run("ip", &["-j", "route", "show", "default"]).unwrap_or_default(),
        );
        linux::adapters(&run("ip", &["-j", "addr"])?, route.as_deref())
    }

    fn add_address(&self, adapter: &str, address: &Address) -> Result<(), Error> {
        if address.netmask().is_none() {
            return Err(Error::Unavailable("invalid IPv4 prefix".into()));
        }
        // polkit's prompt; 126 and 127 are its refusals.
        let output = Command::new("pkexec")
            .args(["ip", "addr", "add", &address.to_string(), "dev", adapter])
            .output()
            .map_err(|e| Error::Unavailable(format!("pkexec: {e}")))?;
        match output.status.code() {
            Some(0) => Ok(()),
            Some(126 | 127) => Err(Error::Denied),
            _ => Err(Error::Unavailable(String::from_utf8_lossy(&output.stderr).trim().to_owned())),
        }
    }

    fn routes_through(&self, ip: Ipv4Addr, adapter: &str) -> Result<bool, Error> {
        let route = run("ip", &["-j", "route", "get", &ip.to_string()])?;
        Ok(linux::route_device(&route).as_deref() == Some(adapter))
    }
}

#[cfg(target_os = "windows")]
impl Os for Live {
    fn adapters(&self) -> Result<Vec<Adapter>, Error> {
        let script = "$a = Get-NetAdapter | Select-Object Name, InterfaceDescription, MacAddress, Status, MediaType, Virtual, ifIndex; $i = Get-NetIPAddress -AddressFamily IPv4 | Select-Object InterfaceIndex, IPAddress, PrefixLength; $r = Get-NetRoute -DestinationPrefix 0.0.0.0/0 -ErrorAction SilentlyContinue | Select-Object InterfaceIndex; @{ adapters = @($a); addresses = @($i); routes = @($r) } | ConvertTo-Json -Depth 3";
        windows::adapters(&run(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", script],
        )?)
    }

    fn add_address(&self, adapter: &str, address: &Address) -> Result<(), Error> {
        let script = windows::add_address_script(adapter, address)?;
        let output = command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| Error::Unavailable(format!("powershell: {e}")))?;
        match output.status.code() {
            Some(0) => Ok(()),
            Some(1223) => Err(Error::Denied),
            Some(2) => Err(Error::Unavailable(format!(
                "Windows did not finish adding {address} to {adapter}; check this adapter's address and DHCP state"
            ))),
            _ => Err(Error::Unavailable(format!(
                "PowerShell elevation failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))),
        }
    }

    fn routes_through(&self, ip: Ipv4Addr, adapter: &str) -> Result<bool, Error> {
        let script = format!("(Find-NetRoute -RemoteIPAddress {ip})[1].InterfaceAlias");
        Ok(run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &script])?.trim()
            == adapter)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
impl Os for Live {
    fn adapters(&self) -> Result<Vec<Adapter>, Error> {
        Err(Error::Unsupported)
    }

    fn add_address(&self, _adapter: &str, _address: &Address) -> Result<(), Error> {
        Err(Error::Unsupported)
    }

    fn routes_through(&self, _ip: Ipv4Addr, _adapter: &str) -> Result<bool, Error> {
        Err(Error::Unsupported)
    }
}

/// A scripted system, for tests and for running the workflow off the
/// machine.
#[derive(Debug, Default)]
pub struct Fake {
    state: std::sync::Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    adapters: Vec<Adapter>,
    deny: bool,
    added: Vec<(String, Address)>,
}

impl Fake {
    /// A system with these adapters.
    #[must_use]
    pub fn with(adapters: Vec<Adapter>) -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState { adapters, deny: false, added: Vec::new() }),
        }
    }

    /// Refuses every authorization prompt.
    pub fn deny(&self) {
        self.lock().deny = true;
    }

    /// The addresses added so far.
    #[must_use]
    pub fn added(&self) -> Vec<(String, Address)> {
        self.lock().added.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Os for Fake {
    fn adapters(&self) -> Result<Vec<Adapter>, Error> {
        Ok(self.lock().adapters.clone())
    }

    fn add_address(&self, adapter: &str, address: &Address) -> Result<(), Error> {
        if address.netmask().is_none() {
            return Err(Error::Unavailable("invalid IPv4 prefix".into()));
        }
        let mut state = self.lock();
        if state.deny {
            return Err(Error::Denied);
        }
        let found = state
            .adapters
            .iter_mut()
            .find(|a| a.name == adapter)
            .ok_or_else(|| Error::Unavailable(format!("no adapter {adapter}")))?;
        found.addresses.push(*address);
        state.added.push((adapter.to_owned(), *address));
        Ok(())
    }

    fn routes_through(&self, ip: Ipv4Addr, adapter: &str) -> Result<bool, Error> {
        Ok(self
            .lock()
            .adapters
            .iter()
            .any(|a| a.name == adapter && a.addresses.iter().any(|x| x.contains(ip))))
    }
}

fn address(
    ip: Option<&serde_json::Value>,
    prefix: Option<&serde_json::Value>,
) -> Result<Address, Error> {
    let invalid = || {
        Error::Unavailable(
            "the adapter inventory contains an invalid IPv4 address or prefix".into(),
        )
    };
    Ok(Address {
        ip: ip.and_then(|ip| ip.as_str()).and_then(|ip| ip.parse().ok()).ok_or_else(invalid)?,
        prefix: prefix
            .and_then(serde_json::Value::as_u64)
            .and_then(|prefix| u8::try_from(prefix).ok())
            .filter(|prefix| *prefix <= 32)
            .ok_or_else(invalid)?,
    })
}

/// The macOS tools' output.
pub mod macos {
    use super::{Adapter, Address, Error};
    use std::net::Ipv4Addr;

    /// Build the privileged alias command for an interface identifier, never shell text.
    pub fn add_address_script(adapter: &str, address: &Address) -> Result<String, Error> {
        if !adapter.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            || adapter.len() > 64
            || !adapter
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        {
            return Err(Error::Unavailable("invalid network adapter name".into()));
        }
        let mask =
            address.netmask().ok_or_else(|| Error::Unavailable("invalid IPv4 prefix".into()))?;
        Ok(format!(
            "do shell script \"/sbin/ifconfig '{adapter}' alias {} {mask}\" with administrator privileges",
            address.ip
        ))
    }

    /// Hardware port names by device from `networksetup
    /// -listallhardwareports`.
    #[must_use]
    pub fn hardware_ports(text: &str) -> Vec<(String, String)> {
        let mut ports = Vec::new();
        let mut port: Option<String> = None;
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port:") {
                port = Some(name.trim().to_owned());
            } else if let Some(device) = line.strip_prefix("Device:")
                && let Some(name) = port.take()
            {
                ports.push((device.trim().to_owned(), name));
            }
        }
        ports
    }

    /// The interface a `route -n get` answer names.
    #[must_use]
    pub fn route_interface(text: &str) -> Option<String> {
        text.lines().find_map(|l| l.trim().strip_prefix("interface:").map(|s| s.trim().to_owned()))
    }

    /// The adapters in `ifconfig -a`, wired when their hardware port is an
    /// Ethernet or LAN port.
    #[must_use]
    pub fn adapters(
        text: &str,
        ports: &[(String, String)],
        default_route: Option<&str>,
    ) -> Vec<Adapter> {
        let mut adapters: Vec<Adapter> = Vec::new();
        for line in text.lines() {
            if !line.starts_with(|c: char| c.is_whitespace()) {
                let Some((name, _)) = line.split_once(':') else { continue };
                let port =
                    ports.iter().find(|(device, _)| device == name).map(|(_, port)| port.clone());
                let wired =
                    port.as_deref().is_some_and(|p| p.contains("Ethernet") || p.contains("LAN"));
                adapters.push(Adapter {
                    name: name.to_owned(),
                    description: port.unwrap_or_default(),
                    mac: None,
                    wired,
                    up: false,
                    addresses: Vec::new(),
                    default_route: default_route == Some(name),
                });
                continue;
            }
            let Some(adapter) = adapters.last_mut() else { continue };
            let line = line.trim();
            if let Some(mac) = line.strip_prefix("ether ") {
                adapter.mac = Some(mac.trim().to_ascii_lowercase());
            } else if line == "status: active" {
                adapter.up = true;
            } else if let Some(rest) = line.strip_prefix("inet ") {
                let mut words = rest.split_whitespace();
                let ip = words.next().and_then(|w| w.parse::<Ipv4Addr>().ok());
                let mask = words
                    .skip_while(|w| *w != "netmask")
                    .nth(1)
                    .and_then(|m| u32::from_str_radix(m.trim_start_matches("0x"), 16).ok());
                if let (Some(ip), Some(mask)) = (ip, mask) {
                    adapter
                        .addresses
                        .push(Address { ip, prefix: mask.count_ones().try_into().unwrap_or(32) });
                }
            }
        }
        adapters.retain(|a| a.name != "lo0");
        adapters
    }
}

/// The `ip -j` JSON.
pub mod linux {
    use super::{Adapter, Error, address};

    /// The device the default route leaves through.
    #[must_use]
    pub fn route_device(json: &str) -> Option<String> {
        let routes: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
        routes.first()?.get("dev")?.as_str().map(str::to_owned)
    }

    /// The adapters in `ip -j addr`; wired when the link is Ethernet and
    /// the name is not a wireless, virtual or container interface.
    pub fn adapters(json: &str, default_route: Option<&str>) -> Result<Vec<Adapter>, Error> {
        let links: Vec<serde_json::Value> =
            serde_json::from_str(json).map_err(|e| Error::Unavailable(format!("ip: {e}")))?;
        let virtual_prefixes = ["wl", "ww", "veth", "docker", "br", "virbr", "tun", "tap", "lo"];
        let mut adapters = Vec::new();
        for link in &links {
            let Some(name) = link.get("ifname").and_then(|name| name.as_str()) else { continue };
            if name == "lo" {
                continue;
            }
            let addresses = link
                .get("addr_info")
                .and_then(|a| a.as_array())
                .into_iter()
                .flatten()
                .filter(|i| i.get("family").and_then(|f| f.as_str()) == Some("inet"))
                .map(|i| address(i.get("local"), i.get("prefixlen")))
                .collect::<Result<_, _>>()?;
            let ether = link.get("link_type").and_then(|t| t.as_str()) == Some("ether");
            adapters.push(Adapter {
                wired: ether && !virtual_prefixes.iter().any(|p| name.starts_with(p)),
                up: link.get("operstate").and_then(|s| s.as_str()) == Some("UP"),
                mac: link.get("address").and_then(|m| m.as_str()).map(str::to_ascii_lowercase),
                description: String::new(),
                default_route: default_route == Some(name),
                name: name.to_owned(),
                addresses,
            });
        }
        Ok(adapters)
    }
}

/// The PowerShell JSON.
pub mod windows {
    use super::{Adapter, Address, Error, address};

    fn literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    /// An elevated command with literal adapter identity and an explicit
    /// child result. New-NetIPAddress itself disables DHCP on that adapter.
    /// `EncodedCommand` takes UTF-16LE, produced here by .NET's Unicode codec.
    pub fn add_address_script(adapter: &str, address: &Address) -> Result<String, Error> {
        if address.netmask().is_none() {
            return Err(Error::Unavailable("invalid IPv4 prefix".into()));
        }
        let inner = format!(
            "$ErrorActionPreference = 'Stop'; try {{ New-NetIPAddress -InterfaceAlias {} -AddressFamily IPv4 -IPAddress {} -PrefixLength {} -ErrorAction Stop | Out-Null; exit 0 }} catch {{ exit 2 }}",
            literal(adapter),
            address.ip,
            address.prefix
        );
        Ok(format!(
            "$ErrorActionPreference = 'Stop'; try {{ $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes({})); $p = Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand',$encoded -ErrorAction Stop; exit $p.ExitCode }} catch {{ if ($_.Exception.NativeErrorCode -eq 1223) {{ exit 1223 }}; [Console]::Error.WriteLine($_); exit 1 }}",
            literal(&inner)
        ))
    }

    /// The adapters from the `adapters`, `addresses` and `routes` lists
    /// the inventory script prints; wired when the media type is 802.3
    /// and the adapter is not virtual.
    pub fn adapters(json: &str) -> Result<Vec<Adapter>, Error> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| Error::Unavailable(format!("powershell: {e}")))?;
        let list =
            |key: &str| value.get(key).and_then(|v| v.as_array()).map_or(&[][..], Vec::as_slice);
        let index_of =
            |v: &serde_json::Value, key: &str| v.get(key).and_then(serde_json::Value::as_u64);
        let addresses = list("addresses");
        let routes = list("routes");
        let mut adapters = Vec::new();
        for a in list("adapters") {
            let Some(index) = index_of(a, "ifIndex") else { continue };
            let Some(name) = a.get("Name").and_then(|name| name.as_str()) else { continue };
            let media = a.get("MediaType");
            adapters.push(Adapter {
                description: a
                    .get("InterfaceDescription")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                mac: a
                    .get("MacAddress")
                    .and_then(|m| m.as_str())
                    .map(|m| m.replace('-', ":").to_ascii_lowercase()),
                wired: (media.and_then(|m| m.as_str()) == Some("802.3")
                    || media.and_then(serde_json::Value::as_u64) == Some(0))
                    && !a.get("Virtual").and_then(serde_json::Value::as_bool).unwrap_or(false),
                up: a.get("Status").and_then(|s| s.as_str()) == Some("Up"),
                addresses: addresses
                    .iter()
                    .filter(|x| index_of(x, "InterfaceIndex") == Some(index))
                    .map(|x| address(x.get("IPAddress"), x.get("PrefixLength")))
                    .collect::<Result<_, _>>()?,
                default_route: routes.iter().any(|r| index_of(r, "InterfaceIndex") == Some(index)),
                name: name.to_owned(),
            });
        }
        Ok(adapters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_alias_commands_accept_only_literal_interface_identifiers() {
        let address = Address { ip: Ipv4Addr::new(10, 1, 1, 10), prefix: 24 };
        assert_eq!(
            macos::add_address_script("en9", &address).unwrap(),
            "do shell script \"/sbin/ifconfig 'en9' alias 10.1.1.10 255.255.255.0\" with administrator privileges"
        );
        for adapter in ["", "-a", "en0;whoami", "en0$(whoami)", "en0\n", "en0\"", "en0'", "en 0"] {
            assert!(macos::add_address_script(adapter, &address).is_err(), "{adapter:?}");
        }
        assert!(macos::add_address_script("en9", &Address { prefix: 33, ..address }).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn command_output_preserves_success_text_and_reports_failure_stderr() {
        use std::os::unix::process::ExitStatusExt;
        let mut output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"ok\n".to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(checked_output("inventory", &output).unwrap(), "ok\n");
        output.status = std::process::ExitStatus::from_raw(256);
        output.stderr = b" permission refused \n".to_vec();
        assert_eq!(
            checked_output("inventory", &output).unwrap_err(),
            Error::Unavailable("inventory failed: permission refused".into())
        );
    }

    #[test]
    fn windows_adapter_names_remain_literal_through_both_command_layers() {
        let address = Address { ip: Ipv4Addr::new(10, 1, 1, 10), prefix: 24 };
        for alias in ["Ethernet 2", "Machinist's cable", "Cable \"A\" $name ` $(exit 9)"] {
            let script = windows::add_address_script(alias, &address).unwrap();
            let inner = script
                .split("Unicode.GetBytes('")
                .nth(1)
                .unwrap()
                .split("')); $p =")
                .next()
                .unwrap()
                .replace("''", "'");
            assert!(inner.contains(&format!(
                "-InterfaceAlias '{}' -AddressFamily",
                alias.replace('\'', "''")
            )));
            assert!(inner.ends_with("exit 0 } catch { exit 2 }"));
            assert!(script.contains("'-EncodedCommand',$encoded"));
            assert!(script.contains("-Verb RunAs -WindowStyle Hidden"));
            assert!(script.contains("exit $p.ExitCode"));
            assert!(!inner.contains("Set-NetIPInterface"));
        }
        assert!(windows::add_address_script("cable", &Address { prefix: 99, ..address }).is_err());
    }

    #[test]
    fn inventory_rejects_invalid_prefixes_and_accepts_numeric_windows_media() {
        let linux = r#"[{"ifname":"eth0","link_type":"ether","operstate":"UP","addr_info":[{"family":"inet","local":"10.1.1.10","prefixlen":99}]}]"#;
        assert!(linux::adapters(linux, None).is_err());
        let windows = r#"{"adapters":[{"Name":"Ethernet","MediaType":0,"Virtual":false,"Status":"Up","ifIndex":12}],"addresses":[{"InterfaceIndex":12,"IPAddress":"10.1.1.10","PrefixLength":24}],"routes":[]}"#;
        assert!(windows::adapters(windows).unwrap()[0].wired);
        assert!(
            windows::adapters(&windows.replace("\"PrefixLength\":24", "\"PrefixLength\":99"))
                .is_err()
        );
    }

    /// A Mac with Wi-Fi carrying the default route and a USB Ethernet
    /// adapter with its link up reads as one wired candidate with its
    /// self-assigned address.
    #[test]
    fn macos_tools_are_read() {
        let ifconfig = "lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384\n\tinet 127.0.0.1 netmask 0xff000000\nen0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tether 60:3e:5f:5a:c3:61\n\tinet 192.168.1.20 netmask 0xffffff00 broadcast 192.168.1.255\n\tstatus: active\nen9: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tether 98:FC:84:E9:1E:7D\n\tinet 169.254.7.8 netmask 0xffff0000 broadcast 169.254.255.255\n\tstatus: active\nen4: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\tether 3e:40:8e:aa:fe:3d\n\tstatus: inactive\n";
        let ports = macos::hardware_ports(
            "\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: 60:3e:5f:5a:c3:61\n\nHardware Port: USB 10/100/1G/2.5G LAN\nDevice: en9\nEthernet Address: 98:fc:84:e9:1e:7d\n\nHardware Port: Ethernet Adapter (en4)\nDevice: en4\n",
        );
        let adapters = macos::adapters(ifconfig, &ports, Some("en0"));
        assert_eq!(adapters.len(), 3);
        let wifi = &adapters[0];
        assert!(!wifi.wired && wifi.up && wifi.default_route);
        let usb = &adapters[1];
        assert_eq!(usb.description, "USB 10/100/1G/2.5G LAN");
        assert_eq!(usb.mac.as_deref(), Some("98:fc:84:e9:1e:7d"));
        assert!(usb.wired && usb.up && !usb.default_route);
        assert_eq!(usb.addresses, vec![Address { ip: "169.254.7.8".parse().unwrap(), prefix: 16 }]);
        assert!(adapters[2].wired && !adapters[2].up);
        assert_eq!(
            macos::route_interface("   route to: 10.1.1.168\n  interface: en9\n"),
            Some("en9".into())
        );
    }

    /// The Linux and Windows inventories read the same way, and the fake
    /// system adds addresses only while authorized.
    #[test]
    fn linux_and_windows_inventories_are_read() {
        let ip = r#"[{"ifname":"lo","link_type":"loopback","operstate":"UNKNOWN","addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8}]},{"ifname":"wlp3s0","link_type":"ether","operstate":"UP","address":"aa:bb:cc:dd:ee:01","addr_info":[{"family":"inet","local":"192.168.1.5","prefixlen":24}]},{"ifname":"enp0s31f6","link_type":"ether","operstate":"UP","address":"AA:BB:CC:DD:EE:02","addr_info":[]}]"#;
        let adapters = linux::adapters(ip, Some("wlp3s0")).unwrap();
        assert_eq!(adapters.len(), 2);
        assert!(!adapters[0].wired && adapters[0].default_route);
        assert!(adapters[1].wired && adapters[1].up && adapters[1].addresses.is_empty());
        assert_eq!(adapters[1].mac.as_deref(), Some("aa:bb:cc:dd:ee:02"));
        assert_eq!(
            linux::route_device(r#"[{"dst":"default","gateway":"192.168.1.1","dev":"wlp3s0"}]"#),
            Some("wlp3s0".into())
        );
        let ps = r#"{"adapters":[{"Name":"Wi-Fi","InterfaceDescription":"Intel Wireless","MacAddress":"AA-BB-CC-DD-EE-01","Status":"Up","MediaType":"Native 802.11","Virtual":false,"ifIndex":5},{"Name":"Ethernet 2","InterfaceDescription":"Realtek USB GbE","MacAddress":"AA-BB-CC-DD-EE-02","Status":"Up","MediaType":"802.3","Virtual":false,"ifIndex":12}],"addresses":[{"InterfaceIndex":5,"IPAddress":"192.168.1.5","PrefixLength":24},{"InterfaceIndex":12,"IPAddress":"169.254.2.3","PrefixLength":16}],"routes":[{"InterfaceIndex":5}]}"#;
        let adapters = windows::adapters(ps).unwrap();
        assert_eq!(adapters.len(), 2);
        assert!(!adapters[0].wired && adapters[0].default_route);
        assert_eq!(adapters[1].name, "Ethernet 2");
        assert!(adapters[1].wired && adapters[1].up && !adapters[1].default_route);
        assert_eq!(adapters[1].mac.as_deref(), Some("aa:bb:cc:dd:ee:02"));
        assert!(adapters[1].addresses[0].is_link_local());
        let fake = Fake::with(adapters);
        let address = Address { ip: "10.1.1.10".parse().unwrap(), prefix: 24 };
        fake.add_address("Ethernet 2", &address).unwrap();
        assert!(fake.routes_through("10.1.1.168".parse().unwrap(), "Ethernet 2").unwrap());
        fake.deny();
        assert_eq!(fake.add_address("Ethernet 2", &address), Err(Error::Denied));
        assert_eq!(fake.added().len(), 1);
    }
}
