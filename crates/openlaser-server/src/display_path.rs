// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded-error polylines for display and recovery projection. Native
//! compiler samples and records are never changed by this module.

const ERROR_MM: f64 = 0.001;

pub(crate) fn simplify(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        if last <= first + 1 {
            continue;
        }
        let (mut farthest, mut distance) = (first, ERROR_MM * ERROR_MM);
        for (offset, &p) in points[first + 1..last].iter().enumerate() {
            let d = distance_squared(p, points[first], points[last]);
            if d > distance {
                farthest = first + 1 + offset;
                distance = d;
            }
        }
        if farthest != first {
            keep[farthest] = true;
            stack.push((first, farthest));
            stack.push((farthest, last));
        }
    }
    points.iter().zip(keep).filter_map(|(&p, keep)| keep.then_some(p)).collect()
}

fn distance_squared(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0., 1.)
    } else {
        0.
    };
    (p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_arc_keeps_its_shape_and_endpoints() {
        let points: Vec<_> = (0..=10000)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 10000.;
                [10. * a.cos(), 10. * a.sin()]
            })
            .collect();
        let short = simplify(&points);
        assert!(short.len() < 500);
        assert_eq!(short.first(), points.first());
        assert_eq!(short.last(), points.last());
        for point in points {
            let nearest = short
                .windows(2)
                .map(|p| distance_squared(point, p[0], p[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(nearest <= ERROR_MM.powi(2) + 1e-12);
        }
        let line: Vec<_> = (0..10000).map(|i| [f64::from(i), f64::from(i) * 2.]).collect();
        assert_eq!(simplify(&line).len(), 2);
    }
}
