// SPDX-License-Identifier: GPL-3.0-or-later

//! Machine-exact. Compiles prepared contours, a recipe and machine settings
//! into the native program the controller executes.
//!
//! The pipeline is the vendor's, stage for stage. A contour is preprocessed
//! the way M-Laser's CAD module does it ([`geometry`]), planned into
//! jerk-limited velocity profiles and sampled at the interpolation cadence
//! ([`planner`]), refined with joints and cooling stops ([`process`]),
//! quantised into pulse increments with carried residuals ([`motion`]), and
//! wrapped in the process commands that pierce, follow, gas and fire
//! ([`program`]). Every stage reproduces recovered arithmetic in the
//! recovered order. Do not simplify an expression here without a golden test
//! proving the samples still match.
//!
//! [`program::Job::schedule_prepared`] is the host entry point for geometry
//! already prepared by OpenLaser. It bypasses the native curve reduction and
//! fitting that can distort small outlines, while retaining corner rounding
//! and the same planning, sampling and record emission. The native entry
//! points remain available for the pinned reference tests.
//!
//! The crate has no I/O. Uploading and executing a program belong to the
//! controller crate; reading vendor settings belongs to the XML crate.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unreadable_literal,
        clippy::too_many_lines,
        reason = "tests unwrap freely and pin long records copied verbatim from verified output"
    )
)]
#![allow(clippy::float_cmp, reason = "the vendor compares floats exactly, and so do we")]
#![allow(clippy::many_single_char_names, reason = "recovered formulas keep their operands' names")]

mod continuation;
pub mod cut;
pub mod frame;
pub mod geometry;
pub mod head;
pub mod motion;
pub mod pass;
pub mod planner;
pub mod process;
pub mod program;
pub mod pulse;
pub mod residue;
pub mod settings;
pub mod travel;

/// A point or vector in millimetres, `[x, y]`.
pub type Point = [f64; 2];

/// Host memory bound for one program's interpolation samples: about 2 h 13 m
/// at a 250 microsecond cadence. Immutable sample arrays are shared by the
/// plan, preview and continuation. Uploads use independently bounded FIFO frames.
pub const MAX_SAMPLES: usize = 32_000_000;

/// Why a contour or program could not be compiled.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// An input is outside the domain the vendor's code handles.
    #[error("{0}")]
    Invalid(&'static str),
    /// Arithmetic left the finite domain. The vendor's code would carry on
    /// with garbage here; we stop.
    #[error("arithmetic left the finite domain")]
    NonFinite,
    /// A resource ceiling was hit. The vendor's loops are unbounded; ours
    /// refuse rather than spin.
    #[error("{0}")]
    Budget(&'static str),
    /// A record could not be validated or packed.
    #[error(transparent)]
    Record(#[from] openlaser_protocol::records::RecordError),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Passes a value through if it is finite.
pub(crate) fn finite(value: f64) -> Result<f64> {
    if value.is_finite() { Ok(value) } else { Err(Error::NonFinite) }
}

/// A count as a float.
#[allow(clippy::cast_precision_loss, reason = "counts here stay far below 2^53")]
pub(crate) const fn float(count: usize) -> f64 {
    count as f64
}

/// A node index as the 32-bit word a profile stores.
pub(crate) fn index(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::Invalid("index does not fit a 32-bit word"))
}

/// Truncates toward zero, as the vendor's float-to-integer conversion does,
/// refusing values that do not fit a 64-bit integer.
#[allow(clippy::cast_possible_truncation, reason = "range checked before the cast")]
pub(crate) fn to_i64(value: f64) -> Result<i64> {
    let truncated = value.trunc();
    if !truncated.is_finite() || truncated.abs() > 9.0e18 {
        return Err(Error::Invalid("value does not fit an integer"));
    }
    Ok(truncated as i64)
}

/// Truncates toward zero into a `u32`, refusing negatives and overflow.
pub(crate) fn to_u32(value: f64) -> Result<u32> {
    u32::try_from(to_i64(value)?).map_err(|_| Error::Invalid("value does not fit a 32-bit field"))
}

/// Truncates toward zero into an `i32`, refusing overflow.
pub(crate) fn to_i32(value: f64) -> Result<i32> {
    i32::try_from(to_i64(value)?)
        .map_err(|_| Error::Invalid("value does not fit a signed 32-bit field"))
}

/// Truncates toward zero into a `u16`, refusing negatives and overflow.
pub(crate) fn to_u16(value: f64) -> Result<u16> {
    u16::try_from(to_i64(value)?).map_err(|_| Error::Invalid("value does not fit a 16-bit field"))
}

/// Truncates toward zero into a `u8`, refusing negatives and overflow.
pub(crate) fn to_u8(value: f64) -> Result<u8> {
    u8::try_from(to_i64(value)?).map_err(|_| Error::Invalid("value does not fit a byte"))
}
