// SPDX-License-Identifier: GPL-3.0-or-later
//! Head calibration belongs to the connected machine and the current material.

use crate::coordinator::Coordinator;
use crate::{Error, Result};
use openlaser_controller::State;
use openlaser_controller::session::Quality;
use serde::Serialize;

/// Material context recorded when the calibration is requested.
#[derive(Clone, Debug)]
pub(crate) struct Calibration {
    epoch: u64,
    binding: u64,
    material: Option<(String, u64)>,
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
        })
    }

    fn matches(&self, c: &Coordinator, state: &State) -> bool {
        state.configuration.is_some_and(|configuration| {
            configuration.epoch == self.epoch && configuration.binding == self.binding
        }) && self.material == material(c)
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
    /// That calibration still applies to the current material and bindings.
    pub current: bool,
}

impl Coordinator {
    pub(crate) fn update_calibration(&mut self, state: &State) {
        if self.calibration.as_ref().is_some_and(|calibration| {
            material(self).is_some() && !calibration.matches(self, state)
        }) {
            self.calibration = None;
        }
    }

    pub(crate) fn calibration_view(&self, state: &State) -> CalibrationView {
        let quality = state.session.calibration.filter(|_| {
            state.feedback.as_ref().is_some_and(|f| f.head.referenced && f.age_ms <= 1000)
        });
        let current = matches!(quality, Some(Quality::Excellent | Quality::Good))
            && self
                .calibration
                .as_ref()
                .is_some_and(|calibration| calibration.matches(self, state));
        CalibrationView { quality, current }
    }
}
