// SPDX-License-Identifier: GPL-3.0-or-later

//! The vendor's velocity planner: source records to sampled distances.
//!
//! CADModule plans a contour in four stages. Source records (radius, distance
//! and scale along the path) become nodes with a corner speed each; nodes are
//! covered by jerk-limited seven-phase [`profile`]s built recursively over
//! the spans between speed extrema; three post-passes merge and trim those
//! profiles; and [`sample`] walks the phases at the interpolation cadence.
//!
//! Evidence: CADModule `0x1010_DF60` (planner entry), `0x1010_D1B0` (node
//! construction), `0x1010_BBC0` (pipeline), `0x1010_64B0` (profile
//! construction), `0x1010_94E0` (sampler). Where a physical name would be a
//! guess, a field keeps the offset name it had in the recovered record.

mod corner;
mod pipeline;
pub mod profile;
mod root;
mod sampler;
mod split;

pub use pipeline::plan;
pub(crate) use pipeline::plan_speeds;
pub use profile::Profile;
pub use sampler::sample;

use crate::{Error, Result, finite};

/// Passes the corner cap repair loop may take. The vendor's loop has none.
const REPAIR_PASSES: usize = 32;
/// Midpoint evaluations a root search may take before we give up.
pub(crate) const ROOT_ITERATIONS: usize = 512;
/// Profile builder calls one contour may take.
const BUILDER_CALLS: usize = 2048;
/// Recursion depth of the span splitter.
const SPLIT_DEPTH: usize = 128;

/// The planner's settings, in the order the vendor's wrapper record holds
/// them. Speeds are mm/s, accelerations mm/s², times seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Cutting acceleration.
    pub acceleration: f64,
    /// Acceleration time; clamped to 0.06..=0.25 s before planning.
    pub acceleration_time: f64,
    /// The spline accuracy rate, which shapes the corner speed formula.
    pub spline_accuracy: f64,
    /// The speed cap.
    pub speed: f64,
    /// A floor under every corner speed.
    pub corner_speed_floor: f64,
    /// The interpolation interval in milliseconds.
    pub interval_ms: f64,
    /// Length of the slow-start region at the contour's beginning.
    pub slow_start_length: f64,
    /// Speed cap inside the slow-start region.
    pub slow_start_speed: f64,
    /// Length of the slow-end region at the contour's end.
    pub slow_end_length: f64,
    /// Speed cap inside the slow-end region.
    pub slow_end_speed: f64,
}

impl Settings {
    /// The vendor's wrapper clamps the acceleration time and lifts the
    /// slow-start speed to the corner floor before planning.
    fn normalized(mut self) -> Result<Self> {
        let fields = [
            self.acceleration,
            self.acceleration_time,
            self.spline_accuracy,
            self.speed,
            self.corner_speed_floor,
            self.interval_ms,
            self.slow_start_length,
            self.slow_start_speed,
            self.slow_end_length,
            self.slow_end_speed,
        ];
        if fields.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("planner settings must be finite"));
        }
        self.acceleration_time = self.acceleration_time.clamp(0.06, 0.25);
        self.slow_start_speed = self.slow_start_speed.max(self.corner_speed_floor);
        if self.acceleration <= 0.
            || self.speed <= 0.
            || self.corner_speed_floor < 0.
            || self.slow_start_length < 0.
            || self.slow_start_speed < 0.
            || self.slow_end_length < 0.
            || self.slow_end_speed < 0.
        {
            return Err(Error::Invalid("planner settings outside the positive domain"));
        }
        Ok(self)
    }

    /// The builder context: the jerk is derived from whether the speed cap
    /// can be reached within half the acceleration time.
    fn context(self) -> Result<Context> {
        let scaled = if self.speed <= self.acceleration * self.acceleration_time * 0.5 {
            (self.speed * 4.) / self.acceleration_time
        } else {
            self.acceleration + self.acceleration
        };
        Ok(Context {
            acceleration: self.acceleration,
            jerk: finite(scaled / self.acceleration_time)?,
            speed: self.speed,
            time: self.acceleration_time,
        })
    }
}

/// One source record of a contour: what the CAD module hands the planner for
/// each geometry extremum along the path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    /// Distance along the path.
    pub distance: f64,
    /// Local curvature radius; straight lines carry 10 000.
    pub radius: f64,
    /// The speed cap for the span ending here.
    pub span_cap: f64,
    /// The speed cap at this point: 30 000 when unconstrained, zero for a stop.
    pub point_cap: f64,
    /// A scale applied to the corner speed here.
    pub scale: f64,
}

impl Source {
    /// A record with finite fields.
    pub fn new(
        distance: f64,
        radius: f64,
        span_cap: f64,
        point_cap: f64,
        scale: f64,
    ) -> Result<Self> {
        if [distance, radius, span_cap, point_cap, scale].iter().all(|v| v.is_finite()) {
            Ok(Self { distance, radius, span_cap, point_cap, scale })
        } else {
            Err(Error::Invalid("source record fields must be finite"))
        }
    }

    /// A record with the unconstrained point cap and unit scale.
    pub fn plain(distance: f64, radius: f64, cap: f64) -> Result<Self> {
        Self::new(distance, radius, cap, 30_000., 1.)
    }
}

/// A planner node: a source record after corner-speed evaluation, in the
/// layout of the vendor's 0x28-byte record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Distance from the previous node.
    pub step: f64,
    /// The speed the profile passes through at this node.
    pub endpoint: f64,
    /// The speed cap for the span ending here.
    pub cap: f64,
    /// The acceleration, copied from the settings.
    pub acceleration: f64,
    /// Cumulative distance along the path.
    pub distance: f64,
}

impl Node {
    fn is_finite(&self) -> bool {
        [self.step, self.endpoint, self.cap, self.acceleration, self.distance]
            .iter()
            .all(|v| v.is_finite())
    }
}

/// What the profile builders read: acceleration, the derived jerk, the speed
/// cap and the acceleration time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Context {
    pub acceleration: f64,
    pub jerk: f64,
    pub speed: f64,
    pub time: f64,
}

/// A planned contour: the normalised settings, the nodes after slow region
/// insertion, and the profiles that cover them in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    /// The settings as the vendor's wrapper normalised them.
    pub settings: Settings,
    /// The nodes the profiles refer to by index.
    pub nodes: Vec<Node>,
    /// The profiles, covering the nodes without gaps.
    pub profiles: Vec<Profile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wrapper clamps the acceleration time to its floor and lifts the
    /// slow-start speed to the corner floor; the context jerk follows from
    /// the clamped time.
    #[test]
    fn wrapper_clamps_time_and_slow_start_before_planning() {
        let settings = Settings {
            acceleration: 100.,
            acceleration_time: 0.01,
            spline_accuracy: 0.,
            speed: 100.,
            corner_speed_floor: 40.,
            interval_ms: 0.,
            slow_start_length: 0.,
            slow_start_speed: 20.,
            slow_end_length: 0.,
            slow_end_speed: 30.,
        };
        let normalized = settings.normalized().unwrap();
        assert_eq!(normalized.acceleration_time, 0.06);
        assert_eq!(normalized.slow_start_speed, 40.);
        assert_eq!(normalized.context().unwrap().jerk, 200. / 0.06);
    }
}
