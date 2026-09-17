// SPDX-License-Identifier: GPL-3.0-or-later
//! Calibration defaults, frozen jobs, position invalidation and nominal coupons.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use common::start;
use openlaser_core::LaserMode;
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use openlaser_correction::Measurement;
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::correction::CorrectionChange;
use openlaser_server::machine;

fn square() -> Drawing {
    let p = [
        Point::new(100., 100.),
        Point::new(110., 100.),
        Point::new(110., 110.),
        Point::new(100., 110.),
    ];
    Drawing {
        contours: vec![Contour {
            layer: "Cut".into(),
            curves: (0..4).map(|i| Curve::Line { start: p[i], end: p[(i + 1) % 4] }).collect(),
        }],
    }
}

#[tokio::test]
async fn enabled_profile_follows_new_dxfs_but_saved_jobs_keep_their_snapshot() {
    let (_sim, shared) = start("correction-snapshots").await;
    let recipe = {
        let mut c = shared.lock().await;
        let view = c.correction_view(LaserMode::Fiber).unwrap();
        let first = [Some(Measurement { x: 101., y: 99. }); 9];
        c.save_correction(CorrectionChange::Measure {
            revision: view.revision,
            mode: LaserMode::Fiber,
            measurements: Box::new(first),
            apply: true,
        })
        .unwrap();
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        let part = c.library.add_part("corrected.DXF", b"first drawing", square()).unwrap();
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        recipe
    };
    machine::prepare(&shared).await.unwrap();
    shared.lock().await.set_origin([100., 100.]).unwrap();
    machine::compile(&shared, false).await.unwrap();
    let (job, first) = {
        let mut c = shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        let profile = d.correction.clone().unwrap();
        let map = openlaser_correction::Map::new(&profile).unwrap();
        let zero = Point::from(d.zero.unwrap());
        let paths = &d.compiled.as_ref().unwrap().view.moves;
        let cut =
            paths.iter().find(|m| m.kind == openlaser_server::document::PathKind::Cut).unwrap();
        let command = Point::from(cut.points[0]) + zero;
        let physical = map.forward(command);
        let wanted = Point::from(d.preview.as_ref().unwrap().contours[0].start) + zero;
        assert!(physical.distance(wanted) < 0.02);
        let job = c.save_job("Frozen correction").unwrap();
        let view = c.correction_view(LaserMode::Fiber).unwrap();
        c.save_correction(CorrectionChange::Measure {
            revision: view.revision,
            mode: LaserMode::Fiber,
            measurements: Box::new([Some(Measurement { x: 100.5, y: 100.2 }); 9]),
            apply: true,
        })
        .unwrap();
        c.open_job(&job.id).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().correction, Some(profile.clone()));
        (job, profile)
    };
    machine::prepare(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    {
        let mut c = shared.lock().await;
        let origin = c.draft.as_ref().unwrap().origin().unwrap();
        c.set_origin([origin[0] + 30., origin[1] + 20.]).unwrap();
        assert!(c.draft.as_ref().unwrap().compiled.is_none());
        assert_eq!(c.library.job(&job.id).unwrap().correction, Some(first));
        let new = c.library.add_part("new.dxf", b"new drawing", square()).unwrap();
        c.open_part(&new.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        assert_eq!(
            c.draft.as_ref().unwrap().correction.as_ref().unwrap().measurements[0],
            Measurement { x: 100.5, y: 100.2 }
        );
        c.correction_coupon(LaserMode::Fiber).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        let d = c.draft.as_ref().unwrap();
        assert!(d.calibration);
        assert!(d.correction.is_none());
        assert_eq!(d.placed.len(), 9);
        assert_eq!(d.zero, Some([0., 0.]));
    }
    machine::prepare(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    let c = shared.lock().await;
    assert_eq!(
        c.draft
            .as_ref()
            .unwrap()
            .compiled
            .as_ref()
            .unwrap()
            .view
            .plan
            .iter()
            .filter(|p| p.kind == openlaser_server::document::PassKind::Cut)
            .count(),
        9
    );
}
