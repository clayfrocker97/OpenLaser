// SPDX-License-Identifier: GPL-3.0-or-later

//! Sampling profiles at the interpolation cadence, CADModule `0x1010_94E0`
//! with the driver `0x1010_A0D0`.
//!
//! Each profile is walked phase by phase at a fixed time step. The time left
//! over at the end of one profile is carried into the next so the cadence
//! never resets, and the distance base accumulates across profiles.

use super::Profile;
use super::profile::{
    ACCELERATE, CRUISE, DECELERATE, ENTRY_JERK_IN, ENTRY_JERK_OUT, EXIT_JERK_IN, EXIT_JERK_OUT,
};
use crate::{Error, Result, finite};

/// Samples `profiles` in order at `interval_ms`, returning cumulative
/// distances. `max_samples` bounds the output; the vendor has no such bound.
pub fn sample(profiles: &[Profile], interval_ms: f64, max_samples: usize) -> Result<Vec<f64>> {
    let interval = finite(finite(interval_ms)? * 0.001)?;
    if interval <= 0. {
        return Err(Error::Invalid("the interpolation interval must be positive"));
    }
    for profile in profiles {
        check_phases(profile)?;
    }
    let mut walk = Walk { values: Vec::new(), interval, base: 0., max_samples, last: None };
    let mut carry = -1.;
    for profile in profiles {
        carry = walk.profile(profile, carry)?;
        walk.base = finite(walk.base + profile.length)?;
    }
    Ok(walk.values)
}

struct Walk {
    values: Vec<f64>,
    interval: f64,
    base: f64,
    max_samples: usize,
    last: Option<f64>,
}

impl Walk {
    /// Emits a sample and returns the next sample time.
    fn emit(&mut self, local: f64, time: f64) -> Result<f64> {
        let value = finite(local + self.base)?;
        if self.values.len() >= self.max_samples {
            return Err(Error::Budget("sample budget exhausted"));
        }
        self.values.push(value);
        self.last = Some(time);
        let next = finite(time + self.interval)?;
        if next <= time {
            return Err(Error::Invalid("the sample time does not advance"));
        }
        Ok(next)
    }

    /// Walks one profile and returns the carry into the next. Each phase's
    /// distance is a polynomial in the time since the phase began, or the
    /// time until it ends for the phases that mirror an earlier one.
    fn profile(&mut self, profile: &Profile, carry: f64) -> Result<f64> {
        let shape = ProfileShape::new(profile)?;
        let total = shape.ends[EXIT_JERK_OUT];
        self.last = None;
        let mut time =
            if carry < 0. && carry < self.interval { 0. } else { finite(self.interval - carry)? };
        for phase in 0..7 {
            while time < shape.ends[phase] || (phase == EXIT_JERK_OUT && time == total) {
                time = self.emit(shape.value(phase, time), time)?;
            }
        }
        match self.last {
            Some(last) => finite(total - last),
            None => finite(carry - total),
        }
    }
}

fn check_phases(profile: &Profile) -> Result<()> {
    let tolerance = profile.cruise_tolerance();
    for (i, phase) in profile.phases.iter().enumerate() {
        if !phase.duration.is_finite()
            || (phase.duration < 0. && !(i == CRUISE && phase.duration >= -tolerance))
        {
            return Err(Error::Invalid("a profile has a negative phase duration"));
        }
        finite(phase.distance)?;
    }
    Ok(())
}

fn cumulative(increments: [f64; 7]) -> Result<[f64; 7]> {
    let mut sums = [0.; 7];
    let mut total = 0.;
    for (sum, increment) in sums.iter_mut().zip(increments) {
        total = finite(total + increment)?;
        *sum = total;
    }
    Ok(sums)
}

struct ProfileShape<'a> {
    profile: &'a Profile,
    duration: [f64; 7],
    ends: [f64; 7],
    bases: [f64; 7],
}

impl<'a> ProfileShape<'a> {
    fn new(profile: &'a Profile) -> Result<Self> {
        let duration = profile.durations();
        Ok(Self {
            profile,
            duration,
            ends: cumulative(duration)?,
            bases: cumulative(profile.distances())?,
        })
    }

    fn value(&self, phase: usize, time: f64) -> f64 {
        let p = self.profile;
        let (entry, peak, exit, acceleration, jerk) =
            (p.entry, p.peak, p.exit, p.acceleration, p.jerk);
        let Self { duration, ends, bases, .. } = self;
        match phase {
            ENTRY_JERK_IN => jerk * cube(time) / 6. + entry * time,
            ACCELERATE => {
                let ramp_speed =
                    ((jerk * duration[ENTRY_JERK_IN]) * duration[ENTRY_JERK_IN]) * 0.5 + entry;
                let tau = time - ends[ENTRY_JERK_IN];
                (bases[ENTRY_JERK_IN] + ramp_speed * tau) + ((acceleration * tau) * tau) * 0.5
            }
            ENTRY_JERK_OUT => {
                let remaining = duration[ENTRY_JERK_OUT] - (time - ends[ACCELERATE]);
                bases[ENTRY_JERK_OUT] - (peak * remaining - jerk * cube(remaining) / 6.)
            }
            CRUISE => peak * (time - ends[ENTRY_JERK_OUT]) + bases[ENTRY_JERK_OUT],
            EXIT_JERK_IN => {
                let tau = time - ends[CRUISE];
                (bases[CRUISE] + peak * tau) - jerk * cube(tau) / 6.
            }
            DECELERATE => {
                let decel_speed =
                    peak - ((jerk * duration[EXIT_JERK_IN]) * duration[EXIT_JERK_IN]) * 0.5;
                let tau = time - ends[EXIT_JERK_IN];
                (bases[EXIT_JERK_IN] + decel_speed * tau) - ((acceleration * tau) * tau) * 0.5
            }
            _ => {
                let remaining = duration[EXIT_JERK_OUT] - (time - ends[DECELERATE]);
                bases[EXIT_JERK_OUT] - (exit * remaining + jerk * cube(remaining) / 6.)
            }
        }
    }
}

fn cube(value: f64) -> f64 {
    (value * value) * value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::profile::{Input, construct};

    fn profile(length: f64, entry: f64, exit: f64) -> Profile {
        construct(Input {
            length,
            entry,
            ceiling: 100.,
            exit,
            acceleration: 100.,
            jerk: 1000.,
            span: [0, 1],
        })
        .unwrap()
    }

    /// A 200 mm profile at 1 ms samples exactly as pinned: the cubic start,
    /// the cruise in the middle, and the final sample landing on the length.
    #[test]
    fn samples_are_pinned_from_start_to_length() {
        let p = profile(200., 0., 0.);
        let samples = sample(std::slice::from_ref(&p), 1., 100_000).unwrap();
        assert_eq!(samples.len(), 3101);
        assert_eq!(samples[..4], [0.0, 1.666666666666667e-7, 1.3333333333333336e-6, 4.5e-6]);
        assert_eq!(samples[1000..1002], [45.16666666666673, 45.26171650000005]);
        assert_eq!(samples[3099..], [199.99999983333333, 200.0]);
        assert!(samples.windows(2).all(|w| w[1] >= w[0]));
    }

    /// The carry keeps the cadence continuous across profiles: the samples
    /// around the join between two 100 mm profiles are pinned.
    #[test]
    fn carry_keeps_the_cadence_across_profiles() {
        let pair = [profile(100., 0., 0.), profile(100., 0., 0.)];
        let samples = sample(&pair, 1., 100_000).unwrap();
        assert_eq!(samples.len(), 4205);
        assert_eq!(samples[..3], [0.0, 1.666666666666667e-7, 1.3333333333333336e-6]);
        assert_eq!(samples[2101..2104], [99.99999943925378, 99.9999999793611, 100.00000002102901]);
        assert_eq!(samples[4203..], [199.9999986728991, 199.99999983488902]);
    }

    /// A root-branch profile carries the slightly negative cruise the
    /// constructor admits; the sampler walks it and still ends on the span.
    #[test]
    fn a_root_profile_with_a_negative_cruise_samples_to_its_span() {
        let p = profile(4., 0., 15.);
        assert!(p.phases()[crate::planner::profile::CRUISE].duration < 0.);
        let samples = sample(std::slice::from_ref(&p), 1., 100_000).unwrap();
        assert_eq!(samples.len(), 374);
        assert_eq!(samples[..3], [0.0, 1.666666666666667e-7, 1.3333333333333336e-6]);
        assert_eq!(samples[372..], [3.9785300428563137, 3.9935305181808665]);
    }

    /// A zero or negative interval and an exhausted budget are refused.
    #[test]
    fn bad_interval_and_budget_are_refused() {
        let p = profile(10., 0., 0.);
        assert!(sample(std::slice::from_ref(&p), 0., 100).is_err());
        assert!(sample(std::slice::from_ref(&p), 1., 3).is_err());
    }
}
