// SPDX-License-Identifier: GPL-3.0-or-later

//! Manual locations follow retained geometry through bridge joins/splits.
//! Generated geometry is never assigned a guessed source interval.

use crate::bridge::{Sourced, nearest_parameter, project};
use crate::{EPS, Result, feature};
use openlaser_core::features::{CoolingPlacement, Features, JointPlacement, LeadOverride, Spot};
use openlaser_core::geometry::{Curve, Drawing};
use openlaser_core::toolpath::SourceInterval;
use std::collections::BTreeMap;

pub(crate) struct Remapped {
    pub features: Features,
    /// Result index to the original edit, retaining its stable source anchor.
    pub lead_overrides: BTreeMap<usize, LeadOverride>,
}

/// Rebind manual spots to the actual bridge results before direction and
/// start rotation are applied. Missing or ambiguous locations are errors.
pub(crate) fn remap(
    drawing: &Drawing,
    results: &[Sourced],
    features: &Features,
) -> Result<Remapped> {
    let mut projection = Projection::new(drawing, results, features);
    let mut mapped = features.clone();
    mapped.start.spots = projection.spots("start", &features.start.spots)?;
    for (i, spot) in mapped.start.spots.iter().enumerate() {
        if mapped.start.spots[..i].iter().any(|other| other.contour == spot.contour) {
            return Err(feature(
                "start",
                "a joined contour has more than one manual start; choose one",
            ));
        }
    }
    if let Some(joints) = &mut mapped.joints
        && let JointPlacement::Manual(picked) = &mut joints.placement
    {
        *picked = projection.spots("joints", picked)?;
    }
    if let Some(cooling) = &mut mapped.cooling
        && let CoolingPlacement::Manual(picked) = &mut cooling.placement
    {
        *picked = projection.spots("cooling", picked)?;
    }
    let lead_overrides = projection.lead_overrides()?;
    Ok(Remapped { features: mapped, lead_overrides })
}

struct Projection<'a> {
    drawing: &'a Drawing,
    results: &'a [Sourced],
    features: &'a Features,
    owners: BTreeMap<usize, Vec<usize>>,
    work: usize,
}

impl<'a> Projection<'a> {
    fn new(drawing: &'a Drawing, results: &'a [Sourced], features: &'a Features) -> Self {
        let mut owners = BTreeMap::<usize, Vec<usize>>::new();
        for (index, result) in results.iter().enumerate() {
            for &source in &result.sources {
                owners.entry(source).or_default().push(index);
            }
        }
        Self { drawing, results, features, owners, work: 10_000_000 }
    }

    fn spots(&mut self, name: &'static str, picked: &[Spot]) -> Result<Vec<Spot>> {
        let mut mapped = Vec::with_capacity(picked.len());
        for spot in picked {
            let source = self.drawing.contours.get(spot.contour).ok_or_else(|| {
                feature(name, format!("contour {} no longer exists", spot.contour))
            })?;
            if self.features.skip_layers.contains(&source.layer) {
                continue;
            }
            let owned = self.owners.get(&spot.contour).map_or(&[][..], Vec::as_slice);
            // Length and projection each visit these curves. Charge only the
            // owners we actually inspect, so selecting the whole sheet stays
            // proportional to its geometry instead of the number of pairs.
            let curves = owned.iter().fold(source.curves.len(), |count, &index| {
                count.saturating_add(self.results[index].contour.curves.len())
            });
            self.work = self
                .work
                .checked_sub(curves.saturating_mul(2))
                .ok_or(crate::Error::Budget("manual feature projection"))?;
            let at = source
                .point_at(source.length() * spot.fraction)
                .ok_or_else(|| feature(name, "the source location is outside its contour"))?;
            let candidates: Vec<_> = owned
                .iter()
                .filter_map(|&contour| {
                    let result = &self.results[contour];
                    let (nearest, fraction) = project(&result.contour, at)?;
                    (nearest.distance(at) <= EPS).then_some(Spot { contour, fraction })
                })
                .collect();
            if candidates.len() != 1 {
                return Err(feature(
                    name,
                    format!(
                        "the point on contour {} at {:.4} was removed or made ambiguous by a bridge",
                        spot.contour, spot.fraction
                    ),
                ));
            }
            mapped.push(candidates[0]);
        }
        Ok(mapped)
    }

    fn lead_overrides(&mut self) -> Result<BTreeMap<usize, LeadOverride>> {
        let mut lead_overrides = BTreeMap::new();
        if let Some(leads) = &self.features.leads {
            for edited in &leads.overrides {
                for target in self.spots("leads", std::slice::from_ref(&edited.location))? {
                    if lead_overrides.insert(target.contour, edited.clone()).is_some() {
                        return Err(feature(
                            "leads",
                            "a joined contour has more than one lead override; reset one before joining",
                        ));
                    }
                }
            }
        }
        Ok(lead_overrides)
    }
}

/// A point on retained source geometry identifies this piece even after
/// compensation, reversal, ordering, common-edge splitting or a later move.
pub(crate) fn lead_target(
    contour: &Sourced,
    drawing: &Drawing,
    work: &mut usize,
) -> Result<Option<Spot>> {
    for curve in &contour.contour.curves {
        if let Some(source) = interval(curve, &contour.sources, drawing, work)? {
            return Ok(Some(Spot {
                contour: source.contour,
                fraction: ((source.start + source.end) * 0.5).clamp(0., 1.),
            }));
        }
    }
    Ok(None)
}

/// Record the exact original interval of a retained line/arc. Kerf offsets,
/// leads and channel edges that no longer lie on a source curve stay generated.
pub(crate) fn interval(
    curve: &Curve,
    sources: &[usize],
    drawing: &Drawing,
    work: &mut usize,
) -> Result<Option<SourceInterval>> {
    let mut found = None;
    for &source in sources {
        let original = &drawing.contours[source];
        let total = original.length();
        let mut offset = 0.;
        for original_curve in &original.curves {
            *work = work.checked_sub(1).ok_or(crate::Error::Budget("source interval matching"))?;
            let length = original_curve.length();
            let mut start = nearest_parameter(original_curve, curve.start());
            let sign =
                if original_curve.tangent(start).dot(curve.tangent(0.)) < 0. { -1. } else { 1. };
            let mut end = start + sign * curve.length() / length;
            if end < -EPS && original_curve.start().distance(original_curve.end()) < EPS {
                start += 1.;
                end += 1.;
            }
            if (-EPS..=1. + EPS).contains(&start)
                && (-EPS..=1. + EPS).contains(&end)
                && [0., 0.5, 1.].iter().all(|&t| {
                    original_curve.point(start + (end - start) * t).distance(curve.point(t)) <= EPS
                })
            {
                let interval = SourceInterval {
                    contour: source,
                    start: (offset + length * start) / total,
                    end: (offset + length * end) / total,
                };
                if found.is_some_and(|prior| prior != interval) {
                    return Err(feature("provenance", "a segment has ambiguous source ownership"));
                }
                found = Some(interval);
            }
            offset += length;
        }
    }
    Ok(found)
}
