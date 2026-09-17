// SPDX-License-Identifier: GPL-3.0-or-later

//! The controller's axis parameter banks as the vendor projects its machine
//! axis configuration into them, NCModule `0x1002_E100` and `0x1002_F232`.
//!
//! Each of the five axes has a fourteen-word bank in the controller. The
//! host packs the binary flags into word 0, the pulse count into words 10
//! and 11, the acceleration into word 3, the limit and origin inputs into
//! word 12, the brake output into word 13, and the speeds, scaled by the
//! controller's coordinate scale, into words 5, 6 and 9. This partial
//! projection preserves other words. Full native startup initialization,
//! which writes all fourteen words, lives in `initialization`.

use crate::document::Document;
use crate::{Error, Result};
use openlaser_protocol::registers::{PARAMETER_BANK_WORDS, parameter_bank};
use std::collections::BTreeMap;

/// One masked write into an axis bank word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// The axis, 0 to 4.
    pub axis: u8,
    /// The bank word.
    pub word: usize,
    /// The bits the patch owns.
    pub mask: u32,
    /// Their value.
    pub value: u32,
    /// The attribute the patch came from.
    pub field: String,
}

/// The patches a document asks for, and the fields it holds that the
/// controller banks do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The controller's coordinate scale the speeds were scaled by.
    pub scale: i32,
    /// The masked writes.
    pub patches: Vec<Patch>,
    /// Attributes with no bank word, kept on the host only.
    pub host_only: Vec<String>,
}

/// The bank word each binary flag occupies in word 0.
fn flag_bit(key: &str) -> Option<u32> {
    Some(match key {
        "NegativeType" => 0,
        "ForwardType" => 1,
        "ZoreType" => 2,
        "SampleType" => 4,
        "GoOriginalDirection" => 5,
        "SecondGoHome" => 6,
        "EnableZphaseSignal" => 7,
        "AxisReverse" => 8,
        "EncoderReverse" => 9,
        "IsRotatingShaft" => 10,
        _ => return None,
    })
}

/// The axis an attribute path belongs to, when it is a machine axis group.
fn axis_of(path: &str) -> Option<u8> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 4 || parts[1] != "ParameterRoot" {
        return None;
    }
    let axis = parts[2].strip_prefix("PMachineAxisConfig_")?.parse::<u8>().ok()?;
    (axis < 5 && (parts[3] == "MAC" || parts[3] == format!("MAC_{axis}"))).then_some(axis)
}

fn invalid(field: &str, reason: &str) -> Error {
    Error::Invalid { field: field.to_owned(), reason: reason.to_owned() }
}

fn scaled_speed(text: &str, field: &str, scale: i32, is_ratio: bool) -> Result<u32> {
    let value = text.parse::<f64>().map_err(|_| invalid(field, "expected a speed"))?;
    let raw = value * f64::from(scale);
    if !raw.is_finite() || raw < 0. || raw > f64::from(i32::MAX) || (is_ratio && raw < 1.) {
        return Err(invalid(field, "outside the controller's range or below one scale unit"));
    }
    crate::whole(raw.trunc()).ok_or_else(|| invalid(field, "outside the controller's range"))
}

/// The patches one attribute asks for: `None` when the attribute has no
/// bank word, two for the pulse count, which the host writes twice.
fn attribute_patches(
    axis: u8,
    key: &str,
    text: &str,
    field: &str,
    scale: i32,
) -> Result<Option<Vec<Patch>>> {
    let patch =
        |word: usize, mask: u32, value: u32| Patch { axis, word, mask, value, field: field.into() };
    let value = IntegerAttribute { text, field };
    let patches = if let Some(bit) = flag_bit(key) {
        let value = value.bounded("expected 0 or 1", "only 0 or 1 is supported", 0..=1)?;
        vec![patch(0, 1 << bit, value << bit)]
    } else {
        match key {
            "WritePluse" => {
                let value = value.bounded(
                    "expected a pulse count",
                    "pulse count outside the controller's range",
                    1..=i32::MAX.cast_unsigned(),
                )?;
                vec![patch(11, u32::MAX, value), patch(10, u32::MAX, value)]
            }
            "Acceleration" => {
                vec![patch(3, u32::MAX, value.acceleration()?)]
            }
            "NegativeLimitInput" | "ForwardLimitInput" | "OriginalInput" => {
                let value =
                    value.bounded("expected an input number", "input exceeds a byte", 0..=255)?;
                let shift = match key {
                    "NegativeLimitInput" => 0,
                    "ForwardLimitInput" => 8,
                    _ => 16,
                };
                vec![patch(12, 255 << shift, value << shift)]
            }
            "BrakeOutput" => {
                let value =
                    value.bounded("expected an output number", "output outside 0 to 26", 0..=26)?;
                vec![patch(13, u32::MAX, value)]
            }
            "FastSpeed" => vec![patch(5, u32::MAX, scaled_speed(text, field, scale, false)?)],
            "SecondSpeed" => vec![patch(6, u32::MAX, scaled_speed(text, field, scale, false)?)],
            "SpeedRatio" => vec![patch(9, u32::MAX, scaled_speed(text, field, scale, true)?)],
            _ => return Ok(None),
        }
    };
    Ok(Some(patches))
}

struct IntegerAttribute<'a> {
    text: &'a str,
    field: &'a str,
}

impl IntegerAttribute<'_> {
    fn bounded(
        &self,
        expected: &str,
        outside: &str,
        range: std::ops::RangeInclusive<u32>,
    ) -> Result<u32> {
        let value = self.text.parse::<u32>().map_err(|_| invalid(self.field, expected))?;
        if !range.contains(&value) {
            return Err(invalid(self.field, outside));
        }
        Ok(value)
    }

    fn acceleration(&self) -> Result<u32> {
        let value = self
            .text
            .parse::<f64>()
            .map_err(|_| invalid(self.field, "expected an acceleration"))?;
        if !value.is_finite() || value <= 0. || value > f64::from(i32::MAX) {
            return Err(invalid(self.field, "acceleration outside the controller's range"));
        }
        Ok(crate::whole(value.trunc()).unwrap_or(0))
    }
}

impl Plan {
    /// The plan for `document` under the controller's coordinate `scale`.
    pub fn from_document(document: &Document, scale: i32) -> Result<Self> {
        if scale <= 0 {
            return Err(Error::Unsupported("the controller scale must be positive".into()));
        }
        let mut plan = Self { scale, patches: vec![], host_only: vec![] };
        for path in document.paths() {
            let attributes = document.attributes(path)?;
            let axis = axis_of(path);
            for (key, text) in attributes {
                let field = format!("{path}/@{key}");
                let patches = match axis {
                    Some(axis) => attribute_patches(axis, key, text, &field, scale)?,
                    None => None,
                };
                match patches {
                    Some(patches) => plan.patches.extend(patches),
                    None => plan.host_only.push(field),
                }
            }
        }
        if plan.patches.is_empty() {
            return Err(Error::Unsupported(
                "the document holds no controller axis parameters".into(),
            ));
        }
        Ok(plan)
    }

    /// The axes the plan touches.
    #[must_use]
    pub fn axes(&self) -> Vec<u8> {
        let mut axes: Vec<_> = self.patches.iter().map(|p| p.axis).collect();
        axes.sort_unstable();
        axes.dedup();
        axes
    }

    /// The bank of `axis` as it should read: `current` with the patches
    /// applied.
    pub fn merge(&self, axis: u8, current: &[u32]) -> Result<Vec<u32>> {
        if current.len() != PARAMETER_BANK_WORDS {
            return Err(Error::Unsupported("an axis bank holds fourteen words".into()));
        }
        let mut result = current.to_vec();
        for patch in self.patches.iter().filter(|p| p.axis == axis) {
            if patch.word >= PARAMETER_BANK_WORDS || patch.value & !patch.mask != 0 {
                return Err(Error::Unsupported("invalid parameter patch".into()));
            }
            result[patch.word] = (result[patch.word] & !patch.mask) | patch.value;
        }
        Ok(result)
    }

    /// The fields whose bank words do not read back as planned.
    #[must_use]
    pub fn mismatches(&self, banks: &BTreeMap<u8, Vec<u32>>) -> Vec<String> {
        self.patches
            .iter()
            .filter(|p| {
                banks.get(&p.axis).and_then(|b| b.get(p.word)).is_none_or(|v| v & p.mask != p.value)
            })
            .map(|p| p.field.clone())
            .collect()
    }
}

/// The register address of an axis bank.
pub fn bank_address(axis: u8) -> Result<u32> {
    if axis < 5 {
        Ok(parameter_bank(axis))
    } else {
        Err(Error::Unsupported("the controller has five axis banks".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Kind;

    /// Flags pack into word 0 without touching the other bits, the pulse
    /// count lands in words 10 and 11, the origin input in the top byte of
    /// word 12, the fast speed scaled into word 5, and untouched words
    /// keep what the controller had.
    #[test]
    fn projection_preserves_unknown_words_and_rejects_overflow() {
        let doc = Document::parse(
            Kind::Hardware,
            br#"<ParameterRoot><PMachineAxisConfig_0><MAC AxisReverse="1" EncoderReverse="0" FastSpeed="80" OriginalInput="7" WritePluse="8000"/></PMachineAxisConfig_0></ParameterRoot>"#,
        )
        .unwrap();
        let plan = Plan::from_document(&doc, 1000).unwrap();
        let mut current = vec![0xdead_beef; 14];
        current[0] = 0xffff_ffff;
        let merged = plan.merge(0, &current).unwrap();
        assert_eq!(merged[0], 0xffff_fdff);
        assert_eq!(merged[5], 80_000);
        assert_eq!(merged[12], 0xde07_beef);
        assert_eq!(merged[10], 8000);
        assert_eq!(merged[11], 8000);
        assert_eq!(merged[4], 0xdead_beef);
        assert!(
            !plan
                .mismatches(&BTreeMap::from([(0, merged)]))
                .iter()
                .any(|f| f.ends_with("FastSpeed"))
        );
        assert!(Plan::from_document(&doc, i32::MAX).is_err());
        assert_eq!(bank_address(1).unwrap(), 50_240);
        assert!(bank_address(5).is_err());
    }

    /// The speed ratio scales and truncates as the vendor serialises it, and
    /// a bank that reads back one unit off is reported by field.
    #[test]
    fn speed_ratio_matches_the_vendor_serialisation() {
        for (ratio, expected) in [
            (0.001, 1),
            (1.234_567, 1234),
            (31.04, 31_040),
            (39.95, 39_950),
            (0.1, 100),
            (2_147.483_646, 2_147_483),
        ] {
            let xml = format!(
                "<ParameterRoot><PMachineAxisConfig_0><MAC WritePluse=\"8000\" SpeedRatio=\"{ratio}\"/></PMachineAxisConfig_0></ParameterRoot>"
            );
            let doc = Document::parse(Kind::Hardware, xml.as_bytes()).unwrap();
            let plan = Plan::from_document(&doc, 1000).unwrap();
            let mut bank = plan.merge(0, &[0; 14]).unwrap();
            assert_eq!(bank[9], expected);
            assert!(plan.mismatches(&BTreeMap::from([(0, bank.clone())])).is_empty());
            bank[9] += 1;
            assert_eq!(
                plan.mismatches(&BTreeMap::from([(0, bank)])),
                vec!["/ParameterRoot/PMachineAxisConfig_0/MAC/@SpeedRatio"]
            );
        }
        for ratio in ["0.0009", "0", "-1", "NaN", "inf", "999999999"] {
            let xml = format!(
                "<ParameterRoot><PMachineAxisConfig_0><MAC WritePluse=\"8000\" SpeedRatio=\"{ratio}\"/></PMachineAxisConfig_0></ParameterRoot>"
            );
            let result = Document::parse(Kind::Hardware, xml.as_bytes())
                .and_then(|doc| Plan::from_document(&doc, 1000));
            assert!(result.is_err(), "{ratio}");
        }
    }
}
