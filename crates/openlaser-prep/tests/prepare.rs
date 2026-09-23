// SPDX-License-Identifier: GPL-3.0-or-later

//! The plate fixture prepared under several feature sets, rendered as
//! text: each contour's sources, depth, closure and every segment with its
//! process. A change to preparation shows up as a snapshot diff.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests read known drawings")]

use openlaser_core::features::{
    Bridge, Bridges, Cooling, CoolingPlacement, CutOrder, Direction, Features, JointBehaviour,
    JointPlacement, Joints, Kerf, Lead, LeadShape, Leads, OrderStrategy, Pick, Seam, Side, Spot,
    Start, StartPosition,
};
use openlaser_core::geometry::{Curve, Drawing, Point};
use openlaser_core::toolpath::{LeadRole, Toolpath};
use openlaser_core::units::{Degrees, Millimeters, Milliseconds, MmPerSecond, Percent};
use std::fmt::Write as _;

fn plate() -> Drawing {
    let path = format!("{}/../../fixtures/dxf/plate-with-holes.dxf", env!("CARGO_MANIFEST_DIR"));
    openlaser_dxf::import(&std::fs::read(path).unwrap()).unwrap().drawing
}

fn render(toolpath: &Toolpath) -> String {
    let mut text = String::new();
    for warning in &toolpath.warnings {
        writeln!(text, "warning: {warning}").unwrap();
    }
    for contour in &toolpath.contours {
        writeln!(
            text,
            "contour from {:?}: layer {:?}, depth {}, {}, length {:.4}",
            contour.sources,
            contour.layer,
            contour.depth,
            if contour.closed { "closed" } else { "open" },
            contour.length()
        )
        .unwrap();
        for segment in &contour.segments {
            let (curve, process) = (&segment.curve, &segment.process);
            let geometry = match *curve {
                Curve::Line { start, end } => {
                    format!("line ({:.4}, {:.4}) -> ({:.4}, {:.4})", start.x, start.y, end.x, end.y)
                }
                Curve::Arc { center, radius, start_angle, sweep } => format!(
                    "arc centre ({:.4}, {:.4}) radius {:.4} from {:.4} rad sweep {:.4} rad",
                    center.x, center.y, radius, start_angle, sweep
                ),
            };
            let mut tags = Vec::new();
            match process.lead {
                Some(LeadRole::Entry) => tags.push("entry lead".to_owned()),
                Some(LeadRole::Exit) => tags.push("exit lead".to_owned()),
                None => {}
            }
            if process.joint {
                tags.push(format!(
                    "joint power {:?} speed {:?}",
                    process.power.unwrap_or(0.),
                    process.speed
                ));
            }
            if process.repierce {
                tags.push("re-pierce".to_owned());
            }
            if process.cool_ms > 0 {
                tags.push(format!("cool {} ms", process.cool_ms));
            }
            writeln!(
                text,
                "  {geometry}{}",
                if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join(", ")) }
            )
            .unwrap();
        }
    }
    text
}

fn lead(shape: LeadShape) -> Lead {
    Lead { shape, length: Millimeters(2.), radius: Millimeters(1.5), angle: Degrees(45.) }
}

/// The plate with nothing switched on: contours as drawn, in drawing order.
#[test]
fn plain_plate() {
    insta::assert_snapshot!(render(
        &openlaser_prep::prepare(&plate(), &Features::default()).unwrap()
    ));
}

/// Holes first, kerf compensated automatically, line leads on the waste
/// side of every contour.
#[test]
fn kerf_and_line_leads_inner_first() {
    let features = Features {
        leads: Some(Leads {
            entry: Some(lead(LeadShape::Line)),
            exit: Some(lead(LeadShape::Line)),
            side: Side::Auto,
            closed_only: true,
            overrides: Vec::new(),
        }),
        kerf: Some(Kerf { width: Millimeters(0.2), side: Side::Auto }),
        order: CutOrder {
            strategy: OrderStrategy::AsDrawn,
            inner_first: true,
            circles_first: false,
            spread_heat: false,
        },
        ..Features::default()
    };
    insta::assert_snapshot!(render(&openlaser_prep::prepare(&plate(), &features).unwrap()));
}

/// Arc leads, two joints per contour with re-piercing, and cooling at the
/// start and at square corners.
#[test]
fn joints_cooling_and_arc_leads() {
    let features = Features {
        leads: Some(Leads {
            entry: Some(lead(LeadShape::Arc)),
            exit: None,
            side: Side::Auto,
            closed_only: false,
            overrides: Vec::new(),
        }),
        joints: Some(Joints {
            placement: JointPlacement::Count(2),
            open_start: false,
            width: Millimeters(0.5),
            minimum_size: Millimeters(8.),
            outer_only: false,
            behaviour: JointBehaviour::Power(Percent(20.)),
            slow_speed: Some(MmPerSecond(3.)),
            repierce: true,
        }),
        cooling: Some(Cooling {
            dwell: Milliseconds(300),
            placement: CoolingPlacement::Automatic {
                at_start: true,
                corners_below: Some(Degrees(90.)),
            },
        }),
        ..Features::default()
    };
    insta::assert_snapshot!(render(&openlaser_prep::prepare(&plate(), &features).unwrap()));
}

/// The left hole bridged into the slot, cut nearest-first with circles
/// ahead of everything else.
#[test]
fn bridged_holes_nearest_first() {
    let features = Features {
        bridges: Some(Bridges {
            width: Millimeters(2.),
            connections: vec![Bridge {
                first: Pick { contour: 1, point: Point::new(25., 30.) },
                second: Pick { contour: 3, point: Point::new(35., 30.) },
            }],
        }),
        order: CutOrder {
            strategy: OrderStrategy::Nearest,
            inner_first: false,
            circles_first: true,
            spread_heat: false,
        },
        ..Features::default()
    };
    insta::assert_snapshot!(render(&openlaser_prep::prepare(&plate(), &features).unwrap()));
}

/// Every contour started a quarter of the way round, run clockwise, with
/// a one millimetre overcut, and line-arc leads.
#[test]
fn clockwise_manual_start_with_overcut() {
    let features = Features {
        leads: Some(Leads {
            entry: Some(lead(LeadShape::LineArc)),
            exit: Some(lead(LeadShape::LineArc)),
            side: Side::Auto,
            closed_only: false,
            overrides: Vec::new(),
        }),
        start: Start {
            position: StartPosition::Manual(0.25),
            direction: Direction::Clockwise,
            spots: vec![],
        },
        seam: Seam::Overcut(Millimeters(1.)),
        ..Features::default()
    };
    insta::assert_snapshot!(render(&openlaser_prep::prepare(&plate(), &features).unwrap()));
}

/// A drawing with crossing contours, a bad feature value and a bridge
/// whose pick is off its contour are refused with their reasons.
#[test]
fn refusals_name_their_cause() {
    let mut crossing = plate();
    crossing.contours.push(crossing.contours[0].translated(Point::new(50., 0.)));
    assert!(matches!(
        openlaser_prep::prepare(&crossing, &Features::default()),
        Err(openlaser_prep::Error::Topology(_))
    ));
    let bad_kerf = Features {
        kerf: Some(Kerf { width: Millimeters(-1.), side: Side::Auto }),
        ..Features::default()
    };
    assert!(matches!(
        openlaser_prep::prepare(&plate(), &bad_kerf),
        Err(openlaser_prep::Error::Feature { feature: "kerf", .. })
    ));
    let off = Features {
        bridges: Some(Bridges {
            width: Millimeters(2.),
            connections: vec![Bridge {
                first: Pick { contour: 1, point: Point::new(0., 0.) },
                second: Pick { contour: 2, point: Point::new(75., 30.) },
            }],
        }),
        ..Features::default()
    };
    assert!(matches!(
        openlaser_prep::prepare(&plate(), &off),
        Err(openlaser_prep::Error::Bridge { index: 1, .. })
    ));
}

/// A skipped layer is left uncut while the other contours keep their
/// indices, a tap snaps to the nearest kept contour or to nothing, and a
/// start chosen on the drawing moves that contour's start alone.
#[test]
fn skipped_layers_picks_and_chosen_starts() {
    let mut drawing = plate();
    drawing.contours[1].layer = "NOTES".into();
    let features = Features { skip_layers: vec!["NOTES".into()], ..Features::default() };
    let toolpath = openlaser_prep::prepare(&drawing, &features).unwrap();
    assert_eq!(toolpath.contours.len(), drawing.contours.len() - 1);
    assert!(toolpath.contours.iter().all(|c| !c.sources.contains(&1)));
    let hole = drawing.contours[2].start().unwrap();
    let (spot, point) =
        openlaser_prep::pick(&drawing, &features, hole + Point::new(0.3, 0.), 1., false)
            .unwrap()
            .unwrap();
    assert_eq!(spot.contour, 2);
    assert!(point.distance(hole) < 1e-6);
    assert!(
        openlaser_prep::pick(&drawing, &features, Point::new(-500., -500.), 1., false)
            .unwrap()
            .is_none()
    );
    let started = Features {
        start: Start {
            position: StartPosition::Keep,
            direction: Direction::Keep,
            spots: vec![Spot { contour: 0, fraction: 0.5 }],
        },
        ..features
    };
    let toolpath = openlaser_prep::prepare(&drawing, &started).unwrap();
    let outer = toolpath.contours.iter().find(|c| c.sources == [0]).unwrap();
    let halfway = drawing.contours[0].point_at(drawing.contours[0].length() / 2.).unwrap();
    assert!(outer.segments[0].curve.start().distance(halfway) < 1e-9);
}

fn square_at(x: f64) -> openlaser_core::geometry::Contour {
    let p =
        [Point::new(x, 0.), Point::new(x + 10., 0.), Point::new(x + 10., 10.), Point::new(x, 10.)];
    openlaser_core::geometry::Contour {
        layer: "0".into(),
        curves: (0..4).map(|i| Curve::Line { start: p[i], end: p[(i + 1) % 4] }).collect(),
    }
}

/// Manual points on both source contours survive a join and reversal at
/// their physical positions; the paired segments retain each source interval.
#[test]
fn manual_features_follow_joined_geometry_in_both_directions() {
    let drawing = Drawing { contours: vec![square_at(0.), square_at(20.)] };
    let mut features = Features {
        bridges: Some(Bridges {
            width: Millimeters(2.),
            connections: vec![Bridge {
                first: Pick { contour: 0, point: Point::new(10., 5.) },
                second: Pick { contour: 1, point: Point::new(20., 5.) },
            }],
        }),
        start: Start { spots: vec![Spot { contour: 0, fraction: 0.875 }], ..Start::default() },
        joints: Some(Joints {
            placement: JointPlacement::Manual(vec![
                Spot { contour: 0, fraction: 0.125 },
                Spot { contour: 1, fraction: 0.125 },
            ]),
            width: Millimeters(1.),
            minimum_size: Millimeters(0.),
            outer_only: false,
            open_start: false,
            behaviour: JointBehaviour::LaserOff,
            slow_speed: None,
            repierce: false,
        }),
        cooling: Some(Cooling {
            dwell: Milliseconds(50),
            placement: CoolingPlacement::Manual(vec![
                Spot { contour: 0, fraction: 0.625 },
                Spot { contour: 1, fraction: 0.625 },
            ]),
        }),
        ..Features::default()
    };
    for direction in [Direction::Keep, Direction::Reverse] {
        features.start.direction = direction;
        let prepared = openlaser_prep::prepare(&drawing, &features).unwrap();
        assert_eq!(prepared.contours.len(), 1);
        let contour = &prepared.contours[0];
        assert!(contour.segments[0].curve.start().distance(Point::new(0., 5.)) < 1e-8);
        let cooling: Vec<_> = contour
            .segments
            .iter()
            .filter(|s| s.process.cool_ms > 0)
            .map(|s| s.curve.start())
            .collect();
        assert_eq!(cooling.len(), 2);
        for x in [5., 25.] {
            assert!(cooling.iter().any(|p| p.distance(Point::new(x, 10.)) < 1e-8));
        }
        for owner in [0, 1] {
            let joints: Vec<_> = contour
                .segments
                .iter()
                .filter(|s| {
                    s.process.joint && s.source.is_some_and(|source| source.contour == owner)
                })
                .collect();
            assert!((joints.iter().map(|s| s.curve.length()).sum::<f64>() - 1.).abs() < 1e-8);
        }
    }
}

/// Splitting maps each manual point to one surviving result. A point in
/// the removed channel is refused by name instead of silently disappearing.
#[test]
fn split_features_have_one_owner_and_removed_points_are_refused() {
    let drawing = Drawing { contours: vec![square_at(0.)] };
    let mut features = Features {
        bridges: Some(Bridges {
            width: Millimeters(2.),
            connections: vec![Bridge {
                first: Pick { contour: 0, point: Point::new(0., 5.) },
                second: Pick { contour: 0, point: Point::new(10., 5.) },
            }],
        }),
        cooling: Some(Cooling {
            dwell: Milliseconds(50),
            placement: CoolingPlacement::Manual(vec![
                Spot { contour: 0, fraction: 0.125 },
                Spot { contour: 0, fraction: 0.625 },
            ]),
        }),
        ..Features::default()
    };
    let prepared = openlaser_prep::prepare(&drawing, &features).unwrap();
    assert_eq!(prepared.contours.len(), 2);
    assert!(
        prepared.contours.iter().all(|c| c
            .segments
            .iter()
            .filter(|s| s.process.cool_ms > 0)
            .count()
            == 1)
    );
    features.start.spots.push(Spot { contour: 0, fraction: 0.875 });
    assert!(matches!(
        openlaser_prep::prepare(&drawing, &features),
        Err(openlaser_prep::Error::Feature { feature: "start", .. })
    ));
}

/// A sheet of many small parts prepares past the former 5000-contour
/// limit; a drawing over the limit, or ordered by a search too large for
/// it, is refused with the limit and what to do instead.
#[test]
fn large_sheets_prepare_and_oversized_ones_name_the_limit() {
    use openlaser_core::geometry::Contour;
    let square = |i: usize| {
        let corner = Point::new(
            f64::from(u32::try_from(i % 100).unwrap()) * 3.,
            f64::from(u32::try_from(i / 100).unwrap()) * 3.,
        );
        let corners =
            [(0., 0.), (2., 0.), (2., 2.), (0., 2.)].map(|(x, y)| corner + Point::new(x, y));
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|k| Curve::Line { start: corners[k], end: corners[(k + 1) % 4] })
                .collect(),
        }
    };
    let sheet = Drawing { contours: (0..6000).map(square).collect() };
    let toolpath = openlaser_prep::prepare(&sheet, &Features::default()).unwrap();
    assert_eq!(toolpath.contours.len(), 6000);

    let over = Drawing { contours: (0..=openlaser_prep::MAX_CONTOURS).map(square).collect() };
    let refused = openlaser_prep::prepare(&over, &Features::default()).unwrap_err().to_string();
    assert!(refused.contains(&format!("up to {}", openlaser_prep::MAX_CONTOURS)), "{refused}");

    let nearest = Features {
        order: CutOrder { strategy: OrderStrategy::Nearest, ..CutOrder::default() },
        ..Features::default()
    };
    let many = Drawing { contours: (0..10_001).map(square).collect() };
    let refused = openlaser_prep::prepare(&many, &nearest).unwrap_err().to_string();
    assert!(refused.contains("nearest-next ordering takes up to 10000"), "{refused}");
}
