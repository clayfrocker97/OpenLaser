// SPDX-License-Identifier: GPL-3.0-or-later

use crate::Error;
use openlaser_core::geometry::{Contour, Curve, Point};

/// Maximum chord deviation used only by collision checks, in millimetres.
pub(crate) const CHORD: f64 = 0.005;
/// Covers chord error, input continuity tolerance and f32 conversion at 100 m.
pub(crate) const GUARD: f64 = 0.05;

/// Smallest outline area, in square millimetres, that counts as an outline
/// rather than a degenerate sliver.
const MIN_OUTLINE_AREA: f64 = 0.0001;
/// Most vertices one tessellated outline or arc may have. It bounds the
/// collision geometry's size; a limit chosen for this crate.
const MAX_OUTLINE_VERTICES: usize = 8192;
/// Largest coordinate, in millimetres, nesting accepts: 100 m, the domain
/// within which [`GUARD`] covers the f32 conversion.
const MAX_COORDINATE: f64 = 100_000.;
/// Most vertices of all remnant cutouts together; the same bound as all
/// nesting geometry together in the search.
pub(crate) const MAX_TOTAL_VERTICES: usize = 50_000;
/// Extra clearance, in millimetres, when checking compiled moves, which the
/// compiler has already rounded to the controller's resolution.
const COMPILED_ROUNDING: f64 = 0.001;

/// Tessellate a closed contour without replacing any original cutting curves.
pub fn polygon(contour: &Contour) -> Result<Vec<[f64; 2]>, Error> {
    if !contour.is_closed() || contour.signed_area().abs() < MIN_OUTLINE_AREA {
        return Err(Error(
            "nesting needs closed part and stock outlines with positive area".into(),
        ));
    }
    let mut points = Vec::new();
    for curve in &contour.curves {
        let samples = sample(curve)?;
        points.extend(samples.into_iter().skip(1).map(<[f64; 2]>::from));
        if points.len() > MAX_OUTLINE_VERTICES {
            return Err(Error(format!(
                "an outline exceeds the nesting limit of {MAX_OUTLINE_VERTICES} vertices"
            )));
        }
    }
    Ok(points)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sample counts are checked and bounded before conversion"
)]
fn sample(curve: &Curve) -> Result<Vec<Point>, Error> {
    if !curve.is_valid()
        || [curve.bounds().min, curve.bounds().max].iter().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || p.x.abs() > MAX_COORDINATE
                || p.y.abs() > MAX_COORDINATE
        })
    {
        return Err(Error("nesting geometry must be finite and within 100000 mm".into()));
    }
    let steps = match *curve {
        Curve::Line { .. } => 1.,
        Curve::Arc { radius, sweep, .. } => {
            let angle =
                (2. * (1. - (CHORD / radius).min(1.)).acos()).min(std::f64::consts::FRAC_PI_4);
            (sweep.abs() / angle).ceil().max(1.)
        }
    };
    if steps > MAX_OUTLINE_VERTICES as f64 {
        return Err(Error("an arc exceeds the nesting tessellation limit".into()));
    }
    let steps = steps as usize;
    Ok((0..=steps).map(|i| curve.point(i as f64 / steps as f64)).collect())
}

/// Check original curves against the actual stock boundary. Works for open
/// leads too. This is also used after machining edits, before compilation.
pub fn check_inside(stock: &Contour, paths: &[Contour], margin: f64) -> Result<(), Error> {
    check_region(stock, &[], paths, margin)
}

/// Check parts and machining curves against stock and all removed material.
pub fn check_region(
    stock: &Contour,
    cutouts: &[Contour],
    paths: &[Contour],
    margin: f64,
) -> Result<(), Error> {
    let mut check = Containment::new(stock, cutouts, margin, PathKind::Machining)?;
    for contour in paths {
        if contour.is_closed() && !check.cutouts.is_empty() {
            let boundary: Vec<Point> = polygon(contour)?.into_iter().map(Into::into).collect();
            if check.cutouts.iter().any(|hole| contains(&boundary, hole[0])) {
                return Err(Error("a part covers material already removed from the sheet".into()));
            }
        }
        for curve in &contour.curves {
            check.path(sample(curve)?.into_iter())?;
        }
    }
    Ok(())
}

/// Check compiled display polylines against stock after recipe shifts and
/// cleanup motions. The caller supplies paths with at most 0.001 mm deviation.
pub fn check_polylines<'a>(
    stock: &Contour,
    paths: impl IntoIterator<Item = &'a [[f64; 2]]>,
    margin: f64,
) -> Result<(), Error> {
    check_region_polylines(stock, &[], paths, margin)
}

/// Check compiled polylines against both stock edges and existing cutouts.
pub fn check_region_polylines<'a>(
    stock: &Contour,
    cutouts: &[Contour],
    paths: impl IntoIterator<Item = &'a [[f64; 2]]>,
    margin: f64,
) -> Result<(), Error> {
    let mut check = Containment::new(stock, cutouts, margin, PathKind::Compiled)?;
    for points in paths {
        check.path(points.iter().copied().map(Point::from))?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PathKind {
    Machining,
    Compiled,
}

impl PathKind {
    fn error(self, reason: &str) -> Error {
        let path = match self {
            Self::Machining => "a part or machining path",
            Self::Compiled => "a compiled cutting path",
        };
        Error(format!("{path} {reason}"))
    }
}

/// Shared work budget, containment and edge-clearance checks for authoring
/// curves and compiled polylines. The latter include their extra 0.001 mm error.
struct Containment {
    boundary: Vec<Point>,
    cutouts: Vec<Vec<Point>>,
    work: usize,
    margin: f64,
    kind: PathKind,
}

impl Containment {
    fn new(
        stock: &Contour,
        cutouts: &[Contour],
        margin: f64,
        kind: PathKind,
    ) -> Result<Self, Error> {
        if !margin.is_finite() || margin < 0. {
            return Err(Error("stock margin must be finite and nonnegative".into()));
        }
        let boundary = polygon(stock)?.into_iter().map(Into::into).collect();
        let margin = margin
            + 2. * CHORD
            + match kind {
                PathKind::Machining => 0.,
                PathKind::Compiled => COMPILED_ROUNDING,
            };
        let cutouts = cutouts
            .iter()
            .map(|c| polygon(c).map(|p| p.into_iter().map(Into::into).collect()))
            .collect::<Result<Vec<Vec<Point>>, Error>>()?;
        if cutouts.iter().map(Vec::len).sum::<usize>() > MAX_TOTAL_VERTICES {
            return Err(Error(format!("remnant cutouts exceed {MAX_TOTAL_VERTICES} vertices")));
        }
        Ok(Self { boundary, cutouts, work: 0, margin, kind })
    }

    fn path(&mut self, mut points: impl ExactSizeIterator<Item = Point>) -> Result<(), Error> {
        self.work = self.work.saturating_add(points.len().saturating_mul(
            self.boundary.len() + self.cutouts.iter().map(Vec::len).sum::<usize>(),
        ));
        if self.work > 50_000_000 {
            return Err(Error("stock containment check exceeds its geometry limit".into()));
        }
        let Some(mut previous) = points.next() else { return Ok(()) };
        for point in points {
            self.segment(previous, point)?;
            previous = point;
        }
        Ok(())
    }

    fn segment(&self, a: Point, b: Point) -> Result<(), Error> {
        if !a.is_finite()
            || !b.is_finite()
            || !contains(&self.boundary, a)
            || !contains(&self.boundary, b)
        {
            return Err(self.kind.error("extends outside the stock outline"));
        }
        for hole in &self.cutouts {
            if contains(hole, a) || contains(hole, b) {
                return Err(self.kind.error("crosses an existing cutout"));
            }
            for (c, d) in hole.iter().zip(hole.iter().cycle().skip(1)).take(hole.len()) {
                if crossing(a, b, *c, *d) || segment_distance(a, b, *c, *d) + 1e-7 < self.margin {
                    return Err(self.kind.error("reaches an existing cutout or its edge margin"));
                }
            }
        }
        for i in 0..self.boundary.len() {
            let c = self.boundary[i];
            let d = self.boundary[(i + 1) % self.boundary.len()];
            if crossing(a, b, c, d) || segment_distance(a, b, c, d) + 1e-7 < self.margin {
                return Err(self.kind.error("reaches the stock edge margin"));
            }
        }
        Ok(())
    }
}

pub(crate) fn contains(poly: &[Point], p: Point) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        if point_distance(p, a, b) < 1e-8 {
            return true;
        }
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

fn point_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    p.distance(a.lerp(b, if t.is_finite() { t } else { 0. }))
}

fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f64 {
    point_distance(a, c, d)
        .min(point_distance(b, c, d))
        .min(point_distance(c, a, b))
        .min(point_distance(d, a, b))
}

fn crossing(a: Point, b: Point, c: Point, d: Point) -> bool {
    let side = |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    side(a, b, c) * side(a, b, d) < -1e-16 && side(c, d, a) * side(c, d, b) < -1e-16
}
