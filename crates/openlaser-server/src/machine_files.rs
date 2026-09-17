// SPDX-License-Identifier: GPL-3.0-or-later

//! A machine backup is staged and validated before its active reference is
//! atomically replaced. Only managed, superseded backup files are pruned.

use crate::bindings::{self, Loaded};
use crate::document::FileView;
use crate::{Error, Result};
use openlaser_core::LaserMode;
use openlaser_library::{atomic_write, sha256};
use openlaser_xml::{Bundle, Document, Kind, bindings as vendor};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Active {
    file: String,
    name: String,
}

pub(crate) struct Candidate {
    pub loaded: Loaded,
    pub bound: vendor::Bindings,
    active: Active,
    dir: PathBuf,
}

fn managed(name: &str) -> bool {
    name.strip_prefix("backup-")
        .and_then(|s| s.strip_suffix(".xml"))
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn active(dir: &Path) -> std::result::Result<Option<Active>, String> {
    let bytes = match std::fs::read(dir.join("active.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("machine backup reference: {error}")),
    };
    let active: Active =
        serde_json::from_slice(&bytes).map_err(|e| format!("machine backup reference: {e}"))?;
    if !managed(&active.file) || !dir.join(&active.file).is_file() {
        return Err("the active machine backup is missing or has an invalid filename".into());
    }
    Ok(Some(active))
}

pub(crate) fn imported(dir: &Path) -> std::result::Result<Option<PathBuf>, String> {
    if let Some(active) = active(dir)? {
        return Ok(Some(dir.join(active.file)));
    }
    // Legacy imports predate the reference. Uncommitted managed candidates
    // must never displace one merely because their filename sorts first.
    let mut files: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"))
                    && !p.file_name().and_then(|n| n.to_str()).is_some_and(managed)
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("machine backups: {error}")),
    };
    files.sort();
    Ok(files.into_iter().next())
}

pub(crate) fn display_name(path: &Path, hash: &str) -> std::result::Result<String, String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if managed(name) {
        if name != format!("backup-{hash}.xml") {
            return Err("the stored machine backup is corrupt".into());
        }
        if let Some(parent) = path.parent()
            && let Some(active) = active(parent)?
            && active.file == name
        {
            return Ok(active.name);
        }
    }
    Ok(name.to_owned())
}

pub(crate) fn stage(
    dir: PathBuf,
    file_name: &str,
    bytes: &[u8],
    mode: Option<LaserMode>,
    scale: i32,
) -> Result<Candidate> {
    let name = Path::new(file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.to_ascii_lowercase().ends_with(".xml"))
        .ok_or_else(|| {
            Error::Request(format!(
                "{file_name} is not a machine backup; import the vendor's 1390backup.xml"
            ))
        })?;
    let mut bundle = Bundle::default();
    bundle.insert(Document::parse(Kind::Backup, bytes).map_err(|e| Error::Request(e.to_string()))?);
    let mode = mode.or_else(|| bindings::saved_mode(&bundle)).unwrap_or(LaserMode::Fiber);
    let bound = vendor::bind(&bundle, mode, scale).map_err(|error| match error {
        openlaser_xml::Error::Missing(field) => {
            Error::Request(format!("{name} has no {field}; it is not a complete machine backup"))
        }
        other => Error::Request(format!("{name}: {other}")),
    })?;
    let hash = sha256(bytes);
    let file = FileView { name: name.to_owned(), bytes: bytes.len() as u64, sha256: hash.clone() };
    let active = Active { file: format!("backup-{hash}.xml"), name: name.to_owned() };
    std::fs::create_dir_all(&dir).map_err(|e| Error::Refused(e.to_string()))?;
    let path = dir.join(&active.file);
    match std::fs::read(&path) {
        Ok(stored) if stored == bytes => {}
        Ok(_) => return Err(Error::Refused("the stored machine backup is corrupt".into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => atomic_write(&path, bytes)?,
        Err(error) => return Err(Error::Refused(error.to_string())),
    }
    Ok(Candidate { loaded: Loaded { bundle, file }, bound, active, dir })
}

impl Candidate {
    pub fn commit(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.active).map_err(|e| Error::Refused(e.to_string()))?;
        Ok(atomic_write(&self.dir.join("active.json"), &bytes)?)
    }

    pub fn pruning(&self) -> impl FnOnce() + Send + 'static {
        let dir = self.dir.clone();
        let current = self.active.file.clone();
        move || {
            let Ok(entries) = std::fs::read_dir(&dir) else { return };
            for entry in entries.filter_map(std::result::Result::ok) {
                let name = entry.file_name();
                if name != current.as_str()
                    && name.to_str().is_some_and(managed)
                    && entry.path().is_file()
                    && let Err(error) = std::fs::remove_file(entry.path())
                {
                    tracing::warn!(%error, "could not remove a superseded managed backup");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_and_failed_commit_preserve_the_active_backup_and_unrelated_files() {
        let serial =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("openlaser-backup-{serial}"));
        let bytes = include_bytes!("../../../fixtures/xml/harness-backup.xml");
        let old = stage(dir.clone(), "original.xml", bytes, Some(LaserMode::Fiber), 1000).unwrap();
        old.commit().unwrap();
        let original = imported(&dir).unwrap().unwrap();
        std::fs::write(dir.join("operator-notes.xml"), b"unrelated").unwrap();
        let mut replacement = bytes.to_vec();
        replacement.push(b'\n');
        let next =
            stage(dir.clone(), "replacement.xml", &replacement, Some(LaserMode::Fiber), 1000)
                .unwrap();
        assert_eq!(imported(&dir).unwrap().unwrap(), original, "staging does not activate");
        assert!(
            stage(dir.clone(), "broken.xml", b"<Root/>", Some(LaserMode::Fiber), 1000).is_err()
        );
        std::fs::rename(dir.join("active.json"), dir.join("saved-reference.json")).unwrap();
        std::fs::create_dir(dir.join("active.json")).unwrap();
        assert!(next.commit().is_err(), "an unreplaceable reference refuses the commit");
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        std::fs::remove_dir(dir.join("active.json")).unwrap();
        std::fs::rename(dir.join("saved-reference.json"), dir.join("active.json")).unwrap();
        assert_eq!(imported(&dir).unwrap().unwrap(), original);
        next.commit().unwrap();
        assert!(original.exists(), "pruning happens after activation");
        (next.pruning())();
        assert!(!original.exists());
        assert_eq!(std::fs::read(dir.join("operator-notes.xml")).unwrap(), b"unrelated");
        let current = imported(&dir).unwrap().unwrap();
        assert_eq!(display_name(&current, &sha256(&replacement)).unwrap(), "replacement.xml");
        assert!(display_name(&current, &sha256(b"corrupt")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
