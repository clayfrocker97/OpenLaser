// SPDX-License-Identifier: GPL-3.0-or-later

//! The residue-cleaning spiral, CADModule `0x1010_43B0`: an Archimedean
//! spiral the planner sees as half-turn source records, sampled in the
//! planner's own path coordinate rather than true arc length.

use crate::motion::Plan;
use crate::planner::{self, Source};
use crate::{Error, Point, Result};
use std::f64::consts::TAU;

/// A spiral of `turns` turns out to `radius` at `speed`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spiral {
    /// The final radius, at least 0.1 mm.
    pub radius: f64,
    /// The number of turns, at least one.
    pub turns: u32,
    /// The cleaning speed.
    pub speed: f64,
}

impl Spiral {
    /// A spiral with the vendor's minima applied.
    pub fn new(radius: f64, turns: u32, speed: f64) -> Result<Self> {
        if !radius.is_finite()
            || !(0. ..=100_000.).contains(&radius)
            || !speed.is_finite()
            || speed <= 0.
            || speed > 100_000.
            || turns > 4096
        {
            return Err(Error::Invalid("spiral radius, turns or speed are outside the domain"));
        }
        Ok(Self { radius: radius.max(0.1), turns: turns.max(1), speed })
    }

    /// The path coordinate at the spiral's end.
    #[must_use]
    pub fn end_position(self) -> f64 {
        self.radius * TAU * f64::from(self.turns) * 0.5
    }

    /// The planner's source records: one per half turn.
    pub fn sources(self) -> Result<Vec<Source>> {
        let angle = TAU * f64::from(self.turns);
        (0..=self.turns * 2)
            .map(|i| {
                let theta = f64::from(i) * TAU * 0.5;
                let r = theta * self.radius / angle;
                Source::plain(r * 0.5 * theta, r, self.speed)
            })
            .collect()
    }

    /// Plans and samples the spiral. The speed cap and slow regions come
    /// from the spiral, the rest from `settings`.
    pub fn plan(self, mut settings: planner::Settings, max_samples: usize) -> Result<Plan> {
        settings.speed = self.speed;
        settings.slow_start_length = 0.;
        settings.slow_end_length = 0.;
        let planned = planner::plan(&self.sources()?, settings)?;
        let distances =
            planner::sample(&planned.profiles, planned.settings.interval_ms, max_samples)?;
        let points = distances.iter().map(|&p| self.point(p)).collect::<Result<Vec<_>>>()?;
        Ok(Plan {
            interval: planned.settings.interval_ms * 0.001,
            distances: distances.into(),
            points: points.into(),
        })
    }

    /// The point at a path coordinate.
    pub fn point(self, position: f64) -> Result<Point> {
        if !position.is_finite() || position < 0. || position > self.end_position() {
            return Err(Error::Invalid("spiral sample is outside its path"));
        }
        let k = self.radius / (TAU * f64::from(self.turns));
        let theta = (2. * position / k).sqrt();
        let r = k * theta;
        Ok([r * theta.cos(), r * theta.sin()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two turns give five half-turn records ending at the full angle times
    /// the radius, all with the spiral's speed as their cap.
    #[test]
    fn half_turn_sources_and_minima() {
        let spiral = Spiral::new(2., 2, 10.).unwrap();
        let rows = spiral.sources().unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].distance, 0.);
        assert!((rows[4].distance - 2. * TAU).abs() < 1e-12);
        assert_eq!(rows[4].radius, 2.);
        assert!(rows.iter().all(|r| r.span_cap == 10. && r.point_cap == 30_000. && r.scale == 1.));
        let minimal = Spiral::new(0., 0, 1.).unwrap();
        assert_eq!((minimal.radius, minimal.turns), (0.1, 1));
    }

    /// The radius grows with the angle: a quarter of the way the point sits
    /// one millimetre out on the axis, and the end lies at the full radius.
    #[test]
    fn radial_growth_is_a_spiral_not_a_circle() {
        let spiral = Spiral::new(2., 2, 10.).unwrap();
        assert_eq!(spiral.point(0.).unwrap(), [0., 0.]);
        let quarter = spiral.point(spiral.end_position() * 0.25).unwrap();
        assert!((quarter[0] - 1.).abs() < 1e-12 && quarter[1].abs() < 1e-12);
        let end = spiral.point(spiral.end_position()).unwrap();
        assert!((end[0] - 2.).abs() < 1e-12 && end[1].abs() < 1e-12);
        assert!(spiral.point(-1.).is_err());
    }

    /// The sampled spiral starts at the origin, stays inside its radius and
    /// visits both negative x and positive y.
    #[test]
    fn spiral_sources_reach_sampled_motion() {
        let settings = planner::Settings {
            acceleration: 5000.,
            acceleration_time: 0.1,
            spline_accuracy: 0.,
            speed: 50.,
            corner_speed_floor: 0.,
            interval_ms: 1.,
            slow_start_length: 0.,
            slow_start_speed: 50.,
            slow_end_length: 0.,
            slow_end_speed: 50.,
        };
        let plan = Spiral::new(2., 2, 10.).unwrap().plan(settings, 100_000).unwrap();
        assert!(plan.points.len() > 100);
        assert_eq!(plan.points[0], [0., 0.]);
        assert_eq!(plan.interval, 0.001);
        assert!(plan.points.iter().all(|p| p[0].hypot(p[1]) <= 2. + 1e-9));
        assert!(plan.points.iter().any(|p| p[0] < -0.5));
        assert!(plan.points.iter().any(|p| p[1] > 0.5));
    }
}
