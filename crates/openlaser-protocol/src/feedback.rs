// SPDX-License-Identifier: GPL-3.0-or-later

//! What the controller's status blocks mean, word by word.
//!
//! Each type wraps one block exactly as read and exposes the fields the vendor
//! software is known to use, through the same getters it uses. Words that the
//! vendor never reads stay reachable through `word`, unnamed.
//!
//! Evidence: NCModule getters `0x1002_b6c0` through `0x1002_bd20` (status),
//! `0x1003_9540` (the axis classifier), `0x1003_9450` (head class),
//! `0x1003_9f60` (interpolation cycle) and `0x1002_c580` (positions).

use crate::registers::{AXIS_COUNT, PARAMETER_BANK_WORDS};

/// A block arrived with the wrong number of words, or a word is out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FeedbackError {
    /// The block does not have the expected number of words.
    #[error("{block} block has {actual} words, expected {expected}")]
    Length {
        /// Which block.
        block: &'static str,
        /// The word count the block must have.
        expected: usize,
        /// The word count received.
        actual: usize,
    },
    /// The interpolation cycle is outside 1 to 1 000 000 microseconds.
    #[error("interpolation cycle {0} us is outside 1..=1000000")]
    Cycle(u32),
    /// The coordinate divisor is zero or negative.
    #[error("coordinate divisor {0} is not positive")]
    Divisor(i32),
}

fn exact<const N: usize>(block: &'static str, words: &[u32]) -> Result<[u32; N], FeedbackError> {
    <[u32; N]>::try_from(words).map_err(|_| FeedbackError::Length {
        block,
        expected: N,
        actual: words.len(),
    })
}

/// Machine status, register 1000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    words: [u32; 36],
}

impl Status {
    /// Decodes the 36-word status block.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        exact("status", words).map(|words| Self { words })
    }

    /// The controller's product id. The MCC100 reports 103.
    #[must_use]
    pub const fn product_id(&self) -> u32 {
        self.words[0]
    }

    /// The firmware program version. Firmware 201.77 reports 20177.
    #[must_use]
    pub const fn program_version(&self) -> u32 {
        self.words[1]
    }

    /// The 24 digital inputs, bit 0 being input 1.
    #[must_use]
    pub const fn inputs(&self) -> u32 {
        self.words[4] & 0x00ff_ffff
    }

    /// Whether digital input `number` (1 to 24) is high.
    #[must_use]
    pub const fn input(&self, number: u8) -> bool {
        number >= 1 && number <= 24 && self.inputs() & (1 << (number - 1)) != 0
    }

    /// The 16 standard digital outputs, bit 0 being port 1.
    #[must_use]
    pub fn outputs(&self) -> u16 {
        low_half(self.words[5])
    }

    /// Whether output `port` (1 to 26) is on in its assigned bank.
    #[must_use]
    pub fn output(&self, port: u8) -> bool {
        use crate::requests::{OutputBank, OutputPort};
        OutputPort::new(port).is_ok_and(|port| match port.bank {
            OutputBank::Standard => self.outputs() & port.mask != 0,
            OutputBank::Extended => self.extended_outputs() & u32::from(port.mask) != 0,
        })
    }

    /// The extended output feedback word.
    #[must_use]
    pub const fn extended_outputs(&self) -> u32 {
        self.words[22]
    }

    /// Controller alarm aggregate, group 1. Bits 0 to 24 summarise the axis
    /// detail records; bits 25 to 31 are direct faults.
    #[must_use]
    pub const fn alarm_group_1(&self) -> u32 {
        self.words[6]
    }

    /// Controller alarm aggregate, group 2: command and execution faults.
    #[must_use]
    pub const fn alarm_group_2(&self) -> u32 {
        self.words[7]
    }

    /// The transfer stamp the FIFO echoes for the last accepted upload.
    #[must_use]
    pub const fn transfer_stamp(&self) -> u32 {
        self.words[15]
    }

    /// Free space in the FIFO, in bytes.
    #[must_use]
    pub const fn fifo_free(&self) -> u32 {
        self.words[16]
    }

    /// The FIFO activity byte: 1 while a program runs, 2 at the finish edge.
    #[must_use]
    pub const fn fifo_activity(&self) -> u8 {
        self.words[19].to_le_bytes()[0]
    }

    /// Any raw word of the block.
    #[must_use]
    pub const fn word(&self, index: usize) -> u32 {
        self.words[index]
    }
}

/// One axis record: ten words, of which the vendor reads five.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisFeedback {
    /// The raw status word: state byte, phase byte, and flags.
    pub status: u32,
    /// Speed in the controller's raw units, signed.
    pub speed: i32,
    /// Position in pulses.
    pub position: i32,
    /// Encoder position in pulses.
    pub encoder: i32,
    /// The pulse position captured at the last stop.
    pub stop_pulse: i32,
}

impl AxisFeedback {
    fn from_record(record: &[u32]) -> Self {
        Self {
            status: record[0],
            speed: record[1].cast_signed(),
            position: record[2].cast_signed(),
            encoder: record[3].cast_signed(),
            stop_pulse: record[5].cast_signed(),
        }
    }

    /// Bits 24 to 31 of the status word: the axis state.
    #[must_use]
    pub const fn state(self) -> u8 {
        self.status.to_le_bytes()[3]
    }

    /// Bits 16 to 23: the motion phase. Nonzero is normal while moving; it is
    /// not a fault byte.
    #[must_use]
    pub const fn phase(self) -> u8 {
        self.status.to_le_bytes()[2]
    }

    /// Bits 0 to 15 of the status word.
    #[must_use]
    pub fn flags(self) -> u16 {
        low_half(self.status)
    }

    /// Whether the axis reports a valid reference: flag bit 15.
    #[must_use]
    pub const fn referenced(self) -> bool {
        self.status & 0x8000 != 0
    }

    /// The position in millimetres for a coordinate `divisor`.
    #[must_use]
    pub fn position_mm(self, divisor: i32) -> f64 {
        f64::from(self.position) / f64::from(divisor)
    }
}

/// The native axis classifier's result, in the order it tests its predicates.
/// These are entry gates for operations, not idle or completion proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AxisClass {
    /// No predicate matched.
    None = 0,
    /// An axis in state 2 with a nonzero phase.
    Active = 1,
    /// An axis in state 3, 4 or 5 with a nonzero phase.
    Settling = 2,
    /// The FIFO activity byte is 1: a program is running.
    Fifo = 4,
}

/// The five axis records, register 2000, or the first fifty words of the
/// runtime block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axes {
    records: [AxisFeedback; AXIS_COUNT],
}

impl Axes {
    /// Decodes fifty words of axis records.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        let words: [u32; 50] = exact("axes", words)?;
        Ok(Self::from_words(&words))
    }

    fn from_words(words: &[u32]) -> Self {
        let mut records =
            [AxisFeedback { status: 0, speed: 0, position: 0, encoder: 0, stop_pulse: 0 };
                AXIS_COUNT];
        for (record, chunk) in records.iter_mut().zip(words.chunks_exact(10)) {
            *record = AxisFeedback::from_record(chunk);
        }
        Self { records }
    }

    /// The record of axis `index` (0 to 4). The vendor displays indices
    /// 0, 1, 3 and 4 as X, Y, Z and W.
    #[must_use]
    pub const fn axis(&self, index: usize) -> AxisFeedback {
        self.records[index]
    }

    /// All five records.
    #[must_use]
    pub const fn all(&self) -> &[AxisFeedback; AXIS_COUNT] {
        &self.records
    }

    /// Whether both X and Y report a reference.
    #[must_use]
    pub const fn xy_referenced(&self) -> bool {
        self.records[0].referenced() && self.records[1].referenced()
    }

    /// Whether every axis reports zero speed and a zero phase.
    #[must_use]
    pub fn stationary(&self) -> bool {
        self.records.iter().all(|axis| axis.speed == 0 && axis.phase() == 0)
    }

    /// The native classifier: the FIFO byte first, then each record in order.
    #[must_use]
    pub fn class(&self, status: &Status) -> AxisClass {
        if status.fifo_activity() == 1 {
            return AxisClass::Fifo;
        }
        for axis in &self.records {
            if axis.phase() != 0 {
                match axis.state() {
                    2 => return AxisClass::Active,
                    3..=5 => return AxisClass::Settling,
                    _ => {}
                }
            }
        }
        AxisClass::None
    }
}

/// The head controller block, register 10000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    words: [u32; 18],
}

impl Head {
    /// Decodes the 18-word head block.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        exact("head", words).map(|words| Self { words })
    }

    /// The raw head alarm word. Only its low 16 bits carry alarms; see
    /// `alarms::head_alarms`.
    #[must_use]
    pub const fn alarm_word(&self) -> u32 {
        self.words[1]
    }

    /// Whether the head reports a valid reference: bit 31 of word 2.
    #[must_use]
    pub const fn referenced(&self) -> bool {
        self.words[2] & 0x8000_0000 != 0
    }

    /// The low byte of word 2, the head status the vendor's getters classify.
    #[must_use]
    pub const fn status_byte(&self) -> u8 {
        self.words[2].to_le_bytes()[0]
    }

    /// The command the head is executing, zero when idle. Known values are
    /// 102 home, 107 calibrate and 117 follow.
    #[must_use]
    pub const fn command(&self) -> u32 {
        self.words[3]
    }

    /// Head height in thousandths of a millimetre.
    #[must_use]
    pub const fn height(&self) -> i32 {
        self.words[6].cast_signed()
    }

    /// The vendor's Go Origin condition: no command active and status byte 1.
    /// Its host completion gate, not proof of position.
    #[must_use]
    pub const fn home_done(&self) -> bool {
        self.words[3] == 0 && self.status_byte() == 1
    }

    /// Stationary at the upper reference: home done, or referenced with a
    /// height at or above zero in the head's negative-up convention.
    #[must_use]
    pub const fn at_upper_reference(&self) -> bool {
        self.words[3] == 0 && (self.home_done() || (self.referenced() && self.height() <= 0))
    }

    /// The native head class from the vendor's Z getter, given whether the
    /// head is enabled. Operation-neutral: it is not an idle or home gate.
    #[must_use]
    pub const fn class(&self, enabled: bool) -> u32 {
        if !enabled {
            return 0;
        }
        if !self.referenced() {
            return 5;
        }
        if self.status_byte() == 4 {
            return 2;
        }
        match self.words[3] {
            0x66 => 6,
            0x67 => 4,
            0x68 => 1,
            0x6a => 8,
            0x6b => 7,
            _ => 0,
        }
    }

    /// Any raw word of the block.
    #[must_use]
    pub const fn word(&self, index: usize) -> u32 {
        self.words[index]
    }
}

/// System configuration, register 50000.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct System {
    words: [u32; 26],
}

impl System {
    /// Decodes the 26-word system block.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        exact("system", words).map(|words| Self { words })
    }

    /// The interpolation cycle in microseconds, which a compiled program's
    /// cadence must match.
    pub fn interpolation_cycle_us(&self) -> Result<u32, FeedbackError> {
        match self.words[5] {
            cycle @ 1..=1_000_000 => Ok(cycle),
            cycle => Err(FeedbackError::Cycle(cycle)),
        }
    }

    /// The paired-axis routing word. The ordinary dual-drive reset binding
    /// requires zero.
    #[must_use]
    pub const fn routing(&self) -> u32 {
        self.words[14]
    }

    /// The signed coordinate divisor that turns axis positions into
    /// millimetres. Exactly 1 raises the vendor's pulse-equivalent alarm.
    pub fn coordinate_divisor(&self) -> Result<i32, FeedbackError> {
        match self.words[17].cast_signed() {
            divisor if divisor > 0 => Ok(divisor),
            divisor => Err(FeedbackError::Divisor(divisor)),
        }
    }

    /// Any raw word of the block.
    #[must_use]
    pub const fn word(&self, index: usize) -> u32 {
        self.words[index]
    }
}

/// The configuration of one axis: the 14 meaningful words of its parameter bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisConfig {
    /// Word 0: the configuration flags.
    pub configuration: u32,
    /// Word 1: the negative soft limit in pulses.
    pub negative_limit: i32,
    /// Word 2: the positive soft limit in pulses.
    pub positive_limit: i32,
    /// Word 3.
    pub acceleration: u32,
    /// Word 4.
    pub jerk: u32,
    /// Word 5.
    pub coarse_home_speed: u32,
    /// Word 6.
    pub fine_home_speed: u32,
    /// Word 7.
    pub origin_offset: i32,
    /// Word 9: the lead, in the vendor's units.
    pub lead: u32,
    /// Word 10: command pulses per revolution.
    pub command_pulses: u32,
    /// Word 11: encoder pulses per revolution.
    pub encoder_pulses: u32,
    /// Word 12.
    pub input_configuration: u32,
    /// Word 13.
    pub output_configuration: u32,
}

impl AxisConfig {
    /// Decodes the 14 words of one bank.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        let w: [u32; PARAMETER_BANK_WORDS] = exact("axis config", words)?;
        Ok(Self {
            configuration: w[0],
            negative_limit: w[1].cast_signed(),
            positive_limit: w[2].cast_signed(),
            acceleration: w[3],
            jerk: w[4],
            coarse_home_speed: w[5],
            fine_home_speed: w[6],
            origin_offset: w[7].cast_signed(),
            lead: w[9],
            command_pulses: w[10],
            encoder_pulses: w[11],
            input_configuration: w[12],
            output_configuration: w[13],
        })
    }
}

/// The five parameter banks, register 50200: 20-word records of which the
/// first 14 words are meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameters {
    banks: [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT],
}

impl Parameters {
    /// Decodes the 100-word parameter block.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        let words: [u32; 100] = exact("parameters", words)?;
        let mut banks = [[0; PARAMETER_BANK_WORDS]; AXIS_COUNT];
        for (bank, record) in banks.iter_mut().zip(words.chunks_exact(20)) {
            bank.copy_from_slice(&record[..PARAMETER_BANK_WORDS]);
        }
        Ok(Self { banks })
    }

    /// The raw words of the bank of axis `index`, as they would be written back.
    #[must_use]
    pub const fn bank(&self, index: usize) -> &[u32; PARAMETER_BANK_WORDS] {
        &self.banks[index]
    }

    /// The decoded configuration of axis `index`.
    pub fn config(&self, index: usize) -> Result<AxisConfig, FeedbackError> {
        AxisConfig::decode(&self.banks[index])
    }
}

/// The fast poll block, register 60001: axis records then axis configs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Runtime {
    axes: Axes,
    configs: [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT],
}

impl Runtime {
    /// Decodes the 120-word runtime block.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        let words: [u32; 120] = exact("runtime", words)?;
        let mut configs = [[0; PARAMETER_BANK_WORDS]; AXIS_COUNT];
        for (config, chunk) in
            configs.iter_mut().zip(words[50..].chunks_exact(PARAMETER_BANK_WORDS))
        {
            config.copy_from_slice(chunk);
        }
        Ok(Self { axes: Axes::from_words(&words[..50]), configs })
    }

    /// The axis records.
    #[must_use]
    pub const fn axes(&self) -> &Axes {
        &self.axes
    }

    /// The complete parameter bank of axis `index`, including words that
    /// have no decoded field. Native runtime records carry all fourteen.
    #[must_use]
    pub const fn bank(&self, index: usize) -> &[u32; PARAMETER_BANK_WORDS] {
        &self.configs[index]
    }

    /// The decoded configuration of axis `index`.
    pub fn config(&self, index: usize) -> Result<AxisConfig, FeedbackError> {
        AxisConfig::decode(&self.configs[index])
    }
}

/// The identity pair at the start of the status block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Product id; the MCC100 reports 103.
    pub product_id: u32,
    /// Program version; firmware 201.77 reports 20177.
    pub program_version: u32,
}

impl Identity {
    /// Decodes the two-word identity read.
    pub fn decode(words: &[u32]) -> Result<Self, FeedbackError> {
        let words: [u32; 2] = exact("identity", words)?;
        Ok(Self { product_id: words[0], program_version: words[1] })
    }
}

fn low_half(word: u32) -> u16 {
    let bytes = word.to_le_bytes();
    u16::from_le_bytes([bytes[0], bytes[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_with(edits: &[(usize, u32)]) -> Status {
        let mut words = [0; 36];
        for &(index, value) in edits {
            words[index] = value;
        }
        Status::decode(&words).unwrap()
    }

    /// Status getters read the words the vendor getters read, with the input
    /// word masked to 24 bits and the output word to 16.
    #[test]
    fn status_words_map_to_the_vendor_getters() {
        let status = status_with(&[
            (0, 103),
            (1, 20_177),
            (4, 0xabcd_1234),
            (5, 0x9876_5432),
            (6, 1 << 30),
            (7, 1 << 5),
            (15, 42),
            (16, 65_000),
            (19, 0x0000_1102),
            (22, 7),
        ]);
        assert_eq!((status.product_id(), status.program_version()), (103, 20_177));
        assert_eq!(status.inputs(), 0x00cd_1234);
        assert!(status.input(3) && !status.input(2) && !status.input(25));
        assert_eq!(status.outputs(), 0x5432);
        assert!(status.output(2) && !status.output(1));
        assert_eq!((status.alarm_group_1(), status.alarm_group_2()), (1 << 30, 1 << 5));
        assert_eq!((status.transfer_stamp(), status.fifo_free()), (42, 65_000));
        assert_eq!(status.fifo_activity(), 2);
        assert_eq!(status.extended_outputs(), 7);
        assert!(matches!(
            Status::decode(&[0; 35]),
            Err(FeedbackError::Length { block: "status", expected: 36, actual: 35 })
        ));
    }

    /// An axis record splits its status word into state, phase and flags and
    /// reads speed and positions as signed words.
    #[test]
    fn axis_records_decode_signed_fields_and_status_bytes() {
        let mut words = [0; 50];
        words[0] = 0x0201_c000;
        words[1] = (-500i32).cast_unsigned();
        words[2] = (-23i32).cast_unsigned();
        words[3] = 9;
        words[5] = 4;
        words[10] = 0x0200_c000;
        let axes = Axes::decode(&words).unwrap();
        let x = axes.axis(0);
        assert_eq!((x.state(), x.phase(), x.flags()), (2, 1, 0xc000));
        assert_eq!((x.speed, x.position, x.encoder, x.stop_pulse), (-500, -23, 9, 4));
        assert!(x.referenced() && axes.xy_referenced());
        assert!(!axes.stationary());
        assert!((axes.axis(0).position_mm(1000) + 0.023).abs() < 1e-12);
    }

    /// The classifier checks the FIFO byte first, then states 2 and 3..=5
    /// with a nonzero phase, in record order, and otherwise reports none.
    #[test]
    fn axis_class_follows_the_native_predicate_order() {
        let quiet = Axes::decode(&[0; 50]).unwrap();
        assert_eq!(quiet.class(&status_with(&[])), AxisClass::None);
        assert_eq!(quiet.class(&status_with(&[(19, 0x201)])), AxisClass::Fifo);
        let mut words = [0; 50];
        words[10] = 0x0401_0000;
        assert_eq!(Axes::decode(&words).unwrap().class(&status_with(&[])), AxisClass::Settling);
        words[0] = 0x0201_0000;
        assert_eq!(Axes::decode(&words).unwrap().class(&status_with(&[])), AxisClass::Active);
        words[0] = 0x0200_0000;
        words[10] = 0x0400_0000;
        assert_eq!(Axes::decode(&words).unwrap().class(&status_with(&[])), AxisClass::None);
    }

    /// The head block exposes the reference bit, the active command, the
    /// signed height, the Go Origin condition and the upper-reference test.
    #[test]
    fn head_words_decode_and_predicates_match_the_vendor() {
        let mut words = [0; 18];
        assert!(!Head::decode(&words).unwrap().at_upper_reference());
        words[2] = 1;
        assert!(Head::decode(&words).unwrap().home_done());
        assert!(Head::decode(&words).unwrap().at_upper_reference());
        words[2] = 0x8000_0010;
        let head = Head::decode(&words).unwrap();
        assert!(head.referenced() && !head.home_done() && head.at_upper_reference());
        words[6] = 20_000;
        assert!(!Head::decode(&words).unwrap().at_upper_reference());
        words[6] = (-1i32).cast_unsigned();
        assert!(Head::decode(&words).unwrap().at_upper_reference());
        assert_eq!(Head::decode(&words).unwrap().height(), -1);
        words[3] = 109;
        assert!(!Head::decode(&words).unwrap().at_upper_reference());
        assert_eq!(Head::decode(&words).unwrap().command(), 109);
    }

    /// The head class reproduces the vendor's Z getter: disabled is 0,
    /// unreferenced is 5, status byte 4 is 2, then the command table.
    #[test]
    fn head_class_matches_the_vendor_getter() {
        let mut words = [0; 18];
        words[3] = 0x66;
        assert_eq!(Head::decode(&words).unwrap().class(false), 0);
        assert_eq!(Head::decode(&words).unwrap().class(true), 5);
        words[2] = 0x8000_0004;
        assert_eq!(Head::decode(&words).unwrap().class(true), 2);
        words[2] = 0x8000_0000;
        for (command, class) in [(0x66, 6), (0x67, 4), (0x68, 1), (0x6a, 8), (0x6b, 7), (0x70, 0)] {
            words[3] = command;
            assert_eq!(Head::decode(&words).unwrap().class(true), class, "command {command:#x}");
        }
    }

    /// The system block validates the interpolation cycle range and requires
    /// a positive coordinate divisor.
    #[test]
    fn system_words_are_range_checked() {
        let mut words = [0; 26];
        words[5] = 250;
        words[14] = 0;
        words[17] = 1000;
        let system = System::decode(&words).unwrap();
        assert_eq!(system.interpolation_cycle_us(), Ok(250));
        assert_eq!(system.coordinate_divisor(), Ok(1000));
        assert_eq!(system.routing(), 0);
        words[5] = 0;
        words[17] = (-5i32).cast_unsigned();
        let bad = System::decode(&words).unwrap();
        assert_eq!(bad.interpolation_cycle_us(), Err(FeedbackError::Cycle(0)));
        assert_eq!(bad.coordinate_divisor(), Err(FeedbackError::Divisor(-5)));
    }

    /// The same 14-word config decodes identically from the parameter block
    /// and from the tail of the runtime block, and the padding words of the
    /// 20-word records are ignored.
    #[test]
    fn configs_decode_from_both_blocks() {
        let bank = [
            131_344,
            (-500i32).cast_unsigned(),
            1_371_000,
            20_000,
            200_000,
            80_000,
            20_000,
            28_000,
            0,
            31_040,
            8_000,
            8_000,
            1_286,
            0,
        ];
        let mut parameters = [0xdead_beef; 100];
        parameters[20..34].copy_from_slice(&bank);
        let from_parameters = Parameters::decode(&parameters).unwrap();
        let mut runtime = [0; 120];
        runtime[64..78].copy_from_slice(&bank);
        let from_runtime = Runtime::decode(&runtime).unwrap();
        let config = from_parameters.config(1).unwrap();
        assert_eq!(config, from_runtime.config(1).unwrap());
        assert_eq!((config.negative_limit, config.positive_limit), (-500, 1_371_000));
        assert_eq!((config.lead, config.command_pulses), (31_040, 8_000));
        assert_eq!(from_parameters.bank(1), &bank);
        assert_eq!(from_parameters.config(0).unwrap().configuration, 0xdead_beef);
        assert_eq!(from_runtime.axes().axis(0).status, 0);
    }

    /// The identity read is the first two status words.
    #[test]
    fn identity_is_the_first_two_words() {
        assert_eq!(
            Identity::decode(&[103, 20_177]).unwrap(),
            Identity { product_id: 103, program_version: 20_177 }
        );
        assert!(Identity::decode(&[103]).is_err());
    }
}
