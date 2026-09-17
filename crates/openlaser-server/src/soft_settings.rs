// SPDX-License-Identifier: GPL-3.0-or-later

//! The three supported host process settings, preserving their imported source.

use crate::{Error, Result};
use openlaser_compiler::settings::Timeouts;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A compact projection of the imported Soft INI.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct SoftView {
    /// Original filename, if imported.
    pub name: Option<String>,
    /// Hash of the exact source bytes.
    pub sha256: Option<String>,
    /// Follow/retract timeout in milliseconds.
    pub follow_ms: u32,
    /// Pierce-height timeout in milliseconds.
    pub section_drill_ms: u32,
    /// Minimum nonzero analog peak request.
    pub analog_minimum: u32,
}

impl Default for SoftView {
    fn default() -> Self {
        SoftSettings::default().view()
    }
}

/// Source and validated projection; the projection is always rebuilt on load.
#[derive(Clone, Debug, Default)]
pub struct SoftSettings {
    source: Option<Source>,
    /// The values used by both the cutting and film binders.
    pub timeouts: Timeouts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    version: u32,
    name: String,
    sha256: String,
    bytes: Vec<u8>,
}

impl SoftSettings {
    /// Parses a UTF-8 or UTF-16LE host INI without acting on any other settings.
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(Error::Request("the Soft INI exceeds 1 MB".into()));
        }
        let name = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.len() <= 255 && n.to_ascii_lowercase().ends_with(".ini"))
            .ok_or_else(|| Error::Request("choose the host's .ini file".into()))?;
        let text = decode_text(bytes)?;
        let timeouts = SoftSection::read(&text)?;
        Ok(Self {
            source: Some(Source {
                version: 1,
                name: name.into(),
                sha256: openlaser_library::sha256(bytes),
                bytes: bytes.to_vec(),
            }),
            timeouts,
        })
    }

    /// Loads the managed import, refusing corruption rather than using defaults.
    pub fn open(root: &Path) -> Result<Self> {
        let bytes = match std::fs::read(root.join("soft-settings.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(Error::Refused(format!("Soft settings: {error}"))),
        };
        let source: Source = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Refused(format!("Soft settings: {e}")))?;
        if source.version != 1 || openlaser_library::sha256(&source.bytes) != source.sha256 {
            return Err(Error::Refused(
                "the saved Soft settings are unsupported or corrupt".into(),
            ));
        }
        Self::parse(&source.name, &source.bytes)
    }

    /// Commits the validated source before it becomes active in memory.
    pub fn save(&self, root: &Path) -> Result<()> {
        let source =
            self.source.as_ref().ok_or_else(|| Error::Request("no Soft INI imported".into()))?;
        let bytes = serde_json::to_vec(source).map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&root.join("soft-settings.json"), &bytes)
            .map_err(Error::from)
    }

    /// Original source for a download.
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        self.source.as_ref().map(|s| s.bytes.as_slice())
    }

    /// Small settings-page projection.
    #[must_use]
    pub fn view(&self) -> SoftView {
        SoftView {
            name: self.source.as_ref().map(|s| s.name.clone()),
            sha256: self.source.as_ref().map(|s| s.sha256.clone()),
            follow_ms: self.timeouts.follow_ms,
            section_drill_ms: self.timeouts.section_drill_ms,
            analog_minimum: self.timeouts.analog_minimum,
        }
    }
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    let invalid = || Error::Request("the Soft INI must be UTF-8 or UTF-16LE text".into());
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(invalid());
        }
        let words: Vec<_> =
            bytes[2..].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        String::from_utf16(&words).map_err(|_| invalid())?
    } else {
        std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
            .map_err(|_| invalid())?
            .to_owned()
    };
    if text.contains('\0') {
        return Err(invalid());
    }
    Ok(text)
}

#[derive(Default)]
struct SoftSection {
    inside: bool,
    found: bool,
    values: [Option<u32>; 3],
}

impl SoftSection {
    fn read(text: &str) -> Result<Timeouts> {
        let mut section = Self::default();
        for (number, line) in text.lines().enumerate() {
            section.line(number, line)?;
        }
        if !section.found {
            return Err(Error::Request("the INI has no [Soft] section".into()));
        }
        let defaults = Timeouts::default();
        Ok(Timeouts {
            follow_ms: section.values[0].unwrap_or(defaults.follow_ms),
            section_drill_ms: section.values[1].unwrap_or(defaults.section_drill_ms),
            analog_minimum: section.values[2].unwrap_or(defaults.analog_minimum),
        })
    }

    fn line(&mut self, number: usize, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() || line.starts_with([';', '#']) {
            return Ok(());
        }
        if line.starts_with('[') {
            let section =
                line.strip_prefix('[').and_then(|s| s.strip_suffix(']')).ok_or_else(|| {
                    Error::Request(format!("invalid INI section at line {}", number + 1))
                })?;
            self.inside = section.trim().eq_ignore_ascii_case("Soft");
            self.found |= self.inside;
            return Ok(());
        }
        if !self.inside {
            return Ok(());
        }
        self.setting(number, line)
    }

    fn setting(&mut self, number: usize, line: &str) -> Result<()> {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            Error::Request(format!("invalid Soft setting at line {}", number + 1))
        })?;
        let key = key.trim();
        let Some(index) = ["FollowOvertime", "SectionDrillOvertime", "DAMinVal"]
            .iter()
            .position(|known| key.eq_ignore_ascii_case(known))
        else {
            return Ok(());
        };
        let value = value
            .trim()
            .parse::<u32>()
            .map_err(|_| Error::Request(format!("{key} must be an unsigned integer")))?;
        if (index < 2 && !(1..=2_146_883_647).contains(&value)) || (index == 2 && value > 10_000) {
            return Err(Error::Request(format!("{key} is outside its supported range")));
        }
        if self.values[index].is_some_and(|previous| previous != value) {
            return Err(Error::Request(format!("conflicting duplicate {key} values")));
        }
        self.values[index] = Some(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_encodings_defaults_and_duplicate_handling() {
        let text = "[Other]\nFollowOvertime=1\n[sOfT]\nfollowovertime=8000\nFollowOvertime=8000\nUnrelated=keep\n";
        let mut utf16 = vec![0xff, 0xfe];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        let parsed = SoftSettings::parse("ipAdd.ini", &utf16).unwrap();
        assert_eq!(parsed.bytes().unwrap(), utf16);
        assert_eq!(parsed.timeouts, Timeouts { follow_ms: 8000, ..Timeouts::default() });
        assert_eq!(parsed.view().sha256.unwrap(), openlaser_library::sha256(&utf16));
        for bad in [
            "[Other]\nFollowOvertime=8",
            "[Soft]\nFollowOvertime=8000\nfollowovertime=9000",
            "[Soft]\nDAMinVal=10001",
            "[Soft]\nSectionDrillOvertime=2147483647",
            "[Soft]\nFollowOvertime=-1",
            "[Soft]\nDAMinVal=1.5",
        ] {
            assert!(SoftSettings::parse("ipAdd.ini", bad.as_bytes()).is_err(), "{bad}");
        }
        utf16.pop();
        assert!(SoftSettings::parse("ipAdd.ini", &utf16).is_err());
    }
}
