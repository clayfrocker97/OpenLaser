// SPDX-License-Identifier: GPL-3.0-or-later

//! Validated, content-addressed fonts kept alongside OpenLaser's library.

use crate::{Error, Result};
use openlaser_library::{atomic_write, sha256};
use openlaser_svg::{FontFace, Fonts};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Sources {
    version: u32,
    files: Vec<Source>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Source {
    name: String,
    sha256: String,
}

#[derive(Clone)]
pub(crate) struct Store {
    root: PathBuf,
    sources: Sources,
    pub fonts: Fonts,
}

impl Store {
    pub fn open(data: &Path) -> Result<Self> {
        let root = data.join("fonts");
        let bytes = match std::fs::read(root.join("index.json")) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Refused(format!("saved fonts: {e}"))),
        };
        let sources: Sources = bytes.map_or_else(
            || Ok(Sources { version: 1, files: vec![] }),
            |bytes| {
                serde_json::from_slice(&bytes)
                    .map_err(|e| Error::Refused(format!("saved fonts: {e}")))
            },
        )?;
        if sources.version != 1 {
            return Err(Error::Refused("unsupported saved font index".into()));
        }
        let mut fonts = Fonts::default();
        for source in &sources.files {
            if source.sha256.len() != 64 || !source.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(Error::Refused("invalid saved font identity".into()));
            }
            let bytes = std::fs::read(root.join(&source.sha256))
                .map_err(|e| Error::Refused(format!("saved font {}: {e}", source.name)))?;
            if sha256(&bytes) != source.sha256 {
                return Err(Error::Refused(format!("saved font {} is corrupt", source.name)));
            }
            fonts = fonts
                .with_font(&source.sha256, bytes)
                .map_err(|e| Error::Refused(format!("saved font {}: {e}", source.name)))?;
        }
        Ok(Self { root, sources, fonts })
    }

    /// The caller serializes font uploads. Publish the returned store only after
    /// the source and index are durable; failures preserve the active catalog.
    pub fn import(&self, name: &str, bytes: &[u8]) -> Result<(Self, Vec<FontFace>, bool)> {
        let name = name.rsplit(['/', '\\']).next().unwrap_or_default();
        let extension =
            Path::new(name).extension().and_then(|ext| ext.to_str()).unwrap_or_default();
        if name.len() > 255
            || !["ttf", "otf", "ttc", "otc"].iter().any(|ext| extension.eq_ignore_ascii_case(ext))
        {
            return Err(Error::Request("choose a .ttf, .otf, .ttc or .otc font file".into()));
        }
        let hash = sha256(bytes);
        let existing = self.sources.files.iter().any(|source| source.sha256 == hash);
        let mut next = self.clone();
        if !existing {
            next.fonts = self
                .fonts
                .with_font(&hash, bytes.to_vec())
                .map_err(|e| Error::Request(e.to_string()))?;
            next.sources.files.push(Source { name: name.into(), sha256: hash.clone() });
            let index = serde_json::to_vec_pretty(&next.sources)
                .map_err(|e| Error::Refused(e.to_string()))?;
            std::fs::create_dir_all(&self.root)
                .map_err(|e| Error::Refused(format!("save font: {e}")))?;
            atomic_write(&self.root.join(&hash), bytes)?;
            atomic_write(&self.root.join("index.json"), &index)?;
        }
        let prefix = format!("{hash}:");
        let imported = next
            .fonts
            .faces()
            .iter()
            .filter(|face| face.id.starts_with(&prefix))
            .cloned()
            .collect();
        Ok((next, imported, existing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ITALIC: &[u8] = include_bytes!("../../../fixtures/fonts/LobsterTwo-Italic.ttf");
    const BOLD: &[u8] = include_bytes!("../../../fixtures/fonts/LobsterTwo-Bold.ttf");

    #[test]
    fn imported_faces_survive_restart_and_duplicates_share_the_saved_source() {
        let root = std::env::temp_dir().join(format!(
            "openlaser-fonts-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let store = Store::open(&root).unwrap();
        assert!(store.import("bad.ttf", b"invalid").is_err());
        assert!(store.import("bad.woff", ITALIC).is_err());
        assert!(!root.exists());
        let (store, faces, existing) = store.import("C:\\Fonts\\Lobster.ttf", ITALIC).unwrap();
        assert!(!existing);
        assert_eq!(faces[0].family, "Lobster Two");
        let (store, duplicate, existing) = store.import("renamed.ttf", ITALIC).unwrap();
        assert!(existing);
        assert_eq!(faces[0].id, duplicate[0].id);
        assert_eq!(store.sources.files.len(), 1);
        let (store, _, _) = store.import("Bold.ttf", BOLD).unwrap();
        let loaded = Store::open(&root).unwrap();
        assert_eq!(loaded.fonts.faces().len(), 2);
        assert_eq!(loaded.fonts.face(&faces[0].id).unwrap().style, "italic");
        assert_eq!(
            serde_json::to_value(store.fonts.faces()).unwrap(),
            serde_json::to_value(loaded.fonts.faces()).unwrap()
        );
        std::fs::write(root.join("fonts").join(sha256(ITALIC)), b"corrupt").unwrap();
        assert!(Store::open(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_commit_does_not_publish_a_font_or_load_an_orphan_source() {
        let root = std::env::temp_dir().join(format!(
            "openlaser-fonts-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let (store, _, _) = Store::open(&root).unwrap().import("Italic.ttf", ITALIC).unwrap();
        let index = root.join("fonts/index.json");
        let prior = std::fs::read(&index).unwrap();
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();
        assert!(store.import("Bold.ttf", BOLD).is_err());
        assert_eq!(store.fonts.faces().len(), 1);
        std::fs::remove_dir(&index).unwrap();
        std::fs::write(&index, prior).unwrap();
        assert_eq!(Store::open(&root).unwrap().fonts.faces().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
