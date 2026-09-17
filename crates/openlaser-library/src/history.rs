// SPDX-License-Identifier: GPL-3.0-or-later

//! Durable undo for the existing item files. An intent is written before an
//! item changes; interrupted edits roll back, committed edits roll forward.
//! Geometry and originals are not copied into a second library format.

use crate::{Error, Id, Library, Result, store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One saved library edit.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edit {
    /// Stable edit identity.
    pub id: Id,
    /// What changed.
    pub label: String,
    /// Time in seconds since the epoch.
    pub at: u64,
}

/// Saved changes that can be undone or redone, oldest first.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EditHistory {
    /// Changes on every edit, undo and redo.
    pub revision: u64,
    /// Applied edits, latest last.
    pub past: Vec<Edit>,
    /// Undone edits, next redo last.
    pub future: Vec<Edit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Change {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related: Vec<Change>,
    path: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Intent {
    change: Change,
    before: EditHistory,
    after: EditHistory,
    committed: bool,
}

fn directory(root: &Path) -> PathBuf {
    root.join("edit-history")
}

fn object(root: &Path, hash: &str) -> Result<PathBuf> {
    Ok(directory(root).join("objects").join(crate::hash_name(hash)?))
}

fn read_current(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io { path: path.display().to_string(), reason: e.to_string() }),
    }
}

fn keep(root: &Path, bytes: Option<&[u8]>) -> Result<Option<String>> {
    bytes.map(|bytes| store::keep_content(&directory(root).join("objects"), bytes)).transpose()
}

fn target(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative != "folders.json" {
        let Some((kind, file)) = relative.split_once('/') else {
            return Err(Error::Invalid("invalid history item path".into()));
        };
        let Some(id) = file.strip_suffix(".json") else {
            return Err(Error::Invalid("invalid history item path".into()));
        };
        if !matches!(kind, "parts" | "recipes" | "jobs") {
            return Err(Error::Invalid("invalid history item kind".into()));
        }
        crate::validation::id(&Id::from(id))?;
    }
    Ok(root.join(relative))
}

fn apply(root: &Path, change: &Change, forward: bool) -> Result<()> {
    apply_one(root, change, forward)?;
    for related in &change.related {
        apply(root, related, forward)?;
    }
    Ok(())
}

fn apply_one(root: &Path, change: &Change, forward: bool) -> Result<()> {
    let path = target(root, &change.path)?;
    let hash = if forward { &change.after } else { &change.before };
    match hash {
        Some(hash) => {
            let bytes = store::read_content(&object(root, hash)?, hash)?;
            store::write_bytes(&path, &bytes)
        }
        None if path.exists() => store::remove(&path),
        None => Ok(()),
    }
}

/// Completes or rolls back an interrupted item transaction before loading files.
pub(crate) fn recover(root: &Path) -> Result<()> {
    let path = directory(root).join("pending.json");
    if let Some(intent) = store::read_optional::<Intent>(&path)? {
        apply(root, &intent.change, intent.committed)?;
        store::write(
            &directory(root).join("index.json"),
            if intent.committed { &intent.after } else { &intent.before },
        )?;
        store::remove(&path)?;
    }
    Ok(())
}

fn index(root: &Path) -> Result<EditHistory> {
    let index: EditHistory =
        store::read_optional(&directory(root).join("index.json"))?.unwrap_or_default();
    if index.past.len() + index.future.len() > 100 {
        return Err(Error::Invalid("saved history exceeds its limit".into()));
    }
    for entry in index.past.iter().chain(&index.future) {
        crate::validation::id(&entry.id)?;
    }
    Ok(index)
}

fn transact(root: &Path, mut intent: Intent) -> Result<()> {
    let path = directory(root).join("pending.json");
    store::write(&path, &intent)?;
    let result = (|| {
        apply(root, &intent.change, true)?;
        store::write(&directory(root).join("index.json"), &intent.after)?;
        intent.committed = true;
        store::write(&path, &intent)
    })();
    if let Err(error) = result {
        recover(root)?;
        return Err(error);
    }
    // A committed intent is safe to replay if its cleanup is interrupted.
    let _ = store::remove(&path);
    if !path.exists() {
        let _ = prune(root, &intent.before, &intent.after);
    }
    Ok(())
}

/// Only expired entries from this transaction are removed. Read every retained
/// reference first, so a failed read cannot discard an object still in use.
fn prune(root: &Path, before: &EditHistory, after: &EditHistory) -> Result<()> {
    let retained: std::collections::BTreeSet<_> =
        after.past.iter().chain(&after.future).map(|e| &e.id).collect();
    let expired: Vec<_> =
        before.past.iter().chain(&before.future).filter(|e| !retained.contains(&e.id)).collect();
    if expired.is_empty() {
        return Ok(());
    }
    let mut hashes = std::collections::BTreeSet::new();
    for id in retained {
        let change: Change = store::read(&directory(root).join(format!("{id}.json")))?;
        hashes.extend(change.hashes());
    }
    for entry in expired {
        let path = directory(root).join(format!("{}.json", entry.id));
        let change: Change = store::read(&path)?;
        store::remove(&path)?;
        for hash in change.hashes().into_iter().filter(|h| !hashes.contains(h)) {
            let path = object(root, &hash)?;
            if path.exists() {
                store::remove(&path)?;
            }
        }
    }
    Ok(())
}

fn library_root(path: &Path) -> Result<&Path> {
    let parent = path.parent().ok_or_else(|| Error::Invalid("missing item directory".into()))?;
    if path.file_name().is_some_and(|n| n == "folders.json") {
        Ok(parent)
    } else {
        parent.parent().ok_or_else(|| Error::Invalid("missing library directory".into()))
    }
}

impl Change {
    fn hashes(&self) -> Vec<String> {
        self.before
            .iter()
            .chain(self.after.iter())
            .cloned()
            .chain(self.related.iter().flat_map(Self::hashes))
            .collect()
    }

    fn reversed(&mut self) {
        std::mem::swap(&mut self.before, &mut self.after);
        for related in &mut self.related {
            related.reversed();
        }
    }

    fn capture(
        root: &Path,
        path: &Path,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
    ) -> Result<Self> {
        let relative = path
            .strip_prefix(root)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        target(root, &relative)?;
        Ok(Self {
            related: Vec::new(),
            path: relative,
            before: keep(root, before)?,
            after: keep(root, after)?,
        })
    }

    /// Refuse an external edit before staging a historical version.
    fn verify_current(&self, root: &Path) -> Result<()> {
        for related in &self.related {
            related.verify_current(root)?;
        }
        let actual = read_current(&target(root, &self.path)?)?.map(|b| crate::sha256(&b));
        if actual != self.before {
            return Err(Error::Invalid("the saved item changed outside this history".into()));
        }
        Ok(())
    }

    fn read_after<T: serde::de::DeserializeOwned>(&self, root: &Path) -> Result<Option<T>> {
        let Some(hash) = &self.after else { return Ok(None) };
        let path = object(root, hash)?;
        let bytes = store::read_content(&path, hash)?;
        store::decode(&path, &bytes).map(Some)
    }

    fn stage_item<T: store::HasId + serde::de::DeserializeOwned>(
        &self,
        root: &Path,
        id: Id,
        items: &mut std::collections::BTreeMap<Id, T>,
    ) -> Result<()> {
        if let Some(item) = self.read_after::<T>(root)? {
            crate::validation::same_id(&id, item.id())?;
            items.insert(id, item);
        } else {
            items.remove(&id);
        }
        Ok(())
    }

    fn stage(&self, library: &Library) -> Result<Library> {
        let mut candidate = library.clone();
        self.stage_into(&mut candidate)?;
        crate::validation::library(&candidate)?;
        Ok(candidate)
    }

    fn stage_into(&self, candidate: &mut Library) -> Result<()> {
        if self.path == "folders.json" {
            candidate.folders =
                self.read_after::<crate::Folders>(&candidate.root)?.unwrap_or_default().folders;
        } else {
            let (kind, file) = self
                .path
                .split_once('/')
                .ok_or_else(|| Error::Invalid("invalid history item path".into()))?;
            let id = Id::from(file.trim_end_matches(".json"));
            match kind {
                "parts" => self.stage_item(&candidate.root, id, &mut candidate.parts),
                "recipes" => self.stage_item(&candidate.root, id, &mut candidate.recipes),
                "jobs" => self.stage_item(&candidate.root, id, &mut candidate.jobs),
                _ => Err(Error::Invalid("invalid history item kind".into())),
            }?;
        }
        for related in &self.related {
            related.stage_into(candidate)?;
        }
        Ok(())
    }
}

impl Edit {
    fn describe(change: &Change, before: Option<&[u8]>, after: Option<&[u8]>) -> Self {
        let name = after
            .or(before)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
            .and_then(|v| v["name"].as_str().map(str::to_owned));
        let verb = if before.is_none() {
            "Added"
        } else if after.is_none() {
            "Deleted"
        } else {
            "Edited"
        };
        let kind = change
            .path
            .split('/')
            .next()
            .unwrap_or("item")
            .trim_end_matches(".json")
            .trim_end_matches('s');
        Self {
            id: Id::generate(),
            label: format!("{verb} {kind}{}", name.map_or(String::new(), |n| format!(": {n}"))),
            at: crate::now(),
        }
    }
}

impl EditHistory {
    fn appended(&self, edit: Edit) -> Self {
        let mut next = self.clone();
        next.past.push(edit);
        if next.past.len() > 100 {
            next.past.remove(0);
        }
        next.future.clear();
        next.revision += 1;
        next
    }

    fn transferred(&self, back: bool, revision: u64) -> Result<(Self, Id)> {
        if self.revision != revision {
            return Err(Error::Invalid("library history changed; review it again".into()));
        }
        let mut next = self.clone();
        let (from, to) = if back {
            (&mut next.past, &mut next.future)
        } else {
            (&mut next.future, &mut next.past)
        };
        let edit = from.pop().ok_or_else(|| {
            Error::Invalid(if back { "nothing to undo" } else { "nothing to redo" }.into())
        })?;
        let id = edit.id.clone();
        to.push(edit);
        next.revision += 1;
        Ok((next, id))
    }
}

fn record(path: &Path, after: Option<&[u8]>) -> Result<()> {
    let root = library_root(path)?;
    recover(root)?;
    let before = read_current(path)?;
    if before.as_deref() == after {
        return Ok(());
    }
    store::create_dir(&directory(root).join("objects"))?;
    let change = Change::capture(root, path, before.as_deref(), after)?;
    let edit = Edit::describe(&change, before.as_deref(), after);
    let current = index(root)?;
    store::write(&directory(root).join(format!("{}.json", edit.id)), &change)?;
    let next = current.appended(edit);
    transact(root, Intent { change, before: current, after: next, committed: false })
}

pub(crate) fn write<T: Serialize>(path: &Path, item: &T) -> Result<()> {
    record(path, Some(&store::encode(path, item)?))
}

/// Commit a folder and its numbered sheets as one recoverable history entry.
pub(crate) fn write_batch(
    root: &Path,
    label: String,
    files: Vec<(PathBuf, Vec<u8>)>,
) -> Result<()> {
    recover(root)?;
    if files.is_empty() || files.len() > 501 {
        return Err(Error::Invalid("invalid sheet set size".into()));
    }
    store::create_dir(&directory(root).join("objects"))?;
    let mut changes = files
        .into_iter()
        .map(|(path, after)| {
            let before = read_current(&path)?;
            Change::capture(root, &path, before.as_deref(), Some(&after))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut change = changes.remove(0);
    change.related = changes;
    let edit = Edit { id: Id::generate(), label, at: crate::now() };
    let current = index(root)?;
    store::write(&directory(root).join(format!("{}.json", edit.id)), &change)?;
    let next = current.appended(edit);
    transact(root, Intent { change, before: current, after: next, committed: false })
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    record(path, None)
}

impl Library {
    /// The durable saved-data history. Draft undo is independent.
    pub fn edit_history(&self) -> Result<EditHistory> {
        index(&self.root)
    }

    /// Undoes or redoes one saved-data edit, refusing externally changed files.
    pub fn undo_saved(&mut self, back: bool, revision: u64) -> Result<()> {
        let current = index(&self.root)?;
        let (next, id) = current.transferred(back, revision)?;
        let mut change: Change = store::read(&directory(&self.root).join(format!("{id}.json")))?;
        if back {
            change.reversed();
        }
        change.verify_current(&self.root)?;
        let candidate = change.stage(self)?;
        transact(&self.root, Intent { change, before: current, after: next, committed: false })?;
        *self = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_history_objects_are_refused_before_files_or_history_change() {
        let root =
            std::env::temp_dir().join(format!("openlaser-history-invalid-{}", Id::generate()));
        let mut library = Library::open(&root).unwrap();
        let drawing = openlaser_core::geometry::Drawing {
            contours: vec![openlaser_core::geometry::Contour {
                layer: "0".into(),
                curves: vec![openlaser_core::geometry::Curve::circle(
                    openlaser_core::geometry::Point::ORIGIN,
                    10.,
                )],
            }],
        };
        let part = library.add_part("circle.dxf", b"original", drawing).unwrap();
        library.update_part(&part.id, |p| p.name = "Updated".into()).unwrap();
        let current = library.edit_history().unwrap();
        let edit_path = directory(&root).join(format!("{}.json", current.past.last().unwrap().id));
        let mut change: Change = store::read(&edit_path).unwrap();
        let saved_bytes = std::fs::read(library.path("parts", &part.id)).unwrap();
        let mut wrong = part.clone();
        wrong.id = Id::generate();
        let wrong_bytes = store::encode(&edit_path, &wrong).unwrap();
        change.before = keep(&root, Some(&wrong_bytes)).unwrap();
        store::write(&edit_path, &change).unwrap();
        assert!(
            library
                .undo_saved(true, current.revision)
                .unwrap_err()
                .to_string()
                .contains("id cannot change")
        );
        assert_eq!(library.part(&part.id).unwrap().name, "Updated");
        assert_eq!(library.edit_history().unwrap().revision, current.revision);
        assert_eq!(std::fs::read(library.path("parts", &part.id)).unwrap(), saved_bytes);
        assert!(!directory(&root).join("pending.json").exists());

        let hash = change.before.as_ref().unwrap();
        std::fs::write(object(&root, hash).unwrap(), b"corrupted").unwrap();
        assert!(
            library.undo_saved(true, current.revision).unwrap_err().to_string().contains("SHA-256")
        );
        assert_eq!(std::fs::read(library.path("parts", &part.id)).unwrap(), saved_bytes);
        assert_eq!(Library::open(&root).unwrap().part(&part.id).unwrap().name, "Updated");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retention_is_bounded_and_new_edits_discard_abandoned_redo_objects() {
        let root = std::env::temp_dir().join(format!("openlaser-history-limit-{}", Id::generate()));
        let mut library = Library::open(&root).unwrap();
        let folder = library.add_folder("First", None).unwrap();
        for index in 0..103 {
            library.update_folder(&folder.id, |f| f.name = format!("Folder {index}")).unwrap();
        }
        let history = library.edit_history().unwrap();
        assert_eq!(history.past.len(), 100);
        assert_eq!(std::fs::read_dir(directory(&root).join("objects")).unwrap().count(), 101);
        library.undo_saved(true, history.revision).unwrap();
        library.update_folder(&folder.id, |f| f.name = "New branch".into()).unwrap();
        assert!(library.edit_history().unwrap().future.is_empty());
        assert_eq!(std::fs::read_dir(directory(&root).join("objects")).unwrap().count(), 101);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_edits_recover_in_the_same_direction_as_the_commit_marker() {
        let root = std::env::temp_dir().join(format!("openlaser-history-{}", Id::generate()));
        let mut library = Library::open(&root).unwrap();
        let folder = library.add_folder("Before", None).unwrap();
        let before = library.edit_history().unwrap();
        let old_bytes = std::fs::read(root.join("folders.json")).unwrap();
        library.update_folder(&folder.id, |f| f.name = "After".into()).unwrap();
        let after = library.edit_history().unwrap();
        let new_bytes = std::fs::read(root.join("folders.json")).unwrap();
        for committed in [false, true] {
            let change = Change {
                related: Vec::new(),
                path: "folders.json".into(),
                before: keep(&root, Some(&old_bytes)).unwrap(),
                after: keep(&root, Some(&new_bytes)).unwrap(),
            };
            store::write_bytes(
                &root.join("folders.json"),
                if committed { &old_bytes } else { &new_bytes },
            )
            .unwrap();
            store::write(
                &directory(&root).join("pending.json"),
                &Intent { change, before: before.clone(), after: after.clone(), committed },
            )
            .unwrap();
            let reopened = Library::open(&root).unwrap();
            assert_eq!(reopened.folders()[0].name, if committed { "After" } else { "Before" });
            assert_eq!(
                reopened.edit_history().unwrap().revision,
                if committed { after.revision } else { before.revision }
            );
            assert!(!directory(&root).join("pending.json").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partially_written_batches_recover_every_related_item() {
        let root = std::env::temp_dir().join(format!("openlaser-history-batch-{}", Id::generate()));
        let mut library = Library::open(&root).unwrap();
        let before = library.edit_history().unwrap();
        let before_folders = read_current(&root.join("folders.json")).unwrap();
        let folder = library.add_folder("Batch", None).unwrap();
        let part = library
            .add_part(
                "circle.svg",
                b"circle fixture",
                openlaser_core::geometry::Drawing {
                    contours: vec![openlaser_core::geometry::Contour {
                        layer: "0".into(),
                        curves: vec![openlaser_core::geometry::Curve::circle(
                            openlaser_core::geometry::Point::ORIGIN,
                            10.,
                        )],
                    }],
                },
            )
            .unwrap();
        let after = library.edit_history().unwrap();
        let part_path = library.path("parts", &part.id);
        let after_part = std::fs::read(&part_path).unwrap();
        let after_folders = std::fs::read(root.join("folders.json")).unwrap();
        for committed in [false, true] {
            let mut change = Change::capture(
                &root,
                &root.join("folders.json"),
                before_folders.as_deref(),
                Some(&after_folders),
            )
            .unwrap();
            change
                .related
                .push(Change::capture(&root, &part_path, None, Some(&after_part)).unwrap());
            // Emulate loss of power between writing the folder and its item.
            store::write_bytes(&root.join("folders.json"), &after_folders).unwrap();
            if part_path.exists() {
                store::remove(&part_path).unwrap();
            }
            store::write(
                &directory(&root).join("pending.json"),
                &Intent { change, before: before.clone(), after: after.clone(), committed },
            )
            .unwrap();
            let reopened = Library::open(&root).unwrap();
            assert_eq!(reopened.folders().iter().any(|f| f.id == folder.id), committed);
            assert_eq!(reopened.part(&part.id).is_ok(), committed);
            assert_eq!(
                reopened.edit_history().unwrap().revision,
                if committed { after.revision } else { before.revision }
            );
            assert!(!directory(&root).join("pending.json").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
