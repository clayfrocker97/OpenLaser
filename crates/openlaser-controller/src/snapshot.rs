// SPDX-License-Identifier: GPL-3.0-or-later

//! One complete read of the controller's feedback.
//!
//! Reads the head, status, axes, system, parameter banks and PWM routes.
//! Banks are read directly because the native runtime monitor mapping may
//! not be configured until initialization. A failed read discards the snapshot.

use crate::link::{Link, LinkError};
use crate::session::Verified;
use openlaser_protocol::alarms::{self, DetailCache};
use openlaser_protocol::feedback::{Axes, AxisClass, FeedbackError, Head, Runtime, Status, System};
use openlaser_protocol::registers;
use openlaser_protocol::requests::Read;
use std::collections::VecDeque;
use std::time::Instant;

/// The controller's feedback at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The head controller block.
    pub head: Head,
    /// The status block.
    pub status: Status,
    /// The five axis records.
    pub axes: Axes,
    /// The system configuration.
    pub system: System,
    /// Runtime-shaped view assembled from axis records and direct bank reads.
    pub runtime: Runtime,
    /// The two PWM synchronization routes.
    pub pwm: [u32; 2],
    /// The FIFO capacity reported with this capture, in bytes.
    pub capacity: u32,
    /// When the first read began; feedback age includes the whole capture.
    pub taken: Instant,
}

impl Snapshot {
    /// Reads every block. When the status aggregates point at axes outside
    /// the standard records, the caches their detail words live in are read
    /// too and remembered in `details`.
    pub async fn capture(link: &mut Link, details: &mut DetailCache) -> Result<Self, LinkError> {
        let mut capture = Capture::new();
        while let Some(read) = capture.next() {
            let words = link.read(read).await?;
            if let Some(snapshot) = capture.accept(words, details)? {
                return Ok(snapshot);
            }
        }
        Err(LinkError::Io("the feedback capture is incomplete".into()))
    }

    /// The coordinate divisor: controller units per millimetre.
    pub fn divisor(&self) -> Result<i32, FeedbackError> {
        self.system.coordinate_divisor()
    }

    /// The complete configuration observed by this poll, for comparison
    /// with the configuration an operation was admitted under.
    pub fn parameters(&self) -> Result<Verified, FeedbackError> {
        Ok(Verified {
            scale: self.divisor()?,
            cycle_us: self.system.interpolation_cycle_us()?,
            banks: std::array::from_fn(|axis| *self.runtime.bank(axis)),
            system: std::array::from_fn(|word| self.system.word(word)),
            pwm: self.pwm,
        })
    }

    /// X, Y and Z in millimetres: axis records 0, 1 and 3, as the vendor
    /// displays them.
    pub fn position_mm(&self) -> Result<[f64; 3], FeedbackError> {
        let divisor = self.divisor()?;
        Ok([0, 1, 3].map(|axis| self.axes.axis(axis).position_mm(divisor)))
    }

    /// X and Y in controller units.
    #[must_use]
    pub fn position(&self) -> [i32; 2] {
        [self.axes.axis(0).position, self.axes.axis(1).position]
    }

    /// X and Y speeds in controller units.
    #[must_use]
    pub fn speed(&self) -> [i32; 2] {
        [self.axes.axis(0).speed, self.axes.axis(1).speed]
    }

    /// The native axis classifier over this snapshot.
    #[must_use]
    pub fn class(&self) -> AxisClass {
        self.axes.class(&self.status)
    }

    /// Whether both controller alarm aggregates are clear.
    #[must_use]
    pub const fn alarms_clear(&self) -> bool {
        self.status.alarm_group_1() == 0 && self.status.alarm_group_2() == 0
    }

    /// Missing head reference may accompany homing and manual positioning.
    /// Every other head fault and both controller fault groups still block.
    #[must_use]
    pub const fn positioning_alarms_clear(&self) -> bool {
        self.head.alarm_word() & 0xefff == 0
            && self.status.alarm_group_1()
                & if !self.head.referenced() || self.head.alarm_word() & (1 << 12) != 0 {
                    !(1 << 24)
                } else {
                    u32::MAX
                }
                == 0
            && self.status.alarm_group_2() == 0
    }

    /// Manual Z may move away from a reported upper or lower limit, and up
    /// from a nozzle touching the plate. The
    /// opposite limit, every unrelated head fault and other controller bits
    /// still block. Bit 24 alone is not evidence of a directional Z limit.
    #[must_use]
    pub const fn head_jog_alarms_clear(&self, up: bool) -> bool {
        let head = self.head.alarm_word();
        // Up also moves away from a nozzle touching the plate, head bit 5.
        let limits = if up { 0b10_1010 } else { 0b0101 };
        let accompanied = !self.head.referenced() || head & ((1 << 12) | limits) != 0;
        head & !((1 << 12) | limits) == 0
            && self.status.alarm_group_1() & if accompanied { !(1 << 24) } else { u32::MAX } == 0
            && self.status.alarm_group_2() == 0
    }

    /// The head's hardware and software upper/lower limit bits.
    #[must_use]
    pub const fn head_limit_alarms(&self) -> u32 {
        self.head.alarm_word() & 0xf
    }

    /// Whether either real X/Y record reports a hardware or software limit.
    #[must_use]
    pub const fn xy_limit_alarms(&self) -> bool {
        (self.axes.axis(0).status | self.axes.axis(1).status) & 0xf != 0
    }

    /// A manual X/Y jog may recover known limits. Other X/Y limits may stay
    /// asserted at a corner, but only an axis at a limit can begin recovery.
    /// An admitted recovery can finish its bounded move after its limit clears.
    #[must_use]
    pub fn xy_jog_alarms_clear(&self, axis: usize, positive: bool, recovering: bool) -> bool {
        if axis > 1 {
            return false;
        }
        if !self.xy_limit_alarms() {
            return self.positioning_alarms_clear()
                && self.axes.all().iter().all(|record| record.status.trailing_zeros() >= 6);
        }
        let head_summary = if !self.head.referenced() || self.head.alarm_word() & (1 << 12) != 0 {
            1 << 24
        } else {
            0
        };
        if self.head.alarm_word() & 0xefff != 0
            || self.status.alarm_group_1() & !(3 | head_summary) != 0
            || self.status.alarm_group_2() != 0
        {
            return false;
        }
        for index in 0..5 {
            let faults = self.axes.axis(index).status & 0x3f;
            let summary = self.status.alarm_group_1() & (1 << index) != 0;
            if index > 1 {
                if faults != 0 {
                    return false;
                }
                continue;
            }
            if faults & !0xf != 0 || (faults & 0b0101 != 0 && faults & 0b1010 != 0) {
                return false;
            }
            if summary && faults == 0 {
                return false;
            }
            if index == axis {
                let away_limits = if positive { 0b1010 } else { 0b0101 };
                if faults & !away_limits != 0 || (faults == 0 && !recovering) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether every output is off except those in `allowed`, a mask of the
    /// standard bank.
    #[must_use]
    pub fn outputs_off(&self, allowed: u16) -> bool {
        self.status.outputs() & !allowed == 0 && self.status.extended_outputs() == 0
    }

    /// Whether X and Y report no speed and no motion phase.
    #[must_use]
    pub const fn xy_stationary(&self) -> bool {
        let (x, y) = (self.axes.axis(0), self.axes.axis(1));
        x.speed == 0 && y.speed == 0 && x.phase() == 0 && y.phase() == 0
    }

    /// Whether the head reports no command in progress.
    #[must_use]
    pub const fn head_idle(&self) -> bool {
        self.head.command() == 0
    }

    /// Every axis reports zero speed and no motion phase, and the head
    /// has no active command. A coarse axis class alone does not mean this.
    #[must_use]
    pub fn stationary(&self) -> bool {
        self.axes.all().iter().all(|axis| axis.speed == 0 && axis.phase() == 0) && self.head_idle()
    }
}

/// A poll advanced one exchange at a time, so control requests can be
/// serviced between its reads. Partial feedback is never published.
pub(crate) struct Capture {
    reads: VecDeque<registers::Block>,
    words: Vec<Vec<u32>>,
    taken: Instant,
}

impl Capture {
    pub(crate) fn new() -> Self {
        Self {
            reads: [
                registers::HEAD,
                registers::STATUS,
                registers::AXES,
                registers::SYSTEM,
                registers::PARAMETERS,
                registers::FIFO_CAPACITY,
                registers::PWM_SYNC,
            ]
            .into(),
            words: Vec::with_capacity(7),
            taken: Instant::now(),
        }
    }

    pub(crate) fn next(&self) -> Option<Read> {
        self.reads.front().copied().map(Read::block)
    }

    pub(crate) fn accept(
        &mut self,
        words: Vec<u32>,
        details: &mut DetailCache,
    ) -> Result<Option<Snapshot>, LinkError> {
        let block = self
            .reads
            .pop_front()
            .ok_or_else(|| LinkError::Io("unexpected feedback reply".into()))?;
        details.observe(block.address, &words);
        if block == registers::STATUS {
            let status = decode(&words, Status::decode)?;
            self.reads.extend(alarms::additional_reads(status.alarm_group_1()));
        }
        if self.words.len() < 7 {
            self.words.push(words);
        }
        if !self.reads.is_empty() {
            return Ok(None);
        }
        self.snapshot().map(Some)
    }

    fn snapshot(&self) -> Result<Snapshot, LinkError> {
        let runtime = self.runtime_words()?;
        Ok(Snapshot {
            head: decode(&self.words[0], Head::decode)?,
            status: decode(&self.words[1], Status::decode)?,
            axes: decode(&self.words[2], Axes::decode)?,
            system: decode(&self.words[3], System::decode)?,
            runtime: decode(&runtime, Runtime::decode)?,
            pwm: self.words[6]
                .as_slice()
                .try_into()
                .map_err(|_| LinkError::Io("incomplete PWM route read".into()))?,
            capacity: self.words[5]
                .first()
                .copied()
                .filter(|capacity| *capacity >= openlaser_protocol::records::MIN_FIFO_CAPACITY)
                .ok_or_else(|| {
                    LinkError::Io("the FIFO capacity is below the vendor's minimum".into())
                })?,
            taken: self.taken,
        })
    }
    fn runtime_words(&self) -> Result<[u32; 120], LinkError> {
        // Read the banks by their real addresses. The fast block at 60001
        // only has this layout after a host has installed its monitor map.
        let mut runtime = [0; 120];
        if self.words[2].len() != 50 || self.words[4].len() != 100 {
            return Err(LinkError::Io("incomplete direct parameter read".into()));
        }
        runtime[..50].copy_from_slice(&self.words[2]);
        for axis in 0..5 {
            runtime[50 + axis * 14..50 + (axis + 1) * 14]
                .copy_from_slice(&self.words[4][axis * 20..axis * 20 + 14]);
        }
        Ok(runtime)
    }
}

fn decode<T>(
    words: &[u32],
    decoder: fn(&[u32]) -> Result<T, FeedbackError>,
) -> Result<T, LinkError> {
    decoder(words).map_err(|e| LinkError::Io(e.to_string()))
}

#[cfg(test)]
pub(crate) mod tests {
    //! Feedback builders shared by the operation tests.

    use super::*;
    use crate::bindings::{Bindings, HomeOutputs};
    use crate::session::Verified;
    use openlaser_core::LaserMode;
    use openlaser_protocol::registers::PARAMETER_BANK_WORDS;
    use openlaser_protocol::requests::{OutputBank, digital_outputs};
    use openlaser_protocol::sequences::{DualDrive, ModeSwitch, Shutdown};

    /// A snapshot with the MCC100 identity, a 250 µs cycle and 1000 units
    /// per millimetre, every other word zero unless overridden by index.
    pub(crate) fn snapshot_with(
        status: &[(usize, u32)],
        axes: &[(usize, u32)],
        head: &[(usize, u32)],
    ) -> Snapshot {
        let mut status_words = vec![0u32; 36];
        status_words[0] = 103;
        status_words[1] = 20_177;
        for &(index, word) in status {
            status_words[index] = word;
        }
        let mut axis_words = vec![0u32; 50];
        for &(index, word) in axes {
            axis_words[index] = word;
        }
        let mut head_words = vec![0u32; 18];
        for &(index, word) in head {
            head_words[index] = word;
        }
        let mut system_words = vec![0u32; 26];
        system_words[5] = 250;
        system_words[17] = 1000;
        let mut runtime_words = vec![0u32; 120];
        runtime_words[..50].copy_from_slice(&axis_words);
        Snapshot {
            head: Head::decode(&head_words).unwrap(),
            status: Status::decode(&status_words).unwrap(),
            axes: Axes::decode(&axis_words).unwrap(),
            system: System::decode(&system_words).unwrap(),
            runtime: Runtime::decode(&runtime_words).unwrap(),
            pwm: [0; 2],
            capacity: 100_000,
            taken: Instant::now(),
        }
    }

    /// A fiber machine with a head, the manual signal on port 1 and the
    /// origin-done output on port 2.
    pub(crate) fn bindings() -> Bindings {
        Bindings {
            mode: LaserMode::Fiber,
            head_enabled: true,
            shutdown: Shutdown {
                mode: LaserMode::Fiber,
                head_enabled: true,
                co2_skip_head_cancel: false,
                extended_outputs: false,
                co2_analog_channel: 0,
                gas_channels: [0; 3],
                point_laser_frequency: 0,
                rapid_deceleration: 5999,
                ports: Vec::new(),
                raise: None,
            },
            home: HomeOutputs { z_origin_done_port: 2, manual_signal_port: 1 },
            mode_switch: ModeSwitch {
                mode: LaserMode::Fiber,
                head_enabled: true,
                cleanup: vec![digital_outputs(OutputBank::Standard, 0x3ff, 0)],
                enable: None,
                xy_limits: [(-500, 1_000_000); 2],
            },
            dual_drive: Some(DualDrive { xy_limits: [(-500, 1_000_000); 2] }),
            rules: Vec::new(),
            co2_pwm_sync_port: None,
        }
    }

    /// Banks with limits of −500 to 1 000 000 units and one unit per pulse.
    pub(crate) fn verified() -> Verified {
        let mut bank = [0; PARAMETER_BANK_WORDS];
        bank[1] = (-500i32).cast_unsigned();
        bank[2] = 1_000_000;
        bank[9] = 8000;
        bank[10] = 8000;
        let mut parameters = snapshot_with(&[], &[], &[]).parameters().unwrap();
        parameters.banks = [bank; 5];
        parameters
    }

    /// The accessors read the words the builder was given: positions scaled
    /// by the divisor, outputs against an allowance, stationary axes and the
    /// native class.
    #[test]
    fn accessors_read_the_feedback_words() {
        let snapshot =
            snapshot_with(&[(5, 0b101), (19, 1)], &[(2, 12_500), (12, 3000), (32, 900)], &[]);
        assert_eq!(snapshot.position_mm().unwrap(), [12.5, 3., 0.9]);
        assert_eq!(snapshot.position(), [12_500, 3000]);
        assert!(!snapshot.outputs_off(0));
        assert!(snapshot.outputs_off(0b101));
        assert!(snapshot.xy_stationary());
        assert_eq!(snapshot.class(), AxisClass::Fifo);
        assert!(snapshot.alarms_clear() && snapshot.head_idle());
    }
}
