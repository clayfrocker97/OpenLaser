// SPDX-License-Identifier: GPL-3.0-or-later

//! A completion checklist belongs to the ended execution, never to an edited draft.

use crate::coordinator::{Coordinator, Shared};
use crate::{Error, Result};
use openlaser_controller::session::Configuration;
use openlaser_core::LaserMode;
use openlaser_library::preflight::{Check, CheckAction, Checklist};
use serde::{Deserialize, Serialize};

/// Mode-specific checklists offered after a completed or stopped job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostflightDefaults {
    /// Fiber completion checks.
    pub fiber: Checklist,
    /// CO2 completion checks.
    pub co2: Checklist,
}

impl Default for PostflightDefaults {
    fn default() -> Self {
        let checklist = Checklist {
            enabled: true,
            steps: vec![
                Check {
                    text: "Park the head".into(),
                    action: Some(CheckAction::MoveTo {
                        point: openlaser_library::Anchor::FrontLeft,
                    }),
                    auto_check: true,
                },
                Check { text: "Inspect and remove parts".into(), action: None, auto_check: false },
            ],
        };
        Self { fiber: checklist.clone(), co2: checklist }
    }
}

impl PostflightDefaults {
    /// Selects the finished job's mode.
    #[must_use]
    pub const fn checklist(&self, mode: LaserMode) -> &Checklist {
        match mode {
            LaserMode::Fiber => &self.fiber,
            LaserMode::Co2 => &self.co2,
        }
    }

    /// Postflight offers parking moves and visual checks only.
    pub fn validate(&self) -> Result<()> {
        for checklist in [&self.fiber, &self.co2] {
            openlaser_library::preflight::validate(&checklist.steps)?;
            if checklist.steps.iter().any(|s| {
                !matches!(
                    s.action,
                    None | Some(CheckAction::MoveTo { .. } | CheckAction::MoveXy { .. })
                )
            }) {
                return Err(Error::Request("postflight actions must be bed or XY moves".into()));
            }
        }
        Ok(())
    }
}

/// A pending completion, included in the event stream for every open client.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct PostflightNotice {
    /// Unique to this completion and server session.
    pub id: String,
    /// Name of the job that completed.
    pub name: String,
}

/// Current completion checks. Checkbox state is never persisted.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct PostflightReview {
    /// Completion identity.
    pub notice: PostflightNotice,
    /// Defaults captured when the job finished.
    pub steps: Vec<Check>,
    /// Checks satisfied by fresh machine feedback.
    pub satisfied: Vec<usize>,
}

pub(crate) struct Pending {
    notice: PostflightNotice,
    steps: Vec<Check>,
    configuration: Configuration,
    execution: u64,
}

impl Coordinator {
    pub(crate) fn completed_postflight(&mut self) {
        let Some(held) = &self.held else { return };
        let Some(execution) = self.execution.as_ref().filter(|e| !e.frame) else { return };
        let checklist = self.preflight.postflight.checklist(held.configuration.mode);
        self.postflight = (checklist.enabled && !checklist.steps.is_empty()).then(|| Pending {
            notice: PostflightNotice {
                id: format!("{}-{}", self.history.session, execution.id),
                name: execution.name.clone(),
            },
            steps: checklist.steps.clone(),
            configuration: held.configuration,
            execution: execution.id,
        });
    }

    pub(crate) fn postflight_notice(&self) -> Option<PostflightNotice> {
        self.postflight.as_ref().map(|p| p.notice.clone())
    }

    fn current_postflight(&self, id: &str) -> Result<&Pending> {
        self.postflight
            .as_ref()
            .filter(|p| p.notice.id == id)
            .ok_or_else(|| Error::Refused("this postflight checklist is no longer current".into()))
    }

    pub(crate) fn postflight_target(&self, id: &str, step: usize) -> Result<[f64; 2]> {
        let pending = self.current_postflight(id)?;
        if self.acceptance()? != pending.configuration
            || self.execution.as_ref().map(|e| e.id) != Some(pending.execution)
        {
            return Err(Error::Refused(
                "the machine or execution changed; close postflight".into(),
            ));
        }
        match pending.steps.get(step).and_then(|s| s.action.as_ref()) {
            Some(CheckAction::MoveTo { point }) => self.bed_point(*point),
            Some(CheckAction::MoveXy { x, y }) => Ok([*x, *y]),
            _ => Err(Error::Request("this check has no parking action".into())),
        }
    }
}

/// Reads completion checks without operating the machine.
pub async fn review(shared: &Shared) -> Option<PostflightReview> {
    let c = shared.lock().await;
    let pending = c.postflight.as_ref()?;
    let state = c.machine.state();
    let satisfied = pending
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| {
            (step.auto_check && step.action.as_ref().is_some_and(|a| c.action_satisfied(a, &state)))
                .then_some(i)
        })
        .collect();
    Some(PostflightReview {
        notice: pending.notice.clone(),
        steps: pending.steps.clone(),
        satisfied,
    })
}

/// Closes a completion checklist, without changing any defaults or doing motion.
pub async fn dismiss(shared: &Shared, id: &str) -> Result<()> {
    let mut c = shared.lock().await;
    c.current_postflight(id)?;
    c.postflight = None;
    c.publish();
    Ok(())
}

/// Performs only an explicitly selected parking action for this completion.
pub async fn action(shared: &Shared, id: &str, step: usize) -> Result<()> {
    crate::machine::go_postflight(shared, id, step).await
}
