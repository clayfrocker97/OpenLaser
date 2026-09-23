// SPDX-License-Identifier: GPL-3.0-or-later
//! Jobs cutting several library parts: layout, pruning, history, saving,
//! retained drafts, merging and nesting.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use common::start;
use openlaser_core::LaserMode;
use openlaser_core::features::{Features, OrderStrategy};
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Placed, Point, Transform};
use openlaser_core::nesting::{NestRotation, NestSettings};
use openlaser_library::Id;
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::nesting::{self, NestRequest, StockChoice};
use openlaser_server::workspace::{flush, key};
use openlaser_server::{Coordinator, machine};
use std::collections::BTreeMap;

fn rect(x: f64, y: f64, width: f64, height: f64) -> Contour {
    let points = [[x, y], [x + width, y], [x + width, y + height], [x, y + height]];
    Contour {
        layer: "Cut".into(),
        curves: (0..4)
            .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
            .collect(),
    }
}

/// A plate with a hole, a tab drawn far from the origin, and a disc.
fn parts(c: &mut Coordinator) -> [Id; 3] {
    let mut add = |name: &str, contours: Vec<Contour>| {
        c.library.add_part(name, name.as_bytes(), Drawing { contours }).unwrap().id
    };
    let disc =
        Contour { layer: "Cut".into(), curves: vec![Curve::circle(Point::new(0., 0.), 15.)] };
    [
        add("plate.dxf", vec![rect(0., 0., 100., 60.), rect(20., 20., 10., 10.)]),
        add("tab.svg", vec![rect(-500., 300., 40., 20.)]),
        add("disc.dxf", vec![disc]),
    ]
}

fn recipe(c: &mut Coordinator) -> Id {
    c.add_recipe(&NewRecipe {
        name: "Steel".into(),
        laser: LaserMode::Fiber,
        thickness_mm: 1.,
        values: Values::Bank(1),
        gas: None,
    })
    .unwrap()
    .id
}

/// Where each of the draft's parts lies on the sheet.
fn part_bounds(c: &Coordinator) -> Vec<Bounds> {
    let draft = c.draft.as_ref().unwrap();
    let view = draft.view();
    view.parts
        .iter()
        .map(|part| {
            let range = part.first..part.first + part.contours;
            let mine: Vec<Placed> =
                draft.placed.iter().filter(|p| range.contains(&p.source)).copied().collect();
            openlaser_server::draft::place(draft.drawing().unwrap(), &mine).bounds().unwrap()
        })
        .collect()
}

fn ids(c: &Coordinator) -> Vec<Id> {
    c.draft.as_ref().unwrap().parts.clone()
}

#[tokio::test]
async fn several_parts_open_side_by_side_and_save_as_one_job() {
    let (_simulator, shared) = start("parts-side-by-side").await;
    let mut c = shared.lock().await;
    let [a, b, disc] = parts(&mut c);
    c.open_parts(&[a.clone(), b.clone(), disc.clone()]).unwrap();
    let view = c.document().draft.unwrap();
    assert_eq!(view.name, "plate + tab + disc");
    let ranges: Vec<_> = view.parts.iter().map(|p| (p.id.clone(), p.first, p.contours)).collect();
    assert_eq!(ranges, [(a.clone(), 0, 2), (b.clone(), 2, 1), (disc.clone(), 3, 1)]);
    let draft = c.draft.as_ref().unwrap();
    assert_eq!(draft.placed.iter().map(|p| p.copy).collect::<Vec<_>>(), [0, 0, 1, 2]);
    assert_eq!(draft.placed[0].transform, Transform::IDENTITY, "the first part stays put");
    let [plate, tab, round] = part_bounds(&c)[..] else { panic!("three parts") };
    assert_eq!((tab.min.x, tab.min.y), (plate.max.x + 10., plate.min.y));
    assert_eq!((round.min.x, round.min.y), (tab.max.x + 10., plate.min.y));
    assert!(c.draft.as_ref().unwrap().correction.is_none(), "no correction measured yet");

    let recipe = recipe(&mut c);
    c.set_recipe(&recipe).unwrap();
    drop(c);
    machine::prepare(&shared).await.unwrap();
    let mut c = shared.lock().await;
    assert!(c.draft.as_ref().unwrap().error.is_none(), "{:?}", c.draft.as_ref().unwrap().error);
    assert_eq!(c.draft.as_ref().unwrap().groups.len(), 3, "each part is its own group");
    let placed = c.draft.as_ref().unwrap().placed.clone();
    let saved = c.save_job("Mixed plate").unwrap();
    assert_eq!(saved.parts, [a.clone(), b.clone(), disc.clone()]);
    let file: serde_json::Value = serde_json::from_slice(
        &std::fs::read(c.config.data_dir.join("jobs").join(format!("{}.json", saved.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(file["version"], 2);
    for part in [&a, &b, &disc] {
        let refused = c.remove_part(part).unwrap_err().to_string();
        assert!(refused.contains("Mixed plate"), "{refused}");
    }
    c.open_part(&a).unwrap();
    assert_eq!(ids(&c), std::slice::from_ref(&a));
    c.open_job(&saved.id).unwrap();
    assert_eq!(ids(&c), [a, b, disc]);
    assert_eq!(c.draft.as_ref().unwrap().placed, placed);
    assert_eq!(c.document().draft.unwrap().name, "Mixed plate");
}

#[tokio::test]
async fn a_part_with_nothing_left_on_the_sheet_leaves_the_job_until_undone() {
    let (_simulator, shared) = start("parts-pruning").await;
    let mut c = shared.lock().await;
    let [a, b, disc] = parts(&mut c);
    c.open_parts(&[a.clone(), b.clone(), disc.clone()]).unwrap();
    let before = c.draft.as_ref().unwrap().placed.clone();
    c.remove(&[2]).unwrap();
    assert_eq!(ids(&c), [a.clone(), disc.clone()]);
    let draft = c.draft.as_ref().unwrap();
    assert_eq!(draft.placed.iter().map(|p| p.source).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(draft.drawing().unwrap().contours.len(), 3);
    assert_eq!(draft.placed[2].transform, before[3].transform, "the disc stays where it was");
    assert_eq!(c.document().draft.unwrap().name, "plate + disc");
    c.undo().unwrap();
    assert_eq!(ids(&c), [a.clone(), b, disc.clone()]);
    assert_eq!(c.draft.as_ref().unwrap().placed, before);
    assert_eq!(c.draft.as_ref().unwrap().drawing().unwrap().contours.len(), 4);
    c.redo().unwrap();
    assert_eq!(ids(&c), [a.clone(), disc]);
    // Removing a hole keeps its part: the plate is still on the sheet.
    c.remove(&[1]).unwrap();
    assert_eq!(ids(&c).len(), 2);
    assert!(c.remove(&[0, 1]).is_err(), "keep at least one contour");
    drop(c);
    machine::prepare(&shared).await.unwrap();
    assert!(shared.lock().await.draft.as_ref().unwrap().preview.is_some());
}

#[tokio::test]
async fn parts_added_to_an_open_job_go_beside_it_in_one_undo_step() {
    let (_simulator, shared) = start("parts-added").await;
    let mut c = shared.lock().await;
    let [a, b, disc] = parts(&mut c);
    c.open_part(&a).unwrap();
    let mut features = c.draft.as_ref().unwrap().features.clone();
    features.order.strategy = OrderStrategy::Manual(vec![1, 0]);
    c.set_features(features).unwrap();
    c.add_parts(&[b.clone(), disc.clone()]).unwrap();
    assert_eq!(ids(&c), [a.clone(), b.clone(), disc.clone()]);
    let draft = c.draft.as_ref().unwrap();
    assert_eq!(
        draft.placed.iter().map(|p| (p.source, p.copy)).collect::<Vec<_>>(),
        [(0, 0), (1, 0), (2, 1), (3, 2)]
    );
    assert_eq!(draft.features.order.strategy, OrderStrategy::Manual(vec![1, 0, 2, 3]));
    let [plate, tab, _] = part_bounds(&c)[..] else { panic!("three parts") };
    assert_eq!((tab.min.x, tab.min.y), (plate.max.x + 10., plate.min.y));
    let refused = c.add_parts(std::slice::from_ref(&b)).unwrap_err().to_string();
    assert!(refused.contains("tab is already in this job"), "{refused}");
    assert!(c.add_parts(&[]).is_err());
    c.undo().unwrap();
    assert_eq!(ids(&c), [a]);
    let draft = c.draft.as_ref().unwrap();
    assert_eq!(draft.placed.len(), 2);
    assert_eq!(draft.features.order.strategy, OrderStrategy::Manual(vec![1, 0]));
    c.redo().unwrap();
    assert_eq!(ids(&c).len(), 3);
    drop(c);
    machine::prepare(&shared).await.unwrap();
    let c = shared.lock().await;
    assert!(c.draft.as_ref().unwrap().error.is_none(), "{:?}", c.draft.as_ref().unwrap().error);
}

#[tokio::test]
async fn several_parts_survive_a_restart_and_older_history_still_undoes() {
    let (_simulator, shared) = start("parts-retained").await;
    let (config, a, b, first, placed) = {
        let mut c = shared.lock().await;
        let [a, b, _] = parts(&mut c);
        c.open_parts(&[a.clone(), b.clone()]).unwrap();
        c.transform(&[2], Some(Transform::translation(Point::new(0., 25.)))).unwrap();
        let draft = c.draft.as_ref().unwrap();
        (c.config.clone(), a, b, key(draft), draft.placed.clone())
    };
    flush(&shared).await.unwrap();
    let path = config.data_dir.join("drafts").join(format!("{first}.json"));
    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["version"], 2);
    assert_eq!(stored["draft"]["part"], serde_json::json!([a, b]));
    openlaser_server::shutdown(&shared).await.unwrap();

    let restored = Coordinator::start(config.clone()).unwrap();
    {
        let mut c = restored.lock().await;
        assert_eq!(ids(&c), [a.clone(), b.clone()]);
        assert_eq!(c.draft.as_ref().unwrap().placed, placed);
        c.undo().unwrap();
        assert_ne!(c.draft.as_ref().unwrap().placed, placed);
        // A single part with history written before jobs could cut several.
        c.open_part(&a).unwrap();
        c.transform(&[0], Some(Transform::translation(Point::new(5., 0.)))).unwrap();
    }
    flush(&restored).await.unwrap();
    let second = key(restored.lock().await.draft.as_ref().unwrap());
    openlaser_server::shutdown(&restored).await.unwrap();
    let path = config.data_dir.join("drafts").join(format!("{second}.json"));
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["version"], 1);
    assert_eq!(stored["draft"]["part"], serde_json::json!(a));
    for step in stored["draft"]["past"].as_array_mut().unwrap() {
        step.as_object_mut().unwrap().remove("part");
    }
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let legacy = Coordinator::start(config).unwrap();
    {
        let mut c = legacy.lock().await;
        assert_eq!(ids(&c), std::slice::from_ref(&a));
        c.undo().unwrap();
        assert_eq!(ids(&c), [a]);
        assert_eq!(c.draft.as_ref().unwrap().placed, Placed::all(2));
    }
    openlaser_server::shutdown(&legacy).await.unwrap();
}

#[tokio::test]
async fn parts_changed_on_two_screens_merge_with_their_layout_as_one_choice() {
    let (_simulator, shared) = start("parts-merge").await;
    let mut c = shared.lock().await;
    let [a, b, disc] = parts(&mut c);
    let recipe = recipe(&mut c);
    c.open_parts(&[a.clone(), b.clone()]).unwrap();
    c.set_recipe(&recipe).unwrap();
    let job = c.save_job("Pair").unwrap();
    // This screen adds the disc; another saves a moved plate meanwhile.
    c.add_parts(&[disc]).unwrap();
    let moved = Transform::translation(Point::new(0., 40.));
    let elsewhere = c.library.update_job(&job.id, |job| job.placed[0].transform = moved).unwrap();
    let review = c.merge_review(None).unwrap();
    let conflict: Vec<_> = review.conflicts.iter().filter(|c| c.path == "/part").collect();
    assert_eq!(conflict.len(), 1, "{:?}", review.conflicts);
    assert_eq!(conflict[0].draft, "plate + tab + disc");
    assert_eq!(conflict[0].saved, "plate + tab");
    assert!(
        review.conflicts.iter().all(|c| c.path != "/placed"),
        "the layout comes with the parts"
    );
    assert!(c.save_job("Pair").is_err(), "a conflict is never saved silently");
    let choices = BTreeMap::from([("/part".to_owned(), false)]);
    c.resolve_merge(&review.token, None, &choices).unwrap();
    assert_eq!(ids(&c), [a, b]);
    assert_eq!(c.draft.as_ref().unwrap().placed, elsewhere.placed);
    assert!(c.draft.as_ref().unwrap().drawing().is_ok());
}

#[tokio::test]
async fn deleting_a_part_closes_the_unsaved_job_that_cuts_it() {
    let (_simulator, shared) = start("parts-deleted").await;
    let mut c = shared.lock().await;
    let [a, b, _] = parts(&mut c);
    c.open_parts(&[a, b.clone()]).unwrap();
    c.remove_part(&b).unwrap();
    assert!(c.draft.is_none());
}

async fn nest(shared: &Shared, contours: Vec<usize>, quantity: u32) -> nesting::NestView {
    let revision = shared.lock().await.document().draft_revision;
    let request = NestRequest {
        contours,
        quantity,
        settings: NestSettings {
            remnant_clearance: 0.,
            spacing: 3.,
            margin: 3.,
            rotation: NestRotation::Fixed,
        },
        seconds: 1,
    };
    let task = nesting::start(shared, revision, request).await.unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let view = nesting::status(shared, task.id).await.unwrap();
        if !view.running {
            assert!(view.error.is_none(), "{:?}", view.error);
            return view;
        }
        assert!(tokio::time::Instant::now() < deadline, "nesting worker did not finish");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Every part of a job nests; a set of sheets saves each sheet as a job of
/// only the parts on it.
#[tokio::test]
async fn nested_sheets_save_as_jobs_of_their_own_parts() {
    let (_simulator, shared) = start("parts-sheets").await;
    {
        let mut c = shared.lock().await;
        let [a, b, disc] = parts(&mut c);
        let recipe = recipe(&mut c);
        c.open_parts(&[a, b, disc]).unwrap();
        c.set_recipe(&recipe).unwrap();
        c.set_features(Features { leads: None, kerf: None, ..Features::default() }).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    // The plate fills one 120 × 80 sheet; the tab and the disc need another.
    shared.lock().await.set_stock(StockChoice::Rectangle { width: 120., height: 80. }).unwrap();
    machine::prepare(&shared).await.unwrap();
    let view = nest(&shared, vec![0], 1).await;
    assert_eq!(view.sheets.len(), 2);
    assert_eq!(view.sheets.iter().map(|s| s.parts).collect::<Vec<_>>(), [1, 2]);
    nesting::apply(&shared, view.id).await.unwrap();
    machine::prepare(&shared).await.unwrap();
    let mut c = shared.lock().await;
    c.save_job("Mixed batch").unwrap();
    let mut jobs: Vec<_> = c.library.jobs().cloned().collect();
    jobs.sort_by_key(|j| j.sheet.as_ref().unwrap().number);
    let names = |job: &openlaser_library::Job| -> Vec<String> {
        job.parts.iter().map(|id| c.library.part(id).unwrap().name.clone()).collect()
    };
    assert_eq!(names(&jobs[0]), ["plate"]);
    assert_eq!(names(&jobs[1]), ["tab", "disc"]);
    let mut sources: Vec<_> = jobs[1].placed.iter().map(|p| p.source).collect();
    sources.sort_unstable();
    assert_eq!(sources, [0, 1], "the tab and the disc renumbered from the start");
}
