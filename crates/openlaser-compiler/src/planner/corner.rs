// SPDX-License-Identifier: GPL-3.0-or-later

//! Source records to nodes: the corner speed each record allows.
//!
//! CADModule `0x1010_D1B0` evaluates a piecewise corner-speed formula
//! (`0x1010_C9F0`) over each record's radius, caps neighbours against each
//! other, repairs zero caps by propagation, then clamps every endpoint by the
//! speed cap, the scale, the floor and the record's own caps. The first and
//! last endpoints are forced to zero afterwards.

use super::{Node, REPAIR_PASSES, Settings, Source};
use crate::{Error, Result, finite};

/// Builds the nodes for `sources` under normalised `settings`.
pub(super) fn nodes(sources: &[Source], settings: &Settings) -> Result<Vec<Node>> {
    let rate = finite(0.5 / settings.acceleration_time)?;
    let mut out = sources
        .iter()
        .map(|source| {
            Ok(Node {
                step: 0.,
                endpoint: corner_speed(source.radius, rate, settings.spline_accuracy)?,
                cap: source.span_cap,
                acceleration: settings.acceleration,
                distance: source.distance,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for i in 1..out.len() {
        // Neighbouring raw endpoints cap each other before the record's own
        // cap applies; the clamped endpoint plays no part here.
        let prior = out[i - 1].endpoint;
        out[i].cap = out[i].endpoint.max(prior).min(sources[i].span_cap);
        out[i].step = finite(out[i].distance - out[i - 1].distance)?;
    }
    repair_caps(&mut out)?;
    for (source, node) in sources.iter().zip(&mut out) {
        let upper = if settings.speed < node.endpoint { settings.speed } else { node.endpoint };
        let scaled = finite(upper * source.scale)?;
        let lower =
            if settings.corner_speed_floor > scaled { settings.corner_speed_floor } else { scaled };
        let capped = if source.point_cap < lower { source.point_cap } else { lower };
        node.endpoint = if source.span_cap < capped { source.span_cap } else { capped };
    }
    if let Some(last) = out.last_mut() {
        last.endpoint = 0.;
    }
    if let Some(first) = out.first_mut() {
        first.endpoint = 0.;
    }
    Ok(out)
}

/// The corner speed for a record of `radius`, with `rate` half the
/// reciprocal acceleration time and `accuracy` the spline accuracy rate.
///
/// The vendor's curve: a fitted piecewise polynomial in the radius, then an
/// accuracy adjustment on either side of 0.03. The coefficients are the fit
/// as recovered; they have no separate meaning.
fn corner_speed(radius: f64, rate: f64, accuracy: f64) -> Result<f64> {
    let base = if radius <= 1.6 {
        (8. * rate) * radius
    } else if radius <= 6. {
        let factor = 30. * (rate - 3.) + 40.;
        (radius * ((102.4 * rate) * rate + (radius - 1.6) * factor)).sqrt()
    } else if radius < 12. {
        let prefix = ((102.4 * rate) * rate + 132. * rate) - 220.;
        (radius * (prefix + (150. * (radius - 6.)) * (rate - 2.))).sqrt()
    } else {
        (radius * (((102.4 * rate) * rate + 1032. * rate) - 2020.)).sqrt()
    };
    let speed = if accuracy >= 0.03 {
        let denominator = 73.8 * accuracy - 3.69;
        let numerator = (25. * rate) * rate;
        // At an accuracy of exactly 0.05 the vendor divides by zero, selects
        // the radius and multiplies it by zero: the speed is the base.
        let selected = if denominator == 0. && numerator > 0. {
            radius
        } else {
            let threshold = (numerator / denominator) / denominator;
            if radius < threshold { radius } else { threshold }
        };
        base + selected * (73.8 * (accuracy - 0.05))
    } else {
        let denominator = 3. - 90. * accuracy;
        let ratio = rate / denominator;
        let threshold = (ratio * rate) / denominator;
        let selected = if radius < threshold { radius } else { threshold };
        base + (15. * selected) * (30. * accuracy - 1.)
    };
    finite(speed)
}

/// Zero caps are copied from a neighbour until every cap clears the
/// threshold; the vendor loops until that happens, which may be never.
fn repair_caps(nodes: &mut [Node]) -> Result<()> {
    if nodes.len() <= 1 {
        return Ok(());
    }
    for _ in 0..REPAIR_PASSES {
        let mut settled = true;
        for i in 1..nodes.len() {
            if nodes[i].cap <= 0.01 {
                if i > 1 {
                    nodes[i].cap = nodes[i - 1].cap;
                } else if i < nodes.len() - 1 && nodes[i + 1].cap > nodes[i].cap {
                    nodes[i].cap = nodes[i + 1].cap;
                }
            }
            if nodes[i].cap < 0.01 {
                settled = false;
            }
        }
        if settled {
            return Ok(());
        }
    }
    Err(Error::Budget("corner cap repair did not settle"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            acceleration: 100.,
            acceleration_time: 0.1,
            spline_accuracy: 0.,
            speed: 100.,
            corner_speed_floor: 0.,
            interval_ms: 1.,
            slow_start_length: 0.,
            slow_start_speed: 20.,
            slow_end_length: 0.,
            slow_end_speed: 20.,
        }
    }

    /// The first and last endpoints are forced to zero, interior endpoints
    /// are capped by the speed, and steps are the distance differences.
    #[test]
    fn endpoints_are_zeroed_at_both_ends_and_capped_inside() {
        let sources = [
            Source::plain(0., 10000., 100.).unwrap(),
            Source::plain(50., 10000., 100.).unwrap(),
            Source::plain(100., 10000., 100.).unwrap(),
        ];
        let nodes = nodes(&sources, &settings()).unwrap();
        assert_eq!(nodes[0].endpoint, 0.);
        assert_eq!(nodes[2].endpoint, 0.);
        assert_eq!(nodes[1].endpoint, 100.);
        assert_eq!(nodes[1].step, 50.);
        assert_eq!(nodes[2].distance, 100.);
    }

    /// A zero cap in the middle is repaired from its predecessor and a zero
    /// cap at index one from its successor when that is larger. Two adjacent
    /// zeros at the start feed each other forever, and the budget stops it.
    #[test]
    fn zero_caps_are_repaired_from_neighbours() {
        let node = |cap| Node { step: 1., endpoint: 0., cap, acceleration: 1., distance: 0. };
        let mut nodes = [10., 5., 0., 10.].map(node);
        repair_caps(&mut nodes).unwrap();
        assert_eq!(nodes.map(|n| n.cap), [10., 5., 5., 10.]);
        let mut nodes = [10., 0., 7., 10.].map(node);
        repair_caps(&mut nodes).unwrap();
        assert_eq!(nodes.map(|n| n.cap), [10., 7., 7., 10.]);
        let mut stuck = [10., 0., 0., 10.].map(node);
        assert_eq!(repair_caps(&mut stuck), Err(Error::Budget("corner cap repair did not settle")));
    }

    /// The straight-line radius of 10 000 yields the fourth formula piece;
    /// a tight radius yields the linear first piece.
    #[test]
    fn corner_speed_pieces_follow_the_radius() {
        let rate = 0.5 / 0.1;
        assert_eq!(corner_speed(1., rate, 0.).unwrap(), 25.);
        let straight = corner_speed(10000., rate, 0.).unwrap();
        assert!(straight > 1000.);
    }
}
