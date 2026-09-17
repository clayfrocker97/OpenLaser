// SPDX-License-Identifier: GPL-3.0-or-later

//! Alarm relief, MainApp `0x0055_6040`: the reset writes for one alarm id,
//! sent only when the machine is still. Resetting never moves an axis,
//! enables an output or restarts a program.

use super::{Operation, Step};
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::Write;
use openlaser_protocol::sequences::ReliefPlan;
use std::time::Instant;

/// The relief.
#[derive(Debug)]
pub struct Relief {
    plan: ReliefPlan,
    done: bool,
}

impl Relief {
    /// Relief with `plan`.
    #[must_use]
    pub const fn new(plan: ReliefPlan) -> Self {
        Self { plan, done: false }
    }

    /// Whether the host's custom and gas rows are cleared once the writes
    /// are acknowledged.
    #[must_use]
    pub const fn clears_host_rows(&self) -> bool {
        self.plan.clears_host_rows
    }
}

impl Operation for Relief {
    fn kind(&self) -> OperationKind {
        OperationKind::Relief
    }

    fn phase(&self) -> &'static str {
        if self.done { "done" } else { "resetting" }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        _blocked: Option<&str>,
        _now: Instant,
    ) -> Result<Step, String> {
        if self.done {
            return Ok(Step::Finish(Vec::new()));
        }
        let still = snapshot.status.fifo_activity() == 0
            && snapshot.outputs_off(0)
            && snapshot.axes.all().iter().all(|axis| axis.speed == 0 && axis.phase() == 0)
            && snapshot.head_idle();
        if !still {
            return Err(
                "wait for stationary axes, an idle FIFO and outputs off before resetting".into()
            );
        }
        self.done = true;
        Ok(Step::Finish(self.plan.writes.clone()))
    }

    fn cancel(&self) -> Vec<Write> {
        Vec::new()
    }
}
