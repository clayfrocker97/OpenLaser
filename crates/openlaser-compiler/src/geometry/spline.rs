// SPDX-License-Identifier: GPL-3.0-or-later

//! The CAD module's cubic spline table, as the spline analyser builds it
//! (`0x100B_FE70`, `0x100B_F9C0`) and the CAD module reads it (`0x100F_83C0`,
//! `0x100F_6C90`, `0x100F_72A0`, `0x100F_5AC0`).
//!
//! A spline is evaluated once into a table of samples carrying tangent,
//! curvature and a fitted arc between neighbours; every later lookup walks
//! that table by distance. Radii are smoothed with a five-point mean before
//! they are exported to the planner.

use super::{add, cross, dot, mul, norm, sub};
use crate::{Error, Point, Result, float, to_i64};
use std::f64::consts::{PI, TAU};

fn angle(a: Point, b: Point) -> f64 {
    let (la, lb) = (norm(a), norm(b));
    if la <= 1e-8 || lb <= 1e-8 { 0. } else { ((dot(a, b) / la) / lb).clamp(-1., 1.).acos() }
}

fn raw_radius(c: Point) -> f64 {
    if c[0].abs() + c[1].abs() > 1500. {
        0.001
    } else if dot(c, c) <= 1e-8 {
        10000.
    } else {
        (1. / dot(c, c).sqrt()).max(0.001)
    }
}

/// The arc fitted between two neighbouring table samples.
#[derive(Clone, Debug, PartialEq)]
pub struct TableArc {
    /// Centre.
    pub center: Point,
    /// Radius.
    pub radius: f64,
    /// Start angle in radians.
    pub start: f64,
    /// End angle in radians.
    pub end: f64,
}

/// One row of the spline table.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// The knot-space parameter.
    pub parameter: f64,
    /// The point on the curve.
    pub point: Point,
    /// Distance from the table's start.
    pub distance: f64,
    /// One over the squared parametric speed.
    pub inverse_speed_squared: f64,
    /// Unit tangent.
    pub tangent: Point,
    /// Curvature vector.
    pub curvature: Point,
    /// Curvature radius after smoothing.
    pub radius: f64,
    /// Distance from the previous row.
    pub span: f64,
    /// The arc from the previous row, when one fits.
    pub arc: Option<TableArc>,
}

/// A cubic B-spline with its evaluated table.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    /// Control points.
    pub controls: Vec<Point>,
    /// Knot vector, four longer than the control points.
    pub knots: Vec<f64>,
    /// The evaluated table.
    pub table: Vec<Sample>,
    /// The distance window in use, `[start, end]` within the table.
    pub bounds: [f64; 2],
    /// A floor under exported radii, set for a bridge between two arcs.
    pub radius_floor: f64,
}

/// A point on the curve with its first and second derivatives.
type Evaluated = (Point, Point, Point);

fn evaluate(controls: &[Point], knots: &[f64], degree: usize, t: f64) -> Point {
    let last = controls.len() - 1;
    let span = if t >= knots[last + 1] {
        last
    } else {
        knots.partition_point(|&knot| knot <= t).saturating_sub(1).clamp(degree, last)
    };
    // Cubics and their derivatives use at most four points. Preserve de
    // Boor's operand order while avoiding one allocation per evaluation.
    let mut d = [[0.; 2]; 4];
    d[..=degree].copy_from_slice(&controls[span - degree..=span]);
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = span - degree + j;
            let denominator = knots[i + degree + 1 - r] - knots[i];
            let alpha = if denominator == 0. { 0. } else { (t - knots[i]) / denominator };
            d[j] = add(mul(d[j - 1], 1. - alpha), mul(d[j], alpha));
        }
    }
    d[degree]
}

fn derivative(controls: &[Point], knots: &[f64], degree: usize) -> Vec<Point> {
    (0..controls.len() - 1)
        .map(|i| {
            let denominator = knots[i + degree + 1] - knots[i + 1];
            if denominator == 0. {
                [0.; 2]
            } else {
                mul(sub(controls[i + 1], controls[i]), float(degree) / denominator)
            }
        })
        .collect()
}

fn basis(knots: &[f64], i: usize, p: usize, t: f64) -> f64 {
    if p == 0 {
        return if knots[i] <= t && t < knots[i + 1] { 1. } else { 0. };
    }
    let a = knots[i + p] - knots[i];
    let b = knots[i + p + 1] - knots[i + 1];
    (if a == 0. { 0. } else { (t - knots[i]) / a * basis(knots, i, p - 1, t) })
        + (if b == 0. { 0. } else { (knots[i + p + 1] - t) / b * basis(knots, i + 1, p - 1, t) })
}

/// The parameters to table: each knot span subdivided by tangent turn and
/// chord length to an even count.
fn parameters(
    eval: &impl Fn(f64) -> Evaluated,
    spans: &[f64],
    sampling_step: f64,
) -> Result<Vec<f64>> {
    let mut parameters = Vec::new();
    for span in spans.windows(2) {
        let (lo, hi) = (span[0], span[1]);
        if hi - lo < 1e-5 {
            continue;
        }
        let (a, av, _) = eval(lo);
        let (b, bv, _) = eval((lo + hi) * 0.5);
        let (c, cv, _) = eval(hi);
        let length = norm(sub(b, a)) + norm(sub(c, b));
        let turn = angle(av, bv) + angle(bv, cv);
        let by_angle = (turn / (2. * PI / 180.) + 0.5).trunc();
        let by_min_distance = (length / 0.005).trunc();
        let by_max_distance = (length / sampling_step + 0.5).trunc();
        let requested = by_angle.min(by_min_distance).max(by_max_distance);
        if !requested.is_finite() || requested > 100_000. {
            return Err(Error::Budget("spline table exceeds its budget"));
        }
        let requested = usize::try_from(to_i64(requested)?).unwrap_or(0);
        let divisions = requested.div_ceil(2) * 2;
        if parameters.len() + divisions.max(1) + 1 > 100_000 {
            return Err(Error::Budget("spline table exceeds its budget"));
        }
        parameters.push(lo);
        for i in 1..divisions {
            parameters.push((float(i) * (hi - lo)) / float(divisions) + lo);
        }
        parameters.push(hi);
    }
    Ok(parameters)
}

/// One row per parameter, with tangent, curvature and raw radius. The
/// analyser drops sub-micron moves; the CAD module applies a second, finer
/// filter while building its own table.
fn tabulate(eval: &impl Fn(f64) -> Evaluated, parameters: &[f64]) -> Result<Vec<Sample>> {
    let mut table: Vec<Sample> = Vec::new();
    let mut last_analyzer_point: Option<Point> = None;
    for &parameter in parameters {
        let (point, v, a) = eval(parameter);
        if last_analyzer_point
            .is_some_and(|p| (p[0] - point[0]).abs() <= 0.001 && (p[1] - point[1]).abs() <= 0.001)
        {
            continue;
        }
        last_analyzer_point = Some(point);
        if table.last().is_some_and(|p| {
            (p.point[0] - point[0]).abs() < 0.0001 && (p.point[1] - point[1]).abs() < 0.0001
        }) {
            continue;
        }
        let speed = norm(v);
        let (tangent, curvature, inverse_speed_squared) = if speed == 0. {
            let len = norm(a);
            (if len > 0. { mul(a, 1. / len) } else { [0.; 2] }, [0.; 2], 0.)
        } else {
            let tangent = mul(v, 1. / speed);
            let inverse = 1. / (speed * speed);
            (tangent, mul(add(a, mul(tangent, -dot(a, tangent))), inverse), inverse)
        };
        let radius = raw_radius(curvature);
        table.push(Sample {
            parameter,
            point,
            distance: 0.,
            inverse_speed_squared,
            tangent,
            curvature,
            radius,
            span: 0.,
            arc: None,
        });
    }
    if table.len() < 2 {
        return Err(Error::Invalid("spline has fewer than two distinct table points"));
    }
    Ok(table)
}

/// The arc from each row to the next, when the tangents turn the same way
/// and the mean radius is neither huge nor tight against the chord; the row
/// spans and distances follow the arcs where they fit and the chords elsewhere.
fn fit_arcs(table: &mut [Sample]) {
    for i in 1..table.len() {
        let previous = &table[i - 1];
        let current = &table[i];
        let delta = sub(current.point, previous.point);
        let chord = norm(delta);
        let radius = (previous.radius + current.radius) * 0.5;
        let current_side = cross(delta, current.tangent) > 0.;
        let previous_side = cross(delta, previous.tangent) > 0.;
        let arc = if radius > 500.
            || radius * (PI / 4.) < chord
            || current_side == previous_side
            || (0.5 * chord * 0.5 * chord * 0.5) / radius < 1e-6
        {
            None
        } else {
            let sign = if previous_side { -1. } else { 1. };
            let offset = (radius * radius - chord * chord * 0.25).sqrt() / chord;
            let center = add(
                mul(add(previous.point, current.point), 0.5),
                mul([-sign * delta[1], sign * delta[0]], offset),
            );
            let a = sub(previous.point, center);
            let b = sub(current.point, center);
            let start = a[1].atan2(a[0]);
            let mut end = b[1].atan2(b[0]);
            if sign < 0. && start < end {
                end -= TAU;
            } else if sign > 0. && end < start {
                end += TAU;
            }
            Some(TableArc { center, radius, start, end })
        };
        let span = arc.as_ref().map_or(chord, |a| (a.end - a.start).abs() * a.radius);
        let distance = previous.distance + span;
        table[i].span = span;
        table[i].distance = distance;
        table[i].arc = arc;
    }
}

/// A symmetric five-point mean of the unfiltered radii, each side stopping
/// after two millimetres of table distance; rows far from their mean move
/// three quarters of the way to it.
fn smooth_radii(table: &mut [Sample]) {
    let mut means = Vec::with_capacity(table.len());
    for i in 0..table.len() {
        let (mut sum, mut count) = (table[i].radius, 1);
        let mut distance = 0.;
        for k in (i.saturating_sub(2)..i).rev() {
            if distance >= 2. {
                break;
            }
            distance += table[k + 1].span;
            sum += table[k].radius;
            count += 1;
        }
        distance = 0.;
        for row in table.iter().take((i + 3).min(table.len())).skip(i + 1) {
            if distance >= 2. {
                break;
            }
            distance += row.span;
            sum += row.radius;
            count += 1;
        }
        means.push(sum / float(count));
    }
    for (sample, mean) in table.iter_mut().zip(means) {
        if sample.radius / mean >= 0.01 {
            sample.radius -= (sample.radius - mean) * 0.75;
        }
    }
}

impl Spline {
    /// A spline from explicit controls and knots, tabled at the default
    /// 0.05 mm step.
    pub fn new(controls: Vec<Point>, knots: Vec<f64>) -> Result<Self> {
        Self::with_sampling_step(controls, knots, 0.05)
    }

    /// Interpolates `points` with chord-length knots and natural end
    /// conditions through the analyser's tridiagonal solve
    /// (`0x100C_2A80`, `0x100C_2CF0`).
    pub fn interpolate(points: &[Point], sampling_step: f64) -> Result<Self> {
        if points.len() < 3 || points.len() > 100_000 {
            return Err(Error::Invalid("spline interpolation needs 3 to 100 000 points"));
        }
        let n = points.len();
        let mut distances = vec![0.];
        for pair in points.windows(2) {
            let d = norm(sub(pair[1], pair[0]));
            if !d.is_finite() || d <= 0. {
                return Err(Error::Invalid("spline interpolation points coincide"));
            }
            distances.push(distances[distances.len() - 1] + d);
        }
        let total = distances[n - 1];
        let mut knots = vec![0.; 4];
        knots.extend_from_slice(&distances[1..n - 1]);
        knots.extend([total; 4]);
        let mut lower = vec![0.; n];
        let mut diagonal = vec![0.; n];
        let mut upper = vec![0.; n];
        let mut rhs = vec![[0.; 2]; n];
        diagonal[0] = knots[4] + knots[5];
        upper[0] = -knots[4];
        rhs[0] = mul(points[0], knots[5]);
        for i in 1..n - 1 {
            lower[i] = basis(&knots, i, 3, knots[i + 3]);
            diagonal[i] = basis(&knots, i + 1, 3, knots[i + 3]);
            upper[i] = basis(&knots, i + 2, 3, knots[i + 3]);
            rhs[i] = points[i];
        }
        lower[n - 1] = total - knots[n + 1];
        diagonal[n - 1] = (knots[n + 1] - (total + total)) + knots[n];
        rhs[n - 1] = mul(points[n - 1], knots[n] - total);
        for i in 0..n - 1 {
            upper[i] /= diagonal[i];
            diagonal[i + 1] -= upper[i] * lower[i + 1];
        }
        rhs[0] = mul(rhs[0], 1. / diagonal[0]);
        for i in 1..n {
            rhs[i] = mul(sub(rhs[i], mul(rhs[i - 1], lower[i])), 1. / diagonal[i]);
        }
        for i in (0..n - 1).rev() {
            rhs[i] = sub(rhs[i], mul(rhs[i + 1], upper[i]));
        }
        let mut controls = vec![points[0]];
        controls.extend(rhs);
        controls.push(points[n - 1]);
        Self::with_sampling_step(controls, knots, sampling_step)
    }

    /// Tables a spline: subdivide, evaluate, fit arcs, smooth radii.
    pub fn with_sampling_step(
        controls: Vec<Point>,
        knots: Vec<f64>,
        sampling_step: f64,
    ) -> Result<Self> {
        if controls.len() < 4
            || controls.len() > 100_000
            || knots.len() != controls.len() + 4
            || controls.iter().flatten().chain(knots.iter()).any(|v| !v.is_finite())
            || knots.windows(2).any(|p| p[1] < p[0])
            || knots[3] >= knots[controls.len()]
            || !sampling_step.is_finite()
            || sampling_step < 0.05
        {
            return Err(Error::Invalid("spline controls, knots or step are outside the domain"));
        }
        let d1 = derivative(&controls, &knots, 3);
        let d2 = derivative(&d1, &knots[1..knots.len() - 1], 2);
        let eval = |t: f64| {
            (
                evaluate(&controls, &knots, 3, t),
                evaluate(&d1, &knots[1..knots.len() - 1], 2, t),
                evaluate(&d2, &knots[2..knots.len() - 2], 1, t),
            )
        };
        let parameters = parameters(&eval, &knots[3..=controls.len()], sampling_step)?;
        let mut table = tabulate(&eval, &parameters)?;
        fit_arcs(&mut table);
        smooth_radii(&mut table);
        let end = table[table.len() - 1].distance;
        Ok(Self { controls, knots, table, bounds: [0., end], radius_floor: 0. })
    }

    /// The length inside the bounds.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.bounds[1] - self.bounds[0]
    }

    /// Moves the controls and the table by `delta`.
    pub fn translate(&mut self, delta: Point) {
        for point in &mut self.controls {
            *point = add(*point, delta);
        }
        for row in &mut self.table {
            row.point = add(row.point, delta);
            if let Some(arc) = &mut row.arc {
                arc.center = add(arc.center, delta);
            }
        }
    }

    /// Narrows the bounds by `first` at the start and `last` at the end.
    pub fn trimmed(&self, first: f64, last: f64) -> Result<Self> {
        if first < 0. || last < 0. || first + last > self.length() + 1e-9 {
            return Err(Error::Invalid("spline trim is outside its domain"));
        }
        let mut result = self.clone();
        result.bounds = [self.bounds[0] + first, self.bounds[1] - last];
        Ok(result)
    }

    fn upper(&self, distance: f64) -> usize {
        self.table.partition_point(|r| r.distance <= distance).clamp(1, self.table.len() - 1)
    }

    /// The point at `distance` from the start of the bounds.
    pub fn point_at_distance(&self, distance: f64) -> Result<Point> {
        if !distance.is_finite() || distance < 0. || distance > self.length() + 1e-9 {
            return Err(Error::Invalid("spline distance is outside its table"));
        }
        let distance = distance + self.bounds[0];
        let i = self.upper(distance);
        let current = &self.table[i];
        let previous = &self.table[i - 1];
        let t = (distance - previous.distance) / current.span;
        Ok(if let Some(a) = &current.arc {
            let angle = (a.end - a.start) * t + a.start;
            [angle.cos() * a.radius + a.center[0], angle.sin() * a.radius + a.center[1]]
        } else {
            let remaining = (current.distance - distance) / current.span;
            sub(current.point, mul(sub(current.point, previous.point), remaining))
        })
    }

    /// The tangent at `distance` from the start of the bounds.
    #[must_use]
    pub fn tangent_at_distance(&self, distance: f64) -> Point {
        let distance = distance + self.bounds[0];
        let i = self.upper(distance);
        let a = &self.table[i - 1];
        let b = &self.table[i];
        let Some(arc) = &b.arc else {
            return mul(sub(b.point, a.point), 1. / b.span);
        };
        let remaining = (b.distance - distance) / b.span;
        if remaining < 0.2 {
            return b.tangent;
        }
        if remaining > 0.8 {
            return a.tangent;
        }
        let angle = arc.end - (arc.end - arc.start) * remaining;
        let sign = if arc.end >= arc.start { 1. } else { -1. };
        [-angle.sin() * sign, angle.cos() * sign]
    }

    /// The radius at a bound, weighting the current row's curvature by the
    /// remaining distance, exactly as `0x100F_6840` does.
    fn boundary_radius(&self, distance: f64) -> f64 {
        let i = self.upper(distance);
        let a = &self.table[i - 1];
        let b = &self.table[i];
        let remaining = b.distance - distance;
        let span = b.distance - a.distance;
        (raw_radius(b.curvature) * remaining + (span - remaining) * raw_radius(a.curvature)) / span
    }

    /// The source triples of the bounded spline: radius extrema thinned by
    /// distance and tangent turn. `compact` keeps only the minimum radius
    /// between the ends, as bridging splines are exported.
    #[must_use]
    pub fn triples(&self, compact: bool) -> Vec<[f64; 3]> {
        let extrema = merged_extrema(&self.table, radius_extrema(&self.table));
        let selected = selected_rows(&self.table, &extrema);
        let triples = self.assemble(&selected);
        if compact { self.compacted(triples) } else { triples }
    }

    /// The triples of the selected rows inside the bounds, with the ends
    /// fixed up and radii floored.
    fn assemble(&self, selected: &[usize]) -> Vec<[f64; 3]> {
        let rows = &self.table;
        let mut triples = vec![[self.boundary_radius(self.bounds[0]), 0., 1.]];
        for &i in selected
            .iter()
            .filter(|&&i| rows[i].distance >= self.bounds[0] && rows[i].distance <= self.bounds[1])
        {
            let row = &rows[i];
            let at = triples.len() - 1;
            let distance = row.distance - self.bounds[0];
            if distance - triples[at][1] <= 0.001 {
                triples[at][0] = triples[at][0].min(row.radius);
            } else {
                triples.push([row.radius, distance, 1.]);
            }
        }
        let end = self.length();
        let radius = self.boundary_radius(self.bounds[1]);
        let at = triples.len() - 1;
        if triples.len() < 2 || end - triples[at][1] > 0.05 {
            triples.push([radius, end, 1.]);
        } else {
            triples[at][1] = end;
            triples[at][0] = triples[at][0].min(radius);
        }
        // Two adjacent tight radii keep only the tighter one below 0.05.
        for i in 2..triples.len() {
            if triples[i][0] < 0.05 && triples[i - 1][0] < 0.05 {
                if triples[i][0] <= triples[i - 1][0] {
                    triples[i - 1][0] = 0.05;
                } else {
                    triples[i][0] = 0.05;
                }
            }
        }
        for row in triples.iter_mut().skip(1) {
            row[0] = row[0].max(0.001);
        }
        triples
    }

    /// The compact form: the ends and the tightest interior triple, floored
    /// by the bridge's radius floor.
    fn compacted(&self, triples: Vec<[f64; 3]>) -> Vec<[f64; 3]> {
        let mut triples = if triples.len() > 2 {
            let minimum = (1..triples.len() - 1)
                .min_by(|&a, &b| triples[a][0].total_cmp(&triples[b][0]))
                .unwrap_or(1);
            vec![triples[0], triples[minimum], triples[triples.len() - 1]]
        } else {
            triples
        };
        for row in triples.iter_mut().skip(1) {
            row[0] = row[0].max(self.radius_floor);
        }
        triples
    }
}

/// The rows where the radius trend changes, with both ends.
fn radius_extrema(rows: &[Sample]) -> Vec<usize> {
    let n = rows.len();
    let sign = |v: f64| {
        if v > 1e-6 {
            1
        } else if v < -1e-6 {
            -1
        } else {
            0
        }
    };
    let mut extrema = vec![0];
    let mut previous = sign(rows[1].radius - rows[0].radius);
    for i in 1..n - 1 {
        let current = sign(rows[i + 1].radius - rows[i].radius);
        if current != previous {
            extrema.push(i);
        }
        previous = current;
    }
    extrema.push(n - 1);
    extrema
}

/// Three extrema within 0.05 mm whose outer radii agree within ten percent
/// collapse onto the middle one, walking from the end.
fn merged_extrema(rows: &[Sample], mut extrema: Vec<usize>) -> Vec<usize> {
    let mut cursor = extrema.len().checked_sub(3);
    while let Some(k) = cursor.filter(|&k| k >= 2) {
        let (a, b, c) = (extrema[k - 1], extrema[k], extrema[k + 1]);
        if rows[c].distance - rows[b].distance < 0.05
            && rows[b].distance - rows[a].distance < 0.05
            && (rows[c].radius - rows[a].radius).abs() < rows[c].radius.max(rows[a].radius) * 0.1
        {
            extrema[k - 1] = b;
            extrema.drain(k..k + 2);
            cursor = k.checked_sub(3);
        } else {
            cursor = Some(k - 1);
        }
    }
    extrema
}

/// The rows exported between extrema: every extremum at least 0.1 mm from
/// the last selection, plus interior rows every 3 mm or at a tangent turn
/// beyond about 32 degrees. A closer extremum replaces the last selection
/// when its radius is tighter.
fn selected_rows(rows: &[Sample], extrema: &[usize]) -> Vec<usize> {
    let n = rows.len();
    let mut selected = vec![0];
    for &end in extrema.iter().skip(1) {
        let start = selected[selected.len() - 1];
        if rows[end].distance - rows[start].distance >= 0.1 {
            let mut last = start;
            for k in start + 1..end {
                let cos = dot(rows[k - 1].tangent, rows[k].tangent);
                if rows[k].distance - rows[last].distance > 3. || !(0.85..=1.00001).contains(&cos) {
                    selected.push(k);
                    last = k;
                }
            }
            selected.push(end);
        } else if rows[end].radius < rows[start].radius {
            let at = selected.len() - 1;
            selected[at] = end;
        }
    }
    if selected[selected.len() - 1] < n - 1 {
        selected.push(n - 1);
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interpolating collinear points yields a straight table whose length
    /// is the chord and whose radii are the straight-line value.
    #[test]
    fn collinear_interpolation_is_straight() {
        let spline = Spline::interpolate(&[[0., 0.], [1., 0.], [2., 0.], [3., 0.]], 0.05).unwrap();
        assert!((spline.length() - 3.).abs() < 1e-9);
        assert!(spline.table.iter().all(|row| row.radius >= 9999.));
        let point = spline.point_at_distance(1.5).unwrap();
        assert!((point[0] - 1.5).abs() < 1e-9 && point[1].abs() < 1e-9);
        assert_eq!(spline.tangent_at_distance(1.5), [1., 0.]);
    }

    /// A trimmed spline reports the narrowed length and refuses trims that
    /// cross.
    #[test]
    fn trimming_narrows_the_bounds() {
        let spline =
            Spline::interpolate(&[[0., 0.], [1., 0.2], [2., 0.], [3., 0.2]], 0.05).unwrap();
        let trimmed = spline.trimmed(0.5, 0.5).unwrap();
        assert!((trimmed.length() - (spline.length() - 1.)).abs() < 1e-9);
        assert!(spline.trimmed(2., 2.).is_err());
    }

    /// Exported triples start at distance zero, end at the spline length and
    /// keep nondecreasing distances; the compact form has three rows.
    #[test]
    fn triples_span_the_bounds() {
        let spline =
            Spline::interpolate(&[[0., 0.], [1., 0.3], [2., 0.], [3., 0.3], [4., 0.]], 0.05)
                .unwrap();
        let full = spline.triples(false);
        assert_eq!(full[0][1], 0.);
        assert!((full[full.len() - 1][1] - spline.length()).abs() < 1e-9);
        assert!(full.windows(2).all(|w| w[1][1] >= w[0][1]));
        assert_eq!(spline.triples(true).len(), 3);
    }

    /// Fewer than four controls or a knot vector of the wrong length is
    /// refused.
    #[test]
    fn malformed_splines_are_refused() {
        assert!(Spline::new(vec![[0., 0.]; 3], vec![0.; 7]).is_err());
        assert!(Spline::new(vec![[0., 0.], [1., 0.], [2., 0.], [3., 0.]], vec![0.; 7]).is_err());
    }
}
