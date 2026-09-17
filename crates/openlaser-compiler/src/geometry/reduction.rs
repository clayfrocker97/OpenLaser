// SPDX-License-Identifier: GPL-3.0-or-later

//! Point reduction for runs of short lines, CADModule `0x1010_1340`,
//! `0x1010_0B60` and `0x1010_09E0`: a windowed average, a split into runs of
//! one turning sense, anchors every few millimetres, and a recursive
//! circle-error subdivision between anchors.

use super::{cross, norm, sub};
use crate::{Error, Point, Result, float};

/// Bound examined points, including dense averaging windows and repeated
/// subdivision scans. This changes refusal policy, never accepted arithmetic.
const MAX_WORK: usize = 10_000_000;

fn charge(budget: &mut usize, count: usize) -> Result<()> {
    *budget = budget
        .checked_sub(count)
        .ok_or(Error::Budget("point reduction exceeded its work budget"))?;
    Ok(())
}

/// The vendor's approximate distance: an octagonal norm, not a hypotenuse.
fn distance(a: Point, b: Point) -> f64 {
    let d = sub(a, b);
    let mut x = d[0].abs();
    let mut y = d[1].abs();
    if x < 1e-8 && y < 1e-8 {
        return 0.;
    }
    if x > y {
        std::mem::swap(&mut x, &mut y);
    }
    let t = x / y;
    ((0.5 * t + 1.) - t * 0.125 * t) * y
}

/// The distance from each point to its predecessor, zero for the first.
fn lengths(points: &[Point]) -> Vec<f64> {
    let mut lengths = vec![0.];
    lengths.extend(points.windows(2).map(|p| distance(p[0], p[1])));
    lengths
}

/// The circle through three points, unless they are too close or collinear.
fn circle(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let u = sub(b, a);
    let v = sub(c, a);
    let (lu, lv) = (norm(u), norm(v));
    let determinant = cross(u, v);
    if lu < 1e-5 || lv < 1e-5 || ((determinant / lu) / lv).abs() < 1e-8 {
        return None;
    }
    let (uu, vv) = (u[0] * u[0] + u[1] * u[1], v[0] * v[0] + v[1] * v[1]);
    let center = [
        a[0] + (uu * v[1] - vv * u[1]) / (2. * determinant),
        a[1] + (u[0] * vv - v[0] * uu) / (2. * determinant),
    ];
    Some((center, norm(sub(center, a))))
}

/// Depth-first subdivision between `a` and `b` on circle error, in the
/// vendor's order but without its recursion on the host stack.
fn split(
    out: &mut Vec<usize>,
    points: &[Point],
    range: (usize, usize),
    tolerance: f64,
    shape: (Point, f64),
    budget: &mut usize,
) -> Result<()> {
    let mut pending = vec![(range.0, range.1, shape)];
    while let Some((a, b, shape)) = pending.pop() {
        charge(budget, b.saturating_sub(a).max(1))?;
        if b <= a {
            continue;
        }
        if b == a + 1 {
            out.push(b);
            continue;
        }
        let mut maximum: Option<(usize, f64)> = None;
        for (i, point) in points.iter().enumerate().take(b).skip(a + 1) {
            let error = (distance(*point, shape.0) - shape.1).abs();
            if error >= tolerance && maximum.is_none_or(|(_, e)| error > e) {
                maximum = Some((i, error));
            }
        }
        if let Some((i, _)) = maximum
            && let Some(shape) = circle(points[a], points[i], points[b])
        {
            pending.push((i, b, shape));
            pending.push((a, i, shape));
            continue;
        }
        out.push(b);
    }
    Ok(())
}

/// Each point averaged with its neighbours within three tolerances of path
/// distance, dropping points that move less than a hundredth of a
/// millimetre; the last point is kept exactly.
fn window_average(points: &[Point], tolerance: f64, budget: &mut usize) -> Result<Vec<Point>> {
    let lengths = lengths(points);
    let window = tolerance * 3.;
    let mut averaged = vec![points[0]];
    for i in 1..points.len() {
        let (mut sum, mut count) = (points[i], 1);
        let mut d = 0.;
        for j in (0..i).rev() {
            charge(budget, 1)?;
            d += lengths[j + 1];
            if window < d {
                break;
            }
            sum[0] += points[j][0];
            sum[1] += points[j][1];
            count += 1;
        }
        d = 0.;
        for j in i + 1..points.len() {
            charge(budget, 1)?;
            d += lengths[j];
            if window < d {
                break;
            }
            sum[0] += points[j][0];
            sum[1] += points[j][1];
            count += 1;
        }
        let scale = 1. / float(count);
        let p = [sum[0] * scale, sum[1] * scale];
        let previous = averaged[averaged.len() - 1];
        if (previous[0] - p[0]).abs() > 0.01 || (previous[1] - p[1]).abs() >= 0.01 {
            averaged.push(p);
        }
    }
    let at = averaged.len() - 1;
    averaged[at] = points[points.len() - 1];
    Ok(averaged)
}

/// The indices where the turning sense changes, with both ends.
fn turning_boundaries(points: &[Point]) -> Vec<usize> {
    let n = points.len();
    let mut signs = vec![0.; n];
    for i in 1..n - 1 {
        let c = cross(sub(points[i], points[i - 1]), sub(points[i + 1], points[i]));
        signs[i] = if c > 0. {
            1.
        } else if c < 0. {
            -1.
        } else {
            0.
        };
    }
    let mut boundaries = vec![0];
    for i in 1..n - 2 {
        if signs[i] * signs[i + 1] < 0. {
            boundaries.push(i);
        }
    }
    boundaries.push(n - 1);
    boundaries
}

/// Anchors between `a` and `b` every `spacing` of path distance. The last
/// anchor moves to `b` unless the remainder is worth an anchor of its own.
fn anchors(a: usize, b: usize, lengths: &[f64], spacing: f64) -> Vec<usize> {
    let mut anchors = vec![a];
    let mut d = 0.;
    for (i, length) in lengths.iter().enumerate().take(b).skip(a + 1) {
        d += length;
        if d >= spacing {
            anchors.push(i);
            d = 0.;
        }
    }
    if anchors.len() < 2 || d >= spacing * 0.4 {
        anchors.push(b);
    } else {
        let at = anchors.len() - 1;
        anchors[at] = b;
    }
    anchors
}

/// Reduces `points` to those needed within `tolerance`.
pub fn reduce_points(points: &[Point], tolerance: f64) -> Result<Vec<Point>> {
    if points.len() > 100_000
        || !tolerance.is_finite()
        || tolerance <= 0.
        || points.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid("point reduction input is outside the domain"));
    }
    if points.len() <= 2 {
        return Ok(points.to_vec());
    }
    let mut budget = MAX_WORK;
    let averaged = window_average(points, tolerance, &mut budget)?;
    if averaged.len() <= 2 {
        return Ok(averaged);
    }
    let points = &averaged;
    let lengths = lengths(points);
    let spacing = (tolerance * 50.).max(4.);
    let mut result = vec![0];
    for pair in turning_boundaries(points).windows(2) {
        let anchors = anchors(pair[0], pair[1], &lengths, spacing);
        for i in 1..anchors.len() {
            let (a, b) = (anchors[i - 1], anchors[i]);
            if b == a + 1 {
                result.push(b);
                continue;
            }
            let (mid, end) = if i + 1 < anchors.len() {
                (b, anchors[i + 1])
            } else {
                (usize::midpoint(a, b), b)
            };
            if let Some(shape) = circle(points[a], points[mid], points[end]) {
                split(&mut result, points, (a, b), tolerance, shape, &mut budget)?;
            } else {
                result.push(b);
            }
        }
    }
    Ok(result.into_iter().map(|i| points[i]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two points or fewer pass through untouched.
    #[test]
    fn tiny_inputs_pass_through() {
        assert_eq!(reduce_points(&[[0., 0.], [1., 0.]], 0.1).unwrap(), vec![[0., 0.], [1., 0.]]);
    }

    /// Points on a straight line reduce to their two ends after averaging.
    #[test]
    fn collinear_points_reduce_to_the_ends() {
        let points: Vec<Point> = (0..=10).map(|i| [f64::from(i), 0.]).collect();
        let reduced = reduce_points(&points, 0.1).unwrap();
        assert_eq!(reduced[0], [0., 0.]);
        assert_eq!(reduced[reduced.len() - 1], [10., 0.]);
        assert!(reduced.len() <= points.len());
    }

    /// The octagonal distance matches the hypotenuse on the axes and
    /// overestimates a 3-4-5 triangle by the vendor's 4.4 percent.
    #[test]
    fn octagonal_distance_approximates_the_hypotenuse() {
        assert_eq!(distance([0., 0.], [3., 0.]), 3.);
        assert!((distance([0., 0.], [3., 4.]) - 5.21875).abs() < 1e-12);
    }

    /// A nonpositive tolerance and a non-finite point are refused.
    #[test]
    fn bad_inputs_are_refused() {
        assert!(reduce_points(&[[0., 0.], [1., 0.], [2., 0.]], 0.).is_err());
        assert!(reduce_points(&[[0., 0.], [f64::NAN, 0.], [2., 0.]], 0.1).is_err());
    }

    /// Dense points cannot make the averaging window perform unbounded
    /// quadratic work before the subdivision limit is consulted.
    #[test]
    fn dense_average_is_bounded_without_truncating_geometry() {
        let points: Vec<_> = (0..20_000).map(|i| [f64::from(i) * 1e-7, 0.]).collect();
        assert!(matches!(reduce_points(&points, 0.1), Err(Error::Budget(_))));
    }
}
