// SPDX-License-Identifier: GPL-3.0-or-later

//! The operations the task can run, one at a time.
//!
//! Each is a state machine over feedback. Every tick it receives the fresh
//! snapshot and answers with the writes to send, and it ends by finishing
//! or failing. Sending, and the stop sequence that follows a failure, is
//! the task's job, so nothing here touches a socket.

pub mod calibrate;
pub mod home;
pub mod mode;
pub mod motion;
pub mod outputs;
pub mod parameters;
pub mod relief;

use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::Write;
use std::time::Instant;

/// What an operation wants after seeing a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing to send; look again next tick.
    Wait,
    /// Send these, in order, then look again next tick.
    Send(Vec<Write>),
    /// Send these, in order, and the operation is complete.
    Finish(Vec<Write>),
}

/// What every operation offers the task.
pub trait Operation: Send {
    /// Which kind it is.
    fn kind(&self) -> OperationKind;

    /// Where it stands, as a short label.
    fn phase(&self) -> &'static str;

    /// Advances on a fresh snapshot. `blocked` names the alarm rows that
    /// block ordinary operations, when there are any.
    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String>;

    /// The writes that abandon the operation before the stop sequence.
    fn cancel(&self) -> Vec<Write>;

    /// Commits a write that the controller acknowledged. Called once per
    /// successful exchange; scheduling a write is not acknowledgement.
    fn acknowledged(&mut self, _write: &Write) {}

    /// The vendor's operation state while this runs, for arming input
    /// rules: 4 for a manual operation, 2 for a cut.
    fn operation_state(&self) -> u32 {
        4
    }
}

/// The admission every operation shares: no blocking alarm rows, both
/// controller aggregates clear, and every output off except `allowed`.
pub(crate) fn admit(
    snapshot: &Snapshot,
    blocked: Option<&str>,
    allowed: u16,
) -> Result<(), String> {
    admit_alarms(snapshot.alarms_clear(), blocked)?;
    if !snapshot.outputs_off(allowed) {
        return Err("outputs are on".into());
    }
    Ok(())
}

pub(crate) fn admit_alarms(clear: bool, blocked: Option<&str>) -> Result<(), String> {
    if let Some(blocked) = blocked {
        return Err(format!("alarms are active: {blocked}"));
    }
    if !clear {
        return Err("controller alarms are active".into());
    }
    Ok(())
}

/// A laser-off manual move can precede referencing. Alarms, idle feedback
/// and output checks still apply.
pub(crate) fn admit_positioning(snapshot: &Snapshot, blocked: Option<&str>) -> Result<(), String> {
    admit_alarms(snapshot.positioning_alarms_clear(), blocked)?;
    if !snapshot.outputs_off(0) {
        return Err("outputs are on".into());
    }
    admit_idle(snapshot)
}

/// Admission for anything that moves: the shared checks, then idle axes
/// and an idle head.
pub(crate) fn admit_motion(snapshot: &Snapshot, blocked: Option<&str>) -> Result<(), String> {
    admit(snapshot, blocked, 0)?;
    admit_idle(snapshot)
}

/// Idle axes and an idle head.
pub(crate) fn admit_idle(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.class() != openlaser_protocol::feedback::AxisClass::None {
        return Err("the axes are not idle".into());
    }
    if !snapshot.xy_stationary() {
        return Err("the axes are moving".into());
    }
    if !snapshot.head_idle() {
        return Err("the head is busy".into());
    }
    Ok(())
}
