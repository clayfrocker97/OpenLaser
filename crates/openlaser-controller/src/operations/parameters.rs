// SPDX-License-Identifier: GPL-3.0-or-later

//! Parameter read/apply over fresh direct bank and system feedback.
//! NCModule `0x1002_e100` initializes full banks and system settings;
//! `0x1002_f232` applies changed banks. Both activate after writing. Every
//! write needs a new stillness check, followed by exact configuration readback.

use super::{Operation, Step};
use crate::machine::Bank;
use crate::session::Verified;
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::{self, Write};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

enum Phase {
    Read,
    Apply { writes: VecDeque<Write>, expected: Verified },
    Verify(Verified),
    Done(Verified),
}

/// A fully projected initialization, built from the host's machine files.
#[derive(Clone, Debug)]
pub struct Initialization {
    /// Fresh measurements used to construct this plan.
    pub before: Verified,
    /// Complete expected readback after all writes.
    pub expected: Verified,
    /// Ordered initialization requests.
    pub writes: Vec<Write>,
    /// Idle outputs the configuration enables intentionally.
    pub idle_outputs: u16,
}

/// One read or apply, advanced by the same task as the motion operations.
pub(crate) struct Parameters {
    before: Option<Verified>,
    banks: Option<Vec<Bank>>,
    phase: Phase,
    started: Instant,
    initialization: Option<Initialization>,
    idle_outputs: u16,
    initialized: bool,
}

impl Parameters {
    pub(crate) fn new(before: Option<&Verified>, banks: Option<Vec<Bank>>) -> Self {
        Self {
            before: before.copied(),
            banks,
            phase: Phase::Read,
            started: Instant::now(),
            initialization: None,
            idle_outputs: 0,
            initialized: false,
        }
    }

    pub(crate) fn initialize(plan: Initialization) -> Self {
        let mut operation = Self::new(Some(&plan.before), None);
        operation.idle_outputs = plan.idle_outputs;
        operation.initialization = Some(plan);
        operation.initialized = true;
        operation
    }

    pub(crate) const fn initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn verified(&self) -> Option<Verified> {
        match self.phase {
            Phase::Done(verified) => Some(verified),
            _ => None,
        }
    }

    pub(crate) fn unchanged(&self) -> bool {
        self.verified().is_some_and(|verified| Some(verified) == self.before)
    }
}

impl Parameters {
    fn read(&mut self, snapshot: &Snapshot) -> Result<Phase, String> {
        let fresh = snapshot.parameters().map_err(|e| e.to_string())?;
        if let Some(plan) = self.initialization.take() {
            if fresh != plan.before {
                return Err("the controller changed before initialization; read and retry".into());
            }
            return Ok(Phase::Apply { writes: plan.writes.into(), expected: plan.expected });
        }
        let Some(banks) = self.banks.take() else { return Ok(Phase::Done(fresh)) };
        if self.before != Some(fresh) {
            return Err("the controller parameters changed; read them again before applying".into());
        }
        let mut expected = fresh;
        for (axis, bank) in &banks {
            let target =
                expected.banks.get_mut(usize::from(*axis)).ok_or("invalid parameter axis")?;
            *target = *bank;
        }
        Ok(Phase::Apply { writes: writes(&fresh, &banks)?.into(), expected })
    }
}

impl Operation for Parameters {
    fn kind(&self) -> OperationKind {
        OperationKind::Parameters
    }

    fn phase(&self) -> &'static str {
        match self.phase {
            Phase::Read => "reading",
            Phase::Apply { .. } => "applying",
            Phase::Verify(_) => "verifying",
            Phase::Done(_) => "done",
        }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        if now.duration_since(self.started) > Duration::from_secs(30) {
            return Err("the parameter operation did not finish in time".into());
        }
        loop {
            match &mut self.phase {
                Phase::Read => self.phase = self.read(snapshot)?,
                Phase::Apply { writes, expected } => {
                    if let Some(write) = writes.pop_front() {
                        require_still(snapshot, blocked, self.idle_outputs)?;
                        return Ok(Step::Send(vec![write]));
                    }
                    self.phase = Phase::Verify(*expected);
                }
                Phase::Verify(expected) => {
                    let verified = snapshot.parameters().map_err(|e| e.to_string())?;
                    if verified != *expected {
                        return Err("the parameter readback does not match what was applied".into());
                    }
                    self.phase = Phase::Done(verified);
                    return Ok(Step::Finish(Vec::new()));
                }
                Phase::Done(_) => return Ok(Step::Finish(Vec::new())),
            }
        }
    }

    fn cancel(&self) -> Vec<Write> {
        Vec::new()
    }
}

/// The existing per-write admission: outputs and alarms clear, FIFO empty,
/// every axis still and without an error, and no active head command.
fn require_still(snapshot: &Snapshot, blocked: Option<&str>, allowed: u16) -> Result<(), String> {
    if let Some(blocked) = blocked {
        return Err(format!("alarms are active: {blocked}"));
    }
    if !snapshot.positioning_alarms_clear() {
        return Err("controller alarms are active".into());
    }
    if !snapshot.outputs_off(allowed) {
        return Err("outputs are on".into());
    }
    if snapshot.status.fifo_free() != snapshot.capacity {
        return Err("the FIFO is not empty".into());
    }
    if !snapshot.stationary()
        || snapshot.axes.all().iter().any(|axis| !matches!(axis.state(), 0..=2))
    {
        return Err("an axis is moving, in error, or the head is busy".into());
    }
    Ok(())
}

/// Whether the requested banks match their readback, word for word.
#[must_use]
pub fn matches(verified: &Verified, banks: &[Bank]) -> bool {
    banks.iter().all(|(axis, bank)| verified.banks.get(usize::from(*axis)) == Some(bank))
}

/// Changed banks followed by activation, preserving the native order.
pub fn writes(current: &Verified, banks: &[Bank]) -> Result<Vec<Write>, String> {
    let mut writes = Vec::new();
    let mut seen = [false; 5];
    for (axis, bank) in banks {
        let index = usize::from(*axis);
        let seen = seen.get_mut(index).ok_or("invalid parameter axis")?;
        if *seen {
            return Err("a parameter axis was supplied twice".into());
        }
        *seen = true;
        if current.banks[index] != *bank {
            writes.push(requests::parameter_bank(*axis, *bank).map_err(|e| e.to_string())?);
        }
    }
    if !writes.is_empty() {
        writes.push(requests::parameters_activate());
    }
    Ok(writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::{snapshot_with, verified};
    use openlaser_protocol::feedback::Runtime;

    fn observed(parameters: &Verified) -> Snapshot {
        let mut snapshot = snapshot_with(&[(16, 100_000)], &[], &[]);
        let mut words = [0; 120];
        for (axis, bank) in parameters.banks.iter().enumerate() {
            words[50 + axis * 14..50 + (axis + 1) * 14].copy_from_slice(bank);
        }
        snapshot.runtime = Runtime::decode(&words).unwrap();
        snapshot.system = openlaser_protocol::feedback::System::decode(&parameters.system).unwrap();
        snapshot.pwm = parameters.pwm;
        snapshot
    }

    /// Initialization verifies system/PWM values as well as banks, refuses
    /// drift before its first write, and checks admission before every write.
    #[test]
    fn initialization_verifies_system_words_and_each_write_needs_fresh_admission() {
        let before = verified();
        let mut expected = before;
        expected.system[13] = 0x1000_0000;
        expected.pwm[1] = 9;
        let plan = Initialization {
            before,
            expected,
            writes: vec![
                Write { address: 50026, words: vec![expected.system[13]] },
                Write { address: 50108, words: vec![9] },
                requests::parameters_activate(),
            ],
            idle_outputs: 0,
        };
        let mut operation = Parameters::initialize(plan.clone());
        let mut no_reference = observed(&before);
        no_reference.head = openlaser_protocol::feedback::Head::decode(&[
            0,
            1 << 12,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ])
        .unwrap();
        assert!(matches!(operation.step(&no_reference, None, Instant::now()), Ok(Step::Send(_))));
        assert!(operation.step(&observed(&before), Some("E-stop"), Instant::now()).is_err());
        assert!(operation.verified().is_none());

        let mut operation = Parameters::initialize(plan.clone());
        let mut drift = before;
        drift.pwm[0] = 6;
        assert!(operation.step(&observed(&drift), None, Instant::now()).is_err());

        for matched in [false, true] {
            let mut operation = Parameters::initialize(plan.clone());
            for _ in 0..3 {
                assert!(matches!(
                    operation.step(&observed(&before), None, Instant::now()),
                    Ok(Step::Send(_))
                ));
            }
            let result = operation.step(
                &observed(if matched { &expected } else { &before }),
                None,
                Instant::now(),
            );
            assert_eq!(result.is_ok(), matched);
            assert_eq!(operation.verified(), matched.then_some(expected));
        }
    }

    #[test]
    fn apply_preserves_unprojected_words_and_requires_matching_readback() {
        let mut before = verified();
        before.banks[0][8] = 0xfeed_beef;
        let mut expected = before;
        expected.banks[0][2] += 1000;
        let mut apply = Parameters::new(Some(&before), Some(vec![(0, expected.banks[0])]));
        let snapshot = observed(&before);
        assert_eq!(
            apply.step(&snapshot, None, Instant::now()).unwrap(),
            Step::Send(vec![requests::parameter_bank(0, expected.banks[0]).unwrap()])
        );
        assert_eq!(
            apply.step(&observed(&expected), None, Instant::now()).unwrap(),
            Step::Send(vec![requests::parameters_activate()])
        );
        assert_eq!(
            apply.step(&observed(&expected), None, Instant::now()).unwrap(),
            Step::Finish(vec![])
        );
        assert_eq!(apply.verified(), Some(expected));
        assert_eq!(apply.verified().unwrap().banks[0][8], 0xfeed_beef);
        assert!(!apply.unchanged());

        let mut mismatch = Parameters::new(Some(&before), Some(vec![(0, expected.banks[0])]));
        mismatch.step(&snapshot, None, Instant::now()).unwrap();
        mismatch.step(&snapshot, None, Instant::now()).unwrap();
        assert!(mismatch.step(&snapshot, None, Instant::now()).is_err());
        assert_eq!(mismatch.verified(), None);
    }

    #[test]
    fn drift_and_each_write_guard_refuse_without_accepting_old_parameters() {
        let before = verified();
        let mut desired = before.banks[0];
        desired[2] += 1000;
        let mut drifted = before;
        drifted.banks[2][8] = 17;
        let mut apply = Parameters::new(Some(&before), Some(vec![(0, desired)]));
        assert!(apply.step(&observed(&drifted), None, Instant::now()).is_err());
        assert_eq!(apply.verified(), None);

        for at in [0, 1] {
            let mut apply = Parameters::new(Some(&before), Some(vec![(0, desired)]));
            if at == 1 {
                apply.step(&observed(&before), None, Instant::now()).unwrap();
            }
            assert!(apply.step(&observed(&before), Some("door"), Instant::now()).is_err());
            assert_eq!(apply.verified(), None);
        }
        assert!(writes(&before, &[(0, desired), (0, desired)]).is_err());
        assert!(writes(&before, &[(5, desired)]).is_err());
        let mut read = Parameters::new(Some(&before), None);
        read.step(&observed(&before), None, Instant::now()).unwrap();
        assert!(read.unchanged());
    }
}
