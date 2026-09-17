// SPDX-License-Identifier: GPL-3.0-or-later

//! Head jump timing, NCModule `0x1000_50B0` to `0x1000_6130`: the lift is a
//! symmetric jerk-limited move, the follow-down ends with the vendor's
//! constant-deceleration tail, and a jump is solved for the height that
//! fits the travel time.

use crate::{Error, Result};

/// The head's motion profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpProfile {
    acceleration: f64,
    jerk: f64,
    speed: f64,
}

/// A solved jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpPlan {
    /// The lift height in millimetres.
    pub height: f64,
    /// The hold at the top in seconds.
    pub hold_seconds: f64,
    /// Lift, hold and descent together.
    pub total_seconds: f64,
}

impl JumpProfile {
    /// A profile from acceleration, acceleration time and speed.
    pub fn new(acceleration: f64, acceleration_time: f64, speed: f64) -> Result<Self> {
        if ![acceleration, acceleration_time, speed].iter().all(|v| v.is_finite() && *v > 0.) {
            return Err(Error::Invalid("head jump needs positive acceleration, time and speed"));
        }
        let jerk = if speed <= acceleration * acceleration_time * 0.5 {
            (speed * 4. / acceleration_time) / acceleration_time
        } else {
            (acceleration * 2.) / acceleration_time
        };
        if !jerk.is_finite() || jerk <= 0. {
            return Err(Error::Invalid("head jump jerk is singular"));
        }
        Ok(Self { acceleration, jerk, speed })
    }

    /// The vendor's head configuration: 20 000 mm/s² and 200 ms, independent
    /// of the machine axis banks.
    pub fn from_follow_speed(speed: f64) -> Result<Self> {
        Self::new(20000., 0.2, speed)
    }

    fn acceleration_distance(self, speed: f64) -> f64 {
        let a = self.acceleration;
        let j = self.jerk;
        if (a / j) * a <= speed {
            (speed / a + a / j) * speed * 0.5
        } else {
            (speed / j).sqrt() * speed
        }
    }

    fn acceleration_seconds(self, speed: f64) -> f64 {
        let threshold = self.acceleration * self.acceleration / self.jerk;
        if threshold < speed {
            2. * self.acceleration / self.jerk + (speed - threshold) / self.acceleration
        } else {
            2. * (speed / self.jerk).sqrt()
        }
    }

    fn lift_seconds(self, height: f64) -> f64 {
        let a = self.acceleration;
        let j = self.jerk;
        let v = self.speed;
        if height >= 2. * self.acceleration_distance(v) {
            height / v + self.acceleration_seconds(v)
        } else if height >= 2. * a * a * a / (j * j) {
            let tj = a / j;
            let ta = ((tj * tj + 4. * height / a).sqrt() - 3. * tj) * 0.5;
            4. * tj + 2. * ta
        } else {
            4. * (height / (2. * j)).cbrt()
        }
    }

    fn down_seconds(self, height: f64) -> f64 {
        let a = self.acceleration;
        let j = self.jerk;
        let v = self.speed;
        let b = 12. * v;
        let at_limit = self.acceleration_distance(v) + v * 0.5 * v / b;
        if height >= at_limit {
            return self.acceleration_seconds(v) + v / b + (height - at_limit) / v;
        }
        let threshold = (a / j) * a;
        let peak = if height <= (a * threshold) / j + threshold * 0.5 * threshold / b {
            // The vendor's bisection stops on 0.02 mm of distance error,
            // including at zero height.
            let mut lo = 0.;
            let mut hi = a * a / j;
            let mut peak = (lo + hi) * 0.5;
            for _ in 0..1000 {
                let distance = peak * (peak.sqrt() / j.sqrt() + (0.5 / b) * peak);
                if (distance - height).abs() < 0.02 {
                    break;
                }
                if distance < height {
                    lo = peak;
                } else {
                    hi = peak;
                }
                peak = (lo + hi) * 0.5;
            }
            peak
        } else {
            let reciprocal = 1. / a + 1. / b;
            (((a / j) * (a / j) + 8. * height * reciprocal).sqrt() - a / j) * 0.5 / reciprocal
        };
        self.acceleration_seconds(peak) + peak / b
    }

    /// Lift and descent times for `height`.
    pub fn motion_seconds(self, height: f64) -> Result<[f64; 2]> {
        if !height.is_finite() || !(0. ..=1000.).contains(&height) {
            return Err(Error::Invalid("head jump height must be 0 to 1000 mm"));
        }
        let result = [self.lift_seconds(height), self.down_seconds(height)];
        if !result.iter().all(|v| v.is_finite() && *v >= 0.) {
            return Err(Error::NonFinite);
        }
        Ok(result)
    }

    fn height_for_time(self, seconds: f64, maximum: f64) -> f64 {
        let half = seconds * 0.5;
        let a = self.acceleration;
        let j = self.jerk;
        let v = self.speed;
        let threshold = a * a / j;
        let acceleration_seconds = self.acceleration_seconds(v);
        let lift_distance = if half >= 2. * acceleration_seconds {
            2. * self.acceleration_distance(v) + v * (half - 2. * acceleration_seconds)
        } else {
            // The vendor's initial bracket, plateau estimate included; the
            // bisection below evaluates real times.
            let peak = if half <= 4. * (threshold / j).sqrt() {
                (0.25 * half) * j * (0.25 * half)
            } else {
                (half * 0.5 - (a * 3.) / j).max(0.) * a + threshold
            };
            2. * self.acceleration_distance(peak)
        };
        let b = v * 12.;
        let down_full = acceleration_seconds + v / b;
        let down_distance = if half > down_full {
            self.acceleration_distance(v) + v * 0.5 * v / b + v * (half - down_full)
        } else {
            let peak = if half <= self.acceleration_seconds(threshold) + threshold / b {
                let t = (((1. + j * half / b).sqrt() - 1.) * b) / j;
                t * j * t
            } else {
                ((half - 2. * a / j - threshold / b) / (a / b + 1.)).max(0.) * a + threshold
            };
            self.acceleration_distance(peak) + peak * 0.5 * peak / b
        };
        let mut low = lift_distance.min(down_distance);
        if maximum < low {
            return maximum;
        }
        let mut high = lift_distance.max(down_distance).min(maximum);
        let mut height = (low + high) * 0.5;
        for _ in 0..=20 {
            let time = self.lift_seconds(height) + self.down_seconds(height);
            if (time - seconds).abs() < 0.005 {
                break;
            }
            if seconds <= time {
                high = height;
            } else {
                low = height;
            }
            if high - low < 0.01 {
                break;
            }
            height = (low + high) * 0.5;
        }
        height
    }

    /// The jump that fits `seconds`: the full `maximum_height` with a hold
    /// when there is time, otherwise the height whose lift and descent fill
    /// the time.
    pub fn solve(self, seconds: f64, maximum_height: f64) -> Result<JumpPlan> {
        if !seconds.is_finite() || seconds < 0. {
            return Err(Error::Invalid("head jump time must be finite and nonnegative"));
        }
        let [up, down] = self.motion_seconds(maximum_height)?;
        let (height, hold_seconds) = if seconds >= up + down {
            (maximum_height, (seconds - up) - down)
        } else {
            (self.height_for_time(seconds, maximum_height), 0.)
        };
        let [up, down] = self.motion_seconds(height)?;
        Ok(JumpPlan { height, hold_seconds, total_seconds: up + hold_seconds + down })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With time to spare the jump reaches the maximum height and holds
    /// there for the remainder.
    #[test]
    fn ample_time_holds_at_the_maximum() {
        let profile = JumpProfile::from_follow_speed(100.).unwrap();
        let plan = profile.solve(5., 40.).unwrap();
        assert_eq!(plan.height, 40.);
        assert!(plan.hold_seconds > 0.);
        assert!((plan.total_seconds - 5.).abs() < 1e-9);
    }

    /// With little time the jump is lower than the maximum and has no hold.
    #[test]
    fn short_time_lowers_the_jump() {
        let profile = JumpProfile::from_follow_speed(100.).unwrap();
        let plan = profile.solve(0.3, 40.).unwrap();
        assert!(plan.height < 40.);
        assert_eq!(plan.hold_seconds, 0.);
        assert!((plan.total_seconds - 0.3).abs() < 0.02);
    }

    /// Nonpositive settings, a negative height and a negative time are
    /// refused.
    #[test]
    fn invalid_inputs_are_refused() {
        assert!(JumpProfile::new(0., 0.2, 100.).is_err());
        let profile = JumpProfile::from_follow_speed(100.).unwrap();
        assert!(profile.motion_seconds(-1.).is_err());
        assert!(profile.solve(-1., 10.).is_err());
    }
}
