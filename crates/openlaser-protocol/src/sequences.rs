// SPDX-License-Identifier: GPL-3.0-or-later

//! The multi-write sequences the vendor host composes from single requests:
//! gas on, off and switch, shutter, CO2 laser enable, output shutdown, alarm
//! relief and the laser mode switch, plus the value conversions that feed
//! them. Everything here is a pure plan; sending it is the controller's job.
//!
//! Evidence: MainApp `0x005a_9b80` (gas toggle), `0x005c_b1d0` (gas switch),
//! `0x0062_fc60` (pressure curve), `0x0055_6040` (relief), `0x004a_9ea0`
//! (dual drive); NCModule `0x1003_0860` and `0x1003_0c30` (CO2 laser),
//! `0x1003_0490` and `0x1003_05b0` (analog scaling), `0x1005_7240` (stop),
//! `0x1002_e100` (mode switch); the prototype's loopback stop tests.

use crate::alarms::{self, Relief};
use crate::requests::{self, LaserChannel, OutputBank, RequestError, Write};
use openlaser_core::LaserMode;

/// One step of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Send this write and wait for its acknowledgement.
    Write(Write),
    /// Wait this many milliseconds before the next step.
    Delay(u32),
}

/// Why a plan could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SequenceError {
    /// A single request in the plan was invalid.
    #[error(transparent)]
    Request(#[from] RequestError),
    /// A converted process value was negative.
    #[error("process value is negative")]
    NegativeValue,
    /// The pressure curve or full-scale value cannot be used.
    #[error("pressure calibration is unusable")]
    PressureCurve,
    /// The requested pressure is negative or not finite.
    #[error("pressure is negative or not a number")]
    Pressure,
    /// Gas selectors run from 0 to 5.
    #[error("gas selector is outside 0..=5")]
    GasSelector,
    /// A low gas selector needs its pressure output binding.
    #[error("gas binding is missing its pressure output")]
    PressureBinding,
    /// The shutter provider branch is not reproduced.
    #[error("shutter branch is not supported")]
    Shutter,
    /// This alarm is not relieved by a reset write.
    #[error("{0}")]
    Relief(&'static str),
    /// The mode switch plan does not satisfy its own rules.
    #[error("{0}")]
    ModeSwitch(&'static str),
    /// A shutdown port cannot be encoded: above 10 without the extended bank.
    #[error("shutdown output {0} needs the extended bank")]
    ShutdownPort(u8),
    /// A stop value is outside the vendor's conversion domain.
    #[error("{0}")]
    Stop(&'static str),
}

// Value conversions ----------------------------------------------------------

/// The vendor's power scaling before truncation: mode 0 scales by 100,
/// mode 1 by 50, mode 2 by 40, anything else to zero.
#[must_use]
pub fn power_scaled(mode: i32, value: f64) -> f64 {
    match mode {
        0 => value * 100.0,
        1 => value * 0.5 * 100.0,
        2 => value * 0.4 * 100.0,
        _ => 0.0,
    }
}

/// The vendor's pressure scaling before truncation: a thousand per unit.
#[must_use]
pub fn pressure_scaled(value: f64) -> f64 {
    value * 1000.0
}

/// One point of a pressure calibration curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressurePoint {
    /// Output voltage.
    pub voltage: f64,
    /// Pressure at that voltage.
    pub pressure: f64,
}

/// The voltage for a pressure: linear over `full_scale` without a curve, and
/// piecewise linear between the curve's points with it, extrapolating past
/// the ends as the vendor does.
pub fn pressure_voltage(
    pressure: f64,
    full_scale: f64,
    curve: &[PressurePoint],
) -> Result<f64, SequenceError> {
    if !pressure.is_finite() || pressure < 0.0 {
        return Err(SequenceError::Pressure);
    }
    let value = if curve.is_empty() {
        if !full_scale.is_finite() || full_scale <= 0.0 {
            return Err(SequenceError::PressureCurve);
        }
        10.0 * pressure / full_scale
    } else {
        let bad_point = |p: &PressurePoint| {
            !p.voltage.is_finite()
                || p.voltage < 0.0
                || !p.pressure.is_finite()
                || p.pressure <= 0.0
        };
        if curve.iter().any(bad_point)
            || curve.windows(2).any(|pair| pair[0].pressure > pair[1].pressure)
        {
            return Err(SequenceError::PressureCurve);
        }
        let upper = curve.iter().position(|p| pressure <= p.pressure).unwrap_or(curve.len() - 1);
        if upper == 0 {
            pressure * curve[0].voltage / curve[0].pressure
        } else {
            let (a, b) = (curve[upper - 1], curve[upper]);
            // The vendor's exact comparison of two calibration points.
            if a.pressure.to_bits() == b.pressure.to_bits() {
                b.voltage
            } else {
                (b.voltage - a.voltage) * (pressure - a.pressure) / (b.pressure - a.pressure)
                    + a.voltage
            }
        }
    };
    if value.is_finite() { Ok(value) } else { Err(SequenceError::PressureCurve) }
}

/// The stop-related settings in the vendor's stored units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StopKinematics {
    /// `MP.JogStopDccFactor`.
    pub jog_stop_deceleration_factor: i32,
    /// `MC.XFastMoveAcc`.
    pub fast_move_acceleration: f64,
    /// `FCP.MaxAcc`.
    pub maximum_acceleration: f64,
    /// `ZF.ZFUpSpeed`.
    pub z_up_speed: f64,
    /// `ZF.ZFDockHeight`.
    pub z_dock_height: f64,
    /// `MP.ZFSafeHeight`.
    pub z_safe_height: f64,
}

/// The stop words the vendor derives from [`StopKinematics`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StopWords {
    /// The rapid-stop deceleration: acceleration times factor, clamped to
    /// at least 2000 and at most the maximum acceleration, truncated.
    pub rapid_deceleration: u32,
    /// Head retract speed in tenths.
    pub retract_speed: u32,
    /// The dock height in thousandths, used by the short stop branch.
    pub short_retract_height: u32,
    /// The safe height in thousandths, used by the long stop branch.
    pub long_retract_height: u32,
}

impl StopKinematics {
    /// The vendor's multiply, clamp and truncate chain, refusing values that
    /// would not fit a signed word or that truncate a speed to zero.
    pub fn words(self) -> Result<StopWords, SequenceError> {
        if self.jog_stop_deceleration_factor <= 0
            || !self.fast_move_acceleration.is_finite()
            || self.fast_move_acceleration <= 0.0
            || !self.maximum_acceleration.is_finite()
            || self.maximum_acceleration <= 0.0
        {
            return Err(SequenceError::Stop("invalid stop acceleration settings"));
        }
        let acceleration =
            self.fast_move_acceleration * f64::from(self.jog_stop_deceleration_factor);
        if !acceleration.is_finite() {
            return Err(SequenceError::Stop("stop acceleration product overflows"));
        }
        Ok(StopWords {
            rapid_deceleration: word(
                acceleration.max(2000.0).min(self.maximum_acceleration),
                true,
            )?,
            retract_speed: word(self.z_up_speed * 10.0, true)?,
            short_retract_height: word(self.z_dock_height * 1000.0, false)?,
            long_retract_height: word(self.z_safe_height * 1000.0, false)?,
        })
    }
}

fn word(value: f64, must_be_positive: bool) -> Result<u32, SequenceError> {
    if !value.is_finite() || !(0.0..2_147_483_648.0).contains(&value) {
        return Err(SequenceError::Stop("stop value exceeds the signed native range"));
    }
    // Truncation is the vendor's conversion; the range check above makes it lossless.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "range checked above"
    )]
    let truncated = value.trunc() as u32;
    if must_be_positive && truncated == 0 {
        return Err(SequenceError::Stop("stop speed or acceleration truncates to zero"));
    }
    Ok(truncated)
}

// Gas ------------------------------------------------------------------------

/// The pressure outputs of a low-pressure gas selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PressureOutput {
    /// The auxiliary valve port, zero for none.
    pub auxiliary_port: u8,
    /// The proportional valve's analog channel, zero for none.
    pub analog_channel: u8,
}

/// What the vendor does for a high gas selector's analog output when
/// switching from a low one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighSelectorAnalog {
    /// The configuration record does not say.
    Unresolved,
    /// No analog write.
    None,
    /// Write this channel.
    Channel(u8),
}

/// A gas selector's bindings from the configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GasBinding {
    /// The selector, 0 to 5: low air, oxygen, nitrogen, then high air,
    /// oxygen, nitrogen.
    pub selector: u8,
    /// The main valve port, zero for none.
    pub main_port: u8,
    /// The pressure outputs; required for selectors 0 to 2.
    pub pressure: Option<PressureOutput>,
    /// The analog behaviour for selectors 3 to 5.
    pub high_selector_analog: HighSelectorAnalog,
    /// The settle delay between valve and pressure, in milliseconds.
    pub delay_ms: u32,
}

/// Gas on or off. `pressure_converted` is the pressure after curve, scaling
/// and truncation, before the clamp.
pub fn gas_toggle(
    binding: GasBinding,
    on: bool,
    pressure_converted: i32,
) -> Result<Vec<Step>, SequenceError> {
    if binding.selector > 5 {
        return Err(SequenceError::GasSelector);
    }
    let mut steps = Vec::new();
    output(&mut steps, binding.main_port, on)?;
    if let Some(pressure) = binding.proportional()? {
        if on {
            let value = pressure_value(pressure_converted)?;
            pressure_on(&mut steps, pressure, value, binding.delay_ms)?;
        } else {
            pressure_off(&mut steps, pressure)?;
        }
    }
    Ok(steps)
}

impl GasBinding {
    fn proportional(self) -> Result<Option<PressureOutput>, SequenceError> {
        if self.selector < 3 {
            self.pressure.map(Some).ok_or(SequenceError::PressureBinding)
        } else {
            Ok(None)
        }
    }

    fn switch_channel(self) -> Result<u8, SequenceError> {
        if let Some(pressure) = self.proportional()? {
            return Ok(pressure.analog_channel);
        }
        match self.high_selector_analog {
            HighSelectorAnalog::Unresolved => Err(SequenceError::PressureBinding),
            HighSelectorAnalog::None => Ok(0),
            HighSelectorAnalog::Channel(channel) => Ok(channel),
        }
    }
}

fn pressure_value(converted: i32) -> Result<u32, SequenceError> {
    if converted < 0 {
        Err(SequenceError::NegativeValue)
    } else {
        Ok(requests::pressure_word(converted))
    }
}

fn pressure_on(
    steps: &mut Vec<Step>,
    pressure: PressureOutput,
    value: u32,
    delay: u32,
) -> Result<(), SequenceError> {
    output(steps, pressure.auxiliary_port, true)?;
    steps.push(Step::Delay(delay));
    analog(steps, pressure.analog_channel, value)
}

fn pressure_off(steps: &mut Vec<Step>, pressure: PressureOutput) -> Result<(), SequenceError> {
    analog(steps, pressure.analog_channel, 0)?;
    output(steps, pressure.auxiliary_port, false)
}

/// Switch from one gas to another while gas is on: the new valve first, the
/// new pressure, then the old valve and pressure off. Nothing when gas is off
/// or the selector is unchanged.
pub fn gas_switch(
    old: GasBinding,
    new: GasBinding,
    gas_on: bool,
    new_pressure_converted: i32,
) -> Result<Vec<Step>, SequenceError> {
    if !switch_required(old, new, gas_on)? {
        return Ok(Vec::new());
    }
    let mut steps = Vec::new();
    output(&mut steps, new.main_port, true)?;
    let old_pressure = old.proportional()?;
    if let Some(pressure) = old_pressure {
        switch_pressure(&mut steps, pressure, new, new_pressure_converted)?;
    }
    output(&mut steps, old.main_port, false)?;
    if let Some(pressure) = old_pressure {
        pressure_off(&mut steps, pressure)?;
    }
    Ok(steps)
}

fn switch_required(old: GasBinding, new: GasBinding, gas_on: bool) -> Result<bool, SequenceError> {
    if old.selector > 5 || new.selector > 5 {
        return Err(SequenceError::GasSelector);
    }
    if !gas_on || old.selector == new.selector {
        return Ok(false);
    }
    if old.delay_ms != new.delay_ms {
        return Err(SequenceError::PressureBinding);
    }
    Ok(true)
}

fn switch_pressure(
    steps: &mut Vec<Step>,
    old: PressureOutput,
    new: GasBinding,
    converted: i32,
) -> Result<(), SequenceError> {
    let value = pressure_value(converted)?;
    let pressure = PressureOutput { analog_channel: new.switch_channel()?, ..old };
    // The native switch uses the old auxiliary valve and the new channel;
    // its converter returns zero for a high selector.
    pressure_on(steps, pressure, if new.selector < 3 { value } else { 0 }, new.delay_ms)
}

// Shutter and CO2 laser ------------------------------------------------------

/// The shutter setter's configuration branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShutterBinding {
    /// Laser mode 1 with a nonzero control type: a port and an analog channel.
    Mode1 {
        /// Whether the shutter output is enabled at all.
        output_enabled: bool,
        /// The shutter port, zero for none.
        port: u8,
        /// The analog channel zeroed on close, zero for none.
        analog_channel: u8,
    },
    /// Laser mode 0 with control type 3: a port and an analog on value.
    Mode0Type3 {
        /// The shutter port.
        port: u8,
        /// The analog channel.
        analog_channel: u8,
        /// The converted analog value written on open.
        converted_on: i32,
        /// Whether closing leaves the analog value alone.
        preserve_analog_on_close: bool,
    },
    /// A provider branch the evidence does not close.
    Unresolved,
}

/// Open or close the shutter.
pub fn shutter(binding: ShutterBinding, open: bool) -> Result<Vec<Step>, SequenceError> {
    match binding {
        ShutterBinding::Mode1 { output_enabled, port, analog_channel } => {
            shutter_mode1(output_enabled, port, analog_channel, open)
        }
        ShutterBinding::Mode0Type3 {
            port,
            analog_channel,
            converted_on,
            preserve_analog_on_close,
        } => shutter_analog(port, analog_channel, converted_on, preserve_analog_on_close, open),
        ShutterBinding::Unresolved => Err(SequenceError::Shutter),
    }
}

fn shutter_mode1(
    enabled: bool,
    port: u8,
    channel: u8,
    open: bool,
) -> Result<Vec<Step>, SequenceError> {
    let mut steps = Vec::new();
    if enabled {
        if !open {
            analog(&mut steps, channel, 0)?;
        }
        output(&mut steps, port, open)?;
    }
    Ok(steps)
}

fn shutter_analog(
    port: u8,
    channel: u8,
    converted: i32,
    keep: bool,
    open: bool,
) -> Result<Vec<Step>, SequenceError> {
    if open && converted < 0 {
        return Err(SequenceError::NegativeValue);
    }
    let mut steps = Vec::new();
    output(&mut steps, port, open)?;
    if open || !keep {
        analog(&mut steps, channel, if open { requests::power_word(converted) } else { 0 })?;
    }
    Ok(steps)
}

/// The CO2 laser enable's configuration, separate from the shutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Co2Laser {
    /// The control type: 0 disables the whole path, 1 and 2 pick the channel.
    pub control_type: u8,
    /// The shutter flag that gates enabling.
    pub shutter_flag: bool,
    /// The analog channel for power, zero for none.
    pub analog_channel: u8,
    /// The converted analog on value.
    pub analog_on: i32,
    /// The laser enable output port, zero for none.
    pub laser_port: u8,
    /// The first on word.
    pub on_first: u32,
    /// The second on word.
    pub on_second: u32,
    /// The off word, the point-laser frequency.
    pub off_value: u32,
    /// Whether the off words are suppressed.
    pub suppress_off: bool,
}

/// Enable or disable the CO2 laser.
pub fn co2_laser(laser: Co2Laser, enable: bool) -> Result<Vec<Step>, SequenceError> {
    if enable { laser.enable() } else { laser.disable() }
}

impl Co2Laser {
    fn enable(self) -> Result<Vec<Step>, SequenceError> {
        let mut steps = Vec::new();
        if self.shutter_flag && self.control_type != 0 {
            analog(&mut steps, self.analog_channel, requests::power_word(self.analog_on))?;
            let channel = match self.control_type {
                1 => Some(LaserChannel::Primary),
                2 => Some(LaserChannel::Secondary),
                _ => None,
            };
            if let Some(channel) = channel {
                steps.push(Step::Write(requests::laser_on(channel, self.on_first, self.on_second)));
            }
            output(&mut steps, self.laser_port, true)?;
        }
        Ok(steps)
    }

    fn disable(self) -> Result<Vec<Step>, SequenceError> {
        let mut steps = Vec::new();
        if self.control_type != 0 {
            output(&mut steps, self.laser_port, false)?;
            analog(&mut steps, self.analog_channel, 0)?;
        }
        if !self.suppress_off {
            steps.extend(laser_off_pair(self.off_value).map(Step::Write));
        }
        Ok(steps)
    }
}

/// The laser off pair the vendor always sends together, primary then
/// secondary channel.
#[must_use]
pub fn laser_off_pair(value: u32) -> [Write; 2] {
    [
        requests::laser_off(LaserChannel::Primary, value),
        requests::laser_off(LaserChannel::Secondary, value),
    ]
}

// Shutdown -------------------------------------------------------------------

/// The configuration a shutdown needs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shutdown {
    /// The selected laser.
    pub mode: LaserMode,
    /// Whether a head controller is configured.
    pub head_enabled: bool,
    /// Whether CO2 mode skips the head cancel.
    pub co2_skip_head_cancel: bool,
    /// Whether the extended output bank exists.
    pub extended_outputs: bool,
    /// The CO2 analog channel, zero for none.
    pub co2_analog_channel: u8,
    /// The three gas analog channels, zero for none.
    pub gas_channels: [u8; 3],
    /// The point-laser frequency written with the laser off pair.
    pub point_laser_frequency: u32,
    /// The rapid-stop deceleration word.
    pub rapid_deceleration: u32,
    /// Every output port the vendor switches off, zeros ignored.
    pub ports: Vec<u8>,
    /// The head raise after a pause, when a head is configured and the
    /// mode keeps it: `ZF.ZFType` nonzero and not CO2 with
    /// `MP.CO2DisabledZAxis`.
    pub raise: Option<Raise>,
}

/// The pause retract: after the head cancel and fresh head feedback, `[103, speed, height]` to `MP.ZFSafeHeight`
/// at `ZF.ZFUpSpeed`, when the head is at least that far below its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Raise {
    /// `ZF.ZFUpSpeed` in tenths.
    pub speed_tenths: u32,
    /// `MP.ZFSafeHeight` in thousandths below the head's origin.
    pub height_thousandths: u32,
}

impl Raise {
    /// The retract request.
    #[must_use]
    pub fn write(self) -> Write {
        requests::head_retract(self.speed_tenths, self.height_thousandths)
    }
}

/// The host's output-off policy, shared by normal completion and fault stop.
///
/// With `stop_axes` it first decelerates the axes, cancels the head when
/// configured (`head_active` forces the cancel even when CO2 skips it), and
/// stops the FIFO. Then the laser off pair, every analog channel to zero,
/// and both output banks masked off. Nothing here reads feedback or moves
/// the head.
pub fn shutdown(
    config: &Shutdown,
    stop_axes: bool,
    head_active: bool,
    extra_analog: &[u8],
) -> Result<Vec<Write>, SequenceError> {
    let mut writes = stop_and_laser_off(config, stop_axes, head_active);
    writes.extend(gas_and_outputs_off(config, extra_analog)?);
    Ok(writes)
}

/// The first part of [`shutdown`]: the axes, the head cancel and the FIFO
/// when `stop_axes`, then the laser off pair. A pause that raises the head
/// sends this, raises, then sends [`gas_and_outputs_off`].
#[must_use]
pub fn stop_and_laser_off(config: &Shutdown, stop_axes: bool, head_active: bool) -> Vec<Write> {
    let mut writes = Vec::new();
    if stop_axes {
        writes.push(requests::rapid_stop(config.rapid_deceleration));
        let co2_skips = config.mode == LaserMode::Co2 && config.co2_skip_head_cancel;
        if config.head_enabled && (!co2_skips || head_active) {
            writes.push(requests::head_cancel());
        }
        writes.push(requests::fifo_stop());
    }
    writes.extend(laser_off_pair(config.point_laser_frequency));
    writes
}

/// The rest of [`shutdown`]: every analog channel to zero and both output
/// banks masked off, the gas with them.
pub fn gas_and_outputs_off(
    config: &Shutdown,
    extra_analog: &[u8],
) -> Result<Vec<Write>, SequenceError> {
    let mut writes = Vec::new();
    let channels =
        config.gas_channels.iter().chain([&config.co2_analog_channel]).chain(extra_analog);
    for &channel in channels.filter(|channel| matches!(channel, 1 | 2)) {
        writes.push(requests::analog_output(channel, 0)?);
    }
    let (standard, extended) = shutdown_masks(config)?;
    if standard != 0 {
        writes.push(requests::digital_outputs(OutputBank::Standard, standard, 0));
    }
    if extended != 0 {
        writes.push(requests::digital_outputs(OutputBank::Extended, extended, 0));
    }
    Ok(writes)
}

fn shutdown_masks(config: &Shutdown) -> Result<(u16, u16), SequenceError> {
    let (mut standard, mut extended) = (0u16, 0u16);
    for &port in &config.ports {
        match port {
            0 => {}
            1..=10 => standard |= 1 << (port - 1),
            11..=26 if config.extended_outputs => extended |= 1 << (port - 11),
            _ => return Err(SequenceError::ShutdownPort(port)),
        }
    }
    Ok((standard, extended))
}

/// The abort after lost feedback: the full shutdown with the axes stopped,
/// then the FIFO discarded so queued enabling records cannot run.
pub fn abort(config: &Shutdown, head_active: bool) -> Result<Vec<Write>, SequenceError> {
    let mut writes = shutdown(config, true, head_active, &[])?;
    writes.push(requests::fifo_clear());
    Ok(writes)
}

// Relief ---------------------------------------------------------------------

/// The XY soft limits restored by a dual-drive reset, lower then upper, X
/// then Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DualDrive {
    /// The limits as imported from the machine settings.
    pub xy_limits: [(i32, i32); 2],
}

/// A relief plan and whether the host's retained rows may be cleared after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliefPlan {
    /// The writes, in order.
    pub writes: Vec<Write>,
    /// Whether the host clears its retained alarm rows once acknowledged.
    pub clears_host_rows: bool,
}

/// The reset writes for an alarm id, plus the vendor's common-reset tail
/// when either controller aggregate was set. Alarms relieved by other
/// operations, reconnect, head home, source reset and the like, are refused
/// rather than replaced with an unrelated reset.
pub fn relief(
    id: Option<u32>,
    group_1: u32,
    group_2: u32,
    dual: Option<DualDrive>,
) -> Result<ReliefPlan, SequenceError> {
    let action = id.map_or(Relief::NoDirectHandler, alarms::relief);
    let mut writes = Vec::new();
    let mut clears_host_rows = false;
    match action {
        Relief::CommonClear => {
            writes.push(requests::alarm_clear());
            clears_host_rows = !id.is_some_and(|id| (1..=4).contains(&id));
        }
        Relief::ClearFifoThenCommon => {
            writes.extend([requests::fifo_clear(), requests::alarm_clear()]);
            clears_host_rows = true;
        }
        Relief::ResetBusThenCommon => {
            writes.extend([requests::bus_reset(), requests::alarm_clear()]);
            clears_host_rows = true;
        }
        Relief::DualDriveAndHomeDecision => {
            let dual = dual
                .ok_or(SequenceError::Relief("dual-drive relief needs the imported XY limits"))?;
            let limits = limit_writes(
                &dual.xy_limits,
                SequenceError::Relief("invalid dual-drive reset limits"),
            )?;
            writes.push(requests::dual_drive_reset(3));
            writes.extend(limits);
            writes.push(requests::alarm_clear());
            clears_host_rows = true;
        }
        Relief::HeadHome => {
            return Err(SequenceError::Relief("relieved by head home, not a reset"));
        }
        Relief::Reconnect => return Err(SequenceError::Relief("relieved by reconnecting")),
        Relief::ServiceItem | Relief::NoDirectHandler => {}
        Relief::SourceReset
        | Relief::CheckPulseEquivalent
        | Relief::AutofocusHome
        | Relief::UnlockLaser
        | Relief::AutofocusHomeDialog => {
            return Err(SequenceError::Relief("no reset write relieves this alarm"));
        }
    }
    if alarms::relief_has_controller_tail(group_1, group_2) {
        writes.push(requests::alarm_clear());
        clears_host_rows = true;
    }
    Ok(ReliefPlan { writes, clears_host_rows })
}

// Mode switch ----------------------------------------------------------------

/// The laser mode switch: outputs off, the head told the new mode, the mode's
/// enable output on, and the XY soft limits for that mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeSwitch {
    /// The mode being switched to.
    pub mode: LaserMode,
    /// Whether a head controller is configured.
    pub head_enabled: bool,
    /// Writes that only switch things off: outputs, analogs, PWM.
    pub cleanup: Vec<Write>,
    /// The output that enables the new mode, if any.
    pub enable: Option<Write>,
    /// The XY soft limits for the new mode, lower then upper, X then Y.
    pub xy_limits: [(i32, i32); 2],
}

impl ModeSwitch {
    /// The writes in the vendor's order. Refuses a plan whose cleanup could
    /// enable something, whose enable is not an output, or whose limits are
    /// not a sane pair.
    pub fn plan(&self) -> Result<Vec<Write>, SequenceError> {
        if self.cleanup.is_empty() {
            return Err(SequenceError::ModeSwitch("mode cleanup is empty"));
        }
        if !self.cleanup.iter().all(switches_off) {
            return Err(SequenceError::ModeSwitch("mode cleanup contains an enabling write"));
        }
        if let Some(enable) = &self.enable {
            match enable.words.as_slice() {
                [9999, 2 | 13, mask, values] if values == mask && *mask != 0 => {}
                _ => return Err(SequenceError::ModeSwitch("mode enable is not an output on")),
            }
        }
        let limits =
            limit_writes(&self.xy_limits, SequenceError::ModeSwitch("invalid mode XY limits"))?;
        let mut writes = self.cleanup.clone();
        if self.head_enabled {
            writes.push(requests::head_mode(self.mode));
        }
        writes.extend(self.enable.clone());
        writes.extend(limits);
        Ok(writes)
    }
}

fn limit_writes(
    limits: &[(i32, i32); 2],
    invalid: SequenceError,
) -> Result<Vec<Write>, SequenceError> {
    if limits.iter().any(|&(lower, upper)| lower > 0 || upper < 0 || lower >= upper) {
        return Err(invalid);
    }
    (0u8..)
        .zip(limits)
        .map(|(axis, &(lower, upper))| Ok(requests::axis_limits(axis, lower, upper)?))
        .collect()
}

/// Whether a command-register write only switches something off: outputs to
/// zero, an analog channel to zero, or PWM with zero power.
fn switches_off(write: &Write) -> bool {
    matches!(
        write.words.as_slice(),
        [9999, 2 | 13, _, 0] | [9999, 4, 0 | 1, 0] | [9999, 3 | 17, _, 0, 0]
    )
}

fn output(steps: &mut Vec<Step>, port: u8, on: bool) -> Result<(), SequenceError> {
    // The vendor ignores an unassigned port.
    if port != 0 {
        steps.push(Step::Write(requests::digital_output(port, on)?));
    }
    Ok(())
}

fn analog(steps: &mut Vec<Step>, channel: u8, value: u32) -> Result<(), SequenceError> {
    if channel != 0 {
        steps.push(Step::Write(requests::analog_output(channel, value)?));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(steps: &[Step]) -> Vec<Vec<u32>> {
        steps
            .iter()
            .map(|step| match step {
                Step::Write(write) => write.words.clone(),
                Step::Delay(ms) => vec![*ms],
            })
            .collect()
    }

    fn low_gas(selector: u8, main_port: u8, analog_channel: u8) -> GasBinding {
        GasBinding {
            selector,
            main_port,
            pressure: Some(PressureOutput { auxiliary_port: 7, analog_channel }),
            high_selector_analog: HighSelectorAnalog::None,
            delay_ms: 120,
        }
    }

    /// Gas on writes main valve, auxiliary valve, the settle delay, then the
    /// pressure; gas off writes main valve, pressure zero, auxiliary valve.
    /// A high selector has no pressure steps and a negative pressure is refused.
    #[test]
    fn gas_toggle_orders_valves_delay_and_pressure() {
        let on = gas_toggle(low_gas(0, 1, 1), true, 50).unwrap();
        assert_eq!(
            words(&on),
            [vec![9999, 2, 1, 1], vec![9999, 2, 64, 64], vec![120], vec![9999, 4, 0, 50]]
        );
        let off = gas_toggle(low_gas(0, 1, 1), false, 0).unwrap();
        assert_eq!(words(&off), [vec![9999, 2, 1, 0], vec![9999, 4, 0, 0], vec![9999, 2, 64, 0]]);
        let high = GasBinding { selector: 3, pressure: None, ..low_gas(0, 5, 1) };
        assert_eq!(gas_toggle(high, true, 0).unwrap().len(), 1);
        assert_eq!(gas_toggle(low_gas(0, 1, 1), true, -1), Err(SequenceError::NegativeValue));
        assert_eq!(
            gas_toggle(GasBinding { pressure: None, ..low_gas(0, 1, 1) }, true, 0),
            Err(SequenceError::PressureBinding)
        );
        assert_eq!(gas_toggle(low_gas(6, 1, 1), true, 0), Err(SequenceError::GasSelector));
    }

    /// Switching gas while on brings the new valve up with its pressure
    /// before the old one comes down; nothing happens when gas is off or the
    /// selector is unchanged; a high target without an analog rule is refused.
    #[test]
    fn gas_switch_brings_the_new_gas_up_before_the_old_down() {
        let steps = gas_switch(low_gas(0, 1, 1), low_gas(1, 2, 2), true, 2500).unwrap();
        assert_eq!(
            words(&steps),
            [
                vec![9999, 2, 2, 2],
                vec![9999, 2, 64, 64],
                vec![120],
                vec![9999, 4, 1, 2500],
                vec![9999, 2, 1, 0],
                vec![9999, 4, 0, 0],
                vec![9999, 2, 64, 0]
            ]
        );
        assert!(gas_switch(low_gas(0, 1, 1), low_gas(1, 2, 2), false, 0).unwrap().is_empty());
        assert!(gas_switch(low_gas(0, 1, 1), low_gas(0, 1, 1), true, 0).unwrap().is_empty());
        let unresolved = GasBinding {
            selector: 3,
            high_selector_analog: HighSelectorAnalog::Unresolved,
            ..low_gas(0, 5, 0)
        };
        assert_eq!(
            gas_switch(low_gas(0, 1, 1), unresolved, true, 0),
            Err(SequenceError::PressureBinding)
        );
    }

    /// The two reproduced shutter branches: mode 1 zeroes the analog before
    /// closing, mode 0 type 3 writes its converted on value and may preserve
    /// the analog on close; the unresolved branch is refused.
    #[test]
    fn shutter_branches_match_the_vendor() {
        let mode1 = ShutterBinding::Mode1 { output_enabled: true, port: 2, analog_channel: 1 };
        assert_eq!(words(&shutter(mode1, true).unwrap()), [vec![9999, 2, 2, 2]]);
        assert_eq!(
            words(&shutter(mode1, false).unwrap()),
            [vec![9999, 4, 0, 0], vec![9999, 2, 2, 0]]
        );
        let disabled = ShutterBinding::Mode1 { output_enabled: false, port: 2, analog_channel: 1 };
        assert!(shutter(disabled, true).unwrap().is_empty());
        let type3 = ShutterBinding::Mode0Type3 {
            port: 2,
            analog_channel: 1,
            converted_on: 12_000,
            preserve_analog_on_close: false,
        };
        assert_eq!(
            words(&shutter(type3, true).unwrap()),
            [vec![9999, 2, 2, 2], vec![9999, 4, 0, 12_000]]
        );
        assert_eq!(
            words(&shutter(type3, false).unwrap()),
            [vec![9999, 2, 2, 0], vec![9999, 4, 0, 0]]
        );
        let keep = ShutterBinding::Mode0Type3 {
            port: 2,
            analog_channel: 1,
            converted_on: 12_000,
            preserve_analog_on_close: true,
        };
        assert_eq!(shutter(keep, false).unwrap().len(), 1);
        assert_eq!(shutter(ShutterBinding::Unresolved, false), Err(SequenceError::Shutter));
    }

    /// CO2 laser enable writes the analog power, the on words on the channel
    /// selected by control type, then the laser port; disable reverses the
    /// port and analog and sends the off pair unless suppressed.
    #[test]
    fn co2_laser_enable_and_disable_match_the_vendor() {
        let laser = Co2Laser {
            control_type: 2,
            shutter_flag: true,
            analog_channel: 1,
            analog_on: 8000,
            laser_port: 9,
            on_first: 5,
            on_second: 6,
            off_value: 100,
            suppress_off: false,
        };
        assert_eq!(
            words(&co2_laser(laser, true).unwrap()),
            [vec![9999, 4, 0, 8000], vec![9999, 17, 5, 6, 1], vec![9999, 2, 256, 256]]
        );
        assert_eq!(
            words(&co2_laser(laser, false).unwrap()),
            [
                vec![9999, 2, 256, 0],
                vec![9999, 4, 0, 0],
                vec![9999, 3, 100, 0, 0],
                vec![9999, 17, 100, 0, 0]
            ]
        );
        assert!(co2_laser(Co2Laser { shutter_flag: false, ..laser }, true).unwrap().is_empty());
        assert_eq!(co2_laser(Co2Laser { suppress_off: true, ..laser }, false).unwrap().len(), 2);
    }

    fn shutdown_config() -> Shutdown {
        Shutdown {
            mode: LaserMode::Co2,
            head_enabled: true,
            co2_skip_head_cancel: false,
            extended_outputs: false,
            co2_analog_channel: 1,
            gas_channels: [0, 2, 0],
            point_laser_frequency: 0,
            rapid_deceleration: 2000,
            ports: vec![9, 0, 12],
            raise: None,
        }
    }

    /// A shutdown that stops the axes decelerates, cancels the head, stops
    /// the FIFO, sends the laser off pair, zeroes each analog channel once,
    /// and masks the outputs off; the abort adds the FIFO clear.
    #[test]
    fn shutdown_and_abort_follow_the_host_policy() {
        let mut config = shutdown_config();
        config.extended_outputs = true;
        let writes = shutdown(&config, true, false, &[]).unwrap();
        let words: Vec<(u32, Vec<u32>)> =
            writes.iter().map(|w| (w.address, w.words.clone())).collect();
        assert_eq!(
            words,
            [
                (101, vec![1, 31, 2, 2000, 200_000]),
                (101, vec![101]),
                (103, vec![3]),
                (101, vec![9999, 3, 0, 0, 0]),
                (101, vec![9999, 17, 0, 0, 0]),
                (101, vec![9999, 4, 1, 0]),
                (101, vec![9999, 4, 0, 0]),
                (101, vec![9999, 2, 256, 0]),
                (101, vec![9999, 13, 2, 0]),
            ]
        );
        assert_eq!(abort(&config, false).unwrap().last(), Some(&requests::fifo_clear()));
        let without_axes = shutdown(&config, false, false, &[]).unwrap();
        assert_eq!(without_axes.len(), 6);
    }

    /// CO2 with the skip flag omits the head cancel unless the head is
    /// active, and an extended port without the extended bank is refused.
    #[test]
    fn shutdown_head_cancel_and_port_rules() {
        let mut config = shutdown_config();
        config.co2_skip_head_cancel = true;
        config.ports = vec![9];
        assert!(!shutdown(&config, true, false, &[]).unwrap().contains(&requests::head_cancel()));
        assert!(shutdown(&config, true, true, &[]).unwrap().contains(&requests::head_cancel()));
        config.mode = LaserMode::Fiber;
        assert!(shutdown(&config, true, false, &[]).unwrap().contains(&requests::head_cancel()));
        config.ports = vec![12];
        assert_eq!(shutdown(&config, true, false, &[]), Err(SequenceError::ShutdownPort(12)));
    }

    /// The relief writes per id reproduce the dialog dispatch and the cached
    /// aggregate tail, including the duplicated common reset.
    #[test]
    fn relief_writes_match_the_dialog_dispatch_and_tail() {
        let common = requests::alarm_clear();
        let cases: [(Option<u32>, u32, u32, Vec<Write>); 10] = [
            (Some(1), 0, 0, vec![common.clone()]),
            (Some(13), 0, 0, vec![requests::fifo_clear(), common.clone()]),
            (Some(8025), 1 << 25, 0, vec![requests::bus_reset(), common.clone(), common.clone()]),
            (Some(9000), 0, 1, vec![common.clone(), common.clone()]),
            (Some(1000), 0, 0, vec![common.clone()]),
            (Some(150), 0, 0, vec![common.clone()]),
            (Some(100), 0, 0, vec![]),
            (Some(100), 1 << 24, 0, vec![common.clone()]),
            (None, 0, 1 << 31, vec![common.clone()]),
            (None, 0, 0, vec![]),
        ];
        for (id, group_1, group_2, expected) in cases {
            assert_eq!(relief(id, group_1, group_2, None).unwrap().writes, expected, "id {id:?}");
        }
        assert!(!relief(Some(1), 0, 0, None).unwrap().clears_host_rows);
        assert!(relief(Some(13), 0, 0, None).unwrap().clears_host_rows);
        for id in [0, 9, 44, 45, 58, 61, 63, 64, 79, 80, 82, 99, 102, 8105] {
            assert!(relief(Some(id), 0, 0, None).is_err(), "id {id}");
        }
    }

    /// Dual-drive relief resets, restores both limit pairs, then clears,
    /// plus the tail; bad limits are refused.
    #[test]
    fn dual_drive_relief_restores_limits_before_the_clear() {
        let dual = DualDrive { xy_limits: [(-500, 1_500_000), (-500, 3_000_000)] };
        let plan = relief(Some(8105), 1, 0, Some(dual)).unwrap();
        let words: Vec<(u32, Vec<u32>)> =
            plan.writes.iter().map(|w| (w.address, w.words.clone())).collect();
        assert_eq!(
            words,
            [
                (101, vec![9999, 8, 3]),
                (50_202, vec![(-500i32).cast_unsigned(), 1_500_000]),
                (50_242, vec![(-500i32).cast_unsigned(), 3_000_000]),
                (101, vec![9999, 5, 0, 0]),
                (101, vec![9999, 5, 0, 0]),
            ]
        );
        assert!(
            relief(Some(8105), 0, 0, Some(DualDrive { xy_limits: [(1, 2), (-1, 1)] })).is_err()
        );
    }

    /// A mode switch writes cleanup, the head mode when a head exists, the
    /// enable output, then both limit pairs; plans that could enable
    /// something in cleanup or carry bad limits are refused.
    #[test]
    fn mode_switch_plan_follows_the_vendor_order() {
        let switch = ModeSwitch {
            mode: LaserMode::Co2,
            head_enabled: true,
            cleanup: vec![
                requests::digital_output(9, false).unwrap(),
                requests::analog_output(1, 0).unwrap(),
            ],
            enable: Some(requests::digital_output(3, true).unwrap()),
            xy_limits: [(-500, 1_000_000), (0, 2_000_000)],
        };
        let words: Vec<(u32, Vec<u32>)> =
            switch.plan().unwrap().iter().map(|w| (w.address, w.words.clone())).collect();
        assert_eq!(
            words,
            [
                (101, vec![9999, 2, 256, 0]),
                (101, vec![9999, 4, 0, 0]),
                (101, vec![118, 5, 1]),
                (101, vec![9999, 2, 4, 4]),
                (50_202, vec![(-500i32).cast_unsigned(), 1_000_000]),
                (50_242, vec![0, 2_000_000]),
            ]
        );
        let no_head = ModeSwitch { head_enabled: false, ..switch.clone() };
        assert_eq!(no_head.plan().unwrap().len(), 5);
        let enabling = ModeSwitch {
            cleanup: vec![requests::digital_output(9, true).unwrap()],
            ..switch.clone()
        };
        assert!(enabling.plan().is_err());
        let bad_limits = ModeSwitch { xy_limits: [(1, 0), (0, 1)], ..switch.clone() };
        assert!(bad_limits.plan().is_err());
        assert!(ModeSwitch { cleanup: vec![], ..switch }.plan().is_err());
    }

    /// The stop words: clamp order and truncation, and refusals for values
    /// that do not fit or truncate a speed to zero.
    #[test]
    fn stop_words_follow_the_conversion_chain() {
        let kinematics = StopKinematics {
            jog_stop_deceleration_factor: 1,
            fast_move_acceleration: 5000.0,
            maximum_acceleration: 20_000.0,
            z_up_speed: 100.0,
            z_dock_height: 20.0,
            z_safe_height: 15.0,
        };
        assert_eq!(
            kinematics.words().unwrap(),
            StopWords {
                rapid_deceleration: 5000,
                retract_speed: 1000,
                short_retract_height: 20_000,
                long_retract_height: 15_000
            }
        );
        let slow = StopKinematics { fast_move_acceleration: 10.0, ..kinematics };
        assert_eq!(slow.words().unwrap().rapid_deceleration, 2000);
        let capped = StopKinematics {
            fast_move_acceleration: 10.0,
            maximum_acceleration: 1500.0,
            ..kinematics
        };
        assert_eq!(capped.words().unwrap().rapid_deceleration, 1500);
        let fractional = StopKinematics { z_up_speed: 12.39, z_safe_height: 1.2349, ..kinematics };
        let words = fractional.words().unwrap();
        assert_eq!((words.retract_speed, words.long_retract_height), (123, 1234));
        for z_up_speed in [f64::NAN, f64::INFINITY, -1.0, 214_748_364.8, 0.01] {
            assert!(StopKinematics { z_up_speed, ..kinematics }.words().is_err(), "{z_up_speed}");
        }
        let overflow = StopKinematics {
            fast_move_acceleration: f64::MAX,
            jog_stop_deceleration_factor: 2,
            ..kinematics
        };
        assert!(overflow.words().is_err());
    }

    /// Pressure to voltage: linear over full scale, piecewise linear along a
    /// curve with extrapolation past its ends, duplicate points, and the
    /// refusals for bad input; the two analog scalings.
    #[test]
    fn pressure_and_analog_conversions_match_the_vendor() {
        assert_eq!(pressure_voltage(3.0, 6.0, &[]), Ok(5.0));
        let curve = [
            PressurePoint { voltage: 1.0, pressure: 2.0 },
            PressurePoint { voltage: 5.0, pressure: 10.0 },
        ];
        assert_eq!(pressure_voltage(1.0, 0.0, &curve), Ok(0.5));
        assert_eq!(pressure_voltage(6.0, 0.0, &curve), Ok(3.0));
        assert_eq!(pressure_voltage(12.0, 0.0, &curve), Ok(6.0));
        let duplicate = [
            PressurePoint { voltage: 1.0, pressure: 2.0 },
            PressurePoint { voltage: 6.0, pressure: 2.0 },
        ];
        assert_eq!(pressure_voltage(5.0, 0.0, &duplicate), Ok(6.0));
        assert_eq!(pressure_voltage(f64::NAN, 1.0, &[]), Err(SequenceError::Pressure));
        assert_eq!(pressure_voltage(1.0, 0.0, &[]), Err(SequenceError::PressureCurve));
        assert_eq!(
            pressure_voltage(1.0, 1.0, &[curve[1], curve[0]]),
            Err(SequenceError::PressureCurve)
        );
        let scaled = [
            (power_scaled(0, 10.0), 1000.0),
            (power_scaled(1, 10.0), 500.0),
            (power_scaled(2, 10.0), 400.0),
            (power_scaled(9, 10.0), 0.0),
            (pressure_scaled(2.5), 2500.0),
        ];
        for (value, want) in scaled {
            assert!((value - want).abs() < 1e-9, "{value} is not {want}");
        }
    }
}
