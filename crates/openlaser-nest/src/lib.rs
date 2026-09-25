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
#[error("{message}")]
pub struct Error {
    /// What went wrong, for the operator.
    pub message: String,
    /// The chosen sheets filled up with parts still to place, so another
    /// sheet would take the rest.
    pub sheets_full: bool,
}

impl Error {
    pub(crate) fn new(message: String) -> Self {
        Self { message, sheets_full: false }
    }

    /// Every chosen sheet is full: `placed` of `total` parts fit.
    pub(crate) fn sheets_full(placed: usize, total: usize) -> Self {
        Self {
            message: format!("{placed} of {total} parts fit on the chosen sheets; add sheets"),
            sheets_full: true,
        }
    }
}

/// Worker threads for one nesting search. Two keep the controller link and
/// the HTTP server responsive on small shop computers while a search runs;
/// a choice, not a measured optimum. Sparrow's separator runs as many
/// workers.
pub(crate) const SEARCH_THREADS: usize = 2;

/// Search without ever changing the input. Progress reports placed/total.
pub fn nest(
    request: &Request,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Solution, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(SEARCH_THREADS)
        .build()
        .map_err(|e| Error::new(format!("could not start nesting workers: {e}")))?;
    pool.install(|| search::run(request, cancel, &progress))
}

/// One kind of sheet a multi-sheet search may use, in the order given.
#[derive(Clone, Debug)]
pub struct Sheet {
    /// A closed stock outline in drawing coordinates.
    pub stock: Contour,
    /// Unavailable material inside or crossing the stock boundary.
    pub cutouts: Vec<Contour>,
    /// Whether stock is an axis-aligned rectangle, enabling strip compression.
    pub rectangular: bool,
    /// Minimum distance from this sheet's edges and cutouts; a remnant keeps
    /// a wider band around earlier cuts than a fresh sheet.
    pub margin: f64,
    /// How many of this sheet exist, or `None` for as many as needed.
    pub count: Option<usize>,
}

/// One complete sheet in a multi-sheet layout.
#[derive(Clone, Debug)]
pub struct SheetSolution {
    /// Which of the requested [`Sheet`]s this is.
    pub sheet: usize,
    /// Rigid placements on this sheet.
    pub layout: Solution,
}

/// Place all requested copies across `sheets`, largest parts first: each
/// copy goes on the first open sheet with room, else on a new sheet of the
/// first kind that still has one left and can hold it, so earlier kinds (a
/// remnant, say) fill before later ones. The last sheet is then compressed
/// with the remaining time when it is a plain rectangle. The request's own
/// stock fields are ignored; its items, settings and limits apply to every
/// sheet. The result lists sheets in the order of their kinds.
///
/// `live` sees the sheets as the search stands and the index of the one
/// that just changed, at most every [`LIVE_INTERVAL`]: each copy placed so
/// far, clear of the others, while sheets fill and each time the last one
/// compacts further. Only the result is checked against the stock's curves
/// and can be applied.
pub fn nest_sheets(
    request: &Request,
    sheets: &[Sheet],
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
    live: impl Fn(&[SheetSolution], usize) + Sync,
) -> Result<Vec<SheetSolution>, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(SEARCH_THREADS)
        .build()
        .map_err(|e| Error::new(format!("could not start nesting workers: {e}")))?;
    pool.install(|| {
        let mut live = search::Live::new(&live);
        let mut solutions = search::run_sheets(request, sheets, cancel, &progress, &mut live)?;
        solutions.sort_by_key(|s| s.sheet);
        Ok(solutions)
    })
}
