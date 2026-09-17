// SPDX-License-Identifier: GPL-3.0-or-later

//! The records a program is made of, and how they travel to the FIFO.
//!
//! A record is one header word followed by its payload: the header's upper
//! half is the payload length in bytes, its lower half the opcode. Records are
//! packed into blocks of at most a few hundred words, each block is uploaded
//! as one write to the program register with a stamp as its first word, and
//! the controller reports how much room it has left after every one.
//!
//! Evidence: NCModule `0x1004_3b10` (the converter that builds records and
//! flushes blocks), `0x1004_3b10` selector 1 and `0x1003_e990` (process
//! records), `0x1005_2ae0` (upload admission), MainApp `0x0045_ad10` and
//! `0x0045_b520` (motion record fields); the simulator's decoder in the
//! prototype accepted exactly these shapes.

mod decode;

use crate::feedback::Status;
use crate::requests::{LaserChannel, OutputBank, OutputPort};
use openlaser_core::LaserMode;

/// One program record.
///
/// The `Move` variant keeps its five laser bytes raw because the same bytes
/// mean different things in pulsed and analog programs; [`Record::pulsed`]
/// and [`Record::analog`] build them, [`PulsedFields`] and [`AnalogFields`]
/// read them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Record {
    /// Opcode `0x0BB8`: one interpolation step of `dx`, `dy` pulses with the
    /// laser fields for that step.
    Move {
        /// Signed pulses on the first axis.
        dx: i8,
        /// Signed pulses on the second axis.
        dy: i8,
        /// Payload bytes 2, 3, 4, 6 and 7; byte 5 is always zero.
        laser: [u8; 5],
    },
    /// Opcode `0x0BB9`, no payload: the barrier the converter appends after a
    /// contour and before process commands.
    Barrier,
    /// Opcode `0x0BBA`: tags the records that follow with an application
    /// item; the controller reports the running tag in the status block.
    Item(u32),
    /// Opcode `0x270F` selector 18: contour control. Zero closes the contour;
    /// 9 and 10 select those CO2 ports; 1000 and 1001 are the pierce markers.
    ContourControl(u16),
    /// Opcode `0x270F` selector 3 or 17: PWM on a laser channel. The output
    /// is enabled exactly when `power` is nonzero.
    Pwm {
        /// Which laser channel.
        channel: LaserChannel,
        /// Frequency in hertz, nonzero.
        frequency: u16,
        /// Power in percent, 0 to 100.
        power: u8,
    },
    /// Opcode `0x270F` selector 2 or 13: write the digital outputs of one bank.
    Outputs {
        /// Which bank.
        bank: OutputBank,
        /// The ports to write, bit 0 being the bank's first port.
        mask: u16,
        /// Their new values.
        values: u16,
    },
    /// Opcode `0x270F` selector 4: set an analog channel, 1 or 2, to a raw
    /// value of at most 10 000.
    Analog {
        /// Channel 1 or 2.
        channel: u8,
        /// The raw value.
        value: u32,
    },
    /// Opcode `0x07D1`: wait for `millis` milliseconds, at most ten minutes.
    Delay {
        /// The wait, in milliseconds.
        millis: u32,
    },
    /// Opcode `0x07D1` mode 3: wait for a controller status, selector 1 to 4,
    /// for at most `timeout_ms`.
    WaitStatus {
        /// Which status, 1 to 4.
        selector: u8,
        /// The timeout in milliseconds, nonzero.
        timeout_ms: u32,
    },
    /// Opcode `0x67`: move the head to an absolute height.
    HeightAbsolute {
        /// Head speed in tenths.
        speed_tenths: u32,
        /// Height in thousandths of a millimetre.
        height_microns: u32,
    },
    /// Opcode `0x68`: follow the sheet at a height.
    Follow {
        /// Head speed in tenths.
        speed_tenths: u32,
        /// The height word, prepacked in the head's units.
        height: u32,
    },
    /// Opcode `0x6D`: lift the head by a signed amount, negative being up.
    Lift {
        /// Head speed in tenths.
        speed_tenths: u32,
        /// The signed distance in thousandths of a millimetre.
        delta_microns: i32,
    },
    /// Opcode `0x69`: go to pierce height, then lift.
    PierceHeight {
        /// Follow speed in tenths.
        follow_speed_tenths: u32,
        /// The height word, prepacked.
        height: u32,
        /// Lift speed in tenths.
        lift_speed_tenths: u32,
        /// The signed lift in thousandths of a millimetre.
        lift_microns: i32,
    },
    /// Opcode `0x6C`: the final gradual-pierce descent handing over to cut height.
    PierceRamp {
        /// Head speed in tenths.
        speed_tenths: u32,
        /// The signed descent in thousandths of a millimetre.
        descent_microns: i32,
        /// The cut height in thousandths of a millimetre.
        cut_height_microns: u32,
    },
    /// Opcode `0x6A`: lift, hold, then follow back down between contours.
    FrogJump {
        /// Lift speed in tenths.
        speed_tenths: u32,
        /// The lift in head units.
        lift: u32,
        /// The hold at the top in milliseconds, at most ten minutes.
        hold_ms: u32,
        /// Descent speed in tenths.
        down_speed_tenths: u16,
        /// The follow target word after the descent.
        target: u32,
    },
    /// Opcode `0x76` selector 4: crash protection value and height.
    CrashProtect {
        /// The protection value.
        value: u32,
        /// The height word.
        height: u32,
    },
}

/// The laser fields of a pulsed-mode [`Record::Move`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PulsedFields {
    /// Power in percent.
    pub power: u8,
    /// Frequency in hertz.
    pub frequency: u16,
    /// The trailing field the general cut producer initialises to `0xFFFF`.
    pub tail: u16,
}

/// The laser fields of an analog-mode [`Record::Move`], as the CO2 producer
/// packs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnalogFields {
    /// Analog channel, 0 or 1, packed into the top two bits of the level word.
    pub channel: u8,
    /// The 14-bit analog level.
    pub level: u16,
    /// The laser output port, positive to enable and negative to disable.
    pub output: i8,
    /// The trailing field.
    pub tail: u16,
}

/// Why words could not be read as records, or a record is not valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// A header declares more payload than the words that follow.
    #[error("record at word {at} declares more payload than remains")]
    Truncated {
        /// Word index of the header.
        at: usize,
    },
    /// A header declares a payload that is not whole words.
    #[error("record at word {at} has header {header:#x} with a payload that is not whole words")]
    Unaligned {
        /// Word index of the header.
        at: usize,
        /// The header word.
        header: u32,
    },
    /// No known record has this shape.
    #[error("record at word {at} with header {header:#x} is not a known shape")]
    Unknown {
        /// Word index of the header.
        at: usize,
        /// The header word.
        header: u32,
    },
    /// A field is outside the range the controller accepts.
    #[error("{0}")]
    Field(&'static str),
    /// A block would exceed what one upload frame can carry.
    #[error("block of {words} words exceeds the {MAX_BLOCK_WORDS} an upload frame carries")]
    BlockTooLarge {
        /// The block's record words.
        words: usize,
    },
}

impl Record {
    /// A pulsed-mode step.
    #[must_use]
    pub const fn pulsed(dx: i8, dy: i8, fields: PulsedFields) -> Self {
        let frequency = fields.frequency.to_le_bytes();
        let tail = fields.tail.to_le_bytes();
        Self::Move { dx, dy, laser: [fields.power, frequency[0], frequency[1], tail[0], tail[1]] }
    }

    /// An analog-mode step.
    #[must_use]
    pub const fn analog(dx: i8, dy: i8, fields: AnalogFields) -> Self {
        let level = ((fields.channel as u16) << 14 | (fields.level & 0x3fff)).to_le_bytes();
        let tail = fields.tail.to_le_bytes();
        Self::Move {
            dx,
            dy,
            laser: [level[0], level[1], fields.output.cast_unsigned(), tail[0], tail[1]],
        }
    }

    /// One digital output switched on or off, port 1 to 26.
    pub fn output(port: u8, on: bool) -> Result<Self, RecordError> {
        let OutputPort { bank, mask } = OutputPort::new(port)
            .map_err(|_| RecordError::Field("output port is outside 1..=26"))?;
        Ok(Self::Outputs { bank, mask, values: if on { mask } else { 0 } })
    }

    /// The laser fields of a step read as pulsed mode.
    #[must_use]
    pub const fn pulsed_fields(laser: [u8; 5]) -> PulsedFields {
        PulsedFields {
            power: laser[0],
            frequency: u16::from_le_bytes([laser[1], laser[2]]),
            tail: u16::from_le_bytes([laser[3], laser[4]]),
        }
    }

    /// The laser fields of a step read as analog mode.
    #[must_use]
    pub const fn analog_fields(laser: [u8; 5]) -> AnalogFields {
        let level = u16::from_le_bytes([laser[0], laser[1]]);
        AnalogFields {
            channel: (level >> 14) as u8,
            level: level & 0x3fff,
            output: laser[2].cast_signed(),
            tail: u16::from_le_bytes([laser[3], laser[4]]),
        }
    }

    /// The record's opcode, the low half of its header word.
    #[must_use]
    pub const fn opcode(&self) -> u16 {
        match self {
            Self::Move { .. } => 0x0bb8,
            Self::Barrier => 0x0bb9,
            Self::Item(_) => 0x0bba,
            Self::ContourControl(_)
            | Self::Pwm { .. }
            | Self::Outputs { .. }
            | Self::Analog { .. } => 0x270f,
            Self::Delay { .. } | Self::WaitStatus { .. } => 0x07d1,
            Self::HeightAbsolute { .. } => 0x67,
            Self::Follow { .. } => 0x68,
            Self::Lift { .. } => 0x6d,
            Self::PierceHeight { .. } => 0x69,
            Self::PierceRamp { .. } => 0x6c,
            Self::FrogJump { .. } => 0x6a,
            Self::CrashProtect { .. } => 0x76,
        }
    }

    /// Checks every field against the ranges the controller accepts.
    pub fn validate(&self) -> Result<(), RecordError> {
        let ok = match *self {
            Self::Move { .. }
            | Self::Barrier
            | Self::Item(_)
            | Self::Outputs { .. }
            | Self::CrashProtect { .. } => true,
            Self::ContourControl(value) => matches!(value, 0 | 9 | 10 | 1000 | 1001),
            Self::Pwm { frequency, power, .. } => frequency != 0 && power <= 100,
            Self::Analog { channel, value } => matches!(channel, 1 | 2) && value <= 10_000,
            Self::Delay { millis } => millis <= 600_000,
            Self::WaitStatus { selector, timeout_ms } => {
                (1..=4).contains(&selector) && timeout_ms != 0
            }
            Self::HeightAbsolute { speed_tenths, height_microns } => {
                speed_tenths != 0 && speed_tenths <= I32_MAX && height_microns <= I32_MAX
            }
            Self::Follow { speed_tenths, .. } | Self::Lift { speed_tenths, .. } => {
                speed_tenths != 0
            }
            Self::PierceHeight { follow_speed_tenths, lift_speed_tenths, .. } => {
                follow_speed_tenths != 0 && lift_speed_tenths != 0
            }
            Self::PierceRamp { speed_tenths, .. } => speed_tenths != 0,
            Self::FrogJump { speed_tenths, lift, hold_ms, down_speed_tenths, .. } => {
                speed_tenths != 0 && down_speed_tenths != 0 && lift <= I32_MAX && hold_ms <= 600_000
            }
        };
        if ok {
            Ok(())
        } else {
            Err(RecordError::Field("record field is outside the controller's range"))
        }
    }

    /// The record's words: header then payload.
    #[must_use]
    pub fn words(&self) -> Vec<u32> {
        let mut words = Vec::new();
        self.append_words(&mut words);
        words
    }

    /// Append one complete record without allocating an intermediate payload.
    /// Call `validate` first when the record comes from outside the compiler.
    pub fn append_words(&self, words: &mut Vec<u32>) {
        let payload: &[u32] = match *self {
            Self::Move { dx, dy, laser } => &[
                u32::from_le_bytes([dx.cast_unsigned(), dy.cast_unsigned(), laser[0], laser[1]]),
                u32::from_le_bytes([laser[2], 0, laser[3], laser[4]]),
            ],
            Self::Barrier => &[],
            Self::Item(tag) => &[tag],
            Self::ContourControl(value) => &[18, u32::from(value)],
            Self::Pwm { channel, frequency, power } => {
                &[channel.selector(), u32::from(frequency), u32::from(power), u32::from(power != 0)]
            }
            Self::Outputs { bank, mask, values } => {
                &[bank_selector(bank), u32::from(mask), u32::from(values)]
            }
            Self::Analog { channel, value } => &[4, u32::from(channel.saturating_sub(1)), value],
            Self::Delay { millis } => &[millis, 3000],
            Self::WaitStatus { selector, timeout_ms } => {
                &[0x0300_0000 | u32::from(selector), timeout_ms]
            }
            Self::HeightAbsolute { speed_tenths, height_microns } => {
                &[speed_tenths, height_microns]
            }
            Self::Follow { speed_tenths, height } => &[speed_tenths, height],
            Self::Lift { speed_tenths, delta_microns } => {
                &[speed_tenths, delta_microns.cast_unsigned()]
            }
            Self::PierceHeight { follow_speed_tenths, height, lift_speed_tenths, lift_microns } => {
                &[follow_speed_tenths, height, lift_speed_tenths, lift_microns.cast_unsigned()]
            }
            Self::PierceRamp { speed_tenths, descent_microns, cut_height_microns } => {
                &[speed_tenths, descent_microns.cast_unsigned(), cut_height_microns, 1]
            }
            Self::FrogJump { speed_tenths, lift, hold_ms, down_speed_tenths, target } => {
                &[speed_tenths, lift, hold_ms, 0, u32::from(down_speed_tenths), target]
            }
            Self::CrashProtect { value, height } => &[4, value, height],
        };
        let header =
            (u32::try_from(payload.len() * 4).unwrap_or(u32::MAX) << 16) | u32::from(self.opcode());
        words.push(header);
        words.extend_from_slice(payload);
    }

    /// Reads one record from the start of `words`, returning it and the
    /// number of words it used.
    pub fn decode(words: &[u32], at: usize) -> Result<(Self, usize), RecordError> {
        decode::record(words, at)
    }
}

const I32_MAX: u32 = i32::MAX.cast_unsigned();

const fn bank_selector(bank: OutputBank) -> u32 {
    match bank {
        OutputBank::Standard => 2,
        OutputBank::Extended => 13,
    }
}

/// Validates and encodes records into one word stream.
pub fn encode(records: &[Record]) -> Result<Vec<u32>, RecordError> {
    let mut words = Vec::new();
    for record in records {
        record.validate()?;
        record.append_words(&mut words);
    }
    Ok(words)
}

/// Decodes a whole word stream into records.
pub fn decode(words: &[u32]) -> Result<Vec<Record>, RecordError> {
    let mut records = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let (record, used) = Record::decode(words, at)?;
        records.push(record);
        at += used;
    }
    Ok(records)
}

// Blocks and upload --------------------------------------------------------

/// The converter's block flush point: once a block holds this many words,
/// counting its four-word seed, it is flushed after the record that crossed
/// the line.
pub const FLUSH_WORDS: usize = 300;
/// The most record words one upload frame can carry, from the vendor's
/// audited buffer of 1448 bytes.
pub const MAX_BLOCK_WORDS: usize = 357;
/// Bytes the vendor keeps free in the FIFO before and after every upload.
pub const FIFO_RESERVE_BYTES: u32 = 2000;
/// The vendor uploads at most this many frames per drain.
pub const MAX_FRAMES_PER_DRAIN: usize = 20;
/// Total FIFO capacity below which the vendor refuses to start a job.
pub const MIN_FIFO_CAPACITY: u32 = 2000;

const SEED_WORDS: usize = 4;

/// Packs fragments into blocks the way the converter does: fragments are
/// never split, a block is closed after the fragment that brings it to
/// [`FLUSH_WORDS`], and the last block closes with the last fragment.
///
/// Each returned block is its record words only; [`crate::requests::program`]
/// adds the stamp when the block is uploaded.
pub fn partition<'a>(
    fragments: impl IntoIterator<Item = &'a [u32]>,
) -> Result<Vec<Vec<u32>>, RecordError> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for fragment in fragments {
        let words = current.len().saturating_add(fragment.len());
        if words > MAX_BLOCK_WORDS {
            return Err(RecordError::BlockTooLarge { words });
        }
        current.extend_from_slice(fragment);
        if SEED_WORDS + current.len() >= FLUSH_WORDS {
            blocks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    Ok(blocks)
}

/// Validates and packs records directly into upload blocks, preserving
/// complete records and the converter's flush-after-crossing rule.
pub fn encode_blocks(records: &[Record]) -> Result<Vec<Vec<u32>>, RecordError> {
    encoded_blocks(records).collect()
}

/// Encodes one upload block at a time, so previews can count the blocks
/// without retaining a second copy of the entire program.
pub fn encoded_blocks(
    records: &[Record],
) -> impl Iterator<Item = Result<Vec<u32>, RecordError>> + '_ {
    let mut records = records.iter().peekable();
    std::iter::from_fn(move || {
        records.peek()?;
        let mut current = Vec::with_capacity(FLUSH_WORDS);
        for record in records.by_ref() {
            if let Err(error) = record.validate() {
                return Some(Err(error));
            }
            record.append_words(&mut current);
            if SEED_WORDS + current.len() >= FLUSH_WORDS {
                break;
            }
        }
        Some(Ok(current))
    })
}

/// The bytes the controller charges for one block of record words: the
/// stamp and the three seed words come with it.
#[must_use]
pub fn block_bytes(record_words: usize) -> u32 {
    u32::try_from(4 * (record_words + SEED_WORDS)).unwrap_or(u32::MAX)
}

/// Whether a block may be uploaded into `free_bytes` of FIFO while keeping
/// the vendor's reserve.
#[must_use]
pub fn admits(free_bytes: u32, record_words: usize) -> bool {
    block_bytes(record_words)
        .checked_add(FIFO_RESERVE_BYTES)
        .is_some_and(|needed| free_bytes >= needed)
}

/// The FIFO configuration word for a job: the extra axes' bits plus the
/// mode bits the vendor selects from the laser mode and CO2 PWM type.
#[must_use]
pub fn fifo_mask(mode: LaserMode, co2_pwm_type: u8, extra_axes: &[u8]) -> u32 {
    let axes = extra_axes.iter().fold(0, |mask, axis| mask | (1u32 << (axis & 31)));
    match (mode, co2_pwm_type) {
        (LaserMode::Fiber, _) | (LaserMode::Co2, 1 | 2) => axes | crate::requests::FIFO_MODE_A,
        (LaserMode::Co2, 3) => axes | crate::requests::FIFO_MODE_B,
        _ => axes,
    }
}

/// The vendor's completion test: the FIFO reports its full capacity free and
/// its activity byte reads 1. A host decision, not physical proof.
#[must_use]
pub fn finished(status: &Status, capacity: u32) -> bool {
    status.fifo_free() == capacity && status.fifo_activity() == 1
}

/// Where execution stands, from the status block: the running item tag and
/// the progress within it. These identify records, never accepted frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Checkpoint {
    /// The [`Record::Item`] tag being executed.
    pub item: i32,
    /// Progress within the item, in the controller's own units.
    pub progress: i32,
}

impl Checkpoint {
    /// Reads the checkpoint words from a status block.
    #[must_use]
    pub const fn from_status(status: &Status) -> Self {
        Self { item: status.word(23).cast_signed(), progress: status.word(24).cast_signed() }
    }
}

/// The vendor host's UDP timing for program traffic.
pub mod timing {
    /// Total time the vendor keeps trying one FIFO exchange.
    pub const FIFO_TIME_MS: u32 = 1600;
    /// Socket timeout per attempt.
    pub const TIMEOUT_MS: u32 = 500;
    /// Receive attempts per send.
    pub const RECEIVE_ATTEMPTS: u32 = 3;
    /// Pause between sends.
    pub const SEND_INTERVAL_MS: u32 = 1;
    /// Sends before giving up: the FIFO time divided by the timeout.
    pub const SEND_ATTEMPTS: u32 = FIFO_TIME_MS / TIMEOUT_MS;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process records reproduce the vendor's word vectors: output bank
    /// boundary at port 11, PWM enable following power, delay tail 3000, and
    /// the status wait's mode byte.
    #[test]
    fn process_records_match_the_vendor_words() {
        assert_eq!(Record::output(10, true).unwrap().words(), [0xc270f, 2, 512, 512]);
        assert_eq!(Record::output(11, false).unwrap().words(), [0xc270f, 13, 1, 0]);
        assert_eq!(Record::output(26, true).unwrap().words(), [0xc270f, 13, 32_768, 32_768]);
        assert!(Record::output(27, true).is_err());
        let pwm = Record::Pwm { channel: LaserChannel::Secondary, frequency: 5000, power: 0 };
        assert_eq!(pwm.words(), [0x0010_270f, 17, 5000, 0, 0]);
        let pwm = Record::Pwm { channel: LaserChannel::Primary, frequency: 20_000, power: 60 };
        assert_eq!(pwm.words(), [0x0010_270f, 3, 20_000, 60, 1]);
        assert_eq!(Record::Delay { millis: 1500 }.words(), [0x807d1, 1500, 3000]);
        assert_eq!(
            Record::WaitStatus { selector: 4, timeout_ms: 2000 }.words(),
            [0x807d1, 0x0300_0004, 2000]
        );
        assert_eq!(Record::ContourControl(1000).words(), [0x8270f, 18, 1000]);
        assert_eq!(Record::Analog { channel: 2, value: 2500 }.words(), [0xc270f, 4, 1, 2500]);
        assert_eq!(Record::Barrier.words(), [0xbb9]);
        assert_eq!(Record::Item(7).words(), [0x40bba, 7]);
    }

    /// Head records carry their fixed tails and signed fields as the
    /// converter emits them.
    #[test]
    fn head_records_match_the_vendor_words() {
        assert_eq!(
            Record::HeightAbsolute { speed_tenths: 300, height_microns: 5000 }.words(),
            [0x80067, 300, 5000]
        );
        assert_eq!(
            Record::Follow { speed_tenths: 300, height: 1000 }.words(),
            [0x80068, 300, 1000]
        );
        assert_eq!(
            Record::Lift { speed_tenths: 300, delta_microns: -2000 }.words(),
            [0x8006d, 300, (-2000i32).cast_unsigned()]
        );
        assert_eq!(
            Record::PierceHeight {
                follow_speed_tenths: 1,
                height: 2,
                lift_speed_tenths: 3,
                lift_microns: -4
            }
            .words(),
            [0x0010_0069, 1, 2, 3, (-4i32).cast_unsigned()]
        );
        assert_eq!(
            Record::PierceRamp { speed_tenths: 5, descent_microns: -6, cut_height_microns: 7 }
                .words(),
            [0x0010_006c, 5, (-6i32).cast_unsigned(), 7, 1]
        );
        assert_eq!(
            Record::FrogJump {
                speed_tenths: 1,
                lift: 2,
                hold_ms: 3,
                down_speed_tenths: 4,
                target: 5
            }
            .words(),
            [0x0018_006a, 1, 2, 3, 0, 4, 5]
        );
        assert_eq!(Record::CrashProtect { value: 8, height: 9 }.words(), [0xc0076, 4, 8, 9]);
    }

    /// A pulsed step packs power, frequency and tail around the two signed
    /// pulse bytes; an analog step packs channel, level and the signed output
    /// port; both read back through their field views.
    #[test]
    fn move_records_pack_their_laser_bytes() {
        let pulsed =
            Record::pulsed(-1, 127, PulsedFields { power: 37, frequency: 12_345, tail: 0 });
        let words = pulsed.words();
        assert_eq!(words[0], 0x80bb8);
        let a = words[1].to_le_bytes();
        let b = words[2].to_le_bytes();
        assert_eq!((a[0].cast_signed(), a[1].cast_signed(), a[2]), (-1, 127, 37));
        assert_eq!(u16::from_le_bytes([a[3], b[0]]), 12_345);
        assert_eq!(&b[1..], &[0, 0, 0]);
        let Record::Move { laser, .. } = pulsed else { panic!() };
        assert_eq!(
            Record::pulsed_fields(laser),
            PulsedFields { power: 37, frequency: 12_345, tail: 0 }
        );

        let analog = Record::analog(
            1,
            -128,
            AnalogFields { channel: 1, level: 4000, output: -9, tail: 0xffff },
        );
        let Record::Move { dx, dy, laser } = analog.clone() else { panic!() };
        assert_eq!((dx, dy), (1, -128));
        assert_eq!(
            Record::analog_fields(laser),
            AnalogFields { channel: 1, level: 4000, output: -9, tail: 0xffff }
        );
        assert_eq!(u16::from_le_bytes([laser[0], laser[1]]), (1 << 14) | 0x0fa0);
        assert_eq!(laser[2], (-9i8).cast_unsigned());
    }

    /// The prototype simulator's fixture decodes into the same four records
    /// with the same signed pulses and delay, and malformed streams are
    /// refused: a bad reserved byte, a stray word, an unaligned header.
    #[test]
    fn decoding_matches_the_simulator_fixture_and_rejects_junk() {
        let words = [
            0x80bb8,
            0x3420_7fff,
            0x1234_0012,
            0xbb9,
            0x80bb8,
            0x4000_8001,
            0x0064_00ab,
            0x807d1,
            125,
            3000,
        ];
        let records = decode(&words).unwrap();
        assert_eq!(records.len(), 4);
        assert!(matches!(records[0], Record::Move { dx: -1, dy: 127, .. }));
        assert_eq!(records[1], Record::Barrier);
        assert!(matches!(records[2], Record::Move { dx: 1, dy: -128, .. }));
        assert_eq!(records[3], Record::Delay { millis: 125 });
        assert_eq!(encode(&records).unwrap(), words);
        assert!(matches!(decode(&[0x80bb8, 0]), Err(RecordError::Truncated { at: 0 })));
        assert!(matches!(decode(&[0x1234]), Err(RecordError::Unknown { at: 0, .. })));
        assert!(matches!(decode(&[0x0001_0bb8, 0]), Err(RecordError::Unaligned { at: 0, .. })));
        assert!(matches!(decode(&[0x80bb8, 0, 0x100]), Err(RecordError::Unknown { at: 0, .. })));
        assert!(matches!(
            decode(&[0x0010_270f, 3, 5000, 101, 1]),
            Err(RecordError::Unknown { .. })
        ));
        assert!(matches!(decode(&[0x0008_07d1, 0x0300_0101, 1]), Err(RecordError::Unknown { .. })));
    }

    /// Every record kind survives an encode and decode round trip.
    #[test]
    fn every_record_round_trips() {
        let records = vec![
            Record::pulsed(3, -4, PulsedFields { power: 50, frequency: 5000, tail: 0xffff }),
            Record::Barrier,
            Record::Item(u32::MAX),
            Record::ContourControl(9),
            Record::Pwm { channel: LaserChannel::Primary, frequency: 1, power: 100 },
            Record::Outputs { bank: OutputBank::Extended, mask: 0x8001, values: 1 },
            Record::Analog { channel: 1, value: 10_000 },
            Record::Delay { millis: 0 },
            Record::WaitStatus { selector: 1, timeout_ms: 1 },
            Record::HeightAbsolute { speed_tenths: 1, height_microns: 2 },
            Record::Follow { speed_tenths: 1, height: 0 },
            Record::Lift { speed_tenths: 1, delta_microns: i32::MIN },
            Record::PierceHeight {
                follow_speed_tenths: 1,
                height: 2,
                lift_speed_tenths: 3,
                lift_microns: -4,
            },
            Record::PierceRamp { speed_tenths: 5, descent_microns: -6, cut_height_microns: 7 },
            Record::FrogJump {
                speed_tenths: 1,
                lift: 2,
                hold_ms: 600_000,
                down_speed_tenths: 4,
                target: 5,
            },
            Record::CrashProtect { value: 8, height: 9 },
        ];
        let words = encode(&records).unwrap();
        assert_eq!(decode(&words).unwrap(), records);
    }

    /// Out-of-range fields fail validation and are not encoded.
    #[test]
    fn invalid_fields_are_refused() {
        for record in [
            Record::ContourControl(2),
            Record::Pwm { channel: LaserChannel::Primary, frequency: 0, power: 1 },
            Record::Pwm { channel: LaserChannel::Primary, frequency: 1, power: 101 },
            Record::Analog { channel: 3, value: 0 },
            Record::Analog { channel: 1, value: 10_001 },
            Record::Delay { millis: 600_001 },
            Record::WaitStatus { selector: 5, timeout_ms: 1 },
            Record::WaitStatus { selector: 1, timeout_ms: 0 },
            Record::Follow { speed_tenths: 0, height: 1 },
            Record::FrogJump {
                speed_tenths: 1,
                lift: 2,
                hold_ms: 600_001,
                down_speed_tenths: 4,
                target: 5,
            },
        ] {
            assert!(record.validate().is_err(), "{record:?}");
            assert!(encode(&[record]).is_err());
        }
    }

    /// Fragments are never split, a block closes after the fragment that
    /// reaches 300 words including the seed, the last block closes with the
    /// input, and an oversized block is refused.
    #[test]
    fn partition_follows_the_converter_flush_rule() {
        let fragment = [0xbb9, 0x8270f, 18, 0];
        let fragments: Vec<&[u32]> = (0..100).map(|_| &fragment[..]).collect();
        let blocks = partition(fragments.iter().copied()).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].len(), 296);
        assert_eq!(blocks[1].len(), 104);
        assert_eq!(partition(std::iter::empty()).unwrap().len(), 0);
        let small: [u32; 3] = [0x807d1, 1, 3000];
        assert_eq!(partition([&small[..]]).unwrap(), vec![small.to_vec()]);
        let huge = vec![0xbb9; 358];
        assert!(matches!(partition([&huge[..]]), Err(RecordError::BlockTooLarge { words: 358 })));
    }

    /// The direct typed encoder preserves every record boundary at and
    /// across the converter's flush threshold, including byte accounting.
    #[test]
    fn direct_block_encoding_matches_fragment_partition() {
        for count in [0, 1, 98, 99, 100, 199, 300] {
            let mut records =
                vec![
                    Record::pulsed(1, -1, PulsedFields { power: 0, frequency: 5000, tail: 0 });
                    count
                ];
            records.push(Record::FrogJump {
                speed_tenths: 100,
                lift: 200,
                hold_ms: 300,
                down_speed_tenths: 100,
                target: 500,
            });
            records.push(Record::Barrier);
            let fragments: Vec<_> = records.iter().map(Record::words).collect();
            let blocks = encode_blocks(&records).unwrap();
            assert_eq!(blocks, partition(fragments.iter().map(Vec::as_slice)).unwrap());
            assert_eq!(decode(&blocks.concat()).unwrap(), records);
            for block in blocks {
                let bytes = block_bytes(block.len());
                assert!(admits(bytes + FIFO_RESERVE_BYTES, block.len()));
                assert!(!admits(bytes + FIFO_RESERVE_BYTES - 1, block.len()));
            }
        }
    }

    #[test]
    fn block_iteration_checks_records_as_they_are_consumed() {
        let mut records = vec![Record::Barrier; FLUSH_WORDS - SEED_WORDS];
        records.push(Record::Analog { channel: 0, value: 0 });
        let mut blocks = encoded_blocks(&records);
        assert_eq!(blocks.next().unwrap().unwrap().len(), FLUSH_WORDS - SEED_WORDS);
        assert!(blocks.next().unwrap().is_err());
        assert!(blocks.next().is_none());
        assert!(encoded_blocks(&[]).next().is_none());
        assert!(encode_blocks(&records).is_err());
    }

    /// Upload accounting: a block costs its words plus the stamp and seed,
    /// the reserve stays free, and the mode word follows laser mode and PWM type.
    #[test]
    fn upload_arithmetic_matches_the_vendor() {
        assert_eq!(block_bytes(1), 20);
        assert!(admits(2020, 1));
        assert!(!admits(2019, 1));
        assert_eq!(fifo_mask(LaserMode::Fiber, 0, &[]), 0x6680_0000);
        assert_eq!(fifo_mask(LaserMode::Co2, 2, &[]), 0x6680_0000);
        assert_eq!(fifo_mask(LaserMode::Co2, 3, &[4]), 0x7880_0010);
        assert_eq!(fifo_mask(LaserMode::Co2, 0, &[2, 3]), 0b1100);
        assert_eq!(timing::SEND_ATTEMPTS, 3);
    }

    /// Completion needs the whole capacity free and activity byte 1; the
    /// checkpoint reads words 23 and 24 as signed values.
    #[test]
    fn completion_and_checkpoint_read_the_status_words() {
        let mut words = [0; 36];
        words[16] = 65_000;
        words[19] = 1;
        words[23] = (-1i32).cast_unsigned();
        words[24] = 42;
        let status = Status::decode(&words).unwrap();
        assert!(finished(&status, 65_000));
        assert!(!finished(&status, 65_001));
        assert_eq!(Checkpoint::from_status(&status), Checkpoint { item: -1, progress: 42 });
        words[19] = 2;
        assert!(!finished(&Status::decode(&words).unwrap(), 65_000));
    }
}
