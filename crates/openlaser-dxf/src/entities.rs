// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading one entity into pieces of geometry, or past it.

use crate::pairs::{Pair, next_entity};
use crate::{Error, Result, Skipped};
use curves::{Cubic, Ellipse, Nurbs, Parametric};
use openlaser_core::geometry::{Curve, Point, Transform};
use std::f64::consts::TAU;

pub(crate) mod curves;
pub(crate) mod text;

/// Entities that carry no geometry to cut and are read past.
const SKIPPED: &[&str] = &[
    "POINT",
    "DIMENSION",
    "LEADER",
    "MLEADER",
    "HATCH",
    "SOLID",
    "TRACE",
    "3DFACE",
    "ATTDEF",
    "ATTRIB",
    "VIEWPORT",
    "IMAGE",
    "WIPEOUT",
    "TOLERANCE",
    "XLINE",
    "RAY",
    "OLE2FRAME",
    "ACAD_TABLE",
];

/// The mirror an entity drawn with extrusion `(0, 0, -1)` needs: its object
/// coordinate system's X axis runs along the world's −X.
pub(crate) const FLIP: Transform = Transform([-1., 0., 0., 1., 0., 0.]);

/// A piece of one entity's geometry, in drawing units.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    /// An exact line or arc.
    Curve(Curve),
    /// A curve to fit with lines and arcs.
    Path(Parametric),
    /// Points on a curve, to fit with lines and arcs.
    Points(Vec<Point>),
}

/// One entity with geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entity {
    pub layer: String,
    pub pieces: Vec<Piece>,
    /// The line it starts on.
    pub line: usize,
    /// Its type.
    pub name: String,
    /// Whether it was drawn with a flipped extrusion and mirrored.
    pub flipped: bool,
}

/// A block reference: which block, on which layer, and where each copy of
/// an array lies.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Insert {
    pub block: String,
    pub layer: String,
    pub copies: Vec<Transform>,
    pub line: usize,
}

/// What an entities section, or a block, yielded so far.
#[derive(Default)]
pub(crate) struct Read {
    pub entities: Vec<Entity>,
    pub texts: Vec<(text::TextEntity, Transform)>,
    pub inserts: Vec<Insert>,
    /// The entities read past.
    pub skipped: Vec<Skipped>,
    /// Paper-space entities left out.
    pub paper: usize,
    /// Entities marked invisible and left out.
    pub invisible: usize,
}

/// Reads the entity whose type is `pairs[start]` into `read` and returns
/// the index of the next entity.
pub(crate) fn read(pairs: &[Pair<'_>], start: usize, read: &mut Read) -> Result<usize> {
    let name = pairs[start].value.to_ascii_uppercase();
    let line = pairs[start].line;
    let end = next_entity(pairs, start + 1);
    let fields = Fields { pairs: &pairs[start + 1..end], line, entity: &name };
    // Whatever follows a polyline or a reference with attributes belongs to
    // it, so it is read past even when the entity itself is left out.
    let next = match name.as_str() {
        "POLYLINE" => sequence_end(pairs, end, &fields)?,
        "INSERT" if fields.flags(66)? & 1 != 0 => sequence_end(pairs, end, &fields)?,
        _ => end,
    };
    if fields.number(67)?.is_some_and(|space| space != 0.) {
        read.paper += 1;
        return Ok(next);
    }
    if fields.number(60)?.is_some_and(|hidden| hidden != 0.) {
        read.invisible += 1;
        return Ok(next);
    }
    if matches!(name.as_str(), "TEXT" | "MTEXT") {
        let flipped = fields.planar()?;
        let text = text::TextEntity::read(&fields, name == "MTEXT")?;
        read.texts.push((text, if flipped { FLIP } else { Transform::IDENTITY }));
        return Ok(next);
    }
    if name == "INSERT" {
        read.inserts.push(fields.insert()?);
        for pair in &pairs[end..next] {
            if pair.code == 0 && pair.value.eq_ignore_ascii_case("ATTRIB") {
                read.skipped.push(Skipped { entity: "ATTRIB".into(), line: pair.line });
            }
        }
        return Ok(next);
    }
    let (pieces, flipped) = match name.as_str() {
        "LINE" => {
            fields.planar()?;
            (vec![Piece::Curve(fields.line()?)], false)
        }
        "CIRCLE" => (vec![Piece::Curve(fields.circle()?)], fields.planar()?),
        "ARC" => (vec![Piece::Curve(fields.arc()?)], fields.planar()?),
        "LWPOLYLINE" => (fields.lwpolyline()?, fields.planar()?),
        "POLYLINE" => (fields.polyline(pairs, end)?, fields.planar()?),
        "ELLIPSE" => (vec![fields.ellipse()?], false),
        "SPLINE" => (vec![fields.spline()?], false),
        _ if SKIPPED.contains(&name.as_str()) => {
            read.skipped.push(Skipped { entity: name, line });
            return Ok(next);
        }
        _ => return Err(Error::Unsupported { line, entity: name }),
    };
    let pieces = if flipped { pieces.iter().map(mirrored).collect() } else { pieces };
    read.entities.push(Entity { layer: fields.layer(), pieces, line, name, flipped });
    Ok(next)
}

fn mirrored(piece: &Piece) -> Piece {
    match piece {
        Piece::Curve(curve) => Piece::Curve(FLIP.curve(curve)),
        Piece::Path(Parametric::Arc(arc)) => Piece::Path(Parametric::Arc(FLIP.curve(arc))),
        Piece::Path(Parametric::Nurbs(n)) => {
            let mut n = n.clone();
            n.controls.iter_mut().for_each(|p| *p = FLIP.apply(*p));
            Piece::Path(Parametric::Nurbs(n))
        }
        Piece::Path(other) => Piece::Path(other.clone()),
        Piece::Points(points) => Piece::Points(points.iter().map(|p| FLIP.apply(*p)).collect()),
    }
}

/// The index after the `SEQEND` that closes the sequence starting at `at`.
fn sequence_end(pairs: &[Pair<'_>], mut at: usize, owner: &Fields<'_>) -> Result<usize> {
    while let Some(pair) = pairs.get(at) {
        let end = next_entity(pairs, at + 1);
        if pair.value.eq_ignore_ascii_case("SEQEND") {
            return Ok(end);
        }
        at = end;
    }
    Err(owner.error("no SEQEND closes it"))
}

/// The groups of one entity.
pub(crate) struct Fields<'a> {
    pub pairs: &'a [Pair<'a>],
    pub line: usize,
    pub entity: &'a str,
}

impl Fields<'_> {
    pub(crate) fn error(&self, reason: impl Into<String>) -> Error {
        Error::Entity { line: self.line, entity: self.entity.to_owned(), reason: reason.into() }
    }

    /// The single value of group `code`, if present.
    pub(crate) fn one(&self, code: i32) -> Result<Option<&str>> {
        let mut values = self.pairs.iter().filter(|p| p.code == code).map(|p| p.value);
        let first = values.next();
        if values.next().is_some() {
            return Err(self.error(format!("group {code} is given more than once")));
        }
        Ok(first)
    }

    fn parse(&self, code: i32, text: &str) -> Result<f64> {
        let value =
            text.parse::<f64>().map_err(|_| self.error(format!("group {code} is not a number")))?;
        if !value.is_finite() || value.abs() > 1e9 {
            return Err(self.error(format!("group {code} is out of range")));
        }
        Ok(value)
    }

    /// The number in group `code`, if present.
    pub(crate) fn number(&self, code: i32) -> Result<Option<f64>> {
        self.one(code)?.map(|text| self.parse(code, text)).transpose()
    }

    /// Every number in group `code`, in order.
    fn numbers(&self, code: i32) -> Result<Vec<f64>> {
        self.pairs.iter().filter(|p| p.code == code).map(|p| self.parse(code, p.value)).collect()
    }

    /// The points written as groups `x` and `y`, one after the other.
    fn points(&self, x: i32, y: i32, z: i32) -> Result<Vec<Point>> {
        let mut points: Vec<Point> = Vec::new();
        let mut has_y = true;
        for pair in self.pairs {
            if pair.code == x {
                if !has_y {
                    return Err(self.error(format!("group {x} without its {y}")));
                }
                points.push(Point::new(self.parse(x, pair.value)?, 0.));
                has_y = false;
            } else if pair.code == y {
                let last = points.last_mut().filter(|_| !has_y);
                let point = last.ok_or_else(|| self.error(format!("group {y} without its {x}")))?;
                point.y = self.parse(y, pair.value)?;
                has_y = true;
            } else if pair.code == z && self.parse(z, pair.value)?.abs() > 1e-9 {
                return Err(self.error("not in the XY plane"));
            }
        }
        if !has_y {
            return Err(self.error(format!("group {x} without its {y}")));
        }
        Ok(points)
    }

    /// The number in group `code`, which must be present.
    pub(crate) fn required(&self, code: i32) -> Result<f64> {
        self.number(code)?.ok_or_else(|| self.error(format!("group {code} is missing")))
    }

    /// The layer, `0` when unnamed.
    pub(crate) fn layer(&self) -> String {
        self.pairs
            .iter()
            .find(|p| p.code == 8)
            .map_or_else(|| "0".to_owned(), |p| p.value.to_owned())
    }

    /// The extrusion direction: along +Z, or along −Z, when the entity is
    /// drawn mirrored. Anything else is off the XY plane.
    fn extrusion(&self) -> Result<bool> {
        let (x, y) = (self.number(210)?.unwrap_or(0.), self.number(220)?.unwrap_or(0.));
        let z = self.number(230)?.unwrap_or(1.);
        if x.abs() > 1e-9 || y.abs() > 1e-9 || (z.abs() - 1.).abs() > 1e-9 {
            return Err(self.error("not in the XY plane"));
        }
        Ok(z < 0.)
    }

    /// Refuses geometry off the XY plane; says whether the entity's object
    /// coordinates are mirrored by a flipped extrusion.
    #[allow(clippy::float_cmp, reason = "the defaults are written exactly as 0")]
    pub(crate) fn planar(&self) -> Result<bool> {
        for code in [30, 31, 38, 39] {
            if self.number(code)?.is_some_and(|v| v != 0.) {
                return Err(self.error("not in the XY plane"));
            }
        }
        self.extrusion()
    }

    pub(crate) fn point(&self, x: i32, y: i32) -> Result<Point> {
        Ok(Point::new(self.required(x)?, self.required(y)?))
    }

    fn checked(&self, curve: Curve) -> Result<Curve> {
        if curve.is_valid() { Ok(curve) } else { Err(self.error("degenerate geometry")) }
    }

    fn line(&self) -> Result<Curve> {
        self.checked(Curve::Line { start: self.point(10, 20)?, end: self.point(11, 21)? })
    }

    fn circle(&self) -> Result<Curve> {
        self.checked(Curve::circle(self.point(10, 20)?, self.required(40)?))
    }

    /// An arc runs counterclockwise from its start angle to its end angle.
    fn arc(&self) -> Result<Curve> {
        let start_angle = self.required(50)?.to_radians();
        let sweep = (self.required(51)?.to_radians() - start_angle).rem_euclid(TAU);
        if sweep == 0. {
            return Err(self.error("an arc needs an extent; draw a full turn as a circle"));
        }
        self.checked(Curve::Arc {
            center: self.point(10, 20)?,
            radius: self.required(40)?,
            start_angle,
            sweep,
        })
    }

    /// An elliptical arc, in world coordinates; its extrusion only says
    /// which way its minor axis turns.
    fn ellipse(&self) -> Result<Piece> {
        let flipped = self.extrusion()?;
        for code in [30, 31] {
            if self.number(code)?.is_some_and(|v| v.abs() > 1e-9) {
                return Err(self.error("not in the XY plane"));
            }
        }
        let center = self.point(10, 20)?;
        let major = self.point(11, 21)?;
        let ratio = self.required(40)?;
        if major.norm() <= 1e-9 || !(1e-6..=1. + 1e-9).contains(&ratio) {
            return Err(self.error("an ellipse needs a major axis and a ratio of 0 to 1"));
        }
        let minor = if flipped { -major.perpendicular() } else { major.perpendicular() } * ratio;
        let start = self.number(41)?.unwrap_or(0.);
        let mut end = self.number(42)?.unwrap_or(TAU);
        if (end - start).abs() >= TAU - 1e-9 {
            end = start + TAU;
        } else {
            end = start + (end - start).rem_euclid(TAU);
        }
        if end - start <= 1e-12 {
            return Err(self.error("an elliptical arc needs an extent"));
        }
        let ellipse = Ellipse { center, major, minor, start, end };
        Ok(match ellipse.as_arc() {
            Some(arc) => Piece::Curve(self.checked(arc)?),
            None => Piece::Path(Parametric::Ellipse(ellipse)),
        })
    }

    /// A spline by control points, or through fit points when it has none.
    fn spline(&self) -> Result<Piece> {
        self.extrusion()?;
        let flags = self.flags(70)?;
        let controls = self.points(10, 20, 30)?;
        let fit = self.points(11, 21, 31)?;
        if controls.len() > 1_000_000 || fit.len() > 1_000_000 {
            return Err(self.error("the spline has too many points"));
        }
        if controls.is_empty() {
            let tangent = |x, y| -> Result<Option<Point>> {
                Ok(match (self.number(x)?, self.number(y)?) {
                    (Some(x), Some(y)) => Some(Point::new(x, y)),
                    _ => None,
                })
            };
            let tangents = (tangent(12, 22)?, tangent(13, 23)?);
            let cubic = Cubic::through(&fit, tangents, flags & 1 != 0)
                .map_err(|reason| self.error(reason))?;
            return Ok(Piece::Path(Parametric::Cubic(cubic)));
        }
        let degree = usize::try_from(self.whole(71)?).map_err(|_| self.error("bad degree"))?;
        let knots = self.numbers(40)?;
        let mut weights = self.numbers(41)?;
        if weights.is_empty() {
            weights = vec![1.; controls.len()];
        }
        let nurbs =
            Nurbs::new(degree, knots, controls, weights).map_err(|reason| self.error(reason))?;
        Ok(Piece::Path(Parametric::Nurbs(nurbs)))
    }

    /// A block reference, with a transform for each copy of its array.
    fn insert(&self) -> Result<Insert> {
        let flipped = self.planar()?;
        let block = self.one(2)?.ok_or_else(|| self.error("no block is named"))?.to_owned();
        let at = Point::new(self.number(10)?.unwrap_or(0.), self.number(20)?.unwrap_or(0.));
        let (sx, sy) = (self.number(41)?.unwrap_or(1.), self.number(42)?.unwrap_or(1.));
        if sx.abs() < 1e-9 || sy.abs() < 1e-9 {
            return Err(self.error("a block reference needs a scale other than zero"));
        }
        let angle = self.number(50)?.unwrap_or(0.).to_radians();
        let (sin, cos) = angle.sin_cos();
        let (columns, rows) = (self.whole(70)?.max(1), self.whole(71)?.max(1));
        if u64::from(columns) * u64::from(rows) > 100_000 {
            return Err(self.error("the block array has more than 100 000 copies"));
        }
        let (dx, dy) = (self.number(44)?.unwrap_or(0.), self.number(45)?.unwrap_or(0.));
        let mut copies = Vec::new();
        for row in 0..rows {
            for column in 0..columns {
                // The array steps along the reference's rotated axes, unscaled.
                let offset = Point::new(f64::from(column) * dx, f64::from(row) * dy);
                let origin = at + offset.rotated(angle);
                let placed =
                    Transform([cos * sx, sin * sx, -sin * sy, cos * sy, origin.x, origin.y]);
                copies.push(if flipped { FLIP.after(&placed) } else { placed });
            }
        }
        Ok(Insert { block, layer: self.layer(), copies, line: self.line })
    }

    /// Refuses a polyline drawn with width.
    #[allow(clippy::float_cmp, reason = "the default is written exactly as 0")]
    fn narrow(&self, codes: &[i32]) -> Result<()> {
        for code in codes {
            if self.number(*code)?.is_some_and(|v| v != 0.) {
                return Err(self.error("polylines with width are not supported"));
            }
        }
        Ok(())
    }

    fn lwpolyline(&self) -> Result<Vec<Piece>> {
        self.narrow(&[43])?;
        let declared = self.whole(90)?;
        let flags = self.flags(70)?;
        if flags & !(1 | 128) != 0 {
            return Err(self.error("unknown lightweight polyline flags"));
        }
        let first = self.pairs.iter().position(|pair| pair.code == 10).unwrap_or(self.pairs.len());
        if self.pairs[..first].iter().any(|pair| matches!(pair.code, 20 | 40 | 41 | 42)) {
            return Err(self.error("vertex fields before the first vertex"));
        }
        let mut remaining = &self.pairs[first..];
        let mut vertices = Vec::new();
        while !remaining.is_empty() {
            let end = remaining
                .iter()
                .skip(1)
                .position(|pair| pair.code == 10)
                .map_or(remaining.len(), |index| index + 1);
            let vertex = Fields { pairs: &remaining[..end], ..*self };
            vertices.push(vertex.vertex()?);
            remaining = &remaining[end..];
        }
        if vertices.len() != declared as usize {
            return Err(
                self.error(format!("{} vertices where {declared} were declared", vertices.len()))
            );
        }
        Ok(self.chain(&vertices, flags & 1 != 0)?.into_iter().map(Piece::Curve).collect())
    }

    fn vertex(&self) -> Result<(Point, f64)> {
        self.planar()?;
        self.narrow(&[40, 41])?;
        Ok((self.point(10, 20)?, self.number(42)?.unwrap_or(0.)))
    }

    /// A heavy polyline: its vertices follow as entities up to `SEQEND`.
    /// A curve-fit polyline keeps the arcs its fitting added; a spline-fit
    /// one is fitted through the points its spline was evaluated at.
    fn polyline(&self, pairs: &[Pair<'_>], mut at: usize) -> Result<Vec<Piece>> {
        self.narrow(&[40, 41])?;
        let flags = self.flags(70)?;
        if flags & !(1 | 2 | 4 | 128) != 0 {
            return Err(self.error("3D polylines and meshes are not supported"));
        }
        let (mut vertices, mut frame) = (Vec::new(), Vec::new());
        loop {
            let Some(pair) = pairs.get(at) else {
                return Err(self.error("no SEQEND closes the polyline"));
            };
            let end = next_entity(pairs, at + 1);
            let name = pair.value.to_ascii_uppercase();
            match name.as_str() {
                "VERTEX" => {
                    let fields =
                        Fields { pairs: &pairs[at + 1..end], line: pair.line, entity: "VERTEX" };
                    let vertex = fields.flags(70)?;
                    if vertex & !(1 | 2 | 8 | 16) != 0 || (flags & 6 == 0 && vertex != 0) {
                        return Err(Error::Unsupported {
                            line: pair.line,
                            entity: "3D or mesh VERTEX".into(),
                        });
                    }
                    if vertex & 16 != 0 {
                        frame.push(fields.vertex()?.0);
                    } else {
                        vertices.push(fields.vertex()?);
                    }
                }
                "SEQEND" => break,
                _ => return Err(self.error(format!("{name} inside a polyline"))),
            }
            at = end;
        }
        let closed = flags & 1 != 0;
        if flags & 4 != 0 {
            let mut points: Vec<Point> = vertices.iter().map(|v| v.0).collect();
            if points.len() < 2 {
                // Only the frame was written: draw the spline it controls.
                let spline = Nurbs::uniform(frame, 3).map_err(|reason| self.error(reason))?;
                return Ok(vec![Piece::Path(Parametric::Nurbs(spline))]);
            }
            if closed && let Some(&first) = points.first() {
                points.push(first);
            }
            return Ok(vec![Piece::Points(points)]);
        }
        Ok(self.chain(&vertices, closed)?.into_iter().map(Piece::Curve).collect())
    }

    /// The whole number in group `code`, zero when absent.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "range checked")]
    pub(crate) fn whole(&self, code: i32) -> Result<u32> {
        let value = self.number(code)?.unwrap_or(0.);
        if value.fract() != 0. || value < 0. || value > f64::from(u32::MAX) {
            return Err(self.error(format!("group {code} is not a whole number")));
        }
        Ok(value as u32)
    }

    /// The flag word in group `code`, zero when absent.
    pub(crate) fn flags(&self, code: i32) -> Result<u32> {
        self.whole(code)
    }

    /// The curves between consecutive vertices, each a line or the arc its
    /// bulge describes, and the closing curve when the polyline is closed.
    fn chain(&self, vertices: &[(Point, f64)], closed: bool) -> Result<Vec<Curve>> {
        if vertices.len() < 2 {
            return Err(self.error("a polyline needs at least two vertices"));
        }
        let mut curves = Vec::with_capacity(vertices.len());
        for pair in vertices.windows(2) {
            curves.push(self.checked(bulged(pair[0].0, pair[1].0, pair[0].1))?);
        }
        if let Some(&(last, bulge)) = vertices.last()
            && closed
        {
            curves.push(self.checked(bulged(last, vertices[0].0, bulge))?);
        }
        Ok(curves)
    }
}

/// The curve from `a` to `b` with `bulge`, the tangent of a quarter of the
/// arc's sweep; zero is a line.
fn bulged(a: Point, b: Point, bulge: f64) -> Curve {
    if bulge == 0. || !bulge.is_finite() {
        return Curve::Line { start: a, end: b };
    }
    let chord = a.distance(b);
    let direction = (b - a) * (1. / chord);
    let sagitta = chord * (1. - bulge * bulge) / (4. * bulge);
    let center = a.lerp(b, 0.5) + direction.perpendicular() * sagitta;
    Curve::Arc {
        center,
        radius: center.distance(a),
        start_angle: (a - center).angle(),
        sweep: 4. * bulge.atan(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bulge of one is a half circle bulging to the right of travel; its
    /// sign picks the side, and the arc ends exactly on the next vertex.
    #[test]
    fn bulges_become_exact_arcs() {
        for bulge in [1., -1.] {
            let arc = bulged(Point::ORIGIN, Point::new(10., 0.), bulge);
            assert!((arc.length() - 5. * std::f64::consts::PI).abs() < 1e-9);
            assert!(arc.end().distance(Point::new(10., 0.)) < 1e-9);
            assert_eq!(arc.point(0.5).y.signum(), -bulge);
        }
        assert_eq!(
            bulged(Point::ORIGIN, Point::new(1., 1.), 0.),
            Curve::Line { start: Point::ORIGIN, end: Point::new(1., 1.) }
        );
    }
}
