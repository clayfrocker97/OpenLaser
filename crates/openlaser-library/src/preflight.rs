// SPDX-License-Identifier: GPL-3.0-or-later

//! Operator checklists saved with jobs and application preferences.

use crate::{Anchor, Error, Result};
use serde::{Deserialize, Serialize};

/// A short operator check, optionally with an explicitly invoked setup action.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Text beside the checkbox.
    pub text: String,
    /// An available action; never executed by checking the box.
    pub action: Option<CheckAction>,
    /// Check automatically only while the current state satisfies the action.
    #[serde(default)]
    pub auto_check: bool,
}

/// The only machine operations a checklist can offer.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckAction {
    /// Establish the machine reference.
    Home {},
    /// Move to the saved job origin.
    Origin {},
    /// Set the job's bottom-left origin at the stationary head; no motion.
    SetOrigin {},
    /// Align this point of the job with the same point on the bed; no motion.
    SetOriginAt {
        /// Point on both the job and bed bounds.
        point: Anchor,
    },
    /// Move the head to a point on the bed, with the laser off.
    MoveTo {
        /// Point on the bed bounds.
        point: Anchor,
    },
    /// Move to explicit machine coordinates, checked against the bed limits.
    MoveXy {
        /// X coordinate in millimetres.
        x: f64,
        /// Y coordinate in millimetres.
        y: f64,
    },
    /// Calibrate the height controller.
    Calibrate {},
    /// Test the gas selections and pressures used by the compiled job.
    JobGasTest {
        /// Maximum time for each test, 50 to 2000 milliseconds.
        duration_ms: u64,
    },
    /// Briefly open a configured gas valve; always closes automatically.
    GasTest {
        /// Native gas selector, 0 to 5.
        selector: u8,
        /// Pressure in bar for a proportional valve.
        pressure: f64,
        /// Maximum on time, 50 to 2000 milliseconds.
        duration_ms: u64,
    },
}

/// A mode's default checklist.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
    /// Whether the checks are required before Start.
    pub enabled: bool,
    /// Checks in display order.
    pub steps: Vec<Check>,
}

impl Default for Checklist {
    fn default() -> Self {
        Self {
            enabled: true,
            steps: [
                "Material and thickness match the recipe",
                "Focus, lens and nozzle are checked",
                "Material is flat and the cutting path is clear",
                "Extraction is on and the work area is clear",
            ]
            .into_iter()
            .map(|text| Check { text: text.into(), action: None, auto_check: false })
            .chain(std::iter::once(Check::gas()))
            .collect(),
        }
    }
}

/// How a saved job chooses its checks. Old jobs inherit the mode defaults.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JobPreflight {
    /// Use the current defaults for the job's laser mode.
    Inherit {},
    /// Omit this job's checklist.
    Off {},
    /// Checks specific to this job.
    Custom {
        /// Checks in display order.
        steps: Vec<Check>,
    },
}

/// Validates both incoming edits and persisted checklists.
pub fn validate(steps: &[Check]) -> Result<()> {
    if steps.len() > 50 {
        return Err(Error::Invalid("use at most 50 preflight checks".into()));
    }
    for step in steps {
        step.validate()?;
    }
    Ok(())
}

impl Check {
    /// The editable default gas check, resolved against the compiled job.
    #[must_use]
    pub fn gas() -> Self {
        Self {
            text: "Gas supply is ready".into(),
            action: Some(CheckAction::JobGasTest { duration_ms: 500 }),
            auto_check: false,
        }
    }

    /// Whether this check confirms the job's gas supplies.
    #[must_use]
    pub const fn checks_gas(&self) -> bool {
        matches!(self.action, Some(CheckAction::JobGasTest { .. } | CheckAction::GasTest { .. }))
    }

    fn validate(&self) -> Result<()> {
        if self.text.trim().is_empty() || self.text.len() > 500 || self.text.contains('\0') {
            return Err(Error::Invalid("each preflight check needs 1 to 500 bytes of text".into()));
        }
        if self.auto_check
            && matches!(
                self.action,
                None | Some(CheckAction::GasTest { .. } | CheckAction::JobGasTest { .. })
            )
        {
            return Err(Error::Invalid(
                "visual checks and gas flow require manual confirmation".into(),
            ));
        }
        if let Some(action) = &self.action {
            action.validate()?;
        }
        Ok(())
    }
}

impl CheckAction {
    fn validate(&self) -> Result<()> {
        match *self {
            Self::MoveXy { x, y } if !x.is_finite() || !y.is_finite() => {
                Err(Error::Invalid("XY coordinates must be finite".into()))
            }
            Self::JobGasTest { duration_ms } if !(50..=2000).contains(&duration_ms) => {
                Err(Error::Invalid("a gas test needs 50 to 2000 ms".into()))
            }
            Self::GasTest { selector, pressure, duration_ms } => {
                validate_gas(selector, pressure, duration_ms)
            }
            _ => Ok(()),
        }
    }
}

fn validate_gas(selector: u8, pressure: f64, duration_ms: u64) -> Result<()> {
    if !(selector <= 5 && 0. < pressure && pressure <= 100. && (50..=2000).contains(&duration_ms)) {
        return Err(Error::Invalid(
            "a gas test needs a gas from 0 to 5, pressure above 0 up to 100 bar, and 50 to 2000 ms"
                .into(),
        ));
    }
    Ok(())
}

impl Default for JobPreflight {
    fn default() -> Self {
        Self::Inherit {}
    }
}

impl JobPreflight {
    /// Checks this job's stored policy.
    pub fn validate(&self) -> Result<()> {
        if let Self::Custom { steps } = self {
            validate(steps)?;
        }
        Ok(())
    }

    /// Resolves the job's policy against its mode defaults.
    #[must_use]
    pub fn steps<'a>(&'a self, defaults: &'a Checklist) -> &'a [Check] {
        match self {
            Self::Inherit {} if defaults.enabled => &defaults.steps,
            Self::Custom { steps } => steps,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_actions_cannot_execute_arbitrary_commands_or_unbounded_gas() {
        let raw = r#"{"text":"Setup","action":{"kind":"shell","command":"anything"}}"#;
        assert!(serde_json::from_str::<Check>(raw).is_err());
        let extra = r#"{"text":"Setup","action":{"kind":"home","code":"anything"}}"#;
        assert!(serde_json::from_str::<Check>(extra).is_err());
        let mut gas = Check {
            text: "Gas flow".into(),
            action: Some(CheckAction::GasTest { selector: 0, pressure: 3., duration_ms: 2001 }),
            auto_check: false,
        };
        assert!(validate(&[gas.clone()]).is_err());
        gas.action = Some(CheckAction::GasTest { selector: 5, pressure: 3., duration_ms: 2000 });
        assert!(validate(&[gas]).is_ok());
    }
}
