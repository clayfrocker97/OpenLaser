// SPDX-License-Identifier: GPL-3.0-or-later

//! The CAD module's contour preprocessing: what the planner actually sees.
//!
//! Before planning, CADModule `0x100F_F000` and `0x100F_C150` drop nodes
//! under five microns, flatten shallow arcs, replace short arcs between
//! lines, merge collinear and co-circular neighbours, fit runs of short lines
//! with splines and, for ordinary cutting, round every corner with a bridging
//! spline sized by the corner accuracy. The result is exported as source
//! records (radius, distance, scale) for the planner, and sampled distances
//! are mapped back to coordinates through the same nodes.

pub mod reduction;
pub mod spline;

use crate::planner::Source;
use crate::{Error, Point, Result, float};
use spline::Spline;
use std::f64::consts::{FRAC_PI_2, PI};

pub(crate) fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}
pub(crate) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
pub(crate) fn mul(a: Point, s: f64) -> Point {
    [a[0] * s, a[1] * s]
}
pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub(crate) fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub(crate) fn norm(a: Point) -> f64 {
    a[0].hypot(a[1])
}

fn check_nodes(nodes: &[Node], tolerance: f64) -> Result<()> {
    if nodes.is_empty() || nodes.len() > 100_000 || !tolerance.is_finite() {
        return Err(Error::Invalid("a contour needs 1 to 100 000 finite nodes"));
    }
    if nodes.iter().any(|node| !node.is_valid()) {
        return Err(Error::Invalid("contour nodes must be finite with positive arc radii"));
    }
    Ok(())
}

/// Nodes shorter than this are dropped before anything else.
const MINIMUM_NODE: f64 = 0.005;
/// Lines no longer than this may be grouped into a fitted spline.
const SHORT_LINE: f64 = 2.;
/// The vendor's literal: ten times the degrees in a radian, used as an
/// angle in radians. Corner controls compare against its cosine.
const VENDOR_CORNER_LIMIT: f64 = 57.295_779_513_082_32 * 10.;

/// One node of a preprocessed contour.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A straight line.
    Line {
        /// Start point.
        start: Point,
        /// End point.
        end: Point,
    },
    /// An arc about `center`; the sweep from `start_angle` to `end_angle`
    /// runs clockwise when negative.
    Arc {
        /// Centre.
        center: Point,
        /// Radius, positive.
        radius: f64,
        /// Start angle in radians.
        start_angle: f64,
        /// End angle in radians.
        end_angle: f64,
    },
    /// A bridging spline the smoothing pass inserted at a corner.
    Spline(Spline),
    /// A spline fitted through a run of short lines.
    FittedSpline(Spline),
    /// A break after a straight entry lead: it blocks merging across the
    /// join and selects the lead accuracy there. Never exported.
    Marker,
}

impl Node {
    /// The node's length along the path.
    #[must_use]
    pub fn length(&self) -> f64 {
        match self {
            Self::Line { start, end } => norm(sub(*end, *start)),
            Self::Arc { radius, start_angle, end_angle, .. } => {
                radius * (end_angle - start_angle).abs()
            }
            Self::Spline(s) | Self::FittedSpline(s) => s.length(),
            Self::Marker => 0.,
        }
    }

    fn triples(&self) -> Vec<[f64; 3]> {
        match self {
            Self::Line { .. } => {
                vec![
                    [10000., 0., 1.],
                    [10000., self.length() * 0.5, 1.],
                    [10000., self.length(), 1.],
                ]
            }
            Self::Arc { radius, .. } => {
                vec![
                    [*radius, 0., 1.],
                    [*radius, self.length() * 0.5, 1.],
                    [*radius, self.length(), 1.],
                ]
            }
            Self::Spline(s) => s.triples(true),
            Self::FittedSpline(s) => s.triples(false),
            Self::Marker => Vec::new(),
        }
    }

    fn point(&self, distance: f64) -> Result<Point> {
        match self {
            Self::Line { start, end } => {
                Ok(add(*start, mul(sub(*end, *start), distance / self.length())))
            }
            Self::Arc { center, radius, start_angle, end_angle } => {
                let angle = (end_angle - start_angle) * (distance / self.length()) + start_angle;
                Ok(add(*center, [angle.cos() * radius, angle.sin() * radius]))
            }
            Self::Spline(s) | Self::FittedSpline(s) => s.point_at_distance(distance),
            Self::Marker => Err(Error::Invalid("a marker has no coordinates")),
        }
    }

    fn tangent(&self, distance: f64) -> Point {
        match self {
            Self::Line { start, end } => mul(sub(*end, *start), 1. / self.length()),
            Self::Arc { start_angle, end_angle, .. } => {
                let angle = (end_angle - start_angle) * (distance / self.length()) + start_angle;
                let sign = (end_angle - start_angle).signum();
                [-sign * angle.sin(), sign * angle.cos()]
            }
            Self::Spline(s) | Self::FittedSpline(s) => s.tangent_at_distance(distance),
            Self::Marker => [0.; 2],
        }
    }

    fn trimmed(&self, first: f64, last: f64) -> Result<Self> {
        Ok(match self {
            Self::Line { .. } => {
                Self::Line { start: self.point(first)?, end: self.point(self.length() - last)? }
            }
            Self::Arc { center, radius, start_angle, end_angle } => {
                let sweep = end_angle - start_angle;
                Self::Arc {
                    center: *center,
                    radius: *radius,
                    start_angle: sweep * (first / self.length()) + start_angle,
                    end_angle: sweep * (1. - last / self.length()) + start_angle,
                }
            }
            Self::Spline(s) => Self::Spline(s.trimmed(first, last)?),
            Self::FittedSpline(s) => Self::FittedSpline(s.trimmed(first, last)?),
            Self::Marker => Self::Marker,
        })
    }

    fn is_line(&self) -> bool {
        matches!(self, Self::Line { .. })
    }

    fn is_valid(&self) -> bool {
        let coordinates = match self {
            Self::Line { start, end } => start.iter().chain(end).all(|v| v.is_finite()),
            Self::Arc { center, radius, start_angle, end_angle } => {
                center.iter().chain([radius, start_angle, end_angle]).all(|v| v.is_finite())
                    && *radius > 0.
            }
            _ => true,
        };
        coordinates && self.length().is_finite() && self.length() >= 0.
    }
}

/// A node while the smoothing pass works out its trims.
#[derive(Clone)]
struct Work {
    node: Node,
    length: f64,
    /// Corner distance at each end.
    base: [f64; 2],
    /// Trim at each end, the base times the node's factor.
    trim: [f64; 2],
    /// Whether the node arrived as a fitted spline.
    input_spline: bool,
}

impl Work {
    fn new(node: Node) -> Self {
        Self { length: node.length(), node, base: [0.; 2], trim: [0.; 2], input_spline: false }
    }

    /// The longest corner distance the node admits: an arc gives up to
    /// thirty degrees of itself.
    fn limit(&self) -> f64 {
        match self.node {
            Node::Arc { radius, .. } => self.length.min(radius * 30f64.to_radians()),
            _ => self.length,
        }
    }

    /// The trim per unit of corner distance.
    fn factor(&self) -> f64 {
        if self.node.is_line() { 4. } else { 2.2 }
    }

    /// The two extra control points an arc contributes to a bridging spline.
    fn extra_arc_controls(&self, distance: f64, front: bool) -> Result<[Point; 2]> {
        let Node::Arc { radius, .. } = self.node else {
            return Err(Error::Invalid("arc controls require an arc"));
        };
        let half = (distance / radius) * 0.5;
        let h = (radius * (16. / 15.) * (1. - half.cos())) / half.sin();
        let (at, sign) = if front {
            (self.trim[0] - distance, 1.)
        } else {
            (self.length - self.trim[1] + distance, -1.)
        };
        let inside = add(self.node.point(at)?, mul(self.node.tangent(at), sign * h));
        let edge = if front { self.trim[0] } else { self.length - self.trim[1] };
        let outside = add(self.node.point(edge)?, mul(self.node.tangent(edge), -sign * h));
        Ok(if front { [inside, outside] } else { [outside, inside] })
    }
}

/// A preprocessed contour: the nodes the planner's source records come from.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// The nodes in cutting order.
    pub nodes: Vec<Node>,
    /// The radius of a near-full small circle the contour ends in, if any;
    /// the small-circle speed limit applies to it.
    pub small_circle_radius: Option<f64>,
}

impl Path {
    /// Preprocesses `nodes`. `smooth` selects corner rounding, which the
    /// vendor enables for ordinary cutting; `tolerance` is the corner
    /// accuracy in millimetres, clamped to 0.01..=0.3.
    pub fn new(nodes: Vec<Node>, smooth: bool, tolerance: f64) -> Result<Self> {
        check_nodes(&nodes, tolerance)?;
        let tolerance = tolerance.clamp(0.01, 0.3);
        let mut lines = wrap(nodes);
        flatten_shallow_arcs(&mut lines)?;
        replace_short_arcs(&mut lines, tolerance);
        // The lead join is recorded before merging compacts the vector, and
        // the marker blocks merging across the join before it disappears.
        let lead_join = lines
            .iter()
            .position(|l| matches!(l.node, Node::Marker))
            .and_then(|i| i.checked_sub(1));
        merge_neighbours(&mut lines);
        lines.retain(|l| !matches!(l.node, Node::Marker));
        if lines.is_empty() {
            return Err(Error::Invalid("preprocessing removed all geometry"));
        }
        let lines = fit_short_lines(&lines, smooth)?;
        let small_circle_radius = small_circle(&lines);
        if !smooth {
            return Ok(Self {
                nodes: lines.into_iter().map(|l| l.node).collect(),
                small_circle_radius,
            });
        }
        Ok(Self { nodes: round_corners(lines, tolerance, lead_join)?, small_circle_radius })
    }

    /// Host-prepared geometry: preserve its lines and arcs without the
    /// native short-line reduction, arc replacement or spline fitting.
    /// Corners still use the configured rounding and curvature speed limits.
    /// This keeps fine counters and correction subdivisions from collapsing
    /// or being approximated a second time.
    pub fn prepared(nodes: Vec<Node>, tolerance: f64) -> Result<Self> {
        check_nodes(&nodes, tolerance)?;
        let mut lines: Vec<_> = nodes
            .into_iter()
            .filter(|node| matches!(node, Node::Marker) || node.length() > 0.)
            .map(Work::new)
            .collect();
        let lead_join = lines
            .iter()
            .position(|line| matches!(line.node, Node::Marker))
            .and_then(|i| i.checked_sub(1));
        lines.retain(|line| !matches!(line.node, Node::Marker));
        if lines.is_empty() {
            return Err(Error::Invalid("a prepared contour needs nonzero geometry"));
        }
        let small_circle_radius = small_circle(&lines);
        let nodes = round_corners(lines, tolerance.clamp(0.01, 0.3), lead_join)?;
        if nodes.iter().all(|node| matches!(node, Node::Marker)) {
            return Err(Error::Invalid("a prepared contour is below the motion resolution"));
        }
        Ok(Self { nodes, small_circle_radius })
    }

    /// The path length.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.nodes.iter().map(Node::length).sum()
    }

    /// Moves every node by `delta`.
    pub fn translate(&mut self, delta: Point) {
        for node in &mut self.nodes {
            match node {
                Node::Line { start, end } => {
                    *start = add(*start, delta);
                    *end = add(*end, delta);
                }
                Node::Arc { center, .. } => *center = add(*center, delta),
                Node::Spline(s) | Node::FittedSpline(s) => s.translate(delta),
                Node::Marker => {}
            }
        }
    }

    /// Clips the prepared path between two distances. Spline tables keep
    /// their samples; this does not rerun the preprocessing.
    pub fn slice(&self, start: f64, end: f64) -> Result<Self> {
        let total = self.length();
        if !start.is_finite() || !end.is_finite() || start < 0. || end <= start || end > total {
            return Err(Error::Invalid("slice bounds are outside the path"));
        }
        if start == 0. && end == total {
            return Ok(self.clone());
        }
        let mut nodes = Vec::new();
        let mut base = 0.;
        for node in &self.nodes {
            let next = base + node.length();
            if matches!(node, Node::Marker) {
                if base > start && base < end {
                    nodes.push(Node::Marker);
                }
            } else {
                let a = start.max(base);
                let b = end.min(next);
                if a < b {
                    nodes.push(node.trimmed((a - base).max(0.), (next - b).max(0.))?);
                }
            }
            base = next;
        }
        Ok(Self { nodes, small_circle_radius: None })
    }

    /// The source triples, `[radius, distance, scale]`, in path order. A
    /// marker zeroes the radius on both sides of the join.
    #[must_use]
    pub fn triples(&self) -> Vec<[f64; 3]> {
        let mut per_node: Vec<_> = self.nodes.iter().map(Node::triples).collect();
        for (i, node) in self.nodes.iter().enumerate() {
            if matches!(node, Node::Marker) {
                if let Some(last) = per_node[..i]
                    .iter_mut()
                    .rev()
                    .find(|v| !v.is_empty())
                    .and_then(|v| v.last_mut())
                {
                    last[0] = 0.;
                }
                if let Some(next) = per_node[i + 1..].iter_mut().find(|v| !v.is_empty()) {
                    next[0][0] = 0.;
                }
            }
        }
        let Some(first) = per_node.iter().find(|v| !v.is_empty()) else {
            return Vec::new();
        };
        let mut result = vec![[first[0][0], 0., 1.]];
        let mut base = 0.;
        for rows in per_node {
            let Some(end) = rows.last() else { continue };
            let length = end[1];
            if let Some(last) = result.last_mut() {
                last[0] = last[0].min(rows[0][0]);
            }
            result.extend(rows.into_iter().skip(1).map(|mut row| {
                row[1] += base;
                row
            }));
            base += length;
        }
        result
    }

    /// The planner's source records under speed cap `cap`.
    pub fn sources(&self, cap: f64) -> Result<Vec<Source>> {
        self.triples()
            .into_iter()
            .map(|[radius, distance, scale]| Source::new(distance, radius, cap, 30_000., scale))
            .collect()
    }

    /// Maps ordered distances along the path to coordinates. A distance on a
    /// shared boundary belongs to the earlier node; the final endpoint
    /// tolerates the round-off between summed exports and lookups.
    pub fn map_distances(&self, distances: &[f64]) -> Result<Vec<Point>> {
        let nodes: Vec<_> = self.nodes.iter().filter(|n| !matches!(n, Node::Marker)).collect();
        let (mut at, mut base, mut previous) = (0, 0., 0.);
        let mut result = Vec::with_capacity(distances.len());
        let total = self.length();
        let roundoff = 8. * f64::EPSILON * total.max(1.);
        for &distance in distances {
            if !distance.is_finite() || distance < previous {
                return Err(Error::Invalid("distances must be finite and ordered"));
            }
            let distance =
                if distance > total && distance - total <= roundoff { total } else { distance };
            while at < nodes.len() && base + nodes[at].length() < distance {
                base += nodes[at].length();
                at += 1;
            }
            let node = nodes.get(at).ok_or(Error::Invalid("a sample lies beyond the path"))?;
            result.push(node.point(distance - base)?);
            previous = distance;
        }
        Ok(result)
    }
}

/// Nodes under five microns dropped and the rest wrapped for smoothing.
fn wrap(nodes: Vec<Node>) -> Vec<Work> {
    nodes
        .into_iter()
        .filter(|n| matches!(n, Node::Marker) || n.length() >= MINIMUM_NODE)
        .map(|node| {
            let input_spline = matches!(node, Node::FittedSpline(_));
            let mut work = Work::new(node);
            work.input_spline = input_spline;
            work
        })
        .collect()
}

/// An arc under thirty degrees whose sagitta is under 0.01 mm becomes a line.
fn flatten_shallow_arcs(lines: &mut [Work]) -> Result<()> {
    for line in lines {
        if let Node::Arc { radius, start_angle, end_angle, .. } = line.node {
            let angle = (end_angle - start_angle).abs();
            if angle < PI / 6. && radius - radius * (angle * 0.5).cos() < 0.01 {
                *line = Work::new(Node::Line {
                    start: line.node.point(0.)?,
                    end: line.node.point(line.length)?,
                });
            }
        }
    }
    Ok(())
}

/// Where the lines around a short arc meet, when the arc is short and
/// shallow enough to replace and the meeting point lies inside its
/// endpoint box. Returns the first line's start, the meeting point and the
/// second line's end.
fn short_arc_corner(before: &Work, arc: &Work, after: &Work, tolerance: f64) -> Option<[Point; 3]> {
    let Node::Arc { radius, start_angle, end_angle, .. } = arc.node else { return None };
    if arc.length > SHORT_LINE {
        return None;
    }
    let (Node::Line { start: a0, end: a1 }, Node::Line { start: b0, end: b1 }) =
        (&before.node, &after.node)
    else {
        return None;
    };
    let (a0, a1, b0, b1) = (*a0, *a1, *b0, *b1);
    let a = mul(sub(a1, a0), 1. / before.length);
    let b = mul(sub(b1, b0), 1. / after.length);
    let cosine = dot(a, b);
    if cosine <= 0.05 {
        return None;
    }
    let sag = radius - radius * ((end_angle - start_angle).abs() * 0.5).cos();
    if sag > ((cosine.clamp(-1., 1.).acos() * 0.5) / FRAC_PI_2 + 0.5) * tolerance {
        return None;
    }
    let determinant = cross(a, b);
    let gap = norm(sub(a1, b0));
    let meeting = if determinant.abs() < 1e-5 {
        mul(add(a1, b0), 0.5)
    } else {
        let delta = sub(b0, a1);
        let t = (delta[1] * a[0] - a[1] * delta[0]) / determinant;
        let p = sub(b0, mul(b, t));
        let da = sub(p, a1);
        let db = sub(p, b0);
        if (a[1] * da[0] - da[1] * a[0]).abs() > 0.1
            || dot(da, a) < 0.
            || (b[1] * db[0] - db[1] * b[0]).abs() > 0.1
            || dot(db, b) < 0.
        {
            return None;
        }
        p
    };
    if [sub(a1, meeting), sub(b0, meeting)].iter().any(|d| d[0].abs() > gap || d[1].abs() > gap) {
        return None;
    }
    Some([a0, meeting, b1])
}

/// The second reverse pass of `0x100F_BAC0`: a short arc between two lines
/// is replaced by their meeting point.
fn replace_short_arcs(lines: &mut Vec<Work>, tolerance: f64) {
    for i in (1..lines.len().saturating_sub(1)).rev() {
        if let Some([start, meeting, end]) =
            short_arc_corner(&lines[i - 1], &lines[i], &lines[i + 1], tolerance)
        {
            lines[i - 1] = Work::new(Node::Line { start, end: meeting });
            lines[i + 1] = Work::new(Node::Line { start: meeting, end });
            lines.remove(i);
        }
    }
}

/// Two lines that continue each other within a thousandth of their lengths,
/// or two arcs of one circle turning the same way, become one node.
fn merged(a: &Work, b: &Work) -> Option<Node> {
    match (&a.node, &b.node) {
        (Node::Line { start, end }, Node::Line { start: b0, end: b1 }) => {
            let u = sub(*end, *start);
            let v = sub(*b1, *b0);
            let c = cross(u, v).abs();
            if dot(u, v) > 0. && c < a.length * 0.001 && c < b.length * 0.001 {
                Some(Node::Line { start: *start, end: *b1 })
            } else {
                None
            }
        }
        (
            Node::Arc { center: ca, radius: ra, start_angle: a0, end_angle: a1 },
            Node::Arc { center: cb, radius: rb, start_angle: b0, end_angle: b1 },
        ) => {
            if (ca[0] - cb[0]).abs() < 0.001
                && (ca[1] - cb[1]).abs() < 0.001
                && (ra - rb).abs() < 0.001
                && (a1 - a0) * (b1 - b0) > 0.
            {
                Some(Node::Arc {
                    center: *ca,
                    radius: *ra,
                    start_angle: *a0,
                    end_angle: a1 + (b1 - b0),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `0x100F_DF30`: neighbours merge in reverse order.
fn merge_neighbours(lines: &mut Vec<Work>) {
    for i in (1..lines.len()).rev() {
        if let Some(node) = merged(&lines[i - 1], &lines[i]) {
            lines[i - 1] = Work::new(node);
            lines.remove(i);
        }
    }
}

/// Runs of lines no longer than 2 mm turning by at most 30° are reduced and
/// fitted with a spline (`0x100F_EAE0`, `0x100F_E930`).
fn fit_short_lines(lines: &[Work], smooth: bool) -> Result<Vec<Work>> {
    let mut fitted = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let first = &lines[i];
        if !first.node.is_line() || first.length > SHORT_LINE {
            fitted.push(first.clone());
            i += 1;
            continue;
        }
        let (end, total) = short_line_run(lines, i);
        let node = fit_run(&lines[i..end], total, smooth)?;
        if let Some(node) = node {
            fitted.push(Work::new(node));
        }
        i = end;
    }
    Ok(fitted)
}

fn short_line_run(lines: &[Work], i: usize) -> (usize, f64) {
    let mut end = i + 1;
    let mut total = lines[i].length;
    while end < lines.len() {
        let (a, b) = (&lines[end - 1], &lines[end]);
        let (at, bt) = (a.node.tangent(a.length), b.node.tangent(0.));
        if !b.node.is_line()
            || b.length > SHORT_LINE
            || dot(at, bt) < 0.
            || cross(at, bt).abs() > 30f64.to_radians().sin()
        {
            break;
        }
        total += b.length;
        end += 1;
    }
    (end, total)
}

fn fit_run(lines: &[Work], total: f64, smooth: bool) -> Result<Option<Node>> {
    let mut points = vec![lines[0].node.point(0.)?];
    for line in lines {
        points.push(line.node.point(line.length)?);
    }
    let points = reduction::reduce_points(&points, 0.1)?;
    Ok(match points.len() {
        2 => Some(Node::Line { start: points[0], end: points[1] }),
        n if n > 2 => Some(Node::FittedSpline(Spline::interpolate(
            &points,
            if smooth { total.clamp(0.05, 1.) } else { 0.05 },
        )?)),
        _ => None,
    })
}

/// `0x1010_5260` reads the preprocessed nodes before smoothing adds splines
/// or trims the final arc.
fn small_circle(lines: &[Work]) -> Option<f64> {
    lines.last().and_then(|last| match last.node {
        Node::Arc { radius, start_angle, end_angle, .. }
            if lines.len() < 4 && radius <= 6. && (end_angle - start_angle).abs() > PI * 1.5 =>
        {
            Some(radius)
        }
        _ => None,
    })
}

/// Existing fitted splines are resampled with a step from their length; the
/// scan producer keeps the default step, ordinary cutting does not.
fn resample_input_splines(lines: &mut [Work]) -> Result<()> {
    for line in lines {
        if line.input_spline
            && let Node::FittedSpline(s) = &line.node
        {
            line.node = Node::FittedSpline(Spline::with_sampling_step(
                s.controls.clone(),
                s.knots.clone(),
                line.length.clamp(0.05, 1.),
            )?);
            line.length = line.node.length();
        }
    }
    Ok(())
}

/// The corner distance for a turn of `turn` (minus the cosine between the
/// tangents) at `tolerance`: the accuracy scaled by the turn, tightened for
/// reversals. A corner that is nearly straight has none.
fn corner_distance(turn: f64, tolerance: f64) -> f64 {
    let mut distance = (tolerance * (2. * 2f64.sqrt())) / (1. + turn.max(-0.984_375)).sqrt();
    if turn <= -0.984_375 {
        distance *= 1.2 * 0.125;
    } else if turn < 0. {
        distance *= 1.
            - (1. - 1.2 * 0.125)
                * ((turn.clamp(-1., 1.).acos() - FRAC_PI_2) / ((-0.984_375f64).acos() - FRAC_PI_2));
    }
    distance
}

/// Each corner's base and trim on both of its nodes; the lead join uses
/// the lead accuracy instead of the corner tolerance.
fn corner_trims(lines: &mut [Work], tolerance: f64, lead_join: Option<usize>) {
    for i in 1..lines.len() {
        let tolerance = if lead_join == Some(i - 1) { 0.001 } else { tolerance };
        let (a, b) = (lines[i - 1].node.tangent(lines[i - 1].length), lines[i].node.tangent(0.));
        let turn = -dot(a, b);
        if cross(a, b).abs() < 0.34 && turn > 0. {
            continue;
        }
        let distance = corner_distance(turn, tolerance);
        lines[i - 1].base[1] = distance.min(lines[i - 1].limit());
        lines[i - 1].trim[1] = lines[i - 1].factor() * lines[i - 1].base[1];
        lines[i].base[0] = distance.min(lines[i].limit());
        lines[i].trim[0] = lines[i].factor() * lines[i].base[0];
    }
}

/// Trims that overrun a node are scaled into it, with the bases following
/// when the node is short, and a remainder under 0.1 mm goes to the larger
/// trim.
fn clamp_trims(lines: &mut [Work]) {
    for line in lines {
        let sum = line.trim[0] + line.trim[1];
        if line.length < sum {
            let factor = line.length / sum;
            line.trim[0] *= factor;
            line.trim[1] *= factor;
            let bases = line.base[0] + line.base[1];
            if line.length < 1.5 * bases {
                let factor = if bases * 0.5 < line.length {
                    (bases / line.length - 0.5) * 0.42 + 1.08
                } else {
                    1.08
                };
                line.base[0] = line.trim[0] / factor;
                line.base[1] = line.trim[1] / factor;
            }
        }
        let remainder = (line.length - line.trim[0]) - line.trim[1];
        if remainder < 0.1 && sum > 0.001 {
            if line.trim[0] <= line.trim[1] {
                line.trim[1] += remainder;
            } else {
                line.trim[0] += remainder;
            }
        }
    }
}

/// The control points of the spline bridging the corner between `line` and
/// `next`, `0x100F_A720`: the trimmed ends, the bases, and extra controls
/// that carry an arc's or spline's curvature into the bridge.
fn bridge_controls(line: &Work, next: &Work) -> Result<Vec<Point>> {
    let mut controls = vec![line.node.point(line.length - line.trim[1])?];
    bridge_exit(line, next, &mut controls)?;
    controls.push(line.node.point(line.length - line.base[1])?);
    controls.push(next.node.point(0.)?);
    bridge_entry(line, next, &mut controls)?;
    controls.push(next.node.point(next.trim[0])?);
    Ok(controls)
}

fn bridge_exit(line: &Work, next: &Work, controls: &mut Vec<Point>) -> Result<()> {
    match line.node {
        Node::Arc { .. } => {
            controls.extend(line.extra_arc_controls((line.trim[1] - line.base[1]) * 0.667, false)?);
        }
        Node::Spline(_) | Node::FittedSpline(_) => {
            let factor = if next.node.is_line() { 0.25 } else { 0.5 };
            let d = (line.trim[1] - line.base[1]).min(line.base[1]) * factor;
            let at = line.length - line.trim[1];
            controls.push(add(line.node.point(at)?, mul(line.node.tangent(at), d)));
        }
        _ => {}
    }
    Ok(())
}

fn bridge_entry(line: &Work, next: &Work, controls: &mut Vec<Point>) -> Result<()> {
    if next.node.is_line() {
        controls.push(next.node.point(next.base[0])?);
    } else {
        let a = line.node.tangent(line.length);
        let b = next.node.tangent(0.);
        let t = next.node.tangent(next.trim[0]);
        let d = -cross(a, b);
        let inside = if cross(b, t) <= 0. { d <= 0. } else { d > 0. };
        if dot(a, b) <= VENDOR_CORNER_LIMIT.cos() && inside {
            controls.push(sub(next.node.point(next.trim[0])?, mul(t, next.base[0])));
        } else {
            controls.push(next.node.point(next.base[0])?);
            if matches!(next.node, Node::Arc { .. }) {
                controls
                    .extend(next.extra_arc_controls((next.trim[0] - next.base[0]) * 0.667, true)?);
            } else {
                let d = if dot(a, b) <= VENDOR_CORNER_LIMIT.cos() {
                    (next.trim[0] - next.base[0]).min(next.base[0])
                } else {
                    next.trim[0] - next.base[0]
                };
                controls.push(sub(next.node.point(next.trim[0])?, mul(t, d * 0.5)));
            }
        }
    }
    Ok(())
}

/// The spline bridging a corner, or none when neither side was trimmed.
/// A bridge between two nearly tangent arcs keeps their smaller radius as
/// a floor under its exported radii.
fn bridge(line: &Work, next: &Work) -> Result<Option<Spline>> {
    if line.base[1] < 1e-5 && next.base[0] < 1e-5 {
        return Ok(None);
    }
    let controls = bridge_controls(line, next)?;
    let intervals = controls.len() - 3;
    let mut knots = vec![0.; 4];
    knots.extend((1..intervals).map(|k| float(k) * (1. / float(intervals))));
    knots.extend([1.; 4]);
    let mut spline = Spline::new(controls, knots)?;
    if let (Node::Arc { radius: a, .. }, Node::Arc { radius: b, .. }) = (&line.node, &next.node)
        && dot(line.node.tangent(line.length), next.node.tangent(0.)) > 5f64.to_radians().cos()
    {
        spline.radius_floor = a.min(*b);
    }
    Ok(Some(spline))
}

/// Corner rounding (`0x100F_F000`): each corner's trims come from the turn
/// and the tolerance, trims are clamped to their nodes, and a bridging
/// spline joins the trimmed neighbours. A corner without a bridge leaves a
/// marker.
fn round_corners(
    mut lines: Vec<Work>,
    tolerance: f64,
    lead_join: Option<usize>,
) -> Result<Vec<Node>> {
    resample_input_splines(&mut lines)?;
    corner_trims(&mut lines, tolerance, lead_join);
    clamp_trims(&mut lines);
    let trimmed =
        lines.iter().map(|l| l.node.trimmed(l.trim[0], l.trim[1])).collect::<Result<Vec<_>>>()?;
    let mut nodes = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if trimmed[i].length() >= 0.0001 {
            nodes.push(trimmed[i].clone());
        }
        let Some(next) = lines.get(i + 1) else { continue };
        match bridge(line, next)? {
            None => nodes.push(Node::Marker),
            Some(spline) if spline.length() >= 0.0001 => nodes.push(Node::Spline(spline)),
            Some(_) => {}
        }
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: Point, end: Point) -> Node {
        Node::Line { start, end }
    }

    #[test]
    fn a_small_closed_polyline_survives_a_degenerate_short_line_fit() {
        let mut points: Vec<_> = (0..32)
            .map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / 32.;
                [0.7 * angle.cos(), 0.87 * angle.sin()]
            })
            .collect();
        points.push(points[0]);
        let nodes = points.windows(2).map(|p| line(p[0], p[1])).collect();
        let path = Path::prepared(nodes, 0.01).unwrap();
        assert!((4.5..5.5).contains(&path.length()));
        let samples = path.map_distances(&[0., path.length() * 0.5, path.length()]).unwrap();
        assert!(norm(sub(samples[0], points[0])) < 0.001);
        assert!(samples[1][0] < -0.65);
        assert!(norm(sub(samples[2], points[0])) < 0.001);
        let triples = path.triples();
        assert!(triples.len() > 2);
        assert!(triples.windows(2).all(|pair| pair[0][1] <= pair[1][1]));
    }

    /// Without smoothing, a polyline exports three triples per line with
    /// the straight-line radius, and the corner keeps both nodes.
    #[test]
    fn unsmoothed_lines_export_three_triples_each() {
        let path =
            Path::new(vec![line([0., 0.], [10., 0.]), line([10., 0.], [10., 10.])], false, 0.01)
                .unwrap();
        assert_eq!(path.nodes.len(), 2);
        let triples = path.triples();
        assert_eq!(triples.len(), 5);
        assert_eq!(triples[0], [10000., 0., 1.]);
        assert_eq!(triples[4], [10000., 20., 1.]);
    }

    /// Collinear neighbours merge into one line and nodes under five
    /// microns disappear before merging.
    #[test]
    fn collinear_lines_merge_and_tiny_nodes_vanish() {
        let path = Path::new(
            vec![
                line([0., 0.], [5., 0.]),
                line([5., 0.], [5.001, 0.]),
                line([5.001, 0.], [10., 0.]),
            ],
            false,
            0.01,
        )
        .unwrap();
        assert_eq!(path.nodes, vec![line([0., 0.], [10., 0.])]);
    }

    /// Smoothing a right-angle corner inserts a bridging spline between the
    /// trimmed lines and keeps the path continuous.
    #[test]
    fn smoothing_bridges_a_corner_with_a_spline() {
        let path =
            Path::new(vec![line([0., 0.], [10., 0.]), line([10., 0.], [10., 10.])], true, 0.02)
                .unwrap();
        assert_eq!(path.nodes.len(), 3);
        assert!(matches!(path.nodes[1], Node::Spline(_)));
        assert!(path.length() < 20.);
        let points = path.map_distances(&[0., path.length() * 0.5, path.length()]).unwrap();
        assert_eq!(points[0], [0., 0.]);
        assert!((points[2][0] - 10.).abs() < 1e-9 && (points[2][1] - 10.).abs() < 1e-9);
    }

    /// A distance on a shared boundary maps through the earlier node, and a
    /// sample beyond the path is refused.
    #[test]
    fn shared_boundary_maps_through_the_earlier_node() {
        let path =
            Path::new(vec![line([0., 0.], [10., 0.]), line([10., 0.], [10., 10.])], false, 0.01)
                .unwrap();
        let points = path.map_distances(&[10., 10.5]).unwrap();
        assert_eq!(points[0], [10., 0.]);
        assert_eq!(points[1], [10., 0.5]);
        assert!(path.map_distances(&[21.]).is_err());
    }

    /// A short near-full arc at the end of a small contour is reported as
    /// the small circle the speed limit applies to.
    #[test]
    fn a_final_small_circle_is_detected() {
        let arc = Node::Arc { center: [0., 0.], radius: 2., start_angle: 0., end_angle: 6. };
        let path = Path::new(vec![arc], false, 0.01).unwrap();
        assert_eq!(path.small_circle_radius, Some(2.));
    }

    /// Slicing keeps the distance domain: the middle half of a line is a
    /// line of half the length starting a quarter in.
    #[test]
    fn slicing_keeps_the_distance_domain() {
        let path = Path::new(vec![line([0., 0.], [20., 0.])], false, 0.01).unwrap();
        let middle = path.slice(5., 15.).unwrap();
        assert_eq!(middle.nodes, vec![line([5., 0.], [15., 0.])]);
        assert!(path.slice(5., 25.).is_err());
    }
}
