// SPDX-License-Identifier: GPL-3.0-or-later

//! Retained authoring drafts. No compiled program, reference or controller
//! permission is written here. Saved jobs remain library items.

mod store;
pub(crate) use store::Store;

use crate::coordinator::{Coordinator, Shared};
use crate::document::JobView;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_library::{Id, Job, Library};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A retained draft on disk. Version 2 holds several parts; a draft of one
/// part stays version 1, which earlier builds read.
#[derive(Serialize, Deserialize)]
struct Stored<T = Draft> {
    version: u32,
    draft: T,
}

/// The newest retained draft version this build reads.
const NEWEST: u32 = 2;

/// The oldest retained draft version that holds a draft of `parts` parts.
pub(crate) const fn version(parts: usize) -> u32 {
    if parts > 1 { 2 } else { 1 }
}

/// One retained draft in the pending-edits review.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct PendingDraft {
    /// Retained draft identity, independent of the active page.
    pub key: String,
    /// Name of the saved job or of its parts.
    pub name: String,
    /// The parts it cuts.
    pub parts: Vec<Id>,
    /// Saved job, if there is one.
    pub job: Option<Id>,
    /// Whether it has a material and can be saved as a job.
    pub can_save: bool,
    /// Missing or invalid source that prevents opening this retained draft.
    pub problem: Option<String>,
}

/// One value changed independently in both the draft and the saved job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct EditConflict {
    /// JSON pointer identifying the conflicting value.
    pub path: String,
    /// Value before either edit.
    pub base: String,
    /// Draft value for review.
    pub draft: String,
    /// Saved value for review.
    pub saved: String,
}

/// A merge review bound to both the current working copy and saved job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct MergeReview {
    /// Changes if either input changes.
    pub token: String,
    /// Name after merging the nonconflicting values.
    pub name: String,
    /// Only values requiring an operator choice.
    pub conflicts: Vec<EditConflict>,
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| Error::Refused(e.to_string()))
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::Refused(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Refused(format!("{}: {e}", path.display()))),
    }
}

/// Stable filename for one authoring document.
#[must_use]
pub fn key(draft: &Draft) -> String {
    draft.job.as_ref().map_or_else(
        || {
            draft.workspace_id.as_ref().map_or_else(
                || format!("part-{}", draft.parts.first().map_or("", Id::as_str)),
                |id| format!("draft-{id}"),
            )
        },
        |id| format!("job-{id}"),
    )
}

fn path(root: &Path, key: &str) -> Result<PathBuf> {
    let valid = key
        .strip_prefix("part-")
        .or_else(|| key.strip_prefix("job-"))
        .or_else(|| key.strip_prefix("draft-"))
        .is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        });
    if !valid {
        return Err(Error::Request("invalid draft identity".into()));
    }
    Ok(root.join("drafts").join(format!("{key}.json")))
}

impl Coordinator {
    /// Saves the draft as a job, or updates the job it was opened from.
    pub fn save_job(&mut self, name: &str) -> Result<JobView> {
        if self.draft.as_ref().is_some_and(|d| d.sheets.is_some()) {
            return self.save_sheets(name);
        }
        let draft =
            self.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let old_key = crate::workspace::key(draft);
        let saved = save_library_job(&mut self.library, draft, name)?;
        let prepared_changed = preparation_changed(draft, &saved);
        if let Some(draft) = &mut self.draft {
            draft.adopt(&saved);
        }
        self.attach_draft()?;
        if prepared_changed {
            self.reprepare();
        } else {
            self.draft_changed();
        }
        if old_key != format!("job-{}", saved.id) {
            self.draft_store.remove(&old_key);
        }
        self.library_changed();
        Ok(JobView::new(&saved, &self.library.job_drawing(&saved.parts)?))
    }

    /// Opens a retained working copy, including one whose saved job was deleted.
    pub fn open_retained(&mut self, key: &str) -> Result<()> {
        self.queue_draft();
        let draft =
            self.retained(key)?.ok_or_else(|| Error::Missing("no retained working copy".into()))?;
        if self.draft.as_ref().is_some_and(|current| self::key(current) != key) {
            self.leave_draft();
        }
        self.draft_generation += 1;
        self.draft = Some(draft);
        self.reprepare();
        Ok(())
    }

    /// Keeps edited setups separate from their source part. Merely opening
    /// a part must not accumulate empty working copies in the draft store.
    pub(crate) fn leave_draft(&mut self) {
        self.queue_draft();
        if let Some(draft) = &self.draft
            && draft.job.is_none()
            && !draft.dirty()
        {
            self.draft_store.remove(&key(draft));
        }
    }

    /// Queues only changed authoring state. Encoding and durable writes happen
    /// on the ordered writer, after the coordinator transaction has ended.
    pub(crate) fn queue_draft(&mut self) {
        if let Some(draft) = &self.draft {
            if self.queued_draft.as_ref().is_some_and(|saved| saved.matches(draft)) {
                return;
            }
            let snapshot = std::sync::Arc::new(draft.authoring_state());
            self.draft_store.save(&key(draft), snapshot.clone());
            self.queued_draft = Some(snapshot);
        } else if self.queued_draft.take().is_some() {
            self.draft_store.clear_active();
        }
    }

    /// Loads a retained authoring draft without its old runtime state.
    pub(crate) fn retained(&self, key: &str) -> Result<Option<Draft>> {
        let mut draft = match self.draft_store.pending(key) {
            store::Retained::Busy => {
                return Err(Error::Refused(
                    "the working copy is being discarded; try again".into(),
                ));
            }
            store::Retained::Pending(snapshot) => snapshot.draft(),
            store::Retained::Removed => return Ok(None),
            store::Retained::OnDisk => {
                let Some(stored) = read::<Stored>(&path(&self.config.data_dir, key)?)? else {
                    return Ok(None);
                };
                if !(version(stored.draft.parts.len())..=NEWEST).contains(&stored.version)
                    || key != self::key(&stored.draft)
                {
                    return Err(Error::Refused(
                        "invalid retained draft version or identity".into(),
                    ));
                }
                stored.draft
            }
        };
        draft.complete_history();
        draft.validate_saved(|parts| self.library.job_drawing(parts).ok().map(|d| d.contours()))?;
        self.attach(&mut draft)?;
        crate::placement::fresh(&mut draft);
        Ok(Some(draft))
    }

    /// Reopens the last authoring document; it still needs fresh preparation.
    pub(crate) fn restore_active(&mut self) -> Result<()> {
        if let Some(Some(key)) = read::<Option<String>>(&self.config.data_dir.join("active.json"))?
            && let Some(draft) = self.retained(&key)?
        {
            self.draft = Some(draft);
            self.reprepare();
        }
        Ok(())
    }

    /// Lists retained unsaved edits across jobs and parts.
    pub fn pending_drafts(&self) -> Result<Vec<PendingDraft>> {
        let dir = self.config.data_dir.join("drafts");
        let mut keys: BTreeSet<String> = self.draft_store.keys().into_iter().collect();
        if dir.exists() {
            for entry in std::fs::read_dir(dir).map_err(|e| Error::Refused(e.to_string()))? {
                let p = entry.map_err(|e| Error::Refused(e.to_string()))?.path();
                if p.extension().is_some_and(|x| x == "json") {
                    keys.insert(
                        p.file_stem()
                            .and_then(|s| s.to_str())
                            .ok_or_else(|| Error::Refused("invalid draft file name".into()))?
                            .to_owned(),
                    );
                }
            }
        }
        let mut pending = Vec::new();
        for key in &keys {
            let draft = if self.draft.as_ref().is_some_and(|d| self::key(d) == *key) {
                self.draft.clone()
            } else {
                match self.retained(key) {
                    Ok(draft) => draft,
                    Err(error) => {
                        pending.push(PendingDraft {
                            key: key.to_owned(),
                            name: key.to_owned(),
                            parts: Vec::new(),
                            job: None,
                            can_save: false,
                            problem: Some(error.to_string()),
                        });
                        continue;
                    }
                }
            };
            if let Some(draft) = draft.filter(Draft::dirty) {
                pending.push(PendingDraft {
                    key: key.to_owned(),
                    name: self.draft_name(&draft),
                    parts: draft.parts,
                    job: draft.job,
                    can_save: draft.recipe.is_some(),
                    problem: None,
                });
            }
        }
        pending.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(pending)
    }

    /// Reviews the three-way merge without saving anything.
    pub fn merge_review(&self, name: Option<&str>) -> Result<MergeReview> {
        let draft = self.draft.as_ref().ok_or_else(|| Error::Refused("open a job first".into()))?;
        let Some(base) = &draft.saved_base else {
            return Ok(MergeReview {
                token: String::new(),
                name: name.unwrap_or("job").to_owned(),
                conflicts: Vec::new(),
            });
        };
        let Ok(saved) = self.library.job(&base.id) else {
            return Ok(MergeReview {
                token: String::new(),
                name: name.unwrap_or(&base.name).to_owned(),
                conflicts: Vec::new(),
            });
        };
        let local = draft.job_value(name.unwrap_or(&base.name))?;
        let (job, mut conflicts) = merge_jobs(base, &local, saved, &BTreeMap::new())?;
        for conflict in conflicts.iter_mut().filter(|c| c.path == "/part") {
            for text in [&mut conflict.base, &mut conflict.draft, &mut conflict.saved] {
                *text = self.part_names(text);
            }
        }
        Ok(MergeReview {
            token: openlaser_library::sha256(&encode(&(base, local, saved))?),
            name: job.name,
            conflicts,
        })
    }

    /// Applies explicit conflict choices to the draft; saving remains explicit.
    pub fn resolve_merge(
        &mut self,
        token: &str,
        name: Option<&str>,
        choices: &BTreeMap<String, bool>,
    ) -> Result<String> {
        if self.merge_review(name)?.token != token {
            return Err(Error::Refused("saved values changed; review the conflicts again".into()));
        }
        let draft = self.draft.as_ref().ok_or_else(|| Error::Refused("open a job first".into()))?;
        let base = draft
            .saved_base
            .as_ref()
            .ok_or_else(|| Error::Refused("this draft has no saved base".into()))?;
        let saved = self.library.job(&base.id)?.clone();
        let (job, conflicts) =
            merge_jobs(base, &draft.job_value(name.unwrap_or(&base.name))?, &saved, choices)?;
        if !conflicts.is_empty() {
            return Err(Error::Refused("choose a value for every conflict".into()));
        }
        let draft = self.draft.as_mut().ok_or_else(|| Error::Refused("open a job first".into()))?;
        let before = draft.clone();
        draft.remember();
        draft.adopt(&job);
        draft.saved_base = Some(saved);
        if let Err(error) = self.attach_draft() {
            self.draft = Some(before);
            return Err(error);
        }
        self.reprepare();
        Ok(job.name)
    }
}

impl Coordinator {
    /// The names of the parts in a stored parts value, for review.
    fn part_names(&self, stored: &str) -> String {
        #[derive(Deserialize)]
        struct Parts(#[serde(with = "openlaser_library::stored_parts")] Vec<Id>);
        serde_json::from_str::<Parts>(stored).map_or_else(
            |_| stored.to_owned(),
            |Parts(ids)| {
                ids.iter()
                    .map(|id| {
                        self.library.part(id).map_or_else(|_| id.to_string(), |p| p.name.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(" + ")
            },
        )
    }
}

/// Discards a working copy durably before replacing the active page. The
/// transaction finishes even when its requesting page goes away.
pub async fn discard(shared: &Shared, key: &str) -> Result<()> {
    let shared = shared.clone();
    let key = key.to_owned();
    tokio::spawn(async move { discard_owned(&shared, &key).await })
        .await
        .map_err(|e| Error::Refused(format!("discard task: {e}")))?
}

async fn discard_owned(shared: &Shared, key: &str) -> Result<()> {
    let store = shared.lock().await.draft_store.clone();
    // Wait for the disk writer without holding the controller document lock.
    let mut writer = store.writer.lock().await;
    let (revision, generation, active, read_block) = {
        let c = shared.lock().await;
        path(&c.config.data_dir, key)?;
        (
            store.revision(),
            c.draft_generation,
            c.draft.as_ref().is_some_and(|d| self::key(d) == key),
            store.block_reads(key),
        )
    };
    let deleted = writer.delete(key).await?;
    let mut c = shared.lock().await;
    if store.revision() != revision || c.draft_generation != generation {
        // A concurrent edit wins. Restore the old file before allowing newer
        // queued snapshots to replace it; never erase an edit made mid-discard.
        drop(c);
        writer.restore(deleted).await?;
        return Err(Error::Refused(
            "the working copy changed while discarding; review it again".into(),
        ));
    }
    store.deleted(key);
    drop(read_block);
    if active {
        let draft = c.draft.take().ok_or_else(|| Error::Refused("no open draft".into()))?;
        c.queued_draft = None;
        c.draft_changed();
        if let Some(id) = draft.job.filter(|id| c.library.job(id).is_ok()) {
            c.open_job(&id)?;
        } else if draft.parts.iter().all(|id| c.library.part(id).is_ok()) {
            c.open_parts(&draft.parts)?;
        }
    }
    drop(c);
    drop(writer);
    store.flush().await
}

/// Waits for queued authoring changes without holding the coordinator lock.
pub async fn flush(shared: &Shared) -> Result<()> {
    let store = shared.lock().await.draft_store.clone();
    store.flush().await
}

impl Draft {
    /// Current authoring values as a saved-job candidate.
    pub(crate) fn job_value(&self, name: &str) -> Result<Job> {
        let recipe =
            self.recipe.clone().ok_or_else(|| Error::Refused("choose a material first".into()))?;
        let mut job = self.saved_base.clone().unwrap_or_else(|| Job {
            placement: None,
            sheet: None,
            correction: None,
            calibration: false,
            grouping: self.grouping.clone(),
            nesting: None,
            tags: Vec::new(),
            notes: String::new(),
            quantity: 1,
            preflight: self.preflight.clone(),
            id: Id::from(""),
            name: name.to_owned(),
            folder: None,
            parts: self.parts.clone(),
            recipe: recipe.clone(),
            film: None,
            features: self.features.clone(),
            placed: Vec::new(),
            zero: None,
            anchor: self.anchor,
            favourite: false,
            created: 0,
            updated: 0,
        });
        name.clone_into(&mut job.name);
        job.parts.clone_from(&self.parts);
        job.recipe = recipe;
        job.film.clone_from(&self.film);
        job.features.clone_from(&self.features);
        job.placed.clone_from(&self.placed);
        job.grouping.clone_from(&self.grouping);
        job.placement.clone_from(&self.placement);
        job.zero = if crate::placement::is_head(self) { None } else { self.zero };
        job.anchor = self.anchor;
        job.preflight.clone_from(&self.preflight);
        job.nesting.clone_from(&self.nesting);
        job.correction.clone_from(&self.correction);
        job.calibration = self.calibration;
        Ok(job)
    }

    /// Adopts saved authoring values while retaining local undo history.
    /// When the parts change, the drawing must be attached again.
    pub(crate) fn adopt(&mut self, job: &Job) {
        let retained_capture = crate::placement::is_head(self)
            && matches!(job.placement, Some(openlaser_library::placement::Placement::Head {}));
        self.parts.clone_from(&job.parts);
        self.job = Some(job.id.clone());
        self.saved_base = Some(job.clone());
        self.recipe = Some(job.recipe.clone());
        self.film.clone_from(&job.film);
        self.features.clone_from(&job.features);
        self.placed.clone_from(&job.placed);
        self.grouping.clone_from(&job.grouping);
        self.placement.clone_from(&job.placement);
        if !retained_capture {
            self.zero = job.zero;
            self.capture_epoch = None;
        }
        self.anchor = job.anchor;
        self.preflight.clone_from(&job.preflight);
        self.nesting.clone_from(&job.nesting);
        self.correction.clone_from(&job.correction);
        self.calibration = job.calibration;
    }
}

/// The values that name contours of the job's drawing by their index.
/// Jobs cutting different parts number those contours differently, so the
/// values only merge one by one between jobs cutting the same parts.
const LAYOUT: [&str; 3] = ["part", "placed", "nesting"];

/// Merges independent saved and local edits; arrays are one coherent value.
/// When the two sides cut different parts, their parts and layout come
/// together from the side that changed them, or from one choice at `/part`.
pub(crate) fn merge_jobs(
    base: &Job,
    local: &Job,
    saved: &Job,
    choices: &BTreeMap<String, bool>,
) -> Result<(Job, Vec<EditConflict>)> {
    let value = |j: &Job| serde_json::to_value(j).map_err(|e| Error::Refused(e.to_string()));
    let (mut base, mut local, mut saved) = (value(base)?, value(local)?, value(saved)?);
    let mut conflicts = Vec::new();
    let layout = (local.get("part") != saved.get("part")).then(|| {
        let take = |job: &mut Value| -> serde_json::Map<String, Value> {
            let fields = job.as_object_mut();
            fields.map_or_else(serde_json::Map::new, |fields| {
                LAYOUT
                    .iter()
                    .filter_map(|&key| Some((key.to_owned(), fields.remove(key)?)))
                    .collect()
            })
        };
        let (base, local, saved) = (take(&mut base), take(&mut local), take(&mut saved));
        if saved == base {
            local
        } else if local == base {
            saved
        } else if let Some(&keep_local) = choices.get("/part") {
            if keep_local { local } else { saved }
        } else {
            let display = |layout: &serde_json::Map<String, Value>| {
                layout.get("part").map_or_else(|| "removed".into(), Value::to_string)
            };
            conflicts.push(EditConflict {
                path: "/part".into(),
                base: display(&base),
                draft: display(&local),
                saved: display(&saved),
            });
            local
        }
    });
    let mut merged = merge(Some(&base), Some(&local), Some(&saved), "", choices, &mut conflicts)
        .unwrap_or(Value::Null);
    if let (Some(layout), Value::Object(fields)) = (layout, &mut merged) {
        fields.extend(layout);
    }
    Ok((serde_json::from_value(merged).map_err(|e| Error::Refused(e.to_string()))?, conflicts))
}

fn merge(
    base: Option<&Value>,
    local: Option<&Value>,
    saved: Option<&Value>,
    path: &str,
    choices: &BTreeMap<String, bool>,
    conflicts: &mut Vec<EditConflict>,
) -> Option<Value> {
    if local == base {
        return saved.cloned();
    }
    if saved == base || local == saved {
        return local.cloned();
    }
    if let (Some(Value::Object(b)), Some(Value::Object(l)), Some(Value::Object(s))) =
        (base, local, saved)
    {
        let keys: BTreeSet<_> = b.keys().chain(l.keys()).chain(s.keys()).collect();
        return Some(Value::Object(
            keys.into_iter()
                .filter_map(|key| {
                    let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    merge(b.get(key), l.get(key), s.get(key), &path, choices, conflicts)
                        .map(|v| (key.clone(), v))
                })
                .collect(),
        ));
    }
    if let Some(local_choice) = choices.get(path) {
        return if *local_choice { local.cloned() } else { saved.cloned() };
    }
    let display = |value: Option<&Value>| value.map_or_else(|| "removed".into(), Value::to_string);
    conflicts.push(EditConflict {
        path: path.to_owned(),
        base: display(base),
        draft: display(local),
        saved: display(saved),
    });
    local.cloned()
}

fn save_library_job(library: &mut Library, draft: &Draft, name: &str) -> Result<Job> {
    let mut job = draft.job_value(name)?;
    if draft.saved_base.is_none() {
        let parts = draft
            .parts
            .iter()
            .map(|id| library.part(id))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        describe(&mut job, &parts);
    }
    Ok(match draft.saved_base.as_ref().filter(|base| library.job(&base.id).is_ok()) {
        Some(base) => {
            let (mut merged, conflicts) =
                merge_jobs(base, &job, library.job(&base.id)?, &BTreeMap::new())?;
            if !conflicts.is_empty() {
                return Err(Error::Refused(format!(
                    "saved job has conflicting edits in {}; review pending changes",
                    conflicts.iter().map(|c| c.path.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
            let id = base.id.clone();
            library.update_job(&id, |existing| {
                merged.created = existing.created;
                *existing = merged;
            })?
        }
        None => library.add_job(job)?,
    })
}

/// A new job's tags, notes and quantity: its part's, or for several parts,
/// every part's tags and each part's notes under its name, for one set.
fn describe(job: &mut Job, parts: &[&openlaser_library::Part]) {
    if let [part] = parts {
        job.tags.clone_from(&part.tags);
        job.notes.clone_from(&part.notes);
        job.quantity = part.quantity;
        return;
    }
    job.tags.clear();
    for tag in parts.iter().flat_map(|part| &part.tags) {
        if job.tags.len() < 40 && !job.tags.contains(tag) {
            job.tags.push(tag.clone());
        }
    }
    job.notes = parts
        .iter()
        .filter(|part| !part.notes.trim().is_empty())
        .map(|part| format!("{}: {}", part.name, part.notes.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    if job.notes.len() > 10_000 {
        let end = (0..=10_000).rev().find(|&i| job.notes.is_char_boundary(i)).unwrap_or(0);
        job.notes.truncate(end);
    }
    job.quantity = 1;
}

fn preparation_changed(draft: &Draft, saved: &Job) -> bool {
    draft.parts != saved.parts
        || draft.correction != saved.correction
        || draft.calibration != saved.calibration
        || draft.nesting != saved.nesting
        || draft.recipe.as_ref() != Some(&saved.recipe)
        || draft.film != saved.film
        || draft.features != saved.features
        || draft.placed != saved.placed
        || !crate::placement::matches_saved(draft, saved)
        || draft.anchor != saved.anchor
}
