// SPDX-License-Identifier: GPL-3.0-or-later

//! Binding one laser and layer into the compiler's settings, the way the
//! vendor host reads its own parameters: MainApp `0x0045_01E0` (the job
//! settings), `0x0045_DD00` and `0x004A_8770` (pierce stages), and the
//! process hardware readers.
//!
//! Every rule here is the vendor's: which attribute feeds which field, how
//! it is scaled, and which combinations are refused. A film process binds
//! from its own bank as cutting motion alone; the recipe's preliminary
//! piercing, film and gas-retention switches bind for the compiler's pass
//! scheduler, and a disabled layer is refused.

use crate::attrs::Group;
use crate::document::{Attributes, Bundle};
use crate::{Error, Result};
use openlaser_compiler::settings::{
    Edge, Hardware, HeadTravel, PierceStage, Residue, Settings, Timeouts,
};
use openlaser_core::LaserMode;

/// What to bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Which laser.
    pub mode: LaserMode,
    /// The layer bank, 1 to 11.
    pub layer: u8,
    /// A dry run: motion without process.
    pub dry_run: bool,
    /// Bypass height control and crash protection for the CO2 setup.
    /// This does not describe optical focus, which is set manually on
    /// both lasers. It must remain false for fiber Z control.
    pub manual_focus: bool,
    /// Bind the film process: cutting motion under the bank's values, with
    /// no piercing, cleaning or passes of its own.
    pub film: bool,
    /// The host timeouts, from `ipAdd.ini`.
    pub timeouts: Timeouts,
}

/// The machine's motion geometry, shared by every recipe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Machine {
    /// Encoder counts per millimetre on each axis: pulses per turn over the
    /// millimetres per turn.
    pub counts_per_mm: [f64; 2],
    /// The soft travel limit of each axis in millimetres.
    pub travel_limits: [f64; 2],
    /// The controller's interpolation cycle in microseconds.
    pub interpolation_cycle_us: u32,
}

/// The vendor's machining types and the ordinary pierce stages each runs:
/// flat cutting, fixed-height cutting, one to three stages, absolute-height
/// cutting, then four and five stages.
pub const MACHINING_KINDS: [(u8, u8); 8] =
    [(0, 0), (1, 0), (2, 1), (3, 2), (4, 3), (5, 0), (6, 4), (7, 5)];

/// Only an absent optional group permits a default; malformed groups do not.
fn optional_read<'a>(
    bundle: &'a Bundle,
    group: &'a str,
    tag: &'a str,
) -> Result<Option<Group<'a>>> {
    match Group::read(bundle, group, tag) {
        Ok(value) => Ok(Some(value)),
        Err(Error::Missing(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// A rate in `0..=max` that must be present, from a group that may not be.
fn required_rate(bundle: &Bundle, group: &str, tag: &str, key: &str, max: f64) -> Result<f64> {
    match bundle.group(group, tag) {
        Ok(attributes) if attributes.contains_key(key) => {
            Group::new(attributes, group, tag).range(key, 0., max)
        }
        Ok(_) | Err(Error::Missing(_)) => Err(Error::Missing(format!("{group}.{tag}.{key}"))),
        Err(error) => Err(error),
    }
}

/// The machine geometry from the axis groups.
pub fn machine(bundle: &Bundle) -> Result<Machine> {
    let ax = Group::read(bundle, "AxisParam", "AX")?;
    let interpolation_cycle_us = ax.uint("InterpolationCycle", 1_000_000)?;
    if interpolation_cycle_us == 0 {
        return Err(ax.invalid("InterpolationCycle", "must be positive"));
    }
    let mut counts_per_mm = [0.; 2];
    let mut travel_limits = [0.; 2];
    for axis in 0..2 {
        let group = format!("MachineAxisConfig_{axis}");
        let tag = if axis == 0 { "MAC".to_owned() } else { format!("MAC_{axis}") };
        let a = Group::new(bundle.group(&group, &tag)?, &group, "");
        let pulses = a.range("WritePluse", 1., f64::from(i32::MAX))?;
        if pulses.fract() != 0. {
            return Err(a.invalid("WritePluse", "must be a whole number"));
        }
        counts_per_mm[axis] = pulses / a.range("SpeedRatio", 1e-9, 1e9)?;
        travel_limits[axis] = a.range("SoftLimitMaxLen", 0.001, 100_000.)?;
    }
    Ok(Machine { counts_per_mm, travel_limits, interpolation_cycle_us })
}

/// The settings for one laser and layer.
pub fn bind(bundle: &Bundle, request: Request) -> Result<Settings> {
    bind_attributes(bundle, request, None)
}

/// Bind recipe attributes over their machine bank without changing or
/// serializing the lossless source documents. Missing recipe fields keep
/// the machine value; duplicate machine groups remain errors.
pub fn bind_overlay(
    bundle: &Bundle,
    request: Request,
    attributes: &Attributes,
) -> Result<Settings> {
    bind_attributes(bundle, request, Some(attributes))
}

mod layer;

fn bind_attributes(
    bundle: &Bundle,
    request: Request,
    attributes: Option<&Attributes>,
) -> Result<Settings> {
    request.validate()?;
    let machine = machine(bundle)?;
    let bank = format!(
        "{}{}",
        if request.mode == LaserMode::Co2 { "CO2LayerParam" } else { "LayerParam" },
        request.layer
    );
    let a = if let Some(attributes) = attributes {
        let defaults = optional_read(bundle, &bank, "GP")?;
        Group::new(attributes, &bank, "").with_defaults(defaults.map(|a| a.attributes()))
    } else {
        Group::new(bundle.group(&bank, "GP")?, &bank, "")
    };
    if a.enabled("NoManu")? {
        return Err(Error::Unsupported(format!("layer {} is disabled", request.layer)));
    }
    layer::read(bundle, a, request, machine)
}

impl Request {
    fn validate(self) -> Result<()> {
        if !(1..=11).contains(&self.layer) {
            return Err(Error::Unsupported("the layer must be 1 through 11".into()));
        }
        if self.manual_focus && self.mode != LaserMode::Co2 {
            return Err(Error::Unsupported(
                "height-control bypass is only supported for CO2".into(),
            ));
        }
        Ok(())
    }
}

/// The older host's contour shift: the X and Y translation when enabled,
/// within its input domain of 100 000 mm, nothing otherwise.
fn contour_shift(a: Group<'_>) -> Result<[f64; 2]> {
    if !a.enabled("EnableContourShift")? {
        return Ok([0.; 2]);
    }
    Ok([
        a.range("ContourShiftXDist", -100_000., 100_000.)?,
        a.range("ContourShiftYDist", -100_000., 100_000.)?,
    ])
}

/// A whole percentage as the vendor stores it, kept as a number.
fn whole_percent(a: Group<'_>, key: &str) -> Result<f64> {
    let value = a.range(key, 0., 100.)?;
    if value.fract() != 0. {
        return Err(a.invalid(key, "must be a whole percentage"));
    }
    Ok(value)
}

/// The layer's machining type and its stage count.
fn machining_kind(a: Group<'_>) -> Result<(u8, u8)> {
    let value = a.optional_uint("ManuType", u32::from(u8::MAX))?;
    MACHINING_KINDS.into_iter().find(|(kind, _)| u32::from(*kind) == value).ok_or_else(|| {
        Error::Unsupported(format!("{}.ManuType {value} is not a known machining type", a.label()))
    })
}

/// The pierce stages in execution order: the vendor runs the highest active
/// stage first. Smooth piercing replaces the ordinary stages with one.
fn pierce_stages(
    a: Group<'_>,
    stages: u8,
    co2: bool,
    smooth: bool,
    cycle_us: u32,
) -> Result<Vec<PierceStage>> {
    if smooth {
        return Ok(vec![smooth_stage(a, co2)?]);
    }
    (0..stages).rev().map(|i| pierce_stage(a, i, co2, cycle_us)).collect()
}

/// Ordinary stage `i`, with its bolt-drill bank when that is switched on.
fn pierce_stage(a: Group<'_>, i: u8, co2: bool, cycle_us: u32) -> Result<PierceStage> {
    let key = |s: &str| format!("{s}{i}");
    let bolt = bolt_drill(a, i, cycle_us)?;
    let duration_ms = gradual_duration(a, i)?;
    let (gas, pressure) = gas_settings(a, !co2, &key("DrillGasType"), &key("DrillGasPressure"))?;
    Ok(PierceStage {
        index: i,
        height: a.range(&key("DrillHeight"), 0., 1000.)?,
        power: a.percent(&key("DrillPower"))?,
        frequency: a.frequency(&key("DrillFreq"))?,
        peak_current: whole_percent(a, &key("DrillPeakCurrent"))?,
        gas,
        pressure,
        duration_ms,
        gradual: a.enabled(&key("EnableGradualDrill"))?,
        gradual_ms: duration_ms,
        before_off_ms: a.optional_uint(&key("BeforeLaserOffDelay"), 600_000)?,
        after_off_ms: a.optional_uint(&key("AfterLaserOffDelay"), 600_000)?,
        bolt,
    })
}

/// CO2 air assist uses the high-air valve, with pressure set at the regulator.
pub const CO2_GAS: u8 = 3;

fn gas_settings(a: Group<'_>, fiber: bool, selector: &str, pressure: &str) -> Result<(u8, f64)> {
    if !fiber {
        return Ok((CO2_GAS, 0.));
    }
    Ok((a.byte(selector, 5)?, a.range(pressure, 0., 100.)?))
}

fn bolt_drill(a: Group<'_>, i: u8, cycle_us: u32) -> Result<Option<(u8, u16)>> {
    let bolt_key = |s: &str| format!("BoltDrill_{s}_{}", i + 1);
    Ok(if a.enabled(&bolt_key("Enable"))? {
        let frequency = a.frequency(&bolt_key("Freq"))?;
        if !1000u32.is_multiple_of(cycle_us) {
            return Err(Error::Unsupported(
                "bolt drilling needs an interpolation cycle that divides one millisecond".into(),
            ));
        }
        Some((a.percent(&bolt_key("Power"))?, frequency))
    } else {
        None
    })
}

/// The smooth-pierce stage, MainApp `0x0045_DD00`'s early return: it hands
/// straight into cutting, so its stored time is never consumed.
fn smooth_stage(a: Group<'_>, co2: bool) -> Result<PierceStage> {
    let (gas, pressure) = gas_settings(a, !co2, "CutGasType", "CutAirPressure")?;
    Ok(PierceStage {
        index: 14,
        height: a.range("SmoothPierceDrillHeight", 0., 1000.)?,
        power: a.percent("SmoothPierceDrillPower")?,
        frequency: a.frequency("SmoothPierceDrillFreq")?,
        peak_current: whole_percent(a, "SmoothPierceDrillPeakCurrent")?,
        gas,
        pressure,
        duration_ms: 0,
        gradual: false,
        gradual_ms: 0,
        before_off_ms: 0,
        after_off_ms: 0,
        bolt: None,
    })
}

/// The three names of a stage's duration slot, for each bank.
pub const DURATION_ALIASES: [&str; 3] = ["DrillDelay", "GradualTime", "FocusGradualTime"];

/// Maximum duration accepted by the native stage writer, in milliseconds.
pub const MAX_STAGE_DURATION_MS: u32 = 600_000;

/// Raw values retained by this host but not connected to a process command.
pub const STORED_FIELDS: [&str; 9] = [
    "CutFocusPos",
    "SmoothPierceDrillFocusPos",
    "CleanResidue_WorkFocus",
    "SmoothPierceDrillTime_ms",
    "PreLaserOnFactor",
    "ZFVibAbat_Coef",
    "LeadLineParam_Enable",
    "PowerCurveSmoothType",
    "FreqCurveSmoothType",
];

/// Stage families retained but not acted on; duration aliases are separate.
pub const STORED_STAGE_FIELDS: [&str; 4] =
    ["DrillFocusPos", "EnableFocusGradual", "FocusGradualEndPos", "DrillTime"];

/// Applies editor changes, canonicalizing only duration slots that were edited.
/// Untouched aliases and unknown attributes retain their imported spelling.
pub fn edit_attributes(attributes: &mut Attributes, edits: &Attributes) -> Result<()> {
    let group = Group::new(edits, "recipe", "");
    let mut durations = Vec::new();
    for stage in 0..5 {
        if DURATION_ALIASES.iter().any(|prefix| edits.contains_key(&format!("{prefix}{stage}"))) {
            durations.push((stage, gradual_duration(group, stage)?));
        }
    }
    attributes.extend(edits.clone());
    for (stage, value) in durations {
        for prefix in DURATION_ALIASES {
            attributes.remove(&format!("{prefix}{stage}"));
        }
        attributes.insert(format!("{}{stage}", DURATION_ALIASES[0]), value.to_string());
    }
    Ok(())
}

/// A stage's duration: `DrillDelay`, `GradualTime` and `FocusGradualTime`
/// name one native slot, so whichever are present must agree; none means
/// zero.
fn gradual_duration(a: Group<'_>, stage: u8) -> Result<u32> {
    let mut selected: Option<u32> = None;
    for prefix in DURATION_ALIASES {
        let key = format!("{prefix}{stage}");
        if !a.has(&key) {
            continue;
        }
        let value = a.uint(&key, MAX_STAGE_DURATION_MS)?;
        if selected.is_some_and(|prior| prior != value) {
            return Err(a.invalid(&key, format!("Conflicting aliases for stage {stage} duration")));
        }
        selected = Some(value);
    }
    Ok(selected.unwrap_or(0))
}

/// The residue-cleaning bank, when the pass is active and enabled.
fn residue(a: Group<'_>, active: bool, gas_enabled: bool) -> Result<Option<Residue>> {
    if !active || !a.enabled("CleanResidue_Enable")? {
        return Ok(None);
    }
    let key = |s: &str| format!("CleanResidue_{s}");
    let (gas, pressure) = gas_settings(a, gas_enabled, &key("GasType"), &key("GasP"))?;
    Ok(Some(Residue {
        height: a.range(&key("WorkH"), 0., 1000.)?,
        speed: a.range(&key("WorkV"), 0.01, 100_000.)?,
        gas,
        pressure,
        peak_current: a.range(&key("PeakCurrent"), 0., 100.)?,
        power: a.percent(&key("Power"))?,
        frequency: a.frequency(&key("Freq"))?,
        radius: a.range(&key("WorkR"), 0., 100_000.)?.max(0.1),
        turns: a.uint(&key("SpiralTimes"), 4096)?.max(1),
    }))
}

/// A slow edge region, `Up` or `Down`, when active and enabled.
fn edge(a: Group<'_>, side: &str, active: bool) -> Result<Option<Edge>> {
    let key = |s: &str| format!("UD_{side}{s}");
    if !active || !a.enabled(&key("Enable"))? {
        return Ok(None);
    }
    let pwm = if a.enabled(&key("AdvEnable"))? {
        Some((a.percent(&key("Duty"))?, a.frequency(&key("Freq"))?))
    } else {
        None
    };
    Ok(Some(Edge {
        length: a.range(&key("Len"), 0., 100_000.)?,
        speed: a.range(&key("Speed"), 0.01, 100_000.)?,
        pwm,
    }))
}

/// Travel speed, acceleration and acceleration time. In CO2 mode the
/// manual parameters may scale speed and acceleration.
fn travel(bundle: &Bundle, mc: Group<'_>, co2: bool) -> Result<(f64, f64, f64)> {
    let factor = |key: &str| -> Result<f64> {
        if !co2 {
            return Ok(1.);
        }
        let mp = Group::read(bundle, "ManuParam", "MP")?;
        if mp.has(key) { mp.range(key, 0.01, 10.) } else { Ok(1.) }
    };
    let speed = mc.range("XFastMoveSpeed", 0.01, 100_000.)? * factor("EmptyMoveSpeedFactor")?;
    let acceleration = mc.range("XFastMoveAcc", 1., 1e7)? * factor("EmptyMoveAccFactor")?;
    if speed > 100_000. || acceleration > 1e7 {
        return Err(Error::Unsupported(
            "the scaled travel settings exceed the planner's limits".into(),
        ));
    }
    let time = mc.range("EmptyMoveAccTime", 1., 10_000.)? * 0.001;
    Ok((speed, acceleration, time))
}

/// The small-circle speed ratio, when the manual parameters enable it.
fn small_circle_ratio(bundle: &Bundle) -> Result<Option<f64>> {
    let Some(mp) = optional_read(bundle, "ManuParam", "MP")? else { return Ok(None) };
    if mp.enabled("EnableSmallCircleSpeedLimit")? {
        mp.range("SmallCircleSpeedLimitRatio", 0., 1.).map(Some)
    } else {
        Ok(None)
    }
}

/// The vibration abatement word: the low level, the thick level in the
/// next byte, both shifted into the upper half, selected by the type.
fn vibration_word(a: Group<'_>) -> Result<u32> {
    let mode = a.optional_uint("ZFVibAbatType", 3)?;
    let low = a.optional_uint("ZFVibAbat_Level", 255)?;
    let high = a.optional_uint("ZFVibAbat_Level_Thick", 255)?;
    let low = if mode == 0 || mode == 2 { 0 } else { low };
    let high = if mode == 0 || mode == 1 { 0 } else { high };
    Ok((low | (high << 8)) << 16)
}

/// How the head travels between contours.
fn head_travel(
    bundle: &Bundle,
    a: Group<'_>,
    stages: u8,
    hardware: Option<&Hardware>,
) -> Result<HeadTravel> {
    let uses_z = hardware.is_some_and(|h| h.z_enabled);
    let fc = optional_read(bundle, "ManuParam", "FC")?;
    let frog_jump =
        uses_z && fc.map(|fc| fc.enabled("EnableLeapFrogUp")).transpose()?.unwrap_or(false);
    let jump_active = frog_jump && hardware.is_some_and(|h| h.crash_height.is_none());
    let minimum_height = if jump_active {
        Some(Group::read(bundle, "FCParam", "FCP")?.range("FrogJumpMinHeight", 0., 1000.)?)
    } else {
        None
    };
    let protect_insertions =
        uses_z && Group::read(bundle, "ManuParam", "MP")?.enabled("EnableZFFrogJumpProtect")?;
    let short_transfer = fc
        .filter(|fc| fc.has("ShortNoUpMaxLength"))
        .map(|fc| fc.range("ShortNoUpMaxLength", 0., 100_000.))
        .transpose()?;
    let keep_height_below = if uses_z && a.enabled("ShortDistNoUp")? {
        Some(
            short_transfer
                .ok_or_else(|| Error::Missing("ManuParam.FC.ShortNoUpMaxLength".into()))?,
        )
    } else {
        None
    };
    // The highest ordinary stage, which the vendor pierces first, even when
    // smooth piercing replaces the ordinary stages.
    let jump_pierce_height = if jump_active && stages > 0 {
        Some(a.range(&format!("DrillHeight{}", stages - 1), 0., 1000.)?)
    } else {
        None
    };
    Ok(HeadTravel {
        frog_jump,
        minimum_height,
        protect_insertions,
        keep_height_below,
        jump_pierce_height,
        short_transfer,
    })
}

mod hardware;

/// The process hardware for one laser.
pub fn hardware(bundle: &Bundle, mode: LaserMode) -> Result<Hardware> {
    hardware::read(bundle, mode)
}
