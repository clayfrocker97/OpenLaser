// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed reads of one parameter group, with the vendor's ranges.

use crate::document::{Attributes, Bundle};
use crate::{Error, Result};
use openlaser_compiler::settings::GasCalibration;

/// One parameter group, read with the vendor's ranges. Errors name an
/// attribute `name.tag.key`, or `name.key` when the tag is blank.
#[derive(Clone, Copy)]
pub(crate) struct Group<'a> {
    attributes: &'a Attributes,
    defaults: Option<&'a Attributes>,
    name: &'a str,
    tag: &'a str,
}

impl<'a> Group<'a> {
    /// Read a required group, retaining its name in every validation error.
    pub(crate) fn read(bundle: &'a Bundle, name: &'a str, tag: &'a str) -> Result<Self> {
        Ok(Self::new(bundle.group(name, tag)?, name, tag))
    }

    pub(crate) const fn new(attributes: &'a Attributes, name: &'a str, tag: &'a str) -> Self {
        Self { attributes, defaults: None, name, tag }
    }

    /// Fall back to the machine bank for fields absent from a recipe.
    pub(crate) const fn with_defaults(mut self, defaults: Option<&'a Attributes>) -> Self {
        self.defaults = defaults;
        self
    }

    /// The attributes by name.
    pub(crate) const fn attributes(&self) -> &'a Attributes {
        self.attributes
    }

    /// Whether `key` is present.
    pub(crate) fn has(&self, key: &str) -> bool {
        self.text(key).is_some()
    }

    /// The raw text of `key`.
    pub(crate) fn text(&self, key: &str) -> Option<&'a str> {
        self.attributes.get(key).or_else(|| self.defaults?.get(key)).map(String::as_str)
    }

    /// How errors name the group.
    pub(crate) fn label(&self) -> String {
        if self.tag.is_empty() {
            self.name.to_owned()
        } else {
            format!("{}.{}", self.name, self.tag)
        }
    }

    /// How errors name `key`.
    fn field(&self, key: &str) -> String {
        if self.tag.is_empty() {
            format!("{}.{key}", self.name)
        } else {
            format!("{}.{}.{key}", self.name, self.tag)
        }
    }

    /// An error for a value of `key` the vendor's rules do not accept.
    pub(crate) fn invalid(&self, key: &str, reason: impl Into<String>) -> Error {
        Error::Invalid { field: self.field(key), reason: reason.into() }
    }

    /// A finite number.
    pub(crate) fn number(&self, key: &str) -> Result<f64> {
        let value = self
            .text(key)
            .ok_or_else(|| Error::Missing(self.field(key)))?
            .parse::<f64>()
            .map_err(|_| self.invalid(key, "not a number"))?;
        if !value.is_finite() {
            return Err(self.invalid(key, "not finite"));
        }
        Ok(value)
    }

    /// A number within `min..=max`.
    pub(crate) fn range(&self, key: &str, min: f64, max: f64) -> Result<f64> {
        let value = self.number(key)?;
        if !(min..=max).contains(&value) {
            return Err(self.invalid(key, format!("must be between {min} and {max}")));
        }
        Ok(value)
    }

    /// A whole number within `0..=max`.
    pub(crate) fn uint(&self, key: &str, max: u32) -> Result<u32> {
        let value = self.range(key, 0., f64::from(max))?;
        crate::whole(value).ok_or_else(|| self.invalid(key, "must be a whole number"))
    }

    /// A whole number within `0..=max`, zero when absent.
    pub(crate) fn optional_uint(&self, key: &str, max: u32) -> Result<u32> {
        if self.has(key) { self.uint(key, max) } else { Ok(0) }
    }

    /// A whole number within `0..=max`, as a byte.
    pub(crate) fn byte(&self, key: &str, max: u8) -> Result<u8> {
        let value = self.uint(key, u32::from(max))?;
        u8::try_from(value).map_err(|_| self.invalid(key, "exceeds a byte"))
    }

    /// A fixed wiring vector, preserving the caller's port order.
    pub(crate) fn bytes<const N: usize>(&self, keys: [&str; N], max: u8) -> Result<[u8; N]> {
        let mut values = [0; N];
        for (value, key) in values.iter_mut().zip(keys) {
            *value = self.byte(key, max)?;
        }
        Ok(values)
    }

    /// A flag: `1` is on, `0` or absence is off.
    pub(crate) fn enabled(&self, key: &str) -> Result<bool> {
        Ok(self.optional_uint(key, 1)? == 1)
    }

    /// A one-based port or channel number, `None` when the vendor's zero
    /// says unassigned.
    pub(crate) fn port(&self, key: &str, max: u32) -> Result<Option<u8>> {
        let value = self.uint(key, max)?;
        Ok(if value == 0 { None } else { u8::try_from(value).ok() })
    }

    /// A percentage that must be whole, as a byte.
    pub(crate) fn percent(&self, key: &str) -> Result<u8> {
        let value = self.range(key, 0., 100.)?;
        crate::whole(value)
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| self.invalid(key, "must be a whole percentage"))
    }

    /// A frequency in hertz: whole, nonzero, at most 65 535.
    pub(crate) fn frequency(&self, key: &str) -> Result<u16> {
        let value = self.uint(key, 65_535)?;
        if value == 0 {
            return Err(self.invalid(key, "must be positive"));
        }
        u16::try_from(value).map_err(|_| self.invalid(key, "exceeds 65535"))
    }

    /// A speed-adjustment curve: comma-separated `(speed percent, output
    /// percent)` pairs from 0 to 100, when its switch is on. The vendor's
    /// graph smoothing selectors are display metadata: MainApp 0x00612410
    /// exports the original nodes for every style, and CAD 0x10104880
    /// interpolates them linearly.
    #[allow(clippy::float_cmp, reason = "the span ends are the vendor's exact percentages")]
    pub(crate) fn curve(&self, switch: &str, nodes: &str) -> Result<Option<Vec<(f64, f64)>>> {
        if !self.enabled(switch)? {
            return Ok(None);
        }
        let values = self
            .text(nodes)
            .ok_or_else(|| Error::Missing(self.field(nodes)))?
            .split(',')
            .map(|v| v.trim().parse::<f64>().map_err(|_| self.invalid(nodes, "not a number")))
            .collect::<Result<Vec<_>>>()?;
        if values.len() < 4
            || values.len() > 256
            || !values.len().is_multiple_of(2)
            || values.iter().any(|v| !v.is_finite() || !(0. ..=100.).contains(v))
        {
            return Err(self.invalid(nodes, "needs 2 to 128 pairs of percentages"));
        }
        let points: Vec<_> = values.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        if points[0].0 != 0.
            || points[points.len() - 1].0 != 100.
            || points.windows(2).any(|p| p[0].0 >= p[1].0)
        {
            return Err(self.invalid(nodes, "must run from speed 0 to 100 percent, increasing"));
        }
        Ok(Some(points))
    }
}

/// A gas calibration: comma-separated `voltage,pressure` pairs, `None` when
/// the text is blank.
pub(crate) fn gas_calibration(
    text: &str,
    group: &str,
    key: &str,
) -> Result<Option<GasCalibration>> {
    let invalid = |reason: &str| Error::Invalid {
        field: format!("{group}.{key}"),
        reason: reason.to_owned(),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    if text.len() > 65_536 {
        return Err(invalid("exceeds the size limit"));
    }
    let fields = text
        .trim()
        .trim_end_matches(',')
        .split(',')
        .map(|v| v.trim().parse::<f64>().map_err(|_| invalid("not a number")))
        .collect::<Result<Vec<_>>>()?;
    if fields.is_empty() || !fields.len().is_multiple_of(2) || fields.len() > 4096 {
        return Err(invalid("needs voltage,pressure pairs"));
    }
    let points = fields.chunks_exact(2).map(|pair| (pair[0], pair[1])).collect();
    GasCalibration::new(points).map(Some).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> Attributes {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    /// Ranges, whole numbers, flags and ports read as the vendor means
    /// them: absent flags are off, a zero port is unassigned.
    #[test]
    fn typed_reads_follow_the_vendor_conventions() {
        let attributes =
            attrs(&[("Speed", "12.5"), ("Count", "3"), ("On", "1"), ("Port", "0"), ("Other", "7")]);
        let a = Group::new(&attributes, "G", "T");
        assert_eq!(a.range("Speed", 0., 100.).unwrap(), 12.5);
        assert!(a.range("Speed", 0., 10.).is_err());
        assert_eq!(a.uint("Count", 10).unwrap(), 3);
        assert!(a.uint("Speed", 100).is_err());
        assert_eq!(a.byte("Count", 5).unwrap(), 3);
        assert!(a.enabled("On").unwrap());
        assert!(!a.enabled("Off").unwrap());
        assert_eq!(a.port("Port", 26).unwrap(), None);
        assert_eq!(a.port("Other", 26).unwrap(), Some(7));
        assert!(
            matches!(a.number("Missing"), Err(Error::Missing(field)) if field == "G.T.Missing")
        );
        assert!(
            matches!(a.uint("Speed", 100), Err(Error::Invalid { field, .. }) if field == "G.T.Speed")
        );
        assert!(
            matches!(Group::new(&attributes, "G", "").number("Missing"), Err(Error::Missing(field)) if field == "G.Missing")
        );
    }

    /// A curve needs its switch, whole percentages, and ends at 0 and 100.
    #[test]
    fn curves_need_their_switch_and_a_full_span() {
        let attributes =
            attrs(&[("Adjust", "1"), ("Nodes", "0,10,50,40,100,100"), ("Smooth", "0")]);
        let a = Group::new(&attributes, "G", "");
        assert_eq!(
            a.curve("Adjust", "Nodes").unwrap(),
            Some(vec![(0., 10.), (50., 40.), (100., 100.)])
        );
        let off = attrs(&[("Adjust", "0")]);
        assert_eq!(Group::new(&off, "G", "").curve("Adjust", "Nodes").unwrap(), None);
        let short = attrs(&[("Adjust", "1"), ("Nodes", "0,10,90,100")]);
        assert!(Group::new(&short, "G", "").curve("Adjust", "Nodes").is_err());
    }

    /// Gas calibration text is voltage then pressure per pair; blank text
    /// means no calibration and malformed text is refused.
    #[test]
    fn gas_calibration_pairs_are_voltage_then_pressure() {
        let map = gas_calibration("8,12,2,3,5,9,", "DO", "Map").unwrap().unwrap();
        assert!((map.voltage(3.).unwrap() - 2.).abs() < 1e-12);
        assert_eq!(gas_calibration(" \t\n", "DO", "Map").unwrap(), None);
        for text in ["1", "1,2,3", "one,two", "1,,2", ","] {
            assert!(gas_calibration(text, "DO", "Map").is_err(), "{text:?}");
        }
    }
}
