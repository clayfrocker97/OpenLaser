// SPDX-License-Identifier: GPL-3.0-or-later

//! Go Origin, MainApp `0x004a_9130`: the head finds its reference first,
//! then X and Y do, with the manual signal output held while they search.
//! The same machine relieves the head reference alarms by searching the
//! head alone.

use super::{Operation, Step, admit_idle};
use crate::bindings::{Bindings, HomeOutputs};
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::feedback::AxisClass;
use openlaser_protocol::requests::{self, OutputBank, OutputPort, Write};
use std::time::{Duration, Instant};

/// Whether the head word carries a fault other than the reference warning
/// (bit 12), which the head-only search exists to relieve.
#[must_use]
pub const fn other_head_faults(snapshot: &Snapshot) -> bool {
    snapshot.head.alarm_word() & 0xefff != 0
        || (snapshot.status.alarm_group_1() & (1 << 24) != 0
            && snapshot.head.referenced()
            && snapshot.head.alarm_word() & (1 << 12) == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    Head,
    Xy,
    Done,
}

/// The reference search.
#[derive(Debug)]
pub struct Home {
    ports: HomeOutputs,
    head: bool,
    head_only: bool,
    phase: Phase,
    signalled: bool,
    started: Option<Instant>,
    timeout: Duration,
}

impl Home {
    /// The vendor's Go Origin timeout.
    pub const TIMEOUT: Duration = Duration::from_mins(2);

    /// A full search, or the head alone when `head_only`.
    #[must_use]
    pub fn new(bindings: &Bindings, head_only: bool) -> Self {
        Self {
            ports: bindings.home,
            head: bindings.head_enabled,
            head_only,
            phase: Phase::Start,
            signalled: false,
            started: None,
            timeout: Self::TIMEOUT,
        }
    }

    /// Whether this search established the XY reference.
    #[must_use]
    pub fn reference_verified(&self) -> bool {
        self.phase == Phase::Done && !self.head_only
    }

    /// Whether this is the head-only search.
    #[must_use]
    pub const fn head_only(&self) -> bool {
        self.head_only
    }

    /// The manual signal write, or nothing when no port is bound.
    fn signal(&self, on: bool) -> Vec<Write> {
        requests::digital_output(self.ports.manual_signal_port, on)
            .map(|write| vec![write])
            .unwrap_or_default()
    }
    fn guard(
        &self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<(), String> {
        if self.started.is_some_and(|started| now.duration_since(started) > self.timeout) {
            return Err("the reference search did not finish in time".into());
        }
        if self.phase != Phase::Done {
            if let Some(blocked) = blocked {
                return Err(format!("alarms are active: {blocked}"));
            }
            if self.head && other_head_faults(snapshot) {
                return Err("another head fault interrupted the reference search".into());
            }
        }
        Ok(())
    }

    fn start(&mut self, snapshot: &Snapshot, now: Instant) -> Result<Step, String> {
        if !snapshot.positioning_alarms_clear() {
            return Err("controller alarms are active".into());
        }
        if !snapshot.outputs_off(0) {
            return Err("outputs are on".into());
        }
        admit_idle(snapshot)?;
        self.started = Some(now);
        if self.head {
            let mut writes = vec![requests::head_home()];
            if !snapshot.alarms_clear() {
                writes.push(requests::alarm_clear());
            }
            if self.ports.z_origin_done_port != 0 {
                let off = requests::digital_output(self.ports.z_origin_done_port, false);
                writes.push(off.map_err(|e| e.to_string())?);
            }
            self.phase = Phase::Head;
            Ok(Step::Send(writes))
        } else {
            self.phase = Phase::Xy;
            Ok(Step::Send(vec![requests::home_xy()]))
        }
    }

    fn advance_head(&mut self, snapshot: &Snapshot) -> Result<Step, String> {
        if !snapshot.positioning_alarms_clear() || !snapshot.outputs_off(0) {
            return Err("an alarm or an output interrupted the head search".into());
        }
        if !snapshot.head.home_done() {
            return Ok(Step::Wait);
        }
        if self.head_only {
            self.phase = Phase::Done;
            return Ok(Step::Finish(Vec::new()));
        }
        self.phase = Phase::Xy;
        Ok(Step::Send(vec![requests::home_xy()]))
    }

    fn advance_xy(&mut self, snapshot: &Snapshot) -> Result<Step, String> {
        self.check_xy_outputs(snapshot)?;
        match snapshot.class() {
            AxisClass::Active if !self.signalled => {
                self.signalled = true;
                Ok(Step::Send(self.signal(true)))
            }
            AxisClass::None if self.signalled && snapshot.axes.xy_referenced() => {
                self.phase = Phase::Done;
                Ok(Step::Finish(self.signal(false)))
            }
            AxisClass::None | AxisClass::Active => Ok(Step::Wait),
            class => Err(format!("unexpected axis state {class:?} during the XY search")),
        }
    }
    fn check_xy_outputs(&self, snapshot: &Snapshot) -> Result<(), String> {
        let (mut standard, mut extended) =
            (snapshot.status.outputs(), snapshot.status.extended_outputs());
        if self.signalled && self.ports.manual_signal_port != 0 {
            let port = OutputPort::new(self.ports.manual_signal_port).map_err(|e| e.to_string())?;
            match port.bank {
                OutputBank::Standard => standard &= !port.mask,
                OutputBank::Extended => extended &= !u32::from(port.mask),
            }
        }
        if !snapshot.positioning_alarms_clear() || standard != 0 || extended != 0 {
            return Err("an alarm or an output interrupted the XY search".into());
        }
        Ok(())
    }
}

impl Operation for Home {
    fn kind(&self) -> OperationKind {
        OperationKind::Home
    }

    fn phase(&self) -> &'static str {
        match self.phase {
            Phase::Start => "starting",
            Phase::Head => "head",
            Phase::Xy => "xy",
            Phase::Done => "done",
        }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        self.guard(snapshot, blocked, now)?;
        match self.phase {
            Phase::Start => self.start(snapshot, now),
            Phase::Head => self.advance_head(snapshot),
            Phase::Xy => self.advance_xy(snapshot),
            Phase::Done => Ok(Step::Finish(Vec::new())),
        }
    }

    fn cancel(&self) -> Vec<Write> {
        if self.signalled { self.signal(false) } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::{bindings, snapshot_with};

    fn words(step: &Step) -> Vec<Vec<u32>> {
        match step {
            Step::Send(writes) | Step::Finish(writes) => {
                writes.iter().map(|w| w.words.clone()).collect()
            }
            Step::Wait => Vec::new(),
        }
    }

    /// The full search: head home, the origin-done output off, then once the
    /// head is home the XY search with the manual signal held on until both
    /// axes report a reference.
    #[test]
    fn go_origin_runs_head_then_xy_with_the_signal() {
        let mut home = Home::new(&bindings(), false);
        let now = Instant::now();
        let idle = snapshot_with(&[], &[], &[(2, 0x8000_0000)]);
        assert_eq!(words(&home.step(&idle, None, now).unwrap()), [vec![102], vec![9999, 2, 2, 0]]);
        assert_eq!(home.step(&idle, None, now).unwrap(), Step::Wait);
        let head_done = snapshot_with(&[], &[], &[(2, 0x8000_0001)]);
        assert_eq!(words(&home.step(&head_done, None, now).unwrap()), [vec![2, 3, 0]]);
        let searching = snapshot_with(&[], &[(0, 0x0201_0000)], &[(2, 0x8000_0001)]);
        assert_eq!(words(&home.step(&searching, None, now).unwrap()), [vec![9999, 2, 1, 1]]);
        let signalled = snapshot_with(&[(5, 1)], &[(0, 0x0201_0000)], &[(2, 0x8000_0001)]);
        assert_eq!(home.step(&signalled, None, now).unwrap(), Step::Wait);
        let referenced =
            snapshot_with(&[(5, 1)], &[(0, 0x8000), (10, 0x8000)], &[(2, 0x8000_0001)]);
        let finish = home.step(&referenced, None, now).unwrap();
        assert_eq!(words(&finish), [vec![9999, 2, 1, 0]]);
        assert!(matches!(finish, Step::Finish(_)));
        assert!(home.reference_verified());
    }

    /// A head-only search sends the head home and finishes when the head
    /// reports done, ignoring the head reference summary bit; an unrelated
    /// output or a settling axis class fails the search.
    #[test]
    fn head_only_search_and_interruptions() {
        let mut home = Home::new(&bindings(), true);
        let now = Instant::now();
        let alarm = snapshot_with(&[(6, 1 << 24)], &[], &[(2, 0)]);
        assert_eq!(
            words(&home.step(&alarm, None, now).unwrap()),
            [vec![102], vec![9999, 5, 0, 0], vec![9999, 2, 2, 0]]
        );
        let done = snapshot_with(&[], &[], &[(2, 0x8000_0001)]);
        assert!(matches!(home.step(&done, None, now).unwrap(), Step::Finish(_)));
        assert!(!home.reference_verified());
        let mut full = Home::new(&bindings(), false);
        full.step(&snapshot_with(&[], &[], &[(2, 0x8000_0000)]), None, now).unwrap();
        full.step(&snapshot_with(&[], &[], &[(2, 0x8000_0001)]), None, now).unwrap();
        let settling = snapshot_with(&[], &[(0, 0x0301_0000)], &[(2, 0x8000_0001)]);
        assert!(full.step(&settling, None, now).is_err());
        let mut late = Home::new(&bindings(), false);
        late.step(&snapshot_with(&[], &[], &[(2, 0x8000_0000)]), None, now).unwrap();
        assert!(late.step(&done, None, now + Duration::from_secs(121)).is_err());
    }

    #[test]
    fn full_home_observes_host_and_head_faults_in_each_phase() {
        let now = Instant::now();
        let idle = snapshot_with(&[], &[], &[(2, 0x8000_0000)]);
        let done = snapshot_with(&[], &[], &[(2, 0x8000_0001)]);
        let fault = snapshot_with(&[], &[], &[(2, 0x8000_0001), (1, 1 << 8)]);
        for xy in [false, true] {
            for host in [false, true] {
                let mut home = Home::new(&bindings(), false);
                home.step(&idle, None, now).unwrap();
                if xy {
                    home.step(&done, None, now).unwrap();
                }
                assert!(
                    home.step(if host { &done } else { &fault }, host.then_some("door"), now)
                        .is_err()
                );
            }
        }
    }
}
