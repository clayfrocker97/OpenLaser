// SPDX-License-Identifier: GPL-3.0-or-later

//! DXF import into [`openlaser_core`] geometry, in millimetres: lines,
//! arcs, circles, polylines with bulges, ellipses, splines, block references
//! and plain TEXT/MTEXT outlines.
//!
//! Our own rules, snapshot-tested against fixture drawings. Lines and arcs
//! are read exactly, so an arc stays an arc; ellipses, splines, spline-fit
//! polylines and arcs stretched by a block's unequal scale are fitted with
//! lines and arcs within [`Options::tolerance`]. Block references are
//! expanded, mirrored, rotated, scaled, nested and arrayed as drawn, and
//! entities drawn with a flipped extrusion are mirrored into place.
//! Entities that carry nothing to cut are skipped and reported; geometry that
//! cannot be cut, such as 3D meshes, is refused rather than dropped.
//!
//! Layers that are turned off or frozen are left out unless chosen in
//! [`Options::layers`]. Curves another curve on the same layer already cuts
//! are dropped, entities on one layer whose ends meet are chained into
//! contours, ends that miss by less than [`Options::gap`] are joined, and a
//! junction where more than two ends meet is left alone rather than guessed
//! at. Every repair is listed in [`Import::repairs`].

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests read known drawings and compare exact values"
    )
)]

mod blocks;
mod colors;
mod entities;
mod pairs;
mod source;

use entities::curves::Parametric;
use entities::{Note, Piece, Read};
use openlaser_core::fit;
use openlaser_core::geometry::{Contour, Curve, Drawing, Point, Transform};
use openlaser_core::repair::{self, Repair, Repairs};
use std::collections::{BTreeMap, BTreeSet};

/// Largest file accepted.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

/// Why a file could not be imported.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is larger than [`MAX_BYTES`].
    #[error("the DXF is larger than {MAX_BYTES} bytes")]
    TooLarge,
    /// The file is not a sequence of group code and value pairs.
    #[error("line {line}: {reason}")]
    Syntax {
        /// The line of the problem, counted from one.
        line: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// An entity has a value that cannot be cut as drawn.
    #[error("line {line}: {entity}: {reason}")]
    Entity {
        /// The line the entity starts on.
        line: usize,
        /// The entity type.
        entity: String,
        /// What is wrong.
        reason: String,
    },
    /// An entity type with no cuttable outline, such as a 3D mesh.
    #[error("line {line}: {entity} entities are not supported; explode or redraw them")]
    Unsupported {
        /// The line the entity starts on.
        line: usize,
        /// The entity type.
        entity: String,
    },
    /// A `$INSUNITS` code for a unit no part is drawn in, such as miles.
    #[error(
        "drawing units code {0} is not a unit parts are drawn in; save in millimetres or inches"
    )]
    Units(u32),
    /// Nothing to cut.
    #[error("the drawing has no lines, arcs, circles, polylines, curves or text outlines")]
    Empty,
    /// A block reference names a block the file does not define, blocks
    /// refer to themselves, or the placed blocks exceed the entity, nesting
    /// or curve limits.
    #[error("line {line}: {reason}")]
    Block {
        /// The line of the reference.
        line: usize,
        /// What is wrong.
        reason: String,
    },
    /// The options are out of range.
    #[error("{0}")]
    Options(&'static str),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// The drawing units the file declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    /// `$INSUNITS` 4.
    Millimeters,
    /// `$INSUNITS` 1, converted to millimetres.
    Inches,
    /// `$INSUNITS` 2.
    Feet,
    /// `$INSUNITS` 5.
    Centimeters,
    /// `$INSUNITS` 6.
    Meters,
    /// `$INSUNITS` 9, thousandths of an inch.
    Mils,
    /// `$INSUNITS` 10.
    Yards,
    /// `$INSUNITS` 13.
    Micrometers,
    /// `$INSUNITS` 14.
    Decimeters,
    /// No usable declaration; the coordinates were taken as millimetres.
    Unspecified,
}

impl Units {
    /// Millimetres in one drawing unit.
    #[must_use]
    pub const fn millimeters(self) -> f64 {
        match self {
            Self::Millimeters | Self::Unspecified => 1.,
            Self::Inches => 25.4,
            Self::Feet => 304.8,
            Self::Centimeters => 10.,
            Self::Meters => 1000.,
            Self::Mils => 0.0254,
            Self::Yards => 914.4,
            Self::Micrometers => 0.001,
            Self::Decimeters => 100.,
        }
    }

    /// The unit's name, when the drawing was converted from it.
    const fn converted(self) -> Option<&'static str> {
        match self {
            Self::Millimeters | Self::Unspecified | Self::Inches => None,
            Self::Feet => Some("feet"),
            Self::Centimeters => Some("centimetres"),
            Self::Meters => Some("metres"),
            Self::Mils => Some("mils"),
            Self::Yards => Some("yards"),
            Self::Micrometers => Some("micrometres"),
            Self::Decimeters => Some("decimetres"),
        }
    }
}

/// An entity that was read past because it carries nothing to cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The entity type.
    pub entity: String,
    /// The line it starts on, counted from one.
    pub line: usize,
}

/// A layer that holds geometry, and whether it was imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// The name, as the file writes it.
    pub name: String,
    /// Whether the file has it turned off or frozen.
    pub hidden: bool,
    /// Whether its geometry was imported.
    pub imported: bool,
    /// How many entities with geometry or text it holds, block copies
    /// included.
    pub entities: usize,
}

/// How to import.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// How far fitted lines and arcs may stray from the ellipses, splines and
    /// stretched arcs they replace, in millimetres; also how close two
    /// curves must lie to count as one drawn twice.
    pub tolerance: f64,
    /// The widest gap between two ends that is closed, in millimetres; zero
    /// joins only ends that meet.
    pub gap: f64,
    /// The layers to import, by name; `None` imports every layer the file
    /// does not turn off or freeze.
    pub layers: Option<Vec<String>>,
}

impl Default for Options {
    fn default() -> Self {
        Self { tolerance: DEFAULT_TOLERANCE, gap: DEFAULT_GAP, layers: None }
    }
}

/// The default fitting tolerance, in millimetres.
pub const DEFAULT_TOLERANCE: f64 = 0.01;

/// The default widest gap closed, in millimetres.
pub const DEFAULT_GAP: f64 = 0.05;

/// The widest gap [`Options::gap`] accepts, in millimetres.
pub const MAX_GAP: f64 = 1.;

/// The most lines and arcs block references may expand to.
const MAX_CURVES: usize = 2_000_000;

/// An imported drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    /// The geometry, in millimetres, chained into contours. It is empty when
    /// every layer with geometry was left out.
    pub drawing: Drawing,
    /// The units the file declared.
    pub units: Units,
    /// How many entities carried geometry, block copies included.
    pub entities: usize,
    /// The entities that were skipped.
    pub skipped: Vec<Skipped>,
    /// Font substitutions and other import details to review.
    pub warnings: Vec<String>,
    /// The layers that hold geometry, in the order first drawn.
    pub layers: Vec<Layer>,
    /// The colour of each imported layer that has one. An entity coloured
    /// otherwise than its layer is on a layer of its own, named after the
    /// layer and its colour, such as `0 Red`.
    pub colors: Vec<(String, [u8; 3])>,
    /// What was repaired, and where.
    pub repairs: Repairs,
}

/// Imports an ASCII DXF file with the default options.
pub fn import(bytes: &[u8]) -> Result<Import> {
    import_with(bytes, &Options::default())
}

/// Imports an ASCII DXF file.
pub fn import_with(bytes: &[u8], options: &Options) -> Result<Import> {
    if !(fit::MIN_TOLERANCE..=fit::MAX_TOLERANCE).contains(&options.tolerance) {
        return Err(Error::Options("the tolerance must be 0.001 to 0.5 mm"));
    }
    if !(0. ..=MAX_GAP).contains(&options.gap) {
        return Err(Error::Options("the gap must be 0 to 1 mm"));
    }
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let text = source::text(bytes)?;
    let pairs = pairs::tokenize(text.trim_start_matches('\u{feff}'))?;
    let sections = Sections::read(&pairs)?;
    let scale = sections.units.millimeters();
    let placed = blocks::expand(&sections.entities, &sections.blocks)?;
    let mut warnings = left_out(&placed);
    if let Some(unit) = sections.units.converted() {
        warnings.push(format!("The drawing is in {unit}; it was converted to millimetres."));
    }
    let mut notes = placed.notes.clone();
    if placed.entities.is_empty() && placed.texts.is_empty() {
        return Err(nothing_to_cut(&notes));
    }
    let layers = layers(&placed, &sections.layers, options.layers.as_deref());
    let chosen: BTreeSet<String> =
        layers.iter().filter(|l| l.imported).map(|l| l.name.to_uppercase()).collect();
    if chosen.is_empty() {
        warnings.push("Every layer with geometry is turned off or left out.".into());
    }
    let mut repairs = Repairs::default();
    let mut pieces = Vec::new();
    let mut curves = 0usize;
    let mut colors: Vec<(String, [u8; 3])> = Vec::new();
    for (entity, transform, layer, own) in &placed.entities {
        if !chosen.contains(&layer.to_uppercase()) {
            continue;
        }
        let by_layer = sections.colors.get(&layer.to_uppercase()).copied();
        // Shapes of another colour than their layer's are a layer of their own.
        let (layer, color) = match own {
            Some(rgb) if Some(*rgb) != by_layer => {
                (&format!("{layer} {}", colors::name(*rgb)), Some(*rgb))
            }
            _ => (layer, by_layer),
        };
        if let Some(rgb) = color
            && !colors.iter().any(|(name, _)| name == layer)
        {
            colors.push((layer.clone(), rgb));
        }
        let contour = match place(entity, transform, layer, scale, options.tolerance) {
            Ok(contour) => contour,
            Err(Error::Entity { reason, .. }) if reason == entities::EMPTY => {
                notes.push(Note { entity: entity.name.clone(), line: entity.line, reason });
                continue;
            }
            Err(other) => return Err(other),
        };
        curves = curves.saturating_add(contour.curves.len());
        if curves > MAX_CURVES {
            return Err(Error::Block {
                line: entity.line,
                reason: "the drawing expands to more than two million lines and arcs".into(),
            });
        }
        if entity.flipped
            && let Some(at) = contour.start()
        {
            repairs.mirrored.push(Repair { at, size: 0., layer: layer.clone() });
        }
        pieces.push(contour);
    }
    let (mut pieces, duplicates) = repair::without_duplicates(pieces, options.tolerance);
    repairs.duplicates = duplicates;
    warnings.extend(outline_text(&placed, &chosen, scale, &mut pieces, &mut notes)?);
    if pieces.is_empty() && !chosen.is_empty() {
        return Err(nothing_to_cut(&notes));
    }
    warnings.extend(grouped(&notes));
    let (contours, gaps) = repair::chain(&pieces, options.gap);
    repairs.gaps = gaps;
    let mut skipped = placed.skipped;
    skipped.sort_by_key(|s| s.line);
    skipped.dedup();
    Ok(Import {
        drawing: Drawing { contours },
        units: sections.units,
        entities: placed.entities.len() + placed.texts.len(),
        skipped,
        warnings,
        layers,
        colors,
        repairs,
    })
}

/// Notices for the entities left out of model space.
fn left_out(placed: &blocks::Placed<'_>) -> Vec<String> {
    let mut warnings = Vec::new();
    let plural = |n: usize| if n == 1 { "entity was" } else { "entities were" };
    if placed.paper > 0 {
        warnings.push(format!(
            "{} paper-space {} left out; only model space is imported.",
            placed.paper,
            plural(placed.paper)
        ));
    }
    if placed.invisible > 0 {
        warnings.push(format!(
            "{} invisible {} left out.",
            placed.invisible,
            plural(placed.invisible)
        ));
    }
    warnings
}

/// Why a drawing has nothing to cut: the first entity left out, when one
/// was, else that there was none.
fn nothing_to_cut(notes: &[Note]) -> Error {
    notes.first().map_or(Error::Empty, |note| Error::Entity {
        line: note.line,
        entity: note.entity.clone(),
        reason: note.reason.clone(),
    })
}

/// One line per reason and entity type, with how many and where the first is.
fn grouped(notes: &[Note]) -> Vec<String> {
    let mut groups: BTreeMap<(&str, &str), (usize, usize)> = BTreeMap::new();
    for note in notes {
        let group = groups.entry((&note.reason, &note.entity)).or_insert((0, note.line));
        group.0 += 1;
        group.1 = group.1.min(note.line);
    }
    groups
        .into_iter()
        .map(|((reason, entity), (count, line))| match count {
            1 => format!("{entity} at line {line}: {reason}."),
            _ => format!("{count} {entity} entities, the first at line {line}: {reason}."),
        })
        .collect()
}

/// Outlines the text on the chosen layers into `pieces`, in millimetres,
/// and says which styles were substituted; text that cannot be laid out is
/// left out and noted.
fn outline_text(
    placed: &blocks::Placed<'_>,
    chosen: &BTreeSet<String>,
    scale: f64,
    pieces: &mut Vec<Contour>,
    notes: &mut Vec<Note>,
) -> Result<BTreeSet<String>> {
    let mut styles = BTreeSet::new();
    for (text, transform, layer) in &placed.texts {
        if !chosen.contains(&layer.to_uppercase()) {
            continue;
        }
        let outlines = match text.outlines(scale) {
            Ok(outlines) => outlines,
            Err(Error::Entity { entity, line, reason }) => {
                notes.push(Note {
                    entity,
                    line,
                    reason: format!("{reason}; the text was left out"),
                });
                continue;
            }
            Err(other) => return Err(other),
        };
        if text.formatted {
            styles.insert("MTEXT fonts, heights, colours and stacking were left out; the lettering is in Noto Sans.".into());
        }
        // The outlines are in millimetres already; only the move scales.
        let mut millimetres = *transform;
        millimetres.0[4] *= scale;
        millimetres.0[5] *= scale;
        for mut contour in outlines {
            for curve in &mut contour.curves {
                *curve = Curve::Line {
                    start: millimetres.apply(curve.start()),
                    end: millimetres.apply(curve.end()),
                };
            }
            contour.layer.clone_from(layer);
            pieces.push(contour);
        }
        styles.insert(format!(
            "DXF text style {} was outlined using Noto Sans. Check the lettering before cutting.",
            text.style
        ));
    }
    Ok(styles)
}

/// A count as a float.
#[allow(clippy::cast_precision_loss, reason = "counts stay far below 2^53")]
pub(crate) fn float(count: usize) -> f64 {
    count as f64
}

/// The layers that hold geometry, whether the file hides them, and whether
/// they are imported: those named in `chosen`, or every visible one.
fn layers(
    placed: &blocks::Placed,
    table: &BTreeMap<String, bool>,
    chosen: Option<&[String]>,
) -> Vec<Layer> {
    let chosen: Option<BTreeSet<String>> =
        chosen.map(|names| names.iter().map(|n| n.to_uppercase()).collect());
    let mut layers: Vec<Layer> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let names = placed
        .entities
        .iter()
        .map(|(_, _, layer, _)| layer)
        .chain(placed.texts.iter().map(|(_, _, layer)| layer));
    for name in names {
        let key = name.to_uppercase();
        let at = *index.entry(key.clone()).or_insert_with(|| {
            let hidden = table.get(&key).copied().unwrap_or(false);
            let imported = chosen.as_ref().map_or(!hidden, |c| c.contains(&key));
            layers.push(Layer { name: name.clone(), hidden, imported, entities: 0 });
            layers.len() - 1
        });
        layers[at].entities += 1;
    }
    layers
}

/// One placed entity as lines and arcs in millimetres: exact curves under
/// a transform that keeps circles circular, fitted curves otherwise.
fn place(
    entity: &entities::Entity,
    transform: &Transform,
    layer: &str,
    scale: f64,
    tolerance: f64,
) -> Result<Contour> {
    let m = Transform([scale, 0., 0., scale, 0., 0.]).after(transform);
    let error = |reason: &str| Error::Entity {
        line: entity.line,
        entity: entity.name.clone(),
        reason: reason.into(),
    };
    let fitted = |path: &Parametric| {
        let (from, to) = path.domain();
        fit::approximate(|t| m.apply(path.at(t)), from, to, tolerance, path.closed())
            .ok_or_else(|| error("the curve cannot be fitted with lines and arcs"))
    };
    let mut curves = Vec::new();
    for piece in &entity.pieces {
        match piece {
            Piece::Curve(curve) if m == Transform::IDENTITY => curves.push(*curve),
            Piece::Curve(line @ Curve::Line { .. }) => curves.push(m.curve(line)),
            Piece::Curve(arc) if m.is_similarity() => curves.push(m.curve(arc)),
            Piece::Curve(arc) => curves.extend(fitted(&Parametric::Arc(*arc))?),
            Piece::Path(path) => curves.extend(fitted(path)?),
            Piece::Points(points) => {
                let points: Vec<Point> = points.iter().map(|p| m.apply(*p)).collect();
                fit::fit_points(&points, tolerance, &mut curves);
            }
        }
    }
    if curves.is_empty() || !curves.iter().all(Curve::is_valid) {
        return Err(error(entities::EMPTY));
    }
    Ok(Contour { layer: layer.to_owned(), curves })
}

/// The sections of a file: its units, layer table, blocks and entities.
struct Sections<'a> {
    pairs: &'a [pairs::Pair<'a>],
    at: usize,
    section: &'a str,
    units: Units,
    entities: Read,
    /// Whether each layer, by upper-case name, is turned off or frozen.
    layers: BTreeMap<String, bool>,
    /// Each layer's colour, by upper-case name.
    colors: BTreeMap<String, [u8; 3]>,
    blocks: BTreeMap<String, blocks::Block>,
    /// The block being read.
    block: Option<(String, blocks::Block)>,
}

impl<'a> Sections<'a> {
    fn read(pairs: &'a [pairs::Pair<'a>]) -> Result<Self> {
        let mut reader = Self {
            pairs,
            at: 0,
            section: "",
            units: Units::Unspecified,
            entities: Read::default(),
            layers: BTreeMap::new(),
            colors: BTreeMap::new(),
            blocks: BTreeMap::new(),
            block: None,
        };
        while reader.at < pairs.len() {
            if reader.control()? || reader.header()? {
                continue;
            }
            reader.at += 1;
        }
        Ok(reader)
    }

    /// Section boundaries and entity records advance the same input cursor.
    fn control(&mut self) -> Result<bool> {
        let pair = self.pairs[self.at];
        if pair.code != 0 {
            return Ok(false);
        }
        match pair.value.to_ascii_uppercase().as_str() {
            "SECTION" => {
                let name =
                    self.pairs.get(self.at + 1).filter(|p| p.code == 2).ok_or(Error::Syntax {
                        line: pair.line,
                        reason: "SECTION must be followed by its name",
                    })?;
                self.section = name.value;
                self.at += 2;
            }
            "ENDSEC" => {
                self.section = "";
                self.at += 1;
            }
            "EOF" => self.at = self.pairs.len(),
            _ if self.section.eq_ignore_ascii_case("ENTITIES") => {
                self.at = entities::read(self.pairs, self.at, &mut self.entities)?;
            }
            "LAYER" if self.section.eq_ignore_ascii_case("TABLES") => self.layer()?,
            "BLOCK" if self.section.eq_ignore_ascii_case("BLOCKS") => self.begin_block()?,
            "ENDBLK" if self.section.eq_ignore_ascii_case("BLOCKS") => {
                if let Some((name, block)) = self.block.take() {
                    self.blocks.insert(name, block);
                }
                self.at = pairs::next_entity(self.pairs, self.at + 1);
            }
            _ if self.section.eq_ignore_ascii_case("BLOCKS") => match &mut self.block {
                Some((_, block)) => {
                    self.at = entities::read(self.pairs, self.at, &mut block.read)?;
                }
                None => self.at = pairs::next_entity(self.pairs, self.at + 1),
            },
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn fields(&self) -> (entities::Fields<'a>, usize) {
        let end = pairs::next_entity(self.pairs, self.at + 1);
        let pair = self.pairs[self.at];
        (
            entities::Fields {
                pairs: &self.pairs[self.at + 1..end],
                line: pair.line,
                entity: pair.value,
            },
            end,
        )
    }

    /// A layer table entry: off when its colour is negative, frozen by flag.
    fn layer(&mut self) -> Result<()> {
        let (fields, end) = self.fields();
        if let Some(name) = fields.one(2)? {
            let off = fields.number(62)?.is_some_and(|colour| colour < 0.);
            let frozen = fields.flags(70)? & 1 != 0;
            self.layers.insert(name.to_uppercase(), off || frozen);
            if let colors::Color::Rgb(rgb) =
                colors::from_groups(fields.number(62)?, fields.number(420)?)
            {
                self.colors.insert(name.to_uppercase(), rgb);
            }
        }
        self.at = end;
        Ok(())
    }

    /// A block definition: its name, base point and whether it is external.
    fn begin_block(&mut self) -> Result<()> {
        let (fields, end) = self.fields();
        let name = fields.one(2)?.unwrap_or_default().to_uppercase();
        let base = Point::new(fields.number(10)?.unwrap_or(0.), fields.number(20)?.unwrap_or(0.));
        let external = fields.flags(70)? & 4 != 0;
        self.block = Some((name, blocks::Block { base, external, read: Read::default() }));
        self.at = end;
        Ok(())
    }

    fn header(&mut self) -> Result<bool> {
        let pair = self.pairs[self.at];
        if self.section.eq_ignore_ascii_case("HEADER")
            && pair.code == 9
            && pair.value.eq_ignore_ascii_case("$INSUNITS")
            && let Some(code) = self.pairs.get(self.at + 1).filter(|p| p.code == 70)
        {
            self.units = declared_units(code.value)?;
            self.at += 2;
            return Ok(true);
        }
        Ok(false)
    }
}

fn declared_units(value: &str) -> Result<Units> {
    match value.trim().parse::<u32>() {
        Ok(0) | Err(_) => Ok(Units::Unspecified),
        Ok(1) => Ok(Units::Inches),
        Ok(2) => Ok(Units::Feet),
        Ok(4) => Ok(Units::Millimeters),
        Ok(5) => Ok(Units::Centimeters),
        Ok(6) => Ok(Units::Meters),
        Ok(9) => Ok(Units::Mils),
        Ok(10) => Ok(Units::Yards),
        Ok(13) => Ok(Units::Micrometers),
        Ok(14) => Ok(Units::Decimeters),
        Ok(other) => Err(Error::Units(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(entities: &str, units: &str) -> Vec<u8> {
        format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{units}\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
        )
        .into_bytes()
    }

    /// Layers keep the table's colour; an entity of another colour than its
    /// layer's is on a layer of its own, so it never joins its neighbours.
    #[test]
    fn colours_come_from_the_layer_table_and_entities_of_their_own_colour() {
        let text = "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n0\nLAYER\n2\nCUT\n70\n0\n62\n1\n0\nENDTAB\n0\nENDSEC\n\
            0\nSECTION\n2\nENTITIES\n\
            0\nLINE\n8\nCUT\n10\n0\n20\n0\n11\n10\n21\n0\n\
            0\nLINE\n8\nCUT\n62\n5\n10\n10\n20\n0\n11\n10\n21\n10\n\
            0\nLINE\n8\nCUT\n62\n1\n10\n10\n20\n10\n11\n0\n21\n10\n\
            0\nCIRCLE\n8\nMARK\n420\n1193046\n10\n5\n20\n5\n40\n1\n0\nENDSEC\n0\nEOF\n";
        let import = import(text.as_bytes()).unwrap();
        let layers: Vec<_> = import.drawing.contours.iter().map(|c| c.layer.as_str()).collect();
        assert_eq!(layers.iter().filter(|l| **l == "CUT Blue").count(), 1, "{layers:?}");
        assert!(layers.contains(&"MARK #123456"));
        // The red lines meet only through the blue one, so they stay apart.
        assert_eq!(import.drawing.contours.iter().filter(|c| c.layer == "CUT").count(), 2);
        assert_eq!(
            import.colors,
            [
                ("CUT".to_owned(), [255, 0, 0]),
                ("CUT Blue".to_owned(), [0, 0, 255]),
                ("MARK #123456".to_owned(), [18, 52, 86])
            ]
        );
    }

    /// A drawing of only what we cannot cut is refused by name, and files that are not
    /// ASCII pairs, have no geometry, or use other units are refused too.
    #[test]
    fn unsupported_files_are_refused_with_the_reason() {
        for (entity, name) in [
            ("0\nMLINE\n8\n0\n", "MLINE"),
            ("0\n3DSOLID\n8\n0\n", "3DSOLID"),
            ("0\nREGION\n8\n0\n", "REGION"),
        ] {
            // Alone, it leaves nothing to cut; the refusal names it.
            let error = import(&file(entity, "4")).unwrap_err();
            assert!(
                matches!(&error, Error::Entity { line: 15, entity, .. } if entity == name),
                "{error:?}"
            );
        }
        assert_eq!(import(b"AutoCAD Binary DXF\r\n\x1a\0").unwrap_err(), Error::Empty);
        assert_eq!(
            import(b"0\nSECTION\n2\n").unwrap_err(),
            Error::Syntax { line: 3, reason: "a group code needs a value line" }
        );
        assert_eq!(import(&file("0\nPOINT\n8\n0\n10\n0\n20\n0\n", "4")).unwrap_err(), Error::Empty);
        assert_eq!(
            import(&file("0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n", "3")).unwrap_err(),
            Error::Units(3)
        );
        let missing = import(&file("0\nINSERT\n8\n0\n2\nNOWHERE\n10\n0\n20\n0\n", "4"));
        assert!(matches!(missing, Err(Error::Block { line: 15, .. })), "{missing:?}");
        let options = Options { tolerance: 0., ..Options::default() };
        assert!(matches!(import_with(&file("", "4"), &options), Err(Error::Options(_))));
    }

    /// Values that make an entity uncuttable as drawn are refused: a line
    /// off the XY plane, a polyline with the wrong vertex count. Shapes with
    /// no size are left out, and refused only when nothing else is left.
    #[test]
    fn bad_entity_values_are_refused() {
        for entity in [
            "0\nLINE\n10\n0\n20\n0\n30\n1\n11\n1\n21\n1\n",
            "0\nLWPOLYLINE\n90\n3\n70\n0\n10\n0\n20\n0\n10\n1\n20\n1\n",
            "0\nARC\n10\n0\n20\n0\n40\n1\n50\n30\n51\n30\n",
            "0\nCIRCLE\n10\n0\n20\n0\n40\n0\n",
            "0\nLINE\n10\n0\n20\n0\n11\n0\n21\n0\n",
        ] {
            let result = import(&file(entity, "4"));
            assert!(matches!(result, Err(Error::Entity { line: 15, .. })), "{entity}: {result:?}");
        }
    }

    /// A zero-length line, a wide polyline, metres and an unreadable text
    /// beside real geometry: the drawing opens, and says what it did.
    #[test]
    fn what_cannot_be_cut_as_drawn_is_noted_not_refused() {
        let line = "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n";
        let empty = "0\nLINE\n10\n5\n20\n5\n11\n5\n21\n5\n";
        let wide = "0\nLWPOLYLINE\n90\n2\n70\n0\n43\n0.5\n10\n0\n20\n2\n10\n1\n20\n2\n";
        let fitted = "0\nTEXT\n10\n0\n20\n0\n40\n1\n1\nH\n72\n5\n";
        let import = import(&file(&format!("{line}{empty}{empty}{wide}{fitted}"), "6")).unwrap();
        assert!((import.drawing.length() - 2000.).abs() < 1e-9, "metres become millimetres");
        let said = import.warnings.join("\n");
        assert!(said.contains("in metres"), "{said}");
        assert!(said.contains("2 LINE entities, the first at line 25: it has no length"), "{said}");
        assert!(
            said.contains("LWPOLYLINE at line 45: it is drawn with width; its centreline is cut"),
            "{said}"
        );
        assert!(
            said.contains(
                "TEXT at line 61: aligned or fitted text is not supported; the text was left out"
            ),
            "{said}"
        );
    }

    /// A byte order mark, Windows line endings, lower-case names and
    /// indented group codes are all read.
    #[test]
    fn tolerant_of_encodings_and_case() {
        let text = "\u{feff}  0\r\nsection\r\n  2\r\nentities\r\n  0\r\nline\r\n 10\r\n0\r\n 20\r\n0\r\n 11\r\n5\r\n 21\r\n0\r\n  0\r\nendsec\r\n  0\r\neof\r\n";
        let import = import(text.as_bytes()).unwrap();
        assert_eq!(import.units, Units::Unspecified);
        assert_eq!(import.entities, 1);
        assert!((import.drawing.length() - 5.).abs() < 1e-12);
    }

    #[test]
    fn polyline_vertices_share_planar_zero_width_admission() {
        let line = "0\nLINE\n10\n0\n20\n0\n11\n10\n21\n0\n";
        let light =
            "0\nLWPOLYLINE\n90\n2\n10\n0\n20\n0\n40\n0\n41\n0\n10\n10\n20\n0\n40\n0\n41\n0\n";
        let heavy = |header: &str, vertex: &str| {
            format!(
                "0\nPOLYLINE\n{header}0\nVERTEX\n10\n0\n20\n0\n{vertex}0\nVERTEX\n10\n10\n20\n0\n0\nSEQEND\n"
            )
        };
        let expected = import(&file(line, "4")).unwrap().drawing;
        assert_eq!(import(&file(light, "4")).unwrap().drawing, expected);
        assert_eq!(import(&file(&heavy("", "40\n0\n41\n0\n"), "4")).unwrap().drawing, expected);
        for header in ["30\n1\n", "39\n1\n", "230\n0.5\n", "70\n8\n", "70\n16\n"] {
            assert!(
                matches!(import(&file(&heavy(header, ""), "4")), Err(Error::Entity { entity, .. }) if entity == "POLYLINE")
            );
        }
        for vertex in ["30\n1\n", "39\n1\n", "70\n1\n", "70\n8\n"] {
            assert!(import(&file(&heavy("", vertex), "4")).is_err(), "{vertex}");
        }
        // Width is how the line is drawn: the centreline is cut, and said so.
        for wide in [
            import(&file(&heavy("40\n1\n", ""), "4")).unwrap(),
            import(&file(&heavy("", "41\n1\n"), "4")).unwrap(),
            import(&file(&light.replacen("40\n0", "40\n1", 1), "4")).unwrap(),
        ] {
            assert_eq!(wide.drawing, expected);
            assert!(wide.warnings.iter().any(|w| w.contains("centreline")), "{:?}", wide.warnings);
        }
        assert!(import(&file(&light.replacen("20\n0", "20\n0\n20\n1", 1), "4")).is_err());
    }
}
