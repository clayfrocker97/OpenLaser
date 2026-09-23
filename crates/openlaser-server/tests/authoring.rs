// SPDX-License-Identifier: GPL-3.0-or-later
//! Saved data, retained working copies and conflicts across a server restart.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known fixtures")]
mod common;

use common::{fixture, start, until};
use openlaser_core::LaserMode;
use openlaser_server::coordinator::{ItemChange, NewRecipe, RecipeChange, Values};
use openlaser_server::{Coordinator, machine};
use std::collections::BTreeMap;

#[tokio::test]
async fn opening_a_part_starts_fresh_while_jobs_and_pending_setups_keep_their_sheets() {
    use openlaser_core::geometry::{Placed, Transform};
    use openlaser_server::nesting::StockChoice;
    use openlaser_server::workspace::{flush, key};

    let (_simulator, shared) = start("part-sheet-separation").await;
    let (config, part, job, first, second, first_nesting, second_nesting) = {
        let mut c = shared.lock().await;
        let part = c
            .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
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
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.set_stock(StockChoice::Rectangle { width: 150., height: 100. }).unwrap();
        let job = c.save_job("Saved sheet").unwrap();
        let saved_nesting = c.draft.as_ref().unwrap().nesting.clone();

        c.open_part(&part.id).unwrap();
        assert!(c.draft.as_ref().unwrap().recipe.is_none());
        c.set_stock(StockChoice::Rectangle { width: 200., height: 120. }).unwrap();
        c.transform(&[0], Some(Transform([1., 0., 0., 1., 5., 7.]))).unwrap();
        let first = key(c.draft.as_ref().unwrap());
        let first_nesting = c.draft.as_ref().unwrap().nesting.clone();

        c.open_job(&job.id).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().nesting, saved_nesting);
        c.open_part(&part.id).unwrap();
        let fresh = c.draft.as_ref().unwrap();
        assert!(fresh.recipe.is_none() && fresh.nesting.is_none() && fresh.sheets.is_none());
        assert_eq!(fresh.placed, Placed::all(1));
        assert_ne!(key(fresh), first);
        assert!(c.undo().is_err(), "a fresh part cannot undo back into an old sheet layout");
        assert_eq!(
            c.pending_drafts().unwrap().iter().map(|d| &d.key).collect::<Vec<_>>(),
            [&first]
        );

        c.set_stock(StockChoice::Rectangle { width: 250., height: 180. }).unwrap();
        let second = key(c.draft.as_ref().unwrap());
        let second_nesting = c.draft.as_ref().unwrap().nesting.clone();
        c.open_retained(&first).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().nesting, first_nesting);
        assert_eq!(c.document().draft.as_ref().unwrap().key, first);
        assert_eq!(c.pending_drafts().unwrap().len(), 2);
        (c.config.clone(), part.id, job.id, first, second, first_nesting, second_nesting)
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let restored = Coordinator::start(config.clone()).unwrap();
    {
        let mut c = restored.lock().await;
        assert_eq!(key(c.draft.as_ref().unwrap()), first);
        assert_eq!(c.draft.as_ref().unwrap().nesting, first_nesting);
        c.open_part(&part).unwrap();
        assert!(c.draft.as_ref().unwrap().nesting.is_none());
        c.open_retained(&second).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().nesting, second_nesting);
        c.open_job(&job).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().nesting, c.library.job(&job).unwrap().nesting);
        for _ in 0..4 {
            c.open_part(&part).unwrap();
        }
        assert_eq!(c.pending_drafts().unwrap().len(), 2);
    }
    flush(&restored).await.unwrap();
    assert_eq!(
        std::fs::read_dir(config.data_dir.join("drafts")).unwrap().count(),
        4,
        "only the saved job, two edited setups and the current empty setup are retained"
    );
    openlaser_server::shutdown(&restored).await.unwrap();
}

#[tokio::test]
async fn legacy_part_drafts_remain_recoverable_without_attaching_to_reopened_parts() {
    use openlaser_server::nesting::StockChoice;
    let (_simulator, shared) = start("legacy-part-sheet").await;
    let (config, part, old_key, legacy, nesting) = {
        let mut c = shared.lock().await;
        let part = c
            .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
            .unwrap();
        c.open_part(&part.id).unwrap();
        c.set_stock(StockChoice::Rectangle { width: 200., height: 120. }).unwrap();
        let draft = c.draft.as_ref().unwrap();
        let old_key = openlaser_server::workspace::key(draft);
        let mut legacy = serde_json::to_value(draft).unwrap();
        legacy.as_object_mut().unwrap().remove("workspace_id");
        (c.config.clone(), part.id, old_key, legacy, draft.nesting.clone())
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let key = format!("part-{part}");
    std::fs::remove_file(config.data_dir.join("drafts").join(format!("{old_key}.json"))).unwrap();
    std::fs::write(
        config.data_dir.join("drafts").join(format!("{key}.json")),
        serde_json::to_vec(&serde_json::json!({"version": 1, "draft": legacy})).unwrap(),
    )
    .unwrap();
    std::fs::write(config.data_dir.join("active.json"), serde_json::to_vec(&key).unwrap()).unwrap();
    let restored = Coordinator::start(config).unwrap();
    {
        let mut c = restored.lock().await;
        assert_eq!(c.draft.as_ref().unwrap().nesting, nesting);
        c.open_part(&part).unwrap();
        assert!(c.draft.as_ref().unwrap().nesting.is_none());
        assert_eq!(c.pending_drafts().unwrap()[0].key, key);
        c.open_retained(&key).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().nesting, nesting);
    }
    openlaser_server::shutdown(&restored).await.unwrap();
}

#[tokio::test]
async fn a_failed_discard_keeps_the_working_copy_and_retained_file() {
    let (_simulator, shared) = start("discard-write-failure").await;
    let mut c = shared.lock().await;
    let bytes = std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap();
    let part = c.import_part("line.dxf", &bytes).unwrap();
    c.open_part(&part.id).unwrap();
    c.transform(&[0], Some(openlaser_core::geometry::Transform([1., 0., 0., 1., 5., 7.]))).unwrap();
    let draft = c.draft.as_ref().unwrap();
    let key = openlaser_server::workspace::key(draft);
    let placed = draft.placed.clone();
    let active = c.config.data_dir.join("active.json");
    let retained = c.config.data_dir.join("drafts").join(format!("{key}.json"));
    drop(c);
    openlaser_server::workspace::flush(&shared).await.unwrap();
    let saved = std::fs::read(&retained).unwrap();
    std::fs::remove_file(&active).unwrap();
    std::fs::create_dir(&active).unwrap();
    assert!(openlaser_server::workspace::discard(&shared, &key).await.is_err());
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().placed, placed);
    assert_eq!(std::fs::read(retained).unwrap(), saved);
    std::fs::remove_dir(&active).unwrap();
    openlaser_server::workspace::discard(&shared, &key).await.unwrap();
    assert_ne!(shared.lock().await.draft.as_ref().unwrap().placed, placed);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn working_copies_history_and_saved_conflicts_survive_a_restart() {
    let (_simulator, shared) = start("authoring").await;
    let (config, job, other, revision, zero, undo_zero) = {
        let mut c = shared.lock().await;
        let bytes = std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap();
        let part = c.import_part("line.dxf", &bytes).unwrap();
        assert_eq!(c.import_part("again.dxf", &bytes).unwrap().id, part.id);
        c.update_part(
            &part.id,
            ItemChange {
                tags: Some(vec!["steel".into()]),
                notes: Some("Deburr".into()),
                quantity: Some(3),
                ..Default::default()
            },
        )
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
        c.open_part(&part.id).unwrap();
        c.set_recipe(&recipe.id).unwrap();
        c.transform(&[0], Some(openlaser_core::geometry::Transform([2., 0., 0., 2., 0., 0.])))
            .unwrap();
        let job = c.save_job("Line job").unwrap();
        let bounds = job.bounds.unwrap();
        assert!((bounds.max.x - bounds.min.x - 6.).abs() < 1e-8);
        assert!((bounds.max.y - bounds.min.y - 6.).abs() < 1e-8);
        let copy = c.duplicate_job(&job.id).unwrap();
        assert_eq!(copy.quantity, 3);
        assert_eq!(copy.tags, ["steel"]);
        drop(c);
        machine::prepare(&shared).await.unwrap();
        let mut c = shared.lock().await;
        c.set_origin([40., 50.]).unwrap();
        c.update_job(
            &job.id,
            ItemChange { notes: Some("Saved elsewhere".into()), ..Default::default() },
        )
        .unwrap();
        assert!(c.merge_review(None).unwrap().conflicts.is_empty());
        c.save_job("Line job").unwrap();
        assert_eq!(c.library.job(&job.id).unwrap().notes, "Saved elsewhere");
        drop(c);
        machine::prepare(&shared).await.unwrap();
        let mut c = shared.lock().await;
        c.set_origin([80., 90.]).unwrap();
        c.library.update_job(&job.id, |j| j.sheet_offset = Some([20., 30.])).unwrap();
        let review = c.merge_review(None).unwrap();
        assert_eq!(review.conflicts.len(), 1);
        assert_eq!(review.conflicts[0].path, "/zero");
        c.resolve_merge(&review.token, None, &BTreeMap::from([("/zero".into(), true)])).unwrap();
        drop(c);
        machine::prepare(&shared).await.unwrap();
        let mut c = shared.lock().await;
        let undo_zero = c.draft.as_ref().unwrap().sheet_offset;
        c.set_origin([110., 120.]).unwrap();
        let zero = c.draft.as_ref().unwrap().sheet_offset;
        let other = c.duplicate_part(&part.id).unwrap();
        c.open_part(&other.id).unwrap();
        drop(c);
        machine::prepare(&shared).await.unwrap();
        let mut c = shared.lock().await;
        c.set_origin([10., 20.]).unwrap();
        assert_eq!(c.pending_drafts().unwrap().len(), 2);
        (c.config.clone(), job.id, other.id, c.document().draft_revision, zero, undo_zero)
    };
    openlaser_server::shutdown(&shared).await.unwrap();
    let restored = Coordinator::start(config).unwrap();
    until(&restored, 5, |d| d.draft.as_ref().is_some_and(|d| d.preview.is_some())).await;
    {
        let mut c = restored.lock().await;
        let document = c.document();
        assert_eq!(document.draft.as_ref().unwrap().parts[0].id, other);
        assert!(document.draft_revision > revision);
        assert!(document.draft.unwrap().compiled.is_none());
        assert!(!document.machine.session.homed);
        assert!(c.held.is_none() && c.recovery.is_none());
        c.open_job(&job).unwrap();
        assert_eq!(c.draft.as_ref().unwrap().sheet_offset, zero);
        c.undo().unwrap();
        assert_eq!(c.draft.as_ref().unwrap().sheet_offset, undo_zero);
        assert!(!c.library.edit_history().unwrap().past.is_empty());
    }
    openlaser_server::shutdown(&restored).await.unwrap();
}

#[tokio::test]
async fn stale_material_and_machine_imports_are_refused_without_overwriting() {
    let (_simulator, shared) = start("saved-conflicts").await;
    {
        let mut c = shared.lock().await;
        let recipe = c
            .add_recipe(&NewRecipe {
                name: "Steel".into(),
                laser: LaserMode::Fiber,
                thickness_mm: 1.,
                values: Values::Bank(1),
                gas: None,
            })
            .unwrap();
        c.update_recipe(
            &recipe.id,
            RecipeChange {
                attributes: BTreeMap::from([("CutPower".into(), "30".into())]),
                ..Default::default()
            },
        )
        .unwrap();
        let change = RecipeChange {
            attributes: BTreeMap::from([("CutPower".into(), "40".into())]),
            expected_attributes: Some(BTreeMap::from([(
                "CutPower".into(),
                recipe.attributes.get("CutPower").cloned(),
            )])),
            ..Default::default()
        };
        assert!(c.update_recipe(&recipe.id, change).is_err());
        assert_eq!(c.library.recipe(&recipe.id).unwrap().attributes["CutPower"], "30");
    }
    let backup = std::fs::read(fixture("xml/harness-backup.xml")).unwrap();
    assert!(
        machine::import_file_reviewed(&shared, "other.xml", &backup, Some("stale")).await.is_err()
    );
    machine::import_soft_reviewed(&shared, "one.ini", b"[Soft]\nFollowOvertime=8000", Some(""))
        .await
        .unwrap();
    assert!(
        machine::import_soft_reviewed(&shared, "two.ini", b"[Soft]\nFollowOvertime=9000", Some(""))
            .await
            .is_err()
    );
    assert_eq!(shared.lock().await.document().soft.follow_ms, 8000);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[test]
fn queued_drafts_remain_readable_and_discard_cannot_resurrect_an_old_save() {
    use openlaser_core::geometry::Transform;
    use std::time::Duration;
    // Occupy the blocking pool after startup, so no draft fsync can finish.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (_simulator, shared) = start("queued-drafts").await;
        let (part, other, config) = {
            let mut c = shared.lock().await;
            let part = c
                .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
                .unwrap();
            let other = c.duplicate_part(&part.id).unwrap();
            (part.id, other.id, c.config.clone())
        };
        let (unblock, block) = std::sync::mpsc::channel();
        let (entered, started) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            let _ = block.recv();
        });
        started.await.unwrap();
        let (key, edited) = {
            let mut c = shared.lock().await;
            c.open_part(&part).unwrap();
            for step in 1..=20 {
                c.transform(&[0], Some(Transform([1., 0., 0., 1., f64::from(step), 7.]))).unwrap();
            }
            let edited = c.draft.as_ref().unwrap().placed.clone();
            let key = openlaser_server::workspace::key(c.draft.as_ref().unwrap());
            c.open_part(&other).unwrap();
            c.transform(&[0], Some(Transform([1., 0., 0., 1., 3., 4.]))).unwrap();
            c.open_retained(&key).unwrap();
            assert_eq!(c.draft.as_ref().unwrap().placed, edited);
            assert_eq!(c.pending_drafts().unwrap().len(), 2);
            (key, edited)
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!config.data_dir.join("drafts").join(format!("{key}.json")).exists());
        let flushing = {
            let shared = shared.clone();
            tokio::spawn(async move { openlaser_server::workspace::flush(&shared).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        flushing.abort(); // An abandoned waiter must not release the disk writer early.
        let discarding = {
            let shared = shared.clone();
            tokio::spawn(async move { openlaser_server::workspace::discard(&shared, &key).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!discarding.is_finished());
        let c = tokio::time::timeout(Duration::from_millis(100), shared.lock()).await.unwrap();
        assert_eq!(c.draft.as_ref().unwrap().placed, edited);
        drop(c);
        unblock.send(()).unwrap();
        blocker.await.unwrap();
        discarding.await.unwrap().unwrap();
        let fresh = shared.lock().await.draft.as_ref().unwrap().placed.clone();
        assert_ne!(fresh, edited);
        openlaser_server::shutdown(&shared).await.unwrap();
        drop(shared);
        let restored = Coordinator::start(config).unwrap();
        {
            let mut c = restored.lock().await;
            assert_eq!(c.draft.as_ref().unwrap().placed, fresh);
            assert!(
                c.undo().is_err(),
                "discarded history must not be recreated by an older queued save"
            );
            assert_eq!(c.pending_drafts().unwrap().len(), 1);
        }
        openlaser_server::shutdown(&restored).await.unwrap();
    });
}

#[tokio::test]
async fn an_authoring_write_failure_is_visible_and_the_latest_edit_can_be_retried() {
    let (_simulator, shared) = start("authoring-write-error").await;
    let (config, retained) = {
        let mut c = shared.lock().await;
        let part = c
            .import_part("line.dxf", &std::fs::read(fixture("dxf/minimal-line.dxf")).unwrap())
            .unwrap();
        c.open_part(&part.id).unwrap();
        let key = openlaser_server::workspace::key(c.draft.as_ref().unwrap());
        (c.config.clone(), c.config.data_dir.join("drafts").join(format!("{key}.json")))
    };
    openlaser_server::workspace::flush(&shared).await.unwrap();
    std::fs::remove_file(&retained).unwrap();
    std::fs::create_dir(&retained).unwrap();
    let edited = {
        let mut c = shared.lock().await;
        c.transform(&[0], Some(openlaser_core::geometry::Transform([1., 0., 0., 1., 9., 11.])))
            .unwrap();
        c.draft.as_ref().unwrap().placed.clone()
    };
    let revision = shared.lock().await.document().draft_revision;
    assert!(
        openlaser_server::placement::change(
            &shared,
            openlaser_server::placement::PlacementChange::Head {},
            revision,
        )
        .await
        .is_err()
    );
    until(&shared, 5, |d| d.persistence_error.is_some()).await;
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().placed, edited);
    assert!(matches!(
        shared.lock().await.draft.as_ref().unwrap().placement,
        Some(openlaser_library::placement::Placement::Head {})
    ));
    std::fs::remove_dir(&retained).unwrap();
    let revision = shared.lock().await.document().draft_revision;
    openlaser_server::placement::change(
        &shared,
        openlaser_server::placement::PlacementChange::Head {},
        revision,
    )
    .await
    .unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&retained).unwrap()).unwrap();
    assert_eq!(saved["draft"]["placement"]["kind"], "head");
    until(&shared, 5, |d| d.persistence_error.is_none()).await;
    openlaser_server::shutdown(&shared).await.unwrap();
    let restored = Coordinator::start(config).unwrap();
    assert_eq!(restored.lock().await.draft.as_ref().unwrap().placed, edited);
    openlaser_server::shutdown(&restored).await.unwrap();
}
