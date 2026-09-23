// SPDX-License-Identifier: GPL-3.0-or-later

//! The head setup a vendor recipe asks for, read from its note and names.
//!
//! M-Laser has no fields for the nozzle, the manual focus or the lens, so
//! operators write them into the bank's note, the layer name and the file
//! name, each in their own spelling: `NOZZLE---SINGLE-2.0`, `1.5S`,
//! `2.0D`, `nozzle 1.2 double`, `F-3`, `FOCUS -2`, `FOCAL LENGTH---(-1)`,
//! `F150`, `lens 150`. This reads the common spellings and leaves the
//! rest for the operator to enter: a value is only taken where its
//! meaning is plain, and nothing here guesses a value that is not written.
//!
//! An `F` with a signed or small number is the focus offset; an unsigned
//! number of 50 or more after `F`, `FL` or `LENS` is the lens focal
//! length, since no head focuses 50 mm away from the nozzle.

use serde::{Deserialize, Serialize};

/// The nozzle's construction.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NozzleKind {
    /// A single-layer nozzle, the usual choice with air and nitrogen.
    Single,
    /// A double-layer nozzle, the usual choice with oxygen.
    Double,
}

impl NozzleKind {
    /// The value the recipe's `OpenLaserNozzleType` attribute holds.
    #[must_use]
    pub const fn attribute(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Double => "double",
        }
    }
}

/// The head setup found in a recipe's words. Each number keeps the text it
/// was written with, without a leading `+`, so `2.0` stays `2.0`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeadSetup {
    /// Nozzle bore in millimetres, such as `1.5`.
    pub nozzle_diameter_mm: Option<String>,
    /// Single or double layer.
    pub nozzle: Option<NozzleKind>,
    /// Manual focus offset in millimetres, negative into the sheet.
    pub focus_mm: Option<String>,
    /// Lens focal length in millimetres, such as `150`.
    pub lens_mm: Option<String>,
}

/// The attribute that keeps the nozzle bore.
pub const NOZZLE_DIAMETER: &str = "OpenLaserNozzleDiameter";
/// The attribute that keeps the nozzle construction.
pub const NOZZLE_TYPE: &str = "OpenLaserNozzleType";
/// The attribute that keeps the manual focus offset.
pub const FOCUS: &str = "OpenLaserManualFocus";
/// The attribute that keeps the lens focal length.
pub const LENS: &str = "OpenLaserLens";

impl HeadSetup {
    /// The setup read from each text in turn; an earlier text wins where
    /// two say different things, so pass the note before the names.
    ///
    /// ```
    /// use openlaser_server::recipes::setup::{HeadSetup, NozzleKind};
    /// let setup = HeadSetup::read(&["N2 1.5 S focus -2 F150"]);
    /// assert_eq!(setup.nozzle, Some(NozzleKind::Single));
    /// assert_eq!(setup.nozzle_diameter_mm.as_deref(), Some("1.5"));
    /// assert_eq!(setup.focus_mm.as_deref(), Some("-2"));
    /// assert_eq!(setup.lens_mm.as_deref(), Some("150"));
    /// ```
    #[must_use]
    pub fn read(texts: &[&str]) -> Self {
        let mut setup = Self::default();
        for text in texts {
            setup.fill(&parse(text));
        }
        setup
    }

    /// Whether nothing was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Takes the values this setup lacks from `other`.
    fn fill(&mut self, other: &Self) {
        if self.nozzle_diameter_mm.is_none() {
            self.nozzle_diameter_mm.clone_from(&other.nozzle_diameter_mm);
            if self.nozzle.is_none() {
                self.nozzle = other.nozzle;
            }
        }
        if self.nozzle.is_none() && other.nozzle_diameter_mm == self.nozzle_diameter_mm {
            self.nozzle = other.nozzle;
        }
        if self.focus_mm.is_none() {
            self.focus_mm.clone_from(&other.focus_mm);
        }
        if self.lens_mm.is_none() {
            self.lens_mm.clone_from(&other.lens_mm);
        }
    }

    /// The setup a recipe's attributes already hold.
    #[must_use]
    pub fn of(attributes: &std::collections::BTreeMap<String, String>) -> Self {
        let nozzle = match attributes.get(NOZZLE_TYPE).map(String::as_str) {
            Some("single") => Some(NozzleKind::Single),
            Some("double") => Some(NozzleKind::Double),
            _ => None,
        };
        Self {
            nozzle_diameter_mm: attributes.get(NOZZLE_DIAMETER).cloned(),
            nozzle,
            focus_mm: attributes.get(FOCUS).cloned(),
            lens_mm: attributes.get(LENS).cloned(),
        }
    }

    /// Writes the setup into `attributes`: a value present here replaces
    /// the attribute and an absent one removes it when `replace` is set,
    /// or is only added where the attribute is missing when it is not.
    pub fn apply(
        &self,
        attributes: &mut std::collections::BTreeMap<String, String>,
        replace: bool,
    ) {
        let pairs = [
            (NOZZLE_DIAMETER, self.nozzle_diameter_mm.clone()),
            (NOZZLE_TYPE, self.nozzle.map(|k| k.attribute().to_owned())),
            (FOCUS, self.focus_mm.clone()),
            (LENS, self.lens_mm.clone()),
        ];
        for (key, value) in pairs {
            match (value, replace) {
                (Some(value), true) => {
                    attributes.insert(key.into(), value);
                }
                (Some(value), false) => {
                    attributes.entry(key.into()).or_insert(value);
                }
                (None, true) => {
                    attributes.remove(key);
                }
                (None, false) => {}
            }
        }
    }

    /// Why the setup cannot be kept, or nothing: every number must be a
    /// number in its plausible range.
    pub fn check(&self) -> Result<(), String> {
        let check =
            |value: &Option<String>, what: &str, range: std::ops::RangeInclusive<f64>| match value
                .as_deref()
                .map(str::parse::<f64>)
            {
                None => Ok(()),
                Some(Ok(v)) if v.is_finite() && range.contains(&v) => Ok(()),
                Some(_) => {
                    Err(format!("the {what} must be {} to {} mm", range.start(), range.end()))
                }
            };
        check(&self.nozzle_diameter_mm, "nozzle diameter", 0.01..=20.)?;
        check(&self.focus_mm, "focus", -1000. ..=1000.)?;
        check(&self.lens_mm, "lens focal length", 1. ..=2000.)
    }
}

/// The bores nozzles are made in; a number outside is not a nozzle.
const NOZZLE_BORE: std::ops::RangeInclusive<f64> = 0.5..=8.;
/// A focus offset beyond this is not one.
const FOCUS_REACH: f64 = 30.;
/// The focal lengths of cutting heads' focusing lenses.
const LENS_FOCAL: std::ops::RangeInclusive<f64> = 50. ..=500.;
/// Words between a label and its value that say nothing.
const FILLER: [&str; 12] = [
    "SOURCE", "NOTE", "FILENAME", "FILE", "NAME", "POS", "POSITION", "AT", "IS", "OFFSET", "SET",
    "DIA",
];

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// Letters, upper-cased.
    Word(String),
    /// A number with its written text, sign included.
    Number { text: String, value: f64, signed: bool },
}

/// One token and whether it touches the one before it.
#[derive(Clone, Debug, PartialEq)]
struct Tok {
    token: Token,
    glued: bool,
}

/// Letters and numbers, every other character a separator. A `+` or `-`
/// right before a digit and after no digit signs the number.
fn tokens(text: &str) -> Vec<Tok> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut glued = false;
    while i < chars.len() {
        let c = chars[i];
        let starts_number = |at: usize| {
            chars.get(at).is_some_and(char::is_ascii_digit)
                || (chars.get(at) == Some(&'.')
                    && chars.get(at + 1).is_some_and(char::is_ascii_digit))
        };
        if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect::<String>().to_uppercase();
            out.push(Tok { token: Token::Word(word), glued });
            glued = true;
        } else if starts_number(i)
            || (matches!(c, '+' | '-')
                && starts_number(i + 1)
                && !(i > 0 && chars[i - 1].is_ascii_digit()))
        {
            let start = i;
            let signed = matches!(c, '+' | '-');
            if signed {
                i += 1;
            }
            let mut dot = false;
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || (chars[i] == '.'
                        && !dot
                        && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                dot |= chars[i] == '.';
                i += 1;
            }
            let written: String = chars[start..i].iter().collect();
            let text = written.trim_start_matches('+').to_owned();
            if let Ok(value) = text.parse::<f64>() {
                out.push(Tok { token: Token::Number { text, value, signed }, glued });
            }
            glued = true;
        } else {
            i += 1;
            glued = false;
        }
    }
    out
}

impl Tok {
    fn word(&self) -> Option<&str> {
        match &self.token {
            Token::Word(w) => Some(w),
            Token::Number { .. } => None,
        }
    }

    fn number(&self) -> Option<(&str, f64, bool)> {
        match &self.token {
            Token::Number { text, value, signed } => Some((text, *value, *signed)),
            Token::Word(_) => None,
        }
    }
}

fn kind(word: &str, abbreviated: bool) -> Option<NozzleKind> {
    match word {
        "SINGLE" | "SINGEL" | "SGL" => Some(NozzleKind::Single),
        "DOUBLE" | "DUAL" | "DBL" => Some(NozzleKind::Double),
        "S" | "SL" if abbreviated => Some(NozzleKind::Single),
        "D" | "DL" if abbreviated => Some(NozzleKind::Double),
        _ => None,
    }
}

/// A bore written with or without a sign (`SINGLE-2.0` reads `-2.0`).
fn bore(text: &str, value: f64) -> Option<String> {
    NOZZLE_BORE.contains(&value.abs()).then(|| text.trim_start_matches('-').to_owned())
}

/// The setup one text states.
fn parse(text: &str) -> HeadSetup {
    let t = tokens(text);
    let mut setup = HeadSetup::default();
    let mut i = 0;
    while i < t.len() {
        i = nozzle_label(&t, i, &mut setup)
            .or_else(|| kind_then_bore(&t, i, &mut setup))
            .or_else(|| bore_then_kind(&t, i, &mut setup))
            .or_else(|| focus_or_lens(&t, i, &mut setup))
            .or_else(|| lens_after(&t, i, &mut setup))
            .unwrap_or(i + 1);
    }
    setup
}

/// Keeps a bore and kind unless the text already gave one.
fn keep_nozzle(setup: &mut HeadSetup, bore: String, kind: Option<NozzleKind>) {
    if setup.nozzle_diameter_mm.is_none() {
        setup.nozzle_diameter_mm = Some(bore);
        setup.nozzle = setup.nozzle.or(kind);
    }
}

/// `NOZZLE`, then a kind and a bore in either order: `NOZZLE---SINGLE-2.0`,
/// `NOZZLE  2.0 SINGLE`, `Nozzle: 1.5mm double layer`. The index after it.
fn nozzle_label(t: &[Tok], i: usize, setup: &mut HeadSetup) -> Option<usize> {
    if !matches!(t[i].word(), Some("NOZZLE" | "NOZZEL" | "NOZ" | "NOZZLES")) {
        return None;
    }
    let mut j = i + 1;
    let mut found_kind = None;
    let mut found_bore = None;
    while j < t.len() && j <= i + 5 {
        if let Some(w) = t[j].word() {
            if let Some(k) = kind(w, true) {
                found_kind.get_or_insert(k);
            } else if !(w == "MM" || w == "LAYER" || FILLER.contains(&w)) {
                break;
            }
        } else if let Some((text, value, _)) = t[j].number() {
            match (&found_bore, bore(text, value)) {
                (None, Some(b)) => found_bore = Some(b),
                _ => break,
            }
        }
        j += 1;
    }
    if let Some(b) = found_bore {
        keep_nozzle(setup, b, found_kind);
    }
    Some(j.max(i + 1))
}

/// A kind word, then a bore: `single 2.0`, `double nozzle 1.5`, and the
/// letter alone only when it touches the bore: `S1.5`, `DL1.2`.
fn kind_then_bore(t: &[Tok], i: usize, setup: &mut HeadSetup) -> Option<usize> {
    let touching = t.get(i + 1).is_some_and(|n| n.glued && n.number().is_some());
    let found = kind(t[i].word()?, touching)?;
    let mut j = i + 1;
    while !touching && matches!(t.get(j).and_then(Tok::word), Some("LAYER" | "NOZZLE")) {
        j += 1;
    }
    let (text, value, _) = t.get(j).and_then(Tok::number)?;
    let diameter = bore(text, value)?;
    if setup.nozzle_diameter_mm.is_some() {
        return None;
    }
    keep_nozzle(setup, diameter, Some(found));
    Some(j + 1)
}

/// A bore, then a kind: `1.5S`, `2.0 D`, `1.2 double`; a letter that
/// touches the next number is that number's (`N2 S1.5`).
fn bore_then_kind(t: &[Tok], i: usize, setup: &mut HeadSetup) -> Option<usize> {
    let (text, value, signed) = t[i].number()?;
    let next = t.get(i + 1)?;
    let abbreviated = next.glued || !t.get(i + 2).is_some_and(|n| n.glued && n.number().is_some());
    let found = kind(next.word()?, abbreviated)?;
    let diameter = bore(text, value)?;
    if signed || setup.nozzle_diameter_mm.is_some() {
        return None;
    }
    keep_nozzle(setup, diameter, Some(found));
    Some(i + 2)
}

/// A focus or lens label and its number: `FOCUS -2`, `F-3`, `F150`,
/// `FOCAL LENGTH---(-1)`, `FL150`, `lens 125`.
fn focus_or_lens(t: &[Tok], i: usize, setup: &mut HeadSetup) -> Option<usize> {
    let label = t[i].word()?;
    if !matches!(label, "FOCUS" | "FOCAL" | "FOKUS" | "FOC" | "F" | "FP" | "FL" | "EFL" | "LENS") {
        return None;
    }
    let lens_only = matches!(label, "FL" | "EFL" | "LENS");
    let mut lens_named = lens_only;
    let mut j = i + 1;
    while let Some(w) = t.get(j).and_then(Tok::word) {
        if (w == "LENGTH" && label == "FOCAL") || w == "LENS" {
            lens_named = true;
        } else if !FILLER.contains(&w) && w != "FOCAL" && w != "MM" {
            break;
        }
        j += 1;
    }
    let (text, value, signed) = t.get(j).and_then(Tok::number)?;
    if !signed && LENS_FOCAL.contains(&value) && (lens_named || label == "F") {
        setup.lens_mm.get_or_insert_with(|| text.to_owned());
    } else if !lens_only && value.abs() <= FOCUS_REACH {
        setup.focus_mm.get_or_insert_with(|| text.to_owned());
    }
    Some(j + 1)
}

/// A lens written after its number: `150 lens`, `150mm lens`.
fn lens_after(t: &[Tok], i: usize, setup: &mut HeadSetup) -> Option<usize> {
    let (text, value, signed) = t[i].number()?;
    if signed || !LENS_FOCAL.contains(&value) {
        return None;
    }
    let mut j = i + 1;
    if t.get(j).and_then(Tok::word) == Some("MM") {
        j += 1;
    }
    (t.get(j).and_then(Tok::word) == Some("LENS")).then(|| {
        setup.lens_mm.get_or_insert_with(|| text.to_owned());
        j + 1
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (Option<String>, Option<NozzleKind>, Option<String>, Option<String>) {
        let s = HeadSetup::read(&[text]);
        (s.nozzle_diameter_mm, s.nozzle, s.focus_mm, s.lens_mm)
    }

    fn setup(
        nozzle: Option<(&str, Option<NozzleKind>)>,
        focus: Option<&str>,
        lens: Option<&str>,
    ) -> (Option<String>, Option<NozzleKind>, Option<String>, Option<String>) {
        (
            nozzle.map(|(d, _)| d.to_owned()),
            nozzle.and_then(|(_, k)| k),
            focus.map(str::to_owned),
            lens.map(str::to_owned),
        )
    }

    use NozzleKind::{Double, Single};

    /// The notes of the vendor's own library, as its machines carry them.
    #[test]
    fn reads_the_vendor_library_notes() {
        assert_eq!(
            read("NOZZLE---SINGLE-2.0     AIR PRESSURE---20BAR  FOCAL LENGTH---(-1)"),
            setup(Some(("2.0", Some(Single))), Some("-1"), None)
        );
        assert_eq!(
            read("NOZZLE---DOUBLE-1.2   FOCAL LENGTH---(+13.5)"),
            setup(Some(("1.2", Some(Double))), Some("13.5"), None)
        );
        assert_eq!(
            read("NOZZLE---SINGLE-2.0    AIR PRESSURE---15BAR  FOCAL LENGTH---(+0)"),
            setup(Some(("2.0", Some(Single))), Some("0"), None)
        );
        assert_eq!(
            read("NOZZLE---DOUBLE-2.0    AIR PRESSURE---20BAR FOCAL LENGTH---(-10)"),
            setup(Some(("2.0", Some(Double))), Some("-10"), None)
        );
        assert_eq!(
            read("NOZZLE  2.0 SINGLE FOCUS    -1 GAS        AIR PRESSURE     8 BAR"),
            setup(Some(("2.0", Some(Single))), Some("-1"), None)
        );
        assert_eq!(read("NOZZLE---SINGLE-2.0\n"), setup(Some(("2.0", Some(Single))), None, None));
    }

    /// The clean library's normalised note.
    #[test]
    fn reads_the_clean_library_note() {
        let note = "MATERIAL: Aluminum\nTHICKNESS: 1 mm\nPROCESS: CUTTING\nGAS: AIR\nNOZZLE: SINGLE 2.0 mm\nFOCUS (SOURCE NOTE/FILENAME): -1 mm\nGAS PRESSURE (SOURCE NOTE): 8 bar";
        assert_eq!(read(note), setup(Some(("2.0", Some(Single))), Some("-1"), None));
    }

    /// File and layer names: the kind glued to the bore, focus after `F`.
    #[test]
    fn reads_file_and_layer_names() {
        assert_eq!(
            read("SS1.0mm 1.5S F-3 N2"),
            setup(Some(("1.5", Some(Single))), Some("-3"), None)
        );
        assert_eq!(
            read("SS2.0mm 1.5S F-3N2.xml"),
            setup(Some(("1.5", Some(Single))), Some("-3"), None)
        );
        assert_eq!(
            read("AL aluminium1.0mm  single 2.0 F -1  air"),
            setup(Some(("2.0", Some(Single))), Some("-1"), None)
        );
        assert_eq!(
            read("Carbon Steel - 10mm - O2 - 4.0D"),
            setup(Some(("4.0", Some(Double))), None, None)
        );
        assert_eq!(
            read("Aluminum - 1mm - AIR - 2.0S.xml"),
            setup(Some(("2.0", Some(Single))), None, None)
        );
        assert_eq!(
            read("CS-3MM_O2 CUTTING 1.2D F+1"),
            setup(Some(("1.2", Some(Double))), Some("1"), None)
        );
        assert_eq!(read("2.0D"), setup(Some(("2.0", Some(Double))), None, None));
        assert_eq!(read("SS 3mm N2 S1.5 F0"), setup(Some(("1.5", Some(Single))), Some("0"), None));
        assert_eq!(read("CS 6mm O2 D1.4"), setup(Some(("1.4", Some(Double))), None, None));
        assert_eq!(
            read("MS_8mm_O2_DL1.2_F+2"),
            setup(Some(("1.2", Some(Double))), Some("2"), None)
        );
    }

    /// The operator's own shorthand, the way it gets typed.
    #[test]
    fn reads_free_text() {
        assert_eq!(
            read("N2 1.5 S focus -2 F150"),
            setup(Some(("1.5", Some(Single))), Some("-2"), Some("150"))
        );
        assert_eq!(read("nozzle 1.2 double"), setup(Some(("1.2", Some(Double))), None, None));
        assert_eq!(
            read("Nozzle: 1.5mm double layer"),
            setup(Some(("1.5", Some(Double))), None, None)
        );
        assert_eq!(read("nozzle 3.0"), setup(Some(("3.0", None)), None, None));
        assert_eq!(
            read("Nozzle S2.0, focus: -2.5mm"),
            setup(Some(("2.0", Some(Single))), Some("-2.5"), None)
        );
        assert_eq!(read("focus=+1.5"), setup(None, Some("1.5"), None));
        assert_eq!(read("Focus position -4 mm, lens 125"), setup(None, Some("-4"), Some("125")));
        assert_eq!(read("FL150 F-1.5"), setup(None, Some("-1.5"), Some("150")));
        assert_eq!(read("focal length 200"), setup(None, None, Some("200")));
        assert_eq!(read("150mm lens, focus -3"), setup(None, Some("-3"), Some("150")));
        assert_eq!(
            read("double nozzle 1.5, F 150, focus 0"),
            setup(Some(("1.5", Some(Double))), Some("0"), Some("150"))
        );
        assert_eq!(read("single-layer 2.5 nozzle"), setup(Some(("2.5", Some(Single))), None, None));
        assert_eq!(read("1.5 single"), setup(Some(("1.5", Some(Single))), None, None));
    }

    /// Words that look like setup but are not: thickness, gases, pressures,
    /// the material's own letters, numbers out of range.
    #[test]
    fn leaves_what_is_not_there() {
        let nothing = setup(None, None, None);
        assert_eq!(read("SS-2MM_N2 CUTTING"), nothing);
        assert_eq!(read("Carbon steel-10MM_O2 CUTTING.xml"), nothing);
        assert_eq!(read("CS  3MM_Carbon steel O2"), nothing);
        assert_eq!(read("GS 1MM_Galvanized sheet  air"), nothing);
        assert_eq!(read("Basswood-3MM_CO2 CUTTING"), nothing);
        assert_eq!(read("AIR PRESSURE---20BAR"), nothing);
        assert_eq!(read(""), nothing);
        assert_eq!(read("nozzle 12"), nothing);
        assert_eq!(read("focus -45"), nothing);
        assert_eq!(read("SS 10 D"), nothing);
        assert_eq!(read("Stainless steel 2mm"), nothing);
        assert_eq!(read("offset 3"), nothing);
    }

    /// The note wins over the names, and the names fill what it lacks.
    #[test]
    fn earlier_texts_win() {
        let s = HeadSetup::read(&["NOZZLE---DOUBLE-1.5", "SS1.0mm 2.0S F-3 N2"]);
        assert_eq!(s.nozzle_diameter_mm.as_deref(), Some("1.5"));
        assert_eq!(s.nozzle, Some(Double));
        assert_eq!(s.focus_mm.as_deref(), Some("-3"));
        let s = HeadSetup::read(&["nozzle 1.5", "1.5D"]);
        assert_eq!(s.nozzle, Some(Double));
        let s = HeadSetup::read(&["nozzle 1.5", "2.0D"]);
        assert_eq!(s.nozzle, None);
    }

    /// Writing keeps what the file had unless the operator confirmed.
    #[test]
    fn applies_to_attributes() {
        let mut attributes =
            std::collections::BTreeMap::from([(FOCUS.to_owned(), "-5".to_owned())]);
        let s = HeadSetup::read(&["1.5S F-3 F150"]);
        s.apply(&mut attributes, false);
        assert_eq!(attributes[FOCUS], "-5");
        assert_eq!(attributes[NOZZLE_DIAMETER], "1.5");
        assert_eq!(attributes[NOZZLE_TYPE], "single");
        assert_eq!(attributes[LENS], "150");
        assert_eq!(HeadSetup::of(&attributes).focus_mm.as_deref(), Some("-5"));
        HeadSetup { focus_mm: Some("-1".into()), ..HeadSetup::default() }
            .apply(&mut attributes, true);
        assert_eq!(attributes.get(FOCUS).map(String::as_str), Some("-1"));
        assert!(!attributes.contains_key(NOZZLE_DIAMETER));
        assert!(!attributes.contains_key(LENS));
        assert!(HeadSetup { lens_mm: Some("x".into()), ..HeadSetup::default() }.check().is_err());
        assert!(HeadSetup::read(&["1.5S F-3 F150"]).check().is_ok());
    }
}
