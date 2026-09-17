// SPDX-License-Identifier: GPL-3.0-or-later

//! From sampled points to move records: the vendor's recordizer, MainApp
//! `0x0045_84A0` with the residual quantiser `0x0046_CE20`.
//!
//! Each point is scaled by 10 000, the previous scaled point subtracted, the
//! axis scale applied, the carried residual added and the sum truncated
//! toward zero into a signed byte; the fraction becomes the next residual. A
//! step that does not fit a byte is an error, never a reason to insert a
//! sample: the cadence belongs to the planner.

use crate::{Error, Point, Result, finite, float, to_i32};
use openlaser_protocol::records::{AnalogFields, PulsedFields, Record};
use std::sync::Arc;

/// A sampled path: one point per interpolation interval.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The interpolation interval in seconds.
    pub interval: f64,
    /// Cumulative distance of each sample.
    pub distances: Arc<[f64]>,
    /// The coordinates of each sample.
    pub points: Arc<[Point]>,
}

/// The process fields of one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The analog power percentage before edge overrides, kept for analog
    /// programs.
    pub analog: u8,
    /// PWM power in percent.
    pub power: u8,
    /// PWM frequency in hertz.
    pub frequency: u16,
    /// The trailing field: `0xFFFF` from the cut producer, zero elsewhere.
    pub tail: u16,
}

/// How a CO2 analog program packs its steps, NCModule `0x1004_5663`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Analog {
    channel: u8,
    port: u8,
    units_per_percent: u16,
}

impl Analog {
    /// Analog channel 1 or 2, laser output port 1 to 10, and the analog
    /// type selecting 100, 50 or 40 units per percent.
    pub fn new(channel: u8, port: u8, da_type: u8) -> Result<Self> {
        if !(1..=2).contains(&channel) || !(1..=10).contains(&port) {
            return Err(Error::Invalid(
                "analog motion needs channel 1..=2 and laser output 1..=10",
            ));
        }
        let units_per_percent = *[100, 50, 40]
            .get(usize::from(da_type))
            .ok_or(Error::Invalid("analog type must be 0, 1 or 2"))?;
        Ok(Self { channel, port, units_per_percent })
    }

    /// One step. Power and the laser gate are independent fields: joints
    /// change the gate without changing the level.
    fn record(self, dx: i8, dy: i8, sample: Sample) -> Result<Record> {
        if sample.analog > 100 {
            return Err(Error::Invalid("an analog sample exceeds 100 percent"));
        }
        let port =
            i8::try_from(self.port).map_err(|_| Error::Invalid("laser output exceeds a byte"))?;
        Ok(Record::analog(
            dx,
            dy,
            AnalogFields {
                channel: self.channel - 1,
                level: u16::from(sample.analog) * self.units_per_percent,
                output: if sample.power != 0 { port } else { -port },
                tail: sample.tail,
            },
        ))
    }
}

/// Which move record a program uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Power and frequency per step.
    Pulsed,
    /// An analog level and a laser gate per step.
    Analog(Analog),
}

/// The residual quantiser: state carried from step to step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantizer {
    scales: [f64; 2],
    residuals: [f64; 2],
    previous: [f64; 2],
}

impl Quantizer {
    /// Starts at `start` with the axis scales (counts per 10 000th of a
    /// millimetre) and the residuals carried in.
    pub fn new(start: Point, scales: [f64; 2], residuals: [f64; 2]) -> Result<Self> {
        if scales.iter().chain(&residuals).any(|v| !v.is_finite()) {
            return Err(Error::Invalid("axis scales and residuals must be finite"));
        }
        let mut previous = [0.; 2];
        for axis in 0..2 {
            previous[axis] = finite(finite(start[axis])? * 10_000.)?;
        }
        Ok(Self { scales, residuals, previous })
    }

    /// The residuals to carry into the next movement.
    #[must_use]
    pub const fn residuals(&self) -> [f64; 2] {
        self.residuals
    }

    /// The increments to `point`. Both axes are checked before any state
    /// changes, so a rejected point leaves the quantiser as it was.
    pub fn step(&mut self, point: Point) -> Result<(i8, i8)> {
        let mut current = [0.; 2];
        let mut residuals = [0.; 2];
        let mut quantized = [0i8; 2];
        for axis in 0..2 {
            current[axis] = finite(finite(point[axis])? * 10_000.)?;
            let delta = finite(current[axis] - self.previous[axis])?;
            let adjusted = finite(finite(delta * self.scales[axis])? + self.residuals[axis])?;
            let truncated = adjusted.trunc();
            if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
                return Err(Error::Invalid("a step exceeds the signed 32-bit domain"));
            }
            let value = to_i32(truncated)?;
            quantized[axis] = i8::try_from(value)
                .map_err(|_| Error::Invalid("a step exceeds one signed byte at this cadence"))?;
            residuals[axis] = finite(adjusted - f64::from(value))?;
        }
        self.previous = current;
        self.residuals = residuals;
        Ok((quantized[0], quantized[1]))
    }
}

/// A framed movement: its records, ending in a barrier, and the residuals
/// carried out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Movement {
    /// The move records followed by the barrier.
    pub records: Vec<Record>,
    /// The residuals after the last step.
    pub residuals: [f64; 2],
    /// The number of steps.
    pub steps: usize,
    /// The movement's duration in seconds.
    pub seconds: f64,
}

/// Frames `plan` with one process `sample` per step, starting from the
/// carried `residuals`, in the program's `output` encoding.
pub fn frame(
    plan: &Plan,
    samples: &[Sample],
    scales: [f64; 2],
    residuals: [f64; 2],
    output: Output,
) -> Result<Movement> {
    check_frame(plan, samples, output)?;
    let mut quantizer = Quantizer::new(plan.points[0], scales, residuals)?;
    let mut records = Vec::with_capacity(samples.len() + 1);
    for (point, sample) in plan.points[1..].iter().zip(samples) {
        let (dx, dy) = quantizer.step(*point)?;
        records.push(match output {
            Output::Pulsed => Record::pulsed(
                dx,
                dy,
                PulsedFields {
                    power: sample.power,
                    frequency: sample.frequency,
                    tail: sample.tail,
                },
            ),
            Output::Analog(analog) => analog.record(dx, dy, *sample)?,
        });
    }
    records.push(Record::Barrier);
    Ok(Movement {
        records,
        residuals: quantizer.residuals(),
        steps: samples.len(),
        seconds: plan.interval * float(samples.len()),
    })
}

fn check_frame(plan: &Plan, samples: &[Sample], output: Output) -> Result<()> {
    if samples.len() != plan.points.len().saturating_sub(1) {
        return Err(Error::Invalid("a movement needs one process sample per step"));
    }
    if output == Output::Pulsed && samples.iter().any(|s| s.power > 100) {
        return Err(Error::Invalid("cutting power must be 0 to 100 percent"));
    }
    if !plan.interval.is_finite() || plan.interval <= 0. {
        return Err(Error::Invalid("the movement interval must be finite and positive"));
    }
    if plan.distances.len() != plan.points.len()
        || plan.distances.iter().any(|v| !v.is_finite() || *v < 0.)
        || plan.distances.windows(2).any(|w| w[1] < w[0])
    {
        return Err(Error::Invalid("a movement needs matching, ordered distance samples"));
    }
    if plan.points.len() < 2 || plan.points.len() > 1_000_000 {
        return Err(Error::Invalid("a movement needs 2 to 1 000 000 sampled points"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        let points: Vec<Point> = (0..=10).map(|i| [f64::from(i) * 0.03, 0.]).collect();
        let distances = points.iter().map(|p| p[0]).collect();
        Plan { interval: 0.001, distances, points: points.into() }
    }

    fn samples(count: usize) -> Vec<Sample> {
        vec![Sample { analog: 37, power: 37, frequency: 12345, tail: 0xffff }; count]
    }

    /// With 1000 counts per millimetre a 0.03 mm step is 30 pulses, and
    /// the residual stays below one pulse throughout.
    #[test]
    fn steps_quantise_with_carried_residuals() {
        let movement = frame(&plan(), &samples(10), [0.1, 0.1], [0., 0.], Output::Pulsed).unwrap();
        assert_eq!(movement.steps, 10);
        assert_eq!(movement.records.len(), 11);
        assert_eq!(movement.records[10], Record::Barrier);
        let Record::Move { dx, dy, laser } = movement.records[0] else { panic!("expected a move") };
        assert_eq!((dx, dy), (30, 0));
        assert_eq!(
            Record::pulsed_fields(laser),
            PulsedFields { power: 37, frequency: 12345, tail: 0xffff }
        );
        assert!(movement.residuals.iter().all(|r| r.abs() < 1.));
        assert!((movement.seconds - 0.01).abs() < 1e-12);
    }

    /// The vendor's scale of 257.73 counts per millimetre leaves a fraction
    /// on every step, and the carried residual makes up for it: ten steps
    /// of 0.03 mm sum to the exact pulse count, truncated.
    #[test]
    fn residuals_conserve_the_total_distance() {
        let scale = 8000. / 31.04 / 10_000.;
        let movement = frame(&plan(), &samples(10), [scale; 2], [0., 0.], Output::Pulsed).unwrap();
        let total: i32 = movement
            .records
            .iter()
            .filter_map(
                |r| if let Record::Move { dx, .. } = r { Some(i32::from(*dx)) } else { None },
            )
            .sum();
        assert_eq!(total, 77);
    }

    /// An analog program packs the level, the signed gate port and the tail,
    /// with the gate negative when power is zero.
    #[test]
    fn analog_steps_pack_level_and_gate() {
        let analog = Analog::new(2, 9, 1).unwrap();
        let mut samples = samples(10);
        samples[0].power = 0;
        let movement = frame(&plan(), &samples, [0.1; 2], [0.; 2], Output::Analog(analog)).unwrap();
        let Record::Move { laser, .. } = movement.records[0] else { panic!("expected a move") };
        let fields = Record::analog_fields(laser);
        assert_eq!(
            (fields.channel, fields.level, fields.output, fields.tail),
            (1, 37 * 50, -9, 0xffff)
        );
        let Record::Move { laser, .. } = movement.records[1] else { panic!("expected a move") };
        assert_eq!(Record::analog_fields(laser).output, 9);
        assert!(Analog::new(3, 9, 1).is_err());
        assert!(Analog::new(1, 9, 3).is_err());
    }

    /// A step that does not fit a signed byte is an error, not a cadence
    /// change, and a sample count that does not match the points is refused.
    #[test]
    fn overflow_and_mismatch_are_refused() {
        assert!(frame(&plan(), &samples(10), [10.; 2], [0.; 2], Output::Pulsed).is_err());
        assert!(frame(&plan(), &samples(9), [0.1; 2], [0.; 2], Output::Pulsed).is_err());
        let mut broken = plan();
        broken.interval = f64::NAN;
        assert!(frame(&broken, &samples(10), [0.1; 2], [0.; 2], Output::Pulsed).is_err());
    }
}
