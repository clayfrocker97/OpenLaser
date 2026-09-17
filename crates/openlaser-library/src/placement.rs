// SPDX-License-Identifier: GPL-3.0-or-later

//! Saved positioning intent. A captured head position belongs to an execution,
//! never to a reusable job.

use serde::{Deserialize, Serialize};

/// How a new physical sheet is positioned on the machine.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    /// Capture the live head for each new cut. No machine coordinates are saved.
    Head {},
    /// Reuse a fixture position in the machine's homed coordinate system.
    Fixed {
        /// Machine XY occupied by the sheet's reference point, in millimetres.
        origin: [f64; 2],
    },
}

impl Default for Placement {
    fn default() -> Self {
        Self::Head {}
    }
}

impl Placement {
    /// Reject malformed saved or incoming fixture coordinates.
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Fixed { origin } = self
            && origin.iter().any(|v| !v.is_finite() || v.abs() > 100_000.)
        {
            return Err("fixture XY must be within 100000 mm".into());
        }
        Ok(())
    }
}
