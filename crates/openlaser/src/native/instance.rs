// SPDX-License-Identifier: GPL-3.0-or-later

//! A process lock keeps a second launch from opening the same library twice.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Instance {
    _lock: File,
    activation: PathBuf,
    last_activation: Option<SystemTime>,
}

impl Instance {
    pub fn acquire(directory: &Path) -> Result<Option<Self>, String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("desktop.lock"))
            .map_err(|e| e.to_string())?;
        let activation = directory.join("desktop.activate");
        match file.try_lock() {
            Ok(()) => {
                let last_activation = modified(&activation);
                Ok(Some(Self { _lock: file, activation, last_activation }))
            }
            Err(TryLockError::WouldBlock) => {
                std::fs::write(activation, std::process::id().to_string())
                    .map_err(|e| e.to_string())?;
                Ok(None)
            }
            Err(TryLockError::Error(error)) => Err(error.to_string()),
        }
    }

    pub fn activated(&mut self) -> bool {
        let current = modified(&self.activation);
        let changed = current.is_some() && current != self.last_activation;
        self.last_activation = current;
        changed
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_activates_the_owner_and_lock_releases_on_exit() -> Result<(), String> {
        let directory = std::env::temp_dir().join(format!("openlaser-lock-{}", std::process::id()));
        let mut first = Instance::acquire(&directory)?.ok_or("first launch was locked")?;
        assert!(!first.activated());
        assert!(Instance::acquire(&directory)?.is_none());
        assert!(first.activated());
        assert!(!first.activated());
        drop(first);
        let second = Instance::acquire(&directory)?.ok_or("lock was not released")?;
        drop(second);
        std::fs::remove_dir_all(directory).map_err(|e| e.to_string())
    }
}
