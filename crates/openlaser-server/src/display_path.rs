// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded-error polylines for display and recovery projection, and the
//! precision coordinates are sent at. Native compiler samples and records
//! are never changed by this module.

use serde::{Serialize, Serializer};

const ERROR_MM: f64 = 0.001;

/// How many thumbnail widths a thumbnail's lines may stray: finer than a
/// pixel of the largest preview that draws one.
const THUMBNAIL_STEPS: f64 = 2000.;

pub(crate) fn simplify(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    simplify_within(points, ERROR_MM)
}

/// Polylines for a thumbnail: within a two-thousandth of their extent, so
/// a drawing of thousands of arcs stays small in the library.
pub(crate) fn thumbnail(lines: &[Vec<[f64; 2]>]) -> Vec<Vec<[f64; 2]>> {
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in lines.iter().flatten() {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    let error = ((max[0] - min[0]).max(max[1] - min[1]) / THUMBNAIL_STEPS).max(ERROR_MM);
    lines.iter().map(|line| simplify_within(line, error)).collect()
}

fn simplify_within(points: &[[f64; 2]], error: f64) -> Vec<[f64; 2]> {
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
        let (mut farthest, mut distance) = (first, error * error);
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

/// Coordinates sent to the micrometre, as `serialize_with`: the display
/// and recovery never need more, and full `f64` digits double the size of
/// every pushed drawing.
pub(crate) fn micrometres<T: Micrometres + ?Sized, S: Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.rounded(serializer)
}

/// A coordinate, point or polyline that serializes to the micrometre.
pub(crate) trait Micrometres {
    fn rounded<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;
}

/// Borrows a value to serialize it through [`Micrometres`].
struct Rounded<'a, T: ?Sized>(&'a T);

impl<T: Micrometres + ?Sized> Serialize for Rounded<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.rounded(serializer)
    }
}

impl Micrometres for f64 {
    fn rounded<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64((self * 1000.).round() / 1000.)
    }
}

impl Micrometres for [f64; 2] {
    fn rounded<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [Rounded(&self[0]), Rounded(&self[1])].serialize(serializer)
    }
}

impl<T: Micrometres> Micrometres for [T] {
    fn rounded<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(Rounded))
    }
}

impl<T: Micrometres> Micrometres for Vec<T> {
    fn rounded<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().rounded(serializer)
    }
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

    #[test]
    fn a_thumbnail_keeps_its_extent_with_fewer_points() {
        let circle: Vec<_> = (0..=4000)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 4000.;
                [100. * a.cos(), 100. * a.sin()]
            })
            .collect();
        let hole = vec![[0., 0.], [0.001, 0.], [0.002, 0.]];
        let thumb = thumbnail(&[circle, hole]);
        assert!(thumb[0].len() < 200, "{}", thumb[0].len());
        assert_eq!(thumb[0].first(), Some(&[100., 0.]));
        assert_eq!(thumb[1].len(), 2, "a speck stays, as its ends");
    }

    #[test]
    fn coordinates_are_sent_to_the_micrometre() {
        #[derive(Serialize)]
        struct Line {
            #[serde(serialize_with = "micrometres")]
            points: Vec<[f64; 2]>,
            #[serde(serialize_with = "micrometres")]
            lines: Vec<Vec<[f64; 2]>>,
        }
        let line = Line {
            points: vec![[1.234_567_89, -0.000_2], [100. / 3., 2.5]],
            lines: vec![vec![[0.1 + 0.2, 7.]]],
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"points":[[1.235,-0.0],[33.333,2.5]],"lines":[[[0.3,7.0]]]}"#
        );
    }
}
