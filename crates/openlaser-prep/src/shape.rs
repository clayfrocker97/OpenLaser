// SPDX-License-Identifier: GPL-3.0-or-later

//! The direction a contour is cut in, where it starts, and its seam.

use crate::EPS;
use openlaser_core::features::{Direction, Seam, StartPosition};
use openlaser_core::geometry::{Contour, Curve};
use openlaser_core::toolpath::PreparedSegment;

/// Whether `direction` asks for the contour to run the other way.
pub(crate) fn reverses(contour: &Contour, direction: Direction) -> bool {
    match direction {
        Direction::Keep => false,
        Direction::Reverse => true,
        Direction::Clockwise => contour.is_closed() && contour.signed_area() > 0.,
        Direction::Counterclockwise => contour.is_closed() && contour.signed_area() < 0.,
    }
}

/// Where the cut starts, as a fraction of the length of `contour` as it
/// now runs: a spot chosen on the drawing, else the position. Both are
/// given as drawn, so a reversed contour reads them from the other end.
/// Open contours start where they start.
pub(crate) fn start_fraction(
    contour: &Contour,
    position: StartPosition,
    spot: Option<f64>,
    reversed: bool,
) -> f64 {
    if !contour.is_closed() {
        return 0.;
    }
    match spot.map_or(position, StartPosition::Manual) {
        StartPosition::Keep => 0.,
        StartPosition::Manual(fraction) => {
            if reversed {
                (1. - fraction).rem_euclid(1.)
            } else {
                fraction.rem_euclid(1.)
            }
        }
        StartPosition::Automatic => {
            let mut longest = 0;
            for (index, curve) in contour.curves.iter().enumerate() {
                if curve.length() > contour.curves[longest].length() {
                    longest = index;
                }
            }
            let before: f64 = contour.curves[..longest].iter().map(Curve::length).sum();
            (before + contour.curves[longest].length() / 2.) / contour.length()
        }
    }
}

/// The contour with its seam treated: a gap stops short of the start, an
/// overcut runs past it again. Segments keep their process tags.
pub(crate) fn seamed(
    segments: &[PreparedSegment],
    seam: Seam,
    closed: bool,
) -> std::result::Result<Vec<PreparedSegment>, String> {
    let total: f64 = segments.iter().map(|s| s.curve.length()).sum();
    let (kept, extra) = match seam {
        Seam::Seal => return Ok(segments.to_vec()),
        Seam::Gap(length) => (total - length.0, 0.),
        Seam::Overcut(length) => (total, length.0),
    };
    if !closed {
        return Err("the seam of an open contour is left alone".into());
    }
    if kept <= EPS || extra >= total {
        return Err("the seam length must be shorter than the contour".into());
    }
    let mut out = Vec::new();
    take(segments, kept, &mut out);
    take(segments, extra, &mut out);
    Ok(out)
}

/// Appends the first `length` of the path to `out`.
fn take(segments: &[PreparedSegment], mut length: f64, out: &mut Vec<PreparedSegment>) {
    for segment in segments {
        if length <= EPS {
            return;
        }
        let whole = segment.curve.length();
        out.push(if length >= whole { segment.clone() } else { segment.slice(0., length / whole) });
        length -= whole;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::toolpath::SegmentProcess;

    fn seamed(
        curves: &[Curve],
        tags: &[SegmentProcess],
        seam: Seam,
        closed: bool,
    ) -> std::result::Result<(Vec<Curve>, Vec<SegmentProcess>), String> {
        let segments: Vec<_> =
            curves.iter().zip(tags).map(|(c, p)| PreparedSegment::new(*c, p.clone())).collect();
        Ok(super::seamed(&segments, seam, closed)?
            .into_iter()
            .map(|s| (s.curve, s.process))
            .unzip())
    }

    use openlaser_core::geometry::Point;
    use openlaser_core::units::Millimeters;

    fn square() -> Contour {
        let corners =
            [Point::ORIGIN, Point::new(10., 0.), Point::new(10., 10.), Point::new(0., 10.)];
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: corners[i], end: corners[(i + 1) % 4] })
                .collect(),
        }
    }

    /// Clockwise reverses a counterclockwise square and leaves a clockwise
    /// one; Reverse always flips; open contours only flip on Reverse.
    #[test]
    fn direction_is_read_from_the_winding() {
        let ccw = square();
        let cw = square().reversed();
        assert!(reverses(&ccw, Direction::Clockwise));
        assert!(!reverses(&cw, Direction::Clockwise));
        assert!(reverses(&cw, Direction::Counterclockwise));
        assert!(reverses(&ccw, Direction::Reverse) && !reverses(&ccw, Direction::Keep));
        let open = Contour { layer: "0".into(), curves: vec![ccw.curves[0]] };
        assert!(!reverses(&open, Direction::Clockwise) && reverses(&open, Direction::Reverse));
    }

    /// The automatic start is halfway along the longest segment, a manual
    /// start reads from the other end after reversal, and open contours
    /// always start at their first point.
    #[test]
    fn start_fractions_follow_the_policy() {
        let mut long = square();
        long.curves[1] = Curve::Line { start: Point::new(10., 0.), end: Point::new(10., 30.) };
        long.curves[2] = Curve::Line { start: Point::new(10., 30.), end: Point::new(0., 30.) };
        long.curves[3] = Curve::Line { start: Point::new(0., 30.), end: Point::ORIGIN };
        assert!(
            (start_fraction(&long, StartPosition::Automatic, None, false) - 25. / 80.).abs()
                < 1e-12
        );
        assert_eq!(start_fraction(&square(), StartPosition::Manual(0.25), None, false), 0.25);
        assert_eq!(start_fraction(&square(), StartPosition::Keep, Some(0.25), false), 0.25);
        assert_eq!(start_fraction(&square(), StartPosition::Manual(0.25), None, true), 0.75);
        assert_eq!(start_fraction(&square(), StartPosition::Manual(1.), None, false), 0.);
        let open = Contour { layer: "0".into(), curves: vec![square().curves[0]] };
        assert_eq!(start_fraction(&open, StartPosition::Manual(0.5), None, false), 0.);
    }

    /// A gap drops the last millimetres, an overcut repeats the first, and
    /// the process tags follow their geometry.
    #[test]
    fn seams_cut_short_or_run_past() {
        let square = square();
        let tags: Vec<SegmentProcess> =
            (0..4).map(|i| SegmentProcess { cool_ms: i, ..SegmentProcess::default() }).collect();
        let (gap, gap_tags) =
            seamed(&square.curves, &tags, Seam::Gap(Millimeters(3.)), true).unwrap();
        assert_eq!(gap.len(), 4);
        assert!((gap.iter().map(Curve::length).sum::<f64>() - 37.).abs() < 1e-12);
        assert_eq!(gap_tags[3].cool_ms, 3);
        let (over, over_tags) =
            seamed(&square.curves, &tags, Seam::Overcut(Millimeters(12.)), true).unwrap();
        assert_eq!(over.len(), 6);
        assert!((over.iter().map(Curve::length).sum::<f64>() - 52.).abs() < 1e-12);
        assert_eq!(over_tags[5].cool_ms, 1);
        assert!(seamed(&square.curves, &tags, Seam::Gap(Millimeters(40.)), true).is_err());
        assert!(
            seamed(&square.curves[..2], &tags[..2], Seam::Gap(Millimeters(1.)), false).is_err()
        );
    }
}
