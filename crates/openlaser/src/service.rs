// SPDX-License-Identifier: GPL-3.0-or-later

//! One server owns each data directory; a native window closes it on exit.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
struct Record {
    address: SocketAddr,
    pid: u32,
}

pub(crate) struct Instance {
    _lock: File,
    record: PathBuf,
}

impl Instance {
    pub fn acquire(directory: &Path) -> Result<Option<Self>, String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let lock = lock_file(directory)?;
        match lock.try_lock() {
            Ok(()) => {
                let record = directory.join("server.json");
                match std::fs::remove_file(&record) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("clear previous server address: {e}")),
                }
                Ok(Some(Self { _lock: lock, record }))
            }
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error.to_string()),
        }
    }

    pub fn ready(&self, address: SocketAddr) -> Result<(), String> {
        let bytes =
            serde_json::to_vec(&Record { address: loopback(address), pid: std::process::id() })
                .map_err(|e| e.to_string())?;
        let temporary = self.record.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(temporary, &self.record))
            .map_err(|e| format!("save server address: {e}"))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.record);
    }
}

fn lock_file(directory: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("server.lock"))
        .map_err(|e| e.to_string())
}

fn current(directory: &Path) -> Result<Option<SocketAddr>, String> {
    let lock = lock_file(directory)?;
    match lock.try_lock() {
        Ok(()) => Ok(None),
        Err(TryLockError::WouldBlock) => {
            let bytes = match std::fs::read(directory.join("server.json")) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(format!("read server address: {e}")),
            };
            let record: Record =
                serde_json::from_slice(&bytes).map_err(|e| format!("read server address: {e}"))?;
            Ok(Some(record.address))
        }
        Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
}

pub(crate) fn wait(directory: &Path) -> Result<SocketAddr, String> {
    for _ in 0..150 {
        if let Some(address) = current(directory)? {
            return Ok(address);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("The OpenLaser server did not become ready. Check server.log in the data directory.".into())
}

pub(crate) fn shutdown(directory: &Path) -> Result<(), String> {
    let address =
        current(directory)?.ok_or("No OpenLaser server is running for this data directory.")?;
    let mut connection =
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    connection.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    connection.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    write!(connection, "POST /api/access/shutdown HTTP/1.1\r\nHost: {address}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut response = String::new();
    connection.read_to_string(&mut response).map_err(|e| e.to_string())?;
    if response.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err("The server refused shutdown.".into())
    }
}

fn loopback(mut address: SocketAddr) -> SocketAddr {
    if !address.ip().is_loopback() {
        address.set_ip(if address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        });
    }
    address
}

#[cfg(feature = "desktop")]
pub(crate) struct DesktopSession {
    address: SocketAddr,
    directory: PathBuf,
    child: Option<std::process::Child>,
    closed: bool,
}

#[cfg(feature = "desktop")]
impl DesktopSession {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn close(mut self) -> Result<(), String> {
        self.stop()
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if current(&self.directory)?.is_some() {
            shutdown(&self.directory)?;
        }
        // The server allows up to 30 seconds for controller cleanup, followed
        // by HTTP draining. The desktop must outwait that shutdown budget.
        for _ in 0..350 {
            let exited = self.child.as_mut().map_or(Ok(true), |child| {
                child.try_wait().map(|status| status.is_some()).map_err(|e| e.to_string())
            })?;
            if exited && current(&self.directory)?.is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("OpenLaser requested shutdown, but the Rust service did not exit. Check server.log."
            .into())
    }
}

#[cfg(feature = "desktop")]
impl Drop for DesktopSession {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            tracing::error!(%error, "closing the desktop server");
        }
    }
}

#[cfg(feature = "desktop")]
pub(crate) fn ensure(
    arguments: &crate::Arguments,
    config: &openlaser_server::Config,
) -> Result<DesktopSession, String> {
    if let Some(address) = current(&config.data_dir)? {
        return Ok(DesktopSession {
            address,
            directory: config.data_dir.clone(),
            child: None,
            closed: false,
        });
    }
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .open(config.data_dir.join("server.log"))
        .map_err(|e| e.to_string())?;
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("--no-browser")
        .arg("--data")
        .arg(&config.data_dir)
        .arg("--listen")
        .arg(config.listen.to_string());
    if arguments.config_explicit || arguments.config.exists() {
        command.arg("--config").arg(&arguments.config);
    }
    if arguments.simulate {
        command.arg("--simulate");
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| format!("start server: {e}"))?;
    for _ in 0..200 {
        if let Some(address) = current(&config.data_dir)? {
            return Ok(DesktopSession {
                address,
                directory: config.data_dir.clone(),
                child: Some(child),
                closed: false,
            });
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "The server stopped ({status}). Check server.log in the data directory."
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // No listener became ready, so no UI could have admitted machine work.
    let _ = child.kill();
    let _ = child.wait();
    Err("The server did not become ready. Check server.log in the data directory.".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "isolated server ownership fixture")]
mod tests {
    use super::*;

    #[test]
    fn one_server_owns_the_directory_and_stale_records_do_not_imply_a_server() {
        let directory =
            std::env::temp_dir().join(format!("openlaser-server-lock-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("server.json"), b"stale").unwrap();
        assert_eq!(current(&directory).unwrap(), None);
        let instance = Instance::acquire(&directory).unwrap().unwrap();
        assert!(Instance::acquire(&directory).unwrap().is_none());
        assert!(!directory.join("server.json").exists());
        instance.ready("192.0.2.1:9000".parse().unwrap()).unwrap();
        assert_eq!(current(&directory).unwrap(), Some("127.0.0.1:9000".parse().unwrap()));
        drop(instance);
        assert!(!directory.join("server.json").exists());
        assert!(Instance::acquire(&directory).unwrap().is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn desktop_close_and_window_setup_failure_both_wait_for_service_exit() {
        use std::net::TcpListener;
        for explicit_close in [true, false] {
            let directory = std::env::temp_dir()
                .join(format!("openlaser-window-close-{}-{explicit_close}", std::process::id()));
            let instance = Instance::acquire(&directory).unwrap().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            instance.ready(address).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 4096);
                }
                assert!(request.starts_with(b"POST /api/access/shutdown HTTP/1.1\r\n"));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
                drop(stream);
                // A shutdown response is not process completion: keep the
                // server lock until its simulated cleanup has finished.
                std::thread::sleep(Duration::from_millis(150));
                drop(instance);
                drop(listener);
            });
            let session = DesktopSession {
                address,
                directory: directory.clone(),
                child: None,
                closed: false,
            };
            if explicit_close {
                session.close().unwrap();
            } else {
                drop(session);
            }
            assert!(current(&directory).unwrap().is_none());
            assert!(TcpStream::connect(address).is_err());
            server.join().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}
