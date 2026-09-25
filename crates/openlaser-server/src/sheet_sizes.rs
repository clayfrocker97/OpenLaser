// SPDX-License-Identifier: GPL-3.0-or-later

//! Sheet sizes the operator saved for the nesting stock chooser. They belong
//! to the machine, so every screen offers the same list.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "sheet-sizes.json";
/// Enough for a shop's stock list without turning the chooser into a scroll.
const MAX_SIZES: usize = 50;

/// One sheet, width along X by height along Y, in millimetres.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetSize {
    /// Along X.
    pub width_mm: f64,
    /// Along Y.
    pub height_mm: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    sizes: Vec<SheetSize>,
}

/// Every size positive and finite, at most `MAX_SIZES` of them, none repeated.
pub fn validate(sizes: &[SheetSize]) -> Result<()> {
    if sizes.len() > MAX_SIZES {
        return Err(Error::Request(format!("keep at most {MAX_SIZES} sheet sizes")));
    }
    for (i, size) in sizes.iter().enumerate() {
        crate::inventory::check_sides(size.width_mm, size.height_mm)?;
        if sizes[..i].contains(size) {
            return Err(Error::Request("that sheet size is already saved".into()));
        }
    }
    Ok(())
}

/// The saved sizes; a missing or unreadable file means none, and an unreadable
/// one is logged.
#[must_use]
pub fn open(root: &Path) -> Vec<SheetSize> {
    let path = root.join(FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "sheet sizes unreadable");
            return Vec::new();
        }
    };
    let loaded =
        serde_json::from_slice::<Saved>(&bytes).map_err(|e| e.to_string()).and_then(|saved| {
            if saved.version != 1 {
                return Err(format!("unsupported version {}", saved.version));
            }
            validate(&saved.sizes).map_err(|e| e.to_string()).map(|()| saved.sizes)
        });
    loaded.unwrap_or_else(|error| {
        tracing::warn!(%error, path = %path.display(), "sheet sizes invalid");
        Vec::new()
    })
}

fn save(root: &Path, sizes: &[SheetSize]) -> Result<()> {
    validate(sizes)?;
    let bytes = serde_json::to_vec_pretty(&Saved { version: 1, sizes: sizes.to_vec() })
        .map_err(|e| Error::Refused(e.to_string()))?;
    openlaser_library::atomic_write(&root.join(FILE), &bytes).map_err(Error::from)
}

impl crate::Coordinator {
    /// Replaces the saved sheet sizes for every screen, if nobody changed
    /// them since the caller read `expected`.
    pub fn save_sheet_sizes(&mut self, next: Vec<SheetSize>, expected: &[SheetSize]) -> Result<()> {
        validate(&next)?;
        if expected != self.sheet_sizes.as_slice() {
            return Err(Error::Refused(
                "the saved sheet sizes changed on another screen; try again".into(),
            ));
        }
        save(&self.config.data_dir, &next)?;
        self.sheet_sizes = next;
        self.publish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openlaser-sheets-{name}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SIZE: SheetSize = SheetSize { width_mm: 1250., height_mm: 2500. };

    #[test]
    fn sizes_round_trip_and_bad_files_mean_none() {
        let root = scratch("round-trip");
        assert!(open(&root).is_empty());
        save(&root, &[SIZE]).unwrap();
        assert_eq!(open(&root), vec![SIZE]);
        for damaged in [
            &b"{"[..],
            br#"{"version":2,"sizes":[]}"#,
            br#"{"version":1,"sizes":[{"width_mm":-1,"height_mm":5}]}"#,
        ] {
            std::fs::write(root.join(FILE), damaged).unwrap();
            assert!(open(&root).is_empty());
        }
    }

    #[test]
    fn repeats_and_impossible_sizes_are_refused() {
        assert!(validate(&[SIZE, SIZE]).is_err());
        assert!(validate(&[SheetSize { width_mm: 0., height_mm: 10. }]).is_err());
        assert!(validate(&[SheetSize { width_mm: f64::NAN, height_mm: 10. }]).is_err());
        assert!(validate(&[SheetSize { width_mm: 25_000., height_mm: 10. }]).is_err());
        let many: Vec<SheetSize> =
            (1..=51).map(|i| SheetSize { width_mm: f64::from(i), height_mm: 10. }).collect();
        assert!(validate(&many[..50]).is_ok());
        assert!(validate(&many).is_err());
    }
}
