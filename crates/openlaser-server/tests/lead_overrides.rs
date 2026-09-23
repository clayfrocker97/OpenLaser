// SPDX-License-Identifier: GPL-3.0-or-later
//! Local lead ownership through editing, copies, history and durable jobs.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use openlaser_core::features::{Features, Lead, LeadOverride, LeadShape, Leads, Side};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point, Transform};
use openlaser_core::units::{Degrees, Millimeters};
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::document::PathKind;
use openlaser_server::{Coordinator, draft::Draft, machine};

fn square() -> Drawing {
    let points = [[0., 0.], [10., 0.], [10., 10.], [0., 10.]];
    Drawing {
        contours: vec![Contour {
            layer: "cut".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
                .collect(),
        }],
    }
}
fn lead(length: f64) -> Lead {
    Lead {
        shape: LeadShape::Line,
        length: Millimeters(length),
        radius: Millimeters(1.),
        angle: Degrees(60.),
    }
}
fn entry_lengths(draft: &Draft) -> Vec<f64> {
    draft
        .prepared
        .as_ref()
        .unwrap()
        .preview
        .contours
        .iter()
        .map(|c| {
            c.paths
                .iter()
                .filter(|s| s.kind == PathKind::LeadIn)
                .flat_map(|s| s.points.windows(2))
                .map(|p| (p[0][0] - p[1][0]).hypot(p[0][1] - p[1][1]))
                .sum()
        })
        .collect()
}
fn assert_lengths(draft: &Draft, expected: &[f64]) {
    let actual = entry_lengths(draft);
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 1e-8, "{actual:?}");
    }
}

#[tokio::test]
async fn local_edits_follow_copies_transforms_deletion_undo_and_saved_jobs_after_restart() {
    let (_sim, shared) = common::start("local-leads").await;
    {
        let mut c = shared.lock().await;
        let part =
            c.library.add_part("square.dxf", b"local lead ownership fixture", square()).unwrap();
        c.open_part(&part.id).unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Lead test".into(),
                laser: openlaser_core::LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.set_features(Features {
            leads: Some(Leads {
                entry: Some(lead(2.)),
                exit: Some(lead(1.)),
                side: Side::Outside,
                closed_only: true,
                overrides: Vec::new(),
            }),
            ..Features::default()
        })
        .unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    {
        let mut c = shared.lock().await;
        let draft = c.draft.as_ref().unwrap();
        let target = draft.prepared.as_ref().unwrap().preview.contours[0].lead_target.unwrap();
        let mut features = draft.current.features.clone();
        features.leads.as_mut().unwrap().overrides.push(LeadOverride {
            location: target,
            entry: Some(lead(3.)),
            exit: None,
        });
        c.set_features(features).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    assert_lengths(shared.lock().await.draft.as_ref().unwrap(), &[3.]);
    shared.lock().await.undo().unwrap();
    machine::prepare(&shared).await.unwrap();
    assert_lengths(shared.lock().await.draft.as_ref().unwrap(), &[2.]);
    shared.lock().await.redo().unwrap();
    machine::prepare(&shared).await.unwrap();
    {
        let mut c = shared.lock().await;
        let edits =
            c.draft.as_ref().unwrap().current.features.leads.as_ref().unwrap().overrides.clone();
        assert_eq!(
            c.add_with_leads(&[(0, Transform::translation(Point::new(30., 0.)))], &edits).unwrap(),
            [1]
        );
    }
    machine::prepare(&shared).await.unwrap();
    assert_lengths(shared.lock().await.draft.as_ref().unwrap(), &[3., 3.]);
    {
        let mut c = shared.lock().await;
        let mut features = c.draft.as_ref().unwrap().current.features.clone();
        features.leads.as_mut().unwrap().overrides[1].entry = Some(lead(5.));
        c.set_features(features).unwrap();
        // Mirror and scale the copy. Leads retain their machining dimensions.
        c.transform(&[1], Some(Transform([-2., 0., 0., 2., 120., 20.]))).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    assert_lengths(shared.lock().await.draft.as_ref().unwrap(), &[3., 5.]);
    let (job, config, features) = {
        let mut c = shared.lock().await;
        let features = c.draft.as_ref().unwrap().current.features.clone();
        assert_eq!(features.leads.as_ref().unwrap().entry, Some(lead(2.)));
        let job = c.save_job("Local leads").unwrap();
        assert_eq!(c.library.job(&job.id).unwrap().features, features);
        (job.id, c.config.clone(), features)
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    common::until(&reopened, 5, |d| d.draft.as_ref().is_some_and(|d| d.preview.is_some())).await;
    {
        let mut c = reopened.lock().await;
        assert_eq!(c.draft.as_ref().unwrap().current.features, features);
        assert_lengths(c.draft.as_ref().unwrap(), &[3., 5.]);
        c.open_job(&job).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().current.features, features);
        c.remove(&[0]).unwrap();
        let edits = &c.draft.as_ref().unwrap().current.features.leads.as_ref().unwrap().overrides;
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].location.contour, 0);
        assert_eq!(edits[0].entry, Some(lead(5.)));
    }
    machine::prepare(&reopened).await.unwrap();
    assert_lengths(reopened.lock().await.draft.as_ref().unwrap(), &[5.]);
    reopened.lock().await.undo().unwrap();
    machine::prepare(&reopened).await.unwrap();
    assert_lengths(reopened.lock().await.draft.as_ref().unwrap(), &[3., 5.]);
    openlaser_server::shutdown(&reopened).await.unwrap();
}

#[test]
fn old_jobs_load_and_new_jobs_round_trip_without_reusing_contour_edits_on_other_drawings() {
    let old = r#"{"leads":{"entry":{"shape":"line","length":2,"radius":1,"angle":90},"side":"auto","closed_only":true}}"#;
    let mut features: Features = serde_json::from_str(old).unwrap();
    assert!(features.leads.as_ref().unwrap().overrides.is_empty());
    features.leads.as_mut().unwrap().overrides.push(LeadOverride {
        location: openlaser_core::features::Spot { contour: 7, fraction: 0.3 },
        entry: Some(lead(5.)),
        exit: None,
    });
    assert_eq!(
        serde_json::from_slice::<Features>(&serde_json::to_vec(&features).unwrap()).unwrap(),
        features
    );
    assert!(features.reusable().leads.unwrap().overrides.is_empty());
    assert_eq!(features.reusable().leads.unwrap().entry, features.leads.unwrap().entry);
}
