// SPDX-License-Identifier: GPL-3.0-or-later

//! What the controller task needs from the machine files beyond the recipe:
//! the stop policy, the Go Origin outputs, the mode switch, the host input
//! rules, jog kinematics and the manual outputs.
//!
//! Every value is read the way the vendor host reads it, with its ranges
//! and its refusals. Branches the vendor host takes on other hardware, a
//! persistent CO2 enable output, rotated or dual-drive limits, are refused
//! before anything is sent.

use crate::attrs::Group;
use crate::document::Bundle;
use crate::recipe::hardware;
use crate::{Error, Result};
use openlaser_compiler::settings::Hardware;
use openlaser_core::LaserMode;
use openlaser_protocol::requests::{self, Write};
use openlaser_protocol::sequences::{
    self, DualDrive, GasBinding, HighSelectorAnalog, ModeSwitch, PressureOutput, Shutdown, Step,
    StopKinematics, gas_toggle,
};

/// The hardware model whose mode switch route the binding reproduces.
const DUAL_SOURCE_MODEL: u32 = 224;
/// The soft limit margin the controller loads before its own minimum and
/// maximum, in millimetres.
const LIMIT_MARGIN_MM: f64 = 0.5;

/// The outputs the vendor's Go Origin sequence drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Home {
    /// `DO.ZFGoOriginDone`: switched off when the head search starts; zero
    /// for none.
    pub z_origin_done_port: u8,
    /// `DO.ManuSignal`: held on while the XY search runs; zero for none.
    pub manual_signal_port: u8,
}

/// A host input rule: a door or other custom input, the cooling water and
/// laser source warnings, or gas feedback qualified by its valve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputRule {
    /// The vendor's id: 1000 to 1015 for custom inputs, 150 to 155 for gas
    /// feedback, 56 for cooling water, 60 for the laser source.
    pub id: u32,
    /// The label shown for the row.
    pub label: String,
    /// The digital input, 1 to 24.
    pub input: u8,
    /// Whether the input trips when low.
    pub active_low: bool,
    /// Whether the rule is armed only while a job runs.
    pub run_only: bool,
    /// Whether the row stays until relieved, even after the input recovers.
    pub latch: bool,
    /// For gas feedback: the valve output that qualifies the input.
    pub gas_valve: Option<u8>,
}

/// The vendor's jog and positioning kinematics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jog {
    /// Slow and fast jog speeds in millimetres per second, capped by the
    /// machine's maximum.
    pub speed: [f64; 2],
    /// Slow and fast jog acceleration words.
    pub acceleration: [u32; 2],
    /// Slow and fast acceleration words for a positioning move.
    pub position_acceleration: [u32; 2],
    /// Each axis's travel in millimetres, lower then upper.
    pub extent: [[f64; 2]; 2],
    /// Whether jogs are clamped to the travel.
    pub soft_limit: bool,
    /// The rapid-stop deceleration word.
    pub deceleration: u32,
}

impl Jog {
    /// The speed word for `mm_s`.
    pub fn speed_word(&self, mm_s: f64, scale: i32) -> Result<u32> {
        let raw = (mm_s * f64::from(scale)).trunc();
        if !mm_s.is_finite() || mm_s <= 0. || raw < 1. || raw > f64::from(i32::MAX) {
            return Err(Error::Unsupported("the jog speed cannot be represented".into()));
        }
        Ok(crate::whole(raw).unwrap_or(1))
    }

    /// The travel of a jog on `axis` in controller units: a held jog runs to
    /// the travel limit, a step by `delta_mm`; both are clamped to the
    /// travel when the soft limit is enabled.
    pub fn travel(
        &self,
        axis: usize,
        current_mm: f64,
        delta_mm: f64,
        held: bool,
        scale: i32,
    ) -> Result<i32> {
        let extent = self.extent.get(axis).ok_or_else(|| Error::Unsupported("jog axis".into()))?;
        let mut distance =
            if held { (extent[1] - extent[0]) * delta_mm.signum() } else { delta_mm };
        if self.soft_limit {
            distance = if distance > 0. {
                distance.min(extent[1] - current_mm)
            } else {
                distance.max(extent[0] - current_mm)
            };
        }
        if distance * delta_mm <= 0. {
            let name = ["X", "Y"].get(axis).copied().unwrap_or("the axis");
            return Err(Error::Unsupported(format!("{name} is at its travel limit")));
        }
        coordinate(distance, scale)
    }
}

/// Millimetres as a controller coordinate.
pub fn coordinate(mm: f64, scale: i32) -> Result<i32> {
    let raw = (mm * f64::from(scale)).trunc();
    if !raw.is_finite() || scale <= 0 || raw < f64::from(i32::MIN) || raw > f64::from(i32::MAX) {
        return Err(Error::Unsupported("the coordinate cannot be represented".into()));
    }
    #[allow(clippy::cast_possible_truncation, reason = "range checked")]
    Ok(raw as i32)
}

/// A manual output the operator can hold on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manual {
    /// The steps that switch it on.
    pub on: Vec<Step>,
    /// The writes that switch it off, in order.
    pub off: Vec<Write>,
    /// How long it stays on without a renewal, in milliseconds.
    pub lease_ms: u64,
}

/// The manual output bindings for one laser.
#[derive(Clone, Debug, PartialEq)]
pub struct Outputs {
    /// The red pointer output, zero for none.
    pub pointer_port: u8,
    /// The shutter output, zero for none.
    pub shutter_port: u8,
    /// The analog channel and converted value the shutter opens with under
    /// analog laser control.
    pub shutter_analog: Option<(u8, u32)>,
    /// The settle delay between a gas valve and its pressure output.
    pub proportional_delay_ms: u32,
    /// Slow and fast head jog speeds in tenths, when a head is configured
    /// with the native jog.
    pub head_jog_tenths: Option<[u32; 2]>,
    /// The process hardware the gas plans are built from.
    pub hardware: Hardware,
}

/// Everything bound for one laser.
#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    /// The laser these bindings are for.
    pub mode: LaserMode,
    /// Whether a head controller is configured.
    pub head_enabled: bool,
    /// The output-off policy and the rapid-stop deceleration.
    pub shutdown: Shutdown,
    /// The Go Origin outputs.
    pub home: Home,
    /// The mode switch plan.
    pub mode_switch: ModeSwitch,
    /// The XY limits restored by a dual-drive reset.
    pub dual_drive: DualDrive,
    /// The host input rules.
    pub rules: Vec<InputRule>,
    /// Jog kinematics.
    pub jog: Jog,
    /// The lifting table, when the machine selects that branch.
    pub table: Option<Table>,
    /// The manual outputs.
    pub outputs: Outputs,
    /// Whether a resumed contour pierces again (`MC.afterContinueDill`).
    pub resume_pierce: bool,
    /// The CO2 laser control type, which selects the program's FIFO mode.
    pub co2_control_type: u8,
    /// The port the controller must report at register 50108 before CO2
    /// output is enabled, when PWM synchronisation is on.
    pub co2_pwm_sync_port: Option<u8>,
    /// The frame kinematics: the bounds traced with the laser off.
    pub frame: openlaser_compiler::frame::Settings,
}

/// Binds everything for `mode` under the controller's coordinate `scale`.
pub fn bind(bundle: &Bundle, mode: LaserMode, scale: i32) -> Result<Bindings> {
    if scale <= 0 || scale > 1_000_000 {
        return Err(Error::Unsupported("the controller scale must be 1 to 1 000 000".into()));
    }
    validate_xy_route(bundle)?;
    let head_enabled = Group::read(bundle, "ZFParam", "ZF")?.uint("ZFType", 10)? != 0;
    let (shutdown, deceleration) = shutdown(bundle, mode, head_enabled)?;
    let mode_switch = mode_switch(bundle, mode, head_enabled, scale)?;
    let dual_drive = DualDrive { xy_limits: mode_switch.xy_limits };
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    let laser = Group::read(bundle, "LaserParam", "LGP")?;
    let co2_pwm_sync_port = match mp.text("CO2PWMOutputSync").unwrap_or("1") {
        "0" => None,
        "1" => laser.port("CO2DOLaser", 26)?,
        _ => return Err(mp.invalid("CO2PWMOutputSync", "must be 0 or 1")),
    };
    Ok(Bindings {
        mode,
        head_enabled,
        shutdown,
        home: home(bundle)?,
        mode_switch,
        dual_drive,
        rules: rules(bundle, mode)?,
        jog: jog(bundle, deceleration)?,
        table: table(bundle)?,
        outputs: outputs(bundle, mode, head_enabled)?,
        resume_pierce: Group::read(bundle, "ManuParam", "MC")?.enabled("afterContinueDill")?,
        co2_control_type: laser.byte("CO2LaserControlType", 3)?,
        co2_pwm_sync_port,
        frame: frame(bundle)?,
    })
}

/// The optional W axis on the lifting-table branch, never the head's Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Table {
    /// Positive travel limit; the negative limit is zero.
    pub maximum_mm: f64,
    /// Native HPA3 acceleration word.
    pub acceleration: u32,
}

fn table(bundle: &Bundle) -> Result<Option<Table>> {
    let Ok(lpf) = Group::read(bundle, "LaserParam", "LPF") else { return Ok(None) };
    if lpf.text("LiftingPlatformType").unwrap_or("0") == "0" {
        return Ok(None);
    }
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    if mp.uint("ExchangePlatformType", 10)? != 0 || mp.uint("RollSheetType", 10)? != 0 {
        return Ok(None);
    }
    Ok(Some(Table {
        maximum_mm: Group::read(bundle, "MachineAxisConfig_4", "MAC_4")?.range(
            "SoftLimitMaxLen",
            0.001,
            100_000.,
        )?,
        acceleration: Group::read(bundle, "HomeParam", "HPA3")?.uint("Acc", 10_000_000)?,
    }))
}

/// Binds the admitted manual pulse branch; the compiler builds its program.
pub fn pulse(
    bundle: &Bundle,
    mode: LaserMode,
    power: u8,
) -> Result<openlaser_compiler::pulse::Settings> {
    use openlaser_protocol::requests::LaserChannel;
    if !(1..=100).contains(&power) {
        return Err(Error::Unsupported("pulse power must be 1–100 percent".into()));
    }
    let co2 = mode == LaserMode::Co2;
    let key = |name: &str| if co2 { format!("CO2{name}") } else { name.to_owned() };
    let laser = Group::read(bundle, "LaserParam", "LGP")?;
    let control = laser.uint(&key("LaserControlType"), 3)?;
    if control == 0 || (co2 && control != 2) {
        return Err(Error::Unsupported(
            "pulse needs an enabled laser; CO2 requires secondary PWM control type 2".into(),
        ));
    }
    let extended = Group::read(bundle, "ManuParam", "EC")?.uint("ECIOType", 100)? != 0;
    let gate = port(laser, &key("DOLaserGate"), extended)?;
    let output = port(laser, &key("DOLaser"), extended)?;
    if gate == 0 && output == 0 {
        return Err(Error::Unsupported("no laser or shutter output is assigned".into()));
    }
    let manual = Group::read(bundle, "ManuParam", "LC")?;
    let frequency = u16::try_from(manual.uint("PtLaserFreq", u32::from(u16::MAX))?)
        .map_err(|_| Error::Unsupported("queued pulse frequency".into()))?;
    if frequency == 0 {
        return Err(Error::Unsupported("pulse frequency must be positive".into()));
    }
    let channel = if co2 { LaserChannel::Secondary } else { LaserChannel::Primary };
    let analog = if control == 3 || co2 {
        let channel = laser.byte(&key("LaserDAPort"), 2)?;
        if channel == 0 {
            return Err(Error::Unsupported("assign the pulse analog channel".into()));
        }
        let peak = manual.range("PtLaserPeakCurrent", 0., 100.)?;
        let value = peak
            * hardware(bundle, mode)?.peak_factor
            * if co2 { f64::from(power) / 100. } else { 1. };
        let raw = coordinate(value, 1)?;
        Some((channel, requests::power_word(raw)))
    } else {
        None
    };
    Ok(openlaser_compiler::pulse::Settings { channel, frequency, power, analog, gate, output })
}

/// The frame kinematics: the bound speed and rapid acceleration under the
/// machine's ceilings, the jerk from the acceleration time, and the
/// interpolation cadence. The vendor's vertical correction is not bound.
fn frame(bundle: &Bundle) -> Result<openlaser_compiler::frame::Settings> {
    let mc = Group::read(bundle, "ManuParam", "MC")?;
    let fcp = Group::read(bundle, "FCParam", "FCP")?;
    if Group::read(bundle, "ManuParam", "MP")?.enabled("EnableVerCorrect")? {
        return Err(Error::Unsupported("the frame's vertical correction is not supported".into()));
    }
    let machine = crate::recipe::machine(bundle)?;
    let acceleration = mc.range("XFastMoveAcc", 5., 1e12)?.min(fcp.range("MaxAcc", 5., 1e12)?);
    Ok(openlaser_compiler::frame::Settings {
        speed: mc.range("BoundSpeed", 0.001, 10_000.)?.min(fcp.range("MaxSpeed", 0.001, 10_000.)?),
        acceleration,
        jerk: 2. * acceleration / (mc.range("AccTime", 1e-9, 1e9)? * 0.001),
        cadence_ms: f64::from(machine.interpolation_cycle_us) * 0.001,
        counts_per_mm: machine.counts_per_mm,
    })
}

/// The axis group of X or Y.
fn axis_group(bundle: &Bundle, axis: usize) -> Result<Group<'_>> {
    let (group, tag): (&'static str, &'static str) = match axis {
        0 => ("MachineAxisConfig_0", "MAC"),
        _ => ("MachineAxisConfig_1", "MAC_1"),
    };
    Group::read(bundle, group, tag)
}

/// The controller wire path the bindings assume: X on axis 0 and Y on 1.
fn validate_xy_route(bundle: &Bundle) -> Result<()> {
    for (tag, expected) in [("A0", 0), ("A1", 1)] {
        if Group::read(bundle, "AxisParam", tag)?.uint("AxisIndex", 31)? != expected {
            return Err(Error::Unsupported("the bindings need X on axis 0 and Y on axis 1".into()));
        }
    }
    Ok(())
}

/// A port that must lie in the standard bank unless the extended bank exists.
fn port(group: Group<'_>, key: &str, extended: bool) -> Result<u8> {
    let port = group.byte(key, 26)?;
    if port > 10 && !extended {
        return Err(group.invalid(key, "is in the extended bank, which is not configured"));
    }
    Ok(port)
}

// Stop ----------------------------------------------------------------------

/// The stop policy and the rapid-stop deceleration word.
fn shutdown(bundle: &Bundle, mode: LaserMode, head_enabled: bool) -> Result<(Shutdown, u32)> {
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    let words = stop_words(bundle, mp)?;
    let ports = shutdown_ports(bundle, mp)?;
    let gas = Group::read(bundle, "GasParam", "MGP")?;
    let co2_skip_head_cancel = mode == LaserMode::Co2 && mp.enabled("CO2DisabledZAxis")?;
    let shutdown = Shutdown {
        mode,
        head_enabled,
        co2_skip_head_cancel,
        extended_outputs: Group::read(bundle, "ManuParam", "EC")?.uint("ECIOType", u32::MAX)? != 0,
        co2_analog_channel: Group::read(bundle, "LaserParam", "LGP")?.byte("CO2LaserDAPort", 3)?,
        gas_channels: gas.bytes(["RatioAir", "RatioO2", "RatioH2"], 3)?,
        point_laser_frequency: Group::read(bundle, "ManuParam", "LC")?
            .uint("PtLaserFreq", u32::MAX)?,
        rapid_deceleration: words.rapid_deceleration,
        ports,
        raise: (head_enabled && !co2_skip_head_cancel).then_some(sequences::Raise {
            speed_tenths: words.retract_speed,
            height_thousandths: words.long_retract_height,
        }),
    };
    Ok((shutdown, words.rapid_deceleration))
}

fn stop_words(bundle: &Bundle, mp: Group<'_>) -> Result<sequences::StopWords> {
    let mc = Group::read(bundle, "ManuParam", "MC")?;
    let zf = Group::read(bundle, "ZFParam", "ZF")?;
    StopKinematics {
        jog_stop_deceleration_factor: i32::try_from(mp.uint("JogStopDccFactor", u32::MAX)?)
            .map_err(|_| mp.invalid("JogStopDccFactor", "exceeds the controller's range"))?,
        fast_move_acceleration: mc.number("XFastMoveAcc")?,
        maximum_acceleration: Group::read(bundle, "FCParam", "FCP")?.number("MaxAcc")?,
        z_up_speed: zf.number("ZFUpSpeed")?,
        z_dock_height: zf.number("ZFDockHeight")?,
        z_safe_height: mp.number("ZFSafeHeight")?,
    }
    .words()
    .map_err(|error| Error::Unsupported(error.to_string()))
}

/// The ordering is part of the native stop policy: mask, optional PLC pair,
/// then gates and proportional switches.
fn shutdown_ports(bundle: &Bundle, mp: Group<'_>) -> Result<Vec<u8>> {
    let laser = Group::read(bundle, "LaserParam", "LGP")?;
    let gas = Group::read(bundle, "GasParam", "MGP")?;
    let outputs = Group::read(bundle, "DOParam", "DO")?;
    let inputs = Group::read(bundle, "DIParam", "DO")?;
    let mut ports = read_port_groups(&[
        (gas, &["HighN2", "HighO2", "HighAir", "LowN2", "LowO2", "LowAir"]),
        (laser, &["DOLaser", "CO2DOLaser"]),
        (
            outputs,
            &[
                "BackRollMoveEnable",
                "RollSheetSlowMove",
                "RollSheetFastMove",
                "RollSheetMoveEnable",
            ],
        ),
        (inputs, &["EnableBackRollSheet"]),
        (outputs, &["ManuDoneOutput", "RollSheetOutput", "ManuOutput"]),
        (gas, &["CoolGas"]),
        (
            outputs,
            &[
                "PlatBMoveEnable",
                "PlatAMoveEnable",
                "PlatBFastMove",
                "PlatBSlowMove",
                "PlatAFastMove",
                "PlatASlowMove",
            ],
        ),
    ])?;
    if mp.enabled("EnablePLCProcess1")? {
        ports.extend(outputs.bytes(["PLCProcess1B", "PLCProcess1A"], 32)?);
    }
    ports.extend(read_port_groups(&[
        (laser, &["DOLaserGate", "CO2DOLaserGate"]),
        (gas, &["RatioAirSwitch", "RatioO2Switch", "RatioN2Switch"]),
    ])?);
    Ok(ports)
}

fn read_port_groups(groups: &[(Group<'_>, &[&str])]) -> Result<Vec<u8>> {
    let mut ports = Vec::new();
    for (group, keys) in groups {
        for key in *keys {
            ports.push(group.byte(key, 32)?);
        }
    }
    Ok(ports)
}

// Home ----------------------------------------------------------------------

fn home(bundle: &Bundle) -> Result<Home> {
    let outputs = Group::read(bundle, "DOParam", "DO")?;
    Ok(Home {
        z_origin_done_port: outputs.byte("ZFGoOriginDone", 10)?,
        manual_signal_port: outputs.byte("ManuSignal", 10)?,
    })
}

// Mode switch ---------------------------------------------------------------

/// The supported branch: the dual-source model with unshifted XY limits,
/// fiber under analog control and CO2 under secondary PWM.
fn mode_switch(
    bundle: &Bundle,
    mode: LaserMode,
    head_enabled: bool,
    scale: i32,
) -> Result<ModeSwitch> {
    validate_switch_model(bundle)?;
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    let laser = Group::read(bundle, "LaserParam", "LGP")?;
    validate_switch_control(mp, laser)?;
    let extended = Group::read(bundle, "ManuParam", "EC")?.uint("ECIOType", 100)? != 0;
    let frequency = Group::read(bundle, "ManuParam", "LC")?.uint("PtLaserFreq", 100_000)?;
    let (mut cleanup, laser_ports) = switch_laser_cleanup(laser, extended, frequency)?;
    switch_enable_cleanup(laser, mode, extended, &laser_ports, &mut cleanup)?;
    let xy_limits = switch_limits(bundle, scale)?;
    Ok(ModeSwitch { mode, head_enabled, cleanup, enable: None, xy_limits })
}

fn switch_error(reason: &str) -> Error {
    Error::Unsupported(format!("mode switch: {reason}"))
}

fn validate_switch_model(bundle: &Bundle) -> Result<()> {
    if Group::read(bundle, "SoftParam", "SP")?.uint("m_iHardwareModel", 255)? != DUAL_SOURCE_MODEL {
        return Err(switch_error("needs the dual-source hardware model"));
    }
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    if mp.uint("EnableVerCorrect", 1)? != 0
        || Group::read(bundle, "MachineAxisConfig", "MAC")?.uint("DoubleDevice", 1)? != 0
    {
        return Err(switch_error("rotated or dual-drive limits need their own binding"));
    }
    Ok(())
}

fn validate_switch_control(mp: Group<'_>, laser: Group<'_>) -> Result<()> {
    if laser.uint("LaserControlType", 4)? != 3 || laser.uint("CO2LaserControlType", 4)? != 2 {
        return Err(switch_error("supports fiber analog control with CO2 secondary PWM only"));
    }
    if mp.uint("LaserDAKeepOutput", 1)? != 0 {
        return Err(switch_error("the held analog branch is not commissioned"));
    }
    if mp.text("CO2EnableSecondSoftLimit").is_some_and(|value| value != "0") {
        return Err(switch_error("shifted CO2 limits need coordinate validation"));
    }
    Ok(())
}

fn switch_laser_cleanup(
    laser: Group<'_>,
    extended: bool,
    frequency: u32,
) -> Result<(Vec<Write>, Vec<u8>)> {
    let mut cleanup = sequences::laser_off_pair(frequency).to_vec();
    let mut laser_ports = Vec::new();
    for prefix in ["", "CO2"] {
        for key in ["DOLaser", "DOLaserGate", "DORedLight"] {
            let port = port(laser, &format!("{prefix}{key}"), extended)?;
            if port != 0 {
                laser_ports.push(port);
                cleanup.push(requests::digital_output(port, false)?);
            }
        }
        let channel = laser.byte(&format!("{prefix}LaserDAPort"), 2)?;
        if channel != 0 {
            cleanup.push(requests::analog_output(channel, 0)?);
        }
    }
    Ok((cleanup, laser_ports))
}

fn switch_enable_cleanup(
    laser: Group<'_>,
    mode: LaserMode,
    extended: bool,
    laser_ports: &[u8],
    cleanup: &mut Vec<Write>,
) -> Result<()> {
    let enable_port = port(laser, "doCO2EnableOutput", extended)?;
    if enable_port == 0 {
        return Ok(());
    }
    // A persistent selector has no owner across home, jog and jobs.
    if mode == LaserMode::Co2 {
        return Err(switch_error("a persistent CO2 enable output is not supported"));
    }
    if laser_ports.contains(&enable_port) {
        return Err(switch_error("the enable output aliases a laser output"));
    }
    cleanup.push(requests::digital_output(enable_port, false)?);
    Ok(())
}

fn switch_limits(bundle: &Bundle, scale: i32) -> Result<[(i32, i32); 2]> {
    let mut limits = [(0, 0); 2];
    for (axis, limit) in limits.iter_mut().enumerate() {
        let group = axis_group(bundle, axis)?;
        let length = group.number("SoftLimitMaxLen")? * f64::from(scale);
        if length <= 0. || length > f64::from(i32::MAX) {
            return Err(switch_error("invalid travel limit"));
        }
        let signed = if group.uint("GoOriginalDirection", 1)? == 1 { -length } else { length };
        let margin = LIMIT_MARGIN_MM * f64::from(scale);
        *limit = (coordinate(signed.min(-margin), 1)?, coordinate(signed.max(margin), 1)?);
    }
    Ok(limits)
}

// Input rules ---------------------------------------------------------------

fn polarity(group: Group<'_>, key: &str, text: &str) -> Result<bool> {
    match text.trim() {
        "0" => Ok(false),
        "1" | "-1" => Ok(true),
        _ => Err(group.invalid(key, format!("polarity {text:?} is not 0, 1 or -1"))),
    }
}

fn input(group: Group<'_>, key: &str, text: &str) -> Result<u8> {
    text.trim()
        .parse::<u8>()
        .ok()
        .filter(|value| *value <= 24)
        .ok_or_else(|| group.invalid(key, format!("input {text:?} is not 0 to 24")))
}

fn rules(bundle: &Bundle, mode: LaserMode) -> Result<Vec<InputRule>> {
    // Optional device providers carry their own alarms; a configured one
    // must not run with those alarms silently omitted.
    for (group, tag, key) in [
        ("ManuParam", "AF", "AFType"),
        ("ManuParam", "EC", "ECType"),
        ("LaserParam", "LGP", "LaserType"),
    ] {
        let configured = Group::read(bundle, group, tag)?.uint(key, u32::MAX)?;
        if configured != 0 {
            return Err(Error::Unsupported(format!(
                "{key}={configured} needs a device alarm provider the beta does not have"
            )));
        }
    }
    let di = Group::read(bundle, "DIParam", "DI")?;
    let mut rules = warning_rules(di, mode == LaserMode::Co2)?;
    rules.extend(custom_rules(di)?);
    rules.extend(gas_rules(di, Group::read(bundle, "GasParam", "MGP")?)?);
    Ok(rules)
}

/// The cooling water and laser source warnings, with the CO2 pair when
/// that laser is selected.
fn warning_rules(di: Group<'_>, co2: bool) -> Result<Vec<InputRule>> {
    let mut rules = vec![];
    let warnings =
        [("WaterWarning", "Cooling-water alarm", 56), ("LaserWarning", "Laser-source alarm", 60)];
    for (key, label, id) in warnings {
        let key = if co2 { format!("CO2{key}") } else { key.to_owned() };
        let input =
            input(di, &key, di.text(&key).ok_or_else(|| Error::Missing(format!("DI.{key}")))?)?;
        if input == 0 {
            continue;
        }
        let polarity_key = format!("{key}Type");
        let text =
            di.text(&polarity_key).ok_or_else(|| Error::Missing(format!("DI.{polarity_key}")))?;
        rules.push(InputRule {
            id,
            label: label.into(),
            input,
            active_low: polarity(di, &polarity_key, text)?,
            run_only: false,
            latch: false,
            gas_valve: None,
        });
    }
    Ok(rules)
}

/// The sixteen custom input records of `AlarmDIStr`: `name#input#polarity#runOnly`.
fn custom_rules(di: Group<'_>) -> Result<Vec<InputRule>> {
    let latch = match di.text("OnlyManualRelieveAlarm") {
        Some("0") => false,
        Some("1" | "-1") => true,
        _ => return Err(di.invalid("OnlyManualRelieveAlarm", "must be 0, 1 or -1")),
    };
    let custom = di.text("AlarmDIStr").ok_or_else(|| Error::Missing("DI.AlarmDIStr".into()))?;
    let records: Vec<&str> = custom.split(',').collect();
    if records.iter().skip(16).any(|record| !record.trim().is_empty()) {
        return Err(di.invalid("AlarmDIStr", "exceeds the sixteen input records"));
    }
    let mut rules = vec![];
    for (index, record) in records.into_iter().take(16).enumerate() {
        if record.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split('#').collect();
        if fields.len() != 4 || fields[0].trim().is_empty() {
            return Err(di.invalid(
                "AlarmDIStr",
                format!("record {record:?} is not name#input#polarity#runOnly"),
            ));
        }
        let input = input(di, "AlarmDIStr", fields[1])?;
        if input == 0 {
            continue;
        }
        rules.push(InputRule {
            id: 1000 + u32::try_from(index).unwrap_or(0),
            label: format!("{} alarm", fields[0].trim()),
            input,
            active_low: polarity(di, "AlarmDIStr", fields[2])?,
            run_only: polarity(di, "AlarmDIStr", fields[3])?,
            latch,
            gas_valve: None,
        });
    }
    Ok(rules)
}

/// The six gas feedback records of `FunctionDIStr`, each qualified by its
/// valve output.
fn gas_rules(di: Group<'_>, gas: Group<'_>) -> Result<Vec<InputRule>> {
    let functions =
        di.text("FunctionDIStr").ok_or_else(|| Error::Missing("DI.FunctionDIStr".into()))?;
    let functions: Vec<&str> = functions.split(',').collect();
    if functions.len() != 10 {
        return Err(di.invalid("FunctionDIStr", "needs the ten indexed records"));
    }
    let mut rules = vec![];
    for (index, key) in
        ["LowAir", "LowO2", "LowN2", "HighAir", "HighO2", "HighN2"].into_iter().enumerate()
    {
        let fields: Vec<&str> = functions[index + 4].split('#').collect();
        if fields.len() != 3 {
            return Err(di.invalid("FunctionDIStr", "gas record is not name#input#polarity"));
        }
        let input = input(di, "FunctionDIStr", fields[1])?;
        if input == 0 {
            continue;
        }
        let valve = gas.byte(key, 26)?;
        if valve == 0 {
            return Err(gas.invalid(key, "feedback is assigned but the valve is not"));
        }
        rules.push(InputRule {
            id: 150 + u32::try_from(index).unwrap_or(0),
            label: format!("{} feedback alarm", fields[0].trim()),
            input,
            active_low: polarity(di, "FunctionDIStr", fields[2])?,
            run_only: false,
            latch: false,
            gas_valve: Some(valve),
        });
    }
    Ok(rules)
}

// Jog -----------------------------------------------------------------------

/// An acceleration word from the rapid acceleration: the vendor truncates
/// the rapid value, takes a tenth for slow motion, and clips to the maximum.
fn acceleration_word(rapid: f64, maximum: f64, factor: f64, fast: bool) -> Result<u32> {
    let mut value = rapid.trunc() * factor;
    if !fast {
        value *= 0.1;
    }
    let word = value.trunc().min(maximum).trunc();
    if word < 1. || word > f64::from(i32::MAX / 10) {
        return Err(Error::Unsupported("the jog acceleration cannot be represented".into()));
    }
    crate::whole(word).ok_or_else(|| Error::Unsupported("the jog acceleration is not whole".into()))
}

/// Each axis's travel in millimetres, lower then upper: the soft limit
/// length, on the side the homing direction puts it.
pub fn extent(bundle: &Bundle) -> Result<[[f64; 2]; 2]> {
    let mut extent = [[0.; 2]; 2];
    for (axis, extent) in extent.iter_mut().enumerate() {
        let group = axis_group(bundle, axis)?;
        let length = group.range("SoftLimitMaxLen", 1e-9, 1e9)?;
        *extent =
            if group.uint("GoOriginalDirection", 1)? == 1 { [-length, 0.] } else { [0., length] };
    }
    Ok(extent)
}

fn jog(bundle: &Bundle, deceleration: u32) -> Result<Jog> {
    let mc = Group::read(bundle, "ManuParam", "MC")?;
    let fcp = Group::read(bundle, "FCParam", "FCP")?;
    let maximum_speed = fcp.range("MaxSpeed", 1e-9, 1e9)?;
    let speed = [
        mc.range("JogSlowSpeed", 1e-9, 100_000.)?.min(maximum_speed),
        mc.range("JogFastSpeed", 1e-9, 100_000.)?.min(maximum_speed),
    ];
    let rapid = mc.range("XFastMoveAcc", 1e-9, 1e12)?;
    let maximum = fcp.range("MaxAcc", 1e-9, 1e12)?;
    let extent = extent(bundle)?;
    Ok(Jog {
        speed,
        acceleration: [
            acceleration_word(rapid, maximum, 1., false)?,
            acceleration_word(rapid, maximum, 1., true)?,
        ],
        // The positioning button uses 0.8 of the rapid acceleration.
        position_acceleration: [
            acceleration_word(rapid, maximum, 0.8, false)?,
            acceleration_word(rapid, maximum, 0.8, true)?,
        ],
        extent,
        soft_limit: Group::read(bundle, "ManuParam", "MS")?.enabled("EnableSoftLimit")?,
        deceleration,
    })
}

// Manual outputs ------------------------------------------------------------

fn outputs(bundle: &Bundle, mode: LaserMode, head_enabled: bool) -> Result<Outputs> {
    let co2 = mode == LaserMode::Co2;
    let laser = Group::read(bundle, "LaserParam", "LGP")?;
    let extended = Group::read(bundle, "ManuParam", "EC")?.uint("ECIOType", 100)? != 0;
    let key = |name: &str| if co2 { format!("CO2{name}") } else { name.to_owned() };
    let pointer_port = port(laser, &key("DORedLight"), extended)?;
    let shutter_port = port(laser, &key("DOLaserGate"), extended)?;
    let hardware = hardware(bundle, mode)?;
    // Under analog control the fiber shutter also raises the peak current
    // to the point-laser setting; CO2 zeroes the channel on close only.
    let shutter_analog = if laser.uint(&key("LaserControlType"), 3)? == 3 {
        let channel = laser.byte(&key("LaserDAPort"), 2)?;
        let peak = Group::read(bundle, "ManuParam", "LC")?.range("PtLaserPeakCurrent", 0., 100.)?;
        let value = if co2 { 0. } else { (peak * hardware.peak_factor).trunc() };
        (channel != 0).then(|| crate::whole(value).map(|value| (channel, value))).flatten()
    } else {
        None
    };
    let head_jog_tenths = head_jog_speeds(bundle, head_enabled)?;
    Ok(Outputs {
        pointer_port,
        shutter_port,
        shutter_analog,
        proportional_delay_ms: Group::read(bundle, "ManuParam", "MGP")?
            .uint("iProportionalOpenSleep", 600_000)?,
        head_jog_tenths,
        hardware,
    })
}

fn head_jog_speeds(bundle: &Bundle, head_enabled: bool) -> Result<Option<[u32; 2]>> {
    let zf = Group::read(bundle, "ZFParam", "ZF")?;
    Ok(if head_enabled && zf.uint("ZFType", 10)? == 1 {
        let tenths = |key: &str| -> Result<u32> {
            crate::whole((zf.range(key, 0., 1000.)? * 10.).trunc())
                .ok_or_else(|| zf.invalid(key, "is not a whole tenth"))
        };
        Some([tenths("ZFJogSpeed")?, tenths("ZFFastJogSpeed")?])
    } else {
        None
    })
}

impl Outputs {
    /// The pointer held on.
    pub fn pointer(&self) -> Result<Manual> {
        let port = assigned(self.pointer_port, "pointer")?;
        Ok(Manual {
            on: vec![Step::Write(requests::digital_output(port, true)?)],
            off: vec![requests::digital_output(port, false)?],
            lease_ms: 2000,
        })
    }

    /// The shutter held open.
    pub fn shutter(&self) -> Result<Manual> {
        let port = assigned(self.shutter_port, "shutter")?;
        let mut on = vec![Step::Write(requests::digital_output(port, true)?)];
        let mut off = vec![requests::digital_output(port, false)?];
        if let Some((channel, value)) = self.shutter_analog {
            if value != 0 {
                on.push(Step::Write(requests::analog_output(channel, value)?));
            }
            off.push(requests::analog_output(channel, 0)?);
        }
        Ok(Manual { on, off, lease_ms: 2000 })
    }

    /// Gas `selector` (0 to 5: low air, oxygen, nitrogen, then high) held
    /// on at `pressure` bar.
    pub fn gas(&self, selector: u8, pressure: f64) -> Result<Manual> {
        let hardware = &self.hardware;
        if !hardware.gas_enabled {
            return Err(Error::Unsupported("gas is not switched on this laser".into()));
        }
        let index = usize::from(selector);
        let main_port = hardware
            .gas_ports
            .get(index)
            .copied()
            .flatten()
            .ok_or_else(|| Error::Unsupported("the gas valve is not assigned".into()))?;
        if !pressure.is_finite() || !(0. ..=100.).contains(&pressure) {
            return Err(Error::Unsupported("the gas pressure must be 0 to 100 bar".into()));
        }
        let (pressure_output, converted) = self.pressure_output(index, pressure)?;
        let binding = GasBinding {
            selector,
            main_port,
            pressure: pressure_output,
            high_selector_analog: HighSelectorAnalog::None,
            delay_ms: self.proportional_delay_ms,
        };
        let on = gas_toggle(binding, true, converted)?;
        let off = gas_toggle(binding, false, 0)?
            .into_iter()
            .filter_map(|step| match step {
                Step::Write(write) => Some(write),
                Step::Delay(_) => None,
            })
            .collect();
        Ok(Manual { on, off, lease_ms: 2000 })
    }

    fn pressure_output(
        &self,
        index: usize,
        pressure: f64,
    ) -> Result<(Option<PressureOutput>, i32)> {
        if index >= 3 {
            return Ok((None, 0));
        }
        let hardware = &self.hardware;
        let channel = hardware.ratio_channels[index].unwrap_or(0);
        let maximum = hardware.gas_max_pressure[index];
        if channel != 0 && maximum <= 0. {
            return Err(Error::Unsupported("the gas maximum pressure must be positive".into()));
        }
        let volts = match &hardware.gas_calibration[index] {
            Some(curve) => curve.voltage(pressure)?,
            None if channel == 0 => 0.,
            None => pressure / maximum * 10.,
        };
        if !(0. ..=10.).contains(&volts) {
            return Err(Error::Unsupported("the gas pressure exceeds the calibrated range".into()));
        }
        let output = PressureOutput {
            auxiliary_port: hardware.ratio_ports[index].unwrap_or(0),
            analog_channel: channel,
        };
        Ok((Some(output), coordinate((volts * 1000.).trunc(), 1)?))
    }

    /// The head jogged up or down while held.
    pub fn head_jog(&self, up: bool, fast: bool) -> Result<Manual> {
        let tenths = self
            .head_jog_tenths
            .ok_or_else(|| Error::Unsupported("the native head jog needs a type 1 head".into()))?;
        let speed = i32::try_from(tenths[usize::from(fast)])
            .map_err(|_| Error::Unsupported("head jog speed".into()))?;
        if speed == 0 || speed > 10_000 {
            return Err(Error::Unsupported("the head jog speed must be 0.1 to 1000".into()));
        }
        let travel = if up { -1_000_000 } else { 1_000_000 };
        Ok(Manual {
            on: vec![Step::Write(requests::head_move(speed, travel))],
            off: vec![requests::head_cancel()],
            lease_ms: 300,
        })
    }
}

fn assigned(port: u8, what: &str) -> Result<u8> {
    if port == 0 {
        return Err(Error::Unsupported(format!("the {what} output is not assigned")));
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, Kind};

    /// A backup with the groups the bindings read, on the capture harness's
    /// hardware: a fiber gate on port 5, CO2 laser on 9, oxygen valve 7 with
    /// its proportional channel 2, a door on input 4, and oxygen feedback
    /// on input 12.
    fn backup() -> Bundle {
        let xml = concat!(
            "<ParameterRoot>",
            "<PAxisParam><AX InterpolationCycle='1000'/><A0 AxisIndex='0'/><A1 AxisIndex='1'/></PAxisParam>",
            "<PMachineAxisConfig><MAC DoubleDevice='0'/></PMachineAxisConfig>",
            "<PMachineAxisConfig_0><MAC WritePluse='8000' SpeedRatio='31.04' SoftLimitMaxLen='1371' GoOriginalDirection='0'/></PMachineAxisConfig_0>",
            "<PMachineAxisConfig_1><MAC_1 WritePluse='8000' SpeedRatio='31.04' SoftLimitMaxLen='971' GoOriginalDirection='1'/></PMachineAxisConfig_1>",
            "<PSoftParam><SP m_iHardwareModel='224' m_iEnableLaserType='0'/></PSoftParam>",
            "<PManuParam>",
            "<MC XFastMoveAcc='5000' BoundSpeed='500' JogSlowSpeed='20' JogFastSpeed='500' AccTime='100' afterContinueDill='1'/>",
            "<MP JogStopDccFactor='2' ZFSafeHeight='10' EnableVerCorrect='0' LaserDAKeepOutput='0' EnablePLCProcess1='0' CO2PWMOutputSync='1' ",
            "DirectDrillMaxHeight='6' EnableManuCrashProtect='1' ManuCrashProtectUpHeight='35'/>",
            "<MS EnableSoftLimit='1'/><LC PtLaserFreq='5000' PtLaserPeakCurrent='30'/><EC ECIOType='0' ECType='0'/><AF AFType='0'/>",
            "<GC GasDelay='100' DirectGasDelay='100' ChangeGasDelay='100'/><MGP iProportionalOpenSleep='300'/>",
            "</PManuParam>",
            "<PFCParam><FCP MaxSpeed='3000' MaxAcc='30000'/></PFCParam>",
            "<PZFParam><ZF ZFType='1' ZFUpSpeed='100' ZFDockHeight='3' ZFJogSpeed='30' ZFFastJogSpeed='60'/></PZFParam>",
            "<PLaserParam><LGP LaserControlType='3' LaserType='0' LaserDAPort='1' LaserDAType='0' DOLaserGate='5' DOLaser='0' DORedLight='6' ",
            "CO2LaserControlType='2' CO2LaserDAType='1' CO2LaserDAPort='0' CO2DOLaserGate='0' CO2DOLaser='9' CO2DORedLight='0' doCO2EnableOutput='0'/></PLaserParam>",
            "<PGasParam><MGP LowAir='0' LowO2='7' LowN2='0' HighAir='3' HighO2='0' HighN2='2' RatioAirSwitch='0' RatioO2Switch='8' RatioN2Switch='0' ",
            "RatioAir='0' RatioO2='2' RatioH2='0' NewDAMAxPressureAir='10' NewDAMAxPressureO2='10' NewDAMAxPressureN2='10' CoolGas='0'/></PGasParam>",
            "<PDOParam><DO ZFGoOriginDone='2' ManuSignal='1' BackRollMoveEnable='0' RollSheetSlowMove='0' RollSheetFastMove='0' RollSheetMoveEnable='0' ",
            "ManuDoneOutput='0' RollSheetOutput='0' ManuOutput='0' PlatBMoveEnable='0' PlatAMoveEnable='0' PlatBFastMove='0' PlatBSlowMove='0' PlatAFastMove='0' PlatASlowMove='0' ",
            "PLCProcess1B='0' PLCProcess1A='0' GtO2EnableGasDAMap='0'/></PDOParam>",
            "<PDIParam><DO EnableBackRollSheet='0' GtO2EnableGasDAMap='0'/>",
            "<DI WaterWarning='3' WaterWarningType='1' LaserWarning='0' LaserWarningType='0' CO2WaterWarning='0' CO2WaterWarningType='0' CO2LaserWarning='0' CO2LaserWarningType='0' ",
            "OnlyManualRelieveAlarm='0' AlarmDIStr='Door#4#1#0,,,,,,,,,,,,,,,' FunctionDIStr='a#0#0,b#0#0,c#0#0,d#0#0,LowAir#0#0,LowO2#12#1,LowN2#0#0,HighAir#0#0,HighO2#0#0,HighN2#0#0'/></PDIParam>",
            "</ParameterRoot>"
        );
        Bundle::from_backup(Document::parse(Kind::Backup, xml.as_bytes()).unwrap())
    }

    fn words(writes: &[Write]) -> Vec<Vec<u32>> {
        writes.iter().map(|w| w.words.clone()).collect()
    }

    /// The fiber bindings, field for field: the stop policy with the
    /// vendor's port list, the Go Origin outputs, the mode switch cleanup
    /// and limits with the half-millimetre margin, the three rules, and the
    /// jog words.
    #[test]
    fn fiber_bindings_reproduce_the_vendor_values() {
        let b = bind(&backup(), LaserMode::Fiber, 1000).unwrap();
        assert!(b.head_enabled && b.resume_pierce);
        assert_eq!(b.co2_pwm_sync_port, Some(9));
        assert_eq!(b.home, Home { z_origin_done_port: 2, manual_signal_port: 1 });
        let shutdown = &b.shutdown;
        assert_eq!((shutdown.rapid_deceleration, shutdown.point_laser_frequency), (10_000, 5000));
        assert_eq!(shutdown.gas_channels, [0, 2, 0]);
        assert_eq!(
            shutdown.raise,
            Some(sequences::Raise { speed_tenths: 1000, height_thousandths: 10_000 })
        );
        assert_eq!(
            shutdown.ports,
            vec![
                2, 0, 3, 0, 7, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 8, 0
            ]
        );
        assert!(!shutdown.co2_skip_head_cancel && !shutdown.extended_outputs);
        assert_eq!(b.mode_switch.xy_limits, [(-500, 1_371_000), (-971_000, 500)]);
        assert_eq!(
            words(&b.mode_switch.cleanup),
            [
                vec![9999, 3, 5000, 0, 0],
                vec![9999, 17, 5000, 0, 0],
                vec![9999, 2, 16, 0],
                vec![9999, 2, 32, 0],
                vec![9999, 4, 0, 0],
                vec![9999, 2, 256, 0],
            ]
        );
        assert_eq!(b.mode_switch.enable, None);
        assert_eq!(b.dual_drive.xy_limits, b.mode_switch.xy_limits);
        assert_eq!(
            b.rules
                .iter()
                .map(|r| (r.id, r.input, r.active_low, r.run_only, r.latch, r.gas_valve))
                .collect::<Vec<_>>(),
            [
                (56, 3, true, false, false, None),
                (1000, 4, true, false, false, None),
                (151, 12, true, false, false, Some(7))
            ]
        );
        assert_eq!(b.rules[1].label, "Door alarm");
        let jog = b.jog;
        assert_eq!(
            (jog.speed, jog.acceleration, jog.position_acceleration),
            ([20., 500.], [500, 5000], [400, 4000])
        );
        assert_eq!(jog.extent, [[0., 1371.], [-971., 0.]]);
        assert!(jog.soft_limit);
        assert_eq!(jog.deceleration, 10_000);
        assert_eq!(jog.speed_word(20., 1000).unwrap(), 20_000);
        assert_eq!(jog.travel(0, 100., 5., false, 1000).unwrap(), 5000);
        assert_eq!(jog.travel(0, 100., 1., true, 1000).unwrap(), 1_271_000);
        assert_eq!(jog.travel(1, -970.8, -1., true, 1000).unwrap(), -200);
        assert!(jog.travel(0, 1371., 1., false, 1000).is_err());
    }

    /// The manual outputs: the pointer, the shutter with its peak current,
    /// oxygen with the proportional valve, delay and pressure, and the head
    /// jog with its cancel.
    #[test]
    fn manual_outputs_follow_the_vendor_sequences() {
        let b = bind(&backup(), LaserMode::Fiber, 1000).unwrap();
        let pointer = b.outputs.pointer().unwrap();
        assert_eq!(pointer.on, vec![Step::Write(requests::digital_output(6, true).unwrap())]);
        assert_eq!(words(&pointer.off), [vec![9999, 2, 32, 0]]);
        let shutter = b.outputs.shutter().unwrap();
        assert_eq!(
            shutter.on,
            vec![
                Step::Write(requests::digital_output(5, true).unwrap()),
                Step::Write(requests::analog_output(1, 3000).unwrap())
            ]
        );
        assert_eq!(words(&shutter.off), [vec![9999, 2, 16, 0], vec![9999, 4, 0, 0]]);
        let oxygen = b.outputs.gas(1, 5.).unwrap();
        assert_eq!(
            oxygen.on,
            vec![
                Step::Write(requests::digital_output(7, true).unwrap()),
                Step::Write(requests::digital_output(8, true).unwrap()),
                Step::Delay(300),
                Step::Write(requests::analog_output(2, 5000).unwrap()),
            ]
        );
        assert_eq!(
            words(&oxygen.off),
            [vec![9999, 2, 64, 0], vec![9999, 4, 1, 0], vec![9999, 2, 128, 0]]
        );
        assert!(b.outputs.gas(0, 5.).is_err(), "air has no valve");
        let up = b.outputs.head_jog(true, false).unwrap();
        assert_eq!(up.on, vec![Step::Write(requests::head_move(300, -1_000_000))]);
        assert_eq!(words(&up.off), [vec![101]]);
        assert_eq!(up.lease_ms, 300);
    }

    /// CO2 bindings skip the fiber warnings for the CO2 ones, and a machine
    /// with a persistent CO2 enable output or an unsupported control type is
    /// refused before anything is sent.
    #[test]
    fn co2_and_refusals() {
        let b = bind(&backup(), LaserMode::Co2, 1000).unwrap();
        assert_eq!(b.rules.iter().map(|r| r.id).collect::<Vec<_>>(), [1000, 151]);
        assert!(b.outputs.pointer().is_err());
        assert!(b.outputs.gas(1, 5.).is_err(), "CO2 uses only the High Air valve");
        let air = b.outputs.gas(3, 0.).unwrap();
        assert_eq!(air.on, vec![Step::Write(requests::digital_output(3, true).unwrap())]);
        assert_eq!(words(&air.off), [vec![9999, 2, 4, 0]]);
        let backup = backup();
        let document = backup.document(Kind::Backup).unwrap();
        let edited = document
            .with_attribute("/ParameterRoot/PLaserParam/LGP", "doCO2EnableOutput", "10")
            .unwrap();
        let bundle = Bundle::from_backup(edited);
        assert!(bind(&bundle, LaserMode::Co2, 1000).is_err());
        let fiber = bind(&bundle, LaserMode::Fiber, 1000).unwrap();
        assert_eq!(words(&fiber.mode_switch.cleanup).last().unwrap(), &vec![9999, 2, 512, 0]);
        let edited = document
            .with_attribute("/ParameterRoot/PLaserParam/LGP", "LaserControlType", "1")
            .unwrap();
        assert!(bind(&Bundle::from_backup(edited), LaserMode::Fiber, 1000).is_err());
    }
}
