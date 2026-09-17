// SPDX-License-Identifier: GPL-3.0-or-later

//! The ordinary MCC machine initialization in NCModule 0x1002e100.
//! Full banks replace the previous host's values. The internal bank word 4
//! starts at 200000 in MainApp 0x00423920; `AccelerationTime` is a host setting.
//! Addresses count 16-bit registers. Optional machine branches are validated
//! before any write is returned.

use crate::{Document, Error, Result, parameters};
use openlaser_core::LaserMode;
use openlaser_protocol::requests::{self, OutputBank, Write};

/// One readback comparison, with the XML fields that determine its value.
#[derive(Clone, Debug)]
pub struct Check {
    /// Register address, in 16-bit units.
    pub address: u32,
    /// Bits owned by these fields.
    pub mask: u32,
    /// Expected masked controller value.
    pub value: u32,
    /// Full XML attribute paths, or the native default's description.
    pub fields: Vec<String>,
}

/// Initialization writes and the words that must subsequently read back.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Ordered native initialization writes.
    pub writes: Vec<Write>,
    /// Word comparisons; packed settings share a register.
    pub checks: Vec<Check>,
    /// Standard idle outputs that initialization intentionally enables.
    pub idle_outputs: u16,
}

fn field(group: &str, tag: &str, key: &str) -> String {
    format!("/ParameterRoot/P{group}/{tag}/@{key}")
}

fn number(doc: &Document, group: &str, tag: &str, key: &str, default: Option<f64>) -> Result<f64> {
    let path = format!("/ParameterRoot/P{group}/{tag}");
    let raw = match doc.attributes(&path) {
        Ok(attributes) => attributes.get(key),
        Err(Error::Missing(_)) => None,
        Err(error) => return Err(error),
    };
    let name = field(group, tag, key);
    let value = match raw {
        Some(value) => value.parse::<f64>().map_err(|_| Error::Invalid {
            field: name.clone(),
            reason: "expected a number".into(),
        })?,
        None => default.ok_or_else(|| Error::Missing(name.clone()))?,
    };
    if !value.is_finite() || value < 0. || value > f64::from(i32::MAX) {
        return Err(Error::Invalid {
            field: name,
            reason: "outside the controller's range".into(),
        });
    }
    Ok(value)
}

fn integer(
    doc: &Document,
    group: &str,
    tag: &str,
    key: &str,
    default: Option<u32>,
    max: u32,
) -> Result<u32> {
    let value = number(doc, group, tag, key, default.map(f64::from))?;
    crate::whole(value).filter(|v| *v <= max).ok_or_else(|| Error::Invalid {
        field: field(group, tag, key),
        reason: format!("expected an integer from 0 to {max}"),
    })
}

fn signed(value: f64, name: &str) -> Result<u32> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(Error::Invalid {
            field: name.into(),
            reason: "scaled value exceeds a signed word".into(),
        });
    }
    // Float-to-int conversion is the native truncation toward zero.
    #[allow(clippy::cast_possible_truncation, reason = "checked signed range above")]
    Ok((value.trunc() as i32).cast_unsigned())
}

impl Plan {
    /// Builds the complete ordinary axis, system, I/O and laser-mode setup.
    pub fn from_document(doc: &Document, scale: i32, mode: LaserMode) -> Result<Self> {
        let projection = parameters::Plan::from_document(doc, scale)?;
        for (group, tag, key) in [
            ("MachineAxisConfig", "MAC", "DoubleDevice"),
            ("ManuParam", "AF", "AFType"),
            ("ManuParam", "MP", "EnableVerCorrect"),
        ] {
            if integer(doc, group, tag, key, Some(0), u32::MAX)? != 0 {
                return Err(Error::Unsupported(format!(
                    "{} needs a separate machine initialization branch",
                    field(group, tag, key)
                )));
            }
        }
        if mode == LaserMode::Co2
            && integer(doc, "ManuParam", "MP", "CO2EnableSecondSoftLimit", Some(0), 1)? != 0
        {
            return Err(Error::Unsupported(format!(
                "{} needs the secondary CO2 travel-limit initialization branch",
                field("ManuParam", "MP", "CO2EnableSecondSoftLimit")
            )));
        }
        let mut plan = Self {
            writes: vec![Write { address: 5001, words: vec![9999, 9, 65535, 0] }],
            checks: vec![],
            idle_outputs: 0,
        };
        let head = integer(doc, "ZFParam", "ZF", "ZFType", None, 10)? != 0;
        plan.alarm_inputs(doc)?;
        plan.axes(doc, scale, head, &projection)?;
        plan.motion_io(doc, head)?;
        plan.laser(doc, mode)?;
        plan.register(50008, 0, vec![field("ManuParam", "AF", "AFType")]);
        plan.writes.push(requests::parameters_activate());
        // Native 0x1003ebb0 pairs two 16-bit source addresses per map word.
        let addresses = (0..50)
            .map(|i| 2000 + 2 * i)
            .chain((0..5).flat_map(|axis| (0..14).map(move |word| 50200 + 40 * axis + 2 * word)))
            .collect::<Vec<u32>>();
        plan.writes.push(Write {
            address: 60000,
            words: addresses.chunks_exact(2).map(|p| p[0] | p[1] << 16).collect(),
        });
        plan.writes.push(requests::fifo_clear());
        Ok(plan)
    }

    fn alarm_inputs(&mut self, doc: &Document) -> Result<()> {
        let estop = integer(doc, "DIParam", "DI", "EStop", Some(0), 255)?;
        let servo = integer(doc, "DIParam", "DI", "ServoAlarm", Some(0), 255)?;
        let servo_type = integer(doc, "DIParam", "DI", "ServoAlarmType", Some(0), 1)?;
        self.register(
            50026,
            ((servo_type << 21 | servo | 0x0010_0000) << 8) | estop,
            ["EStop", "ServoAlarm", "ServoAlarmType"]
                .map(|key| field("DIParam", "DI", key))
                .to_vec(),
        );
        self.register(50028, 0, vec![field("MachineAxisConfig", "MAC", "DoubleDevice")]);
        let brake = integer(
            doc,
            "MachineAxisConfig",
            "MAC",
            "CloseBreakTime",
            Some(500),
            i32::MAX.cast_unsigned(),
        )?;
        self.register(50036, brake, vec![field("MachineAxisConfig", "MAC", "CloseBreakTime")]);
        self.writes.push(requests::alarm_clear());
        Ok(())
    }

    fn axes(
        &mut self,
        doc: &Document,
        scale: i32,
        head: bool,
        projection: &parameters::Plan,
    ) -> Result<()> {
        let soft = integer(doc, "ManuParam", "MS", "EnableSoftLimit", None, 1)?;
        let head_soft = integer(doc, "ECParam", "ZF", "zfSoftLimitEnable", Some(0), 1)?;
        for axis in 0u8..5 {
            let group = format!("MachineAxisConfig_{axis}");
            let tag = if axis == 0 { "MAC".into() } else { format!("MAC_{axis}") };
            let enabled = if head && axis == 3 { head_soft } else { soft };
            let bank = axis_bank(doc, scale, axis, enabled, projection)?;
            self.writes.push(requests::parameter_bank(axis, bank)?);
            for patch in projection.patches.iter().filter(|p| p.axis == axis) {
                self.checks.push(Check {
                    address: openlaser_protocol::registers::parameter_bank(axis)
                        + 2 * u32::try_from(patch.word).unwrap_or(0),
                    mask: patch.mask,
                    value: patch.value,
                    fields: vec![patch.field.clone()],
                });
            }
            let soft_field = if head && axis == 3 {
                field("ECParam", "ZF", "zfSoftLimitEnable")
            } else {
                field("ManuParam", "MS", "EnableSoftLimit")
            };
            self.checks.push(Check {
                address: openlaser_protocol::registers::parameter_bank(axis),
                mask: 1 << 17,
                value: bank[0] & (1 << 17),
                fields: vec![soft_field],
            });
            for (word, fields) in [
                (0, vec!["Native axis flag defaults".into()]),
                (
                    1,
                    vec![
                        field(&group, &tag, "SoftLimitMaxLen"),
                        field(&group, &tag, "GoOriginalDirection"),
                    ],
                ),
                (
                    2,
                    vec![
                        field(&group, &tag, "SoftLimitMaxLen"),
                        field(&group, &tag, "GoOriginalDirection"),
                    ],
                ),
                (4, vec!["Native axis ramp default (200000)".into()]),
                (7, vec![field(&group, &tag, "ReturnLength")]),
                (8, vec!["Native axis reserved word (0)".into()]),
            ] {
                let mask = if word == 0 { !0x0002_07f7 } else { u32::MAX };
                self.checks.push(Check {
                    address: openlaser_protocol::registers::parameter_bank(axis)
                        + u32::try_from(word * 2).unwrap_or(0),
                    mask,
                    value: bank[word] & mask,
                    fields,
                });
            }
        }
        Ok(())
    }

    fn motion_io(&mut self, doc: &Document, head: bool) -> Result<()> {
        let factor = integer(doc, "SoftParam", "SP", "LimitDeccFactor", Some(1), 1000)?;
        let decel = f64::from(factor) * number(doc, "FCParam", "FCP", "MaxAcc", None)?;
        for (address, value) in [(50022, decel), (50024, decel * 10.)] {
            self.register(
                address,
                signed(value, "limit deceleration")?,
                vec![
                    field("SoftParam", "SP", "LimitDeccFactor"),
                    field("FCParam", "FCP", "MaxAcc"),
                ],
            );
        }
        // Bindings already refuse the keep-analog-output branch.
        self.writes.extend([requests::analog_output(1, 0)?, requests::analog_output(2, 0)?]);
        for (group, tag, key) in
            [("DOParam", "DO", "WaitSignal"), ("LaserParam", "LGP", "DORemoteStart")]
        {
            let port = integer(doc, group, tag, key, Some(0), 10)?;
            if port != 0 {
                self.idle_outputs |= 1 << (port - 1);
            }
        }
        self.writes.push(requests::digital_outputs(
            OutputBank::Standard,
            u16::MAX,
            self.idle_outputs,
        ));
        if integer(doc, "ManuParam", "EC", "ECIOType", Some(0), 10)? != 0 {
            self.writes.push(requests::digital_outputs(OutputBank::Extended, u16::MAX, 0));
        }
        self.register(50012, if head { 4 } else { 0 }, vec![field("ZFParam", "ZF", "ZFType")]);
        for pair in 0..6 {
            let keys =
                [format!("DI{}SmoothTime", pair * 2 + 1), format!("DI{}SmoothTime", pair * 2 + 2)];
            let low = integer(doc, "DIParam", "DI", &keys[0], Some(0), 65535)?;
            let high = integer(doc, "DIParam", "DI", &keys[1], Some(0), 65535)?;
            self.register(
                50040 + pair * 2,
                low | high << 16,
                keys.map(|key| field("DIParam", "DI", &key)).to_vec(),
            );
        }
        Ok(())
    }

    fn laser(&mut self, doc: &Document, mode: LaserMode) -> Result<()> {
        self.writes.push(requests::head_mode(mode));
        let co2_enable = integer(doc, "LaserParam", "LGP", "doCO2EnableOutput", Some(0), 10)?;
        if co2_enable != 0 {
            let port = u8::try_from(co2_enable)
                .map_err(|_| Error::Unsupported("CO2 enable port".into()))?;
            self.writes.push(requests::digital_output(port, mode == LaserMode::Co2)?);
            if mode == LaserMode::Co2 {
                self.idle_outputs |= 1 << (port - 1);
            }
        }
        let (sync, port, address) = if mode == LaserMode::Co2 {
            let kind = integer(doc, "LaserParam", "LGP", "CO2LaserControlType", None, 3)?;
            if !matches!(kind, 1 | 2) {
                for address in [50106, 50108] {
                    self.register(
                        address,
                        0,
                        vec![field("LaserParam", "LGP", "CO2LaserControlType")],
                    );
                }
                return Ok(());
            }
            ("CO2PWMOutputSync", "CO2DOLaser", if kind == 2 { 50108 } else { 50106 })
        } else {
            ("PWMOutputSync", "DOLaser", 50106)
        };
        // MainApp property registrations 007f0ca6 and 007f0d5f: fiber 0, CO2 1.
        let enabled =
            integer(doc, "ManuParam", "MP", sync, Some(u32::from(mode == LaserMode::Co2)), 1)?;
        let port_value = integer(doc, "LaserParam", "LGP", port, Some(0), 26)?;
        self.register(
            address,
            if enabled == 1 { port_value } else { 0 },
            vec![field("ManuParam", "MP", sync), field("LaserParam", "LGP", port)],
        );
        Ok(())
    }

    fn register(&mut self, address: u32, value: u32, fields: Vec<String>) {
        self.writes.push(Write { address, words: vec![value] });
        self.checks.push(Check { address, mask: u32::MAX, value, fields });
    }
}

fn axis_bank(
    doc: &Document,
    scale: i32,
    axis: u8,
    enabled: u32,
    projection: &parameters::Plan,
) -> Result<[u32; 14]> {
    let group = format!("MachineAxisConfig_{axis}");
    let tag = if axis == 0 { "MAC".into() } else { format!("MAC_{axis}") };
    let mut bank: [u32; 14] = projection
        .merge(axis, &[0; 14])?
        .try_into()
        .map_err(|_| Error::Unsupported("incomplete axis bank".into()))?;
    // Require the complete bank input; partial XML must not zero a drive.
    for key in [
        "WritePluse",
        "SpeedRatio",
        "Acceleration",
        "FastSpeed",
        "SecondSpeed",
        "ReturnLength",
        "SoftLimitMaxLen",
        "GoOriginalDirection",
        "NegativeType",
        "ForwardType",
        "ZoreType",
        "SampleType",
        "SecondGoHome",
        "EnableZphaseSignal",
        "AxisReverse",
        "EncoderReverse",
        "IsRotatingShaft",
        "NegativeLimitInput",
        "ForwardLimitInput",
        "OriginalInput",
        "BrakeOutput",
    ] {
        number(doc, &group, &tag, key, None)?;
    }
    bank[0] |= (1 - enabled) << 17;
    let length = number(doc, &group, &tag, "SoftLimitMaxLen", None)?;
    if length <= 0. {
        return Err(Error::Invalid {
            field: field(&group, &tag, "SoftLimitMaxLen"),
            reason: "travel must be positive".into(),
        });
    }
    let direction = integer(doc, &group, &tag, "GoOriginalDirection", None, 1)?;
    let limits = if direction == 0 { [-0.5, length] } else { [-length, 0.5] };
    bank[1] = signed(limits[0] * f64::from(scale), "negative soft limit")?;
    bank[2] = signed(limits[1] * f64::from(scale), "positive soft limit")?;
    bank[4] = 200_000;
    bank[7] = signed(
        number(doc, &group, &tag, "ReturnLength", None)? * f64::from(scale),
        "home return length",
    )?;
    bank[8] = 0;
    Ok(bank)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    fn fixture() -> Document {
        Document::parse(Kind::Backup, include_bytes!("../../../fixtures/xml/initialization.xml"))
            .unwrap()
    }

    /// Golden words produced by the native x86 bank writer in isolation,
    /// with both native CRT conversion paths, using the fixture values.
    #[test]
    fn complete_banks_match_the_native_producer() {
        let plan = Plan::from_document(&fixture(), 1000, LaserMode::Fiber).unwrap();
        let expected = [
            [
                272,
                4_294_966_796,
                1_371_000,
                20_000,
                200_000,
                80_000,
                20_000,
                28_000,
                0,
                31_040,
                8000,
                8000,
                1286,
                0,
            ],
            [
                272,
                4_294_966_796,
                950_000,
                20_000,
                200_000,
                80_000,
                20_000,
                15_000,
                0,
                31_060,
                8000,
                8000,
                1800,
                0,
            ],
            [
                272,
                4_294_966_796,
                1_000_000,
                20_000,
                200_000,
                30_000,
                20_000,
                5000,
                0,
                5000,
                2000,
                2000,
                0,
                0,
            ],
            [
                131_088,
                4_294_966_796,
                1_000_000,
                20_000,
                200_000,
                30_000,
                20_000,
                10_000,
                0,
                5000,
                10_000,
                10_000,
                2569,
                0,
            ],
            [
                48,
                4_293_967_296,
                500,
                20_000,
                200_000,
                30_000,
                20_000,
                5000,
                0,
                10_000,
                2000,
                2000,
                770,
                0,
            ],
        ];
        let banks = plan.writes.iter().filter(|w| w.words.len() == 14).collect::<Vec<_>>();
        assert_eq!(banks.len(), expected.len());
        for (axis, (bank, expected)) in banks.iter().zip(expected).enumerate() {
            assert_eq!(bank.address, 50_200 + 40 * u32::try_from(axis).unwrap());
            assert_eq!(bank.words, expected);
        }
        assert_eq!(plan.writes.last(), Some(&requests::fifo_clear()));
        let monitor = plan.writes.iter().find(|w| w.address == 60000).unwrap();
        assert_eq!(monitor.words.len(), 60);
        assert_eq!(monitor.words[0], 0x07d2_07d0);
        assert_eq!(monitor.words[25], 0xc41a_c418);
        assert_eq!(monitor.words[59], 0xc4d2_c4d0);
    }

    /// Packed input fields, inverted head soft limits, native defaults and
    /// the secondary CO2 PWM route remain distinguishable in readback.
    #[test]
    fn system_settings_and_mode_have_exact_readback_fields() {
        let doc = fixture()
            .with_attribute("/ParameterRoot/PECParam/ZF", "zfSoftLimitEnable", "1")
            .unwrap();
        let plan = Plan::from_document(&doc, 1000, LaserMode::Co2).unwrap();
        let check = |address| plan.checks.iter().find(|c| c.address == address).unwrap().value;
        assert_eq!(check(50026), 0x1000_0000);
        assert_eq!(check(50036), 500);
        assert_eq!(check(50022), 20_000);
        assert_eq!(check(50024), 200_000);
        assert_eq!(check(50012), 4);
        assert_eq!(check(50108), 9);
        let head = plan.writes.iter().find(|w| w.address == 50320).unwrap();
        assert_eq!(head.words[0] & (1 << 17), 0);
        let fiber = Plan::from_document(&doc, 1000, LaserMode::Fiber).unwrap();
        assert_eq!(fiber.checks.iter().find(|c| c.address == 50106).unwrap().value, 0);
        assert!(!fiber.writes.iter().any(|w| w.address == 50108));
        let oversized = doc
            .with_attribute(
                "/ParameterRoot/PMachineAxisConfig_4/MAC_4",
                "SoftLimitMaxLen",
                "999999999",
            )
            .unwrap();
        assert!(Plan::from_document(&oversized, 1000, LaserMode::Fiber).is_err());
        let partial = Document::parse(Kind::Backup, br#"<ParameterRoot><PMachineAxisConfig_0><MAC WritePluse="8000"/></PMachineAxisConfig_0></ParameterRoot>"#).unwrap();
        assert!(Plan::from_document(&partial, 1000, LaserMode::Fiber).is_err());
        let secondary = String::from_utf8(doc.original().to_vec())
            .unwrap()
            .replace("<PManuParam>", "<PManuParam><MP CO2EnableSecondSoftLimit=\"1\" />");
        let secondary = Document::parse(Kind::Backup, secondary.as_bytes()).unwrap();
        assert!(Plan::from_document(&secondary, 1000, LaserMode::Co2).is_err());
        assert!(Plan::from_document(&secondary, 1000, LaserMode::Fiber).is_ok());
    }
}
