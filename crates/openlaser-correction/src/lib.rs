// SPDX-License-Identifier: GPL-3.0-or-later

//! Local XY size correction from nine measured 100 mm coupons.
//!
//! The model interpolates local scale, integrates from the bed centre and
//! numerically inverts the resulting coordinate map. It models repeatable
//! size variation; coupon dimensions do not measure absolute position or skew.
//! This crate owns no files, machine commands, UI or global state.

mod model;
mod sampling;

pub use model::{Map, Measurement, Profile, coupon_drawing};
pub use sampling::Piece;

/// Nominal width and height of every calibration coupon, in millimetres.
pub const COUPON_MM: f64 = 100.;

/// Invalid measurements, a folded map or a path beyond the sampling budget.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
