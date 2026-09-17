// SPDX-License-Identifier: GPL-3.0-or-later

//! The frame: the head traces the job's bounds with the laser off, MainApp
//! `0x0045_A180` through CADModule `0x1010_78A0`. From where the head is
//! to the nearest corner, around the rectangle, back to that corner and
//! home again. Each leg is one profile over its Manhattan length, sampled
//! at the cadence and normalised, its fractions applied to both axes at
//! once; the whole rectangle ends in a single barrier.

use crate::motion::Movement;
use crate::planner::profile::{Input, construct};
use crate::planner::sample;
use crate::program::{Builder, Kind, Program};
use crate::{Error, Point, Result, float, to_i32};
use openlaser_core::geometry::Bounds;
use openlaser_protocol::records::{PulsedFields, Record};

/// The machine's frame kinematics, from its parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The frame speed in mm/s, `BoundSpeed` under the machine's ceiling.
    pub speed: f64,
    /// The rapid acceleration under the machine's ceiling.
    pub acceleration: f64,
    /// The jerk: twice the acceleration over the acceleration time.
    pub jerk: f64,
    /// The interpolation interval in milliseconds.
    pub cadence_ms: f64,
    /// Encoder counts per millimetre on each axis.
    pub counts_per_mm: [f64; 2],
}

/// The route: from `current` to the nearest corner by Manhattan distance,
/// around the rectangle, back to that corner, then home.
#[must_use]
pub fn route(bounds: Bounds, current: Point) -> [Point; 7] {
    let (lo, hi): (Point, Point) = (bounds.min.into(), bounds.max.into());
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
    let distance = |p: Point| (p[0] - current[0]).abs() + (p[1] - current[1]).abs();
    let mut nearest = 0;
    for i in 1..4 {
        if distance(corners[i]) < distance(corners[nearest]) {
            nearest = i;
        }
    }
    let corner = |i: usize| corners[(nearest + i) % 4];
    [current, corner(0), corner(1), corner(2), corner(3), corner(0), current]
}

/// The frame program around `bounds` from `current`, in job coordinates.
pub fn program(bounds: Bounds, current: Point, s: &Settings) -> Result<Program> {
    check_bounds(bounds, current)?;
    s.validate()?;
    let mut program = Builder::default();
    let mut residual = [0.; 2];
    for pair in route(bounds, current).windows(2) {
        let delta = [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]];
        let length = delta[0].abs() + delta[1].abs();
        if length < 0.005 {
            continue;
        }
        let movement = leg(delta, length, s, program.motion_points(), &mut residual)?;
        program.motion(Kind::Frame, None, movement, pair.into())?;
    }
    // One barrier closes the complete rectangle.
    program.append(Kind::EndBarrier, None, vec![Record::Barrier], 0.)?;
    let program = program.finish();
    if program.samples == 0 {
        return Err(Error::Invalid("the frame has no movement"));
    }
    Ok(program)
}

fn check_bounds(bounds: Bounds, current: Point) -> Result<()> {
    if !(bounds.min.is_finite() && bounds.max.is_finite() && current.iter().all(|v| v.is_finite()))
        || bounds.min.x >= bounds.max.x
        || bounds.min.y >= bounds.max.y
    {
        return Err(Error::Invalid("the frame needs finite bounds with extent"));
    }
    Ok(())
}

impl Settings {
    fn validate(&self) -> Result<()> {
        let s = self;
        if !(0.001..=10000.).contains(&s.speed)
            || s.acceleration < 5.
            || !s.jerk.is_finite()
            || s.jerk <= 0.
            || !(s.cadence_ms.is_finite() && s.cadence_ms > 0.)
        {
            return Err(Error::Invalid("the frame kinematics are outside the profile domain"));
        }
        Ok(())
    }
}

fn leg(
    delta: Point,
    length: f64,
    s: &Settings,
    budget: usize,
    residual: &mut [f64; 2],
) -> Result<Movement> {
    if !(0.0001..=1_000_000.).contains(&length) {
        return Err(Error::Invalid("a frame leg is outside the profile domain"));
    }
    let profile = construct(Input {
        length,
        entry: 0.,
        ceiling: s.speed,
        exit: 0.,
        acceleration: s.acceleration,
        jerk: s.jerk,
        span: [0, 0],
    })?;
    let samples = sample(std::slice::from_ref(&profile), s.cadence_ms, budget)?;
    if samples.len() < 2 {
        return Err(Error::Invalid("a frame leg has fewer than two samples"));
    }
    // The producer divides each sample by the length, then by the last
    // fraction, and never appends a synthetic end.
    let normalizer = 1. / (samples[samples.len() - 1] / length);
    if !normalizer.is_finite() || normalizer <= 0. {
        return Err(Error::Invalid("the frame normalisation is singular"));
    }
    let mut records = Vec::with_capacity(samples.len() - 1);
    let mut quantizer = FrameSteps::new(s.counts_per_mm, *residual);
    for value in samples.iter().skip(1) {
        let fraction = (value / length) * normalizer;
        let increment = quantizer.step(delta, fraction)?;
        // The template clears the process fields; a zero frequency
        // becomes 5000 Hz; the power stays zero throughout.
        records.push(Record::pulsed(
            increment[0],
            increment[1],
            PulsedFields { power: 0, frequency: 5000, tail: 0 },
        ));
    }
    *residual = quantizer.residual;
    Ok(Movement {
        steps: records.len(),
        seconds: float(records.len()) * s.cadence_ms * 0.001,
        residuals: *residual,
        records,
    })
}

struct FrameSteps {
    scales: [f64; 2],
    previous: [i32; 2],
    residual: [f64; 2],
}

impl FrameSteps {
    fn new(counts: [f64; 2], residual: [f64; 2]) -> Self {
        Self { scales: counts.map(|count| count / 10_000.), previous: [0; 2], residual }
    }

    fn step(&mut self, delta: Point, fraction: f64) -> Result<[i8; 2]> {
        let mut increment = [0; 2];
        for axis in 0..2 {
            increment[axis] = self.axis(axis, delta[axis] * fraction)?;
        }
        Ok(increment)
    }

    fn axis(&mut self, axis: usize, displacement: f64) -> Result<i8> {
        let q = to_i32((displacement * 10_000.).round())?;
        let dq = q.checked_sub(self.previous[axis]).ok_or(Error::Invalid("frame overflow"))?;
        let raw = f64::from(dq) * self.scales[axis] + self.residual[axis];
        if !raw.is_finite() || raw.trunc() < f64::from(i8::MIN) || raw.trunc() > f64::from(i8::MAX)
        {
            return Err(Error::Invalid(
                "a frame step exceeds one signed byte at this speed and cadence",
            ));
        }
        let step = to_i32(raw.trunc())?;
        let increment = i8::try_from(step).map_err(|_| Error::Invalid("frame step"))?;
        self.residual[axis] = raw - f64::from(step);
        self.previous[axis] = q;
        Ok(increment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From below the rectangle the head goes to its nearest corner, once
    /// around and home: every step is a laser-off move at 5 kHz, the steps
    /// sum back to the start on both axes, and one barrier ends it.
    #[test]
    fn the_frame_walks_the_rectangle_with_the_laser_off() {
        let bounds = Bounds { min: [0., 0.].into(), max: [100., 60.].into() };
        let current = [90., -10.];
        let legs = route(bounds, current);
        assert_eq!(legs[1], [100., 0.], "the nearest corner");
        assert_eq!(legs[5], legs[1]);
        assert_eq!(legs[6], current);
        // The machine's own numbers: 500 mm/s, 250 us cycles, 8000 pulses
        // over 31.04 mm per turn, so a full-speed step is 32 counts.
        let settings = Settings {
            speed: 500.,
            acceleration: 5000.,
            jerk: 100_000.,
            cadence_ms: 0.25,
            counts_per_mm: [8000. / 31.04, 8000. / 31.04],
        };
        let program = program(bounds, current, &settings).unwrap();
        let (mut sx, mut sy) = (0i32, 0i32);
        for record in &program.records[..program.records.len() - 1] {
            let Record::Move { dx, dy, laser } = record else { panic!("{record:?}") };
            assert_eq!(
                Record::pulsed_fields(*laser),
                PulsedFields { power: 0, frequency: 5000, tail: 0 }
            );
            sx += i32::from(*dx);
            sy += i32::from(*dy);
        }
        assert!(sx.abs() <= 1 && sy.abs() <= 1, "back where it started: {sx} {sy}");
        assert_eq!(program.records.last(), Some(&Record::Barrier));
        assert_eq!(program.sections.len(), 7, "six legs and the barrier");
        assert!((program.seconds - float(program.samples) * 0.00025).abs() < 1e-9);
        let short = Bounds { min: [0., 0.].into(), max: [0., 60.].into() };
        assert!(program_err(short, current, &settings));
    }

    fn program_err(bounds: Bounds, current: Point, s: &Settings) -> bool {
        program(bounds, current, s).is_err()
    }
}
