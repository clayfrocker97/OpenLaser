// SPDX-License-Identifier: GPL-3.0-or-later

//! One durable replacement primitive for library data and host settings.

use crate::{Error, Result};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Replaces a file through a uniquely created temporary sibling, syncing the
/// bytes before rename and the containing directory on Unix. The parent must
/// already exist. Failed writes remove their temporary file when possible.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    write(path, bytes)
        .map_err(|error| Error::Io { path: path.display().to_string(), reason: error.to_string() })
}

fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!(
        "{}-{}-{serial}.tmp",
        crate::Id::generate(),
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let outcome =
        written.and_then(|()| std::fs::rename(&temporary, path)).and_then(|()| sync_parent(path));
    if outcome.is_err() {
        drop(std::fs::remove_file(&temporary));
    }
    outcome
}

#[cfg_attr(
    not(unix),
    allow(clippy::unnecessary_wraps, reason = "directory syncing is Unix-specific")
)]
pub(crate) fn sync_parent(path: &Path) -> std::io::Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_and_failure_leave_no_partial_files() {
        let dir = std::env::temp_dir().join(format!("openlaser-atomic-{}", crate::Id::generate()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("settings.json");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        let blocked = dir.join("directory.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(atomic_write(&blocked, b"cannot replace a directory").is_err());
        assert!(blocked.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert!(atomic_write(&dir.join("missing/child.json"), b"x").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
