// SPDX-License-Identifier: GPL-3.0-or-later

//! The laser mode switch, NCModule `0x1002_e100`: outputs off, the head
//! told the new mode, the mode's enable output on, and the XY limits for
//! that mode. Afterwards the reference must be established again.

use super::{Operation, Step, admit_positioning};
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_core::LaserMode;
use openlaser_protocol::requests::Write;
use openlaser_protocol::sequences::ModeSwitch;
use std::time::Instant;

/// The switch.
#[derive(Debug)]
pub struct Switch {
    mode: LaserMode,
    writes: Vec<Write>,
    done: bool,
}

impl Switch {
    /// A switch to `plan.mode`.
    pub fn new(plan: &ModeSwitch) -> Result<Self, String> {
        Ok(Self { mode: plan.mode, writes: plan.plan().map_err(|e| e.to_string())?, done: false })
    }

    /// The mode being switched to.
    #[must_use]
    pub const fn mode(&self) -> LaserMode {
        self.mode
    }
}

impl Operation for Switch {
    fn kind(&self) -> OperationKind {
        OperationKind::ModeSwitch
    }

    fn phase(&self) -> &'static str {
        if self.done { "done" } else { "switching" }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        _now: Instant,
    ) -> Result<Step, String> {
        if self.done {
            return Ok(Step::Finish(Vec::new()));
        }
        admit_positioning(snapshot, blocked)?;
        self.done = true;
        Ok(Step::Finish(self.writes.clone()))
    }

    fn cancel(&self) -> Vec<Write> {
        Vec::new()
    }
}
