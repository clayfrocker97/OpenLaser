// SPDX-License-Identifier: GPL-3.0-or-later

//! The plant behind the simulator: feedback words and mechanics for every
//! request the host sends. Values are truncated to integer words the way
//! the controller reports them.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the plant reports integer words truncated from its model"
)]

use super::Fault;
use crate::alarms::Rule;
use openlaser_protocol::alarms::detail_word;
use openlaser_protocol::records::{self, Record};
use openlaser_protocol::registers::{AXIS_COUNT, PARAMETER_BANK_WORDS};
use openlaser_protocol::requests::Write;
use openlaser_protocol::{Direction, Frame, Function};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

/// Controller units per millimetre.
pub const SCALE: f64 = 1000.;
/// The FIFO capacity in bytes.
pub const CAPACITY: u32 = 100_000;
/// Where the sheet is: below the head's upper reference, in microns.
const SURFACE: f64 = 20_000.;
/// Head units per second for one tenth of head speed.
const HEAD_SPEED: f64 = 100.;
/// The minimum time a head search or calibration takes, in seconds, so the
/// host sees its transitions.
const HEAD_SETTLE: f64 = 0.5;
/// The minimum time an axis search takes.
const AXIS_SETTLE: f64 = 0.35;
/// The dual-drive detail bit a reset clears.
const DUAL_DRIVE_BIT: u32 = 1 << 5;

/// The plant as seen from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// Every axis in millimetres, the head as axis 3.
    pub position_mm: [f64; 5],
    /// Which axes hold a reference.
    pub referenced: [bool; 5],
    /// The head command in progress.
    pub head_command: u32,
    /// Controller groups 1 and 2, then the head alarm word.
    pub alarms: [u32; 3],
    /// Whether the head controller reports a usable reference.
    pub head_referenced: bool,
    /// The standard output bank.
    pub outputs: u32,
    /// The extended output bank.
    pub extended_outputs: u32,
    /// The two analog channels.
    pub analog: [u32; 2],
    /// The two PWM banks: frequency, duty, enabled.
    pub pwm: [[u32; 3]; 2],
    /// Whether the FIFO is executing.
    pub running: bool,
    /// Bytes queued in the FIFO.
    pub queued_bytes: u32,
    /// Records executed since the counter reset.
    pub executed: u64,
    /// The application item being executed.
    pub item: u32,
    /// The last refused request, if any.
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct Travel {
    target: f64,
    speed: f64,
    home: bool,
    remaining_min: f64,
    program_clock: bool,
}

#[derive(Clone, Copy)]
struct Limit {
    bit: u8,
    position: f64,
}

struct Queued {
    record: Record,
    byte_cost: u32,
}

#[allow(clippy::struct_excessive_bools, reason = "the plant's switches are independent")]
pub(super) struct Plant {
    pub(super) banks: [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT],
    position: [f64; AXIS_COUNT],
    travel: [Option<Travel>; AXIS_COUNT],
    pub(super) referenced: [bool; AXIS_COUNT],
    head_command: u32,
    head_status: u32,
    head_limits: u32,
    pub(super) head_reference_ready: bool,
    pub(super) calibration_quality: u32,
    calibration_return: bool,
    head_sequence: VecDeque<(f64, f64, f64)>,
    head_hold: f64,
    cycle_us: u32,
    pwm_sync_port: u32,
    primary_pwm_sync: u32,
    system_parameters: [u32; 26],
    pub(super) product: u32,
    digital: u32,
    extended: u32,
    pwm: [[u32; 3]; 2],
    pwm_bank: usize,
    analog: [u32; 2],
    mask: u32,
    queue: VecDeque<Queued>,
    queue_bytes: u32,
    running: bool,
    stamp: u32,
    item: u32,
    item_progress: u32,
    executed: u64,
    delay: f64,
    wait: Option<(u8, f64)>,
    fractional_cycle: f64,
    pub(super) time_scale: f64,
    pub(super) faults: BTreeSet<Fault>,
    pub(super) raw_alarms: [u32; 3],
    pub(super) axis_alarms: [u32; 25],
    latched_axis_alarms: [u32; AXIS_COUNT],
    axis_limits: [Option<Limit>; AXIS_COUNT],
    pub(super) rules: Vec<Rule>,
    pub(super) input_overrides: BTreeMap<u8, bool>,
    pub(super) drop_ack_after: Option<usize>,
    pub(super) reply_delay: Duration,
    pub(super) drop_reads: BTreeMap<u32, usize>,
    /// Writes whose words start with a prefix are refused, controller
    /// busy, this many more times.
    pub(super) refusals: Vec<(Vec<u32>, usize)>,
    pub(super) read_counts: BTreeMap<u32, usize>,
    last_error: Option<String>,
    pub(super) writes: Vec<Write>,
}

impl Plant {
    pub(super) fn new() -> Self {
        let mut bank = [0; PARAMETER_BANK_WORDS];
        bank[1] = (-500i32).cast_unsigned();
        bank[2] = 1_000_000;
        bank[3] = 2000;
        bank[5] = 50_000;
        bank[6] = 10_000;
        bank[9] = 8000;
        bank[10] = 8000;
        bank[11] = 8000;
        Self {
            banks: [bank; AXIS_COUNT],
            position: [0.; AXIS_COUNT],
            travel: [None; AXIS_COUNT],
            referenced: [false; AXIS_COUNT],
            head_command: 0,
            head_status: 0,
            head_limits: 0,
            head_reference_ready: false,
            calibration_quality: 0x10,
            calibration_return: false,
            head_sequence: VecDeque::new(),
            head_hold: 0.,
            cycle_us: 250,
            pwm_sync_port: 9,
            primary_pwm_sync: 0,
            system_parameters: [0; 26],
            product: 103,
            digital: 0,
            extended: 0,
            pwm: [[0; 3]; 2],
            pwm_bank: 0,
            analog: [0; 2],
            mask: 0,
            queue: VecDeque::new(),
            queue_bytes: 0,
            running: false,
            stamp: 0,
            item: u32::MAX,
            item_progress: 0,
            executed: 0,
            delay: 0.,
            wait: None,
            fractional_cycle: 0.,
            time_scale: 1.,
            faults: BTreeSet::new(),
            raw_alarms: [0; 3],
            axis_alarms: [0; 25],
            latched_axis_alarms: [0; AXIS_COUNT],
            axis_limits: [None; AXIS_COUNT],
            rules: Vec::new(),
            input_overrides: BTreeMap::new(),
            drop_ack_after: None,
            reply_delay: Duration::ZERO,
            refusals: Vec::new(),
            drop_reads: BTreeMap::new(),
            read_counts: BTreeMap::new(),
            last_error: None,
            writes: Vec::new(),
        }
    }

    pub(super) fn view(&mut self) -> View {
        View {
            position_mm: self.position.map(|units| units / SCALE),
            referenced: self.referenced,
            head_command: self.head_command,
            alarms: self.alarm_words(),
            head_referenced: self.head_reference_ready,
            outputs: self.digital,
            extended_outputs: self.extended,
            analog: self.analog,
            pwm: self.pwm,
            running: self.running,
            queued_bytes: self.queue_bytes,
            executed: self.executed,
            item: self.item,
            last_error: self.last_error.clone(),
        }
    }

    /// Answers one datagram, or nothing when the request is refused, the
    /// acknowledgement is dropped, or communications are faulted.
    pub(super) fn receive(&mut self, bytes: &[u8]) -> Option<Vec<u8>> {
        if self.faults.contains(&Fault::Communications) {
            return None;
        }
        let frame = match Frame::decode(bytes, Direction::Request) {
            Ok(frame) => frame,
            Err(error) => {
                self.fail(format!("invalid request: {error}"));
                return None;
            }
        };
        let outcome = match frame.function {
            Function::Read => {
                *self.read_counts.entry(frame.address).or_default() += 1;
                if let Some(remaining) = self.drop_reads.get_mut(&frame.address)
                    && *remaining > 0
                {
                    *remaining -= 1;
                    return None;
                }
                self.read(frame.address, frame.count)
            }
            Function::Write => {
                if let Some((_, remaining)) = self
                    .refusals
                    .iter_mut()
                    .find(|(prefix, remaining)| *remaining > 0 && frame.values.starts_with(prefix))
                {
                    *remaining -= 1;
                    return Some(openlaser_protocol::refusal(frame.transaction, frame.function, 6));
                }
                self.writes.push(Write { address: frame.address, words: frame.values.clone() });
                self.write(frame.address, &frame.values).map(|()| Vec::new())
            }
        };
        let values = match outcome {
            Ok(values) => values,
            Err(error) => {
                self.fail(error);
                return None;
            }
        };
        if frame.function == Function::Write
            && let Some(remaining) = self.drop_ack_after
        {
            self.drop_ack_after = remaining.checked_sub(1);
            if remaining == 0 {
                return None;
            }
        }
        frame.response(values).and_then(|response| response.encode()).ok()
    }

    /// A refused request: the plant stops what it was doing and remembers
    /// why. No firmware alarm is invented for it.
    fn fail(&mut self, error: String) {
        self.last_error = Some(error);
        self.running = false;
        self.travel = [None; AXIS_COUNT];
        self.head_command = 0;
        self.head_sequence.clear();
    }

    // Feedback ------------------------------------------------------------------

    fn alarm_words(&self) -> [u32; 3] {
        let mut words = self.raw_alarms;
        words[2] |= self.head_limits;
        for axis in 0..25 {
            let latched = self.latched_axis_alarms.get(axis).is_some_and(|word| *word != 0);
            let contact = self.axis_limits.get(axis).is_some_and(Option::is_some);
            if self.axis_alarms[axis] != 0 || latched || contact {
                words[0] |= 1 << axis;
            }
        }
        let bits = [
            (Fault::EmergencyStop, 0, 30),
            (Fault::Servo, 0, 29),
            (Fault::Bus, 0, 25),
            (Fault::FifoStarvation, 1, 5),
            (Fault::IllegalCommand, 1, 0),
            (Fault::HeadFollow, 2, 8),
            (Fault::HeadTouch, 2, 5),
            (Fault::HeadUpperLimit, 2, 0),
            (Fault::HeadLowerLimit, 2, 1),
        ];
        for (fault, bank, bit) in bits {
            if self.faults.contains(&fault) {
                words[bank] |= 1 << bit;
            }
        }
        if !self.head_reference_ready {
            words[2] |= 1 << 12;
        }
        if words[2] != 0 {
            words[0] |= 1 << 24;
        }
        words
    }

    fn inputs(&self) -> u32 {
        let mut inputs = 0;
        for rule in &self.rules {
            if rule.active_low && (1..=24).contains(&rule.input) {
                inputs |= 1 << (rule.input - 1);
            }
        }
        for (&input, &level) in &self.input_overrides {
            if (1..=24).contains(&input) {
                let mask = 1 << (input - 1);
                inputs = (inputs & !mask) | if level { mask } else { 0 };
            }
        }
        inputs
    }

    fn read(&self, address: u32, count: u16) -> Result<Vec<u32>, String> {
        let mut words = vec![0; usize::from(count)];
        let alarms = self.alarm_words();
        match (address, count) {
            (50_000, 26) => {
                words.copy_from_slice(&self.system_parameters);
                words[5] = self.cycle_us;
                words[17] = SCALE as u32;
            }
            (50_108, 1) => words[0] = self.pwm_sync_port,
            (50_106, 2) => words.copy_from_slice(&[self.primary_pwm_sync, self.pwm_sync_port]),
            (5000, 1) | (11_000, 41) => {}
            (1032, 1) => words[0] = CAPACITY,
            // The status block, or any prefix of it such as the identity pair.
            (1000, 1..=36) => {
                let mut status = [0; 36];
                status[0] = self.product;
                status[1] = 20_177;
                status[4] = self.inputs();
                status[5] = self.digital;
                status[6] = alarms[0];
                status[7] = alarms[1];
                status[15] = self.stamp;
                status[16] = CAPACITY - self.queue_bytes;
                status[19] = u32::from(
                    self.running
                        || (self.queue_bytes > 0
                            && self.faults.contains(&Fault::StoppedFifoActivity)),
                );
                status[22] = self.extended;
                status[23] = self.item;
                status[24] = self.item_progress;
                let count = words.len();
                words.copy_from_slice(&status[..count]);
            }
            (10_000, 18) => {
                words[1] = alarms[2];
                words[2] =
                    if self.head_reference_ready { 0x8000_0000 } else { 0 } | self.head_status;
                words[3] = self.head_command;
                words[6] = (self.position[3].trunc() as i32).cast_unsigned();
            }
            (2000, 50) | (60_001, 120) => {
                self.axis_feedback(address, &mut words);
            }
            (50_200, 100) => {
                for axis in 0..AXIS_COUNT {
                    let bank = axis * 20;
                    words[bank..bank + PARAMETER_BANK_WORDS].copy_from_slice(&self.banks[axis]);
                }
            }
            (address, 14) if (0..AXIS_COUNT as u32).any(|axis| address == 50_200 + axis * 40) => {
                words.copy_from_slice(&self.banks[((address - 50_200) / 40) as usize]);
            }
            _ => return Err(format!("the plant does not model read {address}/{count}")),
        }
        self.project_detail_alarms(address, &mut words);
        Ok(words)
    }

    fn project_detail_alarms(&self, address: u32, words: &mut [u32]) {
        // Detail indices above the five axes alias other caches; the injected
        // detail bits are projected into the words the host reads them from.
        for axis in 5..=20 {
            let Some(source) = detail_word(axis) else { continue };
            let index = match (address, source.group) {
                (5000, 1) | (50_000, 5) | (10_000, 7) | (11_000, 8) => source.index,
                (50_200, 4) => {
                    (source.index / PARAMETER_BANK_WORDS) * 20 + source.index % PARAMETER_BANK_WORDS
                }
                _ => continue,
            };
            if self.axis_alarms[axis] != 0
                && let Some(word) = words.get_mut(index)
            {
                *word = (*word & !0x3f) | self.axis_alarms[axis];
            }
        }
    }

    fn axis_feedback(&self, address: u32, words: &mut [u32]) {
        for axis in 0..AXIS_COUNT {
            let base = axis * 10;
            words[base] = 0x0200_0000 | if self.referenced[axis] { 0xc000 } else { 0 };
            if let Some(travel) = &self.travel[axis] {
                words[base] =
                    if travel.home { 0x0201_c000 } else { 0x0301_0000 | (words[base] & 0xffff) };
                let speed = (travel.target - self.position[axis]).signum() * travel.speed;
                words[base + 1] = (speed.trunc() as i32).cast_unsigned();
            }
            words[base + 2] = (self.position[axis].trunc() as i32).cast_unsigned();
            if axis == 0
                && !self.running
                && self.queue_bytes > 0
                && self.faults.contains(&Fault::StopFeedbackBusy)
            {
                words[base + 1] = 1;
            }
            words[base + 3] = words[base + 2];
            words[base + 5] = words[base + 2];
            words[base] |= self.axis_faults(axis);
            if address == 60_001 {
                let config = 50 + axis * PARAMETER_BANK_WORDS;
                words[config..config + PARAMETER_BANK_WORDS].copy_from_slice(&self.banks[axis]);
            }
        }
    }

    // Requests --------------------------------------------------------------------

    fn write_configuration(&mut self, address: u32, words: &[u32]) -> Result<bool, String> {
        match (address, words) {
            (5001, [9999, 9, 65535, 0]) => {}
            (60000, map) if map.len() == 60 => {}
            (50106, [port]) => self.primary_pwm_sync = *port,
            (50108, [port]) => self.pwm_sync_port = *port,
            (address @ 50000..=50050, data) if address % 2 == 0 => {
                let start = ((address - 50000) / 2) as usize;
                let end = start + data.len();
                let target =
                    self.system_parameters.get_mut(start..end).ok_or("system write range")?;
                target.copy_from_slice(data);
            }
            (address, bank)
                if bank.len() == PARAMETER_BANK_WORDS
                    && (0..AXIS_COUNT as u32).any(|axis| address == 50_200 + axis * 40) =>
            {
                self.banks[((address - 50_200) / 40) as usize].copy_from_slice(bank);
            }
            (address, [lower, upper]) if (0..2).any(|axis| address == 50_202 + axis * 40) => {
                let axis = ((address - 50_202) / 40) as usize;
                self.banks[axis][1] = *lower;
                self.banks[axis][2] = *upper;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn write(&mut self, address: u32, words: &[u32]) -> Result<(), String> {
        if self.write_configuration(address, words)? || self.write_motion(address, words)? {
            return Ok(());
        }
        match (address, words) {
            // Parameter activation and bus reset change nothing the plant
            // models; the banks were already written.
            (100, [9999 | 8888]) | (101, [118, 5, 0 | 1]) => {}
            (101, [9999, 8, mask]) => {
                for axis in 0..AXIS_COUNT {
                    if mask & (1 << axis) != 0 {
                        self.latched_axis_alarms[axis] &= !DUAL_DRIVE_BIT;
                    }
                }
            }
            // The common reset clears the plant's own limit latches; an
            // injected condition stays.
            (101, [9999, 5, 0, 0]) => {
                self.latched_axis_alarms = [0; AXIS_COUNT];
                self.last_error = None;
            }
            (103, [1]) => self.clear_fifo(),
            (103, [2]) => self.running = true,
            (103, [3]) => {
                self.running = false;
                self.fractional_cycle = 0.;
            }
            (102, [stamp, records @ ..]) if !records.is_empty() => self.upload(*stamp, records)?,
            (101, [9999, outputs @ ..]) => self.output(outputs)?,
            _ => return Err(format!("the plant does not model write {address}: {words:x?}")),
        }
        Ok(())
    }

    fn write_motion(&mut self, address: u32, words: &[u32]) -> Result<bool, String> {
        if address != 101 {
            return Ok(false);
        }
        if self.write_head(words) {
            return Ok(true);
        }
        match words {
            [3, axis, speed, _, _, distance] => {
                self.axis_move(*axis as usize, *speed, distance.cast_signed(), false)?;
            }
            [5, 3, speed, _, _, x, y, 0, 0] => {
                let (dx, dy) = (f64::from(x.cast_signed()), f64::from(y.cast_signed()));
                let length = dx.hypot(dy).max(1.);
                let speed = f64::from(*speed);
                self.axis_move(
                    0,
                    (speed * dx.abs() / length).max(1.) as u32,
                    x.cast_signed(),
                    false,
                )?;
                self.axis_move(
                    1,
                    (speed * dy.abs() / length).max(1.) as u32,
                    y.cast_signed(),
                    false,
                )?;
            }
            [2, 3, 0] => {
                for axis in 0..2 {
                    self.axis_move(axis, self.banks[axis][5].max(1000), 0, true)?;
                }
            }
            [1, mask, 2, _, _] => {
                for axis in 0..AXIS_COUNT {
                    if mask & (1 << axis) != 0 {
                        self.travel[axis] = None;
                    }
                }
                self.running = false;
                self.fractional_cycle = 0.;
                self.head_command = 0;
                self.head_sequence.clear();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn write_head(&mut self, words: &[u32]) -> bool {
        match words {
            [102] => {
                self.head_sequence.clear();
                self.head_move(102, 300, 0., true);
            }
            [107] => {
                self.head_sequence.clear();
                self.calibration_return = true;
                self.head_move(107, 300, SURFACE, false);
            }
            [101] => {
                self.travel[3] = None;
                self.head_sequence.clear();
                self.head_command = 0;
                self.head_hold = 0.;
            }
            [109, speed, travel] => {
                self.head_sequence.clear();
                self.head_move(
                    109,
                    *speed,
                    self.position[3] + f64::from(travel.cast_signed()),
                    false,
                );
            }
            [103, speed, height] => {
                // An absolute height below the origin, like FIFO opcode 0x67.
                self.head_sequence.clear();
                self.head_move(103, *speed, f64::from(*height), false);
            }
            _ => return false,
        }
        true
    }

    fn clear_fifo(&mut self) {
        self.queue.clear();
        self.queue_bytes = 0;
        self.running = false;
        self.stamp = 0;
        self.delay = 0.;
        self.wait = None;
        self.fractional_cycle = 0.;
    }

    fn upload(&mut self, stamp: u32, words: &[u32]) -> Result<(), String> {
        let decoded = records::decode(words).map_err(|error| error.to_string())?;
        let block_bytes = records::block_bytes(words.len());
        if self.queue_bytes + block_bytes > CAPACITY {
            return Err("the FIFO capacity is exceeded".into());
        }
        let count = decoded.len();
        for (index, record) in decoded.into_iter().enumerate() {
            let record_bytes = 4 * record.words().len() as u32;
            let byte_cost = record_bytes
                + if index + 1 == count { block_bytes - 4 * words.len() as u32 } else { 0 };
            self.queue.push_back(Queued { record, byte_cost });
        }
        self.queue_bytes += block_bytes;
        self.stamp = stamp;
        Ok(())
    }

    fn output(&mut self, words: &[u32]) -> Result<(), String> {
        match words {
            [2, mask, value] => self.digital = (self.digital & !mask) | (value & mask),
            [13, mask, value] => self.extended = (self.extended & !mask) | (value & mask),
            [selector @ (3 | 17), frequency, duty, enabled] if *duty <= 100 && *enabled <= 1 => {
                self.pwm_bank = usize::from(*selector == 17);
                self.pwm[self.pwm_bank] = [*frequency, *duty, *enabled];
            }
            [4, channel, value] if *channel < 2 && *value <= 10_000 => {
                self.analog[*channel as usize] = *value;
            }
            [18, 0 | 9 | 10 | 1000 | 1001] => {}
            [1, mask, 0] => self.mask = *mask,
            [16] => self.executed = 0,
            _ => return Err(format!("the plant does not model output {words:x?}")),
        }
        Ok(())
    }

    fn axis_move(
        &mut self,
        axis: usize,
        speed: u32,
        distance: i32,
        home: bool,
    ) -> Result<(), String> {
        if axis >= AXIS_COUNT || speed == 0 {
            return Err("invalid axis movement".into());
        }
        let (lead, pulses) = (f64::from(self.banks[axis][9]), f64::from(self.banks[axis][10]));
        if lead <= 0. || pulses <= 0. {
            return Err("the axis pulse conversion is not configured".into());
        }
        let travel = (f64::from(distance) * pulses / lead).trunc() * lead / pulses;
        let faults = self.axis_faults(axis);
        let recovery = faults & 0xf != 0;
        if faults != 0
            && (home
                || faults & !0xf != 0
                || (distance > 0 && faults & 0b0101 != 0)
                || (distance < 0 && faults & 0b1010 != 0))
        {
            return Err(format!("axis {axis} can only move away from its active limit"));
        }
        let mut target = if home { 0. } else { self.position[axis] + travel };
        let minimum = f64::from(self.banks[axis][1].cast_signed());
        let maximum = f64::from(self.banks[axis][2].cast_signed());
        // The host commands from the integer coordinate it reads; a held jog
        // may aim exactly at the limit from there.
        let reported_target = self.position[axis].trunc() + f64::from(distance);
        if !home
            && (minimum..=maximum).contains(&reported_target)
            && target >= minimum - 1.
            && target <= maximum + 1.
        {
            target = target.clamp(minimum, maximum);
        }
        let approaching_range = recovery
            && (!self.referenced[axis]
                || (self.position[axis] < minimum
                    && target > self.position[axis]
                    && target <= maximum)
                || (self.position[axis] > maximum
                    && target < self.position[axis]
                    && target >= minimum));
        if !home && !approaching_range && (target < minimum || target > maximum) {
            self.latched_axis_alarms[axis] |= 1 << if target < minimum { 3 } else { 2 };
            return Err(format!("axis {axis} reached its software limit"));
        }
        self.travel[axis] = Some(Travel {
            target,
            speed: f64::from(speed),
            home,
            remaining_min: if home { AXIS_SETTLE } else { 0. },
            program_clock: false,
        });
        Ok(())
    }

    fn head_move(&mut self, command: u32, speed: u32, target: f64, home: bool) {
        let limits = self.alarm_words()[2] & 0xf;
        if limits != 0
            && !(command == 109
                && ((target > self.position[3] && limits & 0b1010 == 0)
                    || (target < self.position[3] && limits & 0b0101 == 0)))
        {
            self.fail("the head can only move away from its active limit".into());
            return;
        }
        if command != 107 {
            self.calibration_return = false;
        }
        self.head_status = 0x10;
        self.head_command = command;
        self.travel[3] = Some(Travel {
            target,
            speed: f64::from(speed.max(1)) * HEAD_SPEED,
            home,
            remaining_min: if home || command == 107 { HEAD_SETTLE } else { 0. },
            program_clock: self.running,
        });
    }

    /// A physical switch releases after 0.1 mm of travel away. Explicit
    /// injected faults remain active to represent a stuck input.
    pub(super) fn place_head_on_limit(&mut self, upper: bool) {
        self.position[3] = if upper { 0. } else { SURFACE + 100_000. };
        self.head_limits = if upper { 1 } else { 2 };
        self.head_reference_ready = false;
        self.travel[3] = None;
        self.head_command = 0;
        self.head_sequence.clear();
    }

    fn axis_faults(&self, axis: usize) -> u32 {
        self.axis_alarms[axis]
            | self.latched_axis_alarms[axis]
            | self.axis_limits[axis].map_or(0, |limit| 1 << limit.bit)
    }

    /// A modeled limit releases after 0.1 mm away from the contact point.
    /// Injected detail faults remain asserted until explicitly removed.
    pub(super) fn place_axis_on_limit(&mut self, axis: usize, positive: bool, software: bool) {
        if axis > 1 {
            return;
        }
        self.position[axis] =
            f64::from(self.banks[axis][if positive { 2 } else { 1 }].cast_signed());
        self.axis_limits[axis] = Some(Limit {
            bit: u8::from(!positive) + 2 * u8::from(software),
            position: self.position[axis],
        });
        self.referenced[axis] = false;
        self.travel[axis] = None;
    }

    // Time ------------------------------------------------------------------------

    pub(super) fn advance(&mut self, real_seconds: f64) {
        let dt = real_seconds * self.time_scale;
        self.advance_axes(real_seconds, dt);
        self.advance_head_sequence(dt);
        if !self.running || self.faults.contains(&Fault::AxisStall) {
            self.fractional_cycle = 0.;
            return;
        }
        self.advance_fifo(dt);
    }

    fn advance_axes(&mut self, real_seconds: f64, dt: f64) {
        for axis in 0..AXIS_COUNT {
            let stalled =
                self.faults.contains(&if axis == 3 { Fault::HeadStall } else { Fault::AxisStall });
            let Some(travel) = self.travel[axis].as_mut() else { continue };
            if stalled {
                continue;
            }
            let delta = travel.target - self.position[axis];
            // Program motion follows the program clock; an operator's move
            // keeps its real speed so a short hold stays short.
            let step = if travel.program_clock { dt } else { real_seconds };
            self.position[axis] += delta.signum() * (travel.speed * step).min(delta.abs());
            if let Some(limit) = self.axis_limits[axis] {
                let away = if limit.bit % 2 == 0 {
                    limit.position - self.position[axis]
                } else {
                    self.position[axis] - limit.position
                };
                if away >= 100. {
                    self.axis_limits[axis] = None;
                }
            }
            if axis == 3 && !(0. ..=SURFACE + 100_000.).contains(&self.position[axis]) {
                let upper = self.position[axis] < 0.;
                self.position[axis] = self.position[axis].clamp(0., SURFACE + 100_000.);
                self.travel[axis] = None;
                self.head_command = 0;
                self.head_sequence.clear();
                self.head_reference_ready = false;
                self.head_limits |= if upper { 1 } else { 2 };
                self.running = false;
                continue;
            }
            if axis == 3 {
                if self.position[axis] >= 100. {
                    self.head_limits &= !1;
                }
                if self.position[axis] <= SURFACE + 99_900. {
                    self.head_limits &= !2;
                }
            }
            travel.remaining_min = (travel.remaining_min - real_seconds).max(0.);
            if (self.position[axis] - travel.target).abs() >= 1e-7 || travel.remaining_min > 0. {
                continue;
            }
            let home = travel.home;
            self.travel[axis] = None;
            if home {
                self.referenced[axis] = true;
            }
            if axis == 3 {
                self.head_arrived();
            }
        }
    }

    fn head_arrived(&mut self) {
        if self.head_command == 107 && self.calibration_return {
            self.calibration_return = false;
            self.head_move(107, 300, 0., true);
            return;
        }
        self.head_status = match self.head_command {
            102 => {
                self.head_reference_ready = true;
                1
            }
            107 => self.calibration_quality,
            _ => 0x10,
        };
        self.head_command = 0;
    }

    fn advance_head_sequence(&mut self, dt: f64) {
        if self.travel[3].is_some() {
            return;
        }
        self.head_hold = (self.head_hold - dt).max(0.);
        if self.head_hold == 0.
            && let Some((target, speed, hold)) = self.head_sequence.pop_front()
        {
            self.head_move(106, speed as u32, target, false);
            self.head_hold = hold;
        }
    }

    fn advance_fifo(&mut self, dt: f64) {
        let mut budget = dt + self.fractional_cycle;
        self.fractional_cycle = 0.;
        if let Some((selector, timeout)) = &mut self.wait {
            let head_busy = self.travel[3].is_some()
                || !self.head_sequence.is_empty()
                || self.head_hold > 0.
                || self.faults.contains(&Fault::HeadStall);
            if head_busy {
                *timeout -= dt;
                if *timeout <= 0. {
                    let selector = *selector;
                    self.fail(format!("head feedback wait {selector} timed out"));
                }
                return;
            }
            self.wait = None;
        }
        loop {
            if self.delay > 0. {
                let step = budget.min(self.delay);
                self.delay -= step;
                budget -= step;
                if self.delay > 0. {
                    return;
                }
            }
            let Some(front) = self.queue.front() else { return };
            let cycle = if matches!(front.record, Record::Move { .. }) {
                f64::from(self.cycle_us) / 1_000_000.
            } else {
                0.
            };
            if budget + 1e-12 < cycle {
                self.fractional_cycle = budget;
                return;
            }
            budget = (budget - cycle).max(0.);
            let Some(queued) = self.queue.pop_front() else { return };
            self.queue_bytes -= queued.byte_cost;
            self.executed += 1;
            if let Err(error) = self.execute(&queued.record) {
                self.fail(error);
                return;
            }
            if self.wait.is_some() {
                return;
            }
        }
    }

    fn execute(&mut self, record: &Record) -> Result<(), String> {
        match *record {
            Record::Move { dx, dy, laser } => self.step(dx, dy, laser),
            Record::Barrier | Record::CrashProtect { .. } => Ok(()),
            Record::Item(tag) => {
                self.item = tag;
                self.item_progress = 0;
                Ok(())
            }
            Record::ContourControl(_)
            | Record::Pwm { .. }
            | Record::Outputs { .. }
            | Record::Analog { .. } => self.output(&record.words()[1..]),
            Record::Delay { millis } => {
                self.delay = f64::from(millis) / 1000.;
                Ok(())
            }
            Record::WaitStatus { selector, timeout_ms } => {
                self.wait = Some((selector, f64::from(timeout_ms) / 1000.));
                Ok(())
            }
            Record::HeightAbsolute { speed_tenths, height_microns } => {
                self.head_move(103, speed_tenths, f64::from(height_microns), false);
                Ok(())
            }
            Record::Follow { speed_tenths, height } => {
                self.head_move(104, speed_tenths, SURFACE - f64::from(height & 0xffff), false);
                Ok(())
            }
            Record::Lift { speed_tenths, delta_microns } => {
                let target = self.position[3] + f64::from(delta_microns);
                self.head_move(109, speed_tenths, target, false);
                Ok(())
            }
            Record::PierceHeight {
                follow_speed_tenths,
                height,
                lift_speed_tenths,
                lift_microns,
            } => {
                let pierce = SURFACE - f64::from(height & 0xffff);
                self.head_move(105, follow_speed_tenths, pierce, false);
                if lift_microns != 0 {
                    let lifted = pierce + f64::from(lift_microns);
                    self.head_sequence.push_back((lifted, f64::from(lift_speed_tenths), 0.));
                }
                Ok(())
            }
            Record::PierceRamp { speed_tenths, cut_height_microns, .. } => {
                self.head_move(108, speed_tenths, SURFACE - f64::from(cut_height_microns), false);
                Ok(())
            }
            Record::FrogJump { speed_tenths, lift, hold_ms, down_speed_tenths, target } => {
                self.head_sequence.clear();
                self.head_move(106, speed_tenths, self.position[3] - f64::from(lift), false);
                self.head_hold = f64::from(hold_ms) / 1000.;
                let down = SURFACE - f64::from(target & 0xffff);
                self.head_sequence.push_back((down, f64::from(down_speed_tenths), 0.));
                Ok(())
            }
        }
    }

    /// One interpolation step: the pulses on X and Y, then the laser fields
    /// in the layout the FIFO mask selects.
    fn step_axes(&mut self, dx: i8, dy: i8) -> Result<(), String> {
        for (axis, pulses) in [(0, dx), (1, dy)] {
            let increment = f64::from(pulses) * f64::from(self.banks[axis][9])
                / f64::from(self.banks[axis][10]);
            if !increment.is_finite() {
                return Err("invalid pulse scale during execution".into());
            }
            self.position[axis] += increment;
            let (minimum, maximum) = (
                f64::from(self.banks[axis][1].cast_signed()),
                f64::from(self.banks[axis][2].cast_signed()),
            );
            if self.position[axis] < minimum || self.position[axis] > maximum {
                self.latched_axis_alarms[axis] |=
                    1 << if self.position[axis] < minimum { 3 } else { 2 };
                return Err(format!("the program exceeded the axis {axis} limit"));
            }
        }
        Ok(())
    }

    fn step(&mut self, dx: i8, dy: i8, laser: [u8; 5]) -> Result<(), String> {
        self.item_progress = self.item_progress.saturating_add(1);
        self.step_axes(dx, dy)?;
        if self.mask & 0x7800_0000 == 0x7800_0000 {
            let fields = Record::analog_fields(laser);
            let port = fields.output.unsigned_abs();
            if fields.channel > 1 || fields.level > 10_000 || !(1..=10).contains(&port) {
                return Err("invalid scaled CO2 output fields".into());
            }
            self.analog[usize::from(fields.channel)] = u32::from(fields.level);
            let mask = 1 << (port - 1);
            self.digital = (self.digital & !mask) | if fields.output > 0 { mask } else { 0 };
        } else {
            let fields = Record::pulsed_fields(laser);
            let power = u32::from(fields.power);
            self.pwm[self.pwm_bank] = [u32::from(fields.frequency), power, u32::from(power != 0)];
            if self.pwm_bank == 1 && (1..=10).contains(&self.pwm_sync_port) {
                let mask = 1 << (self.pwm_sync_port - 1);
                self.digital = (self.digital & !mask) | if power != 0 { mask } else { 0 };
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_protocol::requests;

    #[test]
    fn startup_needs_a_reference_and_z_overtravel_reports_the_limit() {
        let mut plant = Plant::new();
        assert_eq!(plant.alarm_words(), [1 << 24, 0, 1 << 12]);
        assert!(!plant.head_reference_ready);
        plant.write(101, &[102]).unwrap();
        plant.advance(1.);
        assert!(plant.head_reference_ready);
        assert_eq!(plant.alarm_words(), [0; 3]);
        // The native Z+ command moves in the negative direction. A target
        // beyond the upper reference must produce a fault, never be clamped
        // into a successful move.
        plant.write(101, &[109, 300, (-1000i32).cast_unsigned()]).unwrap();
        plant.advance(1.);
        assert_eq!(plant.alarm_words(), [1 << 24, 0, 1 | (1 << 12)]);
        assert_eq!(plant.head_command, 0);
        assert!(!plant.head_reference_ready);
        assert_eq!(plant.position[3], 0.);
    }

    #[test]
    fn a_physical_z_limit_needs_motion_away_and_a_stuck_input_stays_active() {
        for upper in [true, false] {
            let mut plant = Plant::new();
            plant.place_head_on_limit(upper);
            let bit = if upper { 1 } else { 2 };
            plant.write(101, &requests::alarm_clear().words).unwrap();
            assert_eq!(plant.alarm_words()[2] & 3, bit);
            plant.write(101, &[102]).unwrap();
            plant.advance(1.);
            assert!(!plant.head_reference_ready);
            let travel: i32 = if upper { 1000 } else { -1000 };
            plant.write(101, &[109, 10, travel.cast_unsigned()]).unwrap();
            plant.advance(0.05);
            assert_eq!(plant.alarm_words()[2] & 3, bit);
            plant.advance(0.1);
            assert_eq!(plant.alarm_words()[2] & 3, 0);
            assert!(!plant.head_reference_ready, "moving off a limit is not a reference search");
            plant.faults.insert(if upper { Fault::HeadUpperLimit } else { Fault::HeadLowerLimit });
            plant.advance(1.);
            assert_eq!(plant.alarm_words()[2] & 3, bit, "a stuck input is never auto-cleared");
        }
    }

    #[test]
    fn xy_limit_inputs_release_only_after_motion_away() {
        for axis in 0..2 {
            for positive in [false, true] {
                for software in [false, true] {
                    let mut plant = Plant::new();
                    plant.place_axis_on_limit(axis, positive, software);
                    plant.write(101, &requests::alarm_clear().words).unwrap();
                    assert_ne!(plant.axis_faults(axis), 0);
                    let away = if positive { -1000 } else { 1000 };
                    assert!(plant.axis_move(axis, 1000, -away, false).is_err());
                    assert!(plant.axis_move(axis, 1000, 0, true).is_err());
                    plant.axis_move(axis, 1000, away, false).unwrap();
                    plant.advance(0.05);
                    assert_ne!(plant.axis_faults(axis), 0);
                    plant.advance(0.1);
                    assert_eq!(plant.axis_faults(axis), 0);
                    assert!(!plant.referenced[axis]);
                    plant.axis_alarms[axis] = if positive { 1 } else { 2 };
                    plant.advance(1.);
                    assert_ne!(plant.axis_faults(axis), 0, "an injected stuck input stays active");
                }
            }
        }
    }

    /// Uploaded records wait for the start command, then execute one step
    /// per interpolation cycle with the laser fields applied, delays hold
    /// execution, and the activity byte stays set until the FIFO is stopped.
    #[test]
    fn the_fifo_executes_records_in_order_at_the_cycle() {
        let mut plant = Plant::new();
        let program = records::encode(&[
            Record::Item(7),
            Record::pulsed(10, 3, records::PulsedFields { power: 40, frequency: 5000, tail: 0 }),
            Record::output(1, true).unwrap(),
            Record::Delay { millis: 10 },
            Record::pulsed(-4, -1, records::PulsedFields { power: 0, frequency: 5000, tail: 0 }),
            Record::output(1, false).unwrap(),
            Record::Barrier,
        ])
        .unwrap();
        plant.write(102, &[vec![1], program].concat()).unwrap();
        assert_eq!(plant.position, [0.; 5]);
        assert_eq!(plant.read(1000, 36).unwrap()[19], 0);
        plant.write(103, &[2]).unwrap();
        plant.advance(0.000_249);
        assert_eq!(plant.position, [0.; 5]);
        plant.advance(0.000_001);
        assert_eq!(&plant.position[..2], &[10., 3.]);
        assert_eq!(plant.pwm[0], [5000, 40, 1]);
        assert_eq!(plant.digital, 1);
        assert_eq!((plant.item, plant.item_progress), (7, 1));
        plant.advance(0.009_999);
        assert_eq!(&plant.position[..2], &[10., 3.]);
        plant.advance(0.000_251);
        assert_eq!(&plant.position[..2], &[6., 2.]);
        assert_eq!(plant.digital, 0);
        assert_eq!(plant.queue_bytes, 0);
        assert_eq!(plant.read(1000, 36).unwrap()[19], 1);
        plant.write(103, &[3]).unwrap();
        assert_eq!(plant.read(1000, 36).unwrap()[19], 0);
    }

    /// A jog moves at its commanded speed in real time whatever the program
    /// clock, a rapid stop ends it, and a target past the soft limit is
    /// refused with the latched limit bit.
    #[test]
    fn jogs_move_in_real_time_and_respect_the_limits() {
        let mut plant = Plant::new();
        plant.head_reference_ready = true;
        plant.time_scale = 20.;
        plant.write(101, &[3, 0, 50_000, 599, 5990, 1_000_000]).unwrap();
        for _ in 0..10 {
            plant.advance(0.02);
        }
        assert!((plant.position[0] - 10_000.).abs() < 1e-6);
        plant.write(101, &[1, 31, 2, 5999, 200_000]).unwrap();
        plant.advance(1.);
        assert!((plant.position[0] - 10_000.).abs() < 1e-6);
        assert!(plant.write(101, &[3, 0, 50_000, 599, 5990, 1_000_000]).is_err());
        assert_eq!(plant.read(1000, 36).unwrap()[6], 1);
        assert_eq!(plant.read(2000, 50).unwrap()[0] & 0x3f, 1 << 2);
        plant.write(101, &[9999, 5, 0, 0]).unwrap();
        assert_eq!(plant.read(1000, 36).unwrap()[6], 0);
    }

    /// Unmodelled requests and truncated records are refused rather than
    /// acknowledged, and leave nothing queued.
    #[test]
    fn unmodelled_requests_are_refused() {
        let mut plant = Plant::new();
        assert!(plant.write(101, &[123_456]).is_err());
        assert!(plant.write(102, &[0, 0x80bb8, 1]).is_err());
        assert!(plant.read(999, 1).is_err());
        assert_eq!(plant.queue_bytes, 0);
        assert!(plant.last_error.is_none(), "write refusals are recorded by the receiver");
    }
}
