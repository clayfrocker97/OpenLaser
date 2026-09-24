// SPDX-License-Identifier: GPL-3.0-or-later
//! Head calibration belongs to the connected machine and the current material.

use crate::coordinator::Coordinator;
use crate::document::Gate;
use crate::{Error, Result};
use openlaser_controller::State;
use openlaser_controller::session::Quality;
use serde::Serialize;

/// How far the W table may sit from where it was during calibration before
/// the head must be calibrated again: the sheet surface has moved with it.
const TABLE_TOLERANCE_MM: f64 = 0.1;

/// Material and table context recorded when the calibration is requested.
#[derive(Clone, Debug)]
pub(crate) struct Calibration {
    epoch: u64,
    binding: u64,
    material: Option<(String, u64)>,
    /// The W table position, when feedback reported one.
    table_mm: Option<f64>,
}

/// Why a completed calibration no longer applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stale {
    Connection,
    Material,
    Table,
}

impl Calibration {
    pub(crate) fn capture(c: &Coordinator) -> Result<Self> {
        let configuration = c
            .machine
            .state()
            .configuration
            .ok_or_else(|| Error::Refused("read the machine parameters first".into()))?;
        Ok(Self {
            epoch: configuration.epoch,
            binding: configuration.binding,
            material: material(c),
            table_mm: c.machine.state().feedback.map(|f| f.table_mm),
        })
    }

    fn stale(&self, c: &Coordinator, state: &State) -> Option<Stale> {
        if !state.configuration.is_some_and(|configuration| {
            configuration.epoch == self.epoch && configuration.binding == self.binding
        }) {
            return Some(Stale::Connection);
        }
        if self.material != material(c) {
            return Some(Stale::Material);
        }
        let table = state.feedback.as_ref().map(|f| f.table_mm);
        match (self.table_mm, table) {
            (Some(then), Some(now)) if (then - now).abs() > TABLE_TOLERANCE_MM => {
                Some(Stale::Table)
            }
            _ => None,
        }
    }
}

fn material(c: &Coordinator) -> Option<(String, u64)> {
    c.draft.as_ref()?.current.recipe.as_ref().map(|r| (r.name.clone(), r.thickness_mm.to_bits()))
}

/// Calibration status for the material selected in the current job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, Serialize)]
pub struct CalibrationView {
    /// The quality of a completed calibration on this connection.
    pub quality: Option<Quality>,
    /// That calibration still applies to the current material, W table
    /// position and bindings.
    pub current: bool,
    /// Why a completed calibration no longer applies, in plain words, such
    /// as "material changed" or "W table moved".
    pub stale: Option<String>,
}

impl Coordinator {
    pub(crate) fn update_calibration(&mut self, state: &State) {
        // A new material or connection ends the record; a moved table keeps it
        // so the reason can be named.
        if self.calibration.as_ref().is_some_and(|calibration| {
            material(self).is_some()
                && matches!(
                    calibration.stale(self, state),
                    Some(Stale::Material | Stale::Connection)
                )
        }) {
            self.calibration = None;
        }
    }

    pub(crate) fn calibration_view(&self, state: &State) -> CalibrationView {
        let quality = state.session.calibration.filter(|_| {
            state.feedback.as_ref().is_some_and(|f| {
                f.head.referenced && f.age_ms <= crate::coordinator::FRESH_FEEDBACK_MS
            })
        });
        let why = quality.and(match &self.calibration {
            // The material changed and the record was dropped.
            None => Some(Stale::Material),
            Some(calibration) => calibration.stale(self, state),
        });
        let current = matches!(quality, Some(Quality::Excellent | Quality::Good)) && why.is_none();
        let reason_text = why.map(|reason| {
            match reason {
                Stale::Connection => "reconnected",
                Stale::Material => "material changed",
                Stale::Table => "W table moved",
            }
            .to_owned()
        });
        CalibrationView { quality, current, stale: reason_text }
    }

    /// Start needs the head calibrated for this material at this W table
    /// position, when the machine has a height-sensing head. Framing does not.
    pub(crate) fn calibration_gate(&self, state: &State) -> Gate {
        if !self.bound.as_ref().is_some_and(|b| b.head_enabled) {
            return Gate::open();
        }
        let view = self.calibration_view(state);
        if view.current {
            return Gate::open();
        }
        Gate::closed(match (view.quality, view.stale.as_deref()) {
            (None, _) => "calibrate Z first".to_owned(),
            (Some(Quality::Excellent | Quality::Good), Some(why)) => {
                format!("{why}; calibrate Z again")
            }
            _ => "the last Z calibration was poor; calibrate Z again".to_owned(),
        })
    }
}
