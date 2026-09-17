// SPDX-License-Identifier: GPL-3.0-or-later

//! What preparation hands to the compiler: contours in cutting order, each
//! with how every one of its segments is cut.

use crate::geometry::{Bounds, Curve};
use serde::{Deserialize, Serialize};

/// Which lead a segment belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadRole {
    /// The lead-in, cut before the contour.
    Entry,
    /// The lead-out, cut after it.
    Exit,
}

/// How one segment of a contour is cut.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SegmentProcess {
    /// The lead this segment is part of, if any.
    pub lead: Option<LeadRole>,
    /// Whether the segment is a joint, cut with reduced power or none.
    pub joint: bool,
    /// Whether the cut re-pierces at the start of this segment.
    pub repierce: bool,
    /// A power override in percent. A joint's scales the recipe's power;
    /// any other replaces it.
    pub power: Option<f64>,
    /// A speed override in millimetres per second, for a joint.
    pub speed: Option<f64>,
    /// A cooling stop at the segment's start, in milliseconds.
    pub cool_ms: u32,
}

/// The interval of a placed contour retained by a prepared segment.
/// Fractions follow the original contour; a decreasing interval is reversed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceInterval {
    /// Placed contour identity within the input drawing snapshot.
    pub contour: usize,
    /// Original length fraction at the segment's start.
    pub start: f64,
    /// Original length fraction at the segment's end.
    pub end: f64,
}

/// Geometry and its process travel together through preparation and
/// compilation. `G` is an analytic curve or the compiler's geometry type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedSegment<G = Curve> {
    /// The analytic geometry.
    pub curve: G,
    /// How this segment is cut.
    pub process: SegmentProcess,
    /// Retained source interval; generated leads and channel edges have none.
    pub source: Option<SourceInterval>,
}

impl<G> PreparedSegment<G> {
    /// A generated segment with explicit process settings.
    #[must_use]
    pub const fn new(curve: G, process: SegmentProcess) -> Self {
        Self { curve, process, source: None }
    }

    /// Convert geometry while preserving process and ownership.
    #[must_use]
    pub fn map_curve<H>(&self, map: impl FnOnce(&G) -> H) -> PreparedSegment<H> {
        PreparedSegment {
            curve: map(&self.curve),
            process: self.process.clone(),
            source: self.source,
        }
    }
}

impl PreparedSegment {
    /// Retain a subinterval of a segment and its source interval.
    #[must_use]
    pub fn slice(&self, from: f64, to: f64) -> Self {
        let mut segment = self.map_curve(|curve| curve.slice(from, to));
        segment.source = self.source.map(|source| {
            let span = source.end - source.start;
            SourceInterval {
                contour: source.contour,
                start: source.start + span * from,
                end: source.start + span * to,
            }
        });
        segment
    }
}

/// One contour ready to cut.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedContour {
    /// The drawing contours it was made from.
    pub sources: Vec<usize>,
    /// Stable source location for editing this contour's leads. Kept apart
    /// from shared cutting sources, which common-edge grouping can expand.
    #[serde(default)]
    pub lead_target: Option<crate::features::Spot>,
    /// The drawing layer.
    pub layer: String,
    /// Whether the geometry closes on itself, leads aside.
    pub closed: bool,
    /// How many closed contours enclose it: even is an outline, odd a hole.
    pub depth: usize,
    /// The segments in cutting order, leads included.
    pub segments: Vec<PreparedSegment>,
    /// The bare geometry a film pass traces: the contour as it is cut,
    /// before its joints, seam and leads.
    #[serde(default)]
    pub film: Vec<Curve>,
}

impl PreparedContour {
    /// The cutting length, leads included.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.segments.iter().map(|segment| segment.curve.length()).sum()
    }
}

/// A prepared drawing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Toolpath {
    /// The contours in cutting order.
    pub contours: Vec<PreparedContour>,
    /// Things worth telling the operator that did not stop preparation.
    pub warnings: Vec<String>,
}

impl Toolpath {
    /// The extent of every contour, `None` when there is nothing to cut.
    #[must_use]
    pub fn bounds(&self) -> Option<Bounds> {
        self.contours
            .iter()
            .flat_map(|c| c.segments.iter().map(|s| s.curve.bounds()))
            .reduce(Bounds::union)
    }

    /// The total cutting length.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.contours.iter().map(PreparedContour::length).sum()
    }
}
