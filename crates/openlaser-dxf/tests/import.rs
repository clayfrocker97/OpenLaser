// SPDX-License-Identifier: GPL-3.0-or-later

//! Every fixture drawing under `fixtures/dxf`, imported and rendered as
//! text: the units, the entity count, what was skipped, and each contour
//! with its layer, closure, length and exact curves. A change to the
//! import rules shows up as a snapshot diff.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests read known drawings")]

use openlaser_core::geometry::Curve;
use std::fmt::Write as _;

fn render(import: &openlaser_dxf::Import) -> String {
    let mut text = format!("units: {:?}\nentities: {}\n", import.units, import.entities);
    for skipped in &import.skipped {
        writeln!(text, "skipped: {} at line {}", skipped.entity, skipped.line).unwrap();
    }
    for warning in &import.warnings {
        writeln!(text, "notice: {warning}").unwrap();
    }
    for (index, contour) in import.drawing.contours.iter().enumerate() {
        writeln!(
            text,
            "contour {index}: layer {:?}, {}, length {:.4}",
            contour.layer,
            if contour.is_closed() { "closed" } else { "open" },
            contour.length()
        )
        .unwrap();
        for curve in &contour.curves {
            match *curve {
                Curve::Line { start, end } => {
                    writeln!(
                        text,
                        "  line ({:.4}, {:.4}) -> ({:.4}, {:.4})",
                        start.x, start.y, end.x, end.y
                    )
                    .unwrap();
                }
                Curve::Arc { center, radius, start_angle, sweep } => writeln!(
                    text,
                    "  arc centre ({:.4}, {:.4}) radius {:.4} from {:.4} rad sweep {:.4} rad",
                    center.x, center.y, radius, start_angle, sweep
                )
                .unwrap(),
            }
        }
    }
    text
}

/// Each fixture drawing imports to the same contours as before.
#[test]
fn fixture_drawings_import_as_snapshotted() {
    let dir = format!("{}/../../fixtures/dxf", env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    assert!(!paths.is_empty(), "no fixtures in {dir}");
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let import = openlaser_dxf::import(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        insta::assert_snapshot!(name, render(&import));
    }
}
