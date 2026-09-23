// SPDX-License-Identifier: GPL-3.0-or-later
//! Reusable positioning intent and per-run placement, on the loopback simulator.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known simulator fixtures")]
mod common;

use common::{start, until};
use openlaser_controller::state::ProgramState;
use openlaser_core::LaserMode;
use openlaser_core::features::{Features, Kerf, Lead, LeadShape, Leads, Side};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point, Transform};
use openlaser_core::units::{Degrees, Millimeters};
use openlaser_correction::Measurement;
use openlaser_library::placement::Placement;
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::correction::CorrectionChange;
use openlaser_server::document::PathKind;
use openlaser_server::nesting::StockChoice;
use openlaser_server::placement::{self, PlacementChange, PlacementMode};
use openlaser_server::{connect, machine};

async fn setup(shared: &Shared) -> openlaser_library::Id {
    let p = [[100., 80.], [120., 80.], [120., 100.], [100., 100.]];
    let drawing = Drawing {
        contours: vec![Contour {
            layer: "Cut".into(),
            curves: (0..4)
                .map(|i| Curve::Line { start: p[i].into(), end: p[(i + 1) % 4].into() })
                .collect(),
        }],
    };
    let mut c = shared.lock().await;
    let part = c.library.add_part("placement.dxf", b"placement fixture", drawing).unwrap();
    let recipe = c
        .add_recipe(&NewRecipe {
            name: "Placement steel".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 1.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    c.open_part(&part.id).unwrap();
    c.set_recipe(&recipe.id).unwrap();
    c.set_features(Features::default()).unwrap();
    drop(c);
    machine::prepare(shared).await.unwrap();
    recipe.id
}

async fn home(shared: &Shared) {
    connect::connect(shared).await.unwrap();
    machine::home(shared).await.unwrap();
    until(shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
}

async fn move_to(shared: &Shared, target: [f64; 2]) {
    machine::go_xy(shared, target).await.unwrap();
    until(shared, 30, |d| {
        d.machine.operation.is_none()
            && d.machine.feedback.as_ref().is_some_and(|f| {
                f.stationary
                    && (f.position_mm[0] - target[0]).abs() < 0.01
                    && (f.position_mm[1] - target[1]).abs() < 0.01
            })
    })
    .await;
}

fn near(actual: [f64; 2], wanted: [f64; 2]) {
    assert!(
        actual.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 0.01),
        "{actual:?} != {wanted:?}"
    );
}

#[tokio::test]
async fn offline_head_jobs_save_intent_and_fixed_jobs_save_homed_xy() {
    let (simulator, shared) = start("saved-placement").await;
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    let mut c = shared.lock().await;
    let d = c.draft.as_ref().unwrap();
    assert!(d.sheet_offset.is_none(), "offline compilation must not invent a bed location");
    assert!(d.compiled.is_some());
    assert_eq!(d.dock(), Some([100., 80.]));
    let head = c.save_job("Loose sheets").unwrap();
    let saved = c.library.job(&head.id).unwrap();
    assert_eq!(saved.placement, Some(Placement::Head {}));
    assert!(saved.sheet_offset.is_none());
    assert!(c.change_placement(PlacementChange::SetOrigin {}).is_err());
    assert!(c.draft.as_ref().unwrap().sheet_offset.is_none());
    c.set_stock(StockChoice::Rectangle { width: 250., height: 200. }).unwrap();
    drop(c);
    machine::prepare(&shared).await.unwrap();
    let control = simulator.control();
    control.clear_writes();
    let fixture = {
        let mut c = shared.lock().await;
        c.change_placement(PlacementChange::Fixed { origin: [300., 200.] }).unwrap();
        c.save_job("Fixture").unwrap()
    };
    machine::compile(&shared, false).await.unwrap();
    {
        let mut c = shared.lock().await;
        assert_eq!(
            c.library.job(&fixture.id).unwrap().placement,
            Some(Placement::Fixed { origin: [300., 200.] })
        );
        c.open_job(&fixture.id).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    let mut c = shared.lock().await;
    let d = c.draft.as_ref().unwrap();
    assert_eq!(d.origin(), Some([300., 200.]));
    assert_eq!(d.dock(), Some([0., 0.]));
    assert!(d.view().placement.saved);
    c.change_placement(PlacementChange::NewRun {}).unwrap();
    assert_eq!(c.draft.as_ref().unwrap().origin(), Some([300., 200.]));
    assert!(c.draft.as_ref().unwrap().view().placement.saved);
    assert!(control.writes().is_empty(), "editing and saving never moves the axes");
    assert!(c.change_placement(PlacementChange::Fixed { origin: [f64::NAN, 0.] }).is_err());
}

#[tokio::test]
async fn absolute_origin_captures_machine_coordinates_and_reuses_them_after_jog_and_reopen() {
    let (simulator, shared) = start("absolute-head-origin").await;
    setup(&shared).await;
    assert!(shared.lock().await.change_placement(PlacementChange::FixedHead {}).is_err());
    home(&shared).await;
    move_to(&shared, [150., 120.]).await;
    let control = simulator.control();
    control.clear_writes();
    let job = {
        let mut c = shared.lock().await;
        c.change_placement(PlacementChange::FixedHead {}).unwrap();
        near(c.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
        near(c.draft.as_ref().unwrap().sheet_offset.unwrap(), [50., 40.]);
        c.save_job("Absolute fixture").unwrap()
    };
    assert!(control.writes().is_empty(), "saving an absolute origin never moves the head");
    move_to(&shared, [175., 140.]).await;
    {
        let mut c = shared.lock().await;
        near(c.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
        c.change_placement(PlacementChange::NewRun {}).unwrap();
        near(c.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
        c.open_job(&job.id).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    {
        let mut c = shared.lock().await;
        near(c.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
        c.change_placement(PlacementChange::SetOrigin {}).unwrap();
        near(c.draft.as_ref().unwrap().origin().unwrap(), [175., 140.]);
        assert_eq!(c.document().draft.unwrap().placement.mode, PlacementMode::Fixed);
        let saved = c.save_job("Absolute fixture").unwrap();
        match c.library.job(&saved.id).unwrap().placement {
            Some(Placement::Fixed { origin }) => near(origin, [175., 140.]),
            _ => panic!("absolute coordinates must be saved with the fixture job"),
        }
        c.change_placement(PlacementChange::Head {}).unwrap();
        c.change_placement(PlacementChange::SetOrigin {}).unwrap();
        near(c.draft.as_ref().unwrap().origin().unwrap(), [175., 140.]);
        c.change_placement(PlacementChange::NewRun {}).unwrap();
        assert!(c.draft.as_ref().unwrap().sheet_offset.is_none());
    }
}

#[tokio::test]
async fn set_origin_pins_this_run_jogs_and_resume_keep_it_and_new_runs_recapture() {
    let (simulator, shared) = start("head-lifecycle").await;
    setup(&shared).await;
    home(&shared).await;
    move_to(&shared, [150., 120.]).await;
    machine::compile(&shared, false).await.unwrap();
    assert!(shared.lock().await.document().readiness.run.ok);
    let control = simulator.control();
    control.clear_writes();
    let revision = shared.lock().await.document().draft_revision;
    placement::change(&shared, PlacementChange::SetOrigin {}, revision).await.unwrap();
    assert!(control.writes().is_empty(), "Set origin must not move the axes");
    near(shared.lock().await.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
    move_to(&shared, [175., 140.]).await;
    near(shared.lock().await.draft.as_ref().unwrap().origin().unwrap(), [150., 120.]);
    machine::frame(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.operation.is_none()).await;
    let captured = {
        let c = shared.lock().await;
        let captured = c.draft.as_ref().unwrap().origin().unwrap();
        near(captured, [150., 120.]);
        assert_eq!(c.document().draft.unwrap().placement.mode, PlacementMode::Head);
        captured
    };
    move_to(&shared, [175., 140.]).await;
    placement::prepare(&shared).await.unwrap();
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().origin(), Some(captured));
    control.time_scale(1.);
    common::run(&shared).await.unwrap();
    until(&shared, 10, |d| {
        d.machine.program.as_ref().is_some_and(|p| p.started)
            && d.progress.as_ref().is_some_and(|p| p.pass.is_some() && !p.approaching)
            && d.machine.feedback.as_ref().is_some_and(|f| f.fifo.activity == 1)
    })
    .await;
    machine::hold(&shared).await.unwrap();
    until(&shared, 10, |d| d.can_resume).await;
    {
        let mut c = shared.lock().await;
        assert_eq!(
            c.document().execution.unwrap().origin.map(f64::to_bits),
            captured.map(f64::to_bits)
        );
        assert!(c.change_placement(PlacementChange::Reposition {}).is_err());
        assert!(c.change_placement(PlacementChange::SetOrigin {}).is_err());
        assert!(!c.document().readiness.set_origin.ok);
    }
    control.time_scale(20.);
    common::resume(&shared).await.unwrap();
    until(&shared, 20, |d| {
        d.machine.operation.is_none()
            && d.machine.program.as_ref().is_some_and(|p| p.state == ProgramState::Completed)
    })
    .await;
    assert_eq!(
        shared.lock().await.document().execution.unwrap().origin.map(f64::to_bits),
        captured.map(f64::to_bits)
    );
    let revision = shared.lock().await.document().draft_revision;
    placement::change(&shared, PlacementChange::NewRun {}, revision).await.unwrap();
    {
        let c = shared.lock().await;
        let doc = c.document();
        let draft = doc.draft.unwrap();
        assert!(draft.sheet_offset.is_none());
        assert!(draft.compiled.is_some(), "New run must be ready without revisiting Setup");
        assert!(doc.execution.is_none());
        assert!(doc.completed_sheet.is_none());
        assert!(doc.postflight.is_none());
        assert!(c.recovery.is_none());
    }
    move_to(&shared, [220., 180.]).await;
    placement::prepare(&shared).await.unwrap();
    near(shared.lock().await.draft.as_ref().unwrap().origin().unwrap(), [220., 180.]);
    move_to(&shared, [240., 190.]).await;
    let revision = shared.lock().await.document().draft_revision;
    placement::change(&shared, PlacementChange::SetOrigin {}, revision).await.unwrap();
    near(shared.lock().await.draft.as_ref().unwrap().origin().unwrap(), [240., 190.]);
    let job = {
        let mut c = shared.lock().await;
        let job = c.save_job("Repeat loose sheet").unwrap();
        assert!(c.library.job(&job.id).unwrap().sheet_offset.is_none());
        c.change_placement(PlacementChange::NewRun {}).unwrap();
        assert!(c.draft.as_ref().unwrap().sheet_offset.is_none());
        job
    };
    {
        let mut c = shared.lock().await;
        c.open_job(&job.id).unwrap();
        assert!(c.draft.as_ref().unwrap().sheet_offset.is_none());
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn correction_waits_for_capture_recompiles_on_reposition_and_checks_travel() {
    let (simulator, shared) = start("capture-correction").await;
    {
        let mut c = shared.lock().await;
        let view = c.correction_view(LaserMode::Fiber).unwrap();
        c.save_correction(CorrectionChange::Measure {
            revision: view.revision,
            mode: LaserMode::Fiber,
            measurements: Box::new([Some(Measurement { x: 101., y: 99. }); 9]),
            apply: true,
        })
        .unwrap();
    }
    setup(&shared).await;
    machine::compile(&shared, false).await.unwrap();
    assert!(shared.lock().await.document().draft.unwrap().placement.correction_pending);
    home(&shared).await;
    move_to(&shared, [200., 150.]).await;
    let control = simulator.control();
    control.clear_writes();
    placement::prepare(&shared).await.unwrap();
    {
        let c = shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert!(!d.view().placement.correction_pending);
        let map = openlaser_correction::Map::new(d.correction.as_ref().unwrap()).unwrap();
        let first = d
            .compiled
            .as_ref()
            .unwrap()
            .view
            .moves
            .iter()
            .find(|m| m.kind == PathKind::Cut)
            .unwrap();
        let zero = Point::from(d.sheet_offset.unwrap());
        let physical = map.forward(Point::from(first.points[0]) + zero);
        let wanted = Point::from(d.preview.as_ref().unwrap().contours[0].start) + zero;
        assert!(physical.distance(wanted) < 0.02);
        assert!(control.writes().is_empty());
    }
    shared.lock().await.change_placement(PlacementChange::Reposition {}).unwrap();
    assert!(shared.lock().await.draft.as_ref().unwrap().compiled.is_none());
    move_to(&shared, [300., 250.]).await;
    placement::prepare(&shared).await.unwrap();
    near(shared.lock().await.draft.as_ref().unwrap().origin().unwrap(), [300., 250.]);
    shared.lock().await.change_placement(PlacementChange::Fixed { origin: [1370., 250.] }).unwrap();
    assert!(placement::prepare(&shared).await.unwrap_err().to_string().contains("travel"));
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn calibration_keeps_normal_kerf_leads_and_editing_with_correction_off() {
    let (_sim, shared) = start("normal-calibration").await;
    let recipe = setup(&shared).await;
    let features = Features {
        kerf: Some(Kerf { width: Millimeters(0.2), side: Side::Auto }),
        leads: Some(Leads {
            entry: Some(Lead {
                shape: LeadShape::Line,
                length: Millimeters(2.),
                radius: Millimeters(1.),
                angle: Degrees(45.),
            }),
            exit: None,
            side: Side::Auto,
            closed_only: true,
            overrides: Vec::new(),
        }),
        ..Features::default()
    };
    {
        let mut c = shared.lock().await;
        let view = c.correction_view(LaserMode::Fiber).unwrap();
        c.save_correction(CorrectionChange::Measure {
            revision: view.revision,
            mode: LaserMode::Fiber,
            measurements: Box::new([Some(Measurement { x: 101., y: 99. }); 9]),
            apply: true,
        })
        .unwrap();
        c.correction_coupon(LaserMode::Fiber).unwrap();
        c.set_recipe(&recipe).unwrap();
        c.set_features(features.clone()).unwrap();
        c.transform(&[0], Some(Transform::translation(Point::new(1., 1.)))).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    machine::compile(&shared, false).await.unwrap();
    let mut c = shared.lock().await;
    let d = c.draft.as_ref().unwrap();
    assert!(d.calibration && d.correction.is_none());
    assert_eq!(d.features, features);
    let compiled = &d.compiled.as_ref().unwrap().view;
    assert_eq!(compiled.plan.len(), 9);
    assert!(compiled.moves.iter().any(|m| m.kind == PathKind::LeadIn));
    let job = c.save_job("Normal calibration job").unwrap();
    let job = c.library.job(&job.id).unwrap();
    assert!(job.calibration && job.correction.is_none());
    assert_eq!(job.features, features);
    c.change_placement(PlacementChange::Head {}).unwrap();
    assert_eq!(c.document().draft.unwrap().placement.mode, PlacementMode::Head);
}
