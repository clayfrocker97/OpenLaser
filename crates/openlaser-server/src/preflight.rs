// SPDX-License-Identifier: GPL-3.0-or-later

//! Fresh operator confirmations bound to the job, settings and machine state.
//! Saved policies contain checks, never completed checkboxes or permission to run.

use crate::coordinator::{Coordinator, Shared};
use crate::{Error, Result};
use openlaser_core::LaserMode;
use openlaser_library::preflight::{Check, CheckAction, Checklist, JobPreflight};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Application-wide preflight choices, persisted independently of job files.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightPreferences {
    /// File schema version.
    pub version: u32,
    /// Optimistic concurrency token for preference edits.
    pub revision: u64,
    /// Fiber defaults.
    pub fiber: Checklist,
    /// CO2 defaults.
    pub co2: Checklist,
    /// Legacy version-1 setting, migrated into editable checklist steps.
    #[serde(default)]
    pub confirm_gas: bool,
    /// Checklists offered after confirmed job completion.
    #[serde(default)]
    pub postflight: crate::postflight::PostflightDefaults,
    /// Checks offered after pausing and required before resuming.
    #[serde(default)]
    pub pause: PauseDefaults,
}

/// Editable pause checks for each laser mode.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseDefaults {
    /// Fiber pause checks.
    pub fiber: Checklist,
    /// CO2 pause checks.
    pub co2: Checklist,
}

impl Default for PauseDefaults {
    fn default() -> Self {
        let list = Checklist {
            enabled: true,
            steps: vec![
                Check {
                    text: "Debris is clear of the cutting path".into(),
                    action: None,
                    auto_check: false,
                },
                Check {
                    text: "Material and clamps have not moved".into(),
                    action: None,
                    auto_check: false,
                },
                Check::gas(),
            ],
        };
        Self { fiber: list.clone(), co2: list }
    }
}

impl PauseDefaults {
    /// Selects the retained job's mode.
    #[must_use]
    pub const fn checklist(&self, mode: LaserMode) -> &Checklist {
        match mode {
            LaserMode::Fiber => &self.fiber,
            LaserMode::Co2 => &self.co2,
        }
    }

    fn validate(&self) -> Result<()> {
        for list in [&self.fiber, &self.co2] {
            openlaser_library::preflight::validate(&list.steps)?;
            if list.steps.iter().any(|step| {
                !matches!(
                    step.action,
                    None | Some(CheckAction::GasTest { .. } | CheckAction::JobGasTest { .. })
                )
            }) {
                return Err(Error::Request("pause checklist actions must be gas tests".into()));
            }
        }
        Ok(())
    }
}

impl Default for PreflightPreferences {
    fn default() -> Self {
        Self {
            version: 2,
            revision: 0,
            fiber: Checklist::default(),
            co2: Checklist::default(),
            confirm_gas: false,
            postflight: crate::postflight::PostflightDefaults::default(),
            pause: PauseDefaults::default(),
        }
    }
}

impl PreflightPreferences {
    /// Loads preferences, refusing malformed files rather than silently disabling checks.
    pub fn open(root: &Path) -> Result<Self> {
        let bytes = match std::fs::read(root.join("preflight.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(Error::Refused(format!("preflight preferences: {error}"))),
        };
        let mut preferences: Self = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Refused(format!("preflight preferences: {e}")))?;
        preferences.migrate();
        preferences.validate()?;
        Ok(preferences)
    }

    /// Validates every mode, including a temporarily disabled checklist.
    pub fn validate(&self) -> Result<()> {
        if self.version != 2 {
            return Err(Error::Request("unsupported preflight preferences".into()));
        }
        openlaser_library::preflight::validate(&self.fiber.steps)?;
        openlaser_library::preflight::validate(&self.co2.steps)?;
        self.postflight.validate()?;
        self.pause.validate()?;
        Ok(())
    }

    fn migrate(&mut self) {
        if self.version == 1 {
            if self.confirm_gas {
                for list in [&mut self.fiber, &mut self.co2] {
                    if !list.steps.iter().any(Check::checks_gas) {
                        list.steps.push(Check::gas());
                    }
                }
            }
            self.version = 2;
        }
        // Version 2 has one source of truth: the normal editable steps.
        self.confirm_gas = false;
    }

    /// Defaults for one laser mode.
    #[must_use]
    pub const fn checklist(&self, mode: LaserMode) -> &Checklist {
        match mode {
            LaserMode::Fiber => &self.fiber,
            LaserMode::Co2 => &self.co2,
        }
    }
}

/// The operation the operator is reviewing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightIntent {
    /// A new run requires this job's checklist.
    Run,
    /// A retained continuation requires the pause checklist.
    Resume,
}

/// The exact checks displayed before an operation.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct PreflightReview {
    /// Freshness token, invalid after relevant changes or application restart.
    pub token: String,
    /// What the confirmation admits.
    pub intent: PreflightIntent,
    /// Checks in order; indexes are their IDs within this review.
    pub steps: Vec<Check>,
    /// Opted-in checks satisfied by current, fresh machine or job state.
    pub satisfied: Vec<usize>,
    /// Required gases from the compiled cutting, piercing, residue and film passes.
    pub gases: Vec<String>,
    /// Whether a gas-readiness checkbox is required.
    pub confirm_gas: bool,
    /// Whether the compiled program has its laser off.
    pub dry_run: bool,
}

/// A fresh operator response. Never persisted or reused for a later run.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightConfirmation {
    /// Token from the review shown to the operator.
    pub token: String,
    /// Index of each checked step.
    pub checked: Vec<usize>,
    /// Operator confirmation of the displayed gas supplies.
    pub gas_ready: bool,
}

impl Coordinator {
    /// Saves mode defaults with revision checking.
    pub fn save_preflight_preferences(&mut self, mut next: PreflightPreferences) -> Result<()> {
        next.migrate();
        next.validate()?;
        if next.revision != self.preflight.revision {
            return Err(Error::Refused(
                "preflight defaults changed elsewhere; reopen the editor".into(),
            ));
        }
        next.revision += 1;
        let bytes = serde_json::to_vec_pretty(&next).map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&self.config.data_dir.join("preflight.json"), &bytes)?;
        self.preflight = next;
        self.publish();
        Ok(())
    }

    /// Edits the draft's checklist policy, retaining the compiled geometry.
    pub fn set_preflight(&mut self, policy: JobPreflight) -> Result<()> {
        policy.validate()?;
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        draft.remember();
        draft.current.preflight = policy;
        self.draft_changed();
        Ok(())
    }

    /// Builds a review from the actual immutable program about to execute.
    pub fn preflight_review(
        &self,
        intent: PreflightIntent,
        stop_epoch: u64,
    ) -> Result<PreflightReview> {
        let (job, dry_run, steps) = match intent {
            PreflightIntent::Run => {
                let draft = self
                    .draft
                    .as_ref()
                    .ok_or_else(|| Error::Refused("open a part first".into()))?;
                let compiled = draft
                    .compiled
                    .as_ref()
                    .ok_or_else(|| Error::Refused("compile the job first".into()))?;
                let defaults = self.preflight.checklist(compiled.job.settings.mode);
                (&compiled.job, compiled.dry_run, draft.current.preflight.steps(defaults).to_vec())
            }
            PreflightIntent::Resume => {
                let recovery = self
                    .recovery
                    .as_ref()
                    .ok_or_else(|| Error::Refused("no retained job".into()))?;
                let defaults = self.preflight.pause.checklist(recovery.original.configuration.mode);
                let mut steps = if defaults.enabled { defaults.steps.clone() } else { Vec::new() };
                if recovery.state != openlaser_controller::state::ProgramState::Held {
                    steps.insert(0, Check { text: "The retained job still matches the stock position and the selected restart path is clear".into(), action: None, auto_check: false });
                }
                (&recovery.original.job, recovery.original.dry_run, steps)
            }
        };
        let steps: Vec<_> = resolve_gas_checks(&steps, job)
            .into_iter()
            .map(|step| {
                if matches!(
                    step.action,
                    Some(CheckAction::SetOrigin {} | CheckAction::SetOriginAt { .. })
                ) {
                    Check {
                        text: "Sheet position matches the material".into(),
                        action: None,
                        auto_check: false,
                    }
                } else {
                    step
                }
            })
            .collect();
        let gases = if dry_run { Vec::new() } else { required_gases(job) };
        // Gas confirmation is part of `steps`; retain this response field for
        // older clients without creating a second checkbox below the checklist.
        let confirm_gas = false;
        let state = self.machine.state();
        let satisfied: Vec<_> = steps
            .iter()
            .enumerate()
            .filter(|(_, step)| {
                step.auto_check
                    && step
                        .action
                        .as_ref()
                        .is_some_and(|action| self.action_satisfied(action, &state))
            })
            .map(|(index, _)| index)
            .collect();
        // A stable context, unlike the feedback publication revision which
        // changes every poll. Position binds setup actions and origin review.
        let context = format!(
            "{}|{:?}|{}|{}|{:?}|{:?}|{:?}|{:?}|{}|{:?}|{:?}",
            self.history.session,
            intent,
            stop_epoch,
            self.preflight_stamp(),
            state.configuration,
            state.session,
            state.feedback.as_ref().map(|f| f.position_mm),
            state.program.as_ref().map(|p| (&p.state, &p.checkpoint)),
            state.alarm_revision,
            steps,
            (self.preflight.revision, confirm_gas, &gases, &satisfied)
        );
        Ok(PreflightReview {
            token: openlaser_library::sha256(context.as_bytes()),
            intent,
            steps,
            satisfied,
            gases,
            confirm_gas,
            dry_run,
        })
    }

    pub(crate) fn action_satisfied(
        &self,
        action: &CheckAction,
        state: &openlaser_controller::State,
    ) -> bool {
        use openlaser_library::Anchor;
        let Some(draft) = &self.draft else { return false };
        // Position comparisons use the same one-pulse tolerance as travel. No action is
        // considered complete while feedback is old or any operation is active.
        let Some(feedback) = state
            .feedback
            .as_ref()
            .filter(|f| f.age_ms <= 1000 && f.scale > 0 && f.stationary && f.head.command == 0)
        else {
            return false;
        };
        let Ok(configuration) = self.acceptance() else { return false };
        if self.idle().is_err() || state.alarms.iter().any(|alarm| alarm.blocking) {
            return false;
        }
        let banks = &configuration.verified.banks[..2];
        if banks.iter().any(|bank| bank[9] == 0 || bank[10] == 0) {
            return false;
        }
        let near = |a: [f64; 2], b: [f64; 2]| {
            a.into_iter().zip(b).zip(banks).all(|((a, b), bank)| {
                ((a * f64::from(feedback.scale)).round() - (b * f64::from(feedback.scale)).round())
                    .abs()
                    <= f64::from(bank[9].div_ceil(bank[10]))
            })
        };
        let head = [feedback.position_mm[0], feedback.position_mm[1]];
        let referenced = state.session.homed && feedback.referenced == [true, true];
        match action {
            CheckAction::Home {} => referenced && feedback.head.referenced,
            CheckAction::Origin {} => {
                referenced && draft.origin().is_some_and(|origin| near(head, origin))
            }
            CheckAction::MoveTo { point } => {
                referenced && self.bed_point(*point).is_ok_and(|target| near(head, target))
            }
            CheckAction::MoveXy { x, y } => referenced && near(head, [*x, *y]),
            CheckAction::SetOrigin {} => {
                draft.current.anchor == Anchor::FrontLeft
                    && draft.origin().is_some_and(|origin| near(origin, head))
            }
            CheckAction::SetOriginAt { point } => {
                draft.current.anchor == *point
                    && draft
                        .origin()
                        .zip(self.bed_point(*point).ok())
                        .is_some_and(|(origin, target)| near(origin, target))
            }
            CheckAction::Calibrate {} => {
                feedback.head.referenced && self.calibration_view(state).current
            }
            CheckAction::GasTest { .. } | CheckAction::JobGasTest { .. } => false,
        }
    }

    /// Checks the operator response before reserving or constructing a run.
    pub(crate) fn check_preflight(
        &self,
        intent: PreflightIntent,
        stop_epoch: u64,
        confirmation: Option<&PreflightConfirmation>,
    ) -> Result<()> {
        let review = self.preflight_review(intent, stop_epoch)?;
        if review.steps.is_empty() && !review.confirm_gas && confirmation.is_none() {
            return Ok(());
        }
        let response = confirmation
            .ok_or_else(|| Error::Refused("complete preflight before starting".into()))?;
        if response.token != review.token {
            return Err(Error::Refused(
                "the job or machine changed; review preflight again".into(),
            ));
        }
        let checked: BTreeSet<_> = response.checked.iter().copied().collect();
        if checked.len() != response.checked.len() || checked != (0..review.steps.len()).collect() {
            return Err(Error::Refused("complete every preflight check before starting".into()));
        }
        if review.confirm_gas && !response.gas_ready {
            return Err(Error::Refused("confirm the required gas supplies are ready".into()));
        }
        Ok(())
    }
}

/// Gets the current review with the same stop/restart boundary as the API.
pub async fn review(shared: &Shared, intent: PreflightIntent) -> Result<PreflightReview> {
    let epoch = shared.ensure_running()?;
    shared.lock().await.preflight_review(intent, epoch)
}

/// Gas supplies actually used by the compiled program, deduplicated by selector.
#[must_use]
pub fn required_gases(job: &openlaser_compiler::program::Job) -> Vec<String> {
    required_gas_tests(job)
        .into_iter()
        .map(|(gas, _)| crate::recipes::gas_name(&gas.to_string()))
        .collect()
}

/// Unique native gas selections; test at the highest pressure used by each.
/// High-pressure selections retain their regulator pressure and native routing.
fn required_gas_tests(job: &openlaser_compiler::program::Job) -> Vec<(u8, f64)> {
    let mut gases = std::collections::BTreeMap::<u8, f64>::new();
    for settings in std::iter::once(&job.settings).chain(job.passes.iter().map(|p| &p.settings)) {
        if settings.dry_run || !settings.hardware.as_ref().is_some_and(|h| h.gas_enabled) {
            continue;
        }
        let pairs = std::iter::once((settings.gas, settings.pressure))
            .chain(settings.pierce.iter().map(|p| (p.gas, p.pressure)))
            .chain(settings.residue.iter().map(|r| (r.gas, r.pressure)));
        for (selector, pressure) in pairs {
            gases.entry(selector).and_modify(|p| *p = p.max(pressure)).or_insert(pressure);
        }
    }
    gases.into_iter().collect()
}

fn resolve_gas_checks(steps: &[Check], job: &openlaser_compiler::program::Job) -> Vec<Check> {
    let gases = required_gas_tests(job);
    steps
        .iter()
        .flat_map(|step| match step.action {
            Some(
                CheckAction::JobGasTest { duration_ms } | CheckAction::GasTest { duration_ms, .. },
            ) => gases
                .iter()
                .map(|&(selector, pressure)| Check {
                    text: format!(
                        "{} · {}",
                        step.text,
                        crate::recipes::gas_name(&selector.to_string())
                    ),
                    action: Some(CheckAction::GasTest { selector, pressure, duration_ms }),
                    auto_check: false,
                })
                .collect(),
            _ => vec![step.clone()],
        })
        .collect()
}

/// Starts only an action offered by the current checklist. Checking a box
/// itself never invokes this endpoint or performs motion.
pub async fn action(shared: &Shared, token: &str, step: usize) -> Result<()> {
    let epoch = shared.ensure_running()?;
    let action = {
        let coordinator = shared.lock().await;
        // The token binds the intent as well as the exact job and machine
        // state, so a resume action must match its own current review.
        let review = [PreflightIntent::Run, PreflightIntent::Resume]
            .into_iter()
            .filter_map(|intent| coordinator.preflight_review(intent, epoch).ok())
            .find(|review| token == review.token)
            .ok_or_else(|| Error::Refused("preflight changed; reopen the checklist".into()))?;
        review
            .steps
            .get(step)
            .and_then(|s| s.action.clone())
            .ok_or_else(|| Error::Request("this check has no action".into()))?
    };
    machine_action(shared, action).await
}

pub(crate) async fn machine_action(shared: &Shared, action: CheckAction) -> Result<()> {
    match action {
        CheckAction::Home {} => crate::machine::home(shared).await,
        CheckAction::Origin {} => crate::machine::go_origin(shared, false).await,
        CheckAction::MoveTo { point } => crate::machine::go_bed_point(shared, point).await,
        CheckAction::MoveXy { x, y } => crate::machine::go_xy(shared, [x, y]).await,
        CheckAction::JobGasTest { .. } => {
            Err(Error::Request("gas test was not resolved from the job".into()))
        }
        CheckAction::SetOrigin {} | CheckAction::SetOriginAt { .. } => {
            Err(Error::Request("set sheet position on the Run tab".into()))
        }
        CheckAction::Calibrate {} => crate::machine::calibrate(shared).await,
        CheckAction::GasTest { selector, pressure, duration_ms } => {
            crate::machine::gas_test(shared, selector, pressure, duration_ms).await
        }
    }
}
