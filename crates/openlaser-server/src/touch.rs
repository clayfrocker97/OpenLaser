// SPDX-License-Identifier: GPL-3.0-or-later

//! How long a held control must be held before it acts (ui/DESIGN.md, Touch
//! rules). One setting for every screen, so the machine answers the same
//! deliberate press wherever it is operated from.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The shortest hold an operator may set: longer than any deliberate tap.
pub const MIN_HOLD_MS: u32 = 300;
/// The longest hold an operator may set.
pub const MAX_HOLD_MS: u32 = 3000;

const FILE: &str = "touch.json";

/// Hold durations in milliseconds.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldTimes {
    /// Moving the machine, firing the beam or switching an output on.
    pub move_ms: u32,
    /// Setting an origin or a reference.
    pub zero_ms: u32,
}

impl Default for HoldTimes {
    fn default() -> Self {
        Self { move_ms: 1000, zero_ms: 750 }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    move_ms: u32,
    zero_ms: u32,
}

impl HoldTimes {
    /// Both holds within [`MIN_HOLD_MS`] to [`MAX_HOLD_MS`].
    pub fn validate(&self) -> Result<()> {
        for (name, ms) in [("moving or firing", self.move_ms), ("setting an origin", self.zero_ms)]
        {
            if !(MIN_HOLD_MS..=MAX_HOLD_MS).contains(&ms) {
                return Err(Error::Request(format!(
                    "the hold for {name} must be {} to {} seconds",
                    f64::from(MIN_HOLD_MS) / 1000.,
                    f64::from(MAX_HOLD_MS) / 1000.
                )));
            }
        }
        Ok(())
    }

    /// The saved holds. A missing file means the defaults; so does an
    /// unreadable one, since they are the safe values, and that is logged.
    #[must_use]
    pub fn open(root: &Path) -> Self {
        let path = root.join(FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "hold times unreadable; using defaults");
                return Self::default();
            }
        };
        let loaded =
            serde_json::from_slice::<Saved>(&bytes).map_err(|e| e.to_string()).and_then(|saved| {
                let times = Self { move_ms: saved.move_ms, zero_ms: saved.zero_ms };
                if saved.version != 1 {
                    return Err(format!("unsupported version {}", saved.version));
                }
                times.validate().map_err(|e| e.to_string()).map(|()| times)
            });
        loaded.unwrap_or_else(|error| {
            tracing::warn!(%error, path = %path.display(), "hold times invalid; using defaults");
            Self::default()
        })
    }

    /// Writes the holds, which take effect only once they are on disk.
    pub fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        let saved = Saved { version: 1, move_ms: self.move_ms, zero_ms: self.zero_ms };
        let bytes = serde_json::to_vec_pretty(&saved).map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&root.join(FILE), &bytes).map_err(Error::from)
    }
}

impl crate::Coordinator {
    /// Replaces the hold times for every screen, if nobody changed them since
    /// the caller read `expected`.
    pub fn save_hold_times(&mut self, next: HoldTimes, expected: HoldTimes) -> Result<()> {
        next.validate()?;
        if expected != self.hold {
            return Err(Error::Refused(
                "the hold times changed on another screen; review them again".into(),
            ));
        }
        next.save(&self.config.data_dir)?;
        self.hold = next;
        self.publish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openlaser-touch-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn holds_round_trip_and_bad_files_fall_back_to_the_defaults() {
        let root = scratch("round-trip");
        assert_eq!(HoldTimes::open(&root), HoldTimes::default());
        let longer = HoldTimes { move_ms: 1500, zero_ms: 1000 };
        longer.save(&root).unwrap();
        assert_eq!(HoldTimes::open(&root), longer);
        for damaged in [
            &b"{"[..],
            br#"{"version":2,"move_ms":1500,"zero_ms":1000}"#,
            br#"{"version":1,"move_ms":10,"zero_ms":1000}"#,
        ] {
            std::fs::write(root.join(FILE), damaged).unwrap();
            assert_eq!(HoldTimes::open(&root), HoldTimes::default());
        }
    }

    #[test]
    fn a_tap_can_never_become_a_hold() {
        let root = scratch("range");
        for bad in
            [HoldTimes { move_ms: 299, zero_ms: 750 }, HoldTimes { move_ms: 1000, zero_ms: 3001 }]
        {
            assert!(bad.validate().is_err());
            assert!(bad.save(&root).is_err());
        }
        assert!(HoldTimes { move_ms: MIN_HOLD_MS, zero_ms: MAX_HOLD_MS }.validate().is_ok());
    }
}
