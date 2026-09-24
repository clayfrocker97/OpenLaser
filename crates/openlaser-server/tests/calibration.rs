// SPDX-License-Identifier: GPL-3.0-or-later
//! Start needs the head calibrated for the job's material at the W table's
//! current height; framing does not. On the loopback simulator.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "known simulator fixtures")]
mod common;

use common::{start, until};
use openlaser_controller::lease::Lease;
use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_core::geometry::{Contour, Curve, Drawing};
use openlaser_server::connect;
use openlaser_server::coordinator::{NewRecipe, Shared, Values};
use openlaser_server::machine::{self, TableRequest};

/// The harness backup with a lifting W table configured.
fn table_backup() -> String {
    let xml = std::fs::read_to_string(common::fixture("xml/harness-backup.xml"))
        .unwrap()
        .replace("<PLaserParam>", "<PLaserParam><LPF LiftingPlatformType='1'/>")
        .replace(
            "<MP LaserDAKeepOutput",
            "<MP ExchangePlatformType='0' RollSheetType='0' LaserDAKeepOutput",
        )
        .replace("</ParameterRoot>", "<PHomeParam><HPA3 Acc='1000'/></PHomeParam></ParameterRoot>");
    let doc = openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, xml.as_bytes())
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "SoftLimitMaxLen", "4")
        .unwrap()
        .with_attribute("/ParameterRoot/PMachineAxisConfig_4/MAC_4", "GoOriginalDirection", "0")
        .unwrap();
    String::from_utf8(doc.original().to_vec()).unwrap()
}

/// A square part and two recipes of different thickness; the first is chosen.
async fn setup(shared: &Shared) -> [openlaser_library::Id; 2] {
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
    let part = c.library.add_part("square.dxf", b"calibration fixture", drawing).unwrap();
    let recipe = |thickness_mm: f64| NewRecipe {
        name: "Calibration steel".into(),
        laser: LaserMode::Fiber,
        thickness_mm,
        values: Values::Bank(1),
        gas: None,
    };
    let thin = c.add_recipe(&recipe(1.)).unwrap().id;
    let thick = c.add_recipe(&recipe(3.)).unwrap().id;
    c.open_part(&part.id).unwrap();
    c.set_recipe(&thin).unwrap();
    c.set_features(Features::default()).unwrap();
    [thin, thick]
}

async fn run_reason(shared: &Shared) -> Option<String> {
    shared.lock().await.document().readiness.run.reason
}

#[tokio::test]
async fn start_needs_z_calibrated_for_this_material_and_table_height() {
    let (simulator, shared) = start("calibration-gate").await;
    let control = simulator.control();
    machine::import_file(&shared, "table.xml", table_backup().as_bytes()).await.unwrap();
    common::seed(&shared, &simulator).await;
    let [thin, thick] = setup(&shared).await;
    connect::connect(&shared).await.unwrap();
    machine::home(&shared).await.unwrap();
    until(&shared, 10, |d| d.machine.session.homed && d.machine.operation.is_none()).await;
    machine::compile(&shared, false).await.unwrap();

    // Never calibrated: Start waits, framing does not.
    assert_eq!(run_reason(&shared).await.as_deref(), Some("calibrate Z first"));
    assert!(shared.lock().await.document().readiness.frame.ok);
    let refused =
        common::confirmation(&shared, openlaser_server::preflight::PreflightIntent::Run).await;
    let error = machine::run_reviewed(&shared, Some(&refused)).await.unwrap_err();
    assert!(error.to_string().contains("calibrate Z"), "{error}");

    common::calibrate(&shared).await;
    assert!(shared.lock().await.document().readiness.run.ok, "{:?}", run_reason(&shared).await);

    // Another material needs its own calibration, and so does coming back.
    shared.lock().await.set_recipe(&thick).unwrap();
    machine::compile(&shared, false).await.unwrap();
    assert_eq!(run_reason(&shared).await.as_deref(), Some("material changed; calibrate Z again"));
    shared.lock().await.set_recipe(&thin).unwrap();
    machine::compile(&shared, false).await.unwrap();
    assert!(!shared.lock().await.document().readiness.run.ok);
    common::calibrate(&shared).await;
    assert!(shared.lock().await.document().readiness.run.ok);

    // Moving the W table moves the sheet surface: calibrate again.
    let lease = Lease { client: "calibration".into(), sequence: 1 };
    machine::table(&shared, TableRequest { positive: true, speed_mm_s: 10. }, lease.clone())
        .await
        .unwrap();
    until(&shared, 5, |_| control.view().position_mm[4] > 0.5).await;
    machine::release(&shared, lease).await.unwrap();
    until(&shared, 10, |d| {
        d.machine.operation.is_none()
            && d.machine.feedback.as_ref().is_some_and(|f| f.table_stationary)
    })
    .await;
    assert_eq!(run_reason(&shared).await.as_deref(), Some("W table moved; calibrate Z again"));
    assert_eq!(shared.lock().await.document().calibration.stale.as_deref(), Some("W table moved"));
    common::calibrate(&shared).await;
    assert!(shared.lock().await.document().readiness.run.ok);
    openlaser_server::shutdown(&shared).await.unwrap();
}
