// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{Error, Map};
use openlaser_core::geometry::{Curve, Point};

/// A bounded straight approximation retaining its original curve interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// Start parameter on the input curve.
    pub from: f64,
    /// End parameter on the input curve.
    pub to: f64,
    /// Corrected start, in drawing coordinates.
    pub start: Point,
    /// Corrected end, in drawing coordinates.
    pub end: Point,
}

impl Map {
    /// Invert a curve after adding its machine placement. Returned points are
    /// relative to the same placement. Sampling is limited to 0.001–0.01 mm
    /// deviation and 5 mm segments, with a hard cap of 65536 pieces per curve.
    pub fn curve(
        &self,
        curve: Curve,
        placement: Point,
        tolerance: f64,
    ) -> Result<Vec<Piece>, Error> {
        if !curve.is_valid() || !placement.is_finite() || !(0.001..=0.01).contains(&tolerance) {
            return Err(Error("invalid curve, placement or correction sampling tolerance".into()));
        }
        let mapped = |t| self.inverse(curve.point(t) + placement).map(|p| p - placement);
        let mut pending = vec![(0., 1., mapped(0.)?, mapped(1.)?, 0u8)];
        let mut output = Vec::new();
        while let Some((from, to, start, end, depth)) = pending.pop() {
            let middle = f64::midpoint(from, to);
            let quarter = mapped(from + (to - from) / 4.)?;
            let centre = mapped(middle)?;
            let three_quarters = mapped(from + 3. * (to - from) / 4.)?;
            let deviation = [quarter, centre, three_quarters]
                .into_iter()
                .map(|p| distance(p, start, end))
                .fold(0., f64::max);
            if deviation > tolerance || start.distance(end) > 5. {
                if depth >= 24 || output.len() + pending.len() >= 65_535 {
                    return Err(Error("the correction path exceeds its sampling budget".into()));
                }
                pending.push((middle, to, centre, end, depth + 1));
                pending.push((from, middle, start, centre, depth + 1));
            } else {
                output.push(Piece { from, to, start, end });
            }
        }
        Ok(output)
    }
}

fn distance(point: Point, start: Point, end: Point) -> f64 {
    let delta = end - start;
    let length = delta.dot(delta);
    if length <= 1e-24 {
        return point.distance(start);
    }
    point.distance(start.lerp(end, ((point - start).dot(delta) / length).clamp(0., 1.)))
}
