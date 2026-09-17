// SPDX-License-Identifier: GPL-3.0-or-later

//! Bind a layer directly into compiler settings. Shared readers handle conditional
//! process and head fields without a second set of temporary settings records.

use super::{Group, Machine, Request};
use crate::{Error, Result, document::Bundle};
use openlaser_compiler::settings::{Hardware, PierceStage, PrePierce, Residue, Settings};
use openlaser_core::LaserMode;

struct Layer<'a> {
    bundle: &'a Bundle,
    a: Group<'a>,
    request: Request,
}

#[allow(
    clippy::too_many_lines,
    reason = "direct projection into the compiler's flat settings record"
)]
pub(super) fn read(
    bundle: &Bundle,
    a: Group<'_>,
    request: Request,
    machine: Machine,
) -> Result<Settings> {
    let layer = Layer { bundle, a, request };
    let hardware = layer.hardware()?;
    let uses_z = hardware.as_ref().is_some_and(|h| h.z_enabled);
    let head_off = layer.co2() && !uses_z;
    let (kind, stages) = if request.film { (0, 0) } else { super::machining_kind(a)? };
    let smooth_pierce = layer.smooth()?;
    let pierce = if request.dry_run {
        Vec::new()
    } else {
        super::pierce_stages(a, stages, layer.co2(), smooth_pierce, machine.interpolation_cycle_us)?
    };
    let residue = super::residue(a, !request.dry_run && !smooth_pierce && kind != 0, !layer.co2())?;
    require_head(hardware.as_ref(), &pierce, residue.as_ref())?;
    let pre_pierce = layer.pre_pierce(!pierce.is_empty())?;
    let mc = Group::read(bundle, "ManuParam", "MC")?;
    let (travel_speed, travel_acceleration, travel_acceleration_time) =
        super::travel(bundle, mc, layer.co2())?;
    let (gas, pressure) = layer.gas()?;
    // Registered by the vendor but not consumed: validate without activating it.
    if !request.dry_run {
        a.enabled("LeadLineParam_Enable")?;
    }
    Ok(Settings {
        mode: request.mode,
        layer: request.layer,
        dry_run: request.dry_run,
        counts_per_mm: machine.counts_per_mm,
        interpolation_cycle_us: machine.interpolation_cycle_us,
        acceleration: mc.range("ManuAcc", 1., 1e7)?,
        acceleration_time: mc.range("AccTime", 1., 10_000.)? * 0.001,
        speed: a.range("CutSpeed", 0.01, 100_000.)?,
        maximum_cut_speed: super::required_rate(bundle, "FCParam", "FCP", "MaxSpeed", 100_000.)?,
        native_speed_cap: None,
        corner_accuracy: super::required_rate(bundle, "ManuParam", "MC", "CornerAccuracyRate", 1.)?,
        spline_accuracy: super::required_rate(bundle, "ManuParam", "MC", "SplineAccuracyRate", 1.)?,
        small_circle_limit_ratio: super::small_circle_ratio(bundle)?,
        slow_start: layer.slow_start()?,
        edge_start: super::edge(a, "Up", !request.dry_run)?,
        edge_end: super::edge(a, "Down", !request.dry_run)?,
        power: if request.dry_run {
            0
        } else {
            a.percent(if layer.co2() { "CutDuty" } else { "CutPower" })?
        },
        frequency: a.frequency("CutFreq")?,
        peak_current: layer.peak_current()?,
        gas,
        pressure,
        cut_height: layer.height(head_off, "CutHeight")?,
        retract_height: layer.height(head_off, "UpHeight")?,
        z_up_speed: layer.head_speed(head_off, "ZFUpSpeed")?,
        z_follow_speed: layer.head_speed(head_off, "ZFFollowSpeed")?,
        fixed_height: kind == 1,
        absolute_cut_height: if uses_z && kind == 5 {
            Some(a.range("AdvFixHeightCutPos", 0., 1000.)?)
        } else {
            None
        },
        no_follow: a.enabled("NoFollow")?,
        pierce,
        residue,
        on_delay_ms: layer.delay("LaserOnDelay")?,
        off_before_ms: layer.delay("LaserOffBeforeDelay")?,
        off_after_ms: layer.delay("LaserOffAfterDelay")?,
        power_curve: layer.curve("PowerAdjustWithSpeed", "PWMCurveNodes")?,
        frequency_curve: layer.curve("FreqAdjustWithSpeed", "FreqCurveNodes")?,
        vibration_word: super::vibration_word(a)?,
        head_travel: super::head_travel(bundle, a, stages, hardware.as_ref())?,
        hardware,
        travel_speed,
        travel_acceleration,
        travel_acceleration_time,
        machining_kind: kind,
        smooth_pierce,
        timeouts: request.timeouts,
        pre_pierce,
        with_film: layer.process() && a.enabled("WithFilm")?,
        film_batch: mc.optional_uint("ClearUpFilmNum_Pre", 100_000)? as usize,
        keep_gas_after: layer.gas_switch("NoCloseGasInManu")?,
        short_gas_keep: layer.gas_switch("ShortDistGasKeepOn")?,
        contour_shift: super::contour_shift(a)?,
    })
}

impl Layer<'_> {
    fn co2(&self) -> bool {
        self.request.mode == LaserMode::Co2
    }

    fn process(&self) -> bool {
        !self.request.film && !self.request.dry_run
    }

    fn hardware(&self) -> Result<Option<Hardware>> {
        if self.request.dry_run {
            return Ok(None);
        }
        let mut hardware = super::hardware(self.bundle, self.request.mode)?;
        if self.co2() && hardware.gas_ports[usize::from(super::CO2_GAS)].is_none() {
            return Err(Error::Unsupported(
                "CO2 air assist needs an assigned HighAir valve".into(),
            ));
        }
        if self.request.manual_focus {
            hardware.z_enabled = false;
            hardware.crash_height = None;
        }
        Ok(Some(hardware))
    }

    fn smooth(&self) -> Result<bool> {
        let smooth = self.process() && self.a.enabled("EnableSmoothPierce")?;
        if smooth && self.a.enabled("PreDrill")? {
            return Err(Error::Unsupported(
                "Separate pre-piercing is incompatible with smooth piercing".into(),
            ));
        }
        Ok(smooth)
    }

    fn pre_pierce(&self, stages_present: bool) -> Result<Option<PrePierce>> {
        if !self.process() || !self.a.enabled("PreDrill")? || !stages_present {
            return Ok(None);
        }
        let gp = Group::read(self.bundle, "SoftParam", "GP")?;
        let batch = gp.uint("PreDrillMaxNum", 100_000)?;
        if batch == 0 {
            return Err(gp.invalid("PreDrillMaxNum", "PreDrillMaxNum must be positive"));
        }
        Ok(Some(PrePierce {
            batch: batch as usize,
            repeat_before_cut: self.a.enabled("AfterPreDrillMustDrillBeforeCut")?,
            keep_down: self.a.enabled("PreDrillIsNotUp")?,
        }))
    }

    fn gas_switch(&self, key: &str) -> Result<bool> {
        if self.process() { self.a.enabled(key) } else { Ok(false) }
    }

    fn slow_start(&self) -> Result<Option<(f64, f64)>> {
        if self.request.dry_run || !self.a.enabled("SlowStart")? {
            return Ok(None);
        }
        Ok(Some((
            self.a.range("SlowStartLength", 0., 100_000.)?,
            self.a.range("SlowStartSpeed", 0.01, 100_000.)?,
        )))
    }

    fn peak_current(&self) -> Result<f64> {
        let laser = Group::read(self.bundle, "LaserParam", "LGP")?;
        if self.request.dry_run || (self.co2() && laser.uint("CO2LaserControlType", 3)? != 3) {
            Ok(0.)
        } else {
            super::whole_percent(self.a, "CutPeakCurrent")
        }
    }

    fn gas(&self) -> Result<(u8, f64)> {
        if self.request.dry_run {
            return Ok((0, 0.));
        }
        if self.co2() {
            return Ok((super::CO2_GAS, 0.));
        }
        Ok((self.a.byte("CutGasType", 5)?, self.a.range("CutAirPressure", 0., 100.)?))
    }

    fn height(&self, off: bool, key: &str) -> Result<f64> {
        if off { Ok(0.) } else { self.a.range(key, 0., 1000.) }
    }

    fn head_speed(&self, off: bool, key: &str) -> Result<f64> {
        if off { Ok(0.) } else { Group::read(self.bundle, "ZFParam", "ZF")?.number(key) }
    }

    fn delay(&self, key: &str) -> Result<u32> {
        if self.request.dry_run { Ok(0) } else { self.a.optional_uint(key, 600_000) }
    }

    fn curve(&self, enabled: &str, nodes: &str) -> Result<Option<Vec<(f64, f64)>>> {
        if self.request.dry_run { Ok(None) } else { self.a.curve(enabled, nodes) }
    }
}

fn require_head(
    hardware: Option<&Hardware>,
    steps: &[PierceStage],
    residue: Option<&Residue>,
) -> Result<()> {
    if hardware.is_some_and(|h| h.z_enabled) {
        return Ok(());
    }
    if residue.is_some() {
        return Err(Error::Unsupported("residue cleaning needs the height controller".into()));
    }
    if !steps.is_empty() {
        return Err(Error::Unsupported("piercing needs the height controller".into()));
    }
    Ok(())
}
