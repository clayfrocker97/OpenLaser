// SPDX-License-Identifier: GPL-3.0-or-later
//! Nesting through stock selection, preview, Apply, history and persistence.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known offline fixtures")]
mod common;

use common::start;
use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_core::geometry::{Contour, Curve, Drawing, Point, Transform};
use openlaser_core::nesting::{NestRotation, NestSettings};
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::layers::LayerChange;
use openlaser_server::nesting::{self, NestRequest, NestView, StockChoice};
use openlaser_server::{Coordinator, machine};

fn rect(x: f64, y: f64, width: f64, height: f64) -> Contour {
    let points = [[x, y], [x + width, y], [x + width, y + height], [x, y + height]];
    Contour {
        layer: "Cut".into(),
        curves: (0..4)
            .map(|i| Curve::Line { start: points[i].into(), end: points[(i + 1) % 4].into() })
            .collect(),
    }
}
async fn setup(shared: &Shared) {
    let mut c = shared.lock().await;
    let drawing = Drawing {
        contours: vec![
            rect(0., 0., 240., 160.),
            rect(20., 20., 30., 20.),
            Contour {
                layer: "Hole".into(),
                curves: vec![Curve::Arc {
                    center: Point::new(30., 30.),
                    radius: 3.,
                    start_angle: 0.,
                    sweep: std::f64::consts::TAU,
                }],
            },
            rect(100., 50., 20., 20.),
        ],
    };
    // Synthetic geometry enters the same library path as a parsed DXF.
    let part = c.library.add_part("nesting.dxf", b"nesting fixture geometry", drawing).unwrap();
    let recipe = c
        .add_recipe(&NewRecipe {
            name: "Nest steel".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 1.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    c.open_part(&part.id).unwrap();
    c.set_recipe(&recipe.id).unwrap();
    c.set_features(Features::default()).unwrap();
    // Two layers: each is cut with the job's recipe.
    for layer in ["Cut", "Hole"] {
        let change = LayerChange::Recipe { layer: layer.into(), recipe: None };
        c.change_layers(change).unwrap();
    }
    drop(c);
    machine::prepare(shared).await.unwrap();
    shared.lock().await.set_stock(StockChoice::Outline { contour: 0 }).unwrap();
    machine::prepare(shared).await.unwrap();
}
fn request() -> NestRequest {
    NestRequest {
        contours: vec![0],
        quantity: 4,
        settings: NestSettings {
            remnant_clearance: 0.,
            spacing: 4.,
            margin: 2.,
            rotation: NestRotation::HalfTurn,
        },
        seconds: 1,
        stock: vec![],
    }
}

#[tokio::test]
async fn numbered_sheets_switch_persist_and_save_as_one_history_edit() {
    let (_sim, shared) = start("numbered-sheets").await;
    setup(&shared).await;
    let revision = shared.lock().await.document().draft_revision;
    let mut request = request();
    request.quantity = 65;
    request.seconds = 5;
    let task = nesting::start(&shared, revision, request).await.unwrap();
    let result = finished(&shared, task.id).await;
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.sheets.len() > 1);
    assert_eq!(result.sheets.iter().map(|p| p.parts).sum::<usize>(), 66);
    let other = nesting::sheet_preview(&shared, task.id, 1).await.unwrap();
    assert!(other.preview.is_some());
    nesting::apply(&shared, task.id).await.unwrap();
    let count = result.sheets.len();
    let first = shared.lock().await.draft.as_ref().unwrap().current.placed.clone();
    shared.lock().await.select_sheet(1).unwrap();
    machine::prepare(&shared).await.unwrap();
    let second = shared.lock().await.draft.as_ref().unwrap().current.placed.clone();
    assert_ne!(first, second);
    shared.lock().await.select_sheet(0).unwrap();
    machine::prepare(&shared).await.unwrap();
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().current.placed, first);
    shared.lock().await.select_sheet(1).unwrap();
    machine::prepare(&shared).await.unwrap();
    let mut c = shared.lock().await;
    let saved = c.save_job("Bracket batch").unwrap();
    assert_eq!(saved.sheet.as_ref().unwrap().number, 2);
    assert_eq!(c.library.jobs().count(), count);
    assert!(
        c.library
            .jobs()
            .all(|j| j.folder == saved.folder && j.sheet.as_ref().unwrap().total as usize == count)
    );
    assert_eq!(c.document().draft.unwrap().sheets.as_ref().unwrap().active, 1);
    let root = c.config.data_dir.clone();
    let history = c.library.edit_history().unwrap();
    c.library.undo_saved(true, history.revision).unwrap();
    assert_eq!(c.library.jobs().count(), 0);
    let revision = c.library.edit_history().unwrap().revision;
    c.library.undo_saved(false, revision).unwrap();
    let reopened = openlaser_library::Library::open(root).unwrap();
    assert_eq!(reopened.jobs().count(), count);
    assert_eq!(reopened.job(&saved.id).unwrap().placed, second);
}
async fn finished(shared: &Shared, id: u64) -> NestView {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let view = nesting::status(shared, id).await.unwrap();
        if !view.running {
            return view;
        }
        assert!(tokio::time::Instant::now() < deadline, "nesting worker did not finish");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn stock_quantity_one_undo_save_and_restart() {
    let (_sim, shared) = start("nest-history").await;
    setup(&shared).await;
    let (before, past, revision) = {
        let c = shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert_eq!(d.groups.len(), 2, "removed stock must not keep all enclosed parts grouped");
        assert_eq!(d.current.placed.len(), 3);
        assert!(d.current.placed.iter().all(|p| p.source != 0));
        (d.current.placed.clone(), d.view().past, c.document().draft_revision)
    };
    let work = nesting::start(&shared, revision, request()).await.unwrap();
    let view = finished(&shared, work.id).await;
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.total, 5);
    assert_eq!(
        shared.lock().await.draft.as_ref().unwrap().current.placed,
        before,
        "preview must not mutate the draft"
    );
    nesting::apply(&shared, work.id).await.unwrap();
    let (placed, stock, config) = {
        let c = shared.lock().await;
        let d = c.draft.as_ref().unwrap();
        assert_eq!(d.current.placed.len(), 9);
        assert_eq!(d.groups.len(), 5);
        assert_eq!(d.view().past, past + 1);
        assert_eq!(d.current.placed.iter().filter(|p| p.source == 2).count(), 4);
        assert!(d.current.placed.iter().all(|p| p.source != 0));
        assert!(d.prepared.is_some());
        (d.current.placed.clone(), d.current.nesting.clone(), c.config.clone())
    };
    shared.lock().await.undo().unwrap();
    machine::prepare(&shared).await.unwrap();
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().current.placed, before);
    shared.lock().await.redo().unwrap();
    machine::prepare(&shared).await.unwrap();
    let job = shared.lock().await.save_job("Nested sheet").unwrap();
    machine::compile(&shared, true).await.unwrap();
    assert_eq!(
        shared.lock().await.draft.as_ref().unwrap().compiled.as_ref().unwrap().view.plan.len(),
        9
    );
    openlaser_server::shutdown(&shared).await.unwrap();
    let reopened = Coordinator::start(config).unwrap();
    common::until(&reopened, 5, |d| {
        d.draft.as_ref().is_some_and(|draft| draft.preview.is_some() && draft.error.is_none())
    })
    .await;
    let c = reopened.lock().await;
    let d = c.draft.as_ref().unwrap();
    assert_eq!(d.current.placed, placed);
    assert_eq!(d.current.nesting, stock);
    assert_eq!(c.library.job(&job.id).unwrap().nesting, stock);
    drop(c);
    openlaser_server::shutdown(&reopened).await.unwrap();
}

#[tokio::test]
async fn edited_and_cancelled_results_cannot_apply() {
    let (_sim, shared) = start("nest-stale").await;
    setup(&shared).await;
    let revision = shared.lock().await.document().draft_revision;
    let work = nesting::start(&shared, revision, request()).await.unwrap();
    let view = finished(&shared, work.id).await;
    assert!(view.error.is_none(), "{:?}", view.error);
    shared.lock().await.transform(&[0], Some(Transform::translation(Point::new(1., 0.)))).unwrap();
    assert!(nesting::apply(&shared, work.id).await.is_err());
    machine::prepare(&shared).await.unwrap();
    let revision = shared.lock().await.document().draft_revision;
    let before = shared.lock().await.draft.as_ref().unwrap().current.placed.clone();
    let work = nesting::start(&shared, revision, request()).await.unwrap();
    nesting::cancel(&shared, work.id).await.unwrap();
    let view = finished(&shared, work.id).await;
    assert!(view.error.is_some());
    assert!(nesting::apply(&shared, work.id).await.is_err());
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().current.placed, before);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn rectangle_keeps_the_fixture_position_after_nesting() {
    let (_sim, shared) = start("nest-rectangle-origin").await;
    setup(&shared).await;
    {
        let mut c = shared.lock().await;
        c.set_origin([146., 673.]).unwrap();
        c.set_stock(StockChoice::Rectangle { width: 240., height: 160. }).unwrap();
    }
    machine::prepare(&shared).await.unwrap();
    let revision = shared.lock().await.document().draft_revision;
    let work = nesting::start(&shared, revision, request()).await.unwrap();
    let view = finished(&shared, work.id).await;
    assert!(view.error.is_none(), "{:?}", view.error);
    nesting::apply(&shared, work.id).await.unwrap();
    let c = shared.lock().await;
    let d = c.draft.as_ref().unwrap();
    let drawing = d.drawing().unwrap();
    let bounds =
        nesting::stock(drawing, d.current.nesting.as_ref().unwrap()).unwrap().bounds().unwrap();
    assert!((bounds.min.x + d.sheet_offset().unwrap()[0] - 146.).abs() < 1e-9);
    assert!((bounds.min.y + d.sheet_offset().unwrap()[1] - 673.).abs() < 1e-9);
    drop(c);
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn repeat_without_selection_and_retry_after_no_fit_preserve_counts() {
    let (_sim, shared) = start("nest-repeat").await;
    setup(&shared).await;
    let revision = shared.lock().await.document().draft_revision;
    let first = nesting::start(&shared, revision, request()).await.unwrap();
    assert!(finished(&shared, first.id).await.error.is_none());
    nesting::apply(&shared, first.id).await.unwrap();
    let again = NestRequest { contours: vec![], quantity: 1, ..request() };
    let revision = shared.lock().await.document().draft_revision;
    let second = nesting::start(&shared, revision, again.clone()).await.unwrap();
    let view = finished(&shared, second.id).await;
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.total, 5, "no selection means all current parts at their existing counts");
    nesting::apply(&shared, second.id).await.unwrap();
    let before = shared.lock().await.draft.as_ref().unwrap().current.placed.clone();
    shared.lock().await.set_stock(StockChoice::Rectangle { width: 15., height: 15. }).unwrap();
    machine::prepare(&shared).await.unwrap();
    let revision = shared.lock().await.document().draft_revision;
    let failed = nesting::start(&shared, revision, again.clone()).await.unwrap();
    assert!(finished(&shared, failed.id).await.error.unwrap().contains("add a larger sheet"));
    assert!(nesting::apply(&shared, failed.id).await.is_err());
    assert_eq!(shared.lock().await.draft.as_ref().unwrap().current.placed, before);
    shared.lock().await.set_stock(StockChoice::Rectangle { width: 240., height: 160. }).unwrap();
    machine::prepare(&shared).await.unwrap();
    let revision = shared.lock().await.document().draft_revision;
    let retry = nesting::start(&shared, revision, again).await.unwrap();
    let view = finished(&shared, retry.id).await;
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.total, 5);
    nesting::apply(&shared, retry.id).await.unwrap();
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn chosen_sheets_fill_in_order_within_what_is_on_hand() {
    use openlaser_server::inventory::NewStock;
    use openlaser_server::nesting::StockSource;
    let (_sim, shared) = start("nest-stock").await;
    setup(&shared).await;
    let rack = |material: &str, quantity| NewStock {
        material: material.into(),
        thickness_mm: 1.,
        laser: LaserMode::Fiber,
        width_mm: 100.,
        height_mm: 60.,
        quantity,
        folder: None,
    };
    let (revision, steel, other) = {
        let mut c = shared.lock().await;
        let steel = c.add_stock(rack("nest STEEL", 2)).unwrap();
        let turned = NewStock { width_mm: 60., height_mm: 100., ..rack("Nest steel", 1) };
        assert_eq!(c.add_stock(turned).unwrap(), steel, "the same size either way round adds up");
        let other = c.add_stock(rack("Brass", 5)).unwrap();
        (c.document().draft_revision, steel, other)
    };
    let asking = |stock| NestRequest { quantity: 12, stock, ..request() };
    let refused = [
        vec![StockSource::Stock { id: steel.clone(), count: 4 }],
        vec![
            StockSource::Stock { id: steel.clone(), count: 2 },
            StockSource::Stock { id: steel.clone(), count: 2 },
        ],
        vec![StockSource::Stock { id: other, count: 1 }],
        vec![StockSource::Sheet { width: 80., height: 50., count: Some(0) }],
    ];
    for stock in refused {
        assert!(nesting::start(&shared, revision, asking(stock)).await.is_err());
    }
    let stock = vec![
        StockSource::Stock { id: steel.clone(), count: 1 },
        StockSource::Sheet { width: 80., height: 50., count: None },
    ];
    let task = nesting::start(&shared, revision, asking(stock)).await.unwrap();
    let result = finished(&shared, task.id).await;
    assert!(result.error.is_none(), "{:?}", result.error);
    let sources: Vec<_> = result.sheets.iter().map(|s| s.source).collect();
    assert_eq!(sources[0], 0, "the rack sheet fills first");
    assert!(sources.len() > 1 && sources[1..].iter().all(|&s| s == 1), "{sources:?}");
    let [width, height] = result.sheets[0].size;
    assert!((width - 100.).abs() < 1e-9 && (height - 60.).abs() < 1e-9);
    assert_eq!(result.sheets.iter().map(|s| s.parts).sum::<usize>(), 13);
    nesting::apply(&shared, task.id).await.unwrap();
    machine::prepare(&shared).await.unwrap();
    let mut c = shared.lock().await;
    let job = c.save_job("Rack batch").unwrap();
    c.report_cut_sheet(&job.id).unwrap();
    let left = c.inventory.iter().find(|i| i.id == steel).unwrap().quantity;
    assert_eq!(left, 3, "a cut marked afterwards left the rack before it was counted");
    assert_eq!(c.document().library.stock.len(), 2);
}
