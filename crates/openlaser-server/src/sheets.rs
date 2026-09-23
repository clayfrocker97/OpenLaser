// SPDX-License-Identifier: GPL-3.0-or-later

//! A retained multi-sheet draft and navigation between saved numbered jobs.

use crate::coordinator::Coordinator;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::features::Features;
use openlaser_core::geometry::Placed;
use openlaser_core::grouping::Grouping;
use openlaser_core::nesting::Nesting;
use openlaser_library::{Anchor, Id};
use serde::{Deserialize, Serialize};

/// Most sheets one job set may hold, and most parts one saved sheet may
/// list; the same bound as copies in one nesting search.
const MAX_SHEETS: usize = 500;

/// Geometry specific to a sheet; material and correction belong to the job set.
///
/// These are the sheet-specific fields of the draft's
/// [`Snapshot`](crate::draft::Snapshot), under the same stored names, plus
/// the sheet's part count for the switcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetLayout {
    /// See `Snapshot::placement`.
    #[serde(default)]
    placement: Option<openlaser_library::placement::Placement>,
    /// See `Snapshot::nesting`.
    nesting: Option<Nesting>,
    /// See `Snapshot::placed`.
    placed: Vec<Placed>,
    /// See `Snapshot::grouping`.
    grouping: Grouping,
    /// See `Snapshot::features`.
    features: Features,
    /// See `Snapshot::sheet_offset`.
    #[serde(rename = "zero")]
    sheet_offset: Option<[f64; 2]>,
    /// See `Snapshot::anchor`.
    anchor: Anchor,
    /// How many parts (contour groups) the sheet holds.
    parts: usize,
}

impl SheetLayout {
    /// The sheet the draft shows now.
    fn capture(draft: &Draft) -> Self {
        let current = &draft.current;
        Self {
            placement: current.placement.clone(),
            nesting: current.nesting.clone(),
            placed: current.placed.clone(),
            grouping: current.grouping.clone(),
            features: current.features.clone(),
            sheet_offset: current.sheet_offset,
            anchor: current.anchor,
            parts: draft.groups.len(),
        }
    }

    /// Shows this sheet in the draft, keeping the job-wide fields.
    fn load(&self, draft: &mut Draft) {
        let current = &mut draft.current;
        current.placement.clone_from(&self.placement);
        current.nesting.clone_from(&self.nesting);
        current.placed.clone_from(&self.placed);
        current.grouping.clone_from(&self.grouping);
        current.features.clone_from(&self.features);
        current.sheet_offset = self.sheet_offset;
        current.anchor = self.anchor;
        crate::placement::forget_capture(draft);
    }

    fn validate(&self, count: usize) -> Result<()> {
        if let Some(placement) = &self.placement {
            placement.validate().map_err(Error::Refused)?;
        }
        Placed::validate_all(&self.placed, count).map_err(Error::Refused)?;
        self.grouping.validate(self.placed.len()).map_err(Error::Refused)?;
        if let Some(nesting) = &self.nesting {
            nesting.validate(count).map_err(Error::Refused)?;
        }
        if self.parts > MAX_SHEETS
            || self.placed.is_empty()
            || self.sheet_offset.is_some_and(|p| p.iter().any(|v| !v.is_finite()))
        {
            return Err(Error::Refused("invalid saved sheet layout".into()));
        }
        Ok(())
    }
}

/// Unsaved sheet pages, persisted with the draft and its undo snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetSet {
    /// Zero-based selected page.
    pub active: usize,
    /// Complete individual layouts, updated from the active draft before switching.
    pub pages: Vec<SheetLayout>,
}

impl SheetSet {
    /// Bound retained data before it is reopened.
    pub fn validate(&self, count: usize) -> Result<()> {
        if !(2..=MAX_SHEETS).contains(&self.pages.len()) || self.active >= self.pages.len() {
            return Err(Error::Refused("invalid saved sheet set".into()));
        }
        for page in &self.pages {
            page.validate(count)?;
        }
        Ok(())
    }

    /// Every drawing contour a page places or takes its stock from.
    pub(crate) fn sources(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages.iter().flat_map(|page| {
            page.placed
                .iter()
                .map(|p| p.source)
                .chain(crate::draft::outline_source(page.nesting.as_ref()))
        })
    }

    /// Moves drawing contours on every page, as dropping a part does.
    pub(crate) fn remap_sources(&mut self, moved: impl Fn(&mut usize) + Copy) {
        for page in &mut self.pages {
            page.placed.iter_mut().for_each(|p| moved(&mut p.source));
            crate::draft::remap_outline(&mut page.nesting, moved);
        }
    }

    fn synchronized(&self, draft: &Draft) -> Self {
        let mut set = self.clone();
        set.pages[set.active] = SheetLayout::capture(draft);
        set
    }
}

/// One numbered entry in the sheet switcher.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetTab {
    /// One-based sheet number.
    pub number: usize,
    /// Physical part count, absent when opening a saved job without preparation.
    pub parts: Option<usize>,
    /// Saved job identity, absent until the set is saved.
    pub job: Option<Id>,
}

/// Small navigation data; only the active sheet sends prepared canvas geometry.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetNavigation {
    /// Zero-based selected entry.
    pub active: usize,
    /// Sheets in numbered order.
    pub pages: Vec<SheetTab>,
}

/// Attach complete candidate sheets while preserving other pages in the set.
pub(crate) fn attach(original: &Draft, candidates: &[Draft]) -> Result<Draft> {
    let mut result = candidates
        .first()
        .cloned()
        .ok_or_else(|| Error::Refused("nesting returned no sheets".into()))?;
    let replacements: Vec<_> = candidates.iter().map(SheetLayout::capture).collect();
    let mut set = original.current.sheets.as_ref().map_or_else(
        || SheetSet { active: 0, pages: vec![SheetLayout::capture(original)] },
        |set| set.synchronized(original),
    );
    set.pages.splice(set.active..=set.active, replacements);
    if set.pages.len() > MAX_SHEETS {
        return Err(Error::Refused(format!("a job set can contain at most {MAX_SHEETS} sheets")));
    }
    result.current.sheets = (set.pages.len() > 1).then_some(set);
    Ok(result)
}

impl Coordinator {
    pub(crate) fn sheet_navigation(&self, draft: &Draft) -> Option<SheetNavigation> {
        if let Some(set) = &draft.current.sheets {
            return Some(SheetNavigation {
                active: set.active,
                pages: set
                    .pages
                    .iter()
                    .enumerate()
                    .map(|(i, page)| SheetTab {
                        number: i + 1,
                        parts: Some(if i == set.active { draft.groups.len() } else { page.parts }),
                        job: None,
                    })
                    .collect(),
            });
        }
        let sheet = draft.saved_base.as_ref()?.sheet.as_ref()?;
        let mut jobs: Vec<_> = self
            .library
            .jobs()
            .filter(|j| j.sheet.as_ref().is_some_and(|s| s.set == sheet.set))
            .collect();
        jobs.sort_by_key(|j| j.sheet.as_ref().map(|s| s.number));
        let active = jobs.iter().position(|j| draft.job.as_ref() == Some(&j.id))?;
        Some(SheetNavigation {
            active,
            pages: jobs
                .iter()
                .map(|j| SheetTab {
                    number: j.sheet.as_ref().map_or(1, |s| s.number as usize),
                    parts: None,
                    job: Some(j.id.clone()),
                })
                .collect(),
        })
    }

    /// Switch unsaved layouts; each saved sheet instead opens its retained job.
    pub fn select_sheet(&mut self, index: usize) -> Result<()> {
        let draft = self.draft.as_mut().ok_or_else(|| Error::Refused("open a job first".into()))?;
        let set = draft
            .current
            .sheets
            .as_ref()
            .ok_or_else(|| Error::Refused("this job has no unsaved sheet set".into()))?;
        if index >= set.pages.len() {
            return Err(Error::Request("that sheet does not exist".into()));
        }
        let mut set = set.synchronized(draft);
        set.active = index;
        set.pages[index].load(draft);
        draft.current.sheets = Some(set);
        self.draft_generation += 1;
        self.reprepare();
        Ok(())
    }

    /// Save a batch together and retain the currently selected sheet.
    pub(crate) fn save_sheets(&mut self, name: &str) -> Result<crate::document::JobView> {
        let draft = self.draft.as_ref().ok_or_else(|| Error::Refused("open a job first".into()))?;
        let set = draft
            .current
            .sheets
            .as_ref()
            .ok_or_else(|| Error::Refused("no sheet set".into()))?
            .synchronized(draft);
        let old_key = crate::workspace::key(draft);
        let mut jobs = Vec::with_capacity(set.pages.len());
        for layout in &set.pages {
            let mut page = draft.clone();
            page.current.sheets = None;
            layout.load(&mut page);
            // Each sheet becomes a job of its own, cutting only its parts.
            page.prune_parts();
            jobs.push(page.job_value(name)?);
        }
        let (_, jobs) = self.library.add_sheet_set(name, jobs)?;
        let saved = &jobs[set.active];
        if let Some(draft) = &mut self.draft {
            draft.current.sheets = None;
            draft.adopt(saved);
        }
        self.attach_draft()?;
        self.library_changed();
        self.draft_changed();
        if old_key != format!("job-{}", saved.id) {
            self.draft_store.remove(&old_key);
        }
        Ok(crate::document::JobView::new(saved, &self.library.job_drawing(&saved.parts)?))
    }
}
