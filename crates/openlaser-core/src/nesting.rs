// SPDX-License-Identifier: GPL-3.0-or-later

//! Persisted stock and nesting choices; no optimizer or machine authority.

use crate::geometry::{Bounds, Contour, Placed};
use serde::{Deserialize, Serialize};

/// Allowed rotations relative to each part's current orientation.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestRotation {
    /// Any angle, for dense packing.
    #[default]
    Any,
    /// Zero or 180 degrees, keeping the current grain axis.
    HalfTurn,
    /// 90 or 270 degrees, turning the current grain axis across the sheet.
    Across,
    /// No rotation.
    Fixed,
}

/// Minimum distances in millimetres, independent of machining clearance.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NestSettings {
    /// Extra distance from old cutouts and the outer edge for manual remnant alignment.
    #[serde(default)]
    pub remnant_clearance: f64,
    /// Minimum distance between part edges.
    pub spacing: f64,
    /// Minimum distance from stock edges.
    pub margin: f64,
    /// Grain/orientation constraint.
    pub rotation: NestRotation,
}

impl Default for NestSettings {
    fn default() -> Self {
        Self { spacing: 3., margin: 3., remnant_clearance: 10., rotation: NestRotation::Any }
    }
}

/// Stock is a reference boundary, never a cutting contour.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NestStock {
    /// Rectangular sheet in drawing coordinates.
    Rectangle {
        /// Sheet bounds.
        bounds: Bounds,
    },
    /// An operator-confirmed remaining sheet, with immutable removed regions.
    Remnant {
        /// Library identity for provenance; geometry remains usable independently.
        reference: String,
        /// Operator's sheet name.
        name: String,
        /// Retained outer boundary in drawing coordinates.
        outline: Contour,
        /// Regions from which material has already been removed.
        cutouts: Vec<Contour>,
        /// Conservative clearance for kerf and leads used by previous cuts.
        #[serde(default)]
        clearance: f64,
    },
    /// One closed outline retained from the original drawing.
    Outline {
        /// Its source and placement before it was removed from cutting.
        contour: Placed,
    },
}

/// Nesting authoring data saved with a job and in undo history.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Nesting {
    /// The stock outline.
    pub stock: NestStock,
    /// Minimum spacing and allowed rotations.
    pub settings: NestSettings,
}

impl NestSettings {
    /// Validate distances before storing or searching.
    pub fn validate(&self) -> Result<(), String> {
        if [self.spacing, self.margin, self.remnant_clearance]
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1000.).contains(v))
        {
            return Err(
                "spacing, edge margin and remnant clearance must be between 0 and 1000 mm".into()
            );
        }
        Ok(())
    }
}

impl Nesting {
    /// The requested stock margin, including inherited machining clearance.
    #[must_use]
    pub fn margin(&self) -> f64 {
        let previous = match &self.stock {
            NestStock::Remnant { clearance, .. } => *clearance,
            _ => 0.,
        };
        self.settings.margin.max(previous)
            + if matches!(self.stock, NestStock::Remnant { .. }) {
                self.settings.remnant_clearance
            } else {
                0.
            }
    }

    /// Validate stock references without loading or changing the drawing.
    pub fn validate(&self, source_count: usize) -> Result<(), String> {
        self.settings.validate()?;
        match &self.stock {
            NestStock::Rectangle { bounds } => {
                if [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 100_000.)
                    || bounds.width() <= 0.
                    || bounds.height() <= 0.
                {
                    return Err("stock needs a positive width and height within 100000 mm".into());
                }
            }
            NestStock::Remnant { reference, name, outline, cutouts, clearance } => {
                if reference.len() > 128
                    || name.len() > 480
                    || cutouts.len() > 10_000
                    || !clearance.is_finite()
                    || !(0. ..=1000.).contains(clearance)
                    || std::iter::once(outline)
                        .chain(cutouts)
                        .any(|c| !c.is_closed() || c.curves.iter().any(|p| !p.is_valid()))
                {
                    return Err("invalid remnant boundary or cutouts".into());
                }
            }
            NestStock::Outline { contour } => {
                if contour.source >= source_count || !contour.transform.is_similarity() {
                    return Err("the stock outline no longer matches its drawing".into());
                }
            }
        }
        Ok(())
    }
}
