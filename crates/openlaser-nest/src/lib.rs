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

/// One rigid manufactured part, including its holes and interior markings.
#[derive(Clone, Debug)]
pub struct Item {
    /// Current geometry in drawing coordinates.
    pub contours: Vec<Contour>,
    /// Required copies, including the original.
    pub quantity: usize,
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

/// Search without ever changing the input. Progress reports placed/total.
pub fn nest(
    request: &Request,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Solution, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
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

/// Place all requested copies across numbered sheets. `overflow` replaces the
/// first sheet's outline/cutouts when continuing from a physical remnant.
pub fn nest_sheets(
    request: &Request,
    overflow: Option<&Contour>,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Vec<SheetSolution>, Error> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .map_err(|e| Error(format!("could not start nesting workers: {e}")))?;
    pool.install(|| search::run_sheets(request, overflow, cancel, &progress))
}
