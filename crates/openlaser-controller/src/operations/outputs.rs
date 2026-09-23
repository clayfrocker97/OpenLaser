// SPDX-License-Identifier: GPL-3.0-or-later

//! A manual output held on: gas, the pointer, the shutter, a head jog. The
//! plan's on-steps run in order with their settle delays; the outputs stay
//! on while the operator holds the control and the lease is renewed, and
//! the off-writes run when the control is released, the lease lapses, or
//! anything goes wrong. A head jogged down onto the plate stops and rises
//! to the safe height by itself, whatever the operator's control does.

use super::raise::Raise;
use super::{Operation, Step, admit, admit_alarms, admit_idle};
use crate::alarms::Concession;
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::{self, Write};
use openlaser_protocol::sequences;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A manual output plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// What it is, for the state.
    pub name: String,
    /// The writes and delays that switch it on.
    pub on: Vec<sequences::Step>,
    /// The writes that switch it off, in order.
    pub off: Vec<Write>,
    /// How long the output stays on without a renewal.
    pub lease: Duration,
    /// A timed test ends this long after its setup sequence finishes.
    pub duration: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Admitting,
    Starting,
    Holding,
    Done,
}

/// The output.
#[derive(Debug)]
pub struct Outputs {
    plan: Plan,
    queue: VecDeque<sequences::Step>,
    phase: Phase,
    wake: Option<Instant>,
    renewed: Instant,
    holding_since: Option<Instant>,
    allowed: (u16, u32),
    recovery_deadline: Option<Instant>,
    /// Where a head jogged down onto the plate rises to.
    raise: Option<sequences::Raise>,
    /// The rise after the head touched the plate.
    touched: Option<Box<Raise>>,
}

impl Outputs {
    /// A plan about to start.
    #[must_use]
    pub fn new(plan: Plan, now: Instant) -> Self {
        let queue = plan.on.iter().cloned().collect();
        Self {
            plan,
            queue,
            phase: Phase::Admitting,
            wake: None,
            renewed: now,
            holding_since: None,
            allowed: (0, 0),
            recovery_deadline: None,
            raise: None,
            touched: None,
        }
    }

    /// Raises a head jogged down onto the plate to `raise` afterwards.
    #[must_use]
    pub const fn rising_on_touch(mut self, raise: Option<sequences::Raise>) -> Self {
        self.raise = raise;
        self
    }

    /// A downward head jog whose head reports touching the plate, head
    /// bit 5, after it was admitted.
    fn touched_down(&self, snapshot: &Snapshot) -> bool {
        self.phase != Phase::Admitting
            && self.head_jog() == Some(false)
            && snapshot.head.alarm_word() & (1 << 5) != 0
    }

    /// The plan's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.plan.name
    }

    /// Only a relative head move with its matching cancel gets the
    /// positioning admission; an output plan cannot acquire it by name.
    pub(crate) fn head_jog(&self) -> Option<bool> {
        let [sequences::Step::Write(write)] = self.plan.on.as_slice() else { return None };
        let [109, speed @ 1..=10_000, travel] = write.words.as_slice() else { return None };
        (*travel != 0
            && *write == requests::head_move(speed.cast_signed(), travel.cast_signed())
            && self.plan.off == [requests::head_cancel()])
        .then_some(travel.cast_signed() < 0)
    }

    /// Renews the lease.
    pub fn heartbeat(&mut self, now: Instant) {
        if self.plan.duration.is_none() {
            self.renewed = now;
        }
    }

    pub(crate) fn expired(&self, now: Instant) -> bool {
        let (since, limit) =
            self.holding_since.zip(self.plan.duration).unwrap_or((self.renewed, self.plan.lease));
        self.phase != Phase::Done
            && (now.saturating_duration_since(since) > limit
                || self.recovery_deadline.is_some_and(|deadline| now >= deadline))
    }

    /// Switches off now: the writes to send. A head rising from the plate
    /// is not stopped by letting go; only Stop cancels it.
    pub fn release(&mut self) -> Vec<Write> {
        if self.touched.is_some() {
            return Vec::new();
        }
        self.phase = Phase::Done;
        self.plan.off.clone()
    }

    /// Records an output this plan switched on, so its feedback is expected.
    fn expect(&mut self, write: &Write) {
        if let [9999, selector @ (2 | 13), mask, values] = write.words.as_slice()
            && let Ok(mask) = u16::try_from(*mask)
        {
            let on = u16::try_from(*values).unwrap_or(0) & mask;
            if *selector == 2 {
                self.allowed.0 = (self.allowed.0 & !mask) | on;
            } else {
                self.allowed.1 = (self.allowed.1 & !u32::from(mask)) | u32::from(on);
            }
        }
    }

    fn outputs_expected(&self, snapshot: &Snapshot) -> bool {
        snapshot.status.outputs() & !self.allowed.0 == 0
            && snapshot.status.extended_outputs() & !self.allowed.1 == 0
    }
}

impl Outputs {
    fn admit_and_check(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<(), String> {
        if self.phase == Phase::Admitting {
            if let Some(up) = self.head_jog() {
                admit_head_jog(snapshot, blocked, up)?;
                if snapshot.head_limit_alarms() != 0 {
                    // Bound the actual wire command at fresh admission, so a
                    // stale UI or a Fast press cannot enlarge recovery travel.
                    let Some(sequences::Step::Write(mut write)) = self.queue.pop_front() else {
                        return Err("invalid head jog".into());
                    };
                    write.words[1] = write.words[1].min(10);
                    write.words[2] =
                        write.words[2].cast_signed().clamp(-1000, 1000).cast_unsigned();
                    self.queue.push_front(sequences::Step::Write(write));
                    self.recovery_deadline = Some(now + Duration::from_secs(2));
                }
            } else {
                admit(snapshot, blocked, 0)?;
            }
            self.phase = Phase::Starting;
        }
        let clear = if let Some(up) = self.head_jog() {
            // A limit appearing during an ordinary jog must stop it; only a
            // move bounded at admission has permission to recover a limit.
            snapshot.head_jog_alarms_clear(up)
                && (snapshot.head_limit_alarms() == 0 || self.recovery_deadline.is_some())
        } else {
            snapshot.alarms_clear()
        };
        if !clear || blocked.is_some() {
            return Err("an alarm interrupted the output".into());
        }
        if !self.outputs_expected(snapshot) {
            return Err("an unexpected output is on".into());
        }
        if self.head_jog().is_some()
            && (snapshot.status.fifo_activity() != 0
                || !matches!(snapshot.head.command(), 0 | 109)
                || snapshot
                    .axes
                    .all()
                    .iter()
                    .enumerate()
                    .any(|(index, axis)| index != 3 && (axis.speed != 0 || axis.phase() != 0)))
        {
            return Err("unexpected motion interrupted the head jog".into());
        }
        Ok(())
    }

    fn drain(&mut self, now: Instant) -> Step {
        self.wake = None;
        let mut writes = Vec::new();
        while let Some(step) = self.queue.pop_front() {
            match step {
                sequences::Step::Write(write) => {
                    self.expect(&write);
                    writes.push(write);
                }
                sequences::Step::Delay(millis) => {
                    self.wake = Some(now + Duration::from_millis(u64::from(millis)));
                    break;
                }
            }
        }
        if self.phase == Phase::Starting && self.queue.is_empty() && self.wake.is_none() {
            self.phase = Phase::Holding;
            self.holding_since = Some(now);
        }
        if writes.is_empty() { Step::Wait } else { Step::Send(writes) }
    }
}

impl Operation for Outputs {
    fn kind(&self) -> OperationKind {
        OperationKind::Outputs
    }

    fn phase(&self) -> &'static str {
        if self.touched.is_some() {
            return "raising head";
        }
        match self.phase {
            Phase::Admitting | Phase::Starting => "starting",
            Phase::Holding if self.recovery_deadline.is_some() => "recovering Z",
            Phase::Holding => "on",
            Phase::Done => "done",
        }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        if let Some(raise) = &mut self.touched {
            let writes = raise.step(snapshot, now);
            return Ok(if raise.done() {
                Step::Finish(writes)
            } else if writes.is_empty() {
                Step::Wait
            } else {
                Step::Send(writes)
            });
        }
        if self.phase == Phase::Done {
            return Ok(Step::Finish(Vec::new()));
        }
        if self.touched_down(snapshot) {
            self.phase = Phase::Done;
            self.touched = Some(Box::new(Raise::new(self.raise, Vec::new(), now)));
            return Ok(Step::Send(self.plan.off.clone()));
        }
        self.admit_and_check(snapshot, blocked, now)?;
        if self.expired(now) {
            return Ok(Step::Finish(self.release()));
        }
        if self.wake.is_some_and(|wake| now < wake) {
            return Ok(Step::Wait);
        }
        Ok(self.drain(now))
    }

    fn cancel(&self) -> Vec<Write> {
        self.plan.off.clone()
    }
}

/// The monitor concession and raw-word admission are checked together.
pub(crate) fn head_concession(snapshot: &Snapshot, up: bool) -> Concession {
    Concession::HeadJog { up, accompanied: snapshot.head_jog_alarms_clear(up) }
}

/// Recovery requires stationary axes, an idle FIFO and all outputs off.
pub(crate) fn admit_head_jog(
    snapshot: &Snapshot,
    blocked: Option<&str>,
    up: bool,
) -> Result<(), String> {
    admit_alarms(snapshot.head_jog_alarms_clear(up), blocked)?;
    if !snapshot.outputs_off(0) {
        return Err("outputs are on".into());
    }
    admit_idle(snapshot)?;
    if !snapshot.stationary() || snapshot.status.fifo_activity() != 0 {
        return Err("wait for stationary axes and an idle FIFO".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alarms::Monitor;
    use crate::snapshot::tests::snapshot_with;
    use openlaser_protocol::alarms::DetailCache;
    use openlaser_protocol::requests;

    fn head(up: bool) -> Plan {
        Plan {
            name: "head".into(),
            on: vec![sequences::Step::Write(requests::head_move(
                5000,
                if up { -1_000_000 } else { 1_000_000 },
            ))],
            off: vec![requests::head_cancel()],
            lease: Duration::from_millis(300),
            duration: None,
        }
    }

    fn blocked(snapshot: &Snapshot, up: bool) -> Option<String> {
        let mut monitor = Monitor::new(vec![], true);
        monitor.observe(snapshot, &DetailCache::default());
        monitor.blocked_for(head_concession(snapshot, up))
    }

    #[test]
    fn only_the_away_direction_gets_a_bounded_recovery_command() {
        let now = Instant::now();
        for (bits, up) in [(1, false), (4, false), (5, false), (2, true), (8, true), (10, true)] {
            let snapshot = snapshot_with(&[(6, 1 << 24)], &[], &[(1, bits | (1 << 12))]);
            let mut output = Outputs::new(head(up), now);
            assert_eq!(
                output.step(&snapshot, blocked(&snapshot, up).as_deref(), now).unwrap(),
                Step::Send(vec![requests::head_move(10, if up { -1000 } else { 1000 })])
            );
            assert!(
                Outputs::new(head(!up), now)
                    .step(&snapshot, blocked(&snapshot, !up).as_deref(), now)
                    .is_err()
            );
            assert_eq!(output.release(), vec![requests::head_cancel()]);
        }
    }

    #[test]
    fn recovery_keeps_other_faults_idle_checks_and_command_authentication() {
        let now = Instant::now();
        for snapshot in [
            snapshot_with(&[(6, (1 << 24) | (1 << 30))], &[], &[(1, 1)]),
            snapshot_with(&[(6, 1 << 24), (7, 1)], &[], &[(1, 1)]),
            snapshot_with(&[(6, 1 << 24)], &[], &[(1, 3)]),
            snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1 | (1 << 8))]),
            snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1 | (1 << 15))]),
            snapshot_with(&[(6, 1 << 24)], &[], &[(2, 0x8000_0000)]),
            snapshot_with(&[(5, 1)], &[], &[(1, 1)]),
            snapshot_with(&[(22, 1)], &[], &[(1, 1)]),
            snapshot_with(&[(19, 2)], &[], &[(1, 1)]),
            snapshot_with(&[], &[(41, 1)], &[(1, 1)]),
            snapshot_with(&[], &[], &[(1, 1), (3, 102)]),
        ] {
            assert!(
                Outputs::new(head(false), now)
                    .step(&snapshot, blocked(&snapshot, false).as_deref(), now)
                    .is_err(),
                "unsafe snapshot was admitted: {snapshot:?}"
            );
        }
        let limit = snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1)]);
        assert!(Outputs::new(head(false), now).step(&limit, Some("Door alarm"), now).is_err());
        let mut forged = head(false);
        forged.on.push(sequences::Step::Write(requests::digital_output(1, true).unwrap()));
        assert!(Outputs::new(forged, now).step(&limit, None, now).is_err());
        let mut wrong_cancel = head(false);
        wrong_cancel.off.clear();
        assert!(Outputs::new(wrong_cancel, now).step(&limit, None, now).is_err());
    }

    #[test]
    fn recovery_cannot_turn_into_a_long_jog_and_still_expires_without_heartbeats() {
        let now = Instant::now();
        let limit = snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1 | (1 << 12))]);
        let clear = snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1 << 12)]);
        let mut recovery = Outputs::new(head(false), now);
        recovery.step(&limit, None, now).unwrap();
        assert_eq!(
            recovery.step(&clear, None, now + Duration::from_millis(301)).unwrap(),
            Step::Finish(vec![requests::head_cancel()])
        );
        let mut recovery = Outputs::new(head(false), now);
        recovery.step(&limit, None, now).unwrap();
        recovery.heartbeat(now + Duration::from_millis(1900));
        assert_eq!(
            recovery.step(&clear, None, now + Duration::from_secs(2)).unwrap(),
            Step::Finish(vec![requests::head_cancel()]),
            "renewal cannot extend a recovery press indefinitely"
        );
        let mut normal = Outputs::new(head(false), now);
        normal.step(&clear, None, now).unwrap();
        assert!(normal.step(&limit, None, now + Duration::from_millis(100)).is_err());
    }

    #[test]
    fn timed_gas_runs_for_its_duration_after_setup_and_cannot_be_renewed() {
        let now = Instant::now();
        let mut plan = gas();
        plan.duration = Some(Duration::from_millis(200));
        let off = plan.off.clone();
        let mut output = Outputs::new(plan, now);
        let idle = snapshot_with(&[], &[], &[]);
        output.step(&idle, None, now).unwrap();
        let valve = snapshot_with(&[(5, 1)], &[], &[]);
        output.step(&valve, None, now + Duration::from_millis(150)).unwrap();
        output.heartbeat(now + Duration::from_secs(10));
        assert!(!output.expired(now + Duration::from_millis(300)));
        assert_eq!(
            output.step(&valve, None, now + Duration::from_millis(351)).unwrap(),
            Step::Finish(off)
        );
    }

    fn gas() -> Plan {
        Plan {
            name: "gas".into(),
            on: vec![
                sequences::Step::Write(requests::digital_output(1, true).unwrap()),
                sequences::Step::Delay(100),
                sequences::Step::Write(requests::analog_output(1, 2500).unwrap()),
            ],
            off: vec![
                requests::analog_output(1, 0).unwrap(),
                requests::digital_output(1, false).unwrap(),
            ],
            lease: Duration::from_secs(2),
            duration: None,
        }
    }

    /// The on-steps run with their delay, the switched-on output is then
    /// expected in feedback, and release or a lapsed lease switches off.
    #[test]
    fn outputs_run_their_plan_and_switch_off() {
        let now = Instant::now();
        let mut outputs = Outputs::new(gas(), now);
        let idle = snapshot_with(&[], &[], &[]);
        assert_eq!(
            outputs.step(&idle, None, now).unwrap(),
            Step::Send(vec![requests::digital_output(1, true).unwrap()])
        );
        let valve_on = snapshot_with(&[(5, 1)], &[], &[]);
        assert_eq!(
            outputs.step(&valve_on, None, now + Duration::from_millis(50)).unwrap(),
            Step::Wait
        );
        assert_eq!(
            outputs.step(&valve_on, None, now + Duration::from_millis(150)).unwrap(),
            Step::Send(vec![requests::analog_output(1, 2500).unwrap()])
        );
        assert_eq!(outputs.phase(), "on");
        assert!(outputs.step(&snapshot_with(&[(5, 3)], &[], &[]), None, now).is_err());
        outputs.heartbeat(now + Duration::from_secs(1));
        assert_eq!(
            outputs.step(&valve_on, None, now + Duration::from_secs(2)).unwrap(),
            Step::Wait
        );
        assert_eq!(
            outputs.step(&valve_on, None, now + Duration::from_secs(4)).unwrap(),
            Step::Finish(gas().off)
        );
        let mut released = Outputs::new(gas(), now);
        released.step(&idle, None, now).unwrap();
        assert_eq!(released.release(), gas().off);
        assert!(
            Outputs::new(gas(), now).step(&valve_on, None, now).is_err(),
            "outputs must be off at the start"
        );
    }
}
