// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded Sparrow searches over conservative polygons. Output is only rigid
//! transforms of the original curves. No I/O, compiler or controller access.

mod polygon;
mod search;

use openlaser_core::geometry::{Contour, Transform};
use openlaser_core::nesting::NestSettings;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use polygon::{check_inside, check_polylines, check_region, check_region_polylines, polygon};
pub use search::LIVE_INTERVAL;

/// One rigid manufactured part, including its holes and interior markings.
#[derive(Clone, Debug)]
pub struct Item {
    /// Current geometry in drawing coordinates.
    pub contours: Vec<Contour>,
    /// Required copies, including the original.
    pub quantity: usize,
}

/// What one sheet may hold, every copy's geometry together. Preparation
/// refuses a larger sheet, so a sheet that would pass either bound is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetLimit {
    /// Contours on one sheet.
    pub contours: usize,
    /// Lines and arcs on one sheet.
    pub curves: usize,
}

/// All inputs are immutable snapshots of authoring data.
#[derive(Clone, Debug)]
pub struct Request {
    /// A closed stock outline in drawing coordinates.
    pub stock: Contour,
    /// Unavailable material inside or crossing the stock boundary.
    pub cutouts: Vec<Contour>,
    /// Whether stock is an axis-aligned rectangle, enabling strip compression.
    pub rectangular: bool,
    /// Parts to place; a hole cannot receive another part.
    pub items: Vec<Item>,
    /// Operator distances and rotations.
    pub settings: NestSettings,
    /// Additional outward reach of leads, kerf and seam treatments.
    pub machining_clearance: f64,
    /// What one sheet may hold.
    pub sheet_limit: SheetLimit,
    /// Total search budget, at most 30 seconds.
    pub time_limit: Duration,
    /// Reproducible random input, independent of machine state.
    pub seed: u64,
}

/// One original item under a rigid placement transform.
#[derive(Clone, Debug)]
pub struct Placement {
    /// Index in the request's item list.
    pub item: usize,
    /// Applied after every current contour transform.
    pub transform: Transform,
}

/// A complete, collision-checked result.
#[derive(Clone, Debug)]
pub struct Solution {
    /// Every requested instance, with none silently omitted.
    pub placements: Vec<Placement>,
    /// Fraction of stock covered by outer part areas, holes reserved.
    pub coverage: f64,
}

/// A rejected input or an incomplete heuristic search.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

/// Worker threads for one nesting search. Two keep the controller link and
/// the HTTP server responsive on small shop computers while a search runs;
/// a choice, not a measured optimum.
const SEARCH_THREADS: usize = 2;

/// Search without ever changing the input. Progress reports placed/total.
pub fn nest(
    request: &Request,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Solution, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(SEARCH_THREADS)
        .build()
        .map_err(|e| Error(format!("could not start nesting workers: {e}")))?;
    pool.install(|| search::run(request, cancel, &progress))
}

/// One complete sheet in a multi-sheet layout.
#[derive(Clone, Debug)]
pub struct SheetSolution {
    /// True for the original stock, false for a fresh overflow sheet.
    pub original: bool,
    /// Rigid placements on this sheet.
    pub layout: Solution,
}

/// Place all requested copies across numbered sheets, largest first, then
/// compress the last sheet with the remaining time when it is a plain
/// rectangle. `overflow`, when given, is the outline of every sheet after
/// the first, which keeps the request's own stock and cutouts.
///
/// `live` sees the sheets as the search stands and the index of the one
/// that just changed, at most every [`LIVE_INTERVAL`]: each copy placed so
/// far, clear of the others, while sheets fill and each time the last one
/// compacts further. Only the result is checked against the stock's curves
/// and can be applied.
pub fn nest_sheets(
    request: &Request,
    overflow: Option<&Contour>,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
    live: impl Fn(&[SheetSolution], usize) + Sync,
) -> Result<Vec<SheetSolution>, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(SEARCH_THREADS)
        .build()
        .map_err(|e| Error(format!("could not start nesting workers: {e}")))?;
    pool.install(|| {
        let mut live = search::Live::new(&live);
        search::run_sheets(request, overflow, cancel, &progress, &mut live)
    })
}
