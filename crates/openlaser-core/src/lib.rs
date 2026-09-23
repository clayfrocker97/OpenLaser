// SPDX-License-Identifier: GPL-3.0-or-later

//! Domain types shared by every OpenLaser crate: units, planar geometry, the
//! machining features an operator chooses, and the prepared toolpath that
//! preparation hands to the compiler.
//!
//! This crate is pure. It has no I/O, no async and no framework dependencies,
//! so it can be tested exhaustively and reused anywhere. It is the crate
//! everything else builds on and the one that must never break.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests compare exact values"
    )
)]

pub mod features;
pub mod fit;
pub mod geometry;
pub mod grouping;
pub mod nesting;
pub mod repair;
pub mod toolpath;
pub mod units;

use serde::{Deserialize, Serialize};

/// Which laser the machine is set up to fire. The M-series carries both and
/// the operator selects one; the choice changes head behaviour, outputs and
/// the process fields in every record.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaserMode {
    /// The fiber source, with the following head active.
    Fiber,
    /// The CO2 source, with the fiber head parked.
    Co2,
}
