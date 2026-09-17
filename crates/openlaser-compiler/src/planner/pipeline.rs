// SPDX-License-Identifier: GPL-3.0-or-later

//! The complete planning pipeline, CADModule `0x1010_BBC0`: slow regions,
//! span boundaries, recursive building and the three post-passes.

use super::corner;
use super::profile::{CRUISE, Input, Profile, construct, construct_compact, transition_distance};
use super::split::build_recursive;
use super::{BUILDER_CALLS, Context, Node, Planned, Settings, Source};
use crate::{Error, Result, finite};

/// How the endpoint speed changes from one node to the next: a step of more
/// than one unit either way, or neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trend {
    Up,
    Down,
    Flat,
}

impl Trend {
    fn of(step: f64) -> Self {
        if step > 1. {
            Self::Up
        } else if step < -1. {
            Self::Down
        } else {
            Self::Flat
        }
    }
}

/// A place the endpoint trend marks as a possible span boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Candidate {
    /// A speed minimum.
    Valley,
    /// A descent levelling off.
    PlateauStart,
    /// A level stretch ascending.
    PlateauEnd,
}

/// Span boundaries from the endpoint trend, `0x1010_9070`: every valley is
/// a boundary, and a plateau reached by a descent is one at both ends when
/// it leads into an ascent. Peaks are not; the span builder's ceiling
/// handles them.
fn boundaries(nodes: &[Node]) -> Result<Vec<usize>> {
    if nodes.len() < 2 {
        return Err(Error::Invalid("a contour needs at least two nodes"));
    }
    let end = nodes.len() - 1;
    let mut candidates = vec![];
    let mut prior = Trend::Flat;
    if end > 2 {
        for i in 1..end {
            let current = Trend::of(finite(nodes[i].endpoint - nodes[i - 1].endpoint)?);
            let candidate = match (prior, current) {
                (Trend::Down, Trend::Up) => Some(Candidate::Valley),
                (Trend::Down, Trend::Flat) => Some(Candidate::PlateauStart),
                (Trend::Flat, Trend::Up) => Some(Candidate::PlateauEnd),
                _ => None,
            };
            if let Some(kind) = candidate {
                candidates.push((i - 1, kind));
            }
            prior = current;
        }
    }
    let mut result = vec![];
    let mut i = 0;
    while i < candidates.len() {
        let (at, kind) = candidates[i];
        let next = candidates.get(i + 1);
        match kind {
            Candidate::Valley => result.push(at),
            Candidate::PlateauStart if next.is_none_or(|n| n.1 == Candidate::PlateauEnd) => {
                result.push(at);
                if let Some(&(plateau_end, _)) = next {
                    result.push(plateau_end);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    result.push(end);
    Ok(result)
}

/// `0x1010_8590`: each span's ending cap is lowered to its larger endpoint
/// when no interior cap stands clear above it. Interior caps are untouched.
fn prepare(nodes: &mut [Node], boundaries: &[usize]) {
    if boundaries.len() <= 2 {
        return;
    }
    for pair in boundaries.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let max_endpoint = nodes[a].endpoint.max(nodes[b].endpoint);
        let mut ceiling = nodes[a].endpoint;
        for row in &nodes[a + 1..=b] {
            ceiling = ceiling.max(row.cap);
        }
        if ceiling < max_endpoint * 1.2 {
            nodes[b].cap = nodes[b].cap.min(max_endpoint);
        }
    }
}

/// Rebuilds a merged input with the context's acceleration and derived jerk.
fn rebuilt(mut input: Input, ctx: Context) -> Result<Profile> {
    input.acceleration = ctx.acceleration;
    input.jerk =
        (((input.ceiling - input.entry.min(input.exit)) * 0.6) / ctx.speed + 0.4) * ctx.jerk;
    construct(input)
}

/// Whether a profile's jerk was raised well above the context's, which is
/// what marks it as demanding.
fn demanding(profile: &Profile, ctx: Context) -> bool {
    profile.jerk >= ctx.jerk * 1.2
}

/// `0x1010_B030`: demanding profiles absorb their neighbours, backward then
/// forward, until the transition between the merged endpoints fits. The
/// neighbour that breaks the run is included.
fn demanding_merge(profiles: Vec<Profile>, ctx: Context) -> Result<Vec<Profile>> {
    if profiles.len() < 2 {
        return Ok(profiles);
    }
    let mut reverse = vec![];
    let mut i = profiles.len();
    while i > 0 {
        let profile = &profiles[i - 1];
        if !demanding(profile, ctx) || profile.entry < profile.exit {
            reverse.push(profile.clone());
            i -= 1;
            continue;
        }
        let mut input = profile.rebuild_input();
        i -= 1;
        while i > 0 {
            let previous = profiles[i - 1].rebuild_input();
            input.span[0] = previous.span[0];
            input.length = finite(input.length + previous.length)?;
            input.entry = previous.entry;
            input.ceiling = previous.ceiling;
            let done = input.entry < input.exit
                || finite(transition_distance(
                    input.exit,
                    input.entry,
                    ctx.acceleration,
                    ctx.jerk,
                ))? < input.length;
            i -= 1;
            if done {
                break;
            }
        }
        reverse.push(rebuilt(input, ctx)?);
    }
    reverse.reverse();
    let mut forward = vec![];
    let mut i = 0;
    while i < reverse.len() {
        let profile = &reverse[i];
        if !demanding(profile, ctx) || profile.exit < profile.entry {
            forward.push(profile.clone());
            i += 1;
            continue;
        }
        let mut input = profile.rebuild_input();
        i += 1;
        while i < reverse.len() {
            let next = reverse[i].rebuild_input();
            input.span[1] = next.span[1];
            input.length = finite(input.length + next.length)?;
            input.ceiling = next.ceiling;
            input.exit = next.exit;
            let done = input.exit < input.entry
                || finite(transition_distance(
                    input.entry,
                    input.exit,
                    ctx.acceleration,
                    ctx.jerk,
                ))? < input.length;
            i += 1;
            if done {
                break;
            }
        }
        forward.push(rebuilt(input, ctx)?);
    }
    Ok(forward)
}

/// `0x1010_7930`: a short profile whose time sits almost entirely in one ramp
/// is classed by that ramp's side: 1 for the entry ramp, -1 for the exit.
fn short_class(profile: &Profile, time: f64) -> Result<i8> {
    let first = profile.entry_ramp_time();
    let last = profile.exit_ramp_time();
    let total = finite(profile.phases[CRUISE].duration + first + last)?;
    if total <= time * 1.5 && (first <= 0.001 || last <= 0.001) {
        if finite(first / total)? > 0.75 {
            return Ok(1);
        }
        if finite(last / total)? > 0.75 {
            return Ok(-1);
        }
    }
    Ok(0)
}

/// Runs of equally classed short profiles, singletons included, become one
/// compact profile over their combined nodes.
fn short_merge(profiles: &[Profile], nodes: &[Node], ctx: Context) -> Result<Vec<Profile>> {
    let classes = profiles.iter().map(|p| short_class(p, ctx.time)).collect::<Result<Vec<_>>>()?;
    let mut out = vec![];
    let mut i = 0;
    while i < profiles.len() {
        if classes[i] == 0 {
            out.push(profiles[i].clone());
            i += 1;
            continue;
        }
        let first = i;
        while i + 1 < profiles.len() && classes[i + 1] == classes[first] {
            i += 1;
        }
        let span = [profiles[first].span()[0], profiles[i].span()[1]];
        let [start, end] = span.map(|word| usize::try_from(word).unwrap_or(usize::MAX));
        if start >= end || end >= nodes.len() {
            return Err(Error::Invalid("merged profile spans are not ordered"));
        }
        let entry = nodes[start].endpoint;
        let exit = nodes[end].endpoint;
        let ceiling = entry.max(exit);
        out.push(construct_compact(Input {
            length: nodes[end].distance - nodes[start].distance,
            entry,
            ceiling,
            exit,
            acceleration: ctx.acceleration,
            jerk: (((ceiling - entry.min(exit)) * 0.6) / ctx.speed + 0.4) * ctx.jerk,
            span,
        })?);
        i += 1;
    }
    Ok(out)
}

/// `0x1010_87A0`: a peak barely above the larger endpoint is flattened onto
/// it, as a full profile when enough of the time is on the peak's side and
/// as a compact one otherwise.
fn reduce_peaks(profiles: &mut [Profile]) -> Result<()> {
    for profile in profiles.iter_mut().rev() {
        let endpoint = profile.entry.max(profile.exit);
        if profile.peak <= 1.33 * endpoint && endpoint + 0.01 <= profile.peak {
            let first = profile.entry_ramp_time();
            let last = profile.exit_ramp_time();
            let cruise = profile.phases[CRUISE].duration;
            let selected = if profile.entry < profile.exit { last } else { first };
            let ratio = finite((selected + cruise) / (last + first + cruise))?;
            let mut input = profile.rebuild_input();
            input.ceiling = endpoint;
            *profile = if 0.2 <= ratio { construct(input)? } else { construct_compact(input)? };
        }
    }
    Ok(())
}

/// The stage chain over prepared nodes. On error the nodes are untouched.
fn plan_profiles(nodes: &mut [Node], ctx: Context) -> Result<(Vec<Profile>, Vec<[usize; 2]>)> {
    let mut working = nodes.to_vec();
    let boundaries = boundaries(&working)?;
    for row in &working {
        if !row.is_finite() || row.step < 0. || row.endpoint < 0. {
            return Err(Error::Invalid("node steps and endpoints must be finite and nonnegative"));
        }
    }
    prepare(&mut working, &boundaries);
    let mut profiles = vec![];
    let mut omitted = vec![];
    let mut calls = 0;
    let mut start = 0;
    let allow_refinement = working.len() < 3;
    for &end in &boundaries {
        if start != end {
            if calls >= BUILDER_CALLS {
                return Err(Error::Budget("profile builder budget exhausted"));
            }
            let part = build_recursive(
                &mut working,
                ctx,
                start,
                end,
                allow_refinement,
                BUILDER_CALLS - calls,
            )?;
            calls += part.calls;
            profiles.extend(part.profiles);
            omitted.extend(part.omitted);
        }
        start = end;
    }
    let profiles = demanding_merge(profiles, ctx)?;
    let mut profiles = short_merge(&profiles, &working, ctx)?;
    reduce_peaks(&mut profiles)?;
    nodes.copy_from_slice(&working);
    Ok((profiles, omitted))
}

/// `0x1010_DA50`: the slow-start and slow-end regions each insert at most one
/// node at their boundary and cap the speeds inside. A leading insertion
/// copies node zero, a trailing one copies its successor.
fn refine_regions(rows: &mut Vec<Node>, settings: &Settings) -> Result<()> {
    if rows.len() < 2 {
        return Ok(());
    }
    let total = rows[rows.len() - 1].distance;
    let (mut leading, mut trailing) = (settings.slow_start_length, settings.slow_end_length);
    if total < trailing + leading {
        leading = total * 0.5;
        trailing = leading;
    }
    refine_leading(rows, settings.slow_start_speed, leading)?;
    refine_trailing(rows, settings.slow_end_speed, trailing, total)?;
    if rows.iter().any(|r| !r.is_finite() || r.step < 0.) {
        return Err(Error::Invalid("refined nodes must be finite with nonnegative steps"));
    }
    Ok(())
}

fn refine_leading(rows: &mut Vec<Node>, speed: f64, leading: f64) -> Result<()> {
    if leading >= 0.1
        && let Some(at) = (1..rows.len()).find(|&i| rows[i].distance + 0.01 > leading)
    {
        let remainder = finite(rows[at].distance - leading)?;
        if remainder.abs() > 0.05 {
            let mut inserted = rows[0];
            inserted.cap = inserted.cap.min(speed);
            inserted.endpoint = inserted.cap;
            inserted.step = finite(leading - rows[at - 1].distance)?;
            inserted.distance = leading;
            rows[at].step = remainder;
            rows.insert(at, inserted);
        }
        for row in &mut rows[..=at] {
            row.cap = row.cap.min(speed);
            row.endpoint = row.endpoint.min(speed);
        }
    }
    Ok(())
}

fn refine_trailing(rows: &mut Vec<Node>, speed: f64, trailing: f64, total: f64) -> Result<()> {
    if trailing >= 0.1
        && let Some(at) = (0..rows.len() - 1).rev().find(|&i| total - rows[i].distance > trailing)
    {
        if ((total - rows[at].distance) - trailing).abs() > 0.05 {
            let split = finite(total - trailing)?;
            let mut inserted = rows[at + 1];
            inserted.step = finite(split - rows[at].distance)?;
            inserted.distance = split;
            rows[at + 1].step = finite(rows[at + 1].distance - split)?;
            rows.insert(at + 1, inserted);
        }
        for row in rows.iter_mut().skip(at + 2) {
            row.cap = row.cap.min(speed);
            row.endpoint = row.endpoint.min(speed);
        }
    }
    Ok(())
}

/// Plans `sources` under `settings`: nodes with corner speeds, slow regions,
/// and the profiles covering them. Profile spans refer to the returned nodes.
pub fn plan(sources: &[Source], settings: Settings) -> Result<Planned> {
    if sources.len() < 2 || sources.len() > 1_000_000 {
        return Err(Error::Invalid("a contour needs 2 to 1 000 000 source records"));
    }
    let settings = settings.normalized()?;
    let nodes = corner::nodes(sources, &settings)?;
    finish(nodes, settings)
}

/// A continuation already has speed ceilings on its execution intervals.
/// They may be below the native corner-cap repair threshold, so they enter
/// the same profile pipeline directly, without evaluating corners again.
pub(crate) fn plan_speeds(speeds: &[(f64, f64)], settings: Settings) -> Result<Planned> {
    if !(2..=1_000_000).contains(&speeds.len()) {
        return Err(Error::Budget("continuation speed nodes"));
    }
    let settings = settings.normalized()?;
    let mut nodes = Vec::with_capacity(speeds.len());
    for (i, &(distance, cap)) in speeds.iter().enumerate() {
        let step = distance - speeds[i.saturating_sub(1)].0;
        if !distance.is_finite() || !cap.is_finite() || step < 0. || cap <= 0. {
            return Err(Error::Invalid("continuation speed intervals"));
        }
        let endpoint = if i == 0 || i + 1 == speeds.len() { 0. } else { cap.min(speeds[i + 1].1) };
        nodes.push(Node {
            step,
            endpoint: endpoint.min(settings.speed),
            cap: cap.min(settings.speed),
            acceleration: settings.acceleration,
            distance,
        });
    }
    finish(nodes, settings)
}

fn finish(mut nodes: Vec<Node>, settings: Settings) -> Result<Planned> {
    refine_regions(&mut nodes, &settings)?;
    let (profiles, omitted) = plan_profiles(&mut nodes, settings.context()?)?;
    if !omitted.is_empty() {
        return Err(Error::Invalid("the planner left a source range uncovered"));
    }
    Ok(Planned { settings, nodes, profiles })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(endpoints: &[f64]) -> Vec<Node> {
        endpoints
            .iter()
            .enumerate()
            .map(|(i, &endpoint)| Node {
                step: if i == 0 { 0. } else { 100. },
                endpoint,
                cap: 100.,
                acceleration: 0.,
                distance: crate::float(i) * 100.,
            })
            .collect()
    }

    fn context() -> Context {
        Context { acceleration: 100., jerk: 1000., speed: 100., time: 0.01 }
    }

    fn settings() -> Settings {
        Settings {
            acceleration: 100.,
            acceleration_time: 0.1,
            spline_accuracy: 0.,
            speed: 100.,
            corner_speed_floor: 0.,
            interval_ms: 0.,
            slow_start_length: 0.,
            slow_start_speed: 20.,
            slow_end_length: 0.,
            slow_end_speed: 30.,
        }
    }

    /// A valley marks one boundary, a plateau after a descent keeps both of
    /// its ends, and a slope within the ±1 threshold marks none.
    #[test]
    fn valley_and_plateau_boundaries_preserve_native_pairing() {
        assert_eq!(boundaries(&nodes(&[10., 5., 10., 20.])).unwrap(), vec![1, 3]);
        assert_eq!(boundaries(&nodes(&[10., 5., 5., 10., 20.])).unwrap(), vec![1, 2, 4]);
        assert_eq!(boundaries(&nodes(&[10., 9., 10., 20.])).unwrap(), vec![3]);
    }

    /// A peak is not a boundary, and a plateau reached by an ascent is not
    /// either.
    #[test]
    fn peaks_and_ascending_plateaus_are_not_boundaries() {
        assert_eq!(boundaries(&nodes(&[0., 10., 20., 10., 0.])).unwrap(), vec![4]);
        assert_eq!(boundaries(&nodes(&[0., 10., 10., 20., 30.])).unwrap(), vec![4]);
    }

    /// Preparation lowers only the fixed ending cap of a span.
    #[test]
    fn preparation_only_changes_the_ending_cap() {
        let mut rows = nodes(&[100., 100., 100., 100., 100.]);
        rows[2].cap = 110.;
        rows[3].cap = 110.;
        prepare(&mut rows, &[1, 3, 4]);
        assert_eq!(rows[2].cap, 110.);
        assert_eq!(rows[3].cap, 100.);
    }

    /// The whole chain covers the nodes in order without gaps.
    #[test]
    fn stage_chain_keeps_ordered_coverage() {
        let mut rows = nodes(&[0., 10., 0.]);
        let (profiles, omitted) = plan_profiles(&mut rows, context()).unwrap();
        assert!(omitted.is_empty());
        assert_eq!(profiles[0].span()[0], 0);
        assert_eq!(profiles[profiles.len() - 1].span()[1], 2);
        assert_eq!(profiles.iter().map(Profile::length).sum::<f64>(), 200.);
        for pair in profiles.windows(2) {
            assert_eq!(pair[0].span()[1], pair[1].span()[0]);
        }
    }

    /// A later span failure discards the one working buffer, including
    /// mutations made by successful earlier spans.
    #[test]
    fn failed_later_span_does_not_commit_earlier_nodes() {
        let mut rows = nodes(&[100., 95., 100., 95., 100., 95., 100., 95.]);
        let mut first = rows[..7].to_vec();
        plan_profiles(&mut first, context()).unwrap();
        assert_ne!(first, rows[..7]);
        rows[7].distance = -1000.;
        let before = rows.clone();
        assert!(plan_profiles(&mut rows, context()).is_err());
        assert_eq!(rows, before);
    }

    /// A peak just above its endpoints is flattened onto them.
    #[test]
    fn near_endpoint_peak_is_reduced() {
        let mut profile = construct(Input {
            length: 100.,
            entry: 10.,
            ceiling: 12.,
            exit: 10.,
            acceleration: 100.,
            jerk: 1000.,
            span: [0, 1],
        })
        .unwrap();
        reduce_peaks(std::slice::from_mut(&mut profile)).unwrap();
        assert_eq!(profile.peak(), 10.);
    }

    /// Both slow regions insert a node at their boundary, keep the total
    /// distance and cap the speeds inside their region.
    #[test]
    fn slow_regions_insert_nodes_and_cap_speeds() {
        let mut rows = nodes(&[0., 80., 0.]);
        let mut s = settings();
        s.slow_start_length = 20.;
        s.slow_end_length = 20.;
        refine_regions(&mut rows, &s).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.distance).collect::<Vec<_>>(),
            vec![0., 20., 100., 180., 200.]
        );
        assert_eq!(rows.iter().map(|r| r.step).collect::<Vec<_>>(), vec![0., 20., 80., 80., 20.]);
        assert_eq!(rows[1].endpoint, 20.);
        assert_eq!(rows[3].cap, 100.);
        assert_eq!(rows[4].cap, 30.);
    }

    /// A slow-end boundary within 0.05 mm of an existing node inserts
    /// nothing and leaves that node's cap alone, as the vendor does.
    #[test]
    fn near_tail_boundary_keeps_the_native_skipped_node() {
        let mut rows = nodes(&[0., 80., 0.]);
        let mut s = settings();
        s.slow_end_length = 99.98;
        refine_regions(&mut rows, &s).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].cap, 100.);
        assert_eq!(rows[2].cap, 100.);
    }

    /// The complete planner zeroes both end speeds and covers the length.
    #[test]
    fn source_to_profile_pipeline_is_connected() {
        let sources = [
            Source::plain(0., 1., 100.).unwrap(),
            Source::plain(100., 1., 100.).unwrap(),
            Source::plain(200., 1., 100.).unwrap(),
        ];
        let planned = plan(&sources, settings()).unwrap();
        assert_eq!(planned.nodes[0].endpoint, 0.);
        assert_eq!(planned.nodes[planned.nodes.len() - 1].endpoint, 0.);
        assert_eq!(planned.profiles.iter().map(Profile::length).sum::<f64>(), 200.);
    }
}
