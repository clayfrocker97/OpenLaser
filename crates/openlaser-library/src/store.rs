// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned JSON files, written atomically.

pub(crate) use crate::atomic_write as write_bytes;
use crate::storage::sync_parent;
use crate::{Error, Result, VERSION};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::path::Path;

/// The file envelope: the schema version beside the item's own fields.
#[derive(Serialize, serde::Deserialize)]
struct Envelope<T> {
    version: u32,
    #[serde(flatten)]
    item: T,
}

fn io(path: &Path, error: &std::io::Error) -> Error {
    Error::Io { path: path.display().to_string(), reason: error.to_string() }
}

pub(crate) fn create_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| io(path, &e))
}

pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| io(path, &e))
}

/// Writes `bytes` unless the file already exists with the same content:
/// originals are named by their hash and never change.
pub(crate) fn write_bytes_once(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return if read_bytes(path)? == bytes {
            Ok(())
        } else {
            Err(Error::Format {
                path: path.display().to_string(),
                reason: "stored content differs from its hash-named original".into(),
            })
        };
    }
    write_bytes(path, bytes)
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    std::fs::remove_file(path).and_then(|()| sync_parent(path)).map_err(|e| io(path, &e))
}

/// Writes an item with this build's schema version.
pub(crate) fn write<T: Serialize>(path: &Path, item: &T) -> Result<()> {
    write_bytes(path, &encode(path, item)?)
}

pub(crate) fn encode<T: Serialize>(path: &Path, item: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(&Envelope { version: VERSION, item })
        .map_err(|e| Error::Format { path: path.display().to_string(), reason: e.to_string() })
}

/// Reads an item, migrating an older schema and refusing a newer one.
pub(crate) fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_bytes(path)?;
    decode(path, &bytes)
}

pub(crate) fn decode<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T> {
    let format = |reason: String| Error::Format { path: path.display().to_string(), reason };
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format(e.to_string()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format("no schema version".into()))?;
    if version > VERSION {
        return Err(Error::Newer { path: path.display().to_string(), version });
    }
    if version != VERSION {
        return Err(format(format!("unsupported schema version {version}")));
    }
    let envelope: Envelope<T> = serde_json::from_value(value).map_err(|e| format(e.to_string()))?;
    Ok(envelope.item)
}

/// Reads an item that may not exist yet.
pub(crate) fn read_optional<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if path.exists() { read(path).map(Some) } else { Ok(None) }
}

/// Reads every `.json` item in a directory, keyed by id.
pub(crate) fn read_all<T: DeserializeOwned + HasId>(dir: &Path) -> Result<BTreeMap<crate::Id, T>> {
    let mut items = BTreeMap::new();
    let entries = std::fs::read_dir(dir).map_err(|e| io(dir, &e))?;
    for entry in entries {
        let path = entry.map_err(|e| io(dir, &e))?.path();
        if path.extension().is_some_and(|x| x == "json") {
            let item: T = read(&path)?;
            crate::validation::id(item.id()).map_err(|e| Error::Format {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;
            if path.file_stem().and_then(|s| s.to_str()) != Some(item.id().as_str()) {
                return Err(Error::Format {
                    path: path.display().to_string(),
                    reason: "file name and item id differ".into(),
                });
            }
            if items.insert(item.id().clone(), item).is_some() {
                return Err(Error::Format {
                    path: path.display().to_string(),
                    reason: "duplicate item id".into(),
                });
            }
        }
    }
    Ok(items)
}

/// A validated item becomes visible only after its file has been replaced.
pub(crate) fn commit<T: Serialize + Clone + HasId>(
    path: &Path,
    items: &mut BTreeMap<crate::Id, T>,
    item: T,
) -> Result<T> {
    crate::history::write(path, &item)?;
    items.insert(item.id().clone(), item.clone());
    Ok(item)
}

/// Stage an edit without mutating the live record or allowing its identity to change.
pub(crate) fn edited<T: Clone + HasId>(current: &T, change: impl FnOnce(&mut T)) -> Result<T> {
    let mut candidate = current.clone();
    change(&mut candidate);
    crate::validation::same_id(current.id(), candidate.id())?;
    Ok(candidate)
}

pub(crate) fn read_content(path: &Path, expected: &str) -> Result<Vec<u8>> {
    let bytes = read_bytes(path)?;
    if !crate::sha256(&bytes).eq_ignore_ascii_case(expected) {
        return Err(Error::Format {
            path: path.display().to_string(),
            reason: "content does not match its SHA-256 name".into(),
        });
    }
    Ok(bytes)
}

/// Content-addressed storage shared by originals, photos and undo objects.
pub(crate) fn keep_content(directory: &Path, bytes: &[u8]) -> Result<String> {
    let hash = crate::sha256(bytes);
    write_bytes_once(&directory.join(&hash), bytes)?;
    Ok(hash)
}

/// Items that carry their id.
pub(crate) trait HasId {
    fn id(&self) -> &crate::Id;
}

impl HasId for crate::Part {
    fn id(&self) -> &crate::Id {
        &self.id
    }
}

impl HasId for crate::Recipe {
    fn id(&self) -> &crate::Id {
        &self.id
    }
}

impl HasId for crate::Job {
    fn id(&self) -> &crate::Id {
        &self.id
    }
}
