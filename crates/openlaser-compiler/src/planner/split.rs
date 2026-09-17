// SPDX-License-Identifier: GPL-3.0-or-later

//! Covering a span between speed extrema with profiles.
//!
//! A span is first built as one profile (`0x1010_8720`, `0x1010_8920`,
//! `0x1010_7B10`). If an interior node's speed would be exceeded by that
//! profile's ramps or cruise, the span is split there and both halves are
//! built again (`0x1010_8070`, `0x1010_82E0`, `0x1010_8B80`, `0x1010_A890`).
//! The vendor recurses without a depth ceiling; we bound the recursion and
//! the number of builds.

use super::profile::{
    ACCELERATE, CRUISE, DECELERATE, ENTRY_JERK_IN, ENTRY_JERK_OUT, EXIT_JERK_IN, EXIT_JERK_OUT,
    Input, Phase, Profile, construct,
};
use super::{BUILDER_CALLS, Context, Node, SPLIT_DEPTH};
use crate::{Error, Result, finite, index};

/// The speed margin, as a fraction, an interior node may be exceeded by.
const MARGIN: f64 = 0.05;

/// A built span.
pub(super) struct Built {
    pub profile: Profile,
    /// Whether the vendor's builder reported success. After endpoint
    /// refinement this is the refinement's orientation, a vendor quirk.
    pub native_return: bool,
}

/// Bisection on `(low + x)(x² - low²) = span · jerk · span`, the endpoint
/// refinement of `0x1010_8920`. Endpoint checks are strict, the midpoint
/// check inclusive, as in the vendor.
fn refine_endpoint(
    low: f64,
    jerk: f64,
    span: f64,
    left: f64,
    right: f64,
    tolerance: f64,
) -> Result<f64> {
    let square = finite(low * low)?;
    let target = finite(span * jerk * span)?;
    let eval = |x: f64| finite((low + x) * (x * x - square) - target);
    super::root::solve(eval, left, right, tolerance, "endpoint refinement did not converge")
}

/// The jerk a span is built with: raised above the context's by how far the
/// ceiling sits above the lower endpoint, relative to the speed cap.
fn span_jerk(ceiling: f64, lower_endpoint: f64, ctx: Context) -> f64 {
    (((ceiling - lower_endpoint) * 0.6) / ctx.speed + 0.4) * ctx.jerk
}

/// Builds one profile over `nodes[start..=end]`. Endpoint speeds change only
/// through refinement; on error the nodes are untouched.
pub(super) fn build_span(
    nodes: &mut [Node],
    ctx: Context,
    start: usize,
    end: usize,
    allow_refinement: bool,
) -> Result<Built> {
    check_span(nodes, ctx, start, end)?;
    let mut ceiling = nodes[start].endpoint;
    for row in &nodes[start + 1..=end] {
        if ceiling < row.cap {
            ceiling = row.cap;
        }
    }
    let span = [index(start)?, index(end)?];
    let (entry, exit) = (nodes[start].endpoint, nodes[end].endpoint);
    let mut input = Input {
        length: nodes[end].distance - nodes[start].distance,
        entry,
        ceiling,
        exit,
        acceleration: ctx.acceleration,
        jerk: span_jerk(ceiling, entry.min(exit), ctx),
        span,
    };
    let mut profile = construct(input)?;
    let mut native_return = true;
    if !profile.native_return && 1.2 <= profile.jerk / ctx.jerk && allow_refinement {
        let reverse = profile.exit < profile.entry;
        let low = profile.entry.min(profile.exit);
        let high = profile.entry.max(profile.exit);
        let value = refine_endpoint(
            low,
            profile.jerk,
            profile.length,
            low,
            high,
            (profile.peak * 0.01).min(5.),
        )?;
        input = Input {
            length: profile.length,
            entry: if reverse { value } else { profile.entry },
            ceiling: profile.peak,
            exit: if reverse { profile.exit } else { value },
            acceleration: ctx.acceleration,
            jerk: ctx.jerk,
            span,
        };
        profile = construct(input)?;
        nodes[start].endpoint = input.entry;
        nodes[end].endpoint = input.exit;
        native_return = !reverse;
    }
    Ok(Built { profile, native_return })
}

fn check_span(nodes: &[Node], ctx: Context, start: usize, end: usize) -> Result<()> {
    if start >= end || end >= nodes.len() {
        return Err(Error::Invalid("span indices are not ordered within the nodes"));
    }
    for v in [ctx.acceleration, ctx.jerk, ctx.speed] {
        if !v.is_finite() || v <= 0. {
            return Err(Error::Invalid("planner context must be positive"));
        }
    }
    for row in &nodes[start..=end] {
        if !row.is_finite() || row.endpoint < 0. {
            return Err(Error::Invalid("node fields must be finite with nonnegative endpoints"));
        }
    }
    Ok(())
}

/// Speed reached after `distance` of constant jerk `j` from `low`, through the
/// vendor's cubic solution; `None` when the cube roots order the wrong way.
fn cubic_velocity(low: f64, j: f64, distance: f64) -> Result<Option<f64>> {
    let p = 6. * low / j;
    let q = -6. * distance / j;
    let discriminant = finite((p * p * p) / 27. + q * q * 0.25)?;
    let root = finite(discriminant.sqrt())?;
    // The vendor raises to the binary64 third, not a sign-preserving cube root.
    let time = finite((root - q * 0.5).powf(1. / 3.) - (root + q * 0.5).powf(1. / 3.))?;
    if time < 0. { Ok(None) } else { Ok(Some(finite(j * time * time * 0.5 + low)?)) }
}

/// One ramp as the split walk sees it: the boundary speed it starts from, its
/// three phases in walking order, and the divisor the vendor recovers its
/// constant acceleration with. Walking the exit ramp backwards the vendor
/// divides by the constant-deceleration time where the forward walk divides
/// by the jerk-out time; that mismatch is kept.
struct RampView {
    boundary: f64,
    first: Phase,
    middle: Phase,
    last: Phase,
    divisor: f64,
}

impl Profile {
    fn ramp_view(&self, reverse: bool) -> RampView {
        let p = &self.phases;
        if reverse {
            RampView {
                boundary: self.exit,
                first: p[EXIT_JERK_OUT],
                middle: p[DECELERATE],
                last: p[EXIT_JERK_IN],
                divisor: p[DECELERATE].duration,
            }
        } else {
            RampView {
                boundary: self.entry,
                first: p[ENTRY_JERK_IN],
                middle: p[ACCELERATE],
                last: p[ENTRY_JERK_OUT],
                divisor: p[ENTRY_JERK_OUT].duration,
            }
        }
    }
}

/// The first interior node whose speed the profile's ramp exceeds, walking
/// forward over the entry ramp or backward over the exit ramp.
fn ramp_split(
    nodes: &[Node],
    profile: &Profile,
    start: usize,
    end: usize,
    margin: f64,
    reverse: bool,
) -> Result<Option<usize>> {
    let ramp = RampEvaluator::new(profile, reverse)?;
    let mut distance = 0.;
    for step in 1..end - start {
        let i = if reverse { end - step } else { start + step };
        distance = finite(distance + nodes[if reverse { i + 1 } else { i }].step)?;
        let v = nodes[i].endpoint;
        let threshold =
            finite(if reverse { margin * v + v + 8. } else { v + (v * margin).max(1.) })?;
        match ramp.exceeds(distance, threshold)? {
            RampDecision::Select => return Ok(Some(i)),
            RampDecision::End => return Ok(None),
            RampDecision::Continue => {}
        }
    }
    Ok(None)
}

#[derive(Clone, Copy)]
enum RampDecision {
    Continue,
    Select,
    End,
}

impl RampDecision {
    fn candidate(value: Option<f64>, threshold: f64) -> Self {
        if value.is_some_and(|value| value > threshold) { Self::Select } else { Self::Continue }
    }
}

struct RampEvaluator {
    ramp: RampView,
    peak: f64,
    acceleration: f64,
    jerk: f64,
    v0: f64,
    v2: f64,
    middle_end: f64,
    total: f64,
    reverse: bool,
}

impl RampEvaluator {
    fn new(profile: &Profile, reverse: bool) -> Result<Self> {
        let (peak, acceleration, jerk) = (profile.peak, profile.acceleration, profile.jerk);
        let ramp = profile.ramp_view(reverse);
        let v0 = finite(jerk * ramp.first.duration * ramp.first.duration * 0.5 + ramp.boundary)?;
        let v2 = finite(peak - ramp.last.duration * jerk * ramp.last.duration * 0.5)?;
        let d0 = ramp.first.distance;
        let b = finite(d0 + ramp.middle.distance)?;
        let total = finite(b + ramp.last.distance)?;
        Ok(Self { ramp, peak, acceleration, jerk, v0, v2, middle_end: b, total, reverse })
    }

    fn exceeds(&self, distance: f64, threshold: f64) -> Result<RampDecision> {
        let candidate = if distance < self.ramp.first.distance {
            if threshold < self.v0 {
                cubic_velocity(self.ramp.boundary, self.jerk, distance)?
            } else {
                None
            }
        } else if distance < self.middle_end {
            if self.reverse && self.peak <= threshold {
                None
            } else {
                Some(finite(
                    (self.v0 * self.v0
                        + 2. * self.acceleration * (distance - self.ramp.first.distance))
                        .sqrt(),
                )?)
            }
        } else {
            return self.last_phase(distance, threshold);
        };
        Ok(RampDecision::candidate(candidate, threshold))
    }

    fn last_phase(&self, distance: f64, threshold: f64) -> Result<RampDecision> {
        if self.total <= distance {
            return Ok(RampDecision::End);
        }
        if self.peak <= threshold {
            return Ok(RampDecision::Continue);
        }
        if self.reverse && self.ramp.divisor == 0. {
            // Preserve the native greater-or-unordered split decision; no
            // non-finite value enters the planned profile.
            let a = (self.peak - self.v2) / self.ramp.divisor;
            let velocity = (self.v2 * self.v2 + (a + a) * (distance - self.middle_end)).sqrt();
            return Ok(if velocity > threshold || velocity.is_nan() {
                RampDecision::Select
            } else {
                RampDecision::Continue
            });
        }
        let a = finite((self.peak - self.v2) / self.ramp.divisor)?;
        let velocity = finite((self.v2 * self.v2 + (a + a) * (distance - self.middle_end)).sqrt())?;
        Ok(RampDecision::candidate(Some(velocity), threshold))
    }
}

/// Where a span splits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    None,
    /// Build the left part directly, recurse into the right.
    First(usize),
    /// Recurse into both parts.
    Both(usize),
    /// Recurse into the left part, build the right directly.
    Last(usize),
}

/// The vendor's ordered selectors: the exit ramp first when the exit speed
/// is at most the entry speed, otherwise the entry ramp; then the cruise.
fn select_split(nodes: &[Node], profile: &Profile, start: usize, end: usize) -> Result<Split> {
    if end - start < 2 {
        return Ok(Split::None);
    }
    check_split_nodes(&nodes[start..=end])?;
    let reverse_first = profile.exit <= profile.entry;
    if let Some(i) = ramp_split(nodes, profile, start, end, MARGIN * 1.25, reverse_first)? {
        return Ok(if reverse_first { Split::Last(i) } else { Split::First(i) });
    }
    let first_boundary = if reverse_first { profile.entry } else { profile.exit };
    if profile.peak - first_boundary > 0.001
        && let Some(i) = ramp_split(nodes, profile, start, end, MARGIN + MARGIN, !reverse_first)?
    {
        return Ok(if reverse_first { Split::First(i) } else { Split::Last(i) });
    }
    cruise_split(nodes, profile, start, end)
}

fn check_split_nodes(nodes: &[Node]) -> Result<()> {
    for row in nodes {
        if !row.step.is_finite() || row.step < 0. || !row.endpoint.is_finite() || row.endpoint < 0.
        {
            return Err(Error::Invalid("node steps and endpoints must be finite and nonnegative"));
        }
    }
    Ok(())
}

fn cruise_split(nodes: &[Node], profile: &Profile, start: usize, end: usize) -> Result<Split> {
    let left = finite(profile.entry_ramp_distance())?;
    let right = finite(left + profile.phases[CRUISE].distance)?;
    let mut distance = 0.;
    let mut selected: Option<usize> = None;
    for i in start + 1..end {
        distance = finite(distance + nodes[i].step)?;
        if distance < left {
            continue;
        }
        if right < distance {
            break;
        }
        let v = nodes[i].endpoint;
        let allowed = finite(v + (v * MARGIN).max(0.5))?;
        if allowed < profile.peak && selected.is_none_or(|prior| v < nodes[prior].endpoint) {
            selected = Some(i);
        }
    }
    Ok(selected.map_or(Split::None, Split::Both))
}

/// The profiles covering a span, in order, and the spans the vendor's
/// builder failed on.
pub(super) struct Recursive {
    pub profiles: Vec<Profile>,
    pub omitted: Vec<[usize; 2]>,
    pub calls: usize,
}

/// Covers `nodes[start..=end]` recursively in the plan's private working
/// buffer. The caller discards that buffer if any span fails.
pub(super) fn build_recursive(
    nodes: &mut [Node],
    ctx: Context,
    start: usize,
    end: usize,
    allow_refinement: bool,
    budget: usize,
) -> Result<Recursive> {
    if budget == 0 || budget > BUILDER_CALLS || start >= end || end >= nodes.len() {
        return Err(Error::Invalid("recursive span or budget"));
    }
    let mut walker =
        Walker { ctx, budget, out: Recursive { profiles: vec![], omitted: vec![], calls: 0 } };
    walker.walk(nodes, start, end, allow_refinement, 0)?;
    Ok(walker.out)
}

struct Walker {
    ctx: Context,
    budget: usize,
    out: Recursive,
}

impl Walker {
    fn build(&mut self, nodes: &mut [Node], a: usize, b: usize, refine: bool) -> Result<Built> {
        if self.out.calls >= self.budget {
            return Err(Error::Budget("profile builder budget exhausted"));
        }
        self.out.calls += 1;
        build_span(nodes, self.ctx, a, b, refine)
    }

    fn walk(
        &mut self,
        nodes: &mut [Node],
        a: usize,
        b: usize,
        refine: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > SPLIT_DEPTH {
            return Err(Error::Budget("span splitting recursed too deep"));
        }
        let built = self.build(nodes, a, b, refine)?;
        if !built.native_return {
            self.out.omitted.push([a, b]);
            return Ok(());
        }
        match select_split(nodes, &built.profile, a, b)? {
            Split::None => self.out.profiles.push(built.profile),
            Split::First(i) => {
                let left = self.build(nodes, a, i, false)?;
                self.out.profiles.push(left.profile);
                self.walk(nodes, i, b, false, depth + 1)?;
            }
            Split::Last(i) => {
                self.walk(nodes, a, i, false, depth + 1)?;
                let right = self.build(nodes, i, b, false)?;
                self.out.profiles.push(right.profile);
            }
            Split::Both(i) => {
                self.walk(nodes, a, i, false, depth + 1)?;
                self.walk(nodes, i, b, false, depth + 1)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(step: f64, endpoint: f64, cap: f64, distance: f64) -> Node {
        Node { step, endpoint, cap, acceleration: 0., distance }
    }

    fn context() -> Context {
        Context { acceleration: 100., jerk: 1000., speed: 100., time: 0.1 }
    }

    fn three(mid: f64, speed: f64) -> Vec<Node> {
        vec![node(0., 0., 100., 0.), node(mid, speed, 100., mid), node(200. - mid, 0., 100., 200.)]
    }

    /// A span's ceiling is the largest interior cap and its length the
    /// difference of cumulative distances; the jerk scales with how far the
    /// ceiling sits above the lower endpoint.
    #[test]
    fn span_uses_the_largest_interior_cap_and_cumulative_distance() {
        let mut nodes =
            vec![node(10., 2., 9999., 10.), node(80., 3., 80., 80.), node(210., 4., 60., 210.)];
        let built = build_span(&mut nodes, context(), 0, 2, false).unwrap();
        let profile = &built.profile;
        assert_eq!(
            (profile.length(), profile.entry(), profile.peak(), profile.exit()),
            (200., 2., 80., 4.)
        );
        assert!((profile.jerk() - 868.).abs() < 1e-9);
        assert!(built.native_return);
    }

    /// Refinement reports success only in the forward orientation, a vendor
    /// quirk the recursion relies on, and writes the refined endpoint back.
    #[test]
    fn refinement_keeps_the_vendor_orientation_quirk() {
        for reverse in [false, true] {
            let (entry, exit) = if reverse { (10., 0.) } else { (0., 10.) };
            let mut nodes = vec![node(0., entry, 100., 0.), node(0.01, exit, 100., 0.01)];
            let built = build_span(&mut nodes, context(), 0, 1, true).unwrap();
            assert_eq!(built.native_return, !reverse);
            assert_eq!(nodes[0].endpoint, entry);
            assert_eq!(nodes[1].endpoint, exit);
        }
    }

    /// A reversed range and a singular context are rejected without
    /// touching the nodes.
    #[test]
    fn bad_range_and_singular_context_leave_nodes_alone() {
        let mut nodes = vec![node(0., 0., 100., 0.), node(1., 10., 100., 1.)];
        let before = nodes.clone();
        assert!(build_span(&mut nodes, context(), 1, 0, true).is_err());
        let mut singular = context();
        singular.speed = 0.;
        assert!(build_span(&mut nodes, singular, 0, 1, true).is_err());
        assert_eq!(nodes, before);
    }

    /// The endpoint refinement finds the exact root in one midpoint step
    /// when the root is the interval's centre.
    #[test]
    fn endpoint_refinement_finds_the_centre_root() {
        assert_eq!(refine_endpoint(0., 1., 8., 0., 8., 1e-6).unwrap(), 4.);
    }

    /// The four split outcomes follow the vendor's selector order: a low
    /// node early in the span splits the entry ramp, late splits the exit
    /// ramp, in the cruise splits both sides, and a fast node not at all.
    #[test]
    fn all_four_split_outcomes_follow_native_order() {
        for (mid, speed, expected) in [
            (20., 0., Split::First(1)),
            (180., 0., Split::Last(1)),
            (100., 10., Split::Both(1)),
            (100., 100., Split::None),
        ] {
            let mut nodes = three(mid, speed);
            let built = build_span(&mut nodes, context(), 0, 2, false).unwrap();
            assert_eq!(select_split(&nodes, &built.profile, 0, 2).unwrap(), expected);
        }
    }

    /// Recursion keeps node order and the total span length, and charges
    /// one builder call per build.
    #[test]
    fn recursive_output_preserves_order_and_total_span() {
        let mut nodes = three(100., 10.);
        let out = build_recursive(&mut nodes, context(), 0, 2, false, 20).unwrap();
        assert_eq!(out.profiles.len(), 2);
        assert_eq!(out.calls, 3);
        assert_eq!(out.profiles.iter().map(Profile::length).sum::<f64>(), 200.);
        assert!(out.omitted.is_empty());
        assert_eq!(out.profiles[0].span(), [0, 1]);
        assert_eq!(out.profiles[1].span(), [1, 2]);
    }

    /// Exhausting a span fails the containing plan; it cannot return a
    /// partial set of profiles as successful work.
    #[test]
    fn exhaustion_refuses_the_span() {
        let mut nodes = three(100., 10.);
        assert!(build_recursive(&mut nodes, context(), 0, 2, false, 1).is_err());
    }

    /// The constant-jerk cubic gives the expected speed after one sixth of
    /// a millimetre at jerk 1000.
    #[test]
    fn cubic_has_the_expected_constant_jerk_velocity() {
        let actual = cubic_velocity(0., 1000., 1. / 6.).unwrap().unwrap();
        assert!((actual - 5.).abs() < 1e-10);
    }
}
