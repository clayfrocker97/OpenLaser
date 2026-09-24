// SPDX-License-Identifier: GPL-3.0-or-later

//! Offline preparation timing with exact geometry/grouping fingerprints.
//! Arguments: drawing.dxf features.json repetitions [snapshot.json] [retained-draft.json].

use openlaser_core::features::Features;
use openlaser_core::geometry::Placed;
use openlaser_library::sha256;
use openlaser_server::draft::{self, Draft};
use std::hint::black_box;
use std::time::Instant;

#[derive(serde::Serialize, serde::Deserialize)]
struct Retained {
    version: u32,
    draft: Draft,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let bytes = std::fs::read(args.get(1).ok_or("drawing.dxf is required")?)?;
    let drawing = openlaser_dxf::import(&bytes)?.drawing;
    let features: Features =
        serde_json::from_slice(&std::fs::read(args.get(2).ok_or("features.json is required")?)?)?;
    let repetitions: usize = args.get(3).map_or(Ok(3), |s| s.parse())?;
    for repetition in 0..repetitions {
        let started = Instant::now();
        let groups = black_box(openlaser_prep::groups(black_box(&drawing.contours)));
        let groups_ms = started.elapsed().as_secs_f64() * 1000.;

        let started = Instant::now();
        let toolpath = black_box(openlaser_prep::prepare(&drawing, &features)?);
        let toolpath_ms = started.elapsed().as_secs_f64() * 1000.;

        let started = Instant::now();
        let prepared = black_box(draft::prepare(
            &drawing,
            &Placed::all(drawing.contours.len()),
            &features,
            &[],
        )?);
        let prepared_ms = started.elapsed().as_secs_f64() * 1000.;

        let part = (openlaser_library::Id::from("benchmark"), std::sync::Arc::new(drawing.clone()));
        let sources = openlaser_library::JobDrawing::new(vec![part]);
        let mut draft = Draft::new(std::sync::Arc::new(sources));
        draft.current.features = features.clone();
        let started = Instant::now();
        draft.prepare(black_box(&drawing));
        let draft_ms = started.elapsed().as_secs_f64() * 1000.;
        if let Some(error) = &draft.error {
            return Err(error.clone().into());
        }
        let snapshot = serde_json::json!({
            "groups": groups, "toolpath": toolpath,
            "preview": prepared.preview, "instances": prepared.instances,
            "draft": draft.view(),
        });
        let serialized = serde_json::to_vec(&snapshot)?;
        let hash = sha256(&serialized);
        if repetition == 0
            && let Some(path) = args.get(4)
        {
            std::fs::write(path, &serialized)?;
        }
        println!(
            "{}",
            serde_json::json!({
                "repetition": repetition, "contours": drawing.contours.len(),
                "groups_ms": groups_ms, "toolpath_ms": toolpath_ms,
                "prepared_ms": prepared_ms, "draft_ms": draft_ms,
                "geometry_sha256": hash,
            })
        );
    }
    if let Some(path) = args.get(5) {
        let retained: Retained = serde_json::from_slice(&std::fs::read(path)?)?;
        for repetition in 0..3 {
            let started = Instant::now();
            let bytes = serde_json::to_vec(black_box(&retained))?;
            let encode_ms = started.elapsed().as_secs_f64() * 1000.;
            let started = Instant::now();
            let hash = black_box(sha256(&bytes));
            let hash_ms = started.elapsed().as_secs_f64() * 1000.;
            println!(
                "{}",
                serde_json::json!({
                    "stage": "retained-draft", "repetition": repetition,
                    "bytes": bytes.len(), "encode_ms": encode_ms, "hash_ms": hash_ms,
                    "sha256": hash,
                })
            );
        }
    }
    Ok(())
}
