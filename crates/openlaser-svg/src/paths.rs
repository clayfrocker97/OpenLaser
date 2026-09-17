// SPDX-License-Identifier: GPL-3.0-or-later

//! Converts resolved SVG paths, preserving explicit subpath boundaries.

use crate::{Error, Result, TOLERANCE};
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use usvg::tiny_skia_path::PathSegment;

const MM_PER_PX: f64 = 25.4 / 96.;
const MAX_SEGMENTS: usize = 1_000_000;

pub(crate) fn drawing(tree: &usvg::Tree) -> Result<Drawing> {
    let mut reader = Reader {
        drawing: Drawing::default(),
        height: f64::from(tree.size().height()) * MM_PER_PX,
        count: 0,
        paints: None,
    };
    reader.group(tree.root(), "0", 0)?;
    Ok(reader.drawing)
}

struct Reader {
    drawing: Drawing,
    height: f64,
    count: usize,
    paints: Option<Vec<(std::ops::Range<usize>, usvg::FillRule)>>,
}

impl Reader {
    fn group(&mut self, group: &usvg::Group, layer: &str, depth: usize) -> Result<()> {
        if group.opacity().get() <= 0. {
            return Ok(());
        }
        if depth > 100 {
            return Err(Error("SVG groups are nested too deeply".into()));
        }
        if group.clip_path().is_some() || group.mask().is_some() || !group.filters().is_empty() {
            return Err(Error(
                "SVG clipping, masks and filters must be applied to paths before importing".into(),
            ));
        }
        let layer = if group.id().is_empty() { layer } else { group.id() };
        for node in group.children() {
            match node {
                usvg::Node::Group(group) => self.group(group, layer, depth + 1)?,
                usvg::Node::Path(path) if path.is_visible() && visible_paint(path) => {
                    let start = self.drawing.contours.len();
                    self.path(path, layer)?;
                    if let Some(paints) = &mut self.paints {
                        paints.push((
                            start..self.drawing.contours.len(),
                            path.fill().map_or(usvg::FillRule::NonZero, usvg::Fill::rule),
                        ));
                    }
                }
                usvg::Node::Text(text) => {
                    if let Some(glyph) = text
                        .layouted()
                        .iter()
                        .flat_map(|span| &span.positioned_glyphs)
                        .find(|glyph| glyph.id == usvg::GlyphId(0))
                    {
                        return Err(Error(format!(
                            "the available fonts cannot draw {:?}; import a font that includes this character, or import outlined text",
                            glyph.text
                        )));
                    }
                    self.text(text, layer, depth + 1)?;
                }
                usvg::Node::Image(_) => {
                    return Err(Error(
                        "SVG images must be converted to paths before importing".into(),
                    ));
                }
                usvg::Node::Path(_) => {}
            }
        }
        Ok(())
    }

    fn text(&mut self, text: &usvg::Text, layer: &str, depth: usize) -> Result<()> {
        let mut reader = Self {
            drawing: Drawing::default(),
            height: self.height,
            count: self.count,
            paints: Some(Vec::new()),
        };
        reader.group(text.flattened(), layer, depth)?;
        let paints = reader.paints.unwrap_or_default();
        let contours = crate::weld::text(&reader.drawing.contours, &paints);
        let count = contours.iter().map(|contour| contour.curves.len()).sum::<usize>();
        self.count = self
            .count
            .checked_add(count)
            .filter(|&n| n <= MAX_SEGMENTS)
            .ok_or_else(|| Error("SVG geometry exceeds one million line segments".into()))?;
        self.drawing.contours.extend(contours);
        Ok(())
    }

    fn path(&mut self, path: &usvg::Path, layer: &str) -> Result<()> {
        let transform = path.abs_transform();
        let height = self.height;
        let point = |p: usvg::tiny_skia_path::Point| {
            Point::new(
                (f64::from(transform.sx) * f64::from(p.x)
                    + f64::from(transform.kx) * f64::from(p.y)
                    + f64::from(transform.tx))
                    * MM_PER_PX,
                height
                    - (f64::from(transform.ky) * f64::from(p.x)
                        + f64::from(transform.sy) * f64::from(p.y)
                        + f64::from(transform.ty))
                        * MM_PER_PX,
            )
        };
        let mut contour = Contour { layer: layer.into(), curves: Vec::new() };
        let mut current = Point::ORIGIN;
        let mut start = current;
        for segment in path.data().segments() {
            match segment {
                PathSegment::MoveTo(p) => {
                    self.flush(&mut contour);
                    current = point(p);
                    start = current;
                }
                PathSegment::LineTo(p) => {
                    let end = point(p);
                    self.line(&mut contour, current, end)?;
                    current = end;
                }
                PathSegment::QuadTo(a, b) => {
                    let control = point(a);
                    let end = point(b);
                    self.bezier(
                        &mut contour,
                        [current, current.lerp(control, 2. / 3.), end.lerp(control, 2. / 3.), end],
                    )?;
                    current = end;
                }
                PathSegment::CubicTo(a, b, c) => {
                    let end = point(c);
                    self.bezier(&mut contour, [current, point(a), point(b), end])?;
                    current = end;
                }
                PathSegment::Close => {
                    self.line(&mut contour, current, start)?;
                    self.flush(&mut contour);
                    current = start;
                }
            }
        }
        self.flush(&mut contour);
        Ok(())
    }

    fn flush(&mut self, contour: &mut Contour) {
        if !contour.curves.is_empty() {
            self.drawing.contours.push(Contour {
                layer: contour.layer.clone(),
                curves: std::mem::take(&mut contour.curves),
            });
        }
    }

    fn line(&mut self, contour: &mut Contour, start: Point, end: Point) -> Result<()> {
        if !start.is_finite()
            || !end.is_finite()
            || [start.x, start.y, end.x, end.y].iter().any(|v| v.abs() > 1e6)
        {
            return Err(Error("SVG coordinates are outside the supported range".into()));
        }
        if start.distance(end) <= 1e-9 {
            return Ok(());
        }
        if self.count >= MAX_SEGMENTS {
            return Err(Error("SVG geometry exceeds one million line segments".into()));
        }
        contour.curves.push(Curve::Line { start, end });
        self.count += 1;
        Ok(())
    }

    fn bezier(&mut self, contour: &mut Contour, curve: [Point; 4]) -> Result<()> {
        let mut pending = vec![(curve, 0)];
        while let Some(([a, b, c, d], depth)) = pending.pop() {
            if distance_to_chord(b, a, d).max(distance_to_chord(c, a, d)) <= TOLERANCE {
                self.line(contour, a, d)?;
            } else {
                if depth >= 24 {
                    return Err(Error("SVG curve cannot be resolved to 0.01 mm".into()));
                }
                let ab = a.lerp(b, 0.5);
                let bc = b.lerp(c, 0.5);
                let cd = c.lerp(d, 0.5);
                let abc = ab.lerp(bc, 0.5);
                let bcd = bc.lerp(cd, 0.5);
                let middle = abc.lerp(bcd, 0.5);
                pending.push(([middle, bcd, cd, d], depth + 1));
                pending.push(([a, ab, abc, middle], depth + 1));
            }
        }
        Ok(())
    }
}

fn visible_paint(path: &usvg::Path) -> bool {
    path.fill().is_some_and(|fill| fill.opacity().get() > 0.)
        || path.stroke().is_some_and(|stroke| stroke.opacity().get() > 0.)
}

fn distance_to_chord(point: Point, a: Point, b: Point) -> f64 {
    let chord = b - a;
    let length = chord.dot(chord);
    let fraction = if length == 0. { 0. } else { ((point - a).dot(chord) / length).clamp(0., 1.) };
    point.distance(a.lerp(b, fraction))
}
