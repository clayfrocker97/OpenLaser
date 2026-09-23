// SPDX-License-Identifier: GPL-3.0-or-later

//! Parts, jobs and recipes on disk.
//!
//! One JSON file per item under the data directory, each with a schema
//! version. A file that cannot be loaded as it stands, whether newer,
//! damaged or invalid, is never misread or rewritten: it is left where it
//! is, reported, and the rest of the library opens without it and without
//! anything that depends on it. Originals are kept by their hash and never
//! rewritten. A job is a frozen snapshot: it carries its recipe and its
//! features, so editing a library recipe never changes an accepted job.
//! Writes go to a temporary file and are renamed into place, so a crash
//! leaves either the old file or the new one.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "tests use a scratch directory")
)]

mod hash;
pub mod history;
mod job_parts;
pub mod placement;
pub mod preflight;
mod storage;
mod store;
mod validation;

pub use hash::sha256;
pub use job_parts::{JobDrawing, MAX_PARTS, stored_parts};
pub use storage::atomic_write;
pub use store::Schema;

use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_core::geometry::{Bounds, Drawing, Placed};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const fn one() -> u32 {
    1
}

/// Why the library could not do something.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The disk refused.
    #[error("{path}: {reason}")]
    Io {
        /// The file.
        path: String,
        /// The system's reason.
        reason: String,
    },
    /// A file is not the JSON this build understands.
    #[error("{path}: {reason}")]
    Format {
        /// The file.
        path: String,
        /// What is wrong.
        reason: String,
    },
    /// A file was written by a newer build.
    #[error("{path} is schema version {version}; this build reads up to {newest}")]
    Newer {
        /// The file.
        path: String,
        /// Its version.
        version: u32,
        /// The newest version of its kind this build reads.
        newest: u32,
    },
    /// No such item.
    #[error("no {kind} {id}")]
    Missing {
        /// What was asked for.
        kind: &'static str,
        /// Its id.
        id: Id,
    },
    /// The item is referenced by others.
    #[error("{0}")]
    InUse(String),
    /// The item is not acceptable.
    #[error("{0}")]
    Invalid(String),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// A time-ordered item id. A process-local clock keeps IDs distinct when the
/// system clock repeats a tick or moves backwards.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(String);

impl Id {
    /// Generate a time-ordered identity, distinct within this process.
    #[must_use]
    pub fn generate() -> Self {
        static LAST: std::sync::Mutex<u128> = std::sync::Mutex::new(0);
        let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *last = nanos().max(*last + 1);
        Self(format!("{:016x}", *last))
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Id {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

fn nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// Seconds since the epoch.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The folder list as one file.
#[derive(Default, Serialize, Deserialize)]
struct Folders {
    folders: Vec<Folder>,
}

/// A folder in the parts library.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    /// Its id.
    pub id: Id,
    /// Its name.
    pub name: String,
    /// The folder it sits in, or the root.
    pub parent: Option<Id>,
    /// Whether it is starred.
    #[serde(default)]
    pub favourite: bool,
}

/// An imported drawing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Searchable operator tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Setup or production notes.
    #[serde(default)]
    pub notes: String,
    /// Requested production quantity, independent of placement copies.
    #[serde(default = "one")]
    pub quantity: u32,
    /// Its id.
    pub id: Id,
    /// Its name, without the file extension.
    pub name: String,
    /// The folder it sits in, or the root.
    pub folder: Option<Id>,
    /// The file it was imported from.
    pub file_name: String,
    /// The hash of the original bytes, which the library keeps.
    pub sha256: String,
    /// The geometry in millimetres.
    pub drawing: std::sync::Arc<Drawing>,
    /// Whether it is starred.
    #[serde(default)]
    pub favourite: bool,
    /// When it was imported, in seconds since the epoch.
    pub created: u64,
    /// When it last changed.
    pub updated: u64,
}

impl Part {
    /// The drawing's extent.
    #[must_use]
    pub fn bounds(&self) -> Option<Bounds> {
        self.drawing.bounds()
    }
}

/// A cutting recipe: one of the vendor's layer banks, kept as its attribute
/// group so it binds exactly as M-Laser would bind it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    /// Its id.
    pub id: Id,
    /// The material.
    pub name: String,
    /// Which laser.
    pub laser: LaserMode,
    /// The sheet thickness.
    pub thickness_mm: f64,
    /// The assist gas, as the operator names it.
    pub gas: String,
    /// The vendor layer bank the attributes came from, 1 to 11.
    pub layer: u8,
    /// The layer group's attributes, exactly as imported.
    pub attributes: BTreeMap<String, String>,
    /// The recipe the film pass runs with, when film removal is on: a
    /// process of its own, never read from the cutting bank.
    #[serde(default)]
    pub film: Option<Id>,
    /// The hash of the file the attributes came from, which the library
    /// keeps.
    pub source_sha256: Option<String>,
    /// The vendor's note, as the card shows it.
    #[serde(default)]
    pub note: String,
    /// The process words from the vendor's file name, such as `AIR CUTTING`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The file it was imported from.
    #[serde(default)]
    pub file_name: Option<String>,
    /// The hash of its sample cut photo, which the library keeps.
    #[serde(default)]
    pub photo: Option<String>,
    /// Whether it is starred.
    #[serde(default)]
    pub favourite: bool,
    /// When it was imported.
    pub created: u64,
    /// When it last changed.
    pub updated: u64,
}

impl Recipe {
    /// The identity used to share machining defaults between recipes.
    #[must_use]
    pub fn key(&self) -> RecipeKey<'_> {
        RecipeKey {
            laser: self.laser,
            material: &self.name,
            thickness_mm: self.thickness_mm.max(0.),
            gas: &self.gas,
        }
    }
}

/// Material grouping with the laser included and no delimiter ambiguity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecipeKey<'a> {
    laser: LaserMode,
    material: &'a str,
    thickness_mm: f64,
    gas: &'a str,
}

impl RecipeKey<'_> {
    /// A stable encoding for the UI and persisted references to this group.
    #[must_use]
    pub fn encoded(&self) -> String {
        serde_json::json!([self.laser, self.material, self.thickness_mm, self.gas]).to_string()
    }
}

/// The point of the placed part the job origin stands for: one of the
/// nine positions on its bounds, as the vendor's docking point.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The front-left corner.
    #[default]
    FrontLeft,
    /// The middle of the front edge.
    FrontCenter,
    /// The front-right corner.
    FrontRight,
    /// The middle of the left edge.
    Left,
    /// The centre.
    Center,
    /// The middle of the right edge.
    Right,
    /// The back-left corner.
    BackLeft,
    /// The middle of the back edge.
    BackCenter,
    /// The back-right corner.
    BackRight,
}

impl Anchor {
    /// The anchor's point on `bounds`, `min` to `max`.
    #[must_use]
    pub fn on(self, min: [f64; 2], max: [f64; 2]) -> [f64; 2] {
        let (column, row) = match self {
            Self::FrontLeft => (0., 0.),
            Self::FrontCenter => (0.5, 0.),
            Self::FrontRight => (1., 0.),
            Self::Left => (0., 0.5),
            Self::Center => (0.5, 0.5),
            Self::Right => (1., 0.5),
            Self::BackLeft => (0., 1.),
            Self::BackCenter => (0.5, 1.),
            Self::BackRight => (1., 1.),
        };
        [min[0] + (max[0] - min[0]) * column, min[1] + (max[1] - min[1]) * row]
    }
}

/// A frozen job: its parts, a snapshot of the recipe, the machining
/// features, the drawing's contours on the sheet, and where the sheet
/// lies on the bed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// Positioning method. Missing on older jobs, whose saved offset is retained.
    #[serde(default)]
    pub placement: Option<placement::Placement>,
    /// Membership in a saved numbered sheet set.
    #[serde(default)]
    pub sheet: Option<SheetInfo>,
    /// Frozen local-size correction for this job; older jobs remain uncorrected.
    #[serde(default)]
    pub correction: Option<openlaser_correction::Profile>,
    /// An uncorrected, nominal-size machine calibration job.
    #[serde(default)]
    pub calibration: bool,
    /// Operator overrides to automatic part grouping.
    #[serde(default)]
    pub grouping: openlaser_core::grouping::Grouping,
    /// Stock reference and nesting settings, never included as cut paths.
    #[serde(default)]
    pub nesting: Option<openlaser_core::nesting::Nesting>,
    /// Searchable operator tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Setup or production notes.
    #[serde(default)]
    pub notes: String,
    /// Requested production quantity, independent of placement copies.
    #[serde(default = "one")]
    pub quantity: u32,
    /// Operator checks saved with the job.
    #[serde(default)]
    pub preflight: preflight::JobPreflight,
    /// Its id.
    pub id: Id,
    /// Its name.
    pub name: String,
    /// The folder it sits in, or the root.
    pub folder: Option<Id>,
    /// The parts it cuts, whose drawings join in this order into the job's
    /// drawing ([`JobDrawing`]). Stored under `part`: one part as its id, as
    /// before a job could cut several, and several as a list.
    #[serde(rename = "part", with = "stored_parts")]
    pub parts: Vec<Id>,
    /// The recipe as it was when the job was saved.
    pub recipe: Recipe,
    /// The film process as it was when the job was saved.
    #[serde(default)]
    pub film: Option<Recipe>,
    /// The machining features.
    pub features: Features,
    /// The job drawing's contours on the sheet, copies and all; empty for
    /// the drawing as drawn.
    #[serde(default)]
    pub placed: Vec<Placed>,
    /// What the machine adds to a drawing coordinate: where the sheet lies
    /// on the bed, once placed.
    #[serde(default)]
    pub zero: Option<[f64; 2]>,
    /// Which point of the part the origin stands for.
    #[serde(default)]
    pub anchor: Anchor,
    /// Whether it is starred.
    #[serde(default)]
    pub favourite: bool,
    /// When it was saved.
    pub created: u64,
    /// When it last changed.
    pub updated: u64,
}

/// Version 2 holds several parts. A job of one part stays version 1, which
/// every earlier build reads.
impl Schema for Job {
    const NEWEST: u32 = 2;

    fn version(&self) -> u32 {
        if self.parts.len() > 1 { 2 } else { 1 }
    }
}

/// Stable grouping for individual sheet jobs saved together.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetInfo {
    /// Identity of the set's original folder.
    pub set: Id,
    /// One-based sheet number.
    pub number: u32,
    /// Total sheets when saved.
    pub total: u32,
}

/// An item file the library left out when it opened, untouched on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The file, relative to the library root, such as `jobs/<id>.json`.
    pub file: String,
    /// Why it was left out.
    pub reason: String,
}

/// The library: everything loaded, every change written through.
#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
    folders: Vec<Folder>,
    parts: BTreeMap<Id, Part>,
    recipes: BTreeMap<Id, Recipe>,
    jobs: BTreeMap<Id, Job>,
    skipped: Vec<Skipped>,
}

impl Library {
    /// Opens the library at `root`, creating it when absent. An item file
    /// that cannot be read or does not validate is left out, with every item
    /// that depends on it, and listed by [`Library::skipped`]; the folder
    /// tree, which every item belongs to, must still load.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        for dir in ["parts", "recipes", "jobs", "originals", "photos"] {
            store::create_dir(&root.join(dir))?;
        }
        history::recover(&root)?;
        let folders = store::read_optional::<Folders>(&root.join("folders.json"))?
            .unwrap_or_default()
            .folders;
        let mut skipped = Vec::new();
        let mut library = Self {
            root: root.clone(),
            folders,
            parts: store::read_all(&root, "parts", &mut skipped)?,
            recipes: store::read_all(&root, "recipes", &mut skipped)?,
            jobs: store::read_all(&root, "jobs", &mut skipped)?,
            skipped: Vec::new(),
        };
        validation::folders(&library.folders).map_err(|error| Error::Format {
            path: root.join("folders.json").display().to_string(),
            reason: error.to_string(),
        })?;
        // Leaving an item out can strand what depends on it: repeat until
        // everything left validates against everything else left.
        loop {
            let invalid = validation::invalid_items(&library);
            if invalid.is_empty() {
                break;
            }
            for (kind, id, reason) in invalid {
                match kind {
                    "parts" => drop(library.parts.remove(&id)),
                    "recipes" => drop(library.recipes.remove(&id)),
                    _ => drop(library.jobs.remove(&id)),
                }
                skipped.push(Skipped { file: format!("{kind}/{id}.json"), reason });
            }
        }
        validation::library(&library)?;
        skipped.sort_by(|a, b| a.file.cmp(&b.file));
        library.skipped = skipped;
        Ok(library)
    }

    /// The item files left out when the library opened, and why.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// Where the library lives.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    // Folders -------------------------------------------------------------------

    /// Every folder.
    #[must_use]
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Makes a folder.
    pub fn add_folder(&mut self, name: &str, parent: Option<Id>) -> Result<Folder> {
        let name = clean_name(name)?;
        let folder = Folder { id: Id::generate(), name, parent, favourite: false };
        let mut folders = self.folders.clone();
        folders.push(folder.clone());
        self.write_folders(folders)?;
        Ok(folder)
    }

    /// Changes a folder.
    pub fn update_folder(&mut self, id: &Id, change: impl FnOnce(&mut Folder)) -> Result<Folder> {
        let mut folders = self.folders.clone();
        let folder = folders
            .iter_mut()
            .find(|f| &f.id == id)
            .ok_or_else(|| Error::Missing { kind: "folder", id: id.clone() })?;
        change(folder);
        validation::same_id(id, &folder.id)?;
        folder.name = clean_name(&folder.name)?;
        let folder = folder.clone();
        self.write_folders(folders)?;
        Ok(folder)
    }

    /// Removes an empty folder.
    pub fn remove_folder(&mut self, id: &Id) -> Result<()> {
        let used = self.folders.iter().any(|f| f.parent.as_ref() == Some(id))
            || self.parts.values().any(|p| p.folder.as_ref() == Some(id))
            || self.jobs.values().any(|j| j.folder.as_ref() == Some(id));
        if used {
            return Err(Error::InUse("the folder is not empty".into()));
        }
        let mut folders = self.folders.clone();
        folders.retain(|f| &f.id != id);
        if folders.len() == self.folders.len() {
            return Err(Error::Missing { kind: "folder", id: id.clone() });
        }
        self.write_folders(folders)
    }

    fn write_folders(&mut self, folders: Vec<Folder>) -> Result<()> {
        validation::folders(&folders)?;
        let candidate = Folders { folders };
        history::write(&self.root.join("folders.json"), &candidate)?;
        self.folders = candidate.folders;
        Ok(())
    }

    // Parts ----------------------------------------------------------------------

    /// Every part, oldest first.
    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.values()
    }

    /// One part.
    pub fn part(&self, id: &Id) -> Result<&Part> {
        self.parts.get(id).ok_or_else(|| Error::Missing { kind: "part", id: id.clone() })
    }

    /// Imports a drawing, keeping the original bytes.
    pub fn add_part(&mut self, file_name: &str, bytes: &[u8], drawing: Drawing) -> Result<Part> {
        let name = clean_name(
            Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(file_name),
        )?;
        let sha256 = self.keep_original(bytes)?;
        // A newer importer can recover geometry an older one skipped. Keep
        // existing jobs on their original part and add the corrected drawing.
        if let Some(existing) =
            self.parts.values().find(|p| p.sha256 == sha256 && *p.drawing == drawing)
        {
            return Ok(existing.clone());
        }
        let now = now();
        let part = Part {
            tags: Vec::new(),
            notes: String::new(),
            quantity: 1,
            id: Id::generate(),
            name,
            folder: None,
            file_name: file_name.to_owned(),
            sha256,
            drawing: std::sync::Arc::new(drawing),
            favourite: false,
            created: now,
            updated: now,
        };
        validation::part(&part, self)?;
        store::commit(&self.path("parts", &part.id), &mut self.parts, part)
    }

    /// Keeps `bytes` by their hash, once, and returns the hash.
    pub fn keep_original(&self, bytes: &[u8]) -> Result<String> {
        store::keep_content(&self.root.join("originals"), bytes)
    }

    /// The original bytes of a part or a recipe.
    pub fn original(&self, sha256: &str) -> Result<Vec<u8>> {
        store::read_content(&self.root.join("originals").join(hash_name(sha256)?), sha256)
    }

    /// Keeps a photo by its hash, once, and returns the hash. The bytes
    /// are kept as given; their format is read from them when served.
    pub fn add_photo(&self, bytes: &[u8]) -> Result<String> {
        store::keep_content(&self.root.join("photos"), bytes)
    }

    /// A photo's bytes.
    pub fn photo(&self, sha256: &str) -> Result<Vec<u8>> {
        store::read_content(&self.root.join("photos").join(hash_name(sha256)?), sha256)
    }

    /// A copy of a part beside it, sharing the original bytes.
    pub fn duplicate_part(&mut self, id: &Id) -> Result<Part> {
        let mut copy = self.part(id)?.clone();
        copy.id = Id::generate();
        copy.name = clean_name(&format!("{} copy", copy.name))?;
        copy.favourite = false;
        copy.created = now();
        copy.updated = copy.created;
        validation::part(&copy, self)?;
        store::commit(&self.path("parts", &copy.id), &mut self.parts, copy)
    }

    /// A part drawn from another's geometry, such as a simplified drawing:
    /// the same original file and details under a new name. The part it
    /// came from is unchanged for the jobs that use it; the same result
    /// again is the part already made.
    pub fn add_derived_part(&mut self, from: &Id, name: &str, drawing: Drawing) -> Result<Part> {
        let source = self.part(from)?;
        if let Some(existing) =
            self.parts.values().find(|p| p.sha256 == source.sha256 && *p.drawing == drawing)
        {
            return Ok(existing.clone());
        }
        let mut part = source.clone();
        part.id = Id::generate();
        part.name = clean_name(name)?;
        part.drawing = std::sync::Arc::new(drawing);
        part.favourite = false;
        part.created = now();
        part.updated = part.created;
        validation::part(&part, self)?;
        store::commit(&self.path("parts", &part.id), &mut self.parts, part)
    }

    /// Changes a part's name, folder or star.
    pub fn update_part(&mut self, id: &Id, change: impl FnOnce(&mut Part)) -> Result<Part> {
        let mut part = store::edited(self.part(id)?, change)?;
        part.name = clean_name(&part.name)?;
        part.updated = now();
        validation::part(&part, self)?;
        store::commit(&self.path("parts", id), &mut self.parts, part)
    }

    /// Removes a part no job uses.
    pub fn remove_part(&mut self, id: &Id) -> Result<()> {
        if let Some(job) = self.jobs.values().find(|j| j.parts.contains(id)) {
            return Err(Error::InUse(format!("the saved job “{}” uses this part", job.name)));
        }
        self.part(id)?;
        history::remove(&self.path("parts", id))?;
        self.parts.remove(id);
        Ok(())
    }

    // Recipes --------------------------------------------------------------------

    /// Every recipe, oldest first.
    pub fn recipes(&self) -> impl Iterator<Item = &Recipe> {
        self.recipes.values()
    }

    /// One recipe.
    pub fn recipe(&self, id: &Id) -> Result<&Recipe> {
        self.recipes.get(id).ok_or_else(|| Error::Missing { kind: "recipe", id: id.clone() })
    }

    /// The recipe imported from the file with this hash, if any.
    #[must_use]
    pub fn recipe_by_source(&self, sha256: &str) -> Option<&Recipe> {
        self.recipes.values().find(|r| r.source_sha256.as_deref() == Some(sha256))
    }

    /// Adds a recipe; the id is assigned here.
    pub fn add_recipe(&mut self, mut recipe: Recipe) -> Result<Recipe> {
        recipe.id = Id::generate();
        recipe.name = clean_name(&recipe.name)?;
        let now = now();
        recipe.created = now;
        recipe.updated = now;
        validation::library_recipe(&recipe, self)?;
        store::commit(&self.path("recipes", &recipe.id), &mut self.recipes, recipe)
    }

    /// Changes a recipe. Jobs keep the snapshot they were saved with.
    pub fn update_recipe(&mut self, id: &Id, change: impl FnOnce(&mut Recipe)) -> Result<Recipe> {
        let mut recipe = store::edited(self.recipe(id)?, change)?;
        recipe.name = clean_name(&recipe.name)?;
        recipe.updated = now();
        validation::library_recipe(&recipe, self)?;
        store::commit(&self.path("recipes", id), &mut self.recipes, recipe)
    }

    /// Removes a recipe; saved jobs keep their own copy.
    pub fn remove_recipe(&mut self, id: &Id) -> Result<()> {
        self.recipe(id)?;
        if self.recipes.values().any(|recipe| recipe.film.as_ref() == Some(id)) {
            return Err(Error::InUse("a recipe uses this film process".into()));
        }
        history::remove(&self.path("recipes", id))?;
        self.recipes.remove(id);
        Ok(())
    }

    // Jobs -----------------------------------------------------------------------

    /// Every job, oldest first.
    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values()
    }

    /// One job.
    pub fn job(&self, id: &Id) -> Result<&Job> {
        self.jobs.get(id).ok_or_else(|| Error::Missing { kind: "job", id: id.clone() })
    }

    /// Saves a job; the id is assigned here and its parts must exist.
    pub fn add_job(&mut self, mut job: Job) -> Result<Job> {
        job.id = Id::generate();
        job.name = clean_name(&job.name)?;
        let now = now();
        job.created = now;
        job.updated = now;
        validation::job(&job, self)?;
        store::commit(&self.path("jobs", &job.id), &mut self.jobs, job)
    }

    /// Save all sheets and their folder in one durable, undoable transaction.
    pub fn add_sheet_set(&mut self, name: &str, mut jobs: Vec<Job>) -> Result<(Folder, Vec<Job>)> {
        if !(2..=500).contains(&jobs.len()) {
            return Err(Error::Invalid("save between two and 500 sheets".into()));
        }
        let name = clean_name(name)?;
        let folder =
            Folder { id: Id::generate(), name: name.clone(), parent: None, favourite: false };
        let mut candidate = self.clone();
        candidate.folders.push(folder.clone());
        let total =
            u32::try_from(jobs.len()).map_err(|_| Error::Invalid("too many sheets".into()))?;
        let mut files = Vec::new();
        let path = self.root.join("folders.json");
        files.push((
            path.clone(),
            store::encode(&path, &Folders { folders: candidate.folders.clone() })?,
        ));
        let now = now();
        for (index, job) in jobs.iter_mut().enumerate() {
            let number =
                u32::try_from(index + 1).map_err(|_| Error::Invalid("too many sheets".into()))?;
            job.id = Id::generate();
            job.created = now;
            job.updated = now;
            job.name =
                format!("{} · Sheet {number:02}", name.chars().take(100).collect::<String>());
            job.folder = Some(folder.id.clone());
            job.sheet = Some(SheetInfo { set: folder.id.clone(), number, total });
            candidate.jobs.insert(job.id.clone(), job.clone());
            let path = self.path("jobs", &job.id);
            files.push((path.clone(), store::encode(&path, job)?));
        }
        validation::library(&candidate)?;
        history::write_batch(&self.root, format!("Saved {total} sheets: {name}"), files)?;
        *self = candidate;
        Ok((folder, jobs))
    }

    /// Changes a job.
    pub fn update_job(&mut self, id: &Id, change: impl FnOnce(&mut Job)) -> Result<Job> {
        let mut job = store::edited(self.job(id)?, change)?;
        job.name = clean_name(&job.name)?;
        job.updated = now();
        validation::job(&job, self)?;
        store::commit(&self.path("jobs", id), &mut self.jobs, job)
    }

    /// Duplicates a saved job without opening or altering the working draft.
    pub fn duplicate_job(&mut self, id: &Id) -> Result<Job> {
        let mut copy = self.job(id)?.clone();
        copy.name = format!("{} copy", copy.name.chars().take(115).collect::<String>());
        copy.favourite = false;
        copy.sheet = None;
        self.add_job(copy)
    }

    /// Removes a job.
    pub fn remove_job(&mut self, id: &Id) -> Result<()> {
        self.job(id)?;
        history::remove(&self.path("jobs", id))?;
        self.jobs.remove(id);
        Ok(())
    }

    /// The most recently saved job on a recipe, whose features a new job on
    /// the same recipe starts from.
    #[must_use]
    pub fn last_job_on(&self, recipe_key: &RecipeKey<'_>) -> Option<&Job> {
        self.jobs.values().filter(|j| j.recipe.key() == *recipe_key).max_by_key(|j| j.updated)
    }

    fn path(&self, kind: &str, id: &Id) -> PathBuf {
        self.root.join(kind).join(format!("{id}.json"))
    }
}

/// A hash as a file name: hexadecimal only, so it cannot leave its directory.
fn hash_name(sha256: &str) -> Result<&str> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Invalid("not a hash".into()));
    }
    Ok(sha256)
}

/// A name trimmed and checked: not empty, no path separators or control
/// characters, at most 120 characters.
fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("the name is empty".into()));
    }
    if name.chars().count() > 120 || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(Error::Invalid("the name has invalid characters or is too long".into()));
    }
    Ok(name.to_owned())
}

/// Reads any item file of this crate's kinds, for tools that inspect a
/// library without opening it.
pub fn read_item<T: DeserializeOwned + Schema>(path: &Path) -> Result<T> {
    store::read(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::{Contour, Curve, Point};

    #[test]
    fn concurrent_creates_have_distinct_time_ordered_ids() {
        let threads: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| (0..500).map(|_| Id::generate()).collect::<Vec<_>>()))
            .collect();
        let mut all = std::collections::BTreeSet::new();
        for thread in threads {
            let ids = thread.join().unwrap();
            assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
            for id in ids {
                assert!(all.insert(id));
            }
        }
        assert_eq!(all.len(), 4000);
    }

    #[test]
    fn improved_import_geometry_creates_a_part_without_changing_saved_geometry() {
        let root = scratch("import-revision");
        let mut library = Library::open(&root).unwrap();
        let original = library.add_part("plate.dxf", b"original", square()).unwrap();
        let mut corrected = square();
        corrected.contours.push(square().contours.remove(0));
        let updated = library.add_part("plate.dxf", b"original", corrected.clone()).unwrap();
        assert_ne!(original.id, updated.id);
        assert_eq!(*library.part(&original.id).unwrap().drawing, square());
        assert_eq!(*updated.drawing, corrected);
        assert_eq!(library.add_part("plate.dxf", b"original", corrected).unwrap().id, updated.id);
        assert_eq!(library.original(&original.sha256).unwrap(), b"original");
    }

    #[test]
    fn saved_history_restores_metadata_deletions_and_survives_reopening() {
        let root = scratch("history");
        let mut library = Library::open(&root).unwrap();
        let part = library.add_part("plate.dxf", b"original", square()).unwrap();
        let same = library.add_part("another-name.dxf", b"original", square()).unwrap();
        assert_eq!(part.id, same.id);
        library
            .update_part(&part.id, |p| {
                p.tags = vec!["steel".into()];
                p.notes = "Deburr".into();
                p.quantity = 6;
            })
            .unwrap();
        library.remove_part(&part.id).unwrap();
        let revision = library.edit_history().unwrap().revision;
        assert!(library.undo_saved(true, revision - 1).is_err());
        library.undo_saved(true, revision).unwrap();
        assert_eq!(library.part(&part.id).unwrap().quantity, 6);
        drop(library);
        let mut library = Library::open(&root).unwrap();
        library.undo_saved(true, library.edit_history().unwrap().revision).unwrap();
        assert_eq!(library.part(&part.id).unwrap().quantity, 1);
        library.undo_saved(false, library.edit_history().unwrap().revision).unwrap();
        assert_eq!(library.part(&part.id).unwrap().tags, ["steel"]);
        let path = root.join("parts").join(format!("{}.json", part.id));
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
        assert!(library.undo_saved(true, library.edit_history().unwrap().revision).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openlaser-library-{name}-{}", nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn edit(path: &Path, change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        change(&mut value);
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(path, &bytes).unwrap();
        bytes
    }

    fn square() -> Drawing {
        let corners = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
        let curves = (0..4)
            .map(|i| Curve::Line {
                start: Point::from(corners[i]),
                end: Point::from(corners[(i + 1) % 4]),
            })
            .collect();
        Drawing { contours: vec![Contour { layer: "0".into(), curves }] }
    }

    fn job(part: &Id, recipe: Recipe) -> Job {
        Job {
            placement: None,
            sheet: None,
            correction: None,
            calibration: false,
            grouping: openlaser_core::grouping::Grouping::default(),
            nesting: None,
            tags: Vec::new(),
            notes: String::new(),
            quantity: 1,
            preflight: preflight::JobPreflight::default(),
            id: Id::from(""),
            name: "plate".into(),
            folder: None,
            parts: vec![part.clone()],
            recipe,
            film: None,
            features: Features::default(),
            placed: Vec::new(),
            zero: None,
            anchor: Anchor::default(),
            favourite: false,
            created: 0,
            updated: 0,
        }
    }

    fn recipe() -> Recipe {
        Recipe {
            id: Id::from(""),
            name: "Stainless steel".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 5.,
            gas: "Air".into(),
            layer: 1,
            attributes: BTreeMap::from([("CutSpeed".to_owned(), "50".to_owned())]),
            film: None,
            source_sha256: None,
            note: String::new(),
            tags: Vec::new(),
            file_name: None,
            photo: None,
            favourite: false,
            created: 0,
            updated: 0,
        }
    }

    /// A part, a recipe, a job and a folder round-trip through disk: a
    /// second open reads back what the first wrote, the original bytes are
    /// kept by hash, and the sticky features come from the last job on the
    /// recipe.
    #[test]
    fn items_round_trip_through_disk() {
        let root = scratch("round-trip");
        let (part, job, folder) = {
            let mut library = Library::open(&root).unwrap();
            let folder = library.add_folder("Brackets", None).unwrap();
            let part = library.add_part("bracket-v3.dxf", b"0\nEOF\n", square()).unwrap();
            let part =
                library.update_part(&part.id, |p| p.folder = Some(folder.id.clone())).unwrap();
            let recipe = library.add_recipe(recipe()).unwrap();
            let job = library
                .add_job(Job {
                    placement: None,
                    sheet: None,
                    correction: None,
                    calibration: false,
                    grouping: openlaser_core::grouping::Grouping::default(),
                    nesting: None,
                    tags: Vec::new(),
                    notes: String::new(),
                    quantity: 1,
                    preflight: preflight::JobPreflight::default(),
                    id: Id::from(""),
                    name: "bracket".into(),
                    folder: None,
                    parts: vec![part.id.clone()],
                    recipe: recipe.clone(),
                    film: None,
                    features: Features::default(),
                    placed: Vec::new(),
                    zero: Some([120., 80.]),
                    anchor: Anchor::default(),
                    favourite: true,
                    created: 0,
                    updated: 0,
                })
                .unwrap();
            assert_eq!(library.original(&part.sha256).unwrap(), b"0\nEOF\n");
            let copy = library.duplicate_part(&part.id).unwrap();
            assert_eq!((copy.name.as_str(), &copy.sha256), ("bracket-v3 copy", &part.sha256));
            library.remove_part(&copy.id).unwrap();
            let photo = library.add_photo(b"\x89PNG").unwrap();
            assert_eq!(library.photo(&photo).unwrap(), b"\x89PNG");
            assert!(library.photo("../x").is_err());
            assert_eq!(library.last_job_on(&recipe.key()).map(|j| &j.id), Some(&job.id));
            (part, job, folder)
        };
        let library = Library::open(&root).unwrap();
        assert_eq!(library.part(&part.id).unwrap(), &part);
        assert_eq!(library.job(&job.id).unwrap(), &job);
        assert_eq!(library.folders(), [folder]);
        assert_eq!(library.recipes().count(), 1);
        assert!(
            !std::fs::read_dir(root.join("parts")).unwrap().any(|e| e
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "tmp"))
        );
    }

    /// A part in use by a job and a folder with content cannot be removed;
    /// a removed job frees its part; a file from a newer build is left out.
    #[test]
    fn references_are_protected_and_newer_files_left_out() {
        let root = scratch("references");
        let mut library = Library::open(&root).unwrap();
        let folder = library.add_folder("Customer", None).unwrap();
        let part = library.add_part("plate.dxf", b"plate", square()).unwrap();
        library.update_part(&part.id, |p| p.folder = Some(folder.id.clone())).unwrap();
        let recipe = library.add_recipe(recipe()).unwrap();
        let job = library
            .add_job(Job {
                placement: None,
                sheet: None,
                correction: None,
                calibration: false,
                grouping: openlaser_core::grouping::Grouping::default(),
                nesting: None,
                tags: Vec::new(),
                notes: String::new(),
                quantity: 1,
                preflight: preflight::JobPreflight::default(),
                id: Id::from(""),
                name: "plate".into(),
                folder: None,
                parts: vec![part.id.clone()],
                recipe,
                film: None,
                features: Features::default(),
                placed: Vec::new(),
                zero: None,
                anchor: Anchor::default(),
                favourite: false,
                created: 0,
                updated: 0,
            })
            .unwrap();
        assert!(matches!(library.remove_part(&part.id), Err(Error::InUse(_))));
        assert!(matches!(library.remove_folder(&folder.id), Err(Error::InUse(_))));
        library.remove_job(&job.id).unwrap();
        library.remove_part(&part.id).unwrap();
        library.remove_folder(&folder.id).unwrap();
        assert!(library.add_folder("  ", None).is_err());
        let future = root.join("parts").join("future.json");
        std::fs::write(&future, br#"{"version": 99}"#).unwrap();
        let reopened = Library::open(&root).unwrap();
        assert_eq!(reopened.skipped().len(), 1);
        assert_eq!(reopened.skipped()[0].file, "parts/future.json");
        assert!(reopened.skipped()[0].reason.contains("99"), "{:?}", reopened.skipped());
        assert_eq!(std::fs::read(&future).unwrap(), br#"{"version": 99}"#, "never rewritten");
    }

    /// A job of one part is stored exactly as before, so earlier builds
    /// still read it; a job of several parts is schema version 2, joins its
    /// parts' contours in order and protects every part it cuts.
    #[test]
    fn jobs_of_several_parts_are_version_two_and_protect_each_part() {
        let root = scratch("several-parts");
        let mut library = Library::open(&root).unwrap();
        let recipe = library.add_recipe(recipe()).unwrap();
        let a = library.add_part("a.dxf", b"a", square()).unwrap();
        let mut two = square();
        two.contours.push(two.contours[0].clone());
        let b = library.add_part("b.svg", b"b", two).unwrap();
        let single = library.add_job(job(&a.id, recipe.clone())).unwrap();
        let file = |id: &Id| -> serde_json::Value {
            let path = root.join("jobs").join(format!("{id}.json"));
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        };
        assert_eq!(file(&single.id)["version"], 1);
        assert_eq!(file(&single.id)["part"], a.id.as_str());

        let mut several = job(&a.id, recipe.clone());
        several.parts = vec![b.id.clone(), a.id.clone()];
        several.placed = vec![Placed::drawn(0), Placed::drawn(1), Placed::drawn(2)];
        let several = library.add_job(several).unwrap();
        assert_eq!(file(&several.id)["version"], 2);
        assert_eq!(file(&several.id)["part"], serde_json::json!([b.id, a.id]));
        let drawing = library.job_drawing(&several.parts).unwrap();
        assert_eq!((drawing.contours(), drawing.range(1)), (3, 2..3));
        assert_eq!(Library::open(&root).unwrap().job(&several.id).unwrap(), &several);
        for part in [&a.id, &b.id] {
            let refused = library.remove_part(part).unwrap_err().to_string();
            assert!(refused.contains("plate"), "{refused}");
        }

        let refused = |change: fn(&mut Job)| {
            let mut candidate = several.clone();
            change(&mut candidate);
            library.clone().update_job(&several.id, |job| *job = candidate).is_err()
        };
        assert!(refused(|job| job.parts.clear()));
        assert!(refused(|job| job.parts.push(job.parts[0].clone())));
        assert!(refused(|job| job.parts.push(Id::from("missing"))));
        assert!(refused(|job| job.placed.push(Placed::drawn(3))));
        assert!(!refused(|job| job.placed.truncate(2)));

        // A version 1 file cannot hold several parts, and a later version is
        // left for the build that wrote it.
        let path = library.path("jobs", &several.id);
        edit(&path, |job| job["version"] = 1.into());
        let reason = |library: &Library| library.skipped()[0].reason.clone();
        let reopened = Library::open(&root).unwrap();
        assert!(reason(&reopened).contains("needs schema version 2"), "{}", reason(&reopened));
        edit(&path, |job| job["version"] = 3.into());
        let reopened = Library::open(&root).unwrap();
        assert!(reason(&reopened).contains("reads up to 2"), "{}", reason(&reopened));
        assert_eq!(reopened.job(&single.id).unwrap(), &single);
    }

    #[test]
    fn rejected_edits_preserve_memory_and_reopened_items() {
        let root = scratch("rejected-edits");
        let mut library = Library::open(&root).unwrap();
        let original = library.add_recipe(recipe()).unwrap();
        let invalid: [fn(&mut Recipe); 4] = [
            |recipe| recipe.name.clear(),
            |recipe| recipe.thickness_mm = -1.,
            |recipe| recipe.layer = 12,
            |recipe| recipe.id = Id::from("changed"),
        ];
        for change in invalid {
            assert!(library.update_recipe(&original.id, change).is_err());
            assert_eq!(library.recipe(&original.id).unwrap(), &original);
            assert_eq!(Library::open(&root).unwrap().recipe(&original.id).unwrap(), &original);
        }
        let parent = library.add_folder("Parent", None).unwrap();
        let child = library.add_folder("Child", Some(parent.id.clone())).unwrap();
        for next in [parent.id.clone(), child.id.clone(), Id::from("missing")] {
            assert!(
                library.update_folder(&parent.id, |folder| folder.parent = Some(next)).is_err()
            );
            assert_eq!(library.folders(), [parent.clone(), child.clone()]);
            assert_eq!(Library::open(&root).unwrap().folders(), [parent.clone(), child.clone()]);
        }
    }

    #[test]
    fn failed_disk_changes_do_not_commit_candidates_or_removals() {
        let root = scratch("failed-disk");
        let mut library = Library::open(&root).unwrap();
        let original = library.add_recipe(recipe()).unwrap();
        let path = library.path("recipes", &original.id);
        let backup = root.join("saved-recipe");
        // A directory in place of the target reliably refuses rename and
        // remove_file, even when the test user can bypass Unix permissions.
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            library.update_recipe(&original.id, |recipe| recipe.name = "Changed".into()).is_err()
        );
        assert!(library.remove_recipe(&original.id).is_err());
        assert_eq!(library.recipe(&original.id).unwrap(), &original);
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        assert_eq!(Library::open(&root).unwrap().recipe(&original.id).unwrap(), &original);

        let directory = root.join("recipes");
        let saved = root.join("saved-recipes");
        std::fs::rename(&directory, &saved).unwrap();
        std::fs::write(&directory, b"blocks temporary file creation").unwrap();
        assert!(library.add_recipe(recipe()).is_err());
        assert_eq!(library.recipes().count(), 1);
        std::fs::remove_file(&directory).unwrap();
        std::fs::rename(&saved, &directory).unwrap();
        let updated =
            library.update_recipe(&original.id, |recipe| recipe.name = "Changed".into()).unwrap();
        assert_eq!(Library::open(&root).unwrap().recipe(&original.id).unwrap(), &updated);
        library.remove_recipe(&original.id).unwrap();
        assert_eq!(Library::open(&root).unwrap().recipes().count(), 0);
    }

    /// A bad item file is left out and named, with whatever depends on it,
    /// and the rest opens; a bad folder tree still refuses to open.
    #[test]
    fn invalid_files_are_named_and_left_out_without_overwriting() {
        let root = scratch("invalid-files");
        let mut library = Library::open(&root).unwrap();
        let recipe = library.add_recipe(recipe()).unwrap();
        let kept = library.add_part("kept.dxf", b"kept", square()).unwrap();
        let broken = library.add_part("broken.dxf", b"broken", square()).unwrap();
        let duplicate = root.join("recipes/duplicate.json");
        store::write(&duplicate, &recipe).unwrap();
        let version = root.join("recipes/unsupported.json");
        std::fs::write(&version, br#"{"version":0}"#).unwrap();
        // A field this build does not know, as a newer build might write it.
        let newer = library.add_job(job(&kept.id, recipe.clone())).unwrap();
        let newer_file = format!("jobs/{}.json", newer.id);
        edit(&root.join(&newer_file), |job| job["preflight"]["surprise"] = true.into());
        // A part that no longer validates strands the job built on it.
        let stranded = library.add_job(job(&broken.id, recipe.clone())).unwrap();
        let broken_file = format!("parts/{}.json", broken.id);
        let damaged = edit(&root.join(&broken_file), |part| part["sha256"] = "x".into());
        let reopened = Library::open(&root).unwrap();
        let stranded_file = format!("jobs/{}.json", stranded.id);
        let mut expected = vec![
            "recipes/duplicate.json",
            "recipes/unsupported.json",
            &newer_file,
            &broken_file,
            &stranded_file,
        ];
        expected.sort_unstable();
        let files: Vec<_> = reopened.skipped().iter().map(|s| s.file.as_str()).collect();
        assert_eq!(files, expected);
        let reason =
            |file: &str| &reopened.skipped().iter().find(|s| s.file == file).unwrap().reason;
        assert!(reason("recipes/duplicate.json").contains("file name"));
        assert!(reason("recipes/unsupported.json").contains("schema version 0"));
        assert!(reason(&newer_file).contains("surprise"), "{}", reason(&newer_file));
        assert!(reason(&stranded_file).contains("no part"), "{}", reason(&stranded_file));
        assert_eq!(reopened.recipe(&recipe.id).unwrap(), &recipe);
        assert!(reopened.part(&kept.id).is_ok());
        assert_eq!(std::fs::read(root.join(&broken_file)).unwrap(), damaged, "left as found");
        std::fs::remove_file(&duplicate).unwrap();
        std::fs::remove_file(&version).unwrap();
        let folder = Folder {
            id: Id::from("folder"),
            name: "Folder".into(),
            parent: None,
            favourite: false,
        };
        store::write(
            &root.join("folders.json"),
            &Folders { folders: vec![folder.clone(), folder.clone()] },
        )
        .unwrap();
        assert!(
            matches!(Library::open(&root), Err(Error::Format { reason, .. }) if reason.contains("duplicate folder"))
        );
        let cycle = Folder { parent: Some(folder.id.clone()), ..folder };
        store::write(&root.join("folders.json"), &Folders { folders: vec![cycle] }).unwrap();
        assert!(
            matches!(Library::open(&root), Err(Error::Format { reason, .. }) if reason.contains("parent cycle"))
        );
    }

    #[test]
    fn recipe_identity_includes_the_laser_and_delimiters_are_unambiguous() {
        let first = Recipe { name: "a|1".into(), thickness_mm: 2., gas: "x".into(), ..recipe() };
        let second = Recipe { name: "a".into(), thickness_mm: 1., gas: "2|x".into(), ..recipe() };
        assert_ne!(first.key(), second.key());
        assert_ne!(first.key().encoded(), second.key().encoded());
        let co2 = Recipe { laser: LaserMode::Co2, ..first.clone() };
        assert_ne!(first.key(), co2.key());
        assert_ne!(first.key().encoded(), co2.key().encoded());
    }

    #[test]
    fn corrupted_content_is_neither_returned_nor_silently_accepted() {
        let root = scratch("content");
        let library = Library::open(&root).unwrap();
        let hash = library.keep_original(b"original").unwrap();
        std::fs::write(root.join("originals").join(&hash), b"changed").unwrap();
        assert!(library.original(&hash).is_err());
        assert!(library.keep_original(b"original").is_err());
        assert_eq!(std::fs::read(root.join("originals").join(&hash)).unwrap(), b"changed");
    }
}
