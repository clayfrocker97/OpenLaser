// SPDX-License-Identifier: GPL-3.0-or-later

//! Common edges retain the selected contours' order and direction. Split at
//! shared endpoints, keep the first compatible span, and break a later cut
//! wherever a span was removed. No tolerance-sized laser-on link is invented.

use crate::{EPS, Error, Result, feature};
use openlaser_core::features::CommonEdges;
use openlaser_core::geometry::Curve;
use openlaser_core::toolpath::{PreparedContour, PreparedSegment, SegmentProcess};

pub(crate) fn check(common: &CommonEdges, count: usize) -> Result<()> {
    if !(common.tolerance.is_finite() && common.tolerance > 0. && common.tolerance <= 1.) {
        return Err(feature("common edges", "the tolerance must be above 0 and at most 1 mm"));
    }
    if common.contours.len() < 2 || common.contours.iter().any(|&i| i >= count) {
        return Err(feature("common edges", "select at least two existing contours"));
    }
    if common.contours.iter().enumerate().any(|(i, c)| common.contours[..i].contains(c)) {
        return Err(feature("common edges", "each contour must be selected once"));
    }
    Ok(())
}

fn spend(work: &mut usize) -> Result<()> {
    *work = work.checked_sub(1).ok_or(Error::Budget("common edge comparisons"))?;
    Ok(())
}

fn same(a: Curve, b: Curve, tolerance: f64) -> Option<bool> {
    // A line and an arc must never become interchangeable at a coarse tolerance.
    if std::mem::discriminant(&a) != std::mem::discriminant(&b) {
        return None;
    }
    [false, true].into_iter().find(|&reverse| {
        [0., 0.25, 0.5, 0.75, 1.]
            .iter()
            .all(|&t| a.point(t).distance(b.point(if reverse { 1. - t } else { t })) <= tolerance)
    })
}

fn atoms(
    segment: &PreparedSegment,
    others: &[Curve],
    tolerance: f64,
    work: &mut usize,
) -> Result<Vec<PreparedSegment>> {
    if segment.process.lead.is_some() {
        return Ok(vec![segment.clone()]);
    }
    let mut cuts = vec![0., 1.];
    for other in others {
        spend(work)?;
        if std::mem::discriminant(&segment.curve) != std::mem::discriminant(other) {
            continue;
        }
        for point in [other.start(), other.end()] {
            let t = crate::bridge::nearest_parameter(&segment.curve, point);
            if segment.curve.point(t).distance(point) <= tolerance && t > EPS && t < 1. - EPS {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
    Ok(cuts
        .windows(2)
        .filter_map(|pair| {
            let mut part = segment.slice(pair[0], pair[1]);
            if pair[0] > EPS {
                part.process.cool_ms = 0;
                part.process.repierce = false;
            }
            (part.curve.length() > EPS).then_some(part)
        })
        .collect())
}

fn compatible(a: &SegmentProcess, b: &SegmentProcess, reverse: bool) -> bool {
    a.joint == b.joint
        && a.power == b.power
        && a.speed == b.speed
        && a.lead.is_none()
        && b.lead.is_none()
        && if reverse {
            a.cool_ms == 0 && b.cool_ms == 0 && !a.repierce && !b.repierce
        } else {
            a.cool_ms == b.cool_ms && a.repierce == b.repierce
        }
}

/// Removes repeated spans only across distinct selected contours with the
/// same layer and nesting depth. Film is deduplicated independently of joints.
pub(crate) fn apply(
    contours: &[PreparedContour],
    common: &CommonEdges,
) -> Result<Vec<PreparedContour>> {
    let selected: Vec<_> =
        contours.iter().map(|c| c.sources.iter().all(|s| common.contours.contains(s))).collect();
    let curves: Vec<_> = contours
        .iter()
        .zip(&selected)
        .filter(|(_, yes)| **yes)
        .flat_map(|(c, _)| c.segments.iter().filter(|s| s.process.lead.is_none()).map(|s| s.curve))
        .collect();
    if curves.len() > 10_000 {
        return Err(Error::Budget("common edge segments"));
    }
    let mut dedup = Deduplication {
        contours,
        tolerance: common.tolerance,
        work: 10_000_000,
        seen: Vec::new(),
        groups: (0..contours.len()).map(|i| vec![i]).collect(),
        shared: 0,
    };
    let mut output = Vec::new();
    for (i, contour) in contours.iter().enumerate() {
        if !selected[i] {
            output.push((i, contour.clone()));
            continue;
        }
        let mut split = Vec::new();
        for segment in &contour.segments {
            split.extend(atoms(segment, &curves, common.tolerance, &mut dedup.work)?);
        }
        let mut keep = dedup.retained(i, &split)?;
        if common.allow_overcut {
            retain_between_ends(&mut keep);
        }
        output.extend(runs(contour, split, keep).into_iter().map(|path| (i, path)));
    }
    if dedup.shared == 0 {
        return Err(feature(
            "common edges",
            "the selected contours have no compatible shared spans after preparation",
        ));
    }
    film(contours, &selected, &mut output, common.tolerance, &mut dedup.work)?;
    dedup.assign_sources(&mut output, &selected);
    Ok(output.into_iter().map(|(_, c)| c).collect())
}

struct Deduplication<'a> {
    contours: &'a [PreparedContour],
    tolerance: f64,
    work: usize,
    seen: Vec<(usize, PreparedSegment)>,
    groups: Vec<Vec<usize>>,
    shared: usize,
}

impl Deduplication<'_> {
    fn duplicate(&mut self, i: usize, segment: &PreparedSegment) -> Result<Option<usize>> {
        for (owner, prior) in &self.seen {
            spend(&mut self.work)?;
            if *owner == i || segment.process.lead.is_some() || prior.process.lead.is_some() {
                continue;
            }
            if let Some(reverse) = same(prior.curve, segment.curve, self.tolerance) {
                if self.contours[*owner].layer != self.contours[i].layer
                    || self.contours[*owner].depth != self.contours[i].depth
                    || !compatible(&prior.process, &segment.process, reverse)
                {
                    return Err(feature(
                        "common edges",
                        "a shared span has incompatible layer, depth, joint or cooling treatment",
                    ));
                }
                return Ok(Some(*owner));
            }
        }
        Ok(None)
    }

    fn retained(&mut self, i: usize, split: &[PreparedSegment]) -> Result<Vec<bool>> {
        let mut keep = Vec::with_capacity(split.len());
        for segment in split {
            let duplicate = self.duplicate(i, segment)?;
            keep.push(duplicate.is_none());
            if let Some(owner) = duplicate {
                self.shared += 1;
                self.join(i, owner);
            } else {
                self.seen.push((i, segment.clone()));
            }
        }
        Ok(keep)
    }

    fn join(&mut self, i: usize, owner: usize) {
        let a = self.groups.iter().position(|g| g.contains(&i));
        let b = self.groups.iter().position(|g| g.contains(&owner));
        if let (Some(a), Some(b)) = (a, b)
            && a != b
        {
            let taken = self.groups.remove(b);
            self.groups[if a > b { a - 1 } else { a }].extend(taken);
        }
    }

    fn assign_sources(&self, output: &mut [(usize, PreparedContour)], selected: &[bool]) {
        for (owner, path) in output {
            if !selected[*owner] {
                continue;
            }
            if let Some(group) = self.groups.iter().find(|g| g.contains(owner)) {
                path.sources =
                    group.iter().flat_map(|i| self.contours[*i].sources.iter().copied()).collect();
                path.sources.sort_unstable();
                path.sources.dedup();
            }
        }
    }
}

/// Retracing stays between surviving segments of the same original path.
/// Leading and trailing repeated spans are always removed.
fn retain_between_ends(keep: &mut [bool]) {
    if let (Some(first), Some(last)) = (keep.iter().position(|v| *v), keep.iter().rposition(|v| *v))
    {
        keep[first..=last].fill(true);
    }
}

fn runs(
    contour: &PreparedContour,
    split: Vec<PreparedSegment>,
    keep: Vec<bool>,
) -> Vec<PreparedContour> {
    let mut output = Vec::new();
    let mut run: Vec<PreparedSegment> = Vec::new();
    let mut flush = |run: &mut Vec<PreparedSegment>| {
        if run.is_empty() {
            return;
        }
        // A lead attached to a removed edge cannot stand alone or acquire
        // a new role in the middle of another path.
        if run.iter().all(|s| s.process.lead.is_some()) {
            run.clear();
            return;
        }
        let closed = run
            .first()
            .zip(run.last())
            .is_some_and(|(a, b)| a.curve.start().distance(b.curve.end()) < EPS);
        output.push(PreparedContour {
            segments: std::mem::take(run),
            closed,
            film: Vec::new(),
            ..contour.clone()
        });
    };
    for (segment, keep) in split.into_iter().zip(keep) {
        if !keep {
            flush(&mut run);
            continue;
        }
        if run.last().is_some_and(|prior| prior.curve.end().distance(segment.curve.start()) > EPS) {
            flush(&mut run);
        }
        run.push(segment);
    }
    flush(&mut run);
    output
}

fn film(
    contours: &[PreparedContour],
    selected: &[bool],
    output: &mut [(usize, PreparedContour)],
    tolerance: f64,
    work: &mut usize,
) -> Result<()> {
    let film_curves: Vec<_> = contours
        .iter()
        .zip(selected)
        .filter(|(_, yes)| **yes)
        .flat_map(|(c, _)| c.film.iter().copied())
        .collect();
    let mut seen_film: Vec<Curve> = Vec::new();
    for (i, contour) in contours.iter().enumerate().filter(|(i, _)| selected[*i]) {
        let mut film = Vec::new();
        for curve in &contour.film {
            for part in atoms(
                &PreparedSegment::new(*curve, SegmentProcess::default()),
                &film_curves,
                tolerance,
                work,
            )? {
                let mut duplicate = false;
                for prior in &seen_film {
                    spend(work)?;
                    if same(*prior, part.curve, tolerance).is_some() {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    seen_film.push(part.curve);
                    film.push(part.curve);
                }
            }
        }
        if let Some((_, first)) = output.iter_mut().find(|(owner, _)| *owner == i) {
            first.film = film;
        }
    }
    Ok(())
}
