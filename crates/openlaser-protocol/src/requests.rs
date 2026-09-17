// SPDX-License-Identifier: GPL-3.0-or-later

//! Every request the host is known to send, as typed constructors.
//!
//! A constructor returns the register address and the words, not a frame,
//! because the transaction number belongs to whoever owns the socket. Nothing
//! here is a generic register writer: if a shape is not in this file, the
//! vendor software was never seen to send it.
//!
//! Numeric inputs are the controller's own units after the vendor's
//! conversions: speeds and accelerations as raw words, head speeds in tenths,
//! head heights in thousandths of a millimetre, travel in controller counts.
//! Evidence: NCModule command builders `0x1003_2490` (jog), `0x1003_2220`
//! (XY move), `0x1003_12b0` (alarm clear), `0x1003_1720` (head move),
//! `0x1003_15b0` (calibration), `0x1002_c340` (head cancel), `0x1003_17e0`
//! and `0x1003_18b0` (follow and retract), MainApp `0x004a_b5a0` (head mode)
//! and `0x0055_6040` (relief); FIFO control and output helpers as noted.

use crate::frame::{Frame, FrameError};
use crate::registers::{self, PARAMETER_BANK_WORDS};
use openlaser_core::LaserMode;

/// A read the host may send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Read {
    /// First register to read.
    pub address: u32,
    /// How many 32-bit words.
    pub words: u16,
}

impl Read {
    /// The read of one whole block.
    #[must_use]
    pub const fn block(block: registers::Block) -> Self {
        Self { address: block.address, words: block.words }
    }

    /// As a frame carrying `transaction`.
    pub fn frame(self, transaction: u16) -> Result<Frame, FrameError> {
        Frame::read(transaction, self.address, self.words)
    }
}

/// A write the host may send.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Write {
    /// First register to write.
    pub address: u32,
    /// The words, in wire order.
    pub words: Vec<u32>,
}

impl Write {
    fn command(words: Vec<u32>) -> Self {
        Self { address: registers::COMMAND, words }
    }

    /// As a frame carrying `transaction`.
    pub fn frame(&self, transaction: u16) -> Result<Frame, FrameError> {
        Frame::write(transaction, self.address, self.words.clone())
    }
}

/// Why a request could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// Output ports run from 1 to 26.
    #[error("output port {0} is outside 1..=26")]
    Port(u8),
    /// Analog channels are 1 and 2.
    #[error("analog channel {0} is neither 1 nor 2")]
    Channel(u8),
    /// Axis indices run from 0 to 4.
    #[error("axis index {0} is outside 0..=4")]
    Axis(u8),
    /// A speed must be a positive signed word.
    #[error("speed {0} is zero or exceeds a signed word")]
    Speed(u32),
    /// An acceleration must be positive and leave room for its tenfold jerk.
    #[error("acceleration {0} is zero or its tenfold exceeds a signed word")]
    Acceleration(u32),
    /// A move of nothing is refused rather than sent.
    #[error("travel is zero")]
    NoTravel,
    /// The vendor helper refuses a zero primary value.
    #[error("{0} must be nonzero")]
    Zero(&'static str),
}

// Motion ---------------------------------------------------------------------

/// Relative jog of one axis: `[3, axis, speed, acceleration, jerk, travel]`,
/// where jerk is ten times the acceleration.
pub fn jog(axis: u8, speed: u32, acceleration: u32, travel: i32) -> Result<Write, RequestError> {
    let jerk = kinematics(speed, acceleration)?;
    if travel == 0 {
        return Err(RequestError::NoTravel);
    }
    Ok(Write::command(vec![
        3,
        u32::from(axis_index(axis)?),
        speed,
        acceleration,
        jerk,
        travel.cast_unsigned(),
    ]))
}

/// Coordinated relative XY move: `[5, 3, speed, acceleration, jerk, dx, dy, 0, 0]`.
pub fn move_xy(delta: [i32; 2], speed: u32, acceleration: u32) -> Result<Write, RequestError> {
    let jerk = kinematics(speed, acceleration)?;
    if delta == [0, 0] {
        return Err(RequestError::NoTravel);
    }
    Ok(Write::command(vec![
        5,
        3,
        speed,
        acceleration,
        jerk,
        delta[0].cast_unsigned(),
        delta[1].cast_unsigned(),
        0,
        0,
    ]))
}

/// Reference search for the axes in `mask` (bit 0 is X, bit 1 is Y):
/// `[2, mask, 0]`.
#[must_use]
pub fn home(mask: u32) -> Write {
    Write::command(vec![2, mask, 0])
}

/// The XY phase of the vendor's Go Origin sequence.
#[must_use]
pub fn home_xy() -> Write {
    home(3)
}

/// Decelerate every axis to a stop: `[1, 31, 2, deceleration, 200000]`.
#[must_use]
pub fn rapid_stop(deceleration: u32) -> Write {
    Write::command(vec![1, 31, 2, deceleration, 200_000])
}

// Head -----------------------------------------------------------------------

/// Head reference search, the first write of Go Origin and the relief for
/// head reference alarms.
#[must_use]
pub fn head_home() -> Write {
    Write::command(vec![102])
}

/// Cancel the head's current command. This stops the head only, never XY.
#[must_use]
pub fn head_cancel() -> Write {
    Write::command(vec![101])
}

/// Start the firmware-owned height calibration.
#[must_use]
pub fn head_calibrate() -> Write {
    Write::command(vec![107])
}

/// Follow the sheet at `height` thousandths of a millimetre, moving at
/// `speed` tenths.
#[must_use]
pub fn head_follow(speed_tenths: u32, height_thousandths: u32) -> Write {
    Write::command(vec![104, speed_tenths, height_thousandths])
}

/// Retract to `height` thousandths of a millimetre at `speed` tenths.
#[must_use]
pub fn head_retract(speed_tenths: u32, height_thousandths: u32) -> Write {
    Write::command(vec![103, speed_tenths, height_thousandths])
}

/// Relative head move: `[109, speed, travel]`. The vendor's Z+ button sends a
/// negative travel and Z- a positive one.
#[must_use]
pub fn head_move(speed_tenths: i32, travel: i32) -> Write {
    Write::command(vec![109, speed_tenths.cast_unsigned(), travel.cast_unsigned()])
}

/// Tell the head controller which laser is in use: `[118, 5, 0]` for fiber,
/// `[118, 5, 1]` for CO2. A setting, not a movement.
#[must_use]
pub fn head_mode(mode: LaserMode) -> Write {
    Write::command(vec![118, 5, u32::from(mode == LaserMode::Co2)])
}

// Outputs --------------------------------------------------------------------

/// The two digital output banks and their selector words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputBank {
    /// Ports 1 to 10, selector 2.
    Standard,
    /// Ports 11 to 26 on the expansion bank, selector 13.
    Extended,
}

impl OutputBank {
    const fn selector(self) -> u32 {
        match self {
            Self::Standard => 2,
            Self::Extended => 13,
        }
    }
}

/// One numbered output mapped to its controller bank and bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputPort {
    /// The bank containing this port.
    pub bank: OutputBank,
    /// The port's single-bit mask within the bank.
    pub mask: u16,
}

impl OutputPort {
    /// Maps ports 1–10 to the standard bank and 11–26 to the extension.
    pub const fn new(port: u8) -> Result<Self, RequestError> {
        let (bank, bit) = match port {
            1..=10 => (OutputBank::Standard, port - 1),
            11..=26 => (OutputBank::Extended, port - 11),
            _ => return Err(RequestError::Port(port)),
        };
        Ok(Self { bank, mask: 1u16 << bit })
    }
}

/// Write the ports selected by `mask` in one bank to `values`:
/// `[9999, selector, mask, values]`. Bit 0 is the bank's first port.
#[must_use]
pub fn digital_outputs(bank: OutputBank, mask: u16, values: u16) -> Write {
    Write::command(vec![9999, bank.selector(), u32::from(mask), u32::from(values)])
}

/// Switch one port on or off, choosing the bank by port number.
pub fn digital_output(port: u8, on: bool) -> Result<Write, RequestError> {
    let OutputPort { bank, mask } = OutputPort::new(port)?;
    Ok(digital_outputs(bank, mask, if on { mask } else { 0 }))
}

/// Set analog channel 1 or 2 to a raw value: `[9999, 4, channel - 1, value]`.
pub fn analog_output(channel: u8, value: u32) -> Result<Write, RequestError> {
    if !matches!(channel, 1 | 2) {
        return Err(RequestError::Channel(channel));
    }
    Ok(Write::command(vec![9999, 4, u32::from(channel - 1), value]))
}

/// The raw word the vendor's pressure path sends for a converted value:
/// 1 to 49 becomes 50, anything above 10 000 becomes 10 000, negatives are
/// kept as their signed pattern.
#[must_use]
pub fn pressure_word(converted: i32) -> u32 {
    match converted {
        1..=49 => 50,
        value => value.min(10_000).cast_unsigned(),
    }
}

/// The raw word the vendor's power path sends for a converted value: 1 to 49
/// becomes 50 and there is no upper clamp.
#[must_use]
pub fn power_word(converted: i32) -> u32 {
    match converted {
        1..=49 => 50,
        value => value.cast_unsigned(),
    }
}

// Laser ----------------------------------------------------------------------

/// The two laser control channels the vendor writes through selector words.
/// Fiber and CO2 control type 1 use the primary channel; CO2 control type 2
/// uses the secondary. Shutdown always writes both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LaserChannel {
    /// Selector 3.
    Primary,
    /// Selector 17.
    Secondary,
}

impl LaserChannel {
    /// The selector word.
    #[must_use]
    pub const fn selector(self) -> u32 {
        match self {
            Self::Primary => 3,
            Self::Secondary => 17,
        }
    }
}

/// Apply configured laser values without emitting: `[9999, selector, primary, secondary]`.
pub fn laser_apply(
    channel: LaserChannel,
    primary: u32,
    secondary: u32,
) -> Result<Write, RequestError> {
    if primary == 0 {
        return Err(RequestError::Zero("laser primary value"));
    }
    Ok(Write::command(vec![9999, channel.selector(), primary, secondary]))
}

/// Laser on: `[9999, selector, first, second, 1]`.
#[must_use]
pub fn laser_on(channel: LaserChannel, first: u32, second: u32) -> Write {
    Write::command(vec![9999, channel.selector(), first, second, 1])
}

/// Laser off: `[9999, selector, value, 0, 0]`. The vendor sends this for both
/// channels with the configured point-laser frequency as `value`.
#[must_use]
pub fn laser_off(channel: LaserChannel, value: u32) -> Write {
    Write::command(vec![9999, channel.selector(), value, 0, 0])
}

/// Set PWM: `[9999, 3, first, second, 0]`.
pub fn pwm(first: u32, second: u32) -> Result<Write, RequestError> {
    if first == 0 {
        return Err(RequestError::Zero("PWM first value"));
    }
    Ok(Write::command(vec![9999, 3, first, second, 0]))
}

// FIFO -----------------------------------------------------------------------

/// Reset the controller's processing counter before a fresh job: `[9999, 16]`.
#[must_use]
pub fn process_counter_reset() -> Write {
    Write::command(vec![9999, 16])
}

/// Discard everything queued in the FIFO.
#[must_use]
pub fn fifo_clear() -> Write {
    Write { address: registers::FIFO, words: vec![1] }
}

/// Start executing the FIFO.
#[must_use]
pub fn fifo_start() -> Write {
    Write { address: registers::FIFO, words: vec![2] }
}

/// Stop executing the FIFO.
#[must_use]
pub fn fifo_stop() -> Write {
    Write { address: registers::FIFO, words: vec![3] }
}

/// One of the two FIFO mode words the vendor was seen to send; their meaning
/// is not resolved beyond the observation.
pub const FIFO_MODE_A: u32 = 0x6680_0000;
/// The other observed FIFO mode word.
pub const FIFO_MODE_B: u32 = 0x7880_0000;

/// Configure the FIFO for the axes in `mask` with `mode` bits:
/// `[9999, 1, mask | mode, 0]`.
#[must_use]
pub fn fifo_configure(axis_mask: u32, mode: u32) -> Write {
    Write::command(vec![9999, 1, axis_mask | mode, 0])
}

/// Upload records to the FIFO: `[stamp, records...]` at the program register.
#[must_use]
pub fn program(stamp: u32, records: &[u32]) -> Write {
    let mut words = Vec::with_capacity(records.len() + 1);
    words.push(stamp);
    words.extend_from_slice(records);
    Write { address: registers::PROGRAM, words }
}

// Alarms and parameters ------------------------------------------------------

/// The common alarm reset: `[9999, 5, 0, 0]`.
#[must_use]
pub fn alarm_clear() -> Write {
    Write::command(vec![9999, 5, 0, 0])
}

/// Bus reset, the relief for native alarm 8025; a common reset follows it.
#[must_use]
pub fn bus_reset() -> Write {
    Write { address: registers::BUS, words: vec![8888] }
}

/// Dual-drive reset for the axes in `mask`: `[9999, 8, mask]`.
#[must_use]
pub fn dual_drive_reset(mask: u32) -> Write {
    Write::command(vec![9999, 8, mask])
}

/// Restore the soft limits of axis `index`: lower then upper.
pub fn axis_limits(index: u8, lower: i32, upper: i32) -> Result<Write, RequestError> {
    Ok(Write {
        address: registers::axis_limits(axis_index(index)?),
        words: vec![lower.cast_unsigned(), upper.cast_unsigned()],
    })
}

/// Write a whole parameter bank of axis `index`.
pub fn parameter_bank(
    index: u8,
    words: [u32; PARAMETER_BANK_WORDS],
) -> Result<Write, RequestError> {
    Ok(Write { address: registers::parameter_bank(axis_index(index)?), words: words.to_vec() })
}

/// Activate written parameters: `[9999]` at the bus register.
#[must_use]
pub fn parameters_activate() -> Write {
    Write { address: registers::BUS, words: vec![9999] }
}

fn axis_index(index: u8) -> Result<u8, RequestError> {
    if usize::from(index) < registers::AXIS_COUNT {
        Ok(index)
    } else {
        Err(RequestError::Axis(index))
    }
}

/// Validates speed and acceleration and returns the jerk word, ten times the
/// acceleration, as the vendor computes it.
fn kinematics(speed: u32, acceleration: u32) -> Result<u32, RequestError> {
    if speed == 0 || speed > i32::MAX.cast_unsigned() {
        return Err(RequestError::Speed(speed));
    }
    match acceleration.checked_mul(10) {
        Some(jerk) if acceleration > 0 && jerk <= i32::MAX.cast_unsigned() => Ok(jerk),
        _ => Err(RequestError::Acceleration(acceleration)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five writes captured during a real stop are reproduced exactly:
    /// head cancel, rapid stop, fiber head mode, FIFO stop, FIFO clear.
    #[test]
    fn captured_stop_writes_are_reproduced() {
        assert_eq!(head_cancel().words, [101]);
        assert_eq!(rapid_stop(5999).words, [1, 31, 2, 5999, 200_000]);
        assert_eq!(head_mode(LaserMode::Fiber).words, [118, 5, 0]);
        assert_eq!(fifo_stop(), Write { address: 103, words: vec![3] });
        assert_eq!(fifo_clear(), Write { address: 103, words: vec![1] });
    }

    /// Jog and XY move carry the tenfold jerk word and two's-complement travel.
    #[test]
    fn motion_words_match_the_vendor_builders() {
        assert_eq!(
            jog(0, 500, 2000, -100).unwrap().words,
            [3, 0, 500, 2000, 20_000, (-100i32).cast_unsigned()]
        );
        assert_eq!(
            move_xy([500, -800], 500, 599).unwrap().words,
            [5, 3, 500, 599, 5990, 500, (-800i32).cast_unsigned(), 0, 0]
        );
        assert_eq!(home_xy().words, [2, 3, 0]);
        assert_eq!(home(0x15).words, [2, 0x15, 0]);
    }

    /// Zero travel, zero speed, and accelerations whose jerk overflows a
    /// signed word are refused before anything could reach the machine.
    #[test]
    fn motion_limits_are_enforced() {
        assert_eq!(jog(0, 500, 2000, 0), Err(RequestError::NoTravel));
        assert_eq!(jog(0, 0, 2000, 1), Err(RequestError::Speed(0)));
        assert_eq!(jog(0, 500, 0, 1), Err(RequestError::Acceleration(0)));
        assert_eq!(
            jog(0, 500, i32::MAX.cast_unsigned(), 1),
            Err(RequestError::Acceleration(i32::MAX.cast_unsigned()))
        );
        assert_eq!(jog(5, 500, 2000, 1), Err(RequestError::Axis(5)));
        assert_eq!(move_xy([0, 0], 500, 2000), Err(RequestError::NoTravel));
    }

    /// Head commands are single selector words with the vendor's argument
    /// scaling already applied by the caller.
    #[test]
    fn head_commands_match_the_vendor_builders() {
        assert_eq!(head_home().words, [102]);
        assert_eq!(head_calibrate().words, [107]);
        assert_eq!(head_follow(300, 10_000).words, [104, 300, 10_000]);
        assert_eq!(head_retract(300, 5000).words, [103, 300, 5000]);
        assert_eq!(head_move(125, -1_000_000).words, [109, 125, 0xfff0_bdc0]);
        assert_eq!(head_mode(LaserMode::Co2).words, [118, 5, 1]);
    }

    /// Port numbers map to bank, mask and shifted value the way the native
    /// output setter does; port 0 and ports above 26 are refused.
    #[test]
    fn digital_outputs_select_bank_by_port() {
        assert_eq!(digital_output(1, true).unwrap().words, [9999, 2, 1, 1]);
        assert_eq!(digital_output(7, true).unwrap().words, [9999, 2, 64, 64]);
        assert_eq!(digital_output(7, false).unwrap().words, [9999, 2, 64, 0]);
        assert_eq!(digital_output(11, true).unwrap().words, [9999, 13, 1, 1]);
        assert_eq!(digital_output(26, false).unwrap().words, [9999, 13, 1 << 15, 0]);
        assert_eq!(digital_output(0, true), Err(RequestError::Port(0)));
        assert_eq!(digital_output(27, true), Err(RequestError::Port(27)));
        assert_eq!(
            digital_outputs(OutputBank::Standard, 0xffff, 0x1fd).words,
            [9999, 2, 65_535, 0x1fd]
        );
    }

    /// Analog channels are zero-based on the wire; the pressure path clamps
    /// both ends and the power path only the bottom.
    #[test]
    fn analog_values_follow_the_two_vendor_paths() {
        assert_eq!(analog_output(1, 2500).unwrap().words, [9999, 4, 0, 2500]);
        assert_eq!(analog_output(2, 0).unwrap().words, [9999, 4, 1, 0]);
        assert_eq!(analog_output(3, 0), Err(RequestError::Channel(3)));
        for (input, want) in [
            (0, 0),
            (1, 50),
            (49, 50),
            (50, 50),
            (10_000, 10_000),
            (10_001, 10_000),
            (-3, (-3i32).cast_unsigned()),
        ] {
            assert_eq!(pressure_word(input), want, "pressure {input}");
        }
        assert_eq!(power_word(12_000), 12_000);
        assert_eq!(power_word(7), 50);
        assert_eq!(power_word(0), 0);
    }

    /// Laser apply, on, off and PWM use the channel selector in word 1 and
    /// the vendor's fixed tails; zero primaries are refused.
    #[test]
    fn laser_words_match_the_vendor_helpers() {
        assert_eq!(laser_apply(LaserChannel::Primary, 100, 7).unwrap().words, [9999, 3, 100, 7]);
        assert_eq!(laser_apply(LaserChannel::Secondary, 1, 0).unwrap().words, [9999, 17, 1, 0]);
        assert_eq!(
            laser_apply(LaserChannel::Primary, 0, 7),
            Err(RequestError::Zero("laser primary value"))
        );
        assert_eq!(laser_on(LaserChannel::Secondary, 5, 6).words, [9999, 17, 5, 6, 1]);
        assert_eq!(laser_off(LaserChannel::Primary, 0).words, [9999, 3, 0, 0, 0]);
        assert_eq!(pwm(2, 3).unwrap().words, [9999, 3, 2, 3, 0]);
        assert!(pwm(0, 3).is_err());
    }

    /// FIFO control words, the configure request and the program upload
    /// layout with its leading stamp.
    #[test]
    fn fifo_and_program_requests_are_exact() {
        assert_eq!(fifo_start(), Write { address: 103, words: vec![2] });
        assert_eq!(process_counter_reset().words, [9999, 16]);
        assert_eq!(fifo_configure(0b11, FIFO_MODE_A).words, [9999, 1, 0x6680_0003, 0]);
        assert_eq!(program(7, &[1, 2, 3]), Write { address: 102, words: vec![7, 1, 2, 3] });
    }

    /// Relief and parameter writes go to their own registers with the
    /// vendor's magic words.
    #[test]
    fn relief_and_parameter_requests_are_exact() {
        assert_eq!(alarm_clear().words, [9999, 5, 0, 0]);
        assert_eq!(bus_reset(), Write { address: 100, words: vec![8888] });
        assert_eq!(dual_drive_reset(3).words, [9999, 8, 3]);
        assert_eq!(
            axis_limits(1, -500, 3_000_000).unwrap(),
            Write { address: 50_242, words: vec![(-500i32).cast_unsigned(), 3_000_000] }
        );
        assert_eq!(parameter_bank(2, [7; 14]).unwrap().address, 50_280);
        assert_eq!(parameters_activate(), Write { address: 100, words: vec![9999] });
        assert!(axis_limits(5, 0, 1).is_err());
    }

    /// A request becomes a frame with the caller's transaction number and the
    /// right shape for reads and writes.
    #[test]
    fn requests_become_frames() {
        let read = Read::block(registers::STATUS).frame(9).unwrap();
        assert_eq!((read.address, read.count, read.values.len()), (1000, 36, 0));
        let write = head_mode(LaserMode::Co2).frame(10).unwrap();
        assert_eq!((write.transaction, write.address, write.count), (10, 101, 3));
        assert_eq!(write.values, [118, 5, 1]);
    }
}
