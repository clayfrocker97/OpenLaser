// SPDX-License-Identifier: GPL-3.0-or-later

//! Preparation: turns a drawing and the operator's machining features into
//! an ordered toolpath.
//!
//! In order: bridges join contours, topology tells outlines from holes,
//! kerf compensation moves each closed contour into its waste, the start
//! point and direction are set, micro-joints and cooling stops split the
//! contour into tagged segments, the seam is treated, leads are attached,
//! and the contours are ordered. The output is a toolpath in
//! [`openlaser_core`] types with a process tag on every segment.
//!
//! This side of the design is ours: it owes nothing to the vendor software
//! and is free to be simpler than it.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests build small drawings and compare exact values"
    )
)]

mod bridge;
mod common;
mod joints;
mod leads;
mod order;
mod provenance;
mod shape;
mod topology;

pub use bridge::project;
pub use topology::groups;

use openlaser_core::features::{Bridges, Features, Seam, Spot};
use openlaser_core::geometry::{Contour, Drawing, Point, Transform};
use openlaser_core::toolpath::{PreparedContour, Toolpath};

/// Why a drawing could not be prepared.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A contour cannot be cut as drawn.
    #[error("contour {index}: {reason}")]
    Contour {
        /// The contour's index in the drawing.
        index: usize,
        /// What is wrong.
        reason: String,
    },
    /// A feature has a value that makes no sense.
    #[error("{feature}: {reason}")]
    Feature {
        /// The feature.
        feature: &'static str,
        /// What is wrong.
        reason: String,
    },
    /// A bridge cannot be made.
    #[error("bridge {index}: {reason}")]
    Bridge {
        /// The bridge's index, counted from one.
        index: usize,
        /// What is wrong.
        reason: String,
    },
    /// Contours overlap, so inside and outside are ambiguous.
    #[error("{0}")]
    Topology(String),
    /// The drawing is larger than preparation is willing to handle.
    #[error("the drawing exceeds the {0} budget")]
    Budget(&'static str),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// The most contours a drawing may hold.
pub const MAX_CONTOURS: usize = 5000;

/// A tolerance for comparing distances along a contour.
const EPS: f64 = 1e-8;

fn feature(feature: &'static str, reason: impl Into<String>) -> Error {
    Error::Feature { feature, reason: reason.into() }
}

/// A count as a float.
#[allow(clippy::cast_precision_loss, reason = "counts stay far below 2^53")]
fn float(count: usize) -> f64 {
    count as f64
}

/// The drawing's contours with their indices, less the skipped layers.
fn kept<'a>(drawing: &'a Drawing, features: &Features) -> Vec<(usize, &'a Contour)> {
    drawing
        .contours
        .iter()
        .enumerate()
        .filter(|(_, contour)| !features.skip_layers.contains(&contour.layer))
        .collect()
}

/// Place saved bridge picks onto a sheet. A bridge result uses the local
/// coordinates of its lowest original placed owner, which is preserved
/// through each join/split. Missing ownership is a preparation error.
pub fn place_bridges(
    sheet: &Drawing,
    features: &Features,
    transforms: &[Transform],
) -> Result<Features> {
    let mut placed = features.clone();
    let (_, connections) =
        bridge::resolve(&kept(sheet, features), features.bridges.as_ref(), |owner, point| {
            transforms
                .get(owner)
                .filter(|t| t.is_similarity())
                .map(|t| t.apply(point))
                .ok_or_else(|| feature("bridges", "the placed owner has no valid transform"))
        })?;
    if let Some(bridges) = &mut placed.bridges {
        bridges.connections = connections;
    }
    Ok(placed)
}

/// The local-coordinate owner of a contour in the current bridge result.
pub fn bridge_owner(sheet: &Drawing, placed: &Features, contour: usize) -> Result<usize> {
    bridge::apply(&kept(sheet, placed), placed.bridges.as_ref())?
        .get(contour)
        .and_then(|c| c.sources.first())
        .copied()
        .ok_or_else(|| feature("bridges", "the picked result no longer exists"))
}

/// Renumber the evolving bridge list when placed contours are removed.
/// A bridge touching removed geometry, and bridges depending on that
/// missing result, are removed together. Unrelated results keep their picks.
pub fn retained_bridges(
    sheet: &Drawing,
    features: &Features,
    keep: impl Fn(usize) -> bool,
) -> Result<Option<Bridges>> {
    let Some(bridges) = &features.bridges else { return Ok(None) };
    let mut old: Vec<_> = kept(sheet, features)
        .iter()
        .enumerate()
        .map(|(index, (source, _))| keep(*source).then_some(index))
        .collect();
    let mut new: Vec<_> = old.iter().flatten().copied().collect();
    let mut identity = old.len();
    let mut connections = Vec::new();
    for (index, bridge) in bridges.connections.iter().enumerate() {
        let (a, b) = (bridge.first.contour, bridge.second.contour);
        if a >= old.len() || b >= old.len() {
            return Err(Error::Bridge {
                index: index + 1,
                reason: "refers to a contour that is not there".into(),
            });
        }
        let mapped = |at: usize| old[at].and_then(|id| new.iter().position(|&other| id == other));
        let count = if a == b { 2 } else { 1 };
        let replacements = if let (Some(first), Some(second)) = (mapped(a), mapped(b)) {
            let mut kept = *bridge;
            kept.first.contour = first;
            kept.second.contour = second;
            connections.push(kept);
            let ids: Vec<_> = (identity..identity + count).collect();
            identity += count;
            if first != second {
                new.remove(first.max(second));
            }
            new.splice(first.min(second)..=first.min(second), ids.iter().copied());
            ids.into_iter().map(Some).collect::<Vec<_>>()
        } else {
            vec![None; count]
        };
        if a != b {
            old.remove(a.max(b));
        }
        old.splice(a.min(b)..=a.min(b), replacements);
    }
    Ok(Some(Bridges { width: bridges.width, connections }))
}

/// A tap on the drawing: the nearest point on a contour within
/// `tolerance`, as a spot on that contour. With `bridging` the contours
/// are as they stand after the features' bridges, which the next bridge
/// refers to; otherwise they are the drawing's own, which spots refer to.
pub fn pick(
    drawing: &Drawing,
    features: &Features,
    at: Point,
    tolerance: f64,
    bridging: bool,
) -> Result<Option<(Spot, Point)>> {
    let kept = kept(drawing, features);
    let contours: Vec<(usize, Contour)> = if bridging {
        let bridged = bridge::apply(&kept, features.bridges.as_ref())?;
        bridged.into_iter().enumerate().map(|(index, sourced)| (index, sourced.contour)).collect()
    } else {
        kept.into_iter().map(|(index, contour)| (index, contour.clone())).collect()
    };
    let mut best: Option<(f64, Spot, Point)> = None;
    for (contour, curves) in &contours {
        let Some((point, fraction)) = project(curves, at) else { continue };
        let distance = point.distance(at);
        if distance <= tolerance && best.as_ref().is_none_or(|b| distance < b.0) {
            best = Some((distance, Spot { contour: *contour, fraction }, point));
        }
    }
    Ok(best.map(|(_, spot, point)| (spot, point)))
}

/// Prepares `drawing` under `features`.
pub fn prepare(drawing: &Drawing, features: &Features) -> Result<Toolpath> {
    validate_drawing(drawing, features)?;
    let bridged = bridge::apply(&kept(drawing, features), features.bridges.as_ref())?;
    let mapped = provenance::remap(drawing, &bridged, features)?;
    let depths = prepared_depths(drawing, &bridged, features)?;
    let mut preparation =
        Preparation { drawing, features, mapped: &mapped, warnings: Vec::new(), work: 10_000_000 };
    let mut prepared = Vec::with_capacity(bridged.len());
    let mut items = Vec::with_capacity(bridged.len());
    let mut segment_count = 0usize;
    for (index, (sourced, depth)) in bridged.iter().zip(depths).enumerate() {
        let (contour, item) = preparation.contour(index, sourced, depth)?;
        segment_count = segment_count.saturating_add(contour.segments.len());
        if segment_count > 100_000 {
            return Err(Error::Budget("prepared segments"));
        }
        prepared.push(contour);
        items.push(item);
    }
    let sequence = order::arrange(&items, &features.order)?;
    let ordered: Vec<_> = sequence.into_iter().map(|i| prepared[i].clone()).collect();
    let contours = if let Some(common) = &features.common {
        common::apply(&ordered, common)?
    } else {
        ordered
    };
    Ok(Toolpath { contours, warnings: preparation.warnings })
}

fn validate_drawing(drawing: &Drawing, features: &Features) -> Result<()> {
    if drawing.contours.len() > MAX_CONTOURS {
        return Err(Error::Budget("contour count"));
    }
    if drawing.contours.iter().map(|c| c.curves.len()).sum::<usize>() > 100_000 {
        return Err(Error::Budget("source segments"));
    }
    for (index, contour) in kept(drawing, features) {
        validate(index, contour)?;
    }
    check(features)
}

fn prepared_depths(
    drawing: &Drawing,
    bridged: &[bridge::Sourced],
    features: &Features,
) -> Result<Vec<usize>> {
    if let Some(common) = &features.common {
        common::check(common, drawing.contours.len())?;
    }
    let contours: Vec<_> = bridged.iter().map(|s| &s.contour).collect();
    let selected = |i: usize| {
        features
            .common
            .as_ref()
            .is_some_and(|c| bridged[i].sources.iter().all(|s| c.contours.contains(s)))
    };
    topology::depths_with(&contours, |i, j| selected(i) && selected(j))
}

struct Preparation<'a> {
    drawing: &'a Drawing,
    features: &'a Features,
    mapped: &'a provenance::Remapped,
    warnings: Vec<String>,
    work: usize,
}

impl Preparation<'_> {
    fn contour(
        &mut self,
        index: usize,
        sourced: &bridge::Sourced,
        depth: usize,
    ) -> Result<(PreparedContour, order::Item)> {
        let edited = self.mapped.lead_overrides.get(&index);
        let leads = self.features.leads.as_ref().map(|leads| leads.resolved(edited));
        let lead_target = if leads.as_ref().is_some_and(|l| l.entry.is_some() || l.exit.is_some()) {
            match edited {
                Some(edited) => Some(edited.location),
                None => provenance::lead_target(sourced, self.drawing, &mut self.work)?,
            }
        } else {
            None
        };
        let (contour, frame) = self.shape(index, &sourced.contour, depth)?;
        let film = contour.curves.clone();
        let segments = self.segments(&contour, sourced, &frame)?;
        let counterclockwise = contour.signed_area() >= 0.;
        let segments =
            leads::attached(segments, leads.as_ref(), frame.closed, frame.hole, counterclockwise);
        let item = order::Item {
            center: contour.bounds().map_or(Point::ORIGIN, |b| b.center()),
            circle: contour.is_circle(),
            depth,
            sources: sourced.sources.clone(),
        };
        Ok((
            PreparedContour {
                sources: sourced.sources.clone(),
                lead_target,
                layer: contour.layer,
                closed: frame.closed,
                depth,
                segments,
                film,
            },
            item,
        ))
    }

    fn shape(
        &mut self,
        index: usize,
        source: &Contour,
        depth: usize,
    ) -> Result<(Contour, joints::Frame)> {
        let closed = source.is_closed();
        let hole = depth % 2 == 1;
        let mut contour = match (&self.features.kerf, closed) {
            (Some(kerf), true) => topology::compensated(source, kerf, hole)
                .map_err(|reason| Error::Contour { index, reason })?,
            (kerf, _) => {
                if kerf.is_some() {
                    self.warnings.push(format!(
                        "contour {index} is open, so it is cut without kerf compensation"
                    ));
                }
                source.clone()
            }
        };
        let reversed = shape::reverses(&contour, self.features.start.direction);
        if reversed {
            contour = contour.reversed();
        }
        let spot = self.mapped.features.start.spots.iter().find(|s| index == s.contour);
        let start = shape::start_fraction(
            &contour,
            self.features.start.position,
            spot.map(|s| s.fraction),
            reversed,
        );
        if start > 0. {
            contour = contour.started_at(start * contour.length());
        }
        Ok((contour, joints::Frame { closed, hole, reversed, start, contour: index }))
    }

    fn segments(
        &mut self,
        contour: &Contour,
        sourced: &bridge::Sourced,
        frame: &joints::Frame,
    ) -> Result<Vec<openlaser_core::toolpath::PreparedSegment>> {
        let mut segments = joints::split(
            contour,
            self.mapped.features.joints.as_ref(),
            self.mapped.features.cooling.as_ref(),
            frame,
        )?;
        for segment in &mut segments {
            segment.source = provenance::interval(
                &segment.curve,
                &sourced.sources,
                self.drawing,
                &mut self.work,
            )?;
        }
        match shape::seamed(&segments, self.features.seam, frame.closed) {
            Ok(seamed) => segments = seamed,
            Err(reason) => self.warnings.push(format!("contour {}: {reason}", frame.contour)),
        }
        Ok(segments)
    }
}

/// A contour must have geometry, every curve must be sound, and each curve
/// must start where the one before ends.
fn validate(index: usize, contour: &Contour) -> Result<()> {
    let reason = if contour.curves.is_empty() {
        "has no geometry"
    } else if contour.curves.iter().any(|c| !c.is_valid()) {
        "has a degenerate curve"
    } else if !contour.is_continuous() {
        "is not continuous"
    } else {
        return Ok(());
    };
    Err(Error::Contour { index, reason: reason.into() })
}

/// Every feature value must make sense before any contour is touched.
fn check(features: &Features) -> Result<()> {
    if let Some(kerf) = &features.kerf
        && !(kerf.width.0.is_finite() && kerf.width.0 > 0.)
    {
        return Err(feature("kerf", "the width must be positive"));
    }
    if let Some(joints) = &features.joints {
        joints::check(joints)?;
    }
    if let Some(cooling) = &features.cooling {
        joints::check_cooling(cooling)?;
    }
    if let Some(leads) = &features.leads {
        leads::check(leads)?;
    }
    check_shape_features(features)
}

fn check_shape_features(features: &Features) -> Result<()> {
    if let Some(bridges) = &features.bridges
        && !(bridges.width.0.is_finite() && bridges.width.0 > 0.)
    {
        return Err(feature("bridges", "the width must be positive"));
    }
    if let openlaser_core::features::StartPosition::Manual(fraction) = features.start.position
        && !(0. ..=1.).contains(&fraction)
    {
        return Err(feature("start", "the position is a fraction from 0 to 1"));
    }
    joints::spots_ok("start", &features.start.spots)?;
    if let Seam::Gap(length) | Seam::Overcut(length) = features.seam
        && !(length.0.is_finite() && length.0 >= 0.)
    {
        return Err(feature("seam", "the length must not be negative"));
    }
    Ok(())
}
