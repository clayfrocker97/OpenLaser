// SPDX-License-Identifier: GPL-3.0-or-later

//! Converts resolved SVG paths, preserving explicit subpath boundaries.
//!
//! Circles, circular ellipses and circular arc commands reach us as the
//! standard cubic approximations of arcs; each is recognised and kept as the
//! exact arc it approximates, and consecutive pieces of one circle join.
//! Other curves are fitted with lines and arcs within the tolerance, except
//! text outlines, which are flattened for welding.

use crate::{Error, Result, TOLERANCE};
use openlaser_core::fit;
use openlaser_core::geometry::{Contour, Curve, Drawing, Point};
use std::f64::consts::TAU;
use usvg::tiny_skia_path::PathSegment;

const MAX_SEGMENTS: usize = 1_000_000;

/// A layer's name and its colour.
pub(crate) type LayerColor = (String, [u8; 3]);

/// The layer of paths outside any named group.
const DEFAULT_LAYER: &str = "0";

/// The drawing, and the colour of each layer that has one. Paths outside a
/// named group are on a layer of their colour, as a laser program names
/// layers by colour; black ones stay on the default layer.
pub(crate) fn drawing(
    tree: &usvg::Tree,
    mm_per_px: f64,
    tolerance: f64,
) -> Result<(Drawing, Vec<LayerColor>)> {
    let mut reader = Reader {
        drawing: Drawing::default(),
        height: f64::from(tree.size().height()) * mm_per_px,
        mm_per_px,
        tolerance,
        count: 0,
        paints: None,
        colors: Vec::new(),
    };
    reader.group(tree.root(), DEFAULT_LAYER, 0)?;
    Ok((reader.drawing, reader.colors))
}

/// A path's colour: its stroke's, else its fill's, when it is a plain one.
fn color(path: &usvg::Path) -> Option<[u8; 3]> {
    let paint =
        path.stroke().map(usvg::Stroke::paint).or_else(|| path.fill().map(usvg::Fill::paint))?;
    match paint {
        usvg::Paint::Color(c) => Some([c.red, c.green, c.blue]),
        _ => None,
    }
}

/// A colour's name as a layer: hex, as the file writes it.
fn color_name(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

struct Reader {
    drawing: Drawing,
    height: f64,
    mm_per_px: f64,
    tolerance: f64,
    count: usize,
    paints: Option<Vec<(std::ops::Range<usize>, usvg::FillRule)>>,
    /// The colour of each layer that has one, first path first.
    colors: Vec<LayerColor>,
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
                    let rgb = color(path).filter(|rgb| *rgb != [0, 0, 0]);
                    let named = match rgb {
                        Some(rgb) if layer == DEFAULT_LAYER => color_name(rgb),
                        _ => layer.to_owned(),
                    };
                    if let Some(rgb) = rgb
                        && !self.colors.iter().any(|(name, _)| *name == named)
                    {
                        self.colors.push((named.clone(), rgb));
                    }
                    self.path(path, &named)?;
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
            mm_per_px: self.mm_per_px,
            tolerance: self.tolerance,
            count: self.count,
            paints: Some(Vec::new()),
            colors: Vec::new(),
        };
        reader.group(text.flattened(), layer, depth)?;
        for (name, rgb) in reader.colors {
            if !self.colors.iter().any(|(known, _)| *known == name) {
                self.colors.push((name, rgb));
            }
        }
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
        let (height, mm_per_px) = (self.height, self.mm_per_px);
        let point = |p: usvg::tiny_skia_path::Point| {
            Point::new(
                (f64::from(transform.sx) * f64::from(p.x)
                    + f64::from(transform.kx) * f64::from(p.y)
                    + f64::from(transform.tx))
                    * mm_per_px,
                height
                    - (f64::from(transform.ky) * f64::from(p.x)
                        + f64::from(transform.sy) * f64::from(p.y)
                        + f64::from(transform.ty))
                        * mm_per_px,
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
        in_range(start, end)?;
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
        if self.paints.is_some() {
            return self.flattened(contour, curve);
        }
        let [a, _, _, d] = curve;
        if let Some((center, radius, sweep)) = circular(curve) {
            in_range(a, d)?;
            join_arc(&mut contour.curves, a, d, center, radius, sweep);
            self.count += 1;
            return Ok(());
        }
        let fitted = fit::approximate(|t| bezier(curve, t), 0., 1., self.tolerance, false)
            .ok_or_else(|| Error("SVG curve cannot be resolved to lines and arcs".into()))?;
        for piece in fitted {
            in_range(piece.start(), piece.end())?;
            if self.count >= MAX_SEGMENTS {
                return Err(Error("SVG geometry exceeds one million line segments".into()));
            }
            match piece {
                Curve::Arc { .. } => fit::push_arc(&mut contour.curves, piece),
                Curve::Line { .. } => contour.curves.push(piece),
            }
            self.count += 1;
        }
        Ok(())
    }

    /// Text outlines stay lines, flattened to the tolerance, for welding.
    fn flattened(&mut self, contour: &mut Contour, curve: [Point; 4]) -> Result<()> {
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

fn in_range(start: Point, end: Point) -> Result<()> {
    if !start.is_finite()
        || !end.is_finite()
        || [start.x, start.y, end.x, end.y].iter().any(|v| v.abs() > 1e6)
    {
        return Err(Error("SVG coordinates are outside the supported range".into()));
    }
    Ok(())
}

/// The point at `t` on a cubic Bézier curve; exact at both ends.
fn bezier([start, first, second, end]: [Point; 4], t: f64) -> Point {
    let rest = 1. - t;
    start * (rest * rest * rest)
        + first * (3. * rest * rest * t)
        + second * (3. * rest * t * t)
        + end * (t * t * t)
}

/// The circular arc a cubic approximates, as its centre, radius and sweep:
/// both ends equally far from where their normals meet, both arms the
/// standard `4/3·tan(sweep/4)` of the radius, and the curve between on the
/// circle. The approximations of circles, arcs and rounded corners, which
/// SVG shapes and arc commands become, all qualify.
fn circular(curve: [Point; 4]) -> Option<(Point, f64, f64)> {
    let [a, b, c, d] = curve;
    let (t0, t1) = (b - a, d - c);
    let (n0, n1) = (t0.perpendicular(), t1.perpendicular());
    let across = n0.cross(n1);
    if t0.norm() <= 1e-9 || t1.norm() <= 1e-9 || across.abs() <= 1e-9 * n0.norm() * n1.norm() {
        return None;
    }
    let center = a + n0 * ((d - a).cross(n1) / across);
    let (r0, r1) = (a.distance(center), d.distance(center));
    let radius = 0.5 * (r0 + r1);
    // Coordinates arrive in single precision, so allow for its rounding.
    let slack = 2e-4 * radius + 1e-5;
    if !(radius > 1e-6 && radius < 1e6) || (r0 - r1).abs() > slack {
        return None;
    }
    let sweep = (a - center).cross(d - center).atan2((a - center).dot(d - center));
    if sweep.abs() <= 1e-9 || (a - center).cross(t0).signum() != sweep.signum() {
        return None;
    }
    let arm = 4. / 3. * (sweep.abs() / 4.).tan() * radius;
    if (t0.norm() - arm).abs() > 5e-3 * arm + 1e-5 || (t1.norm() - arm).abs() > 5e-3 * arm + 1e-5 {
        return None;
    }
    let on_circle = [0.125, 0.25, 0.5, 0.75, 0.875]
        .iter()
        .all(|&t| (bezier(curve, t).distance(center) - radius).abs() <= 1e-3 * radius + 1e-5);
    on_circle.then_some((center, radius, sweep))
}

/// The arc from `a` to `b` turning through `sweep`, ending exactly on both.
fn through(a: Point, b: Point, sweep: f64) -> Curve {
    let chord = a.distance(b);
    let direction = (b - a) * (1. / chord);
    let bulge = (sweep / 4.).tan();
    let sagitta = chord * (1. - bulge * bulge) / (4. * bulge);
    let center = a.lerp(b, 0.5) + direction.perpendicular() * sagitta;
    Curve::Arc { center, radius: center.distance(a), start_angle: (a - center).angle(), sweep }
}

/// Adds the arc from `a` to `b`, joining it to the arc before when both lie
/// on one circle, so a circle drawn in quarters becomes one full turn.
fn join_arc(curves: &mut Vec<Curve>, a: Point, b: Point, center: Point, radius: f64, sweep: f64) {
    if let Some(&Curve::Arc { center: c, radius: r, start_angle, sweep: s }) = curves.last()
        && c.distance(center) <= 1e-3 * radius + 1e-5
        && (r - radius).abs() <= 1e-3 * radius + 1e-5
        && s.signum() == sweep.signum()
        && (s + sweep).abs() <= TAU + 1e-6
    {
        let start = curves[curves.len() - 1].start();
        let total = s + sweep;
        curves.pop();
        if (total.abs() - TAU).abs() <= 1e-6 && start.distance(b) <= 1e-3 * radius + 1e-5 {
            curves.push(Curve::Arc {
                center: c,
                radius: r,
                start_angle,
                sweep: TAU * total.signum(),
            });
        } else {
            curves.push(through(start, b, total));
        }
        return;
    }
    curves.push(through(a, b, sweep));
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
