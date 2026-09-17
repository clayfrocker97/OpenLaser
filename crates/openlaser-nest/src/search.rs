// SPDX-License-Identifier: GPL-3.0-or-later

use crate::polygon::{GUARD, contains, polygon};
use crate::{Error, Placement, Request, Solution};
use jagua_rs::entities::{Container, Item, Layout};
use jagua_rs::geometry::DTransformation;
use jagua_rs::geometry::shape_modification::ShapeModifyConfig;
use jagua_rs::io::export::int_to_ext_transformation;
use jagua_rs::io::ext_repr::{ExtContainer, ExtItem, ExtQualityZone, ExtSPolygon, ExtShape};
use jagua_rs::io::import::{Importer, ext_to_int_transformation};
use jagua_rs::probs::spp::entities::{SPInstance, SPSolution, Strip};
use openlaser_core::geometry::{Point, Transform};
use openlaser_core::nesting::NestRotation;
use rand::rngs::Xoshiro256PlusPlus;
use rand::{SeedableRng, seq::SliceRandom};
use sparrow::config::DEFAULT_SPARROW_CONFIG;
use sparrow::eval::lbf_evaluator::LBFEvaluator;
use sparrow::eval::sample_eval::{SampleEval, SampleEvaluator};
use sparrow::sample::search::{SampleConfig, search_placement};
use sparrow::util::listener::DummySolListener;
use sparrow::util::terminator::Terminator;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
        best = compress(best, &geometry.items, &geometry.stock, &mut budget, &mut rng)?;
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
) -> Result<Vec<crate::SheetSolution>, Error> {
    let total = validate(request)?;
    check_cancelled(cancel)?;
    let first = geometry(request, cancel)?;
    let mut blank_request = request.clone();
    if let Some(stock) = overflow {
        blank_request.stock = stock.clone();
        blank_request.cutouts.clear();
        blank_request.rectangular = true;
    }
    let blank = if overflow.is_some() { Some(geometry(&blank_request, cancel)?) } else { None };
    let next = blank.as_ref().unwrap_or(&first);
    let deadline = Instant::now() + request.time_limit;
    let budget = Budget { cancel, deadline, phase: deadline };
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(request.seed);
    let mut order: Vec<usize> =
        first.items.iter().enumerate().flat_map(|(i, (_, q))| std::iter::repeat_n(i, *q)).collect();
    order.sort_by(|a, b| {
        first.items[*b].0.shape_cd.area.total_cmp(&first.items[*a].0.shape_cd.area)
    });
    let mut layouts = vec![Layout::new(first.stock.clone())];
    for (placed, id) in order.into_iter().enumerate() {
        check_cancelled(cancel)?;
        if budget.kill() {
            return Err(Error(format!(
                "nesting time limit reached ({placed} of {total} parts); increase search time"
            )));
        }
        let mut fitted = false;
        for (index, layout) in layouts.iter_mut().enumerate() {
            let geometry = if index == 0 { &first } else { next };
            if place_one(layout, &geometry.items[id].0, &budget, &mut rng) {
                fitted = true;
                break;
            }
        }
        if !fitted {
            if budget.kill() {
                return Err(Error(format!(
                    "nesting time limit reached ({placed} of {total} parts); increase search time"
                )));
            }
            let mut layout = Layout::new(next.stock.clone());
            if !place_one(&mut layout, &next.items[id].0, &budget, &mut rng) {
                return Err(Error(format!(
                    "part {} could not fit on an empty sheet with this spacing and rotation",
                    id + 1
                )));
            }
            layouts.push(layout);
        }
        progress(placed + 1, total);
    }
    check_cancelled(cancel)?;
    let mut counts = vec![0usize; request.items.len()];
    let mut sheets = Vec::new();
    for (index, layout) in layouts.into_iter().enumerate() {
        if layout.placed_items.is_empty() {
            continue;
        }
        if !layout.is_feasible() {
            return Err(Error("a sheet failed its collision check".into()));
        }
        let geometry = if index == 0 { &first } else { next };
        let input = if index == 0 { request } else { &blank_request };
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
        sheets.push(crate::SheetSolution { original: index == 0, layout });
    }
    if counts.iter().zip(&request.items).any(|(count, item)| *count != item.quantity) {
        return Err(Error("multi-sheet nesting lost a requested copy".into()));
    }
    Ok(sheets)
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
    let (sample, _) = search_placement(
        layout,
        item,
        None,
        evaluator,
        SampleConfig { n_container_samples: 350, n_focussed_samples: 0, n_coord_descents: 3 },
        rng,
    );
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
    let mut order: Vec<usize> = geometry
        .items
        .iter()
        .enumerate()
        .flat_map(|(i, (_, q))| std::iter::repeat_n(i, *q))
        .collect();
    order.sort_by(|&a, &b| {
        geometry.items[b].0.shape_cd.area.total_cmp(&geometry.items[a].0.shape_cd.area)
    });
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
        let (sample, _) = search_placement(
            layout,
            item,
            None,
            evaluator,
            SampleConfig { n_container_samples: 250, n_focussed_samples: 0, n_coord_descents: 3 },
            rng,
        );
        if let Some((dt, SampleEval::Clear { .. })) = sample {
            layout.place_item(item, dt);
        } else {
            break;
        }
        progress.placed(layout.placed_items.len());
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "at most 500 item quantities are exactly representable"
)]
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
    for pi in layout.placed_items.values() {
        let item = &items[pi.item_id].0;
        let dt = int_to_ext_transformation(&pi.d_transf, &item.shape_orig.pre_transform);
        let (sin, cos) = rotation(dt.rotation(), request.settings.rotation);
        let item_origin = item_origins[pi.item_id];
        let (x, y) = dt.translation();
        let transform = Transform([
            cos,
            sin,
            -sin,
            cos,
            origin.x + f64::from(x) - cos * item_origin.x + sin * item_origin.y,
            origin.y + f64::from(y) - sin * item_origin.x - cos * item_origin.y,
        ]);
        let original = &request.items[pi.item_id];
        let moved: Vec<_> = original.contours.iter().map(|c| transform.contour(c)).collect();
        crate::check_region(&request.stock, &request.cutouts, &moved, request.settings.margin)?;
        counts[pi.item_id] += 1;
        placements.push(Placement { item: pi.item_id, transform });
    }
    if complete && counts.iter().zip(items).any(|(a, (_, b))| a != b) {
        return Err(Error("nesting result lost a requested copy".into()));
    }
    let area: f64 =
        items.iter().zip(&counts).map(|((i, _), q)| f64::from(i.area()) * *q as f64).sum();
    let available = (request.stock.signed_area().abs()
        - request.cutouts.iter().map(|c| c.signed_area().abs()).sum::<f64>())
    .max(area);
    Ok(Solution { placements, coverage: (area / available).clamp(0., 1.) })
}

fn validate(request: &Request) -> Result<usize, Error> {
    request.settings.validate().map_err(Error)?;
    let total = request
        .items
        .iter()
        .try_fold(0usize, |sum, item| sum.checked_add(item.quantity))
        .ok_or_else(|| Error("nest between 1 and 500 complete parts at a time".into()))?;
    if total == 0
        || total > 500
        || request.items.iter().any(|i| i.quantity == 0 || i.contours.is_empty())
    {
        return Err(Error("nest between 1 and 500 complete parts at a time".into()));
    }
    if request.time_limit.is_zero()
        || request.time_limit > Duration::from_secs(30)
        || !request.machining_clearance.is_finite()
        || !(0. ..=1000.).contains(&request.machining_clearance)
    {
        return Err(Error("invalid nesting time or machining clearance".into()));
    }
    Ok(total)
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
    let deadline = Instant::now() + Duration::from_secs(30);
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
    if hole_vertices + stock_points.len() > 50_000 {
        return Err(Error("nesting geometry exceeds 50000 vertices".into()));
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
        if vertices > 50_000 {
            return Err(Error("nesting geometry exceeds 50000 vertices".into()));
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
    let initial =
        SPSolution { strip, layout_snapshot: layout.save(), time_stamp: jagua_rs::Instant::now() };
    let remaining = budget.deadline.saturating_duration_since(Instant::now());
    let mut config = DEFAULT_SPARROW_CONFIG;
    config.expl_cfg.time_limit = remaining.mul_f64(0.8);
    config.cmpr_cfg.time_limit = remaining.mul_f64(0.2);
    config.expl_cfg.separator_config.n_workers = 2;
    config.cmpr_cfg.separator_config.n_workers = 2;
    let result = sparrow::optimizer::optimize(
        instance,
        rng.clone(),
        &mut DummySolListener,
        budget,
        &config.expl_cfg,
        &config.cmpr_cfg,
        Some(&initial),
    )
    .map_err(err)?;
    let candidate = Layout::from_snapshot(&result.layout_snapshot);
    if candidate.is_feasible() && result.strip.width <= strip.width {
        Ok(candidate)
    } else {
        Ok(layout)
    }
}
