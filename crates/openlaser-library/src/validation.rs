// SPDX-License-Identifier: GPL-3.0-or-later

//! The same model rules for additions, edits and files loaded from disk.

use crate::{
    Error, Folder, Id, Job, Library, Part, Recipe, Result, clean_name, hash_name, store::HasId,
};
use std::collections::{BTreeMap, BTreeSet};

/// Validate the complete candidate before loading it or committing a history edit.
pub(crate) fn library(library: &Library) -> Result<()> {
    folders(&library.folders)
        .map_err(|error| invalid(&library.root.join("folders.json"), &error))?;
    collection(library, "parts", &library.parts, part)?;
    collection(library, "recipes", &library.recipes, library_recipe)?;
    collection(library, "jobs", &library.jobs, job)
}

/// Every part, recipe and job that fails on its own or against the rest:
/// its collection, id and reason.
pub(crate) fn invalid_items(library: &Library) -> Vec<(&'static str, Id, String)> {
    let mut invalid = Vec::new();
    each(library, "parts", &library.parts, part, &mut invalid);
    each(library, "recipes", &library.recipes, library_recipe, &mut invalid);
    each(library, "jobs", &library.jobs, job, &mut invalid);
    invalid
}

fn each<T: HasId>(
    library: &Library,
    kind: &'static str,
    items: &BTreeMap<Id, T>,
    check: fn(&T, &Library) -> Result<()>,
    invalid: &mut Vec<(&'static str, Id, String)>,
) {
    for (key, item) in items {
        if let Err(error) = same_id(key, item.id()).and_then(|()| check(item, library)) {
            invalid.push((kind, key.clone(), error.to_string()));
        }
    }
}

fn invalid(path: &std::path::Path, error: &Error) -> Error {
    Error::Format { path: path.display().to_string(), reason: error.to_string() }
}

fn collection<T: HasId>(
    library: &Library,
    kind: &str,
    items: &BTreeMap<Id, T>,
    check: fn(&T, &Library) -> Result<()>,
) -> Result<()> {
    for (key, item) in items {
        let path = library.path(kind, key);
        same_id(key, item.id()).map_err(|error| invalid(&path, &error))?;
        check(item, library).map_err(|error| invalid(&path, &error))?;
    }
    Ok(())
}

pub(crate) fn id(id: &Id) -> Result<()> {
    let text = id.as_str();
    if text.is_empty()
        || text.len() > 128
        || !text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(Error::Invalid(format!("invalid item id {text:?}")));
    }
    Ok(())
}

pub(crate) fn same_id(expected: &Id, actual: &Id) -> Result<()> {
    if expected != actual {
        return Err(Error::Invalid("an item's id cannot change".into()));
    }
    Ok(())
}

pub(crate) fn folders(folders: &[Folder]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for folder in folders {
        id(&folder.id)?;
        clean_name(&folder.name)?;
        if !ids.insert(&folder.id) {
            return Err(Error::Invalid(format!("duplicate folder id {}", folder.id)));
        }
        let mut seen = BTreeSet::from([&folder.id]);
        let mut parent = folder.parent.as_ref();
        while let Some(next) = parent {
            if !seen.insert(next) {
                return Err(Error::Invalid(format!("folder {} has a parent cycle", folder.id)));
            }
            parent = folders
                .iter()
                .find(|f| &f.id == next)
                .ok_or_else(|| Error::Missing { kind: "folder", id: next.clone() })?
                .parent
                .as_ref();
        }
    }
    Ok(())
}

fn folder(id: Option<&Id>, library: &Library) -> Result<()> {
    if let Some(id) = id
        && !library.folders.iter().any(|f| &f.id == id)
    {
        return Err(Error::Missing { kind: "folder", id: id.clone() });
    }
    Ok(())
}

pub(crate) fn part(part: &Part, library: &Library) -> Result<()> {
    metadata(&part.tags, &part.notes, part.quantity)?;
    id(&part.id)?;
    clean_name(&part.name)?;
    folder(part.folder.as_ref(), library)?;
    hash_name(&part.sha256)?;
    if part.drawing.contours.len() > openlaser_core::geometry::MAX_PLACED_CONTOURS
        || part.drawing.contours.is_empty()
        || part.drawing.contours.iter().any(|contour| {
            contour.curves.is_empty()
                || contour.curves.iter().any(|curve| !curve.is_valid())
                || !contour.is_continuous()
        })
    {
        return Err(Error::Invalid("the drawing is empty, invalid or discontinuous".into()));
    }
    Ok(())
}

pub(crate) fn recipe(recipe: &Recipe) -> Result<()> {
    id(&recipe.id)?;
    clean_name(&recipe.name)?;
    if !(1..=11).contains(&recipe.layer) {
        return Err(Error::Invalid("the layer must be 1 to 11".into()));
    }
    if !recipe.thickness_mm.is_finite() || recipe.thickness_mm < 0. {
        return Err(Error::Invalid("the thickness must be finite and nonnegative".into()));
    }
    for hash in [&recipe.source_sha256, &recipe.photo].into_iter().flatten() {
        hash_name(hash)?;
    }
    Ok(())
}

pub(crate) fn library_recipe(recipe: &Recipe, library: &Library) -> Result<()> {
    self::recipe(recipe)?;
    if let Some(film) = &recipe.film {
        if film == &recipe.id {
            return Err(Error::Invalid("a recipe cannot use itself as its film process".into()));
        }
        if library.recipe(film)?.laser != recipe.laser {
            return Err(Error::Invalid("the film process is for the other laser".into()));
        }
    }
    Ok(())
}

pub(crate) fn job(job: &Job, library: &Library) -> Result<()> {
    metadata(&job.tags, &job.notes, job.quantity)?;
    job.preflight.validate()?;
    id(&job.id)?;
    clean_name(&job.name)?;
    folder(job.folder.as_ref(), library)?;
    let part = library.part(&job.part)?;
    job_layout(job, part)?;
    job_process(job)
}

fn job_layout(job: &Job, part: &Part) -> Result<()> {
    if let Some(placement) = &job.placement {
        placement.validate().map_err(Error::Invalid)?;
    }
    if let Some(sheet) = &job.sheet {
        id(&sheet.set)?;
        if sheet.number == 0 || sheet.number > sheet.total || sheet.total > 500 {
            return Err(Error::Invalid("invalid sheet numbering".into()));
        }
    }
    if let Some(profile) = &job.correction {
        profile.validate().map_err(|e| Error::Invalid(e.to_string()))?;
    }
    if job.calibration && job.correction.is_some() {
        return Err(Error::Invalid("calibration jobs cannot have correction".into()));
    }
    if let Some(nesting) = &job.nesting {
        nesting.validate(part.drawing.contours.len()).map_err(Error::Invalid)?;
    }
    if !job.placed.is_empty() {
        openlaser_core::geometry::Placed::validate_all(&job.placed, part.drawing.contours.len())
            .map_err(Error::Invalid)?;
    }
    let count = if job.placed.is_empty() { part.drawing.contours.len() } else { job.placed.len() };
    job.grouping.validate(count).map_err(Error::Invalid)?;
    if job.zero.is_some_and(|zero| !zero.iter().all(|value| value.is_finite())) {
        return Err(Error::Invalid("the job origin is not finite".into()));
    }
    Ok(())
}

fn job_process(job: &Job) -> Result<()> {
    recipe(&job.recipe)?;
    if let Some(film) = &job.film {
        recipe(film)?;
        if film.laser != job.recipe.laser {
            return Err(Error::Invalid("the film process is for the other laser".into()));
        }
    }
    Ok(())
}

fn metadata(tags: &[String], notes: &str, quantity: u32) -> Result<()> {
    if tags.len() > 40
        || tags
            .iter()
            .any(|t| t.trim().is_empty() || t.len() > 80 || t.chars().any(char::is_control))
        || notes.len() > 10000
        || notes.contains('\0')
        || !(1..=10000).contains(&quantity)
    {
        return Err(Error::Invalid(
            "use up to 40 short tags, 10000 bytes of notes, and a quantity from 1 to 10000".into(),
        ));
    }
    Ok(())
}
