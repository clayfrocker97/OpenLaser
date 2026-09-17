// SPDX-License-Identifier: GPL-3.0-or-later

//! Laser, gas and height-controller bindings, combined only after alias checks.

use super::optional_read;
use crate::attrs::{Group, gas_calibration};
use crate::document::Bundle;
use crate::{Error, Result};
use openlaser_compiler::motion::Analog;
use openlaser_compiler::settings::{GasCalibration, Hardware};
use openlaser_core::LaserMode;

/// The gas hardware. CO2 uses only the high-air valve, without proportional outputs.
#[derive(Default)]
struct Gas {
    ports: [Option<u8>; 6],
    ratio_ports: [Option<u8>; 3],
    ratio_channels: [Option<u8>; 3],
    max_pressure: [f64; 3],
    calibration: [Option<GasCalibration>; 3],
    delays: [u32; 3],
}

impl Gas {
    fn read(bundle: &Bundle, mode: LaserMode) -> Result<Self> {
        let settings = Group::read(bundle, "GasParam", "MGP")?;
        let mut gas = if mode == LaserMode::Co2 {
            let mut gas = Self::default();
            gas.ports[usize::from(super::CO2_GAS)] = settings.port("HighAir", 26)?;
            gas
        } else {
            gas_hardware(bundle, settings)?
        };
        let gc = Group::read(bundle, "ManuParam", "GC")?;
        gas.delays = [
            gc.uint("GasDelay", 600_000)?,
            gc.uint("DirectGasDelay", 600_000)?,
            gc.uint("ChangeGasDelay", 600_000)?,
        ];
        Ok(gas)
    }
}

/// The gas outputs, pressure channels and calibrations: the `GasParam`
/// group, with the calibration switches in the output group and the curves
/// in the input group.
fn gas_hardware(bundle: &Bundle, gas: Group<'_>) -> Result<Gas> {
    let mut out = Gas::default();
    let ports = ["LowAir", "LowO2", "LowN2", "HighAir", "HighO2", "HighN2"];
    for (slot, name) in out.ports.iter_mut().zip(ports) {
        *slot = gas.port(name, 26)?;
    }
    let ratio_ports = ["RatioAirSwitch", "RatioO2Switch", "RatioN2Switch"];
    let ratio_channels = ["RatioAir", "RatioO2", "RatioH2"];
    let max_pressure = ["NewDAMAxPressureAir", "NewDAMAxPressureO2", "NewDAMAxPressureN2"];
    for i in 0..3 {
        out.ratio_ports[i] = gas.port(ratio_ports[i], 26)?;
        out.ratio_channels[i] = gas.port(ratio_channels[i], 2)?;
        out.max_pressure[i] = gas.range(max_pressure[i], 0.001, 100.)?;
    }
    out.calibration = calibrations(bundle)?;
    reject_unbound_maps(gas)?;
    Ok(out)
}

fn calibrations(bundle: &Bundle) -> Result<[Option<GasCalibration>; 3]> {
    let mut out = [None, None, None];
    if let Some(switches) = optional_read(bundle, "DOParam", "DO")? {
        for (i, name) in ["Air", "O2", "N2"].iter().enumerate() {
            if switches.enabled(&format!("Gt{name}EnableGasDAMap"))? {
                let field = format!("Gt{name}GasDAMapStr");
                let calibration = Group::read(bundle, "DIParam", "DO")?;
                let text = calibration
                    .text(&field)
                    .ok_or_else(|| Error::Missing(format!("DIParam.DO.{field}")))?;
                out[i] = gas_calibration(text, "DIParam.DO", &field)?;
            }
        }
    }
    Ok(out)
}

fn reject_unbound_maps(gas: Group<'_>) -> Result<()> {
    for (name, value) in gas.attributes() {
        if (name.contains("Curve") || name.contains("Map")) && !value.is_empty() && value != "0" {
            return Err(Error::Unsupported(format!(
                "calibrated gas setting {name} needs its curve binding"
            )));
        }
    }
    Ok(())
}

/// The analog channel the laser power is written on, under analog control.
/// Stationary writes read the common port even in CO2 mode while motion
/// reads the CO2 port, so both must name the same output.
fn peak_channel(laser: Group<'_>, co2: bool, control: u32) -> Result<Option<u8>> {
    if control != 3 {
        return Ok(None);
    }
    let key = if co2 { "CO2LaserDAPort" } else { "LaserDAPort" };
    let motion = laser.port(key, 2)?;
    if motion.is_none() {
        return Err(Error::Unsupported(format!("analog laser control needs an assigned {key}")));
    }
    let stationary = laser.port("LaserDAPort", 2)?;
    if co2 && stationary != motion {
        return Err(Error::Unsupported(
            "CO2 analog control needs LaserDAPort and CO2LaserDAPort to name the same output"
                .into(),
        ));
    }
    Ok(stationary)
}

struct LaserConfig<'a> {
    a: Group<'a>,
    co2: bool,
    control: u32,
    da: u8,
}

impl<'a> LaserConfig<'a> {
    fn read(bundle: &'a Bundle, mode: LaserMode) -> Result<Self> {
        let a = Group::read(bundle, "LaserParam", "LGP")?;
        let co2 = mode == LaserMode::Co2;
        let control = a.uint(if co2 { "CO2LaserControlType" } else { "LaserControlType" }, 3)?;
        let da = a.byte(if co2 { "CO2LaserDAType" } else { "LaserDAType" }, 2)?;
        Ok(Self { a, co2, control, da })
    }

    fn port(&self, key: &str) -> Result<Option<u8>> {
        let key = if self.co2 { format!("CO2{key}") } else { key.to_owned() };
        self.a.port(&key, 26)
    }

    fn factor(&self) -> Result<f64> {
        Ok([100., 50., 40.][usize::from(self.da)]
            * if !self.co2 && self.a.uint("LaserType", 100)? == 7 { 0.95 } else { 1. })
    }

    fn analog_motion(&self) -> Result<Option<Analog>> {
        if self.co2 && self.control == 3 {
            let channel = self.a.byte("CO2LaserDAPort", 2)?;
            let output = self.a.byte("CO2DOLaser", 10)?;
            Ok(Some(Analog::new(channel, output, self.da)?))
        } else {
            Ok(None)
        }
    }
}

fn crash_height(mp: Group<'_>, enabled: bool) -> Result<Option<u32>> {
    if enabled && mp.enabled("EnableManuCrashProtect")? {
        Ok(crate::whole(mp.range("ManuCrashProtectUpHeight", 0., 1000.)?.trunc()))
    } else {
        Ok(None)
    }
}

pub(super) fn read(bundle: &Bundle, mode: LaserMode) -> Result<Hardware> {
    let mp = Group::read(bundle, "ManuParam", "MP")?;
    let gas = Gas::read(bundle, mode)?;
    let laser = LaserConfig::read(bundle, mode)?;
    if !matches!(mp.text("CO2PWMOutputSync").unwrap_or("1"), "0" | "1") {
        return Err(mp.invalid("CO2PWMOutputSync", "must be 0 or 1"));
    }
    let peak_channel = peak_channel(laser.a, laser.co2, laser.control)?;
    let z_enabled = Group::read(bundle, "ZFParam", "ZF")?.uint("ZFType", 10)? != 0
        && (!laser.co2 || !mp.enabled("CO2DisabledZAxis")?);
    let secondary_pwm = laser.co2 && laser.control == 2;
    // Gas selection clears every pressure channel. Sharing one would clear laser power.
    if peak_channel.is_some_and(|channel| gas.ratio_channels.contains(&Some(channel))) {
        return Err(Error::Unsupported(
            "laser power and gas pressure cannot share an analog output".into(),
        ));
    }
    Ok(Hardware {
        analog_motion: laser.analog_motion()?,
        gas_enabled: true,
        gas_ports: gas.ports,
        ratio_ports: gas.ratio_ports,
        ratio_channels: gas.ratio_channels,
        gas_max_pressure: gas.max_pressure,
        gas_calibration: gas.calibration,
        gate: laser.port("DOLaserGate")?,
        laser: laser.port("DOLaser")?,
        peak_channel,
        peak_factor: laser.factor()?,
        keep_peak_output: mp.enabled("LaserDAKeepOutput")?,
        secondary_pwm,
        co2_contour_control: secondary_pwm,
        z_enabled,
        direct_drill_max: mp.range("DirectDrillMaxHeight", 0., 1000.)?,
        gas_delay_ms: gas.delays[0],
        first_gas_delay_ms: gas.delays[1],
        change_gas_delay_ms: gas.delays[2],
        crash_height: crash_height(mp, z_enabled)?,
        crash_resume_value: mp.optional_uint("EnableManuCrashProtectMinHeight", 1000)?,
    })
}
