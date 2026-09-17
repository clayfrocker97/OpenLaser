// SPDX-License-Identifier: GPL-3.0-or-later

//! Recipe binding, validation and process settings tests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::unnecessary_lazy_evaluations,
    reason = "test code builds fixtures and compares whole structures"
)]

use openlaser_compiler::settings::{PrePierce, Settings, Timeouts};
use openlaser_core::LaserMode;
use openlaser_xml::recipe::{self, Request};
use openlaser_xml::{Bundle, Document, Error, Kind};
use serde_json::{Value, json};

/// Synthetic backup with one fiber and one CO2 layer.
fn backup() -> String {
    let gp = "NoManu='0' ManuType='0' CutSpeed='50' CutPower='35' CutDuty='5' CutFreq='5000' CutPeakCurrent='100' CutGasType='5' CutAirPressure='0.85' CutHeight='0.5' UpHeight='15' NoFollow='0' LaserOnDelay='10' LaserOffBeforeDelay='0' LaserOffAfterDelay='0' PowerAdjustWithSpeed='1' PWMCurveNodes='0,10,100,100' FreqAdjustWithSpeed='0'";
    format!(
        "<ParameterRoot><PAxisParam><AX InterpolationCycle='1000'/></PAxisParam><PMachineAxisConfig_0><MAC WritePluse='8000' SpeedRatio='31.04' SoftLimitMaxLen='1371'/></PMachineAxisConfig_0><PMachineAxisConfig_1><MAC_1 WritePluse='8000' SpeedRatio='31.04' SoftLimitMaxLen='971'/></PMachineAxisConfig_1><PManuParam><MC CornerAccuracyRate='0.02' SplineAccuracyRate='0.01' ManuAcc='5000' AccTime='100' XFastMoveSpeed='500' XFastMoveAcc='5000' EmptyMoveAccTime='100' ResumeBackLength='2'/><MP LaserDAKeepOutput='0' DirectDrillMaxHeight='6' EnableManuCrashProtect='1' ManuCrashProtectUpHeight='35'/><GC GasDelay='100' DirectGasDelay='100' ChangeGasDelay='100'/></PManuParam><PFCParam><FCP MaxSpeed='3000'/></PFCParam><PZFParam><ZF ZFType='1' ZFUpSpeed='100' ZFFollowSpeed='100'/></PZFParam><PLaserParam><LGP LaserControlType='3' LaserType='0' LaserDAPort='1' LaserDAType='0' DOLaserGate='5' DOLaser='0' CO2LaserControlType='2' CO2LaserDAType='1' CO2LaserDAPort='1' CO2DOLaserGate='0' CO2DOLaser='9'/></PLaserParam><PGasParam><MGP LowAir='0' LowO2='7' LowN2='0' HighAir='3' HighO2='0' HighN2='2' RatioAirSwitch='0' RatioO2Switch='0' RatioN2Switch='0' RatioAir='0' RatioO2='2' RatioH2='0' NewDAMAxPressureAir='10' NewDAMAxPressureO2='10' NewDAMAxPressureN2='10'/></PGasParam><PLayerParam1><GP {gp}/></PLayerParam1><PCO2LayerParam1><GP {gp}/></PCO2LayerParam1></ParameterRoot>"
    )
}

/// Apply process options to the synthetic backup template.
fn process_xml(input: &Value) -> String {
    let mut attrs =
        format!("ManuType='{}' AdvFixHeightCutPos='0'", input["kind"].as_u64().unwrap());
    for i in 0..5 {
        use std::fmt::Write as _;
        write!(
            attrs,
            " DrillHeight{i}='{}' DrillPower{i}='{}' DrillFreq{i}='{}' DrillPeakCurrent{i}='{}' DrillGasType{i}='{}' DrillGasPressure{i}='{}' DrillDelay{i}='7' GradualTime{i}='7' EnableGradualDrill{i}='{}' BeforeLaserOffDelay{i}='{}' AfterLaserOffDelay{i}='{}' BoltDrill_Enable_{}='{}' BoltDrill_Power_{}='{}' BoltDrill_Freq_{}='{}'",
            2 + i,
            20 + i * 5,
            1000 + i * 100,
            80 + i,
            1 + i % 3,
            2.5 + f64::from(i),
            u8::from(input["gradual"] == true),
            3 + i,
            2 + i,
            i + 1,
            u8::from(input["bolt"] == true),
            i + 1,
            10 + i,
            i + 1,
            500 + i * 50
        )
        .unwrap();
    }
    if input["smooth"] == true {
        attrs += " EnableSmoothPierce='1' SmoothPierceDrillHeight='2' SmoothPierceDrillPower='20' SmoothPierceDrillFreq='1000' SmoothPierceDrillPeakCurrent='80' SmoothPierceDrillTime_ms='7'";
    }
    if input["residue"] == true {
        attrs += " CleanResidue_Enable='1' CleanResidue_WorkH='2' CleanResidue_WorkV='10' CleanResidue_GasType='2' CleanResidue_GasP='3' CleanResidue_PeakCurrent='0' CleanResidue_Power='31' CleanResidue_Freq='1500' CleanResidue_WorkR='0.2' CleanResidue_SpiralTimes='1'";
    }
    let xml = backup()
        .replace("LowAir='0' LowO2='7' LowN2='0' HighAir='3' HighO2='0' HighN2='2' RatioAirSwitch='0' RatioO2Switch='0' RatioN2Switch='0' RatioAir='0' RatioO2='2' RatioH2='0' NewDAMAxPressureAir='10' NewDAMAxPressureO2='10' NewDAMAxPressureN2='10'", "LowAir='11' LowO2='12' LowN2='13' HighAir='14' HighO2='15' HighN2='16' RatioAirSwitch='20' RatioO2Switch='21' RatioN2Switch='22' RatioAir='1' RatioO2='2' RatioH2='2' NewDAMAxPressureAir='10' NewDAMAxPressureO2='11' NewDAMAxPressureN2='12'")
        .replace("CutGasType='5'", "CutGasType='1'")
        .replace("CutAirPressure='0.85'", "CutAirPressure='2.5'")
        .replace("LaserControlType='3'", "LaserControlType='1'")
        .replace("ManuType='0'", &attrs)
        .replace("CutHeight='0.5'", "CutHeight='1'")
        .replace("CutPower='35'", "CutPower='45'")
        .replace("CutFreq='5000'", "CutFreq='2000'")
        .replace("LaserOnDelay='10'", "LaserOnDelay='9'")
        .replace("DirectDrillMaxHeight='6'", "DirectDrillMaxHeight='5'")
        .replace("ZFUpSpeed='100'", "ZFUpSpeed='50'")
        .replace("ZFFollowSpeed='100'", "ZFFollowSpeed='50'")
        .replace("DOLaser='0'", "DOLaser='3'")
        .replace("DOLaserGate='5'", "DOLaserGate='0'")
        .replace("EnableManuCrashProtect='1'", "EnableManuCrashProtect='0' AdvFixHeightCutPos='0'")
        .replace("GasDelay='100'", "GasDelay='0'")
        .replace("DirectGasDelay='100'", "DirectGasDelay='0'")
        .replace("ChangeGasDelay='100'", "ChangeGasDelay='0'");
    match input["smooth_height"].as_f64() {
        Some(height) => xml
            .replace("SmoothPierceDrillHeight='2'", &format!("SmoothPierceDrillHeight='{height}'")),
        None => xml,
    }
}

fn bundle(xml: &str) -> Bundle {
    Bundle::from_backup(Document::parse(Kind::Backup, xml.as_bytes()).unwrap())
}

fn request(mode: LaserMode, dry_run: bool, manual_focus: bool) -> Request {
    Request { mode, layer: 1, dry_run, manual_focus, film: false, timeouts: Timeouts::default() }
}

fn bind(xml: &str, mode: LaserMode) -> Result<Settings, Error> {
    recipe::bind(&bundle(xml), request(mode, false, false))
}

/// Mlaser's graph styles never change the saved control nodes or the
/// compiled power/frequency curves. Both laser modes retain the metadata.
#[test]
fn graph_smoothing_does_not_reject_or_change_process_curves() {
    let nodes = "0,75,27,89,57,97,100,100";
    let xml = backup()
        .replace("PWMCurveNodes='0,10,100,100'", &format!("PWMCurveNodes='{nodes}'"))
        .replace(
            "FreqAdjustWithSpeed='0'",
            &format!("FreqAdjustWithSpeed='1' FreqCurveNodes='{nodes}'"),
        );
    for mode in [LaserMode::Fiber, LaserMode::Co2] {
        let expected = bind(&xml, mode).unwrap();
        assert_eq!(expected.power_curve, expected.frequency_curve);
        assert_eq!(
            expected.power_curve,
            Some(vec![(0., 75.), (27., 89.), (57., 97.), (100., 100.)])
        );
        for style in 0..=10 {
            let styled = xml.replace(
                "<GP ",
                &format!("<GP PowerCurveSmoothType='{style}' FreqCurveSmoothType='{style}' "),
            );
            let source = bundle(&styled);
            assert_eq!(recipe::bind(&source, request(mode, false, false)).unwrap(), expected);
            let group = if mode == LaserMode::Fiber { "LayerParam1" } else { "CO2LayerParam1" };
            let attrs = source.group(group, "GP").unwrap();
            assert_eq!(attrs["PowerCurveSmoothType"], style.to_string());
            assert_eq!(attrs["FreqCurveSmoothType"], style.to_string());
            assert_eq!(attrs["PWMCurveNodes"], nodes);
            assert_eq!(attrs["FreqCurveNodes"], nodes);
        }
    }
}

/// Optional means absent, not duplicated or malformed. An active gas map
/// requires its setting; explicitly blank text keeps the documented fallback.
#[test]
fn optional_groups_and_active_calibration_preserve_errors() {
    let xml = backup();
    let add = |groups: &str| xml.replace("</ParameterRoot>", &format!("{groups}</ParameterRoot>"));
    for groups in ["<PManuParam><FC/><FC/></PManuParam>", "<PDOParam><DO/><DO/></PDOParam>"] {
        assert!(matches!(bind(&add(groups), LaserMode::Fiber), Err(Error::Duplicate(_))));
    }
    let duplicate_mp = xml.replace("</PManuParam>", "<MP/></PManuParam>");
    assert!(matches!(
        recipe::bind(&bundle(&duplicate_mp), request(LaserMode::Fiber, true, false)),
        Err(Error::Duplicate(_))
    ));
    let enabled = "<PDOParam><DO GtO2EnableGasDAMap='1'/></PDOParam>";
    for calibration in ["", "<PDIParam><DO/></PDIParam>"] {
        assert!(matches!(
            bind(&add(&format!("{enabled}{calibration}")), LaserMode::Fiber),
            Err(Error::Missing(_))
        ));
    }
    let blank = format!("{enabled}<PDIParam><DO GtO2GasDAMapStr=''/></PDIParam>");
    assert!(bind(&add(&blank), LaserMode::Fiber).is_ok());
    let invalid = blank.replace("GasDAMapStr=''", "GasDAMapStr='broken'");
    assert!(matches!(bind(&add(&invalid), LaserMode::Fiber), Err(Error::Invalid { .. })));
    let disabled = invalid.replace("EnableGasDAMap='1'", "EnableGasDAMap='0'");
    assert!(bind(&add(&disabled), LaserMode::Fiber).is_ok());
}

/// Binding a recipe overlay has the same settings as a lossless layer-file
/// import, including fallback fields, while leaving the documents untouched.
#[test]
fn borrowed_recipe_overlay_matches_import_and_rejects_duplicate_defaults() {
    let mut source = bundle(&backup());
    let before = recipe::bind(&source, request(LaserMode::Fiber, false, false)).unwrap();
    let overlay = [("CutSpeed".into(), "72.5".into())].into();
    let settings =
        recipe::bind_overlay(&source, request(LaserMode::Fiber, false, false), &overlay).unwrap();
    assert_eq!(settings.speed, 72.5);
    assert_eq!(before.speed, 50.);
    let mut merged = source.group("LayerParam1", "GP").unwrap().clone();
    merged.extend(overlay.clone());
    source.insert(openlaser_xml::layer_file::write(LaserMode::Fiber, 1, &merged).unwrap());
    assert_eq!(settings, recipe::bind(&source, request(LaserMode::Fiber, false, false)).unwrap());
    let duplicate = backup().replace("</PLayerParam1>", "<GP/></PLayerParam1>");
    assert!(matches!(
        recipe::bind_overlay(
            &bundle(&duplicate),
            request(LaserMode::Fiber, false, false),
            &overlay
        ),
        Err(Error::Duplicate(_))
    ));
}

/// One process capture's recipe, as the compiler's golden tests build it.
/// The CO2 bank of the same template: secondary PWM with contour control,
/// high-air assist, the CO2 laser output, the duty as power, and the head still on.
#[test]
fn co2_bank_binds_the_co2_hardware() {
    let s = bind(&backup(), LaserMode::Co2).unwrap();
    let h = s.hardware.as_ref().unwrap();
    assert!(h.secondary_pwm && h.co2_contour_control && h.gas_enabled);
    assert_eq!(h.gas_ports, [None, None, None, Some(3), None, None]);
    assert!(h.ratio_channels.iter().all(Option::is_none));
    assert_eq!((h.gate, h.laser, h.peak_channel, h.analog_motion), (None, Some(9), None, None));
    assert!(h.z_enabled);
    assert_eq!((h.crash_height, h.crash_resume_value), (Some(35), 0));
    assert_eq!((s.power, s.frequency, s.gas, s.pressure, s.peak_current), (5, 5000, 3, 0., 0.));
    assert_eq!((s.cut_height, s.retract_height, s.z_up_speed), (0.5, 15., 100.));
}

#[test]
fn co2_piercing_and_residue_use_high_air_without_pressure_outputs() {
    let s = bind(&process_xml(&json!({"kind": 3, "residue": true})), LaserMode::Co2).unwrap();
    assert_eq!((s.gas, s.pressure), (3, 0.));
    assert!(!s.pierce.is_empty());
    assert!(s.pierce.iter().all(|p| (p.gas, p.pressure) == (3, 0.)));
    let residue = s.residue.unwrap();
    assert_eq!((residue.gas, residue.pressure), (3, 0.));
    let dry = recipe::bind(&bundle(&backup()), request(LaserMode::Co2, true, false)).unwrap();
    assert!(dry.hardware.is_none());
    assert_eq!((dry.gas, dry.pressure), (0, 0.));
    assert!(matches!(bind(&backup().replace("HighAir='3'", "HighAir='0'"), LaserMode::Co2),
        Err(Error::Unsupported(message)) if message.contains("HighAir valve")));
}

/// A dry run keeps the motion settings and drops every process setting.
#[test]
fn a_dry_run_strips_the_process() {
    let dry = recipe::bind(
        &bundle(&process_xml(&json!({"kind": 3}))),
        request(LaserMode::Fiber, true, false),
    )
    .unwrap();
    assert!(
        dry.dry_run && dry.hardware.is_none() && dry.pierce.is_empty() && dry.residue.is_none()
    );
    assert_eq!(
        (dry.power, dry.gas, dry.pressure, dry.on_delay_ms, dry.power_curve),
        (0, 0, 0., 0, None)
    );
    assert_eq!((dry.speed, dry.acceleration, dry.cut_height), (50., 5000., 1.));
}

/// The CO2 height-controller bypass has no bearing on manually set optical
/// focus. It cannot disable fiber Z control.
#[test]
fn co2_height_bypass_cannot_disable_fiber_z() {
    let piercing = recipe::bind(
        &bundle(&process_xml(&json!({"kind": 3}))),
        request(LaserMode::Fiber, false, true),
    );
    assert!(matches!(piercing, Err(Error::Unsupported(_))), "{piercing:?}");
    let flat = recipe::bind(
        &bundle(&process_xml(&json!({"kind": 0}))),
        request(LaserMode::Co2, false, true),
    )
    .unwrap();
    let hardware = flat.hardware.unwrap();
    assert!(!hardware.z_enabled && hardware.crash_height.is_none());
    assert!(!flat.head_travel.frog_jump && !flat.head_travel.protect_insertions);
}

/// A disabled layer, an unknown machining type, a missing group and a
/// missing layer are refused with their reasons.
#[test]
fn unsupported_layers_are_refused() {
    let refused = |xml: String| bind(&xml, LaserMode::Fiber).unwrap_err();
    let unsupported = |error: Error| matches!(error, Error::Unsupported(_));
    assert!(unsupported(refused(backup().replace("NoManu='0'", "NoManu='1'"))));
    assert!(unsupported(refused(backup().replace("ManuType='0'", "ManuType='8'"))));
    let missing = refused(backup().replace("<PFCParam><FCP MaxSpeed='3000'/></PFCParam>", ""));
    assert!(
        matches!(missing, Error::Missing(ref field) if field == "FCParam.FCP.MaxSpeed"),
        "{missing:?}"
    );
    let mut request = request(LaserMode::Fiber, false, false);
    request.layer = 2;
    assert!(matches!(recipe::bind(&bundle(&backup()), request), Err(Error::Missing(_))));
}

/// The recipe's flags for the pass scheduler bind on a fiber process:
/// preliminary piercing with its batch size from the operating settings,
/// its repeat and keep-down switches, the film switch with the machine's
/// film batch, and the two gas-retention switches. A recipe with no
/// piercing stages keeps `PreDrill` as data, a zero batch is refused, and
/// smooth piercing refuses separate pre-piercing.
#[test]
fn pass_and_gas_switches_bind() {
    let flags = "NoManu='0' PreDrill='1' AfterPreDrillMustDrillBeforeCut='1' PreDrillIsNotUp='1' WithFilm='1' NoCloseGasInManu='1' ShortDistGasKeepOn='1'";
    let operating = |batch: &str, xml: String| {
        xml.replace(
            "<ParameterRoot>",
            &format!("<ParameterRoot><PSoftParam><GP PreDrillMaxNum='{batch}'/></PSoftParam>"),
        )
        .replace("ResumeBackLength='2'/>", "ResumeBackLength='2' ClearUpFilmNum_Pre='3'/>")
        .replace("</PManuParam>", "<FC ShortNoUpMaxLength='12.5'/></PManuParam>")
    };
    let staged = operating("4", process_xml(&json!({"kind": 3})).replace("NoManu='0'", flags));
    let s = bind(&staged, LaserMode::Fiber).unwrap();
    assert_eq!(
        s.pre_pierce,
        Some(PrePierce { batch: 4, repeat_before_cut: true, keep_down: true })
    );
    assert!(s.with_film && s.keep_gas_after && s.short_gas_keep);
    assert_eq!(s.film_batch, 3);
    assert_eq!(s.head_travel.short_transfer, Some(12.5));
    let flat = bind(&backup().replace("NoManu='0'", flags), LaserMode::Fiber).unwrap();
    assert_eq!(flat.pre_pierce, None, "no stages, so PreDrill is kept as data");
    assert!(flat.with_film && flat.keep_gas_after);
    let zero = operating("0", process_xml(&json!({"kind": 3})).replace("NoManu='0'", flags));
    let zero = bind(&zero, LaserMode::Fiber);
    assert!(
        matches!(zero, Err(Error::Invalid { ref reason, .. }) if reason == "PreDrillMaxNum must be positive"),
        "{zero:?}"
    );
    let smooth = process_xml(&json!({"kind": 3, "smooth": true})).replace("NoManu='0'", flags);
    assert!(matches!(
        bind(&smooth, LaserMode::Fiber),
        Err(Error::Unsupported(ref why)) if why == "Separate pre-piercing is incompatible with smooth piercing"
    ));
    let co2 = bind(&backup().replace("NoManu='0'", flags), LaserMode::Co2).unwrap();
    assert!(co2.keep_gas_after && co2.short_gas_keep);
    let dry = recipe::bind(
        &bundle(&backup().replace("NoManu='0'", flags)),
        request(LaserMode::Fiber, true, false),
    )
    .unwrap();
    assert!(dry.pre_pierce.is_none() && !dry.with_film && !dry.keep_gas_after);
}

/// A film request binds the bank as cutting motion alone: no stages, no
/// smooth piercing, no residue cleaning, and none of the pass switches,
/// whatever the bank says.
#[test]
fn a_film_request_binds_cutting_motion_alone() {
    let xml = process_xml(&json!({"kind": 3, "residue": true}))
        .replace("NoManu='0'", "NoManu='0' PreDrill='1' WithFilm='1' EnableSmoothPierce='1'");
    let mut request = request(LaserMode::Fiber, false, false);
    request.film = true;
    let film = recipe::bind(&bundle(&xml), request).unwrap();
    assert!(film.pierce.is_empty() && film.residue.is_none() && !film.smooth_pierce);
    assert_eq!(film.machining_kind, 0);
    assert!(film.pre_pierce.is_none() && !film.with_film);
    assert_eq!(film.power, 45);
}

/// The three names of a stage's duration are one slot: any present subset
/// binds when they agree, the third alone binds, a disagreement names
/// the conflict, and none means zero.
#[test]
fn the_three_duration_aliases_are_one_slot() {
    let stage = |attrs: &str| {
        let xml =
            process_xml(&json!({"kind": 2})).replace("DrillDelay0='7' GradualTime0='7'", attrs);
        bind(&xml, LaserMode::Fiber).map(|s| s.pierce[0].duration_ms)
    };
    assert_eq!(stage("FocusGradualTime0='300'").unwrap(), 300);
    assert_eq!(stage("DrillDelay0='600' GradualTime0='600' FocusGradualTime0='600'").unwrap(), 600);
    assert_eq!(stage("").unwrap(), 0);
    let conflict = stage("DrillDelay0='300' GradualTime0='300' FocusGradualTime0='301'");
    assert!(
        matches!(conflict, Err(Error::Invalid { ref field, ref reason })
            if field == "LayerParam1.FocusGradualTime0" && reason == "Conflicting aliases for stage 0 duration"),
        "{conflict:?}"
    );
}

#[test]
fn duration_edits_canonicalize_only_touched_slots_and_refuse_atomically() {
    use openlaser_xml::document::Attributes;
    let fields = |pairs: &[(&str, &str)]| -> Attributes {
        pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    };
    let mut stored = fields(&[
        ("DrillDelay0", "100"),
        ("FocusGradualTime0", "100"),
        ("GradualTime1", "20"),
        ("UnknownField", "kept"),
    ]);
    recipe::edit_attributes(&mut stored, &fields(&[("FocusGradualTime0", "300")])).unwrap();
    assert_eq!(stored["DrillDelay0"], "300");
    assert!(!stored.contains_key("FocusGradualTime0"));
    assert_eq!(stored["GradualTime1"], "20");
    assert_eq!(stored["UnknownField"], "kept");
    for edits in [
        fields(&[("DrillDelay0", "10"), ("GradualTime0", "11"), ("UnknownField", "lost")]),
        fields(&[("DrillDelay0", "600001")]),
    ] {
        let before = stored.clone();
        assert!(recipe::edit_attributes(&mut stored, &edits).is_err());
        assert_eq!(stored, before);
    }
}

/// The contour shift binds as the older host reads it: nothing when
/// disabled, the two distances within their domain when enabled.
#[test]
fn the_contour_shift_binds_when_enabled() {
    let off = bind(
        &backup().replace("NoManu='0'", "NoManu='0' ContourShiftXDist='10' ContourShiftYDist='-3'"),
        LaserMode::Fiber,
    )
    .unwrap();
    assert_eq!(off.contour_shift, [0., 0.]);
    let on = bind(
        &backup().replace(
            "NoManu='0'",
            "NoManu='0' EnableContourShift='1' ContourShiftXDist='10' ContourShiftYDist='-3'",
        ),
        LaserMode::Fiber,
    )
    .unwrap();
    assert_eq!(on.contour_shift, [10., -3.]);
    let far = bind(
        &backup().replace(
            "NoManu='0'",
            "NoManu='0' EnableContourShift='1' ContourShiftXDist='100001' ContourShiftYDist='0'",
        ),
        LaserMode::Fiber,
    );
    assert!(matches!(far, Err(Error::Invalid { .. })));
}

/// The machine geometry: counts per millimetre from pulses over the ratio,
/// the soft limits, and the interpolation cycle.
#[test]
fn machine_geometry_binds() {
    let machine = recipe::machine(&bundle(&backup())).unwrap();
    assert_eq!(machine.counts_per_mm, [8000. / 31.04; 2]);
    assert_eq!(machine.travel_limits, [1371., 971.]);
    assert_eq!(machine.interpolation_cycle_us, 1000);
}
