// SPDX-License-Identifier: GPL-3.0-or-later

//! Revision-bound background nesting. Only Apply changes an authoring draft.

use crate::coordinator::{Coordinator, Shared};
use crate::document::Preview;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::features::{CoolingPlacement, JointPlacement, OrderStrategy, Spot};
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Placed, Point, Transform};
use openlaser_core::nesting::{NestSettings, NestStock, Nesting};
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
    /// Fresh stock instead of the original remnant.
    pub fresh: bool,
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
        let settings = draft.nesting.as_ref().map_or_else(NestSettings::default, |n| n.settings);
        let remove =
            if let StockChoice::Outline { contour } = &choice { Some(*contour) } else { None };
        let reference = match choice {
            StockChoice::Remnant { id } => {
                let sheet = self.sheet_store.view(&id)?;
                if draft.recipe.as_ref().is_some_and(|r| {
                    r.laser != sheet.mode
                        || (r.thickness_mm - sheet.thickness_mm).abs() > 1e-6
                        || !r.name.eq_ignore_ascii_case(&sheet.material)
                }) {
                    return Err(Error::Refused(
                        "choose a recipe matching this remnant's laser, material and thickness"
                            .into(),
                    ));
                }
                let zero = draft.zero.unwrap_or([0., 0.]);
                let at = self
                    .extent()
                    .map_or(Point::ORIGIN, |e| Point::new(e[0][0] - zero[0], e[1][0] - zero[1]));
                Some(self.sheet_store.stock(&id, at)?)
            }
            StockChoice::Clear => None,
            StockChoice::Outline { contour } => {
                let placed = draft
                    .placed
                    .get(contour)
                    .copied()
                    .ok_or_else(|| Error::Request("select a closed stock outline".into()))?;
                if draft.placed.len() <= 1 {
                    return Err(Error::Request(
                        "stock needs at least one other contour to nest".into(),
                    ));
                }
                let shape = placed.transform.contour(&drawing.contours[placed.source]);
                openlaser_nest::polygon(&shape).map_err(|e| Error::Request(e.to_string()))?;
                Some(NestStock::Outline { contour: placed })
            }
            StockChoice::Rectangle { width, height } => {
                let min = self.extent().map_or_else(
                    || {
                        draft
                            .nesting
                            .as_ref()
                            .and_then(|n| stock(&drawing, n).ok())
                            .and_then(|c| c.bounds())
                            .or_else(|| draft.preview.as_ref().and_then(|p| p.bounds))
                            .map_or(Point::ORIGIN, |b| b.min)
                    },
                    |extent| {
                        Point::new(
                            extent[0][0] - draft.zero.unwrap_or([0., 0.])[0],
                            extent[1][0] - draft.zero.unwrap_or([0., 0.])[1],
                        )
                    },
                );
                Some(NestStock::Rectangle {
                    bounds: Bounds { min, max: Point::new(min.x + width, min.y + height) },
                })
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
        draft.nesting = nesting;
        // Clearing an outline stock can leave its part with nothing on the sheet.
        draft.prune_parts();
        self.reprepare();
        Ok(())
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
    let input = input(draft.drawing()?, &draft, &request)?;
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
    };
    let work =
        Arc::new(Mutex::new(Work { previews: Vec::new(), view: view.clone(), candidate: None }));
    let cancel = Arc::new(AtomicBool::new(false));
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
            search_sheets(&draft, &input, &cancel, request.settings, progress, live)
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
            Err(e) => w.view.error = Some(e.to_string()),
        }
    });
    Ok(view)
}

fn search_sheets(
    draft: &Draft,
    input: &openlaser_nest::Request,
    cancel: &AtomicBool,
    settings: NestSettings,
    progress: impl Fn(usize, usize) + Sync,
    live: impl Fn(NestLive) + Sync,
) -> Result<(Draft, Vec<NestSheetSummary>, Vec<NestSheetPreview>)> {
    let drawing = draft.drawing()?;
    let overflow =
        if draft.nesting.as_ref().is_some_and(|n| matches!(n.stock, NestStock::Remnant { .. })) {
            let bounds =
                input.stock.bounds().ok_or_else(|| Error::Request("empty stock".into()))?;
            Some(stock(drawing, &Nesting { stock: NestStock::Rectangle { bounds }, settings })?)
        } else {
            None
        };
    let outline = |c: &Contour| openlaser_nest::polygon(c).unwrap_or_default();
    let first = (outline(&input.stock), input.cutouts.iter().map(outline).collect::<Vec<_>>());
    let fresh = overflow.as_ref().map(|c| (outline(c), Vec::new()));
    let show = |sheets: &[openlaser_nest::SheetSolution], changed: usize| {
        let Some(shown) = sheets.get(changed) else { return };
        let (stock_outline, stock_cutouts) =
            if changed > 0 { fresh.as_ref().unwrap_or(&first) } else { &first };
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
    let solutions = openlaser_nest::nest_sheets(input, overflow.as_ref(), cancel, progress, show)
        .map_err(|e| Error::Refused(e.to_string()))?;
    let mut drafts = Vec::new();
    let mut sheets = Vec::new();
    let mut previews = Vec::new();
    for (i, solution) in solutions.iter().enumerate() {
        let mut page = draft.clone();
        if !solution.original
            && let Some(outline) = &overflow
            && let Some(nesting) = &mut page.nesting
        {
            nesting.stock = NestStock::Rectangle {
                bounds: outline.bounds().ok_or_else(|| Error::Request("empty stock".into()))?,
            };
        }
        let page = candidate(page, drawing, &draft.groups, &solution.layout, settings)?;
        sheets.push(NestSheetSummary {
            number: i + 1,
            parts: solution.layout.placements.len(),
            coverage: solution.layout.coverage,
            fresh: !solution.original && overflow.is_some(),
        });
        previews.push(NestSheetPreview {
            preview: page.preview.clone(),
            stock_outline: page.stock_outline.clone(),
            stock_cutouts: page.stock_cutouts.clone(),
        });
        drafts.push(page);
    }
    let combined = crate::sheets::attach(draft, &drafts)?;
    Ok::<_, Error>((combined, sheets, previews))
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

fn input(
    drawing: &Drawing,
    draft: &Draft,
    request: &NestRequest,
) -> Result<openlaser_nest::Request> {
    request.settings.validate().map_err(Error::Request)?;
    if !(1..=500).contains(&request.quantity) || !(1..=30).contains(&request.seconds) {
        return Err(Error::Request("use 1–500 copies and a search time of 1–30 seconds".into()));
    }
    if draft.preparing {
        return Err(Error::Refused("wait for drawing preparation".into()));
    }
    if draft.features.common.is_some()
        || draft.features.bridges.as_ref().is_some_and(|b| !b.connections.is_empty())
    {
        return Err(Error::Refused(
            "remove common edges and bridge connections before changing their layout".into(),
        ));
    }
    let nesting = draft
        .nesting
        .as_ref()
        .ok_or_else(|| Error::Request("choose a stock rectangle or outline first".into()))?;
    let groups = draft.groups.clone();
    if groups.is_empty() || request.contours.iter().any(|&i| i >= draft.placed.len()) {
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
                    let p = draft.placed[n];
                    p.transform.contour(&drawing.contours[p.source])
                })
                .collect(),
            quantity: if selected == [i] { request.quantity as usize } else { 1 },
        })
        .collect();
    let lead = draft.features.leads.as_ref().map_or(0., |l| {
        [l.entry, l.exit]
            .into_iter()
            .chain(l.overrides.iter().flat_map(|v| [l.entry.and(v.entry), l.exit.and(v.exit)]))
            .flatten()
            .map(|x| x.length.0 + 2. * x.radius.0)
            .fold(0., f64::max)
    });
    let kerf = draft.features.kerf.as_ref().map_or(0., |k| k.width.0 / 2.);
    Ok(openlaser_nest::Request {
        cutouts: cutouts(nesting).to_vec(),
        stock: stock(drawing, nesting)?,
        rectangular: matches!(nesting.stock, NestStock::Rectangle { .. }),
        items,
        settings: NestSettings {
            margin: request.settings.margin.max(match &nesting.stock {
                NestStock::Remnant { clearance, .. } => *clearance,
                _ => 0.,
            }) + if matches!(nesting.stock, NestStock::Remnant { .. }) {
                request.settings.remnant_clearance
            } else {
                0.
            },
            ..request.settings
        },
        machining_clearance: lead + kerf,
        sheet_limit: openlaser_nest::SheetLimit {
            contours: openlaser_prep::MAX_CONTOURS,
            curves: 100_000,
        },
        time_limit: Duration::from_secs(u64::from(request.seconds)),
        seed: 7,
    })
}

fn candidate(
    mut draft: Draft,
    drawing: &Drawing,
    groups: &[Vec<usize>],
    solution: &openlaser_nest::Solution,
    settings: NestSettings,
) -> Result<Draft> {
    let original = draft.placed.clone();
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
    draft.placed = next;
    if !draft.grouping.0.is_empty() {
        draft.grouping = openlaser_core::grouping::Grouping(next_groups);
    }
    let repeat = |spots: &[Spot]| -> Vec<Spot> {
        spots
            .iter()
            .flat_map(|s| {
                map[s.contour].iter().map(|&contour| Spot { contour, fraction: s.fraction })
            })
            .collect()
    };
    if let Some(j) = &mut draft.features.joints
        && let JointPlacement::Manual(spots) = &mut j.placement
    {
        *spots = repeat(spots);
    }
    if let Some(c) = &mut draft.features.cooling
        && let CoolingPlacement::Manual(spots) = &mut c.placement
    {
        *spots = repeat(spots);
    }
    draft.features.start.spots = repeat(&draft.features.start.spots);
    if let Some(leads) = &mut draft.features.leads {
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
    if let OrderStrategy::Manual(order) = &mut draft.features.order.strategy {
        *order = (0..draft.placed.len()).collect();
    }
    if let Some(n) = &mut draft.nesting {
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
    let Some(nesting) = &draft.nesting else {
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
