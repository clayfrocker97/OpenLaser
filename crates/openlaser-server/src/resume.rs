// SPDX-License-Identifier: GPL-3.0-or-later

//! Continuation from settled XY feedback. The native progress field's unit
//! is unresolved and is not used as a sample index. This host projects onto
//! the compiled execution polyline, with the recovered strict 0.2 mm limit.

use crate::coordinator::{Coordinator, Held};
use crate::document::{ExecutionView, PassView};
use crate::{Error, Result};
use openlaser_compiler::pass::PassKind;
use openlaser_compiler::program::Job;
use openlaser_controller::session::Configuration;
use openlaser_controller::state::ProgramState;
use openlaser_core::geometry::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// How close, in millimetres, the stopped head must lie to the execution
/// path for a continuation to find its place: the strict limit recovered
/// from the vendor host (see the module documentation).
const RESTART_MATCH_MM: f64 = 0.2;
/// Longest step, in millimetres, the operator may move a restart point
/// along the path at once; a bound on the request, not a machine limit.
const MAX_RECOVERY_DISTANCE_MM: f64 = 10_000.;
use std::sync::Arc;

/// Where a held program stopped within the current remainder.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Pass index in this execution, distinct from its original ordinal.
    pub pass: usize,
    /// Distance fraction of its execution path; a point is before or after.
    pub fraction: f64,
}

/// The checkpoint from the item tag and settled position in job coordinates.
pub fn locate(job: &Job, item: u32, position: [f64; 2]) -> Result<Checkpoint> {
    if item >= u32::MAX - 1 || position.iter().any(|v| !v.is_finite()) {
        return Err(Error::Refused("the program stopped outside its passes".into()));
    }
    let pass =
        usize::try_from(item & 0x7fff_ffff).map_err(|_| Error::Refused("item tag".into()))?;
    let compiled =
        job.passes.get(pass).ok_or_else(|| Error::Refused("the item tag names no pass".into()))?;
    if item & 0x8000_0000 != 0 {
        return Ok(Checkpoint { pass, fraction: 0. });
    }
    if compiled.pass.kind == PassKind::PrePierce {
        return Ok(Checkpoint { pass, fraction: 1. });
    }
    let p = Point::from(position);
    let mut total = 0.;
    let mut best: Option<(f64, f64)> = None;
    let mut ambiguous = false;
    for pair in compiled.pieces.iter().flat_map(|piece| piece.plan.points.windows(2)) {
        let a = Point::from(pair[0]);
        let b = Point::from(pair[1]);
        let delta = b - a;
        let length = delta.norm();
        if length <= 1e-12 {
            continue;
        }
        let t = ((p - a).dot(delta) / (length * length)).clamp(0., 1.);
        let error = p.distance(a.lerp(b, t));
        let distance = total + t * length;
        if error < RESTART_MATCH_MM {
            match best {
                None => {
                    best = Some((error, distance));
                    ambiguous = false;
                }
                Some((prior, _)) if error < prior - 1e-9 => {
                    best = Some((error, distance));
                    ambiguous = false;
                }
                Some((prior, prior_distance))
                    if (error - prior).abs() <= 1e-9
                        && (distance - prior_distance).abs() > 1e-7 =>
                {
                    ambiguous = true;
                }
                _ => {}
            }
        }
        total += length;
    }
    let (_, distance) = best.ok_or_else(|| {
        Error::Refused(format!(
            "the stopped position is not within {RESTART_MATCH_MM} mm of the execution path"
        ))
    })?;
    if ambiguous {
        return Err(Error::Refused(
            "the stopped position matches more than one place on the execution path".into(),
        ));
    }
    if total <= 0. {
        return Err(Error::Refused("the pass has no motion to resume".into()));
    }
    Ok(Checkpoint { pass, fraction: (distance / total).clamp(0., 1.) })
}

/// Keeps the exact path remainder and remaining pass identities.
pub fn continuation(job: &Job, checkpoint: Checkpoint, resume_pierce: bool) -> Result<Job> {
    job.continuation(checkpoint.pass, checkpoint.fraction, resume_pierce).map_err(Error::from)
}

/// A selection change never sends a machine command.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryChange {
    /// Choose a pass of the original execution.
    Select {
        /// The original pass index.
        pass: usize,
        /// Fraction of its executed path.
        fraction: f64,
    },
    /// Previous included pass.
    Previous,
    /// Next included pass.
    Next,
    /// Walk backwards, excluding travel between passes.
    Backward {
        /// Distance in millimetres.
        distance: f64,
    },
    /// Walk forwards, excluding travel between passes.
    Forward {
        /// Distance in millimetres.
        distance: f64,
    },
    /// Skip every pass over the selected physical contour instances.
    Skip,
    /// Include all skipped instances again.
    IncludeAll,
    /// Select the last settled pause position.
    LastPause,
    /// Mark the selected point as known good.
    MarkGood,
    /// Recall the known-good point.
    LastGood,
}

/// Retained execution evidence and operator choices. It lives only in this
/// server session; persisted drawing drafts never restore machine authority.
#[derive(Clone, Debug)]
pub struct Recovery {
    /// The original immutable program.
    pub original: Arc<Held>,
    /// Its original display and physical identities.
    pub execution: Arc<ExecutionView>,
    /// Exact machine file identity used by the original program.
    pub machine_hash: String,
    /// A fresh accepted configuration, after stopped-job preparation.
    pub configuration: Configuration,
    /// Whether the selected remainder has been admitted for review.
    pub ready: bool,
    /// Current stopped/running state of this execution.
    pub state: ProgramState,
    /// Revision of recovery choices and evidence.
    pub revision: u64,
    selected: Option<Checkpoint>,
    last_pause: Option<Checkpoint>,
    pause_position: Option<[f64; 2]>,
    pause_cutting: bool,
    good: Option<Checkpoint>,
    skipped: BTreeSet<(usize, u32)>,
    executed: Vec<Vec<[f64; 2]>>,
    // The original paths are immutable. Keep document updates proportional to
    // the number of passes instead of re-walking millions of interpolation points.
    lengths: Vec<f64>,
    current: Arc<Held>,
    current_id: u64,
    stopped: Option<(u64, ProgramState, Option<openlaser_controller::state::CheckpointView>)>,
    problem: Option<String>,
}

/// One original pass and its observed execution intervals.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryStep {
    /// Original process and physical instances.
    pub pass: PassView,
    /// Length of its executed polyline; zero for preliminary points.
    pub length_mm: f64,
    /// Pending, partial, completed or skipped.
    pub status: String,
    /// Observed intervals as fractions of the original path.
    pub executed: Vec<[f64; 2]>,
}

/// Recovery state for the original job, independent of a mutable draft.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryView {
    /// Revision required when changing a selection.
    pub revision: u64,
    /// Original execution identity.
    pub id: u64,
    /// Original job name.
    pub name: String,
    /// Current program state.
    pub state: ProgramState,
    /// Whether stopped-job preparation is still needed.
    pub ready: bool,
    /// Selected original pass/fraction.
    pub selected: Option<Checkpoint>,
    /// Selected position in machine coordinates.
    pub position: Option<[f64; 2]>,
    /// Last settled pause position.
    pub last_pause: Option<Checkpoint>,
    /// Exact settled machine XY, retained while the operator moves the head.
    pub pause_position: Option<[f64; 2]>,
    /// Operator's known-good point.
    pub good: Option<Checkpoint>,
    /// Original pass statuses, including skipped physical contours.
    pub steps: Vec<RecoveryStep>,
    /// A checkpoint that could not be located, requiring explicit selection.
    pub problem: Option<String>,
}

fn length(pass: &openlaser_compiler::program::Compiled) -> f64 {
    pass.pieces
        .iter()
        .flat_map(|p| p.plan.points.windows(2))
        .map(|p| Point::from(p[0]).distance(Point::from(p[1])))
        .sum()
}

impl Recovery {
    pub(crate) fn new(
        original: Arc<Held>,
        execution: Arc<ExecutionView>,
        machine_hash: String,
    ) -> Self {
        Self {
            configuration: original.configuration,
            current: original.clone(),
            current_id: execution.id,
            executed: vec![Vec::new(); original.job.passes.len()],
            lengths: original.job.passes.iter().map(length).collect(),
            original,
            execution,
            machine_hash,
            ready: false,
            state: ProgramState::Running,
            revision: 1,
            selected: None,
            last_pause: None,
            pause_position: None,
            pause_cutting: false,
            good: None,
            skipped: BTreeSet::new(),
            stopped: None,
            problem: None,
        }
    }

    fn allowed(&self, pass: usize) -> bool {
        self.execution.compiled.plan.get(pass).is_some_and(|p| {
            !p.instances.iter().any(|i| self.skipped.contains(&(i.source, i.copy)))
        })
    }

    fn validate(&self, cp: Checkpoint) -> Result<()> {
        let pass = self
            .original
            .job
            .passes
            .get(cp.pass)
            .ok_or_else(|| Error::Request("no such original pass".into()))?;
        if !(0. ..=1.).contains(&cp.fraction) || !self.allowed(cp.pass) {
            return Err(Error::Request("choose a fraction from 0 to 1 on an included pass".into()));
        }
        if pass.pass.kind == PassKind::PrePierce && cp.fraction > 0. && cp.fraction < 1. {
            return Err(Error::Request("a pre-pierce point restarts at 0 or 100 percent".into()));
        }
        Ok(())
    }

    fn adjacent(&self, pass: usize, forward: bool) -> Option<usize> {
        if forward {
            (pass + 1..self.original.job.passes.len()).find(|&i| self.allowed(i))
        } else {
            (0..pass).rev().find(|&i| self.allowed(i))
        }
    }

    /// Whether a selected, included remainder is ready for fresh review.
    #[must_use]
    pub fn can_resume(&self) -> bool {
        self.ready
            && matches!(
                self.state,
                ProgramState::Held | ProgramState::Stopped | ProgramState::Failed
            )
            && self.selected.is_some_and(|cp| {
                self.allowed(cp.pass)
                    && (cp.fraction < 1. || self.adjacent(cp.pass, true).is_some())
            })
    }

    /// Change the selection without moving or compiling anything.
    pub fn change(&mut self, change: RecoveryChange) -> Result<()> {
        let current =
            self.selected.or(self.last_pause).unwrap_or(Checkpoint { pass: 0, fraction: 0. });
        let next = match change {
            RecoveryChange::Select { pass, fraction } => Some(Checkpoint { pass, fraction }),
            RecoveryChange::Previous | RecoveryChange::Next => Some(Checkpoint {
                pass: self
                    .adjacent(current.pass, matches!(change, RecoveryChange::Next))
                    .ok_or_else(|| Error::Refused("no further included pass".into()))?,
                fraction: 0.,
            }),
            RecoveryChange::LastPause => {
                Some(self.last_pause.ok_or_else(|| {
                    Error::Refused("no unambiguous settled pause position".into())
                })?)
            }
            RecoveryChange::LastGood => {
                Some(self.good.ok_or_else(|| Error::Refused("mark a good point first".into()))?)
            }
            RecoveryChange::MarkGood => {
                self.validate(current)?;
                self.good = Some(current);
                self.selected
            }
            RecoveryChange::IncludeAll => {
                self.skipped.clear();
                self.selected.or(self.last_pause).or(Some(current))
            }
            RecoveryChange::Skip => {
                self.validate(current)?;
                for instance in &self.execution.compiled.plan[current.pass].instances {
                    self.skipped.insert((instance.source, instance.copy));
                }
                self.adjacent(current.pass, true)
                    .or_else(|| self.adjacent(current.pass, false))
                    .map(|pass| Checkpoint { pass, fraction: 0. })
            }
            RecoveryChange::Forward { distance } | RecoveryChange::Backward { distance } => Some(
                self.offset(current, distance, matches!(change, RecoveryChange::Forward { .. }))?,
            ),
        };
        if let Some(cp) = next {
            self.validate(cp)?;
        }
        self.selected = next;
        self.problem = None;
        self.revision += 1;
        Ok(())
    }

    fn offset(&self, current: Checkpoint, distance: f64, forward: bool) -> Result<Checkpoint> {
        if !(distance.is_finite() && distance > 0. && distance <= MAX_RECOVERY_DISTANCE_MM) {
            return Err(Error::Request(
                "recovery distance must be above 0 and at most 10000 mm".into(),
            ));
        }
        self.validate(current)?;
        let mut next = current;
        let mut remaining = distance;
        loop {
            let span = self.lengths[next.pass];
            let available = span * if forward { 1. - next.fraction } else { next.fraction };
            if span > 0. && remaining <= available {
                next.fraction += if forward { remaining / span } else { -remaining / span };
                break;
            }
            remaining -= available;
            if let Some(pass) = self.adjacent(next.pass, forward) {
                next = Checkpoint { pass, fraction: if forward { 0. } else { 1. } };
            } else {
                next.fraction = if forward { 1. } else { 0. };
                break;
            }
        }
        Ok(next)
    }

    /// The selected point on the original executed polyline, in machine coordinates.
    #[must_use]
    pub fn position(&self, cp: Checkpoint) -> [f64; 2] {
        if self.last_pause == Some(cp)
            && let Some(position) = self.pause_position
        {
            return position;
        }
        let pass = &self.original.job.passes[cp.pass];
        let mut left = self.lengths[cp.pass] * cp.fraction;
        for pair in pass.pieces.iter().flat_map(|p| p.plan.points.windows(2)) {
            let a = Point::from(pair[0]);
            let b = Point::from(pair[1]);
            let span = a.distance(b);
            if span > 0. && left <= span {
                let p = a.lerp(b, left / span);
                return [p.x + self.original.sheet_offset[0], p.y + self.original.sheet_offset[1]];
            }
            left -= span;
        }
        let point = if cp.fraction == 0. { pass.start } else { pass.end };
        [point[0] + self.original.sheet_offset[0], point[1] + self.original.sheet_offset[1]]
    }

    fn record(&mut self, pass: usize, begin: f64, end: f64) {
        let (begin, end) = (begin.clamp(0., 1.), end.clamp(0., 1.));
        let intervals = &mut self.executed[pass];
        if end <= begin || intervals.iter().any(|p| p[0] <= begin + 1e-8 && p[1] >= end - 1e-8) {
            return;
        }
        intervals.push([begin, end]);
        intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut merged: Vec<[f64; 2]> = Vec::new();
        for range in intervals.iter().copied() {
            if let Some(last) = merged.last_mut()
                && last[1] + 1e-8 >= range[0]
            {
                last[1] = last[1].max(range[1]);
            } else {
                merged.push(range);
            }
        }
        *intervals = merged;
        self.revision += 1;
    }

    pub(crate) fn dispatched(&mut self, current: Arc<Held>, id: u64) {
        self.current = current;
        self.current_id = id;
        self.stopped = None;
        self.state = ProgramState::Running;
        self.ready = false;
        self.revision += 1;
    }

    /// Record only item-tag-confirmed passes and an unambiguous settled point.
    pub(crate) fn observe(
        &mut self,
        state: &openlaser_controller::State,
        execution: Option<&ExecutionView>,
    ) {
        if state.configuration != Some(self.configuration) && self.ready {
            self.ready = false;
            self.revision += 1;
        }
        if execution.is_none_or(|e| e.id != self.current_id) {
            self.lost_execution();
            return;
        }
        let Some(program) = &state.program else {
            self.lost_execution();
            return;
        };
        if self.state != program.state {
            self.state = program.state;
            self.revision += 1;
        }
        let completed = execution.and_then(|e| e.progress(state)).map_or(0, |p| p.completed);
        for i in 0..completed.min(self.current.job.passes.len()) {
            let current = &self.current.job.passes[i];
            if let Some(root) =
                self.original.job.passes.iter().position(|p| p.pass.ordinal == current.pass.ordinal)
            {
                let span = self.lengths[root];
                let begin = if span > 0. { current.consumed / span } else { 0. };
                self.record(root, begin, 1.);
            }
        }
        if !matches!(
            program.state,
            ProgramState::Held | ProgramState::Stopped | ProgramState::Failed
        ) || state.operation.is_some()
            || self.stopped == Some((self.current_id, program.state, program.checkpoint))
        {
            return;
        }
        self.stopped = Some((self.current_id, program.state, program.checkpoint));
        self.ready =
            program.state == ProgramState::Held && state.configuration == Some(self.configuration);
        let located = program
            .checkpoint
            .ok_or_else(|| {
                Error::Refused("no settled checkpoint; select a restart point explicitly".into())
            })
            .and_then(|cp| self.locate_checkpoint(cp));
        match located {
            Ok(cp) => {
                let span = self.lengths[cp.pass];
                let begin = self
                    .current
                    .job
                    .passes
                    .iter()
                    .find(|p| p.pass.ordinal == self.original.job.passes[cp.pass].pass.ordinal)
                    .map_or(0., |p| if span > 0. { p.consumed / span } else { 0. });
                self.record(cp.pass, begin, cp.fraction);
                self.last_pause = Some(cp);
                self.pause_position = program.checkpoint.map(|p| p.position_mm);
                self.pause_cutting = program.checkpoint.is_some_and(|p| p.item >= 0)
                    && self.original.job.passes[cp.pass].pass.kind != PassKind::PrePierce;
                self.selected = Some(cp);
                self.problem = None;
            }
            Err(error) => {
                self.selected = None;
                self.pause_position = None;
                self.pause_cutting = false;
                self.problem = Some(error.to_string());
            }
        }
        self.revision += 1;
    }

    fn locate_checkpoint(
        &self,
        cp: openlaser_controller::state::CheckpointView,
    ) -> Result<Checkpoint> {
        let local = usize::try_from(cp.item.cast_unsigned() & 0x7fff_ffff)
            .map_err(|_| Error::Refused("invalid item tag".into()))?;
        let current = self.current.job.passes.get(local).ok_or_else(|| {
            Error::Refused("the checkpoint is outside the retained passes".into())
        })?;
        let root = self
            .original
            .job
            .passes
            .iter()
            .position(|p| p.pass.ordinal == current.pass.ordinal)
            .ok_or_else(|| Error::Refused("the original pass identity is missing".into()))?;
        if cp.item.cast_unsigned() & 0x8000_0000 != 0 {
            let span = self.lengths[root];
            return Ok(Checkpoint {
                pass: root,
                fraction: if span > 0. { current.consumed / span } else { 0. },
            });
        }
        let tag = u32::try_from(root).map_err(|_| Error::Refused("original pass index".into()))?;
        locate(
            &self.original.job,
            tag,
            [
                cp.position_mm[0] - self.original.sheet_offset[0],
                cp.position_mm[1] - self.original.sheet_offset[1],
            ],
        )
    }

    fn lost_execution(&mut self) {
        if matches!(self.state, ProgramState::Running | ProgramState::Held) {
            self.state = ProgramState::Failed;
            self.ready = false;
            self.problem = Some(
                "the live execution was lost; establish the reference and prepare recovery".into(),
            );
            self.revision += 1;
        }
    }

    /// Build from the original geometry and remove skipped physical instances.
    pub fn remainder(&self, pierce: bool) -> Result<(Job, Vec<PassView>)> {
        let cp =
            self.selected.ok_or_else(|| Error::Refused("select a restart point first".into()))?;
        self.validate(cp)?;
        let mut job = if let Some(position) = self.return_position()
            && self.pause_cutting
            && cp.fraction < 1.
        {
            self.original.job.continuation_at(
                cp.pass,
                cp.fraction,
                pierce,
                [
                    position[0] - self.original.sheet_offset[0],
                    position[1] - self.original.sheet_offset[1],
                ],
            )?
        } else {
            continuation(&self.original.job, cp, pierce)?
        };
        job.passes.retain(|p| {
            self.original
                .job
                .passes
                .iter()
                .position(|o| o.pass.ordinal == p.pass.ordinal)
                .is_some_and(|i| self.allowed(i))
        });
        if job.passes.is_empty() {
            return Err(Error::Refused("no included passes remain".into()));
        }
        let plan = job
            .passes
            .iter()
            .map(|p| {
                self.execution
                    .compiled
                    .plan
                    .iter()
                    .find(|v| v.ordinal == p.pass.ordinal)
                    .cloned()
                    .ok_or_else(|| Error::Refused("the original pass view is missing".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((job, plan))
    }

    /// The immutable physical pause position, only when resuming that pause.
    pub(crate) fn return_position(&self) -> Option<[f64; 2]> {
        (self.selected.is_some() && self.selected == self.last_pause)
            .then_some(self.pause_position)
            .flatten()
    }

    /// A compact view; callers retain it until the recovery revision changes.
    #[must_use]
    pub fn view(&self) -> RecoveryView {
        RecoveryView {
            revision: self.revision,
            id: self.execution.id,
            name: self.execution.name.clone(),
            state: self.state,
            ready: self.ready,
            selected: self.selected,
            position: self.selected.map(|cp| self.position(cp)),
            last_pause: self.last_pause,
            pause_position: self.pause_position,
            good: self.good,
            problem: self.problem.clone(),
            steps: self
                .execution
                .compiled
                .plan
                .iter()
                .enumerate()
                .map(|(i, pass)| RecoveryStep {
                    pass: pass.clone(),
                    length_mm: self.lengths[i],
                    status: if !self.allowed(i) {
                        "skipped"
                    } else if self.executed[i]
                        .first()
                        .is_some_and(|r| r[0] <= 1e-8 && r[1] >= 1. - 1e-8)
                    {
                        "completed"
                    } else if self.executed[i].is_empty() {
                        "pending"
                    } else {
                        "partial"
                    }
                    .into(),
                    executed: self.executed[i].clone(),
                })
                .collect(),
        }
    }
}

impl Coordinator {
    /// Editing recovery requires an idle operation and an exact view revision.
    pub fn change_recovery(&mut self, revision: u64, change: RecoveryChange) -> Result<()> {
        if self.operation.is_some() || self.machine.state().operation.is_some() {
            return Err(Error::Refused("wait for the operation to settle".into()));
        }
        let recovery =
            self.recovery.as_mut().ok_or_else(|| Error::Refused("no retained job".into()))?;
        if recovery.revision != revision {
            return Err(Error::Refused("recovery changed; review the selection again".into()));
        }
        if !matches!(
            recovery.state,
            ProgramState::Held | ProgramState::Stopped | ProgramState::Failed
        ) {
            return Err(Error::Refused("pause or stop the job before selecting recovery".into()));
        }
        recovery.change(change)?;
        self.publish();
        Ok(())
    }

    /// Rebind a retained stopped job only after fresh reference and clearance checks.
    pub fn prepare_recovery(&mut self, revision: u64, clearance: bool) -> Result<()> {
        self.idle()?;
        if !clearance {
            return Err(Error::Refused("confirm the sheet position and restart clearance".into()));
        }
        let configuration = self.acceptance()?;
        let state = self.machine.state();
        if !state.session.homed
            || !state.feedback.is_some_and(|f| {
                f.age_ms <= crate::coordinator::FRESH_FEEDBACK_MS
                    && f.stationary
                    && f.table_stationary
                    && f.head.command == 0
            })
        {
            return Err(Error::Refused(
                "establish the reference and wait for stationary feedback".into(),
            ));
        }
        let recovery = self
            .recovery
            .as_mut()
            .ok_or_else(|| Error::Refused("no retained stopped job".into()))?;
        if recovery.revision != revision {
            return Err(Error::Refused("recovery changed; review it again".into()));
        }
        if !matches!(
            recovery.state,
            ProgramState::Held | ProgramState::Stopped | ProgramState::Failed
        ) {
            return Err(Error::Refused("no stopped job to recover".into()));
        }
        let selected = recovery
            .selected
            .ok_or_else(|| Error::Refused("select a restart point first".into()))?;
        recovery.validate(selected)?;
        if selected.fraction >= 1. && recovery.adjacent(selected.pass, true).is_none() {
            return Err(Error::Refused("the selected point has no remaining pass".into()));
        }
        if configuration.mode != recovery.original.configuration.mode
            || configuration.verified != recovery.original.configuration.verified
            || self.files.backup.as_ref().map(|f| f.sha256.as_str())
                != Some(recovery.machine_hash.as_str())
        {
            return Err(Error::Refused(
                "machine settings differ from the retained execution".into(),
            ));
        }
        recovery.configuration = configuration;
        recovery.ready = true;
        recovery.revision += 1;
        self.publish();
        Ok(())
    }
}
