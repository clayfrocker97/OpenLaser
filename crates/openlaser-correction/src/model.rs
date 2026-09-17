// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{COUPON_MM, Error};
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Point};
use serde::{Deserialize, Serialize};

/// Effective centreline dimensions of one cooled coupon, in millimetres.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    /// Width along the machine X axis.
    pub x: f64,
    /// Height along the machine Y axis.
    pub y: f64,
}

impl Measurement {
    /// Average matching plug/opening dimensions to remove first-order kerf bias.
    pub fn from_edges(plug: [f64; 2], opening: [f64; 2]) -> Result<Self, Error> {
        if plug.into_iter().chain(opening).any(|v| !v.is_finite() || !(80. ..=120.).contains(&v))
            || opening[0] < plug[0]
            || opening[1] < plug[1]
        {
            return Err(Error(
                "opening measurements must be at least as large as the square".into(),
            ));
        }
        let result = Self { x: plug[0].midpoint(opening[0]), y: plug[1].midpoint(opening[1]) };
        result.validate()?;
        Ok(result)
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        if [self.x, self.y].iter().all(|v| v.is_finite() && (90. ..=110.).contains(v)) {
            Ok(())
        } else {
            Err(Error("each effective X/Y measurement must be between 90 and 110 mm".into()))
        }
    }
}

/// A reproducible measurement set in machine coordinates.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Bed bounds used to place the measured squares.
    pub bed: Bounds,
    /// Bottom row first, left to right, then middle and top rows.
    pub measurements: [Measurement; 9],
}

impl Profile {
    /// Nine coupon centres, with 10 mm clearance around each outer square.
    pub fn positions(&self) -> Result<[Point; 9], Error> {
        positions(self.bed)
    }

    /// Validate measurements and confirm the map stays locally invertible.
    pub fn validate(&self) -> Result<(), Error> {
        Map::new(self).map(|_| ())
    }
}

/// A validated immutable map. All coordinates are physical millimetres.
#[derive(Clone, Debug)]
pub struct Map {
    profile: Profile,
    x: [f64; 3],
    y: [f64; 3],
    centre: Point,
}

impl Map {
    /// Build the map without accepting invalid measurements or a fold.
    pub fn new(profile: &Profile) -> Result<Self, Error> {
        let nodes = positions(profile.bed)?;
        for measurement in profile.measurements {
            measurement.validate()?;
        }
        let map = Self {
            profile: profile.clone(),
            x: [nodes[0].x, nodes[1].x, nodes[2].x],
            y: [nodes[0].y, nodes[3].y, nodes[6].y],
            centre: profile.bed.min.lerp(profile.bed.max, 0.5),
        };
        for row in 0..=16 {
            for column in 0..=16 {
                let p = Point::new(
                    profile.bed.min.x + profile.bed.width() * f64::from(column) / 16.,
                    profile.bed.min.y + profile.bed.height() * f64::from(row) / 16.,
                );
                map.jacobian(p)?;
            }
        }
        Ok(map)
    }

    /// Whether correction can preserve the original analytic geometry exactly.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.profile.measurements.iter().all(|m| m.x == COUPON_MM && m.y == COUPON_MM)
    }

    /// The local measured X/Y scale, bilinearly interpolated between coupons.
    #[must_use]
    pub fn scale(&self, point: Point) -> Point {
        let (column, across) = interval(point.x, self.x);
        let (row, up) = interval(point.y, self.y);
        let sample = |index: usize| {
            let measurement = self.profile.measurements[index];
            Point::new(measurement.x / COUPON_MM, measurement.y / COUPON_MM)
        };
        sample(row * 3 + column).lerp(sample(row * 3 + column + 1), across).lerp(
            sample((row + 1) * 3 + column).lerp(sample((row + 1) * 3 + column + 1), across),
            up,
        )
    }

    /// Modelled physical position for a commanded machine coordinate.
    #[must_use]
    pub fn forward(&self, p: Point) -> Point {
        Point::new(
            self.centre.x
                + integral(self.centre.x, p.x, self.x, |x| self.scale(Point::new(x, p.y)).x),
            self.centre.y
                + integral(self.centre.y, p.y, self.y, |y| self.scale(Point::new(p.x, y)).y),
        )
    }

    /// Command coordinate that produces the desired physical position in this model.
    pub fn inverse(&self, desired: Point) -> Result<Point, Error> {
        if !desired.is_finite() || desired.x.abs().max(desired.y.abs()) > 1_000_000. {
            return Err(Error(
                "correction coordinates must be finite and within 1000000 mm".into(),
            ));
        }
        if self.is_identity() {
            return Ok(desired);
        }
        let mut command = desired;
        for _ in 0..20 {
            let error = self.forward(command) - desired;
            if error.norm() <= 1e-8 {
                return Ok(command);
            }
            let [xx, xy, yx, yy] = self.jacobian(command)?;
            let determinant = xx * yy - xy * yx;
            command = command
                - Point::new(
                    (yy * error.x - xy * error.y) / determinant,
                    (xx * error.y - yx * error.x) / determinant,
                );
            if !command.is_finite() || command.x.abs().max(command.y.abs()) > 1_000_000. {
                break;
            }
        }
        Err(Error("the correction map could not resolve this position".into()))
    }

    fn jacobian(&self, p: Point) -> Result<[f64; 4], Error> {
        let h = 0.001;
        let dx = (self.forward(Point::new(p.x + h, p.y)) - self.forward(Point::new(p.x - h, p.y)))
            * (0.5 / h);
        let dy = (self.forward(Point::new(p.x, p.y + h)) - self.forward(Point::new(p.x, p.y - h)))
            * (0.5 / h);
        let det = dx.x * dy.y - dy.x * dx.y;
        if !det.is_finite() || det <= 0.05 {
            return Err(Error(
                "measurements create a folded correction map; check the X/Y values".into(),
            ));
        }
        Ok([dx.x, dy.x, dx.y, dy.y])
    }
}

fn positions(bed: Bounds) -> Result<[Point; 9], Error> {
    if !bed.min.is_finite()
        || !bed.max.is_finite()
        || bed.min.x.abs().max(bed.min.y.abs()).max(bed.max.x.abs()).max(bed.max.y.abs()) > 100_000.
        || bed.width() < 340.
        || bed.height() < 340.
    {
        return Err(Error(
            "the bed must fit a 3 × 3 grid of 100 mm squares with edge clearance".into(),
        ));
    }
    let x = [bed.min.x + 60., bed.min.x.midpoint(bed.max.x), bed.max.x - 60.];
    let y = [bed.min.y + 60., bed.min.y.midpoint(bed.max.y), bed.max.y - 60.];
    Ok(std::array::from_fn(|i| Point::new(x[i % 3], y[i / 3])))
}

fn interval(value: f64, nodes: [f64; 3]) -> (usize, f64) {
    let i = usize::from(value > nodes[1]);
    (i, ((value - nodes[i]) / (nodes[i + 1] - nodes[i])).clamp(0., 1.))
}

fn integral(a: f64, b: f64, knots: [f64; 3], value: impl Fn(f64) -> f64) -> f64 {
    let (lo, hi, sign) = if a <= b { (a, b, 1.) } else { (b, a, -1.) };
    let mut prior = lo;
    let mut sum = 0.;
    for next in knots.into_iter().filter(|p| *p > lo && *p < hi).chain(std::iter::once(hi)) {
        sum += (next - prior) * (value(prior) + value(next)) / 2.;
        prior = next;
    }
    sign * sum
}

/// Uncorrected squares at the nine measurement positions, in machine coordinates.
pub fn coupon_drawing(bed: Bounds) -> Result<Drawing, Error> {
    let contours = positions(bed)?
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let points = [
                Point::new(p.x - 50., p.y - 50.),
                Point::new(p.x + 50., p.y - 50.),
                Point::new(p.x + 50., p.y + 50.),
                Point::new(p.x - 50., p.y + 50.),
            ];
            Contour {
                layer: format!("Coupon {}", i + 1),
                curves: (0..4)
                    .map(|n| Curve::Line { start: points[n], end: points[(n + 1) % 4] })
                    .collect(),
            }
        })
        .collect();
    Ok(Drawing { contours })
}
