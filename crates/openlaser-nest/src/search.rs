// SPDX-License-Identifier: GPL-3.0-or-later

use crate::polygon::{GUARD, MAX_TOTAL_VERTICES, contains, polygon};
use crate::{Error, Placement, Request, SheetSolution, Solution};
use jagua_rs::entities::{Container, Item, Layout, PlacedItem};
use jagua_rs::geometry::DTransformation;
use jagua_rs::geometry::shape_modification::ShapeModifyConfig;
use jagua_rs::io::export::int_to_ext_transformation;
use jagua_rs::io::ext_repr::{ExtContainer, ExtItem, ExtQualityZone, ExtSPolygon, ExtShape};
use jagua_rs::io::import::{Importer, ext_to_int_transformation};
use jagua_rs::probs::spp::entities::{SPInstance, SPProblem, SPSolution, Strip};
use openlaser_core::geometry::{Point, Transform};
use openlaser_core::nesting::NestRotation;
use rand::rngs::Xoshiro256PlusPlus;
use rand::{SeedableRng, seq::SliceRandom};
use sparrow::config::DEFAULT_SPARROW_CONFIG;
use sparrow::eval::lbf_evaluator::LBFEvaluator;
use sparrow::eval::sample_eval::{SampleEval, SampleEvaluator};
use sparrow::sample::search::{SampleConfig, search_placement};
use sparrow::util::listener::DummySolListener;
use sparrow::util::listener::{ReportType, SolutionListener};
use sparrow::util::terminator::Terminator;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Most copies one search places, across every item and sheet.
const MAX_COPIES: usize = 500;
/// Longest search a request may ask for, and the separate bound on shape
/// preparation before it.
const MAX_SEARCH_TIME: Duration = Duration::from_secs(30);
/// Largest machining clearance, in millimetres, a request may add around
/// each part.
const MAX_MACHINING_CLEARANCE: f64 = 1000.;
/// Samples tried for each copy of a first fit across sheets. Sparrow's own
/// construction uses 1000 (`sparrow::consts::LBF_SAMPLE_CONFIG`); fewer keep
/// a many-sheet search within its time.
const SHEET_FIT_SAMPLES: SampleConfig =
    SampleConfig { n_container_samples: 350, n_focussed_samples: 0, n_coord_descents: 3 };
/// Samples tried for each copy of one sheet's randomized first fits, which
/// are repeated many times, so fewer again.
const REPEATED_FIT_SAMPLES: SampleConfig =
    SampleConfig { n_container_samples: 250, n_focussed_samples: 0, n_coord_descents: 3 };
/// Share of the remaining time Sparrow explores before compressing: its own
/// default split (`sparrow::consts::DEFAULT_EXPLORE_TIME_RATIO`, 0.8/0.2).
const EXPLORE_SHARE: f64 = 0.8;
/// Share of the remaining time Sparrow compresses; see [`EXPLORE_SHARE`].
const COMPRESS_SHARE: f64 = 0.2;
/// Sparrow separator workers, matching the search's two threads.
const SEPARATOR_WORKERS: usize = 2;

/// How often a running search shows where it stands.
pub const LIVE_INTERVAL: Duration = Duration::from_millis(200);

/// Shows a search's sheets as they stand and which one just changed, at
/// most every [`LIVE_INTERVAL`].
pub(crate) struct Live<'a> {
    show: &'a (dyn Fn(&[SheetSolution], usize) + Sync),
    last: Option<Instant>,
}

impl<'a> Live<'a> {
    pub(crate) fn new(show: &'a (dyn Fn(&[SheetSolution], usize) + Sync)) -> Self {
        Self { show, last: None }
    }

    /// Whether the last showing is old enough for another.
    fn due(&mut self) -> bool {
        let now = Instant::now();
        if self.last.is_some_and(|last| now - last < LIVE_INTERVAL) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

struct Budget<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
    phase: Instant,
}

impl Terminator for Budget<'_> {
    fn kill(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || Instant::now() >= self.phase.min(self.deadline)
    }
    fn new_timeout(&mut self, timeout: Duration) {
        self.phase = (Instant::now() + timeout).min(self.deadline);
    }
    fn timeout_at(&self) -> Option<jagua_rs::Instant> {
        Some(self.phase.min(self.deadline))
    }
}

struct Evaluation<'a> {
    inner: LBFEvaluator<'a>,
    budget: &'a Budget<'a>,
}

impl SampleEvaluator for Evaluation<'_> {
    fn evaluate_sample(&mut self, dt: DTransformation, upper: Option<SampleEval>) -> SampleEval {
        if self.budget.kill() { SampleEval::Invalid } else { self.inner.evaluate_sample(dt, upper) }
    }
    fn n_evals(&self) -> usize {
        self.inner.n_evals()
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "finite millimetre inputs are bounded to 100 m and padded for f32 conversion"
)]
fn ext(points: &[[f64; 2]], origin: Point) -> ExtShape {
    ExtShape::SimplePolygon(ExtSPolygon(
        points.iter().map(|p| ((p[0] - origin.x) as f32, (p[1] - origin.y) as f32)).collect(),
    ))
}

fn err(error: impl std::fmt::Display) -> Error {
    Error(error.to_string())
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "validated dimensions and at most 500 copies fit these conversions"
)]
pub(crate) fn run(
    request: &Request,
    cancel: &AtomicBool,
    progress: &(impl Fn(usize, usize) + Sync),
) -> Result<Solution, Error> {
    let total = validate(request)?;
    let whole = request
        .items
        .iter()
        .fold([0, 0], |sum, item| (0..item.quantity).fold(sum, |sum, _| plus(sum, load(item))));
    if !fits(&request.sheet_limit, whole) {
        return Err(Error(format!(
            "these copies hold {} contours and {} lines and arcs; one sheet can prepare {} and {}",
            whole[0], whole[1], request.sheet_limit.contours, request.sheet_limit.curves
        )));
    }
    check_cancelled(cancel)?;
    let geometry = geometry(request, cancel)?;
    // Shape preparation has its own bound; the requested duration is search time.
    let deadline = Instant::now() + request.time_limit;
    let mut budget = Budget { cancel, deadline, phase: deadline };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(request.seed);
    let mut progress = Progress { callback: progress, most_placed: 0, total };
    let best = initial_search(request, &geometry, &budget, &mut rng, &mut progress);
    check_cancelled(cancel)?;
    let mut best = best.ok_or_else(|| Error(format!("no complete layout found ({} of {total} parts placed); try fewer copies, a larger container, less spacing, or different rotations", progress.most_placed)))?;
    if request.rectangular && request.cutouts.is_empty() && !budget.kill() {
        let mut quiet = DummySolListener;
        best = compress(best, &geometry.items, &geometry.stock, &mut budget, &mut rng, &mut quiet)?;
    }
    check_cancelled(cancel)?;
    if !best.is_feasible() || best.placed_items.len() != total {
        return Err(Error("nesting result failed its collision check".into()));
    }
    solution(request, &best, &geometry.items, geometry.origin, &geometry.item_origins, true)
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) { Err(Error("nesting cancelled".into())) } else { Ok(()) }
}

/// Bounded first-fit placement across sheets, largest parts first. Every sheet
/// shares the same quantity accounting and the same cancellation/deadline.
pub(crate) fn run_sheets(
    request: &Request,
    overflow: Option<&openlaser_core::geometry::Contour>,
    cancel: &AtomicBool,
    progress: &(impl Fn(usize, usize) + Sync),
    live: &mut Live<'_>,
) -> Result<Vec<SheetSolution>, Error> {
    let total = validate(request)?;
    check_cancelled(cancel)?;
    let first = geometry(request, cancel)?;
    let blank_request = overflow_request(request, overflow);
    let blank = if overflow.is_some() { Some(geometry(&blank_request, cancel)?) } else { None };
    let next = blank.as_ref().unwrap_or(&first);
    let stocks = Stocks { first: &first, next, request, blank: &blank_request };
    let deadline = Instant::now() + request.time_limit;
    let mut budget = Budget { cancel, deadline, phase: deadline };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(request.seed);
    let order = largest_first(&first);
    let mut sheets =
        FirstFit { layouts: vec![Layout::new(first.stock.clone())], loads: vec![[0, 0]] };
    for (placed, id) in order.into_iter().enumerate() {
        check_cancelled(cancel)?;
        if budget.kill() {
            return Err(time_limit_reached(placed, total));
        }
        let fitted =
            sheets.place(&stocks, request, id, &budget, &mut rng).map_err(
                |unplaced| match unplaced {
                    Unplaced::OutOfTime => time_limit_reached(placed, total),
                    Unplaced::TooLarge => Error(format!(
                        "part {} could not fit on an empty sheet with this spacing and rotation",
                        id + 1
                    )),
                },
            )?;
        progress(placed + 1, total);
        if live.due() {
            (live.show)(&stocks.standing(&sheets.layouts), fitted);
        }
    }
    check_cancelled(cancel)?;
    let mut layouts = sheets.layouts;
    let last = layouts.len() - 1;
    let (geometry, input) = stocks.of(last);
    // Every sheet before the last stays as it is while the last compacts.
    let earlier = stocks.standing(&layouts[..last]);
    let mut compacting = Compacting { earlier, input, geometry, original: last == 0, live };
    compress_last(&mut layouts[last], geometry, input, &mut budget, &mut rng, &mut compacting);
    check_cancelled(cancel)?;
    stocks.finished(layouts)
}

/// The request for every sheet after the first: the overflow stock, a
/// plain rectangle without cutouts, when there is one; otherwise the same
/// stock again.
fn overflow_request(
    request: &Request,
    overflow: Option<&openlaser_core::geometry::Contour>,
) -> Request {
    let mut blank = request.clone();
    if let Some(stock) = overflow {
        blank.stock = stock.clone();
        blank.cutouts.clear();
        blank.rectangular = true;
    }
    blank
}

/// Every requested copy, as its item index, largest area first.
fn largest_first(geometry: &Geometry) -> Vec<usize> {
    let mut order: Vec<usize> = geometry
        .items
        .iter()
        .enumerate()
        .flat_map(|(i, (_, q))| std::iter::repeat_n(i, *q))
        .collect();
    order.sort_by(|a, b| {
        geometry.items[*b].0.shape_cd.area.total_cmp(&geometry.items[*a].0.shape_cd.area)
    });
    order
}

fn time_limit_reached(placed: usize, total: usize) -> Error {
    Error(format!("nesting time limit reached ({placed} of {total} parts); increase search time"))
}

/// Why a copy found no sheet.
enum Unplaced {
    /// The search ran out of time.
    OutOfTime,
    /// It does not fit even on an empty sheet.
    TooLarge,
}

/// The sheets of a first-fit placement so far.
struct FirstFit {
    layouts: Vec<Layout>,
    /// Each sheet's contours and curves, kept within what preparation accepts.
    loads: Vec<[usize; 2]>,
}

impl FirstFit {
    /// Places one copy of item `id` on the first sheet with room for it, or
    /// on a new sheet; the index of the sheet it went on.
    fn place(
        &mut self,
        stocks: &Stocks<'_>,
        request: &Request,
        id: usize,
        budget: &Budget<'_>,
        rng: &mut Xoshiro256PlusPlus,
    ) -> Result<usize, Unplaced> {
        let copy = load(&request.items[id]);
        for (index, layout) in self.layouts.iter_mut().enumerate() {
            if !fits(&request.sheet_limit, plus(self.loads[index], copy)) {
                continue;
            }
            let (geometry, _) = stocks.of(index);
            if place_one(layout, &geometry.items[id].0, budget, rng) {
                self.loads[index] = plus(self.loads[index], copy);
                return Ok(index);
            }
        }
        if budget.kill() {
            return Err(Unplaced::OutOfTime);
        }
        let mut layout = Layout::new(stocks.next.stock.clone());
        if !place_one(&mut layout, &stocks.next.items[id].0, budget, rng) {
            return Err(Unplaced::TooLarge);
        }
        self.layouts.push(layout);
        self.loads.push(copy);
        Ok(self.layouts.len() - 1)
    }
}

/// Each sheet's geometry and request: the original stock for the first
/// sheet, the overflow stock for the rest.
struct Stocks<'a> {
    first: &'a Geometry,
    next: &'a Geometry,
    request: &'a Request,
    blank: &'a Request,
}

impl Stocks<'_> {
    fn of(&self, index: usize) -> (&Geometry, &Request) {
        if index == 0 { (self.first, self.request) } else { (self.next, self.blank) }
    }

    /// Every sheet as it stands, for showing.
    fn standing(&self, layouts: &[Layout]) -> Vec<SheetSolution> {
        layouts
            .iter()
            .enumerate()
            .map(|(index, layout)| {
                let (geometry, input) = self.of(index);
                so_far(input, layout.placed_items.values(), geometry, index == 0)
            })
            .collect()
    }

    /// The finished sheets, each checked, with every requested copy placed.
    fn finished(&self, layouts: Vec<Layout>) -> Result<Vec<SheetSolution>, Error> {
        let mut counts = vec![0usize; self.request.items.len()];
        let mut sheets = Vec::new();
        for (index, layout) in layouts.into_iter().enumerate() {
            if layout.placed_items.is_empty() {
                continue;
            }
            if !layout.is_feasible() {
                return Err(Error("a sheet failed its collision check".into()));
            }
            let (geometry, input) = self.of(index);
            let layout = solution(
                input,
                &layout,
                &geometry.items,
                geometry.origin,
                &geometry.item_origins,
                false,
            )?;
            for placement in &layout.placements {
                counts[placement.item] += 1;
            }
            sheets.push(SheetSolution { original: index == 0, layout });
        }
        if counts.iter().zip(&self.request.items).any(|(count, item)| *count != item.quantity) {
            return Err(Error("multi-sheet nesting lost a requested copy".into()));
        }
        Ok(sheets)
    }
}

/// Every sheet before the last is full. Compress the last one towards its
/// start with the remaining time, so its offcut is left in one piece; a
/// failed compression keeps the first-fit layout.
fn compress_last(
    layout: &mut Layout,
    geometry: &Geometry,
    input: &Request,
    budget: &mut Budget<'_>,
    rng: &mut Xoshiro256PlusPlus,
    listener: &mut impl SolutionListener,
) {
    if !input.rectangular || !input.cutouts.is_empty() || budget.kill() {
        return;
    }
    let mut copies: Vec<(Item, usize)> =
        geometry.items.iter().map(|(item, _)| (item.clone(), 0)).collect();
    for placed in layout.placed_items.values() {
        copies[placed.item_id].1 += 1;
    }
    if let Ok(compressed) =
        compress(layout.clone(), &copies, &geometry.stock, budget, rng, listener)
    {
        *layout = compressed;
    }
}

/// Shows the last sheet each time compaction finds a tighter arrangement
/// without overlaps, beside the sheets already full.
struct Compacting<'a, 'b> {
    earlier: Vec<SheetSolution>,
    input: &'b Request,
    geometry: &'b Geometry,
    original: bool,
    live: &'b mut Live<'a>,
}

impl SolutionListener for Compacting<'_, '_> {
    fn report(&mut self, report: ReportType, solution: &SPSolution, _: &SPInstance) {
        if matches!(report, ReportType::ExplFeas | ReportType::CmprFeas) && self.live.due() {
            let placed = solution.layout_snapshot.placed_items.values();
            let mut sheets = self.earlier.clone();
            sheets.push(so_far(self.input, placed, self.geometry, self.original));
            (self.live.show)(&sheets, sheets.len() - 1);
        }
    }
}

fn place_one(
    layout: &mut Layout,
    item: &Item,
    budget: &Budget<'_>,
    rng: &mut Xoshiro256PlusPlus,
) -> bool {
    if budget.kill() {
        return false;
    }
    let evaluator = Evaluation { inner: LBFEvaluator::new(layout, item), budget };
    let (sample, _) = search_placement(layout, item, None, evaluator, SHEET_FIT_SAMPLES, rng);
    if let Some((dt, SampleEval::Clear { .. })) = sample {
        layout.place_item(item, dt);
        true
    } else {
        false
    }
}

struct Progress<'a> {
    callback: &'a (dyn Fn(usize, usize) + Sync),
    most_placed: usize,
    total: usize,
}

impl Progress<'_> {
    fn placed(&mut self, count: usize) {
        self.most_placed = self.most_placed.max(count);
        (self.callback)(self.most_placed, self.total);
    }
}

fn initial_search(
    request: &Request,
    geometry: &Geometry,
    budget: &Budget<'_>,
    rng: &mut Xoshiro256PlusPlus,
    progress: &mut Progress<'_>,
) -> Option<Layout> {
    let mut order = largest_first(geometry);
    let mut best = current_layout(
        &geometry.stock,
        &geometry.items,
        &geometry.item_origins,
        geometry.origin,
        request.settings.rotation,
    );
    let mut best_width = best.as_ref().map_or(f32::INFINITY, occupied_width);
    progress.placed(if best.is_some() { progress.total } else { 0 });
    let mut attempt = 0;
    while !budget.kill() {
        if best.is_some() && request.rectangular {
            break;
        }
        let mut layout = Layout::new(geometry.stock.clone());
        if attempt > 0 {
            order.shuffle(rng);
        }
        fill_layout(&mut layout, &geometry.items, &order, budget, rng, progress);
        if layout.placed_items.len() == progress.total && layout.is_feasible() {
            let width = occupied_width(&layout);
            if width < best_width {
                best_width = width;
                best = Some(layout);
            }
            if request.rectangular {
                break;
            }
        }
        attempt += 1;
    }
    best
}

fn fill_layout(
    layout: &mut Layout,
    items: &[(Item, usize)],
    order: &[usize],
    budget: &Budget<'_>,
    rng: &mut Xoshiro256PlusPlus,
    progress: &mut Progress<'_>,
) {
    for &id in order {
        if budget.kill() {
            break;
        }
        let item = &items[id].0;
        let evaluator = Evaluation { inner: LBFEvaluator::new(layout, item), budget };
        let (sample, _) =
            search_placement(layout, item, None, evaluator, REPEATED_FIT_SAMPLES, rng);
        if let Some((dt, SampleEval::Clear { .. })) = sample {
            layout.place_item(item, dt);
        } else {
            break;
        }
        progress.placed(layout.placed_items.len());
    }
}

fn solution(
    request: &Request,
    layout: &Layout,
    items: &[(Item, usize)],
    origin: Point,
    item_origins: &[Point],
    complete: bool,
) -> Result<Solution, Error> {
    let mut counts = vec![0; items.len()];
    let mut placements = Vec::new();
    for placed in layout.placed_items.values() {
        let placement = placement(request, placed, items, origin, item_origins);
        let original = &request.items[placement.item];
        let moved: Vec<_> =
            original.contours.iter().map(|c| placement.transform.contour(c)).collect();
        crate::check_region(&request.stock, &request.cutouts, &moved, request.settings.margin)?;
        counts[placement.item] += 1;
        placements.push(placement);
    }
    if complete && counts.iter().zip(items).any(|(a, (_, b))| a != b) {
        return Err(Error("nesting result lost a requested copy".into()));
    }
    Ok(Solution { placements, coverage: coverage(request, items, &counts) })
}

/// A sheet as a running search has it, for showing only: every copy clear
/// of the others, not yet checked against the stock's own curves.
fn so_far<'a>(
    request: &Request,
    placed: impl Iterator<Item = &'a PlacedItem>,
    geometry: &Geometry,
    original: bool,
) -> SheetSolution {
    let items = &geometry.items;
    let placements: Vec<_> = placed
        .map(|p| placement(request, p, items, geometry.origin, &geometry.item_origins))
        .collect();
    let mut counts = vec![0; items.len()];
    for placement in &placements {
        counts[placement.item] += 1;
    }
    SheetSolution {
        original,
        layout: Solution { coverage: coverage(request, items, &counts), placements },
    }
}

/// Where a placed copy goes: a rigid transform of its item's contours.
fn placement(
    request: &Request,
    placed: &PlacedItem,
    items: &[(Item, usize)],
    origin: Point,
    item_origins: &[Point],
) -> Placement {
    let item = &items[placed.item_id].0;
    let dt = int_to_ext_transformation(&placed.d_transf, &item.shape_orig.pre_transform);
    let (sin, cos) = rotation(dt.rotation(), request.settings.rotation);
    let item_origin = item_origins[placed.item_id];
    let (x, y) = dt.translation();
    let transform = Transform([
        cos,
        sin,
        -sin,
        cos,
        origin.x + f64::from(x) - cos * item_origin.x + sin * item_origin.y,
        origin.y + f64::from(y) - sin * item_origin.x - cos * item_origin.y,
    ]);
    Placement { item: placed.item_id, transform }
}

/// The share of the usable stock that `counts` copies of each item cover.
#[allow(
    clippy::cast_precision_loss,
    reason = "at most 500 item quantities are exactly representable"
)]
fn coverage(request: &Request, items: &[(Item, usize)], counts: &[usize]) -> f64 {
    let area: f64 =
        items.iter().zip(counts).map(|((i, _), q)| f64::from(i.area()) * *q as f64).sum();
    let available = (request.stock.signed_area().abs()
        - request.cutouts.iter().map(|c| c.signed_area().abs()).sum::<f64>())
    .max(area);
    (area / available).clamp(0., 1.)
}

fn validate(request: &Request) -> Result<usize, Error> {
    request.settings.validate().map_err(Error)?;
    let total = request
        .items
        .iter()
        .try_fold(0usize, |sum, item| sum.checked_add(item.quantity))
        .ok_or_else(|| {
        Error(format!("nest between 1 and {MAX_COPIES} complete parts at a time"))
    })?;
    if total == 0
        || total > MAX_COPIES
        || request.items.iter().any(|i| i.quantity == 0 || i.contours.is_empty())
    {
        return Err(Error(format!("nest between 1 and {MAX_COPIES} complete parts at a time")));
    }
    if request.time_limit.is_zero()
        || request.time_limit > MAX_SEARCH_TIME
        || !request.machining_clearance.is_finite()
        || !(0. ..=MAX_MACHINING_CLEARANCE).contains(&request.machining_clearance)
    {
        return Err(Error("invalid nesting time or machining clearance".into()));
    }
    if request.items.iter().any(|item| !fits(&request.sheet_limit, load(item))) {
        return Err(Error(format!(
            "a part holds more than one sheet can prepare ({} contours, {} lines and arcs)",
            request.sheet_limit.contours, request.sheet_limit.curves
        )));
    }
    Ok(total)
}

/// The contours and curves one copy of `item` adds to a sheet.
fn load(item: &crate::Item) -> [usize; 2] {
    [item.contours.len(), item.contours.iter().map(|c| c.curves.len()).sum()]
}

fn fits(limit: &crate::SheetLimit, load: [usize; 2]) -> bool {
    load[0] <= limit.contours && load[1] <= limit.curves
}

fn plus(a: [usize; 2], b: [usize; 2]) -> [usize; 2] {
    [a[0].saturating_add(b[0]), a[1].saturating_add(b[1])]
}

struct Geometry {
    stock: Container,
    items: Vec<(Item, usize)>,
    origin: Point,
    item_origins: Vec<Point>,
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "validated coordinates are padded for f32 conversion"
)]
fn geometry(request: &Request, cancel: &AtomicBool) -> Result<Geometry, Error> {
    let deadline = Instant::now() + MAX_SEARCH_TIME;
    let stock_points = polygon(&request.stock)?;
    let bounds = request.stock.bounds().ok_or_else(|| Error("empty stock".into()))?;
    let origin = bounds.min;
    let config = DEFAULT_SPARROW_CONFIG;
    let item_offset = request.settings.spacing / 2. + request.machining_clearance + GUARD;
    let stock_offset = request.settings.margin + GUARD;
    let item_importer =
        Importer::new(config.cde_config, None, Some((2. * item_offset) as f32), None);
    let stock_importer =
        Importer::new(config.cde_config, None, Some((2. * stock_offset) as f32), None);
    let holes = request.cutouts.iter().map(polygon).collect::<Result<Vec<_>, Error>>()?;
    let hole_vertices = holes.iter().map(Vec::len).sum::<usize>();
    if hole_vertices + stock_points.len() > MAX_TOTAL_VERTICES {
        return Err(Error(format!("nesting geometry exceeds {MAX_TOTAL_VERTICES} vertices")));
    }
    let stock = stock_importer
        .import_container(&ExtContainer {
            id: 0,
            shape: ext(&stock_points, origin),
            zones: holes
                .iter()
                .map(|p| ExtQualityZone { quality: 0, shape: ext(p, origin) })
                .collect(),
        })
        .map_err(err)?;
    let mut shapes = Shapes {
        importer: item_importer,
        orientations: orientations(request.settings.rotation),
        cache: BTreeMap::new(),
    };
    let mut vertices = stock_points.len() + hole_vertices;
    let mut items = Vec::new();
    let mut item_origins = Vec::new();
    for (id, input) in request.items.iter().enumerate() {
        check_cancelled(cancel)?;
        if Instant::now() >= deadline {
            return Err(Error(
                "shape preparation took too long; use fewer or simpler parts".into(),
            ));
        }
        let points = outer_polygon(input, id)?;
        vertices += points.len();
        if vertices > MAX_TOTAL_VERTICES {
            return Err(Error(format!("nesting geometry exceeds {MAX_TOTAL_VERTICES} vertices")));
        }
        let item_origin = Point::new(
            points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
            points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        );
        let item = shapes.import(id, &points, item_origin)?;
        items.push((item, input.quantity));
        item_origins.push(item_origin);
    }
    Ok(Geometry { stock, items, origin, item_origins })
}

fn orientations(rotation: NestRotation) -> Option<Vec<f32>> {
    match rotation {
        NestRotation::Any => None,
        NestRotation::HalfTurn => Some(vec![0., 180.]),
        NestRotation::Across => Some(vec![90., 270.]),
        NestRotation::Fixed => Some(vec![0.]),
    }
}

struct Shapes {
    importer: Importer,
    orientations: Option<Vec<f32>>,
    cache: BTreeMap<Vec<(i64, i64)>, Item>,
}

impl Shapes {
    fn import(&mut self, id: usize, points: &[[f64; 2]], origin: Point) -> Result<Item, Error> {
        let (key, normalized) = canonical(points, origin);
        if let Some(item) = self.cache.get(&key) {
            let mut item = item.clone();
            item.id = id;
            return Ok(item);
        }
        let item = self
            .importer
            .import_item(&ExtItem {
                id: id.try_into().map_err(err)?,
                shape: ext(&normalized, Point::ORIGIN),
                allowed_orientations: self.orientations.clone(),
                min_quality: None,
            })
            .map_err(err)?;
        self.cache.insert(key, item.clone());
        Ok(item)
    }
}

fn outer_polygon(input: &crate::Item, id: usize) -> Result<Vec<[f64; 2]>, Error> {
    let outer = input
        .contours
        .iter()
        .filter(|c| c.is_closed())
        .max_by(|a, b| a.signed_area().abs().total_cmp(&b.signed_area().abs()))
        .ok_or_else(|| Error(format!("part {} has no closed outer contour", id + 1)))?;
    let points = polygon(outer)?;
    let poly: Vec<Point> = points.iter().copied().map(Into::into).collect();
    // The original outer arc can sit outside its own inscribed polygon.
    // Test only the other contours, once, against that boundary.
    for other in &input.contours {
        if std::ptr::eq(other, outer) {
            continue;
        }
        if other
            .curves
            .iter()
            .any(|c| (0..=8).any(|k| !contains(&poly, c.point(f64::from(k) / 8.))))
        {
            return Err(Error(format!(
                "part {} contains separate outer shapes; remove their connection before nesting",
                id + 1
            )));
        }
    }
    Ok(points)
}

// A 0.0001 mm quantization is covered by the 0.05 mm collision guard. Normalize
// translation and vertex order so copies reuse one expensive offset/surrogate.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "coordinates are bounded to 100 m and padded by the collision guard"
)]
fn canonical(points: &[[f64; 2]], origin: Point) -> (Vec<(i64, i64)>, Vec<[f64; 2]>) {
    let mut key: Vec<_> = points
        .iter()
        .map(|p| {
            (
                ((p[0] - origin.x) * 10_000.).round() as i64,
                ((p[1] - origin.y) * 10_000.).round() as i64,
            )
        })
        .collect();
    let first = key.iter().enumerate().min_by_key(|(_, p)| **p).map_or(0, |(i, _)| i);
    key.rotate_left(first);
    let normalized = key.iter().map(|&(x, y)| [x as f64 / 10_000., y as f64 / 10_000.]).collect();
    (key, normalized)
}

fn occupied_width(layout: &Layout) -> f32 {
    layout.placed_items.values().map(|p| p.shape.bbox.x_max).fold(0., f32::max)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "finite input coordinates are bounded and conversion is padded"
)]
fn current_layout(
    stock: &Container,
    items: &[(Item, usize)],
    origins: &[Point],
    stock_origin: Point,
    rotation: NestRotation,
) -> Option<Layout> {
    if rotation == NestRotation::Across || items.iter().any(|(_, quantity)| *quantity != 1) {
        return None;
    }
    let mut layout = Layout::new(stock.clone());
    for ((item, _), origin) in items.iter().zip(origins) {
        let external = DTransformation::new(
            0.,
            ((origin.x - stock_origin.x) as f32, (origin.y - stock_origin.y) as f32),
        );
        let transform = ext_to_int_transformation(&external, &item.shape_orig.pre_transform);
        if !matches!(
            LBFEvaluator::new(&layout, item).evaluate_sample(transform, None),
            SampleEval::Clear { .. }
        ) {
            return None;
        }
        layout.place_item(item, transform);
    }
    layout.is_feasible().then_some(layout)
}

fn rotation(angle: f32, allowed: NestRotation) -> (f64, f64) {
    if allowed == NestRotation::Any {
        return f64::from(angle).sin_cos();
    }
    // jagua stores angles in f32. Preserve the exact requested grain rotations
    // in the authoring geometry instead of accumulating tiny skews each run.
    let turn = (f64::from(angle) / std::f64::consts::FRAC_PI_2).round().rem_euclid(4.);
    if turn < 0.5 {
        (0., 1.)
    } else if turn < 1.5 {
        (1., 0.)
    } else if turn < 2.5 {
        (0., -1.)
    } else {
        (-1., 0.)
    }
}

#[allow(clippy::cast_possible_truncation, reason = "remaining time is bounded to 30 seconds")]
fn compress(
    mut layout: Layout,
    items: &[(Item, usize)],
    stock: &Container,
    budget: &mut Budget<'_>,
    rng: &mut Xoshiro256PlusPlus,
    listener: &mut impl SolutionListener,
) -> Result<Layout, Error> {
    let bbox = stock.outer_orig.shape.bbox;
    let strip = Strip::new(
        bbox.height(),
        DEFAULT_SPARROW_CONFIG.cde_config,
        ShapeModifyConfig {
            offset: stock.outer_orig.modify_config.offset,
            ..ShapeModifyConfig::default()
        },
        bbox.width(),
    )
    .map_err(err)?;
    // Rebuild under the same rectangular container identity as SPProblem.
    layout.swap_container(strip.into());
    if !layout.is_feasible() {
        return Err(Error("rectangular warm start failed its collision check".into()));
    }
    let instance = SPInstance::new(items.to_vec(), strip);
    // Start from a strip fitted to the first fit, as Sparrow does after its
    // own construction. From the whole sheet, it shrinks by a tenth of a
    // percent per step and spends the search loosening a tight first fit.
    let mut start = SPProblem::new(instance.clone());
    start.restore(&SPSolution {
        strip,
        layout_snapshot: layout.save(),
        time_stamp: jagua_rs::Instant::now(),
    });
    start.fit_strip();
    let initial = start.save();
    let fitted = initial.strip.width;
    let remaining = budget.deadline.saturating_duration_since(Instant::now());
    let mut config = DEFAULT_SPARROW_CONFIG;
    config.expl_cfg.time_limit = remaining.mul_f64(EXPLORE_SHARE);
    config.cmpr_cfg.time_limit = remaining.mul_f64(COMPRESS_SHARE);
    config.expl_cfg.separator_config.n_workers = SEPARATOR_WORKERS;
    config.cmpr_cfg.separator_config.n_workers = SEPARATOR_WORKERS;
    // Sparrow panics once it shrinks the strip below a part it must place,
    // which a sheet of few parts can start close to. The first fit stands.
    let optimized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sparrow::optimizer::optimize(
            instance,
            rng.clone(),
            listener,
            budget,
            &config.expl_cfg,
            &config.cmpr_cfg,
            Some(&initial),
        )
    }));
    let Ok(result) = optimized else { return Ok(layout) };
    let result = result.map_err(err)?;
    let candidate = Layout::from_snapshot(&result.layout_snapshot);
    if candidate.is_feasible() && result.strip.width <= fitted { Ok(candidate) } else { Ok(layout) }
}
