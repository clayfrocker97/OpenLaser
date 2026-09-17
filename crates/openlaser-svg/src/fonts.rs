// SPDX-License-Identifier: GPL-3.0-or-later

//! Portable bundled fonts plus explicitly supplied font bytes; no file discovery.

use crate::{Error, Result};
use read_fonts::{FileRef, FontRef, TableProvider, types::Tag};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};
use usvg::fontdb::{self, Database, FaceInfo};

/// A selectable face from an imported font file.
#[derive(Clone, Debug, Serialize)]
pub struct FontFace {
    /// Stable file identity and face index, independent of database insertion order.
    pub id: String,
    /// Family name used by SVG font matching.
    pub family: String,
    /// The font's PostScript face name, including its style.
    pub name: String,
    /// Actual face weight, normally 400 or 700.
    pub weight: u16,
    /// CSS face style: normal, italic or oblique.
    pub style: String,
    /// CSS face width, such as normal or condensed.
    pub stretch: String,
}

/// An inexpensive, immutable snapshot shared by import and text outlining.
#[derive(Clone)]
pub struct Fonts {
    database: Arc<Database>,
    faces: Arc<Vec<FontFace>>,
}

impl Default for Fonts {
    fn default() -> Self {
        static BUNDLED: OnceLock<Arc<Database>> = OnceLock::new();
        let database = Arc::clone(BUNDLED.get_or_init(|| {
            let mut fonts = Database::new();
            for bytes in [
                include_bytes!("../fonts/NotoSans-Regular.ttf").as_slice(),
                include_bytes!("../fonts/NotoSans-Bold.ttf").as_slice(),
                include_bytes!("../fonts/NotoSerif-Regular.ttf").as_slice(),
                include_bytes!("../fonts/NotoSerif-Bold.ttf").as_slice(),
                include_bytes!("../fonts/NotoSansMono-Regular.ttf").as_slice(),
                include_bytes!("../fonts/NotoSansMono-Bold.ttf").as_slice(),
            ] {
                fonts.load_font_data(bytes.to_vec());
            }
            fonts.set_sans_serif_family("Noto Sans");
            fonts.set_serif_family("Noto Serif");
            fonts.set_monospace_family("Noto Sans Mono");
            fonts.set_cursive_family("Noto Sans");
            fonts.set_fantasy_family("Noto Sans");
            Arc::new(fonts)
        }));
        Self { database, faces: Arc::default() }
    }
}

impl Fonts {
    /// Imported faces only; bundled family choices are always available.
    #[must_use]
    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    /// Finds an explicitly selected imported face; unknown IDs must not fall back.
    pub fn face(&self, id: &str) -> Result<&FontFace> {
        self.faces
            .iter()
            .find(|face| face.id == id)
            .ok_or_else(|| Error("the selected font is unavailable; import it again".into()))
    }

    /// Validates all faces before adding an immutable font-file snapshot.
    /// `key` identifies the source bytes, normally their SHA-256 digest.
    pub fn with_font(&self, key: &str, bytes: Vec<u8>) -> Result<Self> {
        let count = validate(&bytes)?;
        let mut database = (*self.database).clone();
        let ids = database.load_font_source(fontdb::Source::Binary(Arc::new(bytes)));
        if ids.len() != count {
            return Err(Error("the font has an unreadable face or missing family name".into()));
        }
        let mut faces = (*self.faces).clone();
        for id in &ids {
            let face = database.face(*id).ok_or_else(|| Error("unreadable font face".into()))?;
            let duplicate =
                database.faces().find(|other| other.id != *id && same_selection(face, other));
            let already_loaded = if let Some(other) = duplicate {
                let same_bytes = database
                    .with_face_data(*id, |bytes, index| {
                        database
                            .with_face_data(other.id, |prior, prior_index| {
                                index == prior_index && bytes == prior
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !same_bytes {
                    return Err(Error(format!(
                        "a different {} font with the same style is already available; use a file with a distinct family or style name",
                        face.families[0].0
                    )));
                }
                true
            } else {
                false
            };
            let view = FontFace {
                id: format!("{key}:{}", face.index),
                family: face.families[0].0.clone(),
                name: face.post_script_name.clone(),
                weight: face.weight.0,
                style: style_name(face.style).into(),
                stretch: stretch_name(face.stretch).into(),
            };
            if !faces.iter().any(|prior| prior.id == view.id) {
                faces.push(view);
            }
            if already_loaded {
                database.remove_face(*id);
            }
        }
        faces.sort_by(|a, b| a.family.cmp(&b.family).then(a.name.cmp(&b.name)));
        Ok(Self { database: Arc::new(database), faces: Arc::new(faces) })
    }

    pub(crate) fn options<'a>(&self, warnings: &'a Mutex<BTreeSet<String>>) -> usvg::Options<'a> {
        let select = usvg::FontResolver::default_font_selector();
        let fallback = usvg::FontResolver::default_fallback_selector();
        usvg::Options {
            font_family: "Noto Sans".into(),
            fontdb: Arc::clone(&self.database),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            font_resolver: usvg::FontResolver {
                select_font: Box::new(move |font, fonts| {
                    let id = select(font, fonts)?;
                    let face = fonts.face(id)?;
                    report_substitution(font, face, fonts, warnings);
                    Some(id)
                }),
                select_fallback: Box::new(move |character, excluded, fonts| {
                    let id = fallback(character, excluded, fonts)?;
                    let face = fonts.face(id)?;
                    warn(
                        warnings,
                        format!(
                            "Character {character} uses {} as a fallback. Check the text before cutting.",
                            face.families[0].0
                        ),
                    );
                    Some(id)
                }),
            },
            ..usvg::Options::default()
        }
    }
}

fn validate(bytes: &[u8]) -> Result<usize> {
    if bytes.len() > crate::MAX_BYTES {
        return Err(Error("the font exceeds 64 MiB".into()));
    }
    let file = FileRef::new(bytes)
        .map_err(|_| Error("choose a valid TTF or OTF font, or TTC/OTC font collection".into()))?;
    if matches!(&file, FileRef::Collection(collection) if collection.len() > 128) {
        return Err(Error("font collections may contain at most 128 faces".into()));
    }
    let mut count = 0;
    for font in file.fonts() {
        let font = font.map_err(|e| Error(format!("invalid font: {e}")))?;
        if ![b"glyf", b"CFF ", b"CFF2"].iter().any(|tag| font.table_data(Tag::new(tag)).is_some()) {
            return Err(Error(
                "the font has no vector outlines; choose an outline TTF or OTF font".into(),
            ));
        }
        count += 1;
    }
    if count == 0 {
        return Err(Error("the font collection is empty".into()));
    }
    Ok(count)
}

fn same_selection(a: &FaceInfo, b: &FaceInfo) -> bool {
    a.style == b.style
        && a.weight == b.weight
        && a.stretch == b.stretch
        && a.families.iter().any(|a| b.families.iter().any(|b| a.0.eq_ignore_ascii_case(&b.0)))
}

fn style_name(style: fontdb::Style) -> &'static str {
    match style {
        fontdb::Style::Normal => "normal",
        fontdb::Style::Italic => "italic",
        fontdb::Style::Oblique => "oblique",
    }
}

fn stretch_name(stretch: fontdb::Stretch) -> &'static str {
    [
        "ultra-condensed",
        "extra-condensed",
        "condensed",
        "semi-condensed",
        "normal",
        "semi-expanded",
        "expanded",
        "extra-expanded",
        "ultra-expanded",
    ][usize::from(stretch.to_number() - 1)]
}

fn axis_supports(fonts: &Database, face: &FaceInfo, tag: [u8; 4], value: f64) -> bool {
    fonts
        .with_face_data(face.id, |bytes, index| {
            let Ok(font) = FontRef::from_index(bytes, index) else {
                return false;
            };
            let Ok(axes) = font.fvar().and_then(|table| table.axes()) else {
                return false;
            };
            axes.iter().any(|axis| {
                axis.axis_tag() == Tag::new(&tag)
                    && (axis.min_value().to_f64()..=axis.max_value().to_f64()).contains(&value)
            })
        })
        .unwrap_or(false)
}

fn report_substitution(
    font: &usvg::Font,
    face: &FaceInfo,
    fonts: &Database,
    warnings: &Mutex<BTreeSet<String>>,
) {
    let matched = font.families().iter().any(|family| match family {
        usvg::FontFamily::Named(name) => {
            face.families.iter().any(|f| f.0.eq_ignore_ascii_case(name))
        }
        _ => true,
    });
    if !matched {
        let requested =
            font.families().iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        warn(
            warnings,
            format!(
                "Font {requested} is unavailable; used {}. Check the text before cutting.",
                face.families[0].0
            ),
        );
    }
    let style = fontdb::Style::from(font.style());
    let style_matches = style == face.style
        || match style {
            fontdb::Style::Italic => axis_supports(fonts, face, *b"ital", 1.),
            fontdb::Style::Oblique => axis_supports(fonts, face, *b"slnt", -12.),
            fontdb::Style::Normal => false,
        };
    let stretch = fontdb::Stretch::from(font.stretch());
    let width =
        [50., 62.5, 75., 87.5, 100., 112.5, 125., 150., 200.][usize::from(stretch.to_number() - 1)];
    if !style_matches
        || (stretch != face.stretch && !axis_supports(fonts, face, *b"wdth", width))
        || (font.weight() != face.weight.0
            && !axis_supports(fonts, face, *b"wght", f64::from(font.weight())))
    {
        let actual =
            if face.style == fontdb::Style::Normal { "upright" } else { style_name(face.style) };
        warn(
            warnings,
            format!(
                "Font style {}, stretch {}, weight {} was replaced by an {actual} font of stretch {} and weight {}. Check the text before cutting.",
                style_name(style),
                stretch_name(stretch),
                font.weight(),
                stretch_name(face.stretch),
                face.weight.0
            ),
        );
    }
}

fn warn(warnings: &Mutex<BTreeSet<String>>, message: String) {
    warnings.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(message);
}
