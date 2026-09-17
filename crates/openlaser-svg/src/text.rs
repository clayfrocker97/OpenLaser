// SPDX-License-Identifier: GPL-3.0-or-later

//! Text creation uses the same shaping and outline conversion as SVG import.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt::Write;

/// Portable font families included with OpenLaser.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Noto Sans.
    #[default]
    Sans,
    /// Noto Serif.
    Serif,
    /// Noto Sans Mono.
    Mono,
}

impl Family {
    /// The SVG font family name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sans => "Noto Sans",
            Self::Serif => "Noto Serif",
            Self::Mono => "Noto Sans Mono",
        }
    }
}

/// Alignment of each line about a shared text anchor.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// Start each line at the anchor.
    #[default]
    Left,
    /// Centre each line at the anchor.
    Center,
    /// End each line at the anchor.
    Right,
}

/// A new text part. Size is the typographic em size in millimetres.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Text {
    /// Text, including line breaks.
    pub value: String,
    /// Font family.
    #[serde(default)]
    pub family: Family,
    /// Font size in millimetres.
    pub size: f64,
    /// Use the family's bold font.
    #[serde(default)]
    pub bold: bool,
    /// Multiline alignment.
    #[serde(default)]
    pub alignment: Alignment,
}

impl Text {
    /// Outlines relative to the first line's baseline and text anchor.
    pub fn outlines(&self) -> Result<crate::Import> {
        let svg = self.svg()?;
        let mut imported = crate::import(svg.as_bytes())?;
        let rows = self.value.replace("\r\n", "\n").replace('\r', "\n").split('\n').count();
        let rows = f64::from(u32::try_from(rows).map_err(|_| Error("too many text lines".into()))?);
        let baseline = self.size * (rows * 1.3 + 0.2 - 1.);
        let (_, anchor) = self.width_and_anchor()?;
        for contour in &mut imported.drawing.contours {
            for curve in &mut contour.curves {
                *curve = curve.translated(openlaser_core::geometry::Point::new(-anchor, -baseline));
            }
        }
        Ok(imported)
    }

    /// A self-contained SVG source using bundled fonts; the original stays
    /// with the library part so its wording and dimensions are recoverable.
    pub fn svg(&self) -> Result<String> {
        self.render(self.family.name(), if self.bold { 700 } else { 400 }, "normal", "normal")
    }

    /// SVG using the imported face's actual family, weight, style and width.
    pub fn svg_with_font(&self, font: &crate::FontFace) -> Result<String> {
        self.render(&font.family, font.weight, &font.style, &font.stretch)
    }

    fn render(&self, family: &str, weight: u16, style: &str, stretch: &str) -> Result<String> {
        if self.value.trim().is_empty() {
            return Err(Error("enter some text first".into()));
        }
        if self.value.chars().count() > 10_000 {
            return Err(Error("text exceeds 10,000 characters".into()));
        }
        if !self.size.is_finite() || !(0.1..=1000.).contains(&self.size) {
            return Err(Error("font size must be between 0.1 and 1,000 mm".into()));
        }
        let text = self.value.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<_> = text.split('\n').collect();
        let count =
            f64::from(u32::try_from(lines.len()).map_err(|_| Error("too many text lines".into()))?);
        let height = self.size * (count * 1.3 + 0.2);
        let (width, x) = self.width_and_anchor()?;
        let anchor = match self.alignment {
            Alignment::Left => "start",
            Alignment::Center => "middle",
            Alignment::Right => "end",
        };
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}mm\" height=\"{height}mm\" viewBox=\"0 0 {width} {height}\"><text font-family=\"'{}'\" font-size=\"{}\" font-weight=\"{weight}\" font-style=\"{style}\" font-stretch=\"{stretch}\" text-anchor=\"{anchor}\" xml:space=\"preserve\">",
            escape(&family.replace('\\', "\\\\").replace('\'', "\\'")).replace('"', "&quot;"),
            self.size,
        );
        for (index, line) in lines.iter().enumerate() {
            let y = self.size
                * (1.
                    + f64::from(
                        u32::try_from(index).map_err(|_| Error("too many text lines".into()))?,
                    ) * 1.3);
            write!(svg, "<tspan x=\"{x}\" y=\"{y}\">{}</tspan>", escape(line))
                .map_err(|e| Error(e.to_string()))?;
        }
        svg.push_str("</text></svg>");
        Ok(svg)
    }

    fn width_and_anchor(&self) -> Result<(f64, f64)> {
        let longest = self.value.lines().map(|line| line.chars().count()).max().unwrap_or(1);
        let count =
            f64::from(u32::try_from(longest).map_err(|_| Error("text line is too long".into()))?);
        let width = (count + 1.) * self.size * 1.5;
        let anchor = match self.alignment {
            Alignment::Left => 0.,
            Alignment::Center => width / 2.,
            Alignment::Right => width,
        };
        Ok((width, anchor))
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
