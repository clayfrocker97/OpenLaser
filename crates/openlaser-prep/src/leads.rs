// SPDX-License-Identifier: GPL-3.0-or-later

//! Entry and exit leads: the approach before the contour and the departure
//! after it, on the waste side, so the pierce and the finish stay off the
//! part's edge.

use crate::{Result, feature};
use openlaser_core::features::{Lead, LeadShape, Leads, Side};
use openlaser_core::geometry::{Curve, Point};
use openlaser_core::toolpath::{LeadRole, PreparedSegment, SegmentProcess};
use std::f64::consts::FRAC_PI_2;

pub(crate) fn check(leads: &Leads) -> Result<()> {
    let roles = [("entry lead", leads.entry), ("exit lead", leads.exit)];
    let edited = leads.overrides.iter().flat_map(|edited| {
        [("edited entry lead", edited.entry), ("edited exit lead", edited.exit)]
    });
    for (name, lead) in roles.into_iter().chain(edited) {
        let Some(lead) = lead else { continue };
        check_lead(name, lead)?;
    }
    for edited in &leads.overrides {
        crate::joints::spots_ok("leads", std::slice::from_ref(&edited.location))?;
        if edited.entry.is_none() && edited.exit.is_none() {
            return Err(feature("leads", "an override must edit an entry or exit"));
        }
    }
    Ok(())
}

fn check_lead(name: &'static str, lead: Lead) -> Result<()> {
    let positive = |value: f64| value.is_finite() && value > 0.;
    let angle = lead.angle.0;
    if !angle.is_finite() {
        return Err(feature(name, "the angle must be a number"));
    }
    if matches!(lead.shape, LeadShape::Line | LeadShape::LineArc) && !positive(lead.length.0) {
        return Err(feature(name, "the length must be positive"));
    }
    if matches!(lead.shape, LeadShape::Arc | LeadShape::LineArc)
        && (!positive(lead.radius.0) || angle <= 0. || angle >= 360.)
    {
        return Err(feature(
            name,
            "an arc needs a positive radius and an angle between 0 and 360 degrees",
        ));
    }
    Ok(())
}

/// The contour with its leads attached and tagged. `counterclockwise` is
/// the winding of the contour as it runs; with it and the side, the leads
/// know which way is into the waste.
pub(crate) fn attached(
    segments: Vec<PreparedSegment>,
    leads: Option<&Leads>,
    closed: bool,
    hole: bool,
    counterclockwise: bool,
) -> Vec<PreparedSegment> {
    let Some(leads) = leads else { return segments };
    if (leads.closed_only && !closed) || segments.is_empty() {
        return segments;
    }
    let inside = match leads.side {
        Side::Inside => true,
        Side::Outside => false,
        Side::Auto => hole,
    };
    // Counterclockwise, the inside is to the left of travel.
    let side = if counterclockwise == inside { 1. } else { -1. };
    let first = segments[0].curve;
    let last = segments[segments.len() - 1].curve;
    let entry = leads
        .entry
        .map_or_else(Vec::new, |lead| shape(lead, first.start(), first.tangent(0.), side, true));
    let exit = leads
        .exit
        .map_or_else(Vec::new, |lead| shape(lead, last.end(), last.tangent(1.), side, false));
    let tag = |role| SegmentProcess { lead: Some(role), ..SegmentProcess::default() };
    let mut all: Vec<_> =
        entry.into_iter().map(|curve| PreparedSegment::new(curve, tag(LeadRole::Entry))).collect();
    all.extend(segments);
    all.extend(exit.into_iter().map(|curve| PreparedSegment::new(curve, tag(LeadRole::Exit))));
    all
}

/// The curves of one lead at `anchor`, where the contour's tangent is
/// `tangent`; `side` is +1 when the waste is to the left of travel.
fn shape(lead: Lead, anchor: Point, tangent: Point, side: f64, entry: bool) -> Vec<Curve> {
    let angle = lead.angle.0.to_radians();
    let length = lead.length.0;
    let line = |at: Point, direction: Point| {
        if entry {
            Curve::Line { start: at - direction * length, end: at }
        } else {
            Curve::Line { start: at, end: at + direction * length }
        }
    };
    if lead.shape == LeadShape::Line {
        let turn = if entry { -side * angle } else { side * angle };
        return vec![line(anchor, tangent.rotated(turn))];
    }
    let radius = lead.radius.0;
    let center = anchor + tangent.rotated(side * FRAC_PI_2) * radius;
    let at = (anchor - center).angle();
    let sweep = side * angle;
    let arc =
        Curve::Arc { center, radius, start_angle: if entry { at - sweep } else { at }, sweep };
    if lead.shape == LeadShape::Arc {
        return vec![arc];
    }
    let (join, direction) =
        if entry { (arc.start(), arc.tangent(0.)) } else { (arc.end(), arc.tangent(1.)) };
    let straight = line(join, direction);
    if entry { vec![straight, arc] } else { vec![arc, straight] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attached(
        curves: Vec<Curve>,
        processes: Vec<SegmentProcess>,
        leads: Option<&Leads>,
        closed: bool,
        hole: bool,
        counterclockwise: bool,
    ) -> (Vec<Curve>, Vec<SegmentProcess>) {
        let segments =
            curves.into_iter().zip(processes).map(|(c, p)| PreparedSegment::new(c, p)).collect();
        super::attached(segments, leads, closed, hole, counterclockwise)
            .into_iter()
            .map(|s| (s.curve, s.process))
            .unzip()
    }

    use openlaser_core::units::{Degrees, Millimeters};

    fn lead(shape: LeadShape) -> Lead {
        Lead { shape, length: Millimeters(2.), radius: Millimeters(1.), angle: Degrees(90.) }
    }

    fn leads(shape: LeadShape) -> Leads {
        Leads {
            entry: Some(lead(shape)),
            exit: Some(lead(shape)),
            side: Side::Auto,
            closed_only: false,
            overrides: Vec::new(),
        }
    }

    /// A line running along +X on a counterclockwise outline gets its
    /// entry from below (the waste is outside, to the right) and its exit
    /// to below; on a hole both go up, into the hole.
    #[test]
    fn line_leads_lie_on_the_waste_side() {
        let along = Curve::Line { start: Point::ORIGIN, end: Point::new(10., 0.) };
        let (curves, tags) = attached(
            vec![along],
            vec![SegmentProcess::default()],
            Some(&leads(LeadShape::Line)),
            true,
            false,
            true,
        );
        assert_eq!(curves.len(), 3);
        assert_eq!(tags[0].lead, Some(LeadRole::Entry));
        assert_eq!(tags[2].lead, Some(LeadRole::Exit));
        assert!(curves[0].start().distance(Point::new(0., -2.)) < 1e-9);
        assert_eq!(curves[0].end(), Point::ORIGIN);
        assert!(curves[2].end().distance(Point::new(10., -2.)) < 1e-9);
        let (hole, _) = attached(
            vec![along],
            vec![SegmentProcess::default()],
            Some(&leads(LeadShape::Line)),
            true,
            true,
            true,
        );
        assert!(hole[0].start().distance(Point::new(0., 2.)) < 1e-9);
        assert!(hole[2].end().distance(Point::new(10., 2.)) < 1e-9);
    }

    /// An arc lead is tangent to the contour where it meets it, and a
    /// line-arc lead runs straight into the arc's far end.
    #[test]
    fn arc_leads_are_tangent_at_the_anchor() {
        let along = Curve::Line { start: Point::ORIGIN, end: Point::new(10., 0.) };
        let (curves, _) = attached(
            vec![along],
            vec![SegmentProcess::default()],
            Some(&leads(LeadShape::Arc)),
            true,
            false,
            true,
        );
        let entry = curves[0];
        assert!(entry.end().distance(Point::ORIGIN) < 1e-9);
        assert!(entry.tangent(1.).distance(Point::new(1., 0.)) < 1e-9);
        assert!(entry.start().distance(Point::new(-1., -1.)) < 1e-9);
        let (curves, _) = attached(
            vec![along],
            vec![SegmentProcess::default()],
            Some(&leads(LeadShape::LineArc)),
            true,
            false,
            true,
        );
        assert_eq!(curves.len(), 5);
        assert!(curves[0].end().distance(curves[1].start()) < 1e-9);
        assert!(curves[0].tangent(1.).distance(curves[1].tangent(0.)) < 1e-9);
        assert!(curves[0].start().distance(Point::new(-1., -3.)) < 1e-9);
    }

    /// Open contours get no leads when only closed ones should, and bad
    /// lead values are refused.
    #[test]
    fn closed_only_and_checks() {
        let along = Curve::Line { start: Point::ORIGIN, end: Point::new(10., 0.) };
        let only = Leads { closed_only: true, ..leads(LeadShape::Line) };
        assert_eq!(
            attached(vec![along], vec![SegmentProcess::default()], Some(&only), false, false, true)
                .0
                .len(),
            1
        );
        assert!(
            check(&Leads {
                entry: Some(Lead { length: Millimeters(0.), ..lead(LeadShape::Line) }),
                ..leads(LeadShape::Line)
            })
            .is_err()
        );
        assert!(
            check(&Leads {
                exit: Some(Lead { angle: Degrees(360.), ..lead(LeadShape::Arc) }),
                ..leads(LeadShape::Arc)
            })
            .is_err()
        );
        assert!(check(&leads(LeadShape::LineArc)).is_ok());
    }
}
