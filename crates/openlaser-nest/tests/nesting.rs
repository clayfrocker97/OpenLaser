// SPDX-License-Identifier: GPL-3.0-or-later
//! Physical quantity, exact-curve preservation, grain and container acceptance.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known geometry fixtures")]

use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_core::nesting::{NestRotation, NestSettings, NestStock, Nesting};
use openlaser_nest::{Item, Request, SheetLimit, nest};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn shape(points: &[[f64; 2]]) -> Contour {
    Contour {
        layer: "Cut".into(),
        curves: (0..points.len())
            .map(|i| Curve::Line {
                start: points[i].into(),
                end: points[(i + 1) % points.len()].into(),
            })
            .collect(),
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> Contour {
    shape(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]])
}

#[test]
fn remnant_overflow_uses_fresh_stock_and_preserves_every_copy() {
    let mut input = request();
    input.stock = rect(0., 0., 100., 60.);
    input.cutouts = vec![rect(0., 0., 75., 60.)];
    input.rectangular = false;
    input.items = vec![Item { contours: vec![rect(0., 0., 15., 15.)], quantity: 16 }];
    input.settings = NestSettings {
        remnant_clearance: 0.,
        spacing: 2.,
        margin: 1.,
        rotation: NestRotation::Fixed,
    };
    input.time_limit = Duration::from_secs(5);
    let sheets = openlaser_nest::nest_sheets(
        &input,
        Some(&input.stock),
        &AtomicBool::new(false),
        |_, _| {},
        |_, _| {},
    )
    .unwrap();
    assert!(sheets.len() > 1);
    assert!(sheets[0].original);
    assert!(sheets[0].layout.placements.len() < 4);
    assert_eq!(sheets.iter().map(|s| s.layout.placements.len()).sum::<usize>(), 16);
    for sheet in &sheets {
        for placement in &sheet.layout.placements {
            let moved = placement.transform.contour(&input.items[0].contours[0]);
            let bounds = moved.bounds().unwrap();
            assert!(
                bounds.min.x >= 1.
                    && bounds.max.x <= 99.
                    && bounds.min.y >= 1.
                    && bounds.max.y <= 59.
            );
            if sheet.original {
                assert!(bounds.min.x > 76.);
            }
        }
    }
    assert!(
        sheets.iter().skip(1).flat_map(|s| &s.layout.placements).any(|p| p
            .transform
            .contour(&input.items[0].contours[0])
            .bounds()
            .unwrap()
            .min
            .x
            < 70.)
    );
}

#[test]
fn cutout_checks_reject_crossing_covering_and_nearby_paths() {
    let stock = rect(0., 0., 100., 100.);
    let cutouts = vec![rect(40., 40., 20., 20.)];
    assert!(
        openlaser_nest::check_region(&stock, &cutouts, &[rect(30., 30., 40., 40.)], 1.).is_err()
    );
    let cross = Contour {
        layer: "Lead".into(),
        curves: vec![Curve::Line { start: Point::new(20., 50.), end: Point::new(80., 50.) }],
    };
    assert!(openlaser_nest::check_region(&stock, &cutouts, &[cross], 1.).is_err());
    assert!(
        openlaser_nest::check_region(&stock, &cutouts, &[rect(30., 45., 9.5, 10.)], 1.).is_err()
    );
    openlaser_nest::check_region(&stock, &cutouts, &[rect(5., 5., 25., 25.)], 1.).unwrap();
}

#[test]
fn manual_remnant_alignment_adds_clearance_to_cutouts_and_edges_only_on_remnants() {
    let outline = rect(0., 0., 100., 100.);
    let cutouts = vec![rect(40., 40., 20., 20.)];
    let mut nesting = Nesting {
        stock: NestStock::Remnant {
            reference: "fixture".into(),
            name: "Remainder".into(),
            outline: outline.clone(),
            cutouts: cutouts.clone(),
            clearance: 4.,
        },
        settings: NestSettings { margin: 3., remnant_clearance: 10., ..NestSettings::default() },
    };
    assert!((nesting.margin() - 14.).abs() < f64::EPSILON);
    for nearby in [rect(8., 15., 8., 8.), rect(25., 44., 6., 6.)] {
        openlaser_nest::check_region(&outline, &cutouts, std::slice::from_ref(&nearby), 4.)
            .unwrap();
        assert!(
            openlaser_nest::check_region(&outline, &cutouts, &[nearby], nesting.margin()).is_err()
        );
    }
    openlaser_nest::check_region(&outline, &cutouts, &[rect(16., 16., 8., 8.)], nesting.margin())
        .unwrap();
    nesting.stock = NestStock::Rectangle { bounds: outline.bounds().unwrap() };
    assert!(
        (nesting.margin() - 3.).abs() < f64::EPSILON,
        "fresh stock has no remnant alignment allowance"
    );
}
fn request() -> Request {
    Request {
        cutouts: Vec::new(),
        stock: rect(100., 50., 180., 100.),
        rectangular: true,
        items: vec![Item {
            contours: vec![
                rect(10., 20., 35., 20.),
                Contour {
                    layer: "Hole".into(),
                    curves: vec![Curve::Arc {
                        center: Point::new(20., 30.),
                        radius: 3.,
                        start_angle: 0.,
                        sweep: std::f64::consts::TAU,
                    }],
                },
            ],
            quantity: 6,
        }],
        settings: NestSettings {
            remnant_clearance: 0.,
            spacing: 4.,
            margin: 2.,
            rotation: NestRotation::HalfTurn,
        },
        machining_clearance: 0.,
        sheet_limit: SheetLimit { contours: 5000, curves: 100_000 },
        time_limit: Duration::from_millis(1500),
        seed: 11,
    }
}

/// A sheet is full at the preparation limit even with room to spare, so the
/// copies spread over more sheets; one sheet alone refuses them outright.
#[test]
fn copies_beyond_the_preparation_limit_start_a_new_sheet() {
    let mut input = request();
    input.items = vec![Item { contours: vec![rect(0., 0., 10., 10.)], quantity: 5 }];
    input.sheet_limit = SheetLimit { contours: 2, curves: 100_000 };
    let sheets =
        openlaser_nest::nest_sheets(&input, None, &AtomicBool::new(false), |_, _| {}, |_, _| {})
            .unwrap();
    let counts: Vec<_> = sheets.iter().map(|s| s.layout.placements.len()).collect();
    assert_eq!(counts, [2, 2, 1]);
    assert!(nest(&input, &AtomicBool::new(false), |_, _| {}).is_err());
    input.sheet_limit = SheetLimit { contours: 5000, curves: 7 };
    let sheets =
        openlaser_nest::nest_sheets(&input, None, &AtomicBool::new(false), |_, _| {}, |_, _| {})
            .unwrap();
    assert!(sheets.iter().all(|s| s.layout.placements.len() == 1), "4 lines per copy, 7 per sheet");
    input.sheet_limit = SheetLimit { contours: 5000, curves: 3 };
    assert!(
        openlaser_nest::nest_sheets(&input, None, &AtomicBool::new(false), |_, _| {}, |_, _| {})
            .is_err()
    );
}

/// Compaction starts from the first fit's own width, so identical
/// rectangles end at least as tight as the first fit packs them: two
/// columns at most on a sheet taller than five of them.
#[test]
fn compaction_never_loosens_the_first_fit() {
    let mut input = request();
    input.stock = rect(0., 0., 600., 400.);
    input.items = vec![Item { contours: vec![rect(0., 0., 120., 70.)], quantity: 8 }];
    input.settings = NestSettings {
        spacing: 3.,
        margin: 3.,
        remnant_clearance: 0.,
        rotation: NestRotation::Any,
    };
    input.time_limit = Duration::from_secs(2);
    let sheets =
        openlaser_nest::nest_sheets(&input, None, &AtomicBool::new(false), |_, _| {}, |_, _| {})
            .unwrap();
    assert_eq!(sheets.len(), 1);
    let right = sheets[0]
        .layout
        .placements
        .iter()
        .map(|p| p.transform.contour(&input.items[0].contours[0]).bounds().unwrap().max.x)
        .fold(f64::MIN, f64::max);
    assert!(right <= 3. + 120. + 3. + 120. + 1., "the copies reach x = {right}");
}

/// A sheet whose parts stand in one column cannot compact past its widest
/// part; the search keeps the first fit instead of failing.
#[test]
fn a_single_column_sheet_keeps_its_first_fit() {
    let mut input = request();
    input.stock = rect(0., 0., 120., 80.);
    input.items = vec![
        Item { contours: vec![rect(0., 0., 40., 20.)], quantity: 1 },
        Item { contours: vec![rect(0., 0., 30., 30.)], quantity: 1 },
    ];
    input.settings = NestSettings {
        spacing: 3.,
        margin: 3.,
        remnant_clearance: 0.,
        rotation: NestRotation::Fixed,
    };
    input.time_limit = Duration::from_secs(2);
    let sheets =
        openlaser_nest::nest_sheets(&input, None, &AtomicBool::new(false), |_, _| {}, |_, _| {})
            .unwrap();
    assert_eq!(sheets.iter().map(|s| s.layout.placements.len()).sum::<usize>(), 2);
}

/// A running search shows its sheets as they fill and compact, every copy
/// inside the stock and clear of the others, never more copies than asked.
#[test]
fn a_running_search_shows_each_sheet_as_it_stands() {
    let mut input = request();
    input.items = vec![Item { contours: vec![rect(0., 0., 20., 12.)], quantity: 30 }];
    input.time_limit = Duration::from_secs(2);
    let shown = std::sync::Mutex::new(Vec::new());
    let sheets = openlaser_nest::nest_sheets(
        &input,
        None,
        &AtomicBool::new(false),
        |_, _| {},
        |s, changed| {
            assert!(changed < s.len());
            shown.lock().unwrap().push((std::time::Instant::now(), s.to_vec()));
        },
    )
    .unwrap();
    let shown = shown.into_inner().unwrap();
    assert!(shown.len() > 1, "shown {} times", shown.len());
    for pair in shown.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= openlaser_nest::LIVE_INTERVAL);
    }
    let stock = input.stock.bounds().unwrap();
    for (_, sheets) in &shown {
        assert!(sheets.iter().map(|s| s.layout.placements.len()).sum::<usize>() <= 30);
        for sheet in sheets {
            let copies: Vec<_> = sheet
                .layout
                .placements
                .iter()
                .map(|p| p.transform.contour(&input.items[0].contours[0]).bounds().unwrap())
                .collect();
            for (i, a) in copies.iter().enumerate() {
                assert!(a.min.x >= stock.min.x - 1e-6 && a.max.x <= stock.max.x + 1e-6);
                assert!(a.min.y >= stock.min.y - 1e-6 && a.max.y <= stock.max.y + 1e-6);
                for b in &copies[i + 1..] {
                    let apart = a.max.x <= b.min.x + 1e-6
                        || b.max.x <= a.min.x + 1e-6
                        || a.max.y <= b.min.y + 1e-6
                        || b.max.y <= a.min.y + 1e-6;
                    assert!(apart, "shown copies overlap: {a:?} {b:?}");
                }
            }
        }
    }
    assert_eq!(sheets.iter().map(|s| s.layout.placements.len()).sum::<usize>(), 30);
}

#[test]
fn complete_rectangle_preserves_holes_arcs_and_grain() {
    let input = request();
    let original = input.items[0].contours.clone();
    let result = nest(&input, &AtomicBool::new(false), |_, _| {}).unwrap();
    assert_eq!(result.placements.len(), 6);
    assert!(result.coverage > 0. && result.coverage < 1.);
    for p in &result.placements {
        let [_, b, c, _, _, _] = p.transform.0;
        assert!(b.abs() < 1e-6 && c.abs() < 1e-6);
        assert!(matches!(p.transform.contour(&original[1]).curves[0], Curve::Arc { .. }));
        let moved: Vec<_> = original.iter().map(|c| p.transform.contour(c)).collect();
        openlaser_nest::check_inside(&input.stock, &moved, 2.).unwrap();
    }
    // Independent f64 distances for the rectangular outside edges, not the
    // optimizer's own collision result. Parallel grain makes them axis-aligned.
    for (i, a) in result.placements.iter().enumerate() {
        let a = a.transform.contour(&original[0]).bounds().unwrap();
        for b in &result.placements[i + 1..] {
            let b = b.transform.contour(&original[0]).bounds().unwrap();
            let dx = (a.min.x - b.max.x).max(b.min.x - a.max.x).max(0.);
            let dy = (a.min.y - b.max.y).max(b.min.y - a.max.y).max(0.);
            assert!(dx.hypot(dy) >= input.settings.spacing - 1e-6);
        }
    }
    assert_eq!(input.items[0].contours, original);
}

#[test]
fn concave_stock_is_used_as_an_outline() {
    let mut input = request();
    input.stock = shape(&[[0., 0.], [100., 0.], [100., 35.], [35., 35.], [35., 100.], [0., 100.]]);
    input.rectangular = false;
    input.items = vec![Item { contours: vec![rect(0., 0., 20., 20.)], quantity: 4 }];
    let result = nest(&input, &AtomicBool::new(false), |_, _| {}).unwrap();
    assert_eq!(result.placements.len(), 4);
    for p in result.placements {
        let bounds = p.transform.contour(&input.items[0].contours[0]).bounds().unwrap();
        assert!(bounds.max.x <= 35. || bounds.max.y <= 35.);
    }
}

#[test]
fn a_finished_layout_can_be_nested_again_with_individual_parts() {
    let mut input = request();
    input.items[0].quantity = 25;
    input.stock = rect(-100., -100., 1371., 971.);
    input.time_limit = Duration::from_secs(2);
    let first = nest(&input, &AtomicBool::new(false), |_, _| {}).unwrap();
    input.items = first
        .placements
        .iter()
        .map(|p| Item {
            contours: input.items[p.item].contours.iter().map(|c| p.transform.contour(c)).collect(),
            quantity: 1,
        })
        .collect();
    let before = input.items.iter().map(|i| i.contours.clone()).collect::<Vec<_>>();
    input.time_limit = Duration::from_millis(500);
    let second = nest(&input, &AtomicBool::new(false), |_, _| {}).unwrap();
    assert_eq!(second.placements.len(), 25);
    for p in &second.placements {
        assert!(p.transform.0[1].abs() < 1e-15);
        assert!(p.transform.0[2].abs() < 1e-15);
    }
    assert_eq!(input.items.iter().map(|i| i.contours.clone()).collect::<Vec<_>>(), before);
}

#[test]
fn impossible_and_cancelled_searches_return_no_partial_solution() {
    let mut input = request();
    input.stock = rect(0., 0., 5., 5.);
    input.time_limit = Duration::from_millis(20);
    assert!(
        nest(&input, &AtomicBool::new(false), |_, _| {})
            .unwrap_err()
            .0
            .contains("no complete layout")
    );
    assert!(
        nest(&request(), &AtomicBool::new(true), |_, _| {}).unwrap_err().0.contains("cancelled")
    );
}

#[test]
fn quantities_cannot_overflow_and_containment_requires_a_valid_margin() {
    let mut input = request();
    input.items[0].quantity = usize::MAX;
    input.items.push(Item { contours: input.items[0].contours.clone(), quantity: 1 });
    assert!(
        nest(&input, &AtomicBool::new(false), |_, _| {})
            .unwrap_err()
            .to_string()
            .contains("1 and 500")
    );
    let stock = rect(0., 0., 100., 100.);
    let path = rect(10., 10., 10., 10.);
    let compiled = [[10., 10.], [20., 10.]];
    for margin in [f64::NAN, f64::INFINITY, -1.] {
        assert!(openlaser_nest::check_inside(&stock, std::slice::from_ref(&path), margin).is_err());
        assert!(openlaser_nest::check_polylines(&stock, [compiled.as_slice()], margin).is_err());
    }
}

#[test]
fn concave_crossing_is_rejected_even_when_segment_ends_are_inside() {
    let stock = shape(&[[0., 0.], [100., 0.], [100., 30.], [30., 30.], [30., 100.], [0., 100.]]);
    let diagonal = Contour {
        layer: String::new(),
        curves: vec![Curve::Line { start: Point::new(15., 80.), end: Point::new(80., 15.) }],
    };
    assert!(openlaser_nest::check_inside(&stock, &[diagonal], 0.).is_err());
}
