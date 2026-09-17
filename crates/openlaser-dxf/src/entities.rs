// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading one entity into a contour, or past it.

use crate::pairs::{Pair, next_entity};
use crate::{Error, Result, Skipped};
use openlaser_core::geometry::{Contour, Curve, Point};
use std::f64::consts::TAU;

mod text;

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

/// What the entities section yielded so far.
#[derive(Default)]
pub(crate) struct Read {
    /// One contour per entity with geometry.
    pub contours: Vec<Contour>,
    /// How many entities had geometry.
    pub entities: usize,
    /// The entities read past.
    pub skipped: Vec<Skipped>,
    texts: Vec<text::TextEntity>,
}

impl Read {
    pub(crate) fn outline_text(&mut self, scale: f64) -> Result<Vec<String>> {
        let mut warnings = std::collections::BTreeSet::new();
        for text in &self.texts {
            self.contours.extend(text.outlines(scale)?);
            warnings.insert(format!("DXF text style {} was outlined using Noto Sans. Check the lettering before cutting.", text.style));
        }
        Ok(warnings.into_iter().collect())
    }
}

/// Reads the entity whose type is `pairs[start]` into `read` and returns
/// the index of the next entity.
pub(crate) fn read(pairs: &[Pair<'_>], start: usize, read: &mut Read) -> Result<usize> {
    let name = pairs[start].value.to_ascii_uppercase();
    let line = pairs[start].line;
    let end = next_entity(pairs, start + 1);
    let fields = Fields { pairs: &pairs[start + 1..end], line, entity: &name };
    if matches!(name.as_str(), "TEXT" | "MTEXT") {
        read.texts.push(text::TextEntity::read(&fields, name == "MTEXT")?);
        read.entities += 1;
        return Ok(end);
    }
    let (curves, layer, next) = match name.as_str() {
        "LINE" => (vec![fields.line()?], fields.layer(), end),
        "CIRCLE" => (vec![fields.circle()?], fields.layer(), end),
        "ARC" => (vec![fields.arc()?], fields.layer(), end),
        "LWPOLYLINE" => (fields.lwpolyline()?, fields.layer(), end),
        "POLYLINE" => {
            let (curves, next) = fields.polyline(pairs, end)?;
            (curves, fields.layer(), next)
        }
        _ if SKIPPED.contains(&name.as_str()) => {
            read.skipped.push(Skipped { entity: name, line });
            return Ok(end);
        }
        _ => return Err(Error::Unsupported { line, entity: name }),
    };
    read.entities += 1;
    read.contours.push(Contour { layer, curves });
    Ok(next)
}

/// The groups of one entity.
struct Fields<'a> {
    pairs: &'a [Pair<'a>],
    line: usize,
    entity: &'a str,
}

impl Fields<'_> {
    fn error(&self, reason: impl Into<String>) -> Error {
        Error::Entity { line: self.line, entity: self.entity.to_owned(), reason: reason.into() }
    }

    /// The single value of group `code`, if present.
    fn one(&self, code: i32) -> Result<Option<&str>> {
        let mut values = self.pairs.iter().filter(|p| p.code == code).map(|p| p.value);
        let first = values.next();
        if values.next().is_some() {
            return Err(self.error(format!("group {code} is given more than once")));
        }
        Ok(first)
    }

    /// The number in group `code`, if present.
    fn number(&self, code: i32) -> Result<Option<f64>> {
        let Some(text) = self.one(code)? else { return Ok(None) };
        let value =
            text.parse::<f64>().map_err(|_| self.error(format!("group {code} is not a number")))?;
        if !value.is_finite() || value.abs() > 1e9 {
            return Err(self.error(format!("group {code} is out of range")));
        }
        Ok(Some(value))
    }

    /// The number in group `code`, which must be present.
    fn required(&self, code: i32) -> Result<f64> {
        self.number(code)?.ok_or_else(|| self.error(format!("group {code} is missing")))
    }

    /// The layer, `0` when unnamed.
    fn layer(&self) -> String {
        self.pairs
            .iter()
            .find(|p| p.code == 8)
            .map_or_else(|| "0".to_owned(), |p| p.value.to_owned())
    }

    /// Refuses geometry off the XY plane or with a flipped extrusion.
    #[allow(clippy::float_cmp, reason = "the defaults are written exactly as 0 and 1")]
    fn planar(&self) -> Result<()> {
        for (code, expected) in
            [(30, 0.), (31, 0.), (38, 0.), (39, 0.), (210, 0.), (220, 0.), (230, 1.)]
        {
            if self.number(code)?.is_some_and(|v| v != expected) {
                return Err(self.error("not in the XY plane"));
            }
        }
        Ok(())
    }

    fn point(&self, x: i32, y: i32) -> Result<Point> {
        Ok(Point::new(self.required(x)?, self.required(y)?))
    }

    fn checked(&self, curve: Curve) -> Result<Curve> {
        if curve.is_valid() { Ok(curve) } else { Err(self.error("degenerate geometry")) }
    }

    fn line(&self) -> Result<Curve> {
        self.planar()?;
        self.checked(Curve::Line { start: self.point(10, 20)?, end: self.point(11, 21)? })
    }

    fn circle(&self) -> Result<Curve> {
        self.planar()?;
        self.checked(Curve::circle(self.point(10, 20)?, self.required(40)?))
    }

    /// An arc runs counterclockwise from its start angle to its end angle.
    fn arc(&self) -> Result<Curve> {
        self.planar()?;
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

    /// Refuses a polyline drawn with width.
    fn narrow(&self, codes: &[i32]) -> Result<()> {
        for code in codes {
            if self.number(*code)?.is_some_and(|v| v != 0.) {
                return Err(self.error("polylines with width are not supported"));
            }
        }
        Ok(())
    }

    fn lwpolyline(&self) -> Result<Vec<Curve>> {
        self.planar()?;
        self.narrow(&[43])?;
        let declared = self.whole(90)?;
        let flags = self.polyline_flags()?;
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
        self.chain(&vertices, flags & 1 != 0)
    }

    fn polyline_flags(&self) -> Result<u32> {
        let flags = self.flags(70)?;
        if flags & !(1 | 128) != 0 {
            return Err(self.error("fitted, mesh and 3D polylines are not supported"));
        }
        Ok(flags)
    }

    fn vertex(&self) -> Result<(Point, f64)> {
        self.planar()?;
        self.narrow(&[40, 41])?;
        Ok((self.point(10, 20)?, self.number(42)?.unwrap_or(0.)))
    }

    fn polyline_vertex(&self) -> Result<(Point, f64)> {
        if self.flags(70)? != 0 {
            return Err(Error::Unsupported {
                line: self.line,
                entity: "3D or spline VERTEX".into(),
            });
        }
        self.vertex()
    }

    /// A heavy polyline: its vertices follow as entities up to `SEQEND`.
    fn polyline(&self, pairs: &[Pair<'_>], mut at: usize) -> Result<(Vec<Curve>, usize)> {
        self.planar()?;
        self.narrow(&[40, 41])?;
        let flags = self.polyline_flags()?;
        let mut vertices = Vec::new();
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
                    vertices.push(fields.polyline_vertex()?);
                }
                "SEQEND" => return Ok((self.chain(&vertices, flags & 1 != 0)?, end)),
                _ => return Err(self.error(format!("{name} inside a polyline"))),
            }
            at = end;
        }
    }

    /// The whole number in group `code`, zero when absent.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "range checked")]
    fn whole(&self, code: i32) -> Result<u32> {
        let value = self.number(code)?.unwrap_or(0.);
        if value.fract() != 0. || value < 0. || value > f64::from(u32::MAX) {
            return Err(self.error(format!("group {code} is not a whole number")));
        }
        Ok(value as u32)
    }

    /// The flag word in group `code`, zero when absent.
    fn flags(&self, code: i32) -> Result<u32> {
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
