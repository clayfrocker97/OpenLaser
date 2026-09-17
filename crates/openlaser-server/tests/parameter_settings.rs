// SPDX-License-Identifier: GPL-3.0-or-later

//! Complete XML retention and verified settings application on loopback.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "fixture acceptance tests")]

mod common;
use openlaser_server::{connect, machine, parameter_settings as settings};

#[tokio::test]
async fn every_attribute_is_editable_and_save_applies_then_verifies() {
    let (simulator, shared) = common::start("xml-settings").await;
    let xml = std::fs::read_to_string(common::fixture("xml/harness-backup.xml")).unwrap()
        .replace("</ParameterRoot>", "<!-- retained exactly --><PCustom><KV Name='A &amp; B' Unused='0.0000001'/></PCustom></ParameterRoot>");
    machine::import_file(&shared, "complete-settings.xml", xml.as_bytes()).await.unwrap();
    let before = settings::view(&shared).await.unwrap();
    let parsed =
        openlaser_xml::Document::parse(openlaser_xml::Kind::Backup, xml.as_bytes()).unwrap();
    let count: usize = parsed.paths().iter().map(|p| parsed.attributes(p).unwrap().len()).sum();
    assert_eq!(before.fields.len(), count);
    assert!(!before.connected);
    connect::connect(&shared).await.unwrap();
    let matched = settings::view(&shared).await.unwrap();
    assert!(matched.connected && matched.problem.is_none(), "{:?}", matched.problem);
    assert!(matched.comparisons.iter().all(|c| c.actual == Some(c.expected)));
    assert!(!matched.comparisons.iter().any(|c| c.fields.iter().any(|f| f.contains("PCustom"))));

    settings::save(
        &shared,
        settings::Change {
            expected: before.sha256.clone(),
            edits: vec![
                settings::Edit {
                    path: "/ParameterRoot/PMachineAxisConfig_0/MAC".into(),
                    name: "ReturnLength".into(),
                    value: "12.345".into(),
                },
                settings::Edit {
                    path: "/ParameterRoot/PCustom/KV".into(),
                    name: "Name".into(),
                    value: "Exact <xml> & value".into(),
                },
            ],
        },
    )
    .await
    .unwrap();
    let saved = settings::view(&shared).await.unwrap();
    assert_ne!(saved.sha256, before.sha256);
    assert_eq!(saved.fields.len(), before.fields.len());
    assert!(saved.comparisons.iter().all(|c| c.actual == Some(c.expected)));
    assert_eq!(simulator.control().parameters()[0][7], 12_345);
    assert_eq!(
        saved.fields.iter().find(|f| f.name == "Name").unwrap().value,
        "Exact <xml> & value"
    );
    let bytes = shared
        .lock()
        .await
        .bundle
        .as_ref()
        .unwrap()
        .document(openlaser_xml::Kind::Backup)
        .unwrap()
        .original()
        .to_vec();
    let retained = String::from_utf8(bytes).unwrap();
    assert!(retained.contains("<!-- retained exactly -->"));
    assert!(retained.contains("Unused='0.0000001'"));
    assert!(retained.contains("Name='Exact &lt;xml&gt; &amp; value'"));
    assert!(
        settings::save(&shared, settings::Change { expected: before.sha256, edits: vec![] })
            .await
            .is_err()
    );
    assert_eq!(settings::view(&shared).await.unwrap().sha256, saved.sha256);
    simulator.control().clear_writes();
    machine::parameters(&shared, false, None).await.unwrap();
    assert!(simulator.control().writes().is_empty(), "Read must not write controller settings");
    machine::parameters(&shared, true, Some(&saved.sha256)).await.unwrap();
    assert!(simulator.control().writes().iter().any(|w| w.address == 60_000));
    openlaser_server::shutdown(&shared).await.unwrap();
}

#[tokio::test]
async fn mismatches_name_xml_fields_and_do_not_replace_the_file() {
    let (simulator, shared) = common::start("xml-mismatch").await;
    connect::connect(&shared).await.unwrap();
    let source = settings::view(&shared).await.unwrap();
    // A fresh binding invalidates prior authority before a different observed
    // configuration is injected into the isolated controller fixture.
    let bindings =
        openlaser_server::bindings::controller(shared.lock().await.bound.as_ref().unwrap());
    shared.machine.configure(bindings).await.unwrap();
    let control = simulator.control();
    let mut banks = control.parameters();
    banks[0][9] = 62_080;
    control.set_parameters(banks);
    control.clear_writes();
    assert!(machine::parameters(&shared, false, None).await.is_err());
    let read = settings::view(&shared).await.unwrap();
    let mismatch = read.comparisons.iter().find(|c| c.address == 50_218).unwrap();
    assert_eq!((mismatch.expected, mismatch.actual), (31_040, Some(62_080)));
    assert_eq!(mismatch.fields, ["/ParameterRoot/PMachineAxisConfig_0/MAC/@SpeedRatio"]);
    assert_eq!(source.sha256, read.sha256);
    assert!(control.writes().is_empty());
    machine::parameters(&shared, true, Some(&read.sha256)).await.unwrap();
    let fixed = settings::view(&shared).await.unwrap();
    assert!(fixed.problem.is_none());
    assert!(fixed.comparisons.iter().all(|c| c.actual == Some(c.expected)));
    assert_eq!(fixed.sha256, source.sha256);
    openlaser_server::shutdown(&shared).await.unwrap();
}
