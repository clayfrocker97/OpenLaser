// SPDX-License-Identifier: GPL-3.0-or-later

//! What the compiler needs to know about the machine and the recipe.
//!
//! These are the vendor's own parameters, validated and converted to the
//! units the producers use, named for what they control rather than for the
//! XML attribute they came from. The XML crate fills them from a machine
//! backup; our own recipes fill the same structs. Ports and analog channels
//! are one-based, as on the machine's labels; zero in the vendor's XML means
//! unassigned and becomes `None` here.

use crate::motion::Analog;
use crate::{Error, Result};
use openlaser_core::LaserMode;

/// Host-side timeouts and the analog floor, from the `[Soft]` section of
/// the vendor host's `ipAdd.ini`. The defaults are the host's own when the
/// file leaves them unset; a machine's file usually sets them higher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// How long a follow or retract may take before the program aborts.
    pub follow_ms: u32,
    /// How long a pierce-height move may take.
    pub section_drill_ms: u32,
    /// The smallest nonzero analog peak request the host emits.
    pub analog_minimum: u32,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { follow_ms: 3000, section_drill_ms: 3000, analog_minimum: 50 }
    }
}

/// A gas pressure calibration: output voltage against pressure.
#[derive(Clone, Debug, PartialEq)]
pub struct GasCalibration {
    points: Vec<(f64, f64)>,
}

impl GasCalibration {
    /// From `(voltage, pressure)` pairs in any order, MainApp `0x004D_69CD`.
    pub fn new(mut points: Vec<(f64, f64)>) -> Result<Self> {
        if points.is_empty() || points.len() > 2048 {
            return Err(Error::Invalid("a gas calibration needs 1 to 2048 points"));
        }
        if points.iter().any(|(voltage, pressure)| {
            !voltage.is_finite()
                || !(0. ..=10.).contains(voltage)
                || !pressure.is_finite()
                || *pressure < 0.
        }) {
            return Err(Error::Invalid(
                "gas calibration points need 0 to 10 volts and nonnegative pressure",
            ));
        }
        points.sort_by(|a, b| a.1.total_cmp(&b.1));
        if points.windows(2).any(|w| w[0].1 == w[1].1) {
            return Err(Error::Invalid("gas calibration has duplicate pressure points"));
        }
        Ok(Self { points })
    }

    /// The output voltage for `pressure`, MainApp `0x0062_FC60`: a slope
    /// through the origin below the first point, interpolation between
    /// points, and the last slope beyond.
    pub fn voltage(&self, pressure: f64) -> Result<f64> {
        if !pressure.is_finite() || pressure < 0. {
            return Err(Error::Invalid("gas pressure must be finite and nonnegative"));
        }
        let first = self.points[0];
        let value = if pressure <= first.1 || self.points.len() == 1 {
            if first.1 == 0. {
                if pressure == 0. {
                    first.0
                } else {
                    return Err(Error::Invalid(
                        "a single zero-pressure calibration point has no slope",
                    ));
                }
            } else {
                pressure * first.0 / first.1
            }
        } else {
            let upper =
                self.points.partition_point(|point| point.1 < pressure).min(self.points.len() - 1);
            let a = self.points[upper - 1];
            let b = self.points[upper];
            a.0 + (pressure - a.1) * (b.0 - a.0) / (b.1 - a.1)
        };
        if !value.is_finite() || !(0. ..=10.).contains(&value) {
            return Err(Error::Invalid("calibrated gas output exceeds 0 to 10 volts"));
        }
        Ok(value)
    }
}

/// The process outputs the machine is wired with.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "the vendor's wiring flags, as they are")]
pub struct Hardware {
    /// The analog packing of a CO2 program under analog laser control.
    pub analog_motion: Option<Analog>,
    /// Whether the process switches its configured assist-gas valve.
    pub gas_enabled: bool,
    /// Main valve ports: low air, low oxygen, low nitrogen, high air, high
    /// oxygen, high nitrogen.
    pub gas_ports: [Option<u8>; 6],
    /// Proportional valve ports for air, oxygen, nitrogen.
    pub ratio_ports: [Option<u8>; 3],
    /// Proportional pressure analog channels for air, oxygen, nitrogen.
    pub ratio_channels: [Option<u8>; 3],
    /// Full-scale pressure of each proportional channel.
    pub gas_max_pressure: [f64; 3],
    /// Calibration curves for the proportional channels, when configured.
    pub gas_calibration: [Option<GasCalibration>; 3],
    /// The laser gate output.
    pub gate: Option<u8>,
    /// The laser enable output.
    pub laser: Option<u8>,
    /// The analog channel carrying the peak current or power.
    pub peak_channel: Option<u8>,
    /// Analog units per percent for the peak channel.
    pub peak_factor: f64,
    /// Whether the peak output is left on when the program ends.
    pub keep_peak_output: bool,
    /// Whether PWM goes to the secondary channel (CO2 control type 2).
    pub secondary_pwm: bool,
    /// Whether contours are bracketed by the CO2 contour control records,
    /// as CO2 control type 2 requires.
    pub co2_contour_control: bool,
    /// Whether the height controller is present and used.
    pub z_enabled: bool,
    /// The tallest pierce height the head reaches directly.
    pub direct_drill_max: f64,
    /// Delay after every gas switch.
    pub gas_delay_ms: u32,
    /// Extra delay before the first gas of a program.
    pub first_gas_delay_ms: u32,
    /// Extra delay when the gas changes.
    pub change_gas_delay_ms: u32,
    /// Crash protection height, when enabled.
    pub crash_height: Option<u32>,
    /// The value written to resume crash protection.
    pub crash_resume_value: u32,
}

/// One piercing stage of the recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct PierceStage {
    /// The vendor's stage index; execution runs the highest active index first.
    pub index: u8,
    /// Pierce height in millimetres.
    pub height: f64,
    /// Power in percent.
    pub power: u8,
    /// Frequency in hertz.
    pub frequency: u16,
    /// Peak current in percent.
    pub peak_current: f64,
    /// Gas selector, 0 to 5.
    pub gas: u8,
    /// Gas pressure.
    pub pressure: f64,
    /// Dwell at this stage in milliseconds.
    pub duration_ms: u32,
    /// Whether the head descends gradually to the next stage.
    pub gradual: bool,
    /// The gradual descent time in milliseconds.
    pub gradual_ms: u32,
    /// Delay before the laser is switched off.
    pub before_off_ms: u32,
    /// Delay after the laser is switched off.
    pub after_off_ms: u32,
    /// A bolt-drill ramp to this power and frequency, when enabled.
    pub bolt: Option<(u8, u16)>,
}

/// The residue-cleaning pass between piercing and cutting.
#[derive(Clone, Debug, PartialEq)]
pub struct Residue {
    /// Head height during cleaning.
    pub height: f64,
    /// Cleaning speed.
    pub speed: f64,
    /// Gas selector.
    pub gas: u8,
    /// Gas pressure.
    pub pressure: f64,
    /// Peak current in percent.
    pub peak_current: f64,
    /// Power in percent.
    pub power: u8,
    /// Frequency in hertz.
    pub frequency: u16,
    /// Spiral radius.
    pub radius: f64,
    /// Spiral turns.
    pub turns: u32,
}

/// A slow region at the start or end of every contour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// The region's length.
    pub length: f64,
    /// The speed inside it.
    pub speed: f64,
    /// A power and frequency override inside it.
    pub pwm: Option<(u8, u16)>,
}

/// How the head travels between contours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadTravel {
    /// Whether the head jumps rather than retracting fully.
    pub frog_jump: bool,
    /// The lowest jump height, when jumping is active.
    pub minimum_height: Option<f64>,
    /// Whether crash protection is written around every travel.
    pub protect_insertions: bool,
    /// Keep the head down for travels shorter than this.
    pub keep_height_below: Option<f64>,
    /// The pierce height the jump lands at, from the ordinary stage bank.
    pub jump_pierce_height: Option<f64>,
    /// The machine's short-transfer distance (`FC.ShortNoUpMaxLength`),
    /// which the gas retention rule compares strictly against.
    pub short_transfer: Option<f64>,
}

/// Preliminary piercing: contours pierced in batches before they are cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrePierce {
    /// How many contours each batch pierces before the cuts resume.
    pub batch: usize,
    /// Whether the cut pierces again after the preliminary pass.
    pub repeat_before_cut: bool,
    /// Whether the head stays down between the points of one batch.
    pub keep_down: bool,
}

/// A layer's recipe together with the machine settings it runs on.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "the vendor's recipe flags, as they are")]
pub struct Settings {
    /// Which laser.
    pub mode: LaserMode,
    /// The layer bank, 1 to 11.
    pub layer: u8,
    /// A dry run moves without firing and skips every process step.
    pub dry_run: bool,
    /// Encoder counts per millimetre on each axis.
    pub counts_per_mm: [f64; 2],
    /// Cutting acceleration.
    pub acceleration: f64,
    /// Acceleration time in seconds.
    pub acceleration_time: f64,
    /// The controller's interpolation cycle in microseconds.
    pub interpolation_cycle_us: u32,
    /// The recipe's cutting speed.
    pub speed: f64,
    /// The machine's cutting speed ceiling.
    pub maximum_cut_speed: f64,
    /// The controller's own speed ceiling, once its scale is known.
    pub native_speed_cap: Option<f64>,
    /// Corner accuracy in millimetres.
    pub corner_accuracy: f64,
    /// Spline accuracy rate.
    pub spline_accuracy: f64,
    /// The small-circle speed ratio, when the limit is enabled.
    pub small_circle_limit_ratio: Option<f64>,
    /// The recipe's slow start: length and speed.
    pub slow_start: Option<(f64, f64)>,
    /// The leading edge region.
    pub edge_start: Option<Edge>,
    /// The trailing edge region.
    pub edge_end: Option<Edge>,
    /// Cutting power in percent.
    pub power: u8,
    /// Cutting frequency in hertz.
    pub frequency: u16,
    /// Cutting peak current in percent.
    pub peak_current: f64,
    /// Cutting gas selector.
    pub gas: u8,
    /// Cutting gas pressure.
    pub pressure: f64,
    /// Cutting height.
    pub cut_height: f64,
    /// Retract height between contours.
    pub retract_height: f64,
    /// Head lift speed.
    pub z_up_speed: f64,
    /// Head follow speed.
    pub z_follow_speed: f64,
    /// Whether the head cuts at a fixed height.
    pub fixed_height: bool,
    /// An absolute cutting height, for the vendor's fixed-position mode.
    pub absolute_cut_height: Option<f64>,
    /// Whether the head does not follow at all.
    pub no_follow: bool,
    /// The piercing stages in execution order.
    pub pierce: Vec<PierceStage>,
    /// Residue cleaning, when enabled.
    pub residue: Option<Residue>,
    /// Delay after the laser comes on before cutting.
    pub on_delay_ms: u32,
    /// Delay at the contour end before the laser goes off.
    pub off_before_ms: u32,
    /// Delay after the laser goes off.
    pub off_after_ms: u32,
    /// Power against speed, as percentages.
    pub power_curve: Option<Vec<(f64, f64)>>,
    /// Frequency against speed, as percentages.
    pub frequency_curve: Option<Vec<(f64, f64)>>,
    /// The vibration abatement word, combined into follow targets by bitwise or.
    pub vibration_word: u32,
    /// The process hardware, absent in a dry run.
    pub hardware: Option<Hardware>,
    /// Travel speed.
    pub travel_speed: f64,
    /// Travel acceleration.
    pub travel_acceleration: f64,
    /// Travel acceleration time in seconds.
    pub travel_acceleration_time: f64,
    /// The vendor's machining kind: 0 ordinary, 1 fixed height, 5 absolute
    /// height, others select the pierce stage count.
    pub machining_kind: u8,
    /// Whether smooth piercing replaces the ordinary stages.
    pub smooth_pierce: bool,
    /// Head travel between contours.
    pub head_travel: HeadTravel,
    /// Host timeouts.
    pub timeouts: Timeouts,
    /// Preliminary piercing, when the recipe asks for it and has stages.
    pub pre_pierce: Option<PrePierce>,
    /// Whether every contour gets a film pass before its cut.
    pub with_film: bool,
    /// The film batch size; zero means the whole job.
    pub film_batch: usize,
    /// Keep the gas on after a contour (`NoCloseGasInManu`).
    pub keep_gas_after: bool,
    /// Keep the gas on into this contour over a short transfer
    /// (`ShortDistGasKeepOn`).
    pub short_gas_keep: bool,
    /// The older host's contour shift: an XY translation of the prepared
    /// geometry, zero when disabled.
    pub contour_shift: [f64; 2],
}

impl Settings {
    /// The planner's speed cap: the recipe speed under the machine's and the
    /// controller's ceilings.
    #[must_use]
    pub fn planner_speed(&self) -> f64 {
        self.speed.min(self.maximum_cut_speed).min(self.native_speed_cap.unwrap_or(f64::INFINITY))
    }

    /// Binds the controller's coordinate scale, MainApp `0x0045_01E0`: the
    /// speed cap is twice 750 000 over the scale.
    pub fn bind_speed_cap(&mut self, scale: i32) -> Result<()> {
        if scale <= 0 {
            return Err(Error::Invalid("the controller scale must be positive"));
        }
        let half = 750_000. / f64::from(scale);
        self.native_speed_cap = Some(half + half);
        Ok(())
    }

    /// The interpolation interval in milliseconds.
    #[must_use]
    pub fn cadence_ms(&self) -> f64 {
        f64::from(self.interpolation_cycle_us) * 0.001
    }

    /// The quantiser's axis scales.
    #[must_use]
    pub fn scales(&self) -> [f64; 2] {
        [self.counts_per_mm[0] / 10_000., self.counts_per_mm[1] / 10_000.]
    }

    /// The power and frequency at `distance` along a contour of `total`
    /// length: the edge overrides, the trailing one applied last.
    #[must_use]
    pub fn edge_pwm(
        &self,
        distance: f64,
        total: f64,
        mut power: u8,
        mut frequency: u16,
    ) -> (u8, u16) {
        if self.dry_run {
            return (0, frequency);
        }
        if let Some(edge) = &self.edge_start
            && distance < edge.length
            && let Some(pwm) = edge.pwm
        {
            (power, frequency) = pwm;
        }
        if let Some(edge) = &self.edge_end
            && total - distance < edge.length
            && let Some(pwm) = edge.pwm
        {
            (power, frequency) = pwm;
        }
        (power, frequency)
    }
}

pub use openlaser_core::toolpath::{LeadRole, SegmentProcess};

/// The power byte for a sample of `base` under `process`. A joint scales
/// the existing byte with integer truncation, as the vendor's joint writer
/// does; any other override replaces it.
pub fn sample_power(process: &SegmentProcess, base: u8) -> Result<u8> {
    let Some(value) = process.power else { return Ok(base) };
    if !value.is_finite() || !(0. ..=100.).contains(&value) {
        return Err(Error::Invalid("segment power must be 0 to 100 percent"));
    }
    if process.joint {
        if value.fract() != 0. {
            return Err(Error::Invalid("joint power must be a whole percentage"));
        }
        let percent = crate::to_u16(value)?;
        Ok(crate::to_u8(f64::from(u16::from(base) * percent / 100))?)
    } else {
        crate::to_u8(value.round())
    }
}

/// Per-contour overrides of the recipe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    /// `None` inherits the recipe; `Some(None)` disables the slow start.
    pub slow_start: Option<Option<(f64, f64)>>,
    /// A slow end: length and speed.
    pub slow_end: Option<(f64, f64)>,
    /// The laser-on delay.
    pub on_delay_ms: Option<u32>,
    /// The laser-off delay.
    pub off_delay_ms: Option<u32>,
}

impl Overrides {
    /// Resolves against `settings` for a contour of `total` length: the
    /// recipe's edges fill in, and regions that overlap share the length.
    #[must_use]
    pub fn resolved(&self, settings: &Settings, total: f64) -> Self {
        if settings.dry_run {
            return Self::default();
        }
        let mut start = self.slow_start.unwrap_or_else(|| {
            settings.edge_start.map(|e| (e.length, e.speed)).or(settings.slow_start)
        });
        let mut end = self.slow_end.or_else(|| settings.edge_end.map(|e| (e.length, e.speed)));
        if start.map_or(0., |v| v.0) + end.map_or(0., |v| v.0) > total {
            if let Some(v) = &mut start {
                v.0 = total * 0.5;
            }
            if let Some(v) = &mut end {
                v.0 = total * 0.5;
            }
        }
        Self {
            slow_start: Some(start),
            slow_end: end,
            on_delay_ms: self.on_delay_ms,
            off_delay_ms: self.off_delay_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pairs are voltage then pressure and sort by pressure; below the first
    /// point the slope runs through the origin, beyond the last it continues.
    #[test]
    fn calibration_interpolates_by_pressure() {
        let calibration = GasCalibration::new(vec![(8., 12.), (2., 3.), (5., 9.)]).unwrap();
        for (pressure, volts) in
            [(0., 0.), (1.5, 1.), (3., 2.), (6., 3.5), (9., 5.), (12., 8.), (13., 9.)]
        {
            assert!((calibration.voltage(pressure).unwrap() - volts).abs() < 1e-12);
        }
        assert!(calibration.voltage(15.).is_err());
        assert!(GasCalibration::new(vec![(1., 2.), (3., 2.)]).is_err());
    }

    /// A single point defines a slope through the origin, and a single
    /// zero-pressure point only defines its own voltage.
    #[test]
    fn single_points_define_a_slope_or_nothing() {
        let single = GasCalibration::new(vec![(4., 8.)]).unwrap();
        assert_eq!(single.voltage(4.).unwrap(), 2.);
        assert_eq!(single.voltage(12.).unwrap(), 6.);
        let zero = GasCalibration::new(vec![(1., 0.)]).unwrap();
        assert_eq!(zero.voltage(0.).unwrap(), 1.);
        assert!(zero.voltage(1.).is_err());
    }

    /// A joint scales the sample byte with truncation; a plain override
    /// rounds and replaces it; fractional joint power is refused.
    #[test]
    fn joint_power_scales_with_truncation() {
        let joint = SegmentProcess { joint: true, power: Some(20.), ..Default::default() };
        assert_eq!(sample_power(&joint, 35).unwrap(), 7);
        assert_eq!(sample_power(&joint, 34).unwrap(), 6);
        assert_eq!(sample_power(&joint, 0).unwrap(), 0);
        let plain = SegmentProcess { power: Some(20.4), ..Default::default() };
        assert_eq!(sample_power(&plain, 35).unwrap(), 20);
        assert!(sample_power(&SegmentProcess { power: Some(20.5), ..joint }, 35).is_err());
        assert_eq!(sample_power(&SegmentProcess::default(), 35).unwrap(), 35);
    }
}
