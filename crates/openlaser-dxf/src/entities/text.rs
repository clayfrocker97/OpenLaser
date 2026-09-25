// SPDX-License-Identifier: GPL-3.0-or-later

//! Plain TEXT and MTEXT lettering, resolved after drawing units are known.

use super::Fields;
use crate::{Error, Result};
use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_svg::{Alignment, Family, Text};
use std::sync::OnceLock;

pub(crate) struct TextEntity {
    value: String,
    position: Point,
    height: f64,
    width: f64,
    rotation: f64,
    oblique: f64,
    mirror: u32,
    horizontal: u32,
    vertical: u32,
    rectangle: Option<f64>,
    multiline: bool,
    spacing: f64,
    /// MTEXT formatting (fonts, heights, colours, stacking) was dropped and
    /// the lettering kept in the bundled font.
    pub formatted: bool,
    pub layer: String,
    pub line: usize,
    pub style: String,
}

impl TextEntity {
    pub(crate) fn read(fields: &Fields<'_>, multiline: bool) -> Result<Self> {
        fields.planar()?;
        let raw = if multiline {
            fields
                .pairs
                .iter()
                .filter(|p| matches!(p.code, 1 | 3))
                .map(|p| p.value)
                .collect::<String>()
        } else {
            fields.one(1)?.ok_or_else(|| fields.error("text is missing"))?.into()
        };
        let (value, formatted) = plain(&raw, multiline).map_err(|reason| fields.error(reason))?;
        if value.trim().is_empty() {
            return Err(fields.error("text is empty"));
        }
        let height = fields.required(40)?;
        let width = if multiline { 1. } else { fields.number(41)?.unwrap_or(1.) };
        if height <= 0. || width <= 0. {
            return Err(fields.error("text height and width must be positive"));
        }
        let horizontal = if multiline { fields.flags(71)? } else { fields.flags(72)? };
        let vertical = if multiline { 0 } else { fields.flags(73)? };
        if (!multiline && (horizontal > 2 || vertical > 3)) || (multiline && horizontal > 9) {
            return Err(fields.error("aligned or fitted text is not supported"));
        }
        if multiline && !matches!(fields.flags(72)?, 0 | 1 | 5) {
            return Err(fields.error("vertical MTEXT is not supported"));
        }
        if multiline && (fields.flags(75)? != 0 || fields.flags(76)? > 1) {
            return Err(fields.error("MTEXT columns are not supported"));
        }
        let spacing = if multiline { fields.number(44)?.unwrap_or(1.) } else { 1. };
        if !(0.25..=4.).contains(&spacing) {
            return Err(fields.error("MTEXT line spacing must be between 0.25 and 4"));
        }
        let position = if !multiline && (horizontal != 0 || vertical != 0) {
            fields.point(11, 21)?
        } else {
            fields.point(10, 20)?
        };
        let mut rotation = fields.number(50)?.unwrap_or(0.);
        if multiline {
            let direction_at = fields.pairs.iter().rposition(|p| p.code == 11);
            let angle_at = fields.pairs.iter().rposition(|p| p.code == 50);
            if direction_at.is_some() && direction_at > angle_at {
                let x = fields.required(11)?;
                let y = fields.number(21)?.unwrap_or(0.);
                if x == 0. && y == 0. {
                    return Err(fields.error("MTEXT direction has zero length"));
                }
                rotation = y.atan2(x);
            }
        } else {
            rotation = rotation.to_radians();
        }
        let oblique = if multiline { 0. } else { fields.number(51)?.unwrap_or(0.).to_radians() };
        if oblique.abs() >= 85f64.to_radians() {
            return Err(fields.error("text oblique angle is too steep"));
        }
        let mirror = if multiline { 0 } else { fields.flags(71)? };
        if mirror & !6 != 0 {
            return Err(fields.error("unknown text generation flags"));
        }
        Ok(Self {
            value,
            position,
            height,
            width,
            rotation,
            oblique,
            mirror,
            horizontal,
            vertical,
            rectangle: if multiline { fields.number(41)?.filter(|v| *v > 0.) } else { None },
            multiline,
            spacing,
            formatted,
            layer: fields.layer(),
            line: fields.line,
            style: fields.one(7)?.unwrap_or("STANDARD").into(),
        })
    }

    /// The rows to draw. MTEXT wraps its words to the width of its box, as
    /// `AutoCAD` lays it out; a word longer than the box stays whole. Each
    /// word is measured once and a row is its words and the spaces between.
    fn lines(
        &self,
        text: &mut Text,
        scale: f64,
    ) -> std::result::Result<Vec<String>, openlaser_svg::Error> {
        let Some(limit) = self.rectangle.map(|w| w * scale + 0.01) else {
            return Ok(self.value.split('\n').map(str::to_owned).collect());
        };
        let mut widths: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        let mut measure = |word: &str| -> std::result::Result<f64, openlaser_svg::Error> {
            if let Some(width) = widths.get(word) {
                return Ok(*width);
            }
            text.value = word.into();
            let width = text.outlines()?.drawing.bounds().map_or(0., |b| b.width());
            widths.insert(word.into(), width);
            Ok(width)
        };
        // A space, with the bearings either side of it, as the font sets it.
        let space = measure("H H")? - 2. * measure("H")?;
        let mut lines = Vec::new();
        for paragraph in self.value.split('\n') {
            let (mut current, mut width) = (String::new(), 0.);
            for word in paragraph.split(' ') {
                let own = measure(word)?;
                if !current.is_empty() && width + space + own > limit {
                    lines.push(std::mem::take(&mut current));
                    width = 0.;
                }
                if !current.is_empty() {
                    current.push(' ');
                    width += space;
                }
                current.push_str(word);
                width += own;
            }
            lines.push(current);
        }
        Ok(lines)
    }

    pub(crate) fn outlines(&self, scale: f64) -> Result<Vec<Contour>> {
        let error = |reason: String| Error::Entity {
            line: self.line,
            entity: if self.multiline { "MTEXT" } else { "TEXT" }.into(),
            reason,
        };
        let size = self.height * scale / cap_ratio().map_err(|e| error(e.to_string()))?;
        let mut text = Text {
            value: String::new(),
            family: Family::Sans,
            size,
            bold: false,
            alignment: Alignment::Left,
        };
        let lines = self.lines(&mut text, scale).map_err(|e| error(e.to_string()))?;
        let mut drawing = openlaser_core::geometry::Drawing::default();
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            text.value.clone_from(line);
            let outlined = text.outlines().map_err(|e| error(e.to_string()))?;
            let row = f64::from(u32::try_from(index).map_err(|_| error("too many lines".into()))?);
            // DXF default baseline spacing is 5/3 of nominal text height.
            let offset = Point::new(0., -row * self.height * scale * (5. / 3.) * self.spacing);
            drawing.contours.extend(outlined.drawing.contours.into_iter().map(|mut contour| {
                for curve in &mut contour.curves {
                    *curve = curve.translated(offset);
                }
                contour
            }));
        }
        let bounds =
            drawing.bounds().ok_or_else(|| error("the text produced no outlines".into()))?;
        let (horizontal, vertical) = if self.multiline {
            let attachment = self.horizontal.max(1) - 1;
            (attachment % 3, 3 - attachment / 3)
        } else {
            (self.horizontal, self.vertical)
        };
        let x = match horizontal {
            1 => bounds.min.x.midpoint(bounds.max.x),
            2 => bounds.max.x,
            _ => 0.,
        };
        let y = match vertical {
            1 => bounds.min.y,
            2 => bounds.min.y.midpoint(bounds.max.y),
            3 => bounds.max.y,
            _ => 0.,
        };
        let transform = |p: Point| {
            let y = p.y - y;
            let x = (p.x - x) * self.width + y * self.oblique.tan();
            let local = Point::new(
                if self.mirror & 2 != 0 { -x } else { x },
                if self.mirror & 4 != 0 { -y } else { y },
            );
            local.rotated(self.rotation) + self.position * scale
        };
        for contour in &mut drawing.contours {
            contour.layer.clone_from(&self.layer);
            for curve in &mut contour.curves {
                *curve = Curve::Line {
                    start: transform(curve.point(0.)),
                    end: transform(curve.point(1.)),
                };
            }
        }
        Ok(drawing.contours)
    }
}

fn cap_ratio() -> std::result::Result<f64, openlaser_svg::Error> {
    static RATIO: OnceLock<std::result::Result<f64, openlaser_svg::Error>> = OnceLock::new();
    RATIO
        .get_or_init(|| {
            let text = Text {
                value: "H".into(),
                family: Family::Sans,
                size: 1.,
                bold: false,
                alignment: Alignment::Left,
            };
            text.outlines()?
                .drawing
                .bounds()
                .map(|b| b.height())
                .filter(|height| *height > 0.)
                .ok_or_else(|| {
                    openlaser_svg::Error("the bundled font has no capital height".into())
                })
        })
        .clone()
}

/// The characters to draw, and whether MTEXT formatting was dropped on the
/// way. MTEXT codes that change fonts, heights, colours, alignment or
/// underlining are left out and the lettering kept; a stacked fraction is
/// written as `a/b`. TEXT has no formatting codes: its backslashes are text.
fn plain(source: &str, multiline: bool) -> std::result::Result<(String, bool), String> {
    let source = special(source);
    let mut chars = source.chars().peekable();
    let mut output = String::new();
    let mut formatted = false;
    // An argument runs to the next `;`.
    let argument = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| -> String {
        chars.by_ref().take_while(|c| *c != ';').collect()
    };
    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' if multiline => {}
            '\\' => match chars.next() {
                Some('P' | 'N' | 'X') if multiline => output.push('\n'),
                Some('~') if multiline => output.push(' '),
                Some(ch @ ('\\' | '{' | '}')) => output.push(ch),
                Some('U') if chars.peek() == Some(&'+') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    let value = (hex.len() == 4)
                        .then(|| u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32))
                        .flatten()
                        .ok_or_else(|| "invalid Unicode text escape".to_owned())?;
                    output.push(value);
                }
                Some('f' | 'F' | 'H' | 'W' | 'Q' | 'T' | 'A' | 'C' | 'c' | 'p') if multiline => {
                    argument(&mut chars);
                    formatted = true;
                }
                Some('L' | 'l' | 'O' | 'o' | 'K' | 'k') if multiline => formatted = true,
                Some('S') if multiline => {
                    output.push_str(&argument(&mut chars).replace(['^', '#'], "/"));
                    formatted = true;
                }
                Some(code) if multiline => {
                    return Err(format!("text formatting \\{code} is not supported"));
                }
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None if multiline => return Err("unfinished text escape".into()),
                None => output.push('\\'),
            },
            _ => output.push(ch),
        }
    }
    Ok((output, formatted))
}

/// `AutoCAD`'s `%%` codes: degrees, plus-minus, diameter and a character by
/// number drawn as themselves; underline, overline and strike toggles left out.
fn special(source: &str) -> String {
    let mut output = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("%%") {
        output.push_str(&rest[..at]);
        let code = &rest[at + 2..];
        let mut chars = code.chars();
        match chars.next().map(|c| c.to_ascii_lowercase()) {
            Some('d') => output.push('°'),
            Some('p') => output.push('±'),
            Some('c') => output.push('Ø'),
            Some('u' | 'o' | 'k') => {}
            Some('%') => output.push('%'),
            Some(d) if d.is_ascii_digit() => {
                let digits: String =
                    code.chars().take(3).take_while(char::is_ascii_digit).collect();
                if let Some(ch) = digits.parse::<u32>().ok().and_then(char::from_u32) {
                    output.push(ch);
                }
                rest = &code[digits.len()..];
                continue;
            }
            _ => {
                output.push_str("%%");
                rest = code;
                continue;
            }
        }
        rest = &code[1..];
    }
    output.push_str(rest);
    output
}
