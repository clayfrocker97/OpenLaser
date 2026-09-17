// SPDX-License-Identifier: GPL-3.0-or-later
//! A handle edits one prepared contour, including topology and order changes.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known geometry fixtures")]

use openlaser_core::features::{
    Bridge, Bridges, CommonEdges, Direction, Features, Kerf, Lead, LeadOverride, LeadShape, Leads,
    OrderStrategy, Pick, Side, Spot,
};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use openlaser_core::toolpath::{LeadRole, PreparedContour, PreparedSegment};
use openlaser_core::units::{Degrees, Millimeters};

fn square(x: f64) -> Contour {
    let points = [[x, 0.], [x + 10., 0.], [x + 10., 10.], [x, 10.]];
    Contour {
        layer: "cut".into(),
        curves: (0..4)
            .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
            .collect(),
    }
}
fn lead(length: f64, angle: f64) -> Lead {
    Lead {
        shape: LeadShape::Line,
        length: Millimeters(length),
        radius: Millimeters(1.),
        angle: Degrees(angle),
    }
}
fn features() -> Features {
    Features {
        leads: Some(Leads {
            entry: Some(lead(2., 45.)),
            exit: Some(lead(1., 60.)),
            side: Side::Outside,
            closed_only: true,
            overrides: Vec::new(),
        }),
        ..Features::default()
    }
}
fn role(contour: &PreparedContour, role: Option<LeadRole>) -> Vec<PreparedSegment> {
    contour.segments.iter().filter(|s| s.process.lead == role).cloned().collect()
}
fn entry_length(contour: &PreparedContour) -> f64 {
    role(contour, Some(LeadRole::Entry)).iter().map(|s| s.curve.length()).sum()
}

#[test]
fn repeated_handle_edits_change_only_the_target_entry_and_inherit_other_defaults() {
    let drawing = Drawing { contours: vec![square(0.), square(30.), square(60.)] };
    let mut settings = features();
    let original = openlaser_prep::prepare(&drawing, &settings).unwrap();
    let location = original.contours[1].lead_target.unwrap();
    let globals = settings.leads.clone().unwrap();
    for (length, angle) in [(4., 70.), (3., 120.)] {
        settings.leads.as_mut().unwrap().overrides =
            vec![LeadOverride { location, entry: Some(lead(length, angle)), exit: None }];
        let edited = openlaser_prep::prepare(&drawing, &settings).unwrap();
        for (i, contour) in edited.contours.iter().enumerate() {
            if i == 1 {
                assert!((entry_length(contour) - length).abs() < 1e-9);
                assert_eq!(role(contour, None), role(&original.contours[i], None));
                assert_eq!(
                    role(contour, Some(LeadRole::Exit)),
                    role(&original.contours[i], Some(LeadRole::Exit))
                );
                assert_ne!(
                    role(contour, Some(LeadRole::Entry)),
                    role(&original.contours[i], Some(LeadRole::Entry))
                );
                assert_eq!(contour.lead_target, Some(location));
            } else {
                assert_eq!(contour, &original.contours[i]);
            }
        }
        assert_eq!(settings.leads.as_ref().unwrap().entry, globals.entry);
        assert_eq!(settings.leads.as_ref().unwrap().exit, globals.exit);
    }
    settings.leads.as_mut().unwrap().entry = Some(lead(5., 90.));
    let updated = openlaser_prep::prepare(&drawing, &settings).unwrap();
    for (i, contour) in updated.contours.iter().enumerate() {
        assert!((entry_length(contour) - if i == 1 { 3. } else { 5. }).abs() < 1e-9);
    }
    settings.leads.as_mut().unwrap().entry = None;
    assert!(
        openlaser_prep::prepare(&drawing, &settings)
            .unwrap()
            .contours
            .iter()
            .all(|c| entry_length(c) == 0.)
    );
}

#[test]
fn owner_survives_reordering_skipped_layers_compensation_and_direction() {
    let mut drawing = Drawing { contours: vec![square(0.), square(30.), square(60.)] };
    drawing.contours[0].layer = "skip".into();
    let mut settings = features();
    let location =
        openlaser_prep::prepare(&drawing, &settings).unwrap().contours[1].lead_target.unwrap();
    settings.leads.as_mut().unwrap().overrides.push(LeadOverride {
        location,
        entry: Some(lead(4., 70.)),
        exit: None,
    });
    settings.skip_layers = vec!["skip".into()];
    settings.order.strategy = OrderStrategy::Manual(vec![2, 1]);
    settings.start.direction = Direction::Reverse;
    settings.kerf = Some(Kerf { width: Millimeters(0.2), side: Side::Auto });
    let path = openlaser_prep::prepare(&drawing, &settings).unwrap();
    assert_eq!(path.contours[0].sources, [2]);
    assert_eq!(path.contours[1].sources, [1]);
    assert!((entry_length(&path.contours[0]) - 2.).abs() < 1e-9);
    assert!((entry_length(&path.contours[1]) - 4.).abs() < 1e-9);
    assert_eq!(path.contours[1].lead_target, Some(location));
    settings.skip_layers.push("cut".into());
    settings.order.strategy = OrderStrategy::AsDrawn;
    assert!(openlaser_prep::prepare(&drawing, &settings).unwrap().contours.is_empty());
}

#[test]
fn bridge_halves_and_common_edges_do_not_share_a_lead_owner() {
    let drawing = Drawing { contours: vec![square(0.)] };
    let mut settings = features();
    settings.bridges = Some(Bridges {
        width: Millimeters(2.),
        connections: vec![Bridge {
            first: Pick { contour: 0, point: Point::new(5., 0.) },
            second: Pick { contour: 0, point: Point::new(5., 10.) },
        }],
    });
    let before = openlaser_prep::prepare(&drawing, &settings).unwrap();
    assert_eq!(before.contours.len(), 2);
    assert_ne!(before.contours[0].lead_target, before.contours[1].lead_target);
    let location = before.contours[0].lead_target.unwrap();
    settings.leads.as_mut().unwrap().overrides.push(LeadOverride {
        location,
        entry: Some(lead(3., 65.)),
        exit: None,
    });
    let after = openlaser_prep::prepare(&drawing, &settings).unwrap();
    assert!((entry_length(&after.contours[0]) - 3.).abs() < 1e-9);
    assert_eq!(before.contours[1], after.contours[1]);

    let drawing = Drawing { contours: vec![square(0.), square(10.)] };
    let mut settings = features();
    settings.common =
        Some(CommonEdges { contours: vec![0, 1], tolerance: 0.01, allow_overcut: false });
    let before = openlaser_prep::prepare(&drawing, &settings).unwrap();
    assert!(before.contours.iter().all(|c| c.sources == [0, 1]));
    let location =
        before.contours.iter().find_map(|c| c.lead_target.filter(|s| s.contour == 1)).unwrap();
    settings.leads.as_mut().unwrap().overrides.push(LeadOverride {
        location,
        entry: Some(lead(3., 65.)),
        exit: None,
    });
    let after = openlaser_prep::prepare(&drawing, &settings).unwrap();
    for (a, b) in before.contours.iter().zip(&after.contours) {
        if a.lead_target.unwrap().contour == 0 {
            assert_eq!(a, b);
        }
    }
    assert!(
        after
            .contours
            .iter()
            .any(|c| c.lead_target == Some(location) && (entry_length(c) - 3.).abs() < 1e-9)
    );
}

#[test]
fn removed_anchors_conflicting_joins_and_invalid_edits_are_explicit_errors() {
    let drawing = Drawing { contours: vec![square(0.), square(14.)] };
    let base = features();
    for edit in [
        LeadOverride {
            location: Spot { contour: 2, fraction: 0. },
            entry: Some(lead(3., 60.)),
            exit: None,
        },
        LeadOverride {
            location: Spot { contour: 0, fraction: 1.1 },
            entry: Some(lead(3., 60.)),
            exit: None,
        },
        LeadOverride {
            location: Spot { contour: 0, fraction: 0. },
            entry: Some(lead(-1., 60.)),
            exit: None,
        },
        LeadOverride { location: Spot { contour: 0, fraction: 0. }, entry: None, exit: None },
    ] {
        let mut settings = base.clone();
        settings.leads.as_mut().unwrap().overrides.push(edit);
        assert!(openlaser_prep::prepare(&drawing, &settings).is_err());
    }
    let mut settings = base;
    settings.bridges = Some(Bridges {
        width: Millimeters(2.),
        connections: vec![Bridge {
            first: Pick { contour: 0, point: Point::new(10., 5.) },
            second: Pick { contour: 1, point: Point::new(14., 5.) },
        }],
    });
    settings.leads.as_mut().unwrap().overrides = (0..2)
        .map(|contour| LeadOverride {
            location: Spot { contour, fraction: 0.125 },
            entry: Some(lead(3., 60.)),
            exit: None,
        })
        .collect();
    assert!(
        openlaser_prep::prepare(&drawing, &settings)
            .unwrap_err()
            .to_string()
            .contains("more than one lead override")
    );
    settings.leads.as_mut().unwrap().overrides = vec![LeadOverride {
        location: Spot { contour: 0, fraction: 0.375 },
        entry: Some(lead(3., 60.)),
        exit: None,
    }];
    assert!(
        openlaser_prep::prepare(&drawing, &settings)
            .unwrap_err()
            .to_string()
            .contains("removed or made ambiguous")
    );
}

#[test]
fn a_whole_sheet_of_local_leads_uses_work_proportional_to_its_geometry() {
    let drawing = Drawing { contours: (0..4998).map(|i| square(f64::from(i) * 30.)).collect() };
    let mut settings = features();
    settings.leads.as_mut().unwrap().overrides = (0..drawing.contours.len())
        .map(|contour| LeadOverride {
            location: Spot { contour, fraction: 0.125 },
            entry: Some(lead(4., 70.)),
            exit: None,
        })
        .collect();
    let path = openlaser_prep::prepare(&drawing, &settings).unwrap();
    assert_eq!(path.contours.len(), 4998);
    for contour in &path.contours {
        assert!((entry_length(contour) - 4.).abs() < 1e-8);
        assert_eq!(contour.lead_target.unwrap().contour, contour.sources[0]);
        assert!(
            (role(contour, Some(LeadRole::Exit)).iter().map(|s| s.curve.length()).sum::<f64>()
                - 1.)
                .abs()
                < 1e-8
        );
    }
}
