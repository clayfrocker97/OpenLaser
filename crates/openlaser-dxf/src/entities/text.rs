// SPDX-License-Identifier: GPL-3.0-or-later

//! Plain TEXT and MTEXT lettering, resolved after drawing units are known.

use super::Fields;
use crate::{Error, Result};
use openlaser_core::geometry::{Contour, Curve, Point};
use openlaser_svg::{Alignment, Family, Text};
use std::sync::OnceLock;

pub(super) struct TextEntity {
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
    layer: String,
    line: usize,
    pub style: String,
}

impl TextEntity {
    pub(super) fn read(fields: &Fields<'_>, multiline: bool) -> Result<Self> {
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
        let value = plain(&raw, multiline).map_err(|reason| fields.error(reason))?;
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
            return Err(fields
                .error("aligned or fitted text must be converted to outlines before importing"));
        }
        if multiline && !matches!(fields.flags(72)?, 0 | 1 | 5) {
            return Err(
                fields.error("vertical MTEXT must be converted to outlines before importing")
            );
        }
        if multiline && (fields.flags(75)? != 0 || fields.flags(76)? > 1) {
            return Err(
                fields.error("MTEXT columns must be converted to outlines before importing")
            );
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
            layer: fields.layer(),
            line: fields.line,
            style: fields.one(7)?.unwrap_or("STANDARD").into(),
        })
    }

    pub(super) fn outlines(&self, scale: f64) -> Result<Vec<Contour>> {
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
        let mut drawing = openlaser_core::geometry::Drawing::default();
        for (index, line) in self.value.split('\n').enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            text.value = line.into();
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
        if self.rectangle.is_some_and(|width| bounds.width() > width * scale + 0.01) {
            return Err(error("MTEXT needs automatic wrapping; add explicit line breaks or convert it to outlines before importing".into()));
        }
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

fn plain(source: &str, multiline: bool) -> std::result::Result<String, String> {
    let source = source.replace("%%d", "°").replace("%%p", "±").replace("%%c", "Ø");
    let mut chars = source.chars();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' if multiline => {}
            '\\' => match chars.next() {
                Some('P') if multiline => output.push('\n'),
                Some('~') if multiline => output.push(' '),
                Some(ch @ ('\\' | '{' | '}')) => output.push(ch),
                Some('U') if chars.next() == Some('+') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() != 4 {
                        return Err("invalid Unicode text escape".into());
                    }
                    let value = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| "invalid Unicode text escape".to_owned())?;
                    output.push(value);
                }
                Some(code) => {
                    return Err(format!(
                        "text formatting \\{code} must be converted to outlines before importing"
                    ));
                }
                None => return Err("unfinished text escape".into()),
            },
            _ => output.push(ch),
        }
    }
    Ok(output)
}
