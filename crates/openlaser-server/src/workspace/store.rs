// SPDX-License-Identifier: GPL-3.0-or-later

//! Coalesced authoring snapshots and one ordered disk writer. Pending values
//! remain readable until their durable replacements exist on disk.

use super::{Stored, encode, path, read};
use crate::draft::AuthoringState;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as ShortMutex, MutexGuard};
use tokio::sync::{Mutex, watch};

#[derive(Clone)]
struct Entry {
    revision: u64,
    draft: Option<Arc<AuthoringState>>,
}

#[derive(Clone, Default)]
struct Pending {
    revision: u64,
    dirty: bool,
    active: Option<String>,
    deleting: Option<String>,
    entries: BTreeMap<String, Entry>,
}

pub(super) enum Retained {
    Busy,
    OnDisk,
    Removed,
    Pending(Arc<AuthoringState>),
}

#[derive(Clone)]
pub(crate) struct Store {
    pending: Arc<ShortMutex<Pending>>,
    requests: watch::Sender<u64>,
    status: watch::Sender<Option<String>>,
    pub(super) writer: Arc<Mutex<Writer>>,
}

impl Store {
    pub(crate) fn new(root: &Path) -> Self {
        // Startup's restore_active reports a malformed active reference.
        let reference = read::<Option<String>>(&root.join("active.json"));
        let reference_known = reference.is_ok();
        let active = reference.ok().flatten().flatten();
        Self {
            pending: Arc::new(ShortMutex::new(Pending {
                active: active.clone(),
                ..Pending::default()
            })),
            requests: watch::channel(0).0,
            status: watch::channel(None).0,
            writer: Arc::new(Mutex::new(Writer { root: root.to_owned(), active, reference_known })),
        }
    }

    fn state(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn changed(&self, pending: &mut Pending) {
        pending.revision += 1;
        pending.dirty = true;
        self.requests.send_replace(pending.revision);
    }

    pub(crate) fn save(&self, key: &str, draft: Arc<AuthoringState>) {
        let mut pending = self.state();
        self.changed(&mut pending);
        let revision = pending.revision;
        pending.entries.insert(key.to_owned(), Entry { revision, draft: Some(draft) });
        pending.active = Some(key.to_owned());
    }

    pub(crate) fn remove(&self, key: &str) {
        let mut pending = self.state();
        self.changed(&mut pending);
        let revision = pending.revision;
        pending.entries.insert(key.to_owned(), Entry { revision, draft: None });
    }

    pub(crate) fn clear_active(&self) {
        let mut pending = self.state();
        pending.active = None;
        self.changed(&mut pending);
    }

    pub(super) fn deleted(&self, key: &str) {
        let mut pending = self.state();
        pending.entries.remove(key);
        if pending.active.as_deref() == Some(key) {
            pending.active = None;
        }
        self.changed(&mut pending);
    }

    pub(super) fn pending(&self, key: &str) -> Retained {
        let pending = self.state();
        if pending.deleting.as_deref() == Some(key) {
            return Retained::Busy;
        }
        match pending.entries.get(key) {
            None => Retained::OnDisk,
            Some(Entry { draft: None, .. }) => Retained::Removed,
            Some(Entry { draft: Some(draft), .. }) => Retained::Pending(draft.clone()),
        }
    }

    pub(super) fn block_reads(&self, key: &str) -> ReadBlock<'_> {
        self.state().deleting = Some(key.to_owned());
        ReadBlock(self)
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        self.state().entries.keys().cloned().collect()
    }

    pub(super) fn revision(&self) -> u64 {
        self.state().revision
    }

    pub(crate) fn requests(&self) -> watch::Receiver<u64> {
        self.requests.subscribe()
    }

    pub(crate) fn watch(&self) -> watch::Receiver<Option<String>> {
        self.status.subscribe()
    }

    pub(crate) async fn flush(&self) -> Result<()> {
        let store = self.clone();
        tokio::spawn(async move { store.flush_owned().await })
            .await
            .map_err(|e| Error::Refused(format!("draft flush: {e}")))?
    }

    async fn flush_owned(&self) -> Result<()> {
        let mut writer = self.writer.lock().await;
        let batch = self.state().clone();
        if !batch.dirty {
            return Ok(());
        }
        let root = writer.root.clone();
        let previous = writer.active.clone();
        let reference_known = writer.reference_known;
        let writing = batch.clone();
        let result = tokio::task::spawn_blocking(move || {
            write(&root, &writing, previous.as_deref(), reference_known)
        })
        .await
        .map_err(|e| Error::Refused(format!("draft writer: {e}")))
        .and_then(std::convert::identity);
        // A failure can happen after active.json was replaced. Invalidate the
        // cache so the next batch cannot mistake an old key for the disk value.
        writer.reference_known = result.is_ok();
        if result.is_ok() {
            writer.active.clone_from(&batch.active);
            let mut pending = self.state();
            pending.entries.retain(|_, entry| entry.revision > batch.revision);
            pending.dirty = pending.revision != batch.revision;
        }
        // A failed batch stays queued. An edit or explicit flush retries it.
        self.status.send_replace(result.as_ref().err().map(ToString::to_string));
        result
    }
}

fn write(
    root: &Path,
    batch: &Pending,
    previous: Option<&str>,
    reference_known: bool,
) -> Result<()> {
    std::fs::create_dir_all(root.join("drafts")).map_err(|e| Error::Refused(e.to_string()))?;
    for (key, entry) in &batch.entries {
        if let Some(draft) = &entry.draft {
            openlaser_library::atomic_write(
                &path(root, key)?,
                &encode(&Stored { version: super::version(draft.parts()), draft })?,
            )?;
        }
    }
    // A reference only points to an already-written draft. Routine edits to
    // the same document need no second atomic write and fsync.
    if !reference_known || batch.active.as_deref() != previous {
        openlaser_library::atomic_write(&root.join("active.json"), &encode(&batch.active)?)?;
    }
    for (key, entry) in &batch.entries {
        if entry.draft.is_none() {
            remove(&path(root, key)?)?;
        }
    }
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Refused(error.to_string())),
    }
}

// The writer lock guarantees one discard at a time. Retained reads must not
// mistake its temporarily removed file for a new, empty working copy.
pub(super) struct ReadBlock<'a>(&'a Store);

impl Drop for ReadBlock<'_> {
    fn drop(&mut self) {
        self.0.state().deleting = None;
    }
}

pub(super) struct Writer {
    root: PathBuf,
    active: Option<String>,
    reference_known: bool,
}

pub(super) struct Deleted {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    active: Option<String>,
}

impl Writer {
    pub(super) async fn delete(&mut self, key: &str) -> Result<Deleted> {
        let path = path(&self.root, key)?;
        let active_path = self.root.join("active.json");
        let key = key.to_owned();
        self.reference_known = false;
        let deleted = tokio::task::spawn_blocking(move || {
            let active = read::<Option<String>>(&active_path)?.flatten();
            let clear = active.as_deref() == Some(key.as_str());
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(Error::Refused(error.to_string())),
            };
            if clear {
                openlaser_library::atomic_write(&active_path, b"null")?;
            }
            if let Err(error) = remove(&path) {
                if clear {
                    openlaser_library::atomic_write(&active_path, &encode(&active)?)?;
                }
                return Err(error);
            }
            Ok(Deleted { path, bytes, active })
        })
        .await
        .map_err(|e| Error::Refused(format!("discard writer: {e}")))??;
        self.active = deleted.active.clone().filter(|active| {
            deleted.path.file_stem().and_then(|s| s.to_str()) != Some(active.as_str())
        });
        self.reference_known = true;
        Ok(deleted)
    }

    pub(super) async fn restore(&mut self, deleted: Deleted) -> Result<()> {
        let root = self.root.clone();
        let active = deleted.active.clone();
        self.reference_known = false;
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(bytes) = deleted.bytes {
                openlaser_library::atomic_write(&deleted.path, &bytes)?;
            }
            openlaser_library::atomic_write(&root.join("active.json"), &encode(&deleted.active)?)?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Refused(format!("discard recovery: {e}")))??;
        self.active = active;
        self.reference_known = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_cleanup_cannot_leave_a_stale_active_reference_cached() {
        let root = std::env::temp_dir().join(format!(
            "openlaser-draft-writer-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let store = Store::new(&root);
        let empty = || openlaser_core::geometry::Drawing { contours: Vec::new() };
        let first = Arc::new(crate::draft::Draft::of(empty()).authoring_state());
        let second = Arc::new(crate::draft::Draft::of(empty()).authoring_state());
        store.save("part-a", first.clone());
        store.flush().await.unwrap();
        let blocked = path(&root, "part-a").unwrap();
        std::fs::remove_file(&blocked).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        store.save("part-b", second);
        store.remove("part-a");
        assert!(store.flush().await.is_err());
        assert_eq!(
            read::<Option<String>>(&root.join("active.json")).unwrap().flatten().as_deref(),
            Some("part-b")
        );
        std::fs::remove_dir(&blocked).unwrap();
        store.save("part-a", first);
        store.flush().await.unwrap();
        assert_eq!(
            read::<Option<String>>(&root.join("active.json")).unwrap().flatten().as_deref(),
            Some("part-a")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A retained draft written before the draft grouped its undoable
    /// fields into one snapshot: two sheets, one with a sheet offset, and
    /// an undo and a redo step. It must still load, and writing it back
    /// must give the same bytes.
    #[test]
    fn a_retained_draft_from_an_earlier_build_loads_and_writes_back_unchanged() {
        let old = include_str!("fixtures/retained_draft_v1.json");
        let stored: Stored = serde_json::from_str(old).unwrap();
        assert_eq!(stored.version, 1);
        let draft = stored.draft;
        let view = draft.view();
        assert_eq!((view.past, view.future), (1, 1));
        assert_eq!(view.sheet_offset, Some([12.5, 7.25]));
        assert!(draft.current.sheets.is_some());
        let state = draft.authoring_state();
        let written =
            encode(&Stored { version: super::super::version(state.parts()), draft: state })
                .unwrap();
        assert_eq!(String::from_utf8(written).unwrap(), old.trim_end());
    }
}
