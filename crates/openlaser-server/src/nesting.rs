// SPDX-License-Identifier: GPL-3.0-or-later

//! Revision-bound background nesting. Only Apply changes an authoring draft.

use crate::coordinator::{Coordinator, Shared};
use crate::document::Preview;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::features::{CoolingPlacement, JointPlacement, OrderStrategy, Spot};
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Placed, Point, Transform};
use openlaser_core::nesting::{NestSettings, NestStock, Nesting};
use openlaser_library::Id;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

/// A stock choice is a separate, undoable authoring edit.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StockChoice {
    /// Choose an inspected remnant with all its existing cutouts.
    Remnant {
        /// Sheet library identity.
        id: String,
    },
    /// A rectangular sheet at the bed minimum, or current stock if no bed is known.
    Rectangle {
        /// Width in millimetres.
        width: f64,
        /// Height in millimetres.
        height: f64,
    },
    /// Remove just this closed contour from cutting and retain it as stock.
    Outline {
        /// Placed contour index.
        contour: usize,
    },
    /// Remove the stock reference; undo can restore it.
    Clear,
}

/// Simple operator choices for one search.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
pub struct NestRequest {
    /// Selected contour identities; quantity requires one complete group.
    pub contours: Vec<usize>,
    /// Required copies of the selected part, including the original.
    pub quantity: u32,
    /// Distances and rotation constraint.
    pub settings: NestSettings,
    /// Bounded search duration, between one and thirty seconds.
    pub seconds: u32,
    /// The sheets to fill, in order; empty uses the draft's own sheet,
    /// once. More sheets are never opened unasked.
    #[serde(default)]
    pub stock: Vec<StockSource>,
}

/// One kind of sheet a search may fill. Sheets fill in the order listed,
/// so remnants usually come first.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StockSource {
    /// Rectangular sheets of one size.
    Sheet {
        /// Width in millimetres.
        width: f64,
        /// Height in millimetres.
        height: f64,
        /// At most this many.
        count: u32,
    },
    /// Sheets from the rack, at most as many as are on hand.
    Stock {
        /// Inventory entry.
        id: Id,
        /// Sheets to use at most.
        count: u32,
    },
    /// One inspected remnant with its existing cutouts.
    Remnant {
        /// Sheet library identity.
        id: String,
    },
}

/// Most sources one search may list.
const MAX_SOURCES: usize = 32;

/// A resolved source: stock placed at the bed minimum and how many sheets
/// of it may be opened.
#[derive(Clone)]
struct Source {
    stock: NestStock,
    count: usize,
}

/// Compact progress plus a complete preview once a search succeeds.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct NestView {
    /// Numbered sheets in the result; geometry is fetched one page at a time.
    pub sheets: Vec<NestSheetSummary>,
    /// Outer boundary of the first preview sheet.
    pub stock_outline: Vec<[f64; 2]>,
    /// Existing cutouts on the first preview sheet.
    pub stock_cutouts: Vec<Vec<[f64; 2]>>,
    /// Search identity, used by Cancel and Apply.
    pub id: u64,
    /// Draft revision this result belongs to.
    pub revision: u64,
    /// Whether the background worker is still searching.
    pub running: bool,
    /// Most parts placed in any attempt.
    pub placed: usize,
    /// Required physical part count.
    pub total: usize,
    /// Area fraction of the entire stock, with part holes reserved.
    pub coverage: Option<f64>,
    /// Prepared paths for a complete result, never an intermediate overlap.
    pub preview: Option<Arc<Preview>>,
    /// Changes whenever the running search shows another arrangement.
    pub live_serial: u64,
    /// The arrangement as the running search stands, for watching; left out
    /// of a status when the caller already has `live_serial`.
    pub live: Option<Arc<NestLive>>,
    /// Error or unsuccessful-search explanation.
    pub error: Option<String>,
    /// The chosen sheets filled up with parts still to place: `placed` of
    /// `total` fit, and the operator chooses the next sheet.
    pub needs_sheets: bool,
}

/// Where a running search has the draft's groups on the sheet it is filling
/// or compacting: each copy clear of the others, not yet checked against the
/// stock's own curves, and never applied.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct NestLive {
    /// One-based number of the sheet shown: the one the search last changed.
    pub sheet: usize,
    /// The shown sheet's outer boundary.
    pub stock_outline: Vec<[f64; 2]>,
    /// Material already removed from the shown sheet.
    pub stock_cutouts: Vec<Vec<[f64; 2]>>,
    /// Every copy on the shown sheet.
    pub copies: Vec<NestCopy>,
}

/// One copy of one of the draft's groups in a running search.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Serialize)]
pub struct NestCopy {
    /// The group, as the draft lists its groups.
    pub group: usize,
    /// Moves the group from where it lies now to where the copy goes.
    pub transform: Transform,
}

/// A compact summary in a nesting preview's sheet switcher.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct NestSheetSummary {
    /// One-based sheet number.
    pub number: usize,
    /// Physical copies on this sheet.
    pub parts: usize,
    /// Area fraction used on this sheet.
    pub coverage: f64,
    /// Which of the requested stock sources it is; 0, the draft's own
    /// sheet, when none were requested.
    pub source: usize,
    /// Outer width and height of the sheet in millimetres.
    pub size: [f64; 2],
}

/// Preview geometry for exactly one sheet.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct NestSheetPreview {
    /// Prepared cutting geometry.
    pub preview: Option<Arc<Preview>>,
    /// Outer reference boundary.
    pub stock_outline: Vec<[f64; 2]>,
    /// Previously removed regions.
    pub stock_cutouts: Vec<Vec<[f64; 2]>>,
}

struct Work {
    previews: Vec<NestSheetPreview>,
    view: NestView,
    candidate: Option<Draft>,
}

pub(crate) struct Task {
    id: u64,
    revision: u64,
    cancel: Arc<AtomicBool>,
    work: Arc<Mutex<Work>>,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn lock(work: &Mutex<Work>) -> std::sync::MutexGuard<'_, Work> {
    work.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Resolve stock geometry from the retained source, never from prepared cuts.
pub fn stock(drawing: &Drawing, nesting: &Nesting) -> Result<Contour> {
    nesting.validate(drawing.contours.len()).map_err(Error::Request)?;
    match &nesting.stock {
        NestStock::Rectangle { bounds } => {
            let a = bounds.min;
            let b = Point::new(bounds.max.x, a.y);
            let c = bounds.max;
            let d = Point::new(a.x, c.y);
            Ok(Contour {
                layer: "Stock".into(),
                curves: [(a, b), (b, c), (c, d), (d, a)]
                    .into_iter()
                    .map(|(start, end)| Curve::Line { start, end })
                    .collect(),
            })
        }
        NestStock::Remnant { outline, .. } => Ok(outline.clone()),
        NestStock::Outline { contour } => {
            let shape = contour.transform.contour(&drawing.contours[contour.source]);
            openlaser_nest::polygon(&shape).map_err(|e| Error::Request(e.to_string()))?;
            Ok(shape)
        }
    }
}

/// Unavailable regions retained with the stock reference.
pub(crate) fn cutouts(nesting: &Nesting) -> &[Contour] {
    match &nesting.stock {
        NestStock::Remnant { cutouts, .. } => cutouts,
        _ => &[],
    }
}

impl Coordinator {
    /// Set stock without adding it to the prepared cutting geometry.
    pub fn set_stock(&mut self, choice: StockChoice) -> Result<()> {
        let draft =
            self.draft.as_ref().ok_or_else(|| Error::Refused("open a drawing first".into()))?;
        let drawing = draft.drawing()?.clone();
        let settings =
            draft.current.nesting.as_ref().map_or_else(NestSettings::default, |n| n.settings);
        let remove =
            if let StockChoice::Outline { contour } = &choice { Some(*contour) } else { None };
        let reference =
            match choice {
                StockChoice::Remnant { id } => Some(self.remnant(draft, &id)?),
                StockChoice::Clear => None,
                StockChoice::Outline { contour } => {
                    let placed =
                        draft.current.placed.get(contour).copied().ok_or_else(|| {
                            Error::Request("select a closed sheet outline".into())
                        })?;
                    if draft.current.placed.len() <= 1 {
                        return Err(Error::Request(
                            "the sheet outline needs at least one other contour to nest".into(),
                        ));
                    }
                    let shape = placed.transform.contour(&drawing.contours[placed.source]);
                    openlaser_nest::polygon(&shape).map_err(|e| Error::Request(e.to_string()))?;
                    Some(NestStock::Outline { contour: placed })
                }
                StockChoice::Rectangle { width, height } => {
                    Some(self.rectangle(draft, &drawing, width, height))
                }
            };
        let nesting = reference.map(|stock| Nesting { stock, settings });
        if let Some(nesting) = &nesting {
            nesting.validate(drawing.contours.len()).map_err(Error::Request)?;
        }
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a drawing first".into()))?;
        if let Some(contour) = remove {
            draft.remove(&drawing, &[contour])?;
        } else {
            draft.remember();
        }
        draft.current.nesting = nesting;
        // Clearing an outline stock can leave its part with nothing on the sheet.
        draft.prune_parts();
        self.reprepare();
        Ok(())
    }

    /// An inspected remnant at the bed minimum, when it suits the recipe.
    fn remnant(&self, draft: &Draft, id: &str) -> Result<NestStock> {
        let sheet = self.sheet_store.view(id)?;
        if draft.current.recipe.as_ref().is_some_and(|r| {
            !crate::inventory::suits(
                sheet.mode,
                &sheet.material,
                sheet.thickness_mm,
                r.laser,
                &r.name,
                r.thickness_mm,
            )
        }) {
            return Err(Error::Refused(
                "choose a recipe matching this remnant's laser, material and thickness".into(),
            ));
        }
        let zero = draft.current.sheet_offset.unwrap_or([0., 0.]);
        let at = self
            .extent()
            .map_or(Point::ORIGIN, |e| Point::new(e[0][0] - zero[0], e[1][0] - zero[1]));
        self.sheet_store.stock(id, at)
    }

    /// A rectangular sheet at the bed minimum, or where the current stock
    /// or drawing lies when no bed is known.
    fn rectangle(&self, draft: &Draft, drawing: &Drawing, width: f64, height: f64) -> NestStock {
        let zero = draft.current.sheet_offset.unwrap_or([0., 0.]);
        let min = self.extent().map_or_else(
            || {
                draft
                    .current
                    .nesting
                    .as_ref()
                    .and_then(|n| stock(drawing, n).ok())
                    .and_then(|c| c.bounds())
                    .or_else(|| draft.preview.as_ref().and_then(|p| p.bounds))
                    .map_or(Point::ORIGIN, |b| b.min)
            },
            |e| Point::new(e[0][0] - zero[0], e[1][0] - zero[1]),
        );
        NestStock::Rectangle {
            bounds: Bounds { min, max: Point::new(min.x + width, min.y + height) },
        }
    }

    /// The sheets a search fills, in order.
    fn sources(&self, draft: &Draft, request: &NestRequest) -> Result<Vec<Source>> {
        let drawing = draft.drawing()?;
        if request.stock.is_empty() {
            let nesting = draft
                .current
                .nesting
                .as_ref()
                .ok_or_else(|| Error::Request("choose the sheets to nest on first".into()))?;
            return Ok(vec![Source { stock: nesting.stock.clone(), count: 1 }]);
        }
        if request.stock.len() > MAX_SOURCES {
            return Err(Error::Request(format!("choose at most {MAX_SOURCES} kinds of sheet")));
        }
        let recipe = draft.current.recipe.as_ref();
        let mut remnants = Vec::new();
        request
            .stock
            .iter()
            .map(|source| match source {
                StockSource::Sheet { width, height, count } => {
                    if *count == 0 {
                        return Err(Error::Request("use at least one sheet of each size".into()));
                    }
                    Ok(Source {
                        stock: self.rectangle(draft, drawing, *width, *height),
                        count: *count as usize,
                    })
                }
                StockSource::Stock { id, count } => {
                    let item = self.inventory.iter().find(|i| &i.id == id).ok_or_else(|| {
                        Error::Missing("those sheets are no longer on the rack".into())
                    })?;
                    if recipe.is_some_and(|r| !item.suits(r.laser, &r.name, r.thickness_mm)) {
                        return Err(Error::Refused(
                            "choose sheets matching the recipe's laser, material and thickness"
                                .into(),
                        ));
                    }
                    // Listed twice, the sheets still come from the one pile.
                    let listed: u32 = request
                        .stock
                        .iter()
                        .filter_map(|s| match s {
                            StockSource::Stock { id: other, count } if other == id => Some(*count),
                            _ => None,
                        })
                        .sum();
                    if *count == 0 || listed > item.quantity {
                        return Err(Error::Request(format!(
                            "use 1–{} of the {} × {} mm sheets on hand",
                            item.quantity, item.width_mm, item.height_mm
                        )));
                    }
                    Ok(Source {
                        stock: self.rectangle(draft, drawing, item.width_mm, item.height_mm),
                        count: *count as usize,
                    })
                }
                StockSource::Remnant { id } => {
                    if remnants.contains(&id) {
                        return Err(Error::Request("list each remnant once".into()));
                    }
                    remnants.push(id);
                    Ok(Source { stock: self.remnant(draft, id)?, count: 1 })
                }
            })
            .collect()
    }
}

/// Begin one bounded background search over an immutable draft snapshot.
pub async fn start(shared: &Shared, revision: u64, request: NestRequest) -> Result<NestView> {
    let mut c = shared.lock().await;
    c.check_draft(revision)?;
    if c.nesting_task.as_ref().is_some_and(|t| lock(&t.work).view.running) {
        return Err(Error::Refused(
            "wait for the current nesting search to finish or cancel".into(),
        ));
    }
    let draft = c.draft.clone().ok_or_else(|| Error::Refused("open a drawing first".into()))?;
    let sources = c.sources(&draft, &request)?;
    let (input, sheets) = input(draft.drawing()?, &draft, &request, &sources)?;
    let id = NEXT_TASK.fetch_add(1, Ordering::Relaxed);
    let total = input.items.iter().map(|i| i.quantity).sum();
    let view = NestView {
        sheets: Vec::new(),
        stock_outline: Vec::new(),
        stock_cutouts: Vec::new(),
        id,
        revision,
        running: true,
        placed: 0,
        total,
        coverage: None,
        preview: None,
        live_serial: 0,
        live: None,
        error: None,
        needs_sheets: false,
    };
    let work =
        Arc::new(Mutex::new(Work { previews: Vec::new(), view: view.clone(), candidate: None }));
    let cancel = Arc::new(AtomicBool::new(false));
    let full = Arc::new(AtomicBool::new(false));
    c.nesting_task = Some(Task { id, revision, cancel: cancel.clone(), work: work.clone() });
    drop(c);
    tokio::task::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let progress = |placed, total| {
                let mut work = lock(&work);
                work.view.placed = placed;
                work.view.total = total;
            };
            let live = |live| {
                let mut work = lock(&work);
                work.view.live_serial += 1;
                work.view.live = Some(Arc::new(live));
            };
            search_sheets(&draft, &input, &sheets, &sources, &cancel, &full, progress, live)
        }))
        .unwrap_or_else(|_| {
            Err(Error::Refused(
                "the nesting worker stopped unexpectedly; the drawing was kept".into(),
            ))
        });
        let mut w = lock(&work);
        w.view.running = false;
        w.view.live = None;
        match result {
            Ok((draft, sheets, previews)) if !cancel.load(Ordering::Relaxed) => {
                w.view.coverage = sheets.first().map(|s| s.coverage);
                if let Some(first) = previews.first() {
                    w.view.preview.clone_from(&first.preview);
                    w.view.stock_outline.clone_from(&first.stock_outline);
                    w.view.stock_cutouts.clone_from(&first.stock_cutouts);
                }
                w.view.sheets = sheets;
                w.previews = previews;
                w.candidate = Some(draft);
            }
            Ok(_) => w.view.error = Some("nesting cancelled".into()),
            Err(e) => {
                w.view.error = Some(e.to_string());
                w.view.needs_sheets = full.load(Ordering::Relaxed);
            }
        }
    });
    Ok(view)
}

#[allow(clippy::too_many_arguments, reason = "one search's inputs and callbacks")]
fn search_sheets(
    draft: &Draft,
    input: &openlaser_nest::Request,
    sheets: &[openlaser_nest::Sheet],
    sources: &[Source],
    cancel: &AtomicBool,
    full: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
    live: impl Fn(NestLive) + Sync,
) -> Result<(Draft, Vec<NestSheetSummary>, Vec<NestSheetPreview>)> {
    let drawing = draft.drawing()?;
    let settings = input.settings;
    let outline = |c: &Contour| openlaser_nest::polygon(c).unwrap_or_default();
    let shapes: Vec<_> = sheets
        .iter()
        .map(|s| (outline(&s.stock), s.cutouts.iter().map(outline).collect::<Vec<_>>()))
        .collect();
    let show = |solutions: &[openlaser_nest::SheetSolution], changed: usize| {
        let Some(shown) = solutions.get(changed) else { return };
        let (stock_outline, stock_cutouts) = &shapes[shown.sheet];
        live(NestLive {
            sheet: changed + 1,
            stock_outline: stock_outline.clone(),
            stock_cutouts: stock_cutouts.clone(),
            copies: shown
                .layout
                .placements
                .iter()
                .map(|p| NestCopy { group: p.item, transform: p.transform })
                .collect(),
        });
    };
    let solutions =
        openlaser_nest::nest_sheets(input, sheets, cancel, progress, show).map_err(|e| {
            full.store(e.sheets_full, Ordering::Relaxed);
            Error::Refused(e.to_string())
        })?;
    let mut drafts = Vec::new();
    let mut summaries = Vec::new();
    let mut previews = Vec::new();
    for (i, solution) in solutions.iter().enumerate() {
        let mut page = draft.clone();
        let stock = sources[solution.sheet].stock.clone();
        page.current.nesting = Some(Nesting { stock, settings });
        let page = candidate(page, drawing, &draft.groups, &solution.layout, settings)?;
        let bounds = sheets[solution.sheet]
            .stock
            .bounds()
            .ok_or_else(|| Error::Request("empty sheet".into()))?;
        summaries.push(NestSheetSummary {
            number: i + 1,
            parts: solution.layout.placements.len(),
            coverage: solution.layout.coverage,
            source: solution.sheet,
            size: [bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y],
        });
        previews.push(NestSheetPreview {
            preview: page.preview.clone(),
            stock_outline: page.stock_outline.clone(),
            stock_cutouts: page.stock_cutouts.clone(),
        });
        drafts.push(page);
    }
    let combined = crate::sheets::attach(draft, &drafts)?;
    Ok::<_, Error>((combined, summaries, previews))
}

/// Return a compact current search state.
pub async fn status(shared: &Shared, id: u64) -> Result<NestView> {
    let c = shared.lock().await;
    let task = task(&c, id)?;
    if c.document().draft_revision != task.revision {
        task.cancel.store(true, Ordering::Relaxed);
    }
    Ok(lock(&task.work).view.clone())
}

/// Return one preview page without sending other sheets' prepared paths.
pub async fn sheet_preview(shared: &Shared, id: u64, index: usize) -> Result<NestSheetPreview> {
    let c = shared.lock().await;
    let task = task(&c, id)?;
    c.check_draft(task.revision)?;
    lock(&task.work)
        .previews
        .get(index)
        .cloned()
        .ok_or_else(|| Error::Missing("that preview sheet does not exist".into()))
}

/// Cancel cooperatively; no partial placements can be applied.
pub async fn cancel(shared: &Shared, id: u64) -> Result<()> {
    let c = shared.lock().await;
    let task = task(&c, id)?;
    task.cancel.store(true, Ordering::Relaxed);
    lock(&task.work).candidate = None;
    Ok(())
}

/// Apply a complete result as one undo step, rejecting stale inputs.
pub async fn apply(shared: &Shared, id: u64) -> Result<()> {
    let mut c = shared.lock().await;
    let task = task(&c, id)?;
    c.check_draft(task.revision)?;
    if task.cancel.load(Ordering::Relaxed) {
        return Err(Error::Refused("nesting was cancelled".into()));
    }
    let candidate = lock(&task.work)
        .candidate
        .clone()
        .ok_or_else(|| Error::Refused("no complete nesting result to apply".into()))?;
    c.draft = Some(candidate);
    c.draft_changed();
    c.nesting_task = None;
    Ok(())
}

fn task(c: &Coordinator, id: u64) -> Result<&Task> {
    c.nesting_task
        .as_ref()
        .filter(|t| t.id == id)
        .ok_or_else(|| Error::Missing("that nesting search is no longer available".into()))
}

/// Most copies one nesting request may ask for; the nesting crate's own
/// limit.
pub(crate) const MAX_NEST_COPIES: u32 = 500;
/// Longest nesting search, in seconds, a request may ask for; the nesting
/// crate's own limit.
const MAX_NEST_SECONDS: u32 = 30;
/// The random seed of every nesting search, fixed so the same request
/// gives the same layout.
const NEST_SEED: u64 = 7;

/// The search request and the sheets it fills, one per source.
fn input(
    drawing: &Drawing,
    draft: &Draft,
    request: &NestRequest,
    sources: &[Source],
) -> Result<(openlaser_nest::Request, Vec<openlaser_nest::Sheet>)> {
    request.settings.validate().map_err(Error::Request)?;
    if !(1..=MAX_NEST_COPIES).contains(&request.quantity)
        || !(1..=MAX_NEST_SECONDS).contains(&request.seconds)
    {
        return Err(Error::Request(format!(
            "use 1–{MAX_NEST_COPIES} copies and a search time of 1–{MAX_NEST_SECONDS} seconds"
        )));
    }
    if draft.preparing {
        return Err(Error::Refused("wait for drawing preparation".into()));
    }
    if draft.current.features.common.is_some()
        || draft.current.features.bridges.as_ref().is_some_and(|b| !b.connections.is_empty())
    {
        return Err(Error::Refused(
            "remove common edges and bridge connections before changing their layout".into(),
        ));
    }
    let sheets = sources
        .iter()
        .map(|source| {
            let nesting = Nesting { stock: source.stock.clone(), settings: request.settings };
            Ok(openlaser_nest::Sheet {
                stock: stock(drawing, &nesting)?,
                cutouts: cutouts(&nesting).to_vec(),
                rectangular: matches!(nesting.stock, NestStock::Rectangle { .. }),
                margin: nesting.margin(),
                count: Some(source.count),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let first =
        sheets.first().ok_or_else(|| Error::Request("choose the sheets to nest on".into()))?;
    let groups = draft.groups.clone();
    if groups.is_empty() || request.contours.iter().any(|&i| i >= draft.current.placed.len()) {
        return Err(Error::Request("select a current part".into()));
    }
    let selected: Vec<_> = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.iter().any(|i| request.contours.contains(i)))
        .map(|(i, _)| i)
        .collect();
    if request.quantity > 1 && selected.len() != 1 {
        return Err(Error::Request("select one complete part before entering its quantity".into()));
    }
    let items = groups
        .iter()
        .enumerate()
        .map(|(i, g)| openlaser_nest::Item {
            contours: g
                .iter()
                .map(|&n| {
                    let p = draft.current.placed[n];
                    p.transform.contour(&drawing.contours[p.source])
                })
                .collect(),
            quantity: if selected == [i] { request.quantity as usize } else { 1 },
        })
        .collect();
    let lead = draft.current.features.leads.as_ref().map_or(0., |l| {
        [l.entry, l.exit]
            .into_iter()
            .chain(l.overrides.iter().flat_map(|v| [l.entry.and(v.entry), l.exit.and(v.exit)]))
            .flatten()
            .map(|x| x.length.0 + 2. * x.radius.0)
            .fold(0., f64::max)
    });
    let kerf = draft.current.features.kerf.as_ref().map_or(0., |k| k.width.0 / 2.);
    let request = openlaser_nest::Request {
        cutouts: first.cutouts.clone(),
        stock: first.stock.clone(),
        rectangular: first.rectangular,
        items,
        settings: request.settings,
        machining_clearance: lead + kerf,
        sheet_limit: openlaser_nest::SheetLimit {
            contours: openlaser_prep::MAX_CONTOURS,
            curves: openlaser_prep::MAX_CURVES,
        },
        time_limit: Duration::from_secs(u64::from(request.seconds)),
        seed: NEST_SEED,
    };
    Ok((request, sheets))
}

fn candidate(
    mut draft: Draft,
    drawing: &Drawing,
    groups: &[Vec<usize>],
    solution: &openlaser_nest::Solution,
    settings: NestSettings,
) -> Result<Draft> {
    let original = draft.current.placed.clone();
    let mut next = Vec::new();
    let mut next_groups = Vec::new();
    let mut map = vec![Vec::<usize>::new(); original.len()];
    for (copy, placement) in solution.placements.iter().enumerate() {
        let copy = u32::try_from(copy).map_err(|_| Error::Refused("too many copies".into()))?;
        let first = next.len();
        for &i in &groups[placement.item] {
            let p = original[i];
            map[i].push(next.len());
            next.push(Placed {
                source: p.source,
                copy,
                transform: placement.transform.after(&p.transform),
            });
        }
        next_groups.push((first..next.len()).collect());
    }
    if next.len() > openlaser_core::geometry::MAX_PLACED_CONTOURS {
        return Err(Error::Refused("the nested layout exceeds 100000 contours".into()));
    }
    draft.remember();
    draft.current.placed = next;
    if !draft.current.grouping.0.is_empty() {
        draft.current.grouping = openlaser_core::grouping::Grouping(next_groups);
    }
    let repeat = |spots: &[Spot]| -> Vec<Spot> {
        spots
            .iter()
            .flat_map(|s| {
                map[s.contour].iter().map(|&contour| Spot { contour, fraction: s.fraction })
            })
            .collect()
    };
    if let Some(j) = &mut draft.current.features.joints
        && let JointPlacement::Manual(spots) = &mut j.placement
    {
        *spots = repeat(spots);
    }
    if let Some(c) = &mut draft.current.features.cooling
        && let CoolingPlacement::Manual(spots) = &mut c.placement
    {
        *spots = repeat(spots);
    }
    draft.current.features.start.spots = repeat(&draft.current.features.start.spots);
    if let Some(leads) = &mut draft.current.features.leads {
        leads.overrides = leads
            .overrides
            .iter()
            .flat_map(|edited| {
                map[edited.location.contour].iter().map(|&contour| {
                    let mut copied = edited.clone();
                    copied.location.contour = contour;
                    copied
                })
            })
            .collect();
    }
    if let OrderStrategy::Manual(order) = &mut draft.current.features.order.strategy {
        *order = (0..draft.current.placed.len()).collect();
    }
    if let Some(n) = &mut draft.current.nesting {
        n.settings = settings;
    }
    draft.prepare(drawing);
    if let Some(error) = &draft.error {
        return Err(Error::Refused(format!("nested machining paths: {error}")));
    }
    Ok(draft)
}

/// Check exact prepared curves, including leads and film, against saved stock.
pub(crate) fn check_prepared(
    drawing: &Drawing,
    draft: &Draft,
    prepared: &crate::draft::Prepared,
) -> Result<()> {
    let Some(nesting) = &draft.current.nesting else {
        return Ok(());
    };
    let stock = stock(drawing, nesting)?;
    let paths: Vec<_> = prepared
        .contours
        .iter()
        .chain(prepared.film.iter().flatten())
        .map(|c| Contour {
            layer: String::new(),
            curves: c
                .segments
                .iter()
                .map(|s| match s.curve {
                    openlaser_compiler::cut::Segment::Line { start, end } => {
                        Curve::Line { start: start.into(), end: end.into() }
                    }
                    openlaser_compiler::cut::Segment::Arc {
                        center,
                        radius,
                        start_angle,
                        sweep,
                    } => Curve::Arc { center: center.into(), radius, start_angle, sweep },
                })
                .collect(),
        })
        .collect();
    openlaser_nest::check_region(&stock, cutouts(nesting), &paths, nesting.margin())
        .map_err(|e| Error::Refused(e.to_string()))
}
