// SPDX-License-Identifier: GPL-3.0-or-later

//! Head height calibration, MainApp `0x0050_3f40` and `0x0062_4930`: the
//! host sends one command and the head controller does the rest, reporting
//! the calibration's quality in its status byte when it finishes.

use super::{Operation, Step, admit_motion};
use crate::session::Quality;
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::{self, Write};
use std::time::{Duration, Instant};

/// The head command word while calibrating.
const CALIBRATING: u32 = 107;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    AwaitingRunning,
    Running,
    Done(Quality),
}

/// The calibration.
#[derive(Debug)]
pub struct Calibrate {
    phase: Phase,
    started: Option<Instant>,
    timeout: Duration,
}

impl Calibrate {
    /// How long the host waits for the head controller.
    pub const TIMEOUT: Duration = Duration::from_mins(1);

    /// A calibration.
    #[must_use]
    pub const fn new() -> Self {
        Self { phase: Phase::Start, started: None, timeout: Self::TIMEOUT }
    }

    /// The quality reported, once the calibration finished.
    #[must_use]
    pub const fn quality(&self) -> Option<Quality> {
        match self.phase {
            Phase::Done(quality) => Some(quality),
            _ => None,
        }
    }
}

impl Default for Calibrate {
    fn default() -> Self {
        Self::new()
    }
}

impl Operation for Calibrate {
    fn kind(&self) -> OperationKind {
        OperationKind::Calibrate
    }

    fn phase(&self) -> &'static str {
        match self.phase {
            Phase::Start => "starting",
            Phase::AwaitingRunning => "requested",
            Phase::Running => "running",
            Phase::Done(_) => "done",
        }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        if self.started.is_some_and(|started| now.duration_since(started) > self.timeout) {
            return Err("the calibration did not finish in time".into());
        }
        if !snapshot.head.referenced() {
            return Err("the head has no reference; run Go Origin first".into());
        }
        match self.phase {
            Phase::Start => {
                admit_motion(snapshot, blocked)?;
                self.started = Some(now);
                self.phase = Phase::AwaitingRunning;
                Ok(Step::Send(vec![requests::head_calibrate()]))
            }
            Phase::AwaitingRunning | Phase::Running => {
                if !snapshot.alarms_clear() || blocked.is_some() {
                    return Err("an alarm interrupted the calibration".into());
                }
                if snapshot.status.fifo_activity() == 1 {
                    return Err("a program started during the calibration".into());
                }
                let calibrating = snapshot.head.command() == CALIBRATING;
                match (self.phase, calibrating) {
                    (Phase::AwaitingRunning, true) => {
                        self.phase = Phase::Running;
                        Ok(Step::Wait)
                    }
                    (Phase::Running, false) => {
                        match Quality::from_status_byte(snapshot.head.status_byte()) {
                            Some(Quality::Bad) => Err("the head reported a bad calibration".into()),
                            Some(quality) => {
                                self.phase = Phase::Done(quality);
                                Ok(Step::Finish(Vec::new()))
                            }
                            None => Err("the head did not report a calibration result".into()),
                        }
                    }
                    _ => Ok(Step::Wait),
                }
            }
            Phase::Done(_) => Ok(Step::Finish(Vec::new())),
        }
    }

    fn cancel(&self) -> Vec<Write> {
        vec![requests::head_cancel()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::snapshot_with;

    /// The calibration sends 107, waits for the head to report it running,
    /// then reads the quality when the command clears; bad quality fails.
    #[test]
    fn calibration_follows_the_head_command_and_reads_quality() {
        let now = Instant::now();
        let mut calibrate = Calibrate::new();
        let idle = snapshot_with(&[], &[], &[(2, 0x8000_0010)]);
        let step = calibrate.step(&idle, None, now).unwrap();
        assert_eq!(step, Step::Send(vec![requests::head_calibrate()]));
        assert_eq!(calibrate.step(&idle, None, now).unwrap(), Step::Wait);
        let running = snapshot_with(&[], &[], &[(2, 0x8000_0010), (3, 107)]);
        assert_eq!(calibrate.step(&running, None, now).unwrap(), Step::Wait);
        let good = snapshot_with(&[], &[], &[(2, 0x8000_0011)]);
        assert_eq!(calibrate.step(&good, None, now).unwrap(), Step::Finish(Vec::new()));
        assert_eq!(calibrate.quality(), Some(Quality::Good));
        let mut bad = Calibrate::new();
        bad.step(&idle, None, now).unwrap();
        bad.step(&running, None, now).unwrap();
        assert!(bad.step(&snapshot_with(&[], &[], &[(2, 0x8000_0012)]), None, now).is_err());
        let mut unreferenced = Calibrate::new();
        assert!(unreferenced.step(&snapshot_with(&[], &[], &[]), None, now).is_err());
        assert_eq!(calibrate.cancel(), vec![requests::head_cancel()]);
    }
}
