// SPDX-License-Identifier: GPL-3.0-or-later

//! Rapid travel between contours, MainApp `0x0045_F5D0`: each axis gets its
//! own profile from rest to rest, sampled at the cadence and normalised, and
//! the shorter axis holds its final coordinate while the longer one finishes.

use crate::motion::Plan;
use crate::planner::profile::{Input, construct};
use crate::planner::sample;
use crate::{Error, Point, Result, to_u32};

/// Travel motion settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Travel speed in mm/s.
    pub speed: f64,
    /// Travel acceleration in mm/s².
    pub acceleration: f64,
    /// Acceleration time in seconds.
    pub acceleration_time: f64,
    /// The interpolation interval in milliseconds.
    pub cadence_ms: f64,
}

/// Whether an axis moves at all: the producer clears its samples for
/// lengths under 0.1 µm or over a kilometre.
#[must_use]
pub fn axis_in_domain(length: f64) -> bool {
    (0.0001..=1_000_000.).contains(&length)
}

/// The travel duration in whole milliseconds, CADModule `0x100E_8470`: the
/// unsampled profile phases summed and truncated. Head planning uses this,
/// not the interpolation cadence.
pub fn duration_millis(length: f64, speed: f64, acceleration: f64, jerk: f64) -> Result<u32> {
    if !length.is_finite() || length < 0. {
        return Err(Error::Invalid("travel length must be finite and nonnegative"));
    }
    if !axis_in_domain(length) {
        return Ok(0);
    }
    let profile = construct(Input {
        length,
        entry: 0.,
        ceiling: speed,
        exit: 0.,
        acceleration,
        jerk,
        span: [0, 0],
    })?;
    // The phases are summed in the vendor's order: acceleration first, then
    // the entry jerk phases, then the rest.
    let d = profile.durations();
    let millis = ((((((d[1] + d[0]) + d[2]) + d[3]) + d[4]) + d[5]) + d[6]) * 1000.;
    if !millis.is_finite() || millis < 0. || millis > f64::from(i32::MAX) {
        return Err(Error::Invalid("travel duration overflows"));
    }
    to_u32(millis)
}

/// One axis's samples as fractions of its length, normalised so the last
/// sample is exactly one. Empty when the axis does not move.
pub fn normalized_axis(length: f64, s: Settings, max_samples: usize) -> Result<Vec<f64>> {
    if !length.is_finite() || length < 0. {
        return Err(Error::Invalid("travel length must be finite and nonnegative"));
    }
    if ![s.speed, s.acceleration, s.acceleration_time, s.cadence_ms]
        .iter()
        .all(|v| v.is_finite() && *v > 0.)
        || !(0.001..=10000.).contains(&s.speed)
        || s.acceleration < 5.
    {
        return Err(Error::Invalid("travel settings are outside the profile domain"));
    }
    if !axis_in_domain(length) {
        return Ok(Vec::new());
    }
    let profile = construct(Input {
        length,
        entry: 0.,
        ceiling: s.speed,
        exit: 0.,
        acceleration: s.acceleration,
        jerk: (2. * s.acceleration) / s.acceleration_time,
        span: [0, 0],
    })?;
    let samples = sample(std::slice::from_ref(&profile), s.cadence_ms, max_samples)?;
    if samples.len() < 2 {
        return Err(Error::Invalid("a travel profile has fewer than two samples"));
    }
    // The producer divides by the length first, then by the final fraction.
    let normalizer = 1. / (samples[samples.len() - 1] / length);
    if !normalizer.is_finite() || normalizer <= 0. {
        return Err(Error::Invalid("travel normalisation is singular"));
    }
    Ok(samples.iter().map(|d| (d / length) * normalizer).collect())
}

/// The sampled travel from `start` to `end`.
pub fn plan(start: Point, end: Point, s: Settings, max_samples: usize) -> Result<Plan> {
    if !start.iter().chain(&end).all(|v| v.is_finite()) {
        return Err(Error::Invalid("travel coordinates must be finite"));
    }
    let delta = [end[0] - start[0], end[1] - start[1]];
    let axes = [
        normalized_axis(delta[0].abs(), s, max_samples)?,
        normalized_axis(delta[1].abs(), s, max_samples)?,
    ];
    let count = axes[0].len().max(axes[1].len());
    if count < 2 {
        return Err(Error::Invalid("travel has no movement samples"));
    }
    let mut points = Vec::with_capacity(count);
    let mut distances = Vec::with_capacity(count);
    let mut distance = 0.;
    let mut previous = start;
    for n in 0..count {
        let mut point = start;
        for axis in 0..2 {
            // The shorter axis keeps its last coordinate; the producer leaves
            // its remaining records at zero increment. An idle axis stays put.
            if !axes[axis].is_empty() {
                let i = n.min(axes[axis].len() - 1);
                point[axis] = axes[axis][i] * delta[axis] + start[axis];
            }
        }
        distance += (point[0] - previous[0]).hypot(point[1] - previous[1]);
        points.push(point);
        distances.push(distance);
        previous = point;
    }
    Ok(Plan { interval: s.cadence_ms * 0.001, distances: distances.into(), points: points.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings { speed: 250., acceleration: 6000., acceleration_time: 0.25, cadence_ms: 0.25 }
    }

    /// Each axis runs its own profile, so a 2:1 diagonal is not a straight
    /// line: halfway through, the short axis is further along its fraction.
    #[test]
    fn axes_are_independent() {
        let plan = plan([0., 0.], [20., 10.], settings(), 1_000_000).unwrap();
        let middle = plan.points[plan.points.len() / 2];
        assert!((middle[1] - middle[0] * 0.5).abs() > 0.1);
        assert_eq!(plan.points[plan.points.len() - 1], [20., 10.]);
        assert_eq!(plan.interval, 0.00025);
    }

    /// An axis under the domain floor contributes no samples and the point
    /// keeps that coordinate; a travel with no moving axis is refused.
    #[test]
    fn idle_axes_stay_put() {
        let plan = plan([1., 2.], [1., 12.], settings(), 1_000_000).unwrap();
        assert!(plan.points.iter().all(|p| p[0] == 1.));
        assert!(super::plan([1., 2.], [1., 2.], settings(), 1_000_000).is_err());
    }

    /// The whole-millisecond duration is zero below the domain floor and
    /// truncates the summed phases above it.
    #[test]
    fn duration_truncates_to_milliseconds() {
        assert_eq!(duration_millis(0., 250., 6000., 48000.).unwrap(), 0);
        let millis = duration_millis(100., 250., 6000., 48000.).unwrap();
        assert!(millis > 400 && millis < 700);
    }
}
