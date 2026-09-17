// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection groups and corresponding lead ownership across durable edits.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use openlaser_core::features::{Features, Lead, LeadOverride, LeadShape, Leads, Side};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point, Transform};
use openlaser_core::grouping::Grouping;
use openlaser_core::units::{Degrees, Millimeters};
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::document::PathKind;
use openlaser_server::{Coordinator, draft::Draft, machine};

fn parts() -> Drawing {
    let mut contours = Vec::new();
    for x in [0., 100., 200.] {
        let points = [[x, 0.], [x + 40., 0.], [x + 40., 20.], [x, 20.]];
        contours.push(Contour {
            layer: "cut".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
                .collect(),
        });
        for dx in [10., 30.] {
            contours.push(Contour {
                layer: "cut".into(),
                curves: vec![Curve::circle(Point::new(x + dx, 10.), 3.)],
            });
        }
    }
    Drawing { contours }
}

fn lead(length: f64) -> Lead {
    Lead {
        shape: LeadShape::Line,
        length: Millimeters(length),
        radius: Millimeters(1.),
        angle: Degrees(45.),
    }
}

#[test]
fn only_corresponding_right_holes_change_on_two_selected_parts() {
    let drawing = parts();
    let mut draft = Draft::new("parts".into(), 9);
    draft.features.leads = Some(Leads {
        entry: Some(lead(1.)),
        exit: Some(lead(0.5)),
        side: Side::Auto,
        closed_only: true,
        overrides: Vec::new(),
    });
    draft.prepare(&drawing);
    draft.transform(&[3], Some(Transform([0., 1., -1., 0., 300., 0.]))).unwrap();
    draft.prepare(&drawing);
    let before = draft.preview.clone().unwrap();
    let right = before.contours.iter().find(|c| c.sources == [2]).unwrap().matching_contour;
    assert!(right.is_some());
    let targets: Vec<_> = before
        .contours
        .iter()
        .filter(|c| c.matching_contour == right && c.sources[0] < 6)
        .map(|c| c.lead_target.unwrap())
        .collect();
    assert_eq!(targets.iter().map(|s| s.contour).collect::<Vec<_>>(), [2, 5]);
    draft.features.leads.as_mut().unwrap().overrides = targets
        .into_iter()
        .map(|location| LeadOverride { location, entry: Some(lead(2.)), exit: None })
        .collect();
    draft.prepare(&drawing);
    let after = draft.preview.as_ref().unwrap();
    for contour in &after.contours {
        let previous = before.contours.iter().find(|c| c.sources == contour.sources).unwrap();
        let changed = contour.paths != previous.paths;
        assert_eq!(changed, contour.sources == [2] || contour.sources == [5]);
        let unaffected = |c: &openlaser_server::document::PreviewContour| {
            c.paths.iter().filter(|p| p.kind != PathKind::LeadIn).cloned().collect::<Vec<_>>()
        };
        assert_eq!(unaffected(contour), unaffected(previous));
    }
    // Manual assembly groups do not collapse the left and right contour roles.
    draft.set_grouped(&[0, 3], true);
    draft.prepare(&drawing);
    assert_eq!(draft.groups, [vec![0, 1, 2, 3, 4, 5], vec![6, 7, 8]]);
    assert_eq!(
        draft
            .preview
            .as_ref()
            .unwrap()
            .contours
            .iter()
            .filter(|c| c.matching_contour == right)
            .count(),
        3
    );
}

#[test]
fn partial_selection_groups_expand_and_paste_delete_undo_preserve_membership() {
    let drawing = parts();
    let mut draft = Draft::new("parts".into(), 9);
    draft.prepare(&drawing);
    draft.set_grouped(&[1, 5], true);
    draft.prepare(&drawing);
    assert_eq!(draft.groups, [vec![0, 1, 2, 3, 4, 5], vec![6, 7, 8]]);
    draft.set_grouped(&[2], false);
    draft.prepare(&drawing);
    assert_eq!(draft.groups, [vec![0], vec![1], vec![2], vec![3], vec![4], vec![5], vec![6, 7, 8]]);
    draft.undo().unwrap();
    draft.prepare(&drawing);
    let pasted: Vec<_> = (0..6).map(|i| (i, Transform::translation(Point::new(0., 50.)))).collect();
    draft.add_grouped(&pasted, &[], &Grouping(vec![(0..6).collect()]));
    draft.prepare(&drawing);
    assert_eq!(draft.groups.last().unwrap(), &[9, 10, 11, 12, 13, 14]);
    draft.remove(&drawing, &[0, 1, 2, 3, 4, 5]).unwrap();
    draft.prepare(&drawing);
    assert_eq!(draft.groups, [vec![0, 1, 2], vec![3, 4, 5, 6, 7, 8]]);
    draft.undo().unwrap();
    draft.prepare(&drawing);
    assert_eq!(draft.groups.len(), 3);
    let mut restored: Draft = serde_json::from_slice(&serde_json::to_vec(&draft).unwrap()).unwrap();
    restored.validate_saved(9).unwrap();
    restored.prepare(&drawing);
    assert_eq!(restored.groups, draft.groups);
    restored.grouping.0.push(vec![0]);
    assert!(restored.validate_saved(9).is_err());
}

#[test]
fn copying_repeated_sources_and_high_copy_ids_keeps_every_instance_unique() {
    let mut draft = Draft::new("parts".into(), 3);
    draft.placed[0].copy = u32::MAX;
    let contours: Vec<_> =
        [0, 1, 2, 0, 1, 2].into_iter().map(|i| (i, Transform::IDENTITY)).collect();
    draft.add_grouped(&contours, &[], &Grouping(vec![(0..6).collect()]));
    let instances: std::collections::BTreeSet<_> =
        draft.placed.iter().map(|p| (p.source, p.copy)).collect();
    assert_eq!(instances.len(), draft.placed.len());
    assert_eq!(draft.placed[3].copy, draft.placed[5].copy);
    assert_eq!(draft.placed[6].copy, draft.placed[8].copy);
    assert_ne!(draft.placed[3].copy, draft.placed[6].copy);
}

#[tokio::test]
async fn saved_and_retained_jobs_restore_grouping_and_undo_after_restart() {
    let (_sim, shared) = common::start("grouping").await;
    {
        let mut c = shared.lock().await;
        let part = c.library.add_part("parts.dxf", b"grouping fixture", parts()).unwrap();
        c.open_part(&part.id).unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Grouping test".into(),
                laser: openlaser_core::LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.set_features(Features::default()).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    shared.lock().await.set_grouped(&[0, 3], true).unwrap();
    machine::prepare(&shared).await.unwrap();
    let (job, config, grouping) = {
        let mut c = shared.lock().await;
        let job = c.save_job("Grouped parts").unwrap();
        let draft = c.draft.as_ref().unwrap();
        assert!(!draft.dirty());
        (job.id, c.config.clone(), draft.grouping.clone())
    };
    shared.lock().await.set_grouped(&[0], false).unwrap();
    machine::prepare(&shared).await.unwrap();
    assert!(shared.lock().await.draft.as_ref().unwrap().dirty());
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    common::until(&reopened, 5, |d| d.draft.as_ref().is_some_and(|d| d.preview.is_some())).await;
    assert_eq!(reopened.lock().await.draft.as_ref().unwrap().groups.len(), 7);
    reopened.lock().await.undo().unwrap();
    machine::prepare(&reopened).await.unwrap();
    {
        let mut c = reopened.lock().await;
        assert_eq!(c.draft.as_ref().unwrap().grouping, grouping);
        assert_eq!(c.library.job(&job).unwrap().grouping, grouping);
        c.open_job(&job).unwrap();
    }
    machine::prepare(&reopened).await.unwrap();
    assert_eq!(reopened.lock().await.draft.as_ref().unwrap().groups.len(), 2);
    openlaser_server::shutdown(&reopened).await.unwrap();
}
