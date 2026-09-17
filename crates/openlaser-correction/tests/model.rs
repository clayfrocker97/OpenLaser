// SPDX-License-Identifier: GPL-3.0-or-later
//! Measurement, inverse-map and small-curve acceptance.
#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    reason = "known fixtures with exact subdivision boundaries"
)]

use openlaser_core::geometry::{Bounds, Curve, Point};
use openlaser_correction::{Map, Measurement, Profile, coupon_drawing};

fn profile() -> Profile {
    Profile {
        bed: Bounds { min: Point::new(-20., 30.), max: Point::new(1380., 930.) },
        measurements: [Measurement { x: 100., y: 100. }; 9],
    }
}

#[test]
fn nine_nominal_squares_and_identity_preserve_coordinates() {
    let p = profile();
    let drawing = coupon_drawing(p.bed).unwrap();
    let positions = p.positions().unwrap();
    let map = Map::new(&p).unwrap();
    assert!(map.is_identity());
    assert_eq!(drawing.contours.len(), 9);
    for (i, contour) in drawing.contours.iter().enumerate() {
        assert_eq!(contour.curves.len(), 4);
        assert!((contour.signed_area().abs() - 10_000.).abs() < 1e-6);
        for curve in &contour.curves {
            assert!((curve.length() - 100.).abs() < 1e-8);
        }
        assert_eq!(map.inverse(positions[i]).unwrap(), positions[i]);
    }
    assert_eq!(positions[0], Point::new(40., 90.));
    assert_eq!(positions[8], Point::new(1320., 870.));
}

#[test]
fn uniform_anisotropic_scale_has_the_analytic_inverse() {
    let mut p = profile();
    p.measurements = [Measurement { x: 101., y: 99. }; 9];
    let map = Map::new(&p).unwrap();
    let centre = p.bed.min.lerp(p.bed.max, 0.5);
    for point in [p.bed.min, p.bed.max, centre, Point::new(-50., 1000.)] {
        let actual = map.inverse(point).unwrap();
        let expected = Point::new(
            centre.x + (point.x - centre.x) / 1.01,
            centre.y + (point.y - centre.y) / 0.99,
        );
        assert!(actual.distance(expected) < 1e-7);
        assert!(map.forward(actual).distance(point) < 1e-7);
    }
}

#[test]
fn coupled_field_round_trips_and_corrects_a_placed_small_circle() {
    let mut p = profile();
    p.measurements = std::array::from_fn(|i| {
        let column = [0., 1., 2.][i % 3];
        let row = [0., 1., 2.][i / 3];
        Measurement { x: 99. + column * 0.9 + row * 0.2, y: 100.8 - row * 0.7 + column * 0.15 }
    });
    let map = Map::new(&p).unwrap();
    for row in 0..31 {
        for col in 0..41 {
            let wanted = Point::new(-50. + f64::from(col) * 37., 10. + f64::from(row) * 32.);
            assert!(map.forward(map.inverse(wanted).unwrap()).distance(wanted) < 1e-7);
        }
    }
    let curve = Curve::Arc {
        center: Point::new(3., 4.),
        radius: 0.8,
        start_angle: 0.2,
        sweep: std::f64::consts::TAU,
    };
    for placement in [Point::new(70., 100.), Point::new(1200., 820.)] {
        let pieces = map.curve(curve, placement, 0.002).unwrap();
        assert!(pieces.len() >= 32);
        assert_eq!(pieces.first().unwrap().from, 0.);
        assert_eq!(pieces.last().unwrap().to, 1.);
        for pair in pieces.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
            assert_eq!(pair[0].to, pair[1].from);
        }
        for piece in pieces {
            for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
                let actual = map.forward(piece.start.lerp(piece.end, t) + placement);
                let expected = curve.point(piece.from + (piece.to - piece.from) * t) + placement;
                assert!(actual.distance(expected) < 0.0022);
            }
        }
    }
}

#[test]
fn rejects_bad_dimensions_and_reduces_kerf_bias() {
    assert_eq!(
        Measurement::from_edges([99.8, 99.7], [100.2, 100.3]).unwrap(),
        Measurement { x: 100., y: 100. }
    );
    assert!(Measurement::from_edges([100.2, 100.], [99.8, 100.]).is_err());
    let mut p = profile();
    for invalid in [f64::NAN, f64::INFINITY, 0., 89.99, 110.01] {
        p.measurements[0].x = invalid;
        assert!(Map::new(&p).is_err());
    }
    p = profile();
    p.bed.max.x = 100.;
    assert!(p.positions().is_err());
    let map = Map::new(&profile()).unwrap();
    assert!(map.inverse(Point::new(f64::NAN, 0.)).is_err());
}
