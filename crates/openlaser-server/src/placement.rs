// SPDX-License-Identifier: GPL-3.0-or-later

//! Physical sheet placement, separate from reusable toolpaths and saved intent.

use crate::coordinator::{Coordinator, Shared};
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::geometry::{Bounds, Point};
use openlaser_core::nesting::NestStock;
use openlaser_library::placement::Placement;
use serde::{Deserialize, Serialize};

/// The two positioning choices shown beside the jog controls.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementMode {
    /// Follow the head until this run captures its position.
    Head,
    /// Reuse homed machine coordinates.
    Fixed,
}

/// Per-run captures are transient; absolute fixture coordinates belong to the job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlacementView {
    /// Positioning method.
    pub mode: PlacementMode,
    /// A fixed fixture or captured head location is available.
    pub captured: bool,
    /// Correction awaits a physical location; the current preview is nominal.
    pub correction_pending: bool,
    /// The method and fixture values match the saved job.
    pub saved: bool,
}

/// Explicit changes made in the Run positioning panel. None causes motion.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlacementChange {
    /// Use the live head for the next placement.
    Head {},
    /// Explicitly capture the head for this run; a new run gets a fresh position.
    SetOrigin {},
    /// Capture the current head as the reusable absolute fixture position.
    FixedHead {},
    /// Place the bottom-left reference at machine XY.
    Fixed {
        /// Machine millimetres from the homed zero.
        origin: [f64; 2],
    },
    /// Release the captured head location so manual jogs position it again.
    Reposition {},
    /// Begin another cut after completion, retaining only reusable settings.
    NewRun {},
}

pub(crate) fn is_head(draft: &Draft) -> bool {
    matches!(draft.placement, Some(Placement::Head {}))
}

pub(crate) fn view(draft: &Draft) -> PlacementView {
    PlacementView {
        mode: if is_head(draft) { PlacementMode::Head } else { PlacementMode::Fixed },
        captured: draft.zero.is_some(),
        correction_pending: draft.correction.is_some() && draft.zero.is_none(),
        saved: draft.saved_base.as_ref().is_some_and(|saved| matches_saved(draft, saved)),
    }
}

/// Stock keeps its reference even when the parts inside it are rearranged.
pub(crate) fn reference_bounds(draft: &Draft) -> Option<Bounds> {
    match draft.nesting.as_ref().map(|n| &n.stock) {
        Some(NestStock::Rectangle { bounds }) => Some(*bounds),
        Some(NestStock::Remnant { outline, .. }) => outline.bounds(),
        Some(NestStock::Outline { .. }) => {
            let mut points = draft.stock_outline.iter().copied();
            let mut bounds = Bounds::of(Point::from(points.next()?));
            for point in points {
                bounds.include(point.into());
            }
            Some(bounds)
        }
        None => draft.preview.as_ref()?.bounds,
    }
}

/// Transient captures never carry into another loose sheet.
pub(crate) fn fresh(draft: &mut Draft) {
    draft.capture_epoch = None;
    draft.capture_used = false;
    if is_head(draft) {
        draft.zero = None;
        draft.compiled = None;
    }
}

pub(crate) fn matches_saved(draft: &Draft, saved: &openlaser_library::Job) -> bool {
    match (&draft.placement, &saved.placement) {
        (Some(Placement::Head {}), Some(Placement::Head {})) => true,
        (Some(current), Some(previous)) => current == previous,
        // Legacy jobs store the translation; preserve it until explicitly edited.
        (_, None) => !is_head(draft) && draft.zero == saved.zero,
        _ => false,
    }
}

impl Coordinator {
    pub(crate) fn origin_gate(&self, state: &openlaser_controller::State) -> crate::document::Gate {
        use crate::document::Gate;
        if self.operation.is_some() || state.operation.is_some() {
            return Gate::closed("another operation is active");
        }
        if self.held.is_some() {
            return Gate::closed("resume keeps the retained origin");
        }
        match self.draft.as_ref().map(|draft| head_position(draft, state)) {
            Some(Ok(_)) => Gate::open(),
            Some(Err(error)) => Gate::closed(error.to_string()),
            None => Gate::closed("open a part first"),
        }
    }

    /// Change positioning intent while stationary. Resume owns immutable placement.
    pub fn change_placement(&mut self, change: PlacementChange) -> Result<()> {
        self.idle()?;
        if self.held.is_some() {
            return Err(Error::Refused("resume keeps the retained sheet position".into()));
        }
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        if let PlacementChange::Fixed { origin } = &change {
            Placement::Fixed { origin: *origin }.validate().map_err(Error::Request)?;
        }
        if !matches!(change, PlacementChange::SetOrigin {} | PlacementChange::FixedHead {}) {
            draft.remember();
        }
        match change {
            PlacementChange::Head {} => {
                draft.placement = Some(Placement::Head {});
                draft.anchor = openlaser_library::Anchor::FrontLeft;
                fresh(draft);
            }
            PlacementChange::SetOrigin {} if is_head(draft) => {
                pin_head(draft, &self.machine.state(), true)?;
            }
            PlacementChange::SetOrigin {} | PlacementChange::FixedHead {} => {
                let origin = head_position(draft, &self.machine.state())?;
                draft.remember();
                draft.anchor = openlaser_library::Anchor::FrontLeft;
                draft.pin(origin)?;
                draft.placement = Some(Placement::Fixed { origin });
                draft.capture_epoch = None;
                draft.capture_used = false;
            }
            PlacementChange::Fixed { origin } => {
                draft.anchor = openlaser_library::Anchor::FrontLeft;
                draft.pin(origin)?;
                draft.placement = Some(Placement::Fixed { origin });
            }
            PlacementChange::Reposition {} => {
                if is_head(draft) {
                    draft.zero = None;
                    draft.capture_epoch = None;
                }
            }
            PlacementChange::NewRun {} => {
                fresh(draft);
                self.completed_sheet = None;
                self.postflight = None;
                self.recovery = None;
            }
        }
        draft.compiled = None;
        self.execution_changed(None);
        self.draft_changed();
        Ok(())
    }

    /// Capture the head once. Framing, preflight motion and cutting cannot move it.
    pub(crate) fn capture_placement(&mut self) -> Result<()> {
        self.idle()?;
        let state = self.machine.state();
        let epoch = state.configuration.map(|c| c.epoch);
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        if is_head(draft) && draft.capture_used && self.held.is_none() {
            fresh(draft);
        }
        if is_head(draft)
            && (draft.zero.is_none() || epoch.is_none() || draft.capture_epoch != epoch)
        {
            pin_head(draft, &state, false)?;
            self.draft_changed();
        }
        Ok(())
    }
}

/// The button and both capture paths share the same fresh, stationary head.
fn head_position(draft: &Draft, state: &openlaser_controller::State) -> Result<[f64; 2]> {
    if !state.session.homed {
        return Err(Error::Refused("home XY before positioning this sheet".into()));
    }
    let feedback = state
        .feedback
        .as_ref()
        .filter(|f| f.age_ms <= 1000 && f.stationary && f.head.command == 0)
        .ok_or_else(|| {
            Error::Refused("set the origin with fresh feedback and the axes stationary".into())
        })?;
    if draft.dock().is_none() {
        return Err(Error::Refused("nothing prepared".into()));
    }
    Ok([feedback.position_mm[0], feedback.position_mm[1]])
}

fn pin_head(draft: &mut Draft, state: &openlaser_controller::State, remember: bool) -> Result<()> {
    let origin = head_position(draft, state)?;
    if remember {
        draft.remember();
    }
    draft.pin(origin)?;
    draft.placement = Some(Placement::Head {});
    draft.capture_epoch = state.configuration.map(|c| c.epoch);
    draft.capture_used = false;
    if draft.correction.is_some() {
        draft.compiled = None;
    }
    Ok(())
}

/// Apply a placement edit and rebuild using the retained run choice.
pub async fn change(shared: &Shared, change: PlacementChange, revision: u64) -> Result<()> {
    let (input, revision) = {
        let mut c = shared.lock().await;
        c.check_draft(revision)?;
        let dry_run = c.execution.as_ref().filter(|e| !e.frame).map(|e| e.compiled.dry_run);
        c.change_placement(change)?;
        if matches!(change, PlacementChange::NewRun {})
            && let (Some(draft), Some(dry_run)) = (&mut c.draft, dry_run)
        {
            draft.dry_run = dry_run;
        }
        c.queue_draft();
        (c.preparation()?, c.document().draft_revision)
    };
    crate::workspace::flush(shared).await?;
    let revision = if let Some(input) = input {
        crate::machine::prepare_input(shared, input).await?.revision
    } else {
        revision
    };
    crate::machine::compile_automatically(shared, revision).await?;
    Ok(())
}

/// Capture and finish any location-dependent compilation before review or motion.
pub async fn prepare(shared: &Shared) -> Result<()> {
    prepare_revision(shared, None).await
}

/// The HTTP request may capture only the draft revision shown by its caller.
pub(crate) async fn prepare_revision(shared: &Shared, revision: Option<u64>) -> Result<()> {
    let dry_run = {
        let mut c = shared.lock().await;
        if let Some(revision) = revision {
            c.check_draft(revision)?;
        }
        let dry_run = c.draft.as_ref().is_some_and(|d| d.dry_run);
        c.capture_placement()?;
        let compiled = c.draft.as_ref().and_then(|d| d.compiled.as_ref());
        if compiled.is_some_and(|p| {
            p.configuration.is_some_and(|configuration| c.acceptance().ok() == Some(configuration))
        }) {
            None
        } else {
            Some(dry_run)
        }
    };
    if let Some(dry_run) = dry_run {
        crate::machine::compile(shared, dry_run).await?;
    }
    let c = shared.lock().await;
    let draft = c.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
    let compiled =
        draft.compiled.as_ref().ok_or_else(|| Error::Refused("compile the job first".into()))?;
    let zero = draft.zero()?;
    let bounds = crate::envelope::process_bounds(
        &compiled.job,
        u32::try_from(compiled.scale)
            .map_err(|_| Error::Refused("invalid coordinate scale".into()))?,
    )?;
    let extent = c.extent().ok_or_else(|| Error::Refused("machine travel is unknown".into()))?;
    for p in [bounds.min, bounds.max] {
        let p = [p.x + zero[0], p.y + zero[1]];
        if (0..2).any(|axis| p[axis] < extent[axis][0] - 1e-9 || p[axis] > extent[axis][1] + 1e-9) {
            return Err(Error::Refused(
                "the positioned cut leaves machine travel; adjust the sheet position".into(),
            ));
        }
    }
    Ok(())
}
