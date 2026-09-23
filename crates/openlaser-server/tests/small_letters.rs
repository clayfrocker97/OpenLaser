// SPDX-License-Identifier: GPL-3.0-or-later
//! Fine lettering retains every counter and contour through import and CAM.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "offline font fixtures")]
mod common;

use openlaser_core::LaserMode;
use openlaser_core::features::{CutOrder, Features, Kerf, Side};
use openlaser_core::units::Millimeters;
use openlaser_server::coordinator::{NewRecipe, Values};
use openlaser_server::document::{PassKind, PathKind};
use openlaser_server::draft::Draft;
use openlaser_server::machine;

#[tokio::test]
async fn small_svg_and_dxf_letters_retain_counters_and_cut_them_before_their_outlines() {
    let (_sim, shared) = common::start("small-letters").await;
    let recipe = shared
        .lock()
        .await
        .add_recipe(&NewRecipe {
            name: "Letter coupon".into(),
            laser: LaserMode::Fiber,
            thickness_mm: 1.,
            values: Values::Bank(1),
            gas: None,
        })
        .unwrap();
    let tiny_dxf =
        "0\nSECTION\n2\nENTITIES\n0\nTEXT\n10\n5\n20\n10\n40\n3\n1\nB08i.\n0\nENDSEC\n0\nEOF\n";
    for (name, source) in [
        (
            "small-letter-coupon.svg",
            include_bytes!("../../../fixtures/svg/small-letter-coupon.svg").as_slice(),
        ),
        ("small-letters.dxf", tiny_dxf.as_bytes()),
    ] {
        let groups = {
            let mut c = shared.lock().await;
            let part = c.import_part(name, source).unwrap();
            let drawing = &c.library.part(&part.id).unwrap().drawing;
            assert!(drawing.contours.iter().all(openlaser_core::geometry::Contour::is_closed));
            let groups = openlaser_prep::groups(&drawing.contours);
            assert!(groups.iter().any(|g| g.len() == 3), "B and 8 retain both counters");
            c.open_part(&part.id).unwrap();
            c.set_recipe(&recipe.id).unwrap();
            groups
        };
        for width in [0., 0.1] {
            let features = Features {
                kerf: (width > 0.).then_some(Kerf { width: Millimeters(width), side: Side::Auto }),
                order: CutOrder { inner_first: true, ..CutOrder::default() },
                ..Features::default()
            };
            shared.lock().await.set_features(features).unwrap();
            machine::prepare(&shared).await.unwrap();
            machine::compile(&shared, false)
                .await
                .unwrap_or_else(|error| panic!("{name}, kerf {width}: {error}"));
            let c = shared.lock().await;
            let draft = c.draft.as_ref().unwrap();
            let count = draft.drawing().unwrap().contours.len();
            let preview = draft.preview.as_ref().unwrap();
            assert_eq!(preview.contours.len(), count, "{name}, kerf {width}");
            assert!(preview.contours.iter().all(|p| p.closed));
            let compiled = &draft.compiled.as_ref().unwrap().view;
            assert_eq!(compiled.plan.iter().filter(|p| p.kind == PassKind::Cut).count(), count);
            assert!(compiled.moves.iter().flat_map(|m| &m.points).flatten().all(|p| p.is_finite()));
            assert_shape_fidelity(draft, name, width);
            for group in &groups {
                let members: Vec<_> = preview
                    .contours
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.sources.iter().any(|s| group.contains(s)))
                    .collect();
                for (i, hole) in members.iter().filter(|(_, p)| p.depth % 2 == 1) {
                    for (j, outer) in members.iter().filter(|(_, p)| p.depth < hole.depth) {
                        assert!(
                            i < j,
                            "{name}: counter {:?} follows outline {:?}",
                            hole.sources,
                            outer.sources
                        );
                    }
                }
            }
        }
    }
    openlaser_server::shutdown(&shared).await.unwrap();
}

fn assert_shape_fidelity(draft: &Draft, name: &str, width: f64) {
    let compiled = draft.compiled.as_ref().unwrap();
    for contour in &draft.preview.as_ref().unwrap().contours {
        let expected: Vec<_> = contour.paths.iter().flat_map(|p| &p.points).copied().collect();
        let pass = compiled
            .view
            .plan
            .iter()
            .position(|p| {
                p.kind == PassKind::Cut
                    && p.instances.iter().any(|i| contour.sources.contains(&i.source))
            })
            .unwrap();
        let actual: Vec<_> = compiled
            .view
            .moves
            .iter()
            .filter(|m| m.pass == Some(pass) && m.kind == PathKind::Cut)
            .flat_map(|m| &m.points)
            .map(|p| [p[0] - compiled.shift[0], p[1] - compiled.shift[1]])
            .collect();
        assert!(actual.len() > 2);
        let error = deviation(&expected, &actual).max(deviation(&actual, &expected));
        assert!(
            error < 0.05,
            "{name}, kerf {width}, sources {:?}: {error} mm shape error",
            contour.sources
        );
    }
}

fn deviation(points: &[[f64; 2]], path: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .map(|point| {
            path.windows(2)
                .map(|line| {
                    let delta = [line[1][0] - line[0][0], line[1][1] - line[0][1]];
                    let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
                    let fraction = if length_squared > 0. {
                        (((point[0] - line[0][0]) * delta[0] + (point[1] - line[0][1]) * delta[1])
                            / length_squared)
                            .clamp(0., 1.)
                    } else {
                        0.
                    };
                    (point[0] - line[0][0] - fraction * delta[0])
                        .hypot(point[1] - line[0][1] - fraction * delta[1])
                })
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0., f64::max)
}
