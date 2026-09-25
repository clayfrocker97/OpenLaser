// SPDX-License-Identifier: GPL-3.0-or-later

//! Block references, expanded into the entities they place.
//!
//! A reference places its block's entities through its own insertion point,
//! rotation and scale, once for each copy of its array, after moving the
//! block's base point to the origin. References nest. A negative scale
//! mirrors, and the arcs it mirrors run the other way. An entity on layer
//! `0` takes the layer of the reference that places it.

use crate::colors::Color;
use crate::entities::text::TextEntity;
use crate::entities::{Entity, Insert, Note, Read};
use crate::{Error, Result, Skipped};
use openlaser_core::geometry::{Point, Transform};
use std::collections::BTreeMap;

/// The deepest block nesting followed.
const MAX_DEPTH: usize = 32;

/// The most entities references may place.
const MAX_PLACED: usize = 1_000_000;

/// A block definition.
pub(crate) struct Block {
    /// The point of the block placed at a reference's insertion point.
    pub base: Point,
    /// Whether the block is an external reference, whose geometry lives in
    /// another file.
    pub external: bool,
    /// Its entities.
    pub read: Read,
}

/// Every entity of the drawing where it lies, and on which layer.
#[derive(Default)]
pub(crate) struct Placed<'a> {
    /// Each entity with where it lies, its layer, and its own colour when
    /// it has one rather than its layer's.
    pub entities: Vec<(&'a Entity, Transform, String, Option<[u8; 3]>)>,
    pub texts: Vec<(&'a TextEntity, Transform, String)>,
    pub skipped: Vec<Skipped>,
    pub notes: Vec<Note>,
    pub paper: usize,
    pub invisible: usize,
}

/// Places the drawing's own entities and everything its references place.
pub(crate) fn expand<'a>(top: &'a Read, blocks: &'a BTreeMap<String, Block>) -> Result<Placed<'a>> {
    let mut placed = Placed {
        skipped: top.skipped.clone(),
        notes: top.notes.clone(),
        paper: top.paper,
        invisible: top.invisible,
        ..Placed::default()
    };
    let mut expansion =
        Expansion { blocks, placed: &mut placed, stack: Vec::new(), used: Vec::new() };
    expansion.read(top, &Transform::IDENTITY, None, None)?;
    Ok(placed)
}

struct Expansion<'a, 'p> {
    blocks: &'a BTreeMap<String, Block>,
    placed: &'p mut Placed<'a>,
    /// The blocks being expanded, to refuse a block that places itself.
    stack: Vec<String>,
    /// The blocks already counted for skipped entities.
    used: Vec<String>,
}

impl<'a> Expansion<'a, '_> {
    /// Places what `read` holds through `transform`; entities on layer `0`
    /// take `inherited` when a reference places them, and entities coloured
    /// by block take the reference's colour, `block`.
    fn read(
        &mut self,
        read: &'a Read,
        transform: &Transform,
        inherited: Option<&str>,
        block: Option<[u8; 3]>,
    ) -> Result<()> {
        let layer = |own: &str| match inherited {
            Some(outer) if own == "0" => outer.to_owned(),
            _ => own.to_owned(),
        };
        let color = |own: Color| match own {
            Color::Rgb(rgb) => Some(rgb),
            Color::ByBlock => block,
            Color::ByLayer => None,
        };
        for entity in &read.entities {
            let placed = (entity, *transform, layer(&entity.layer), color(entity.color));
            self.placed.entities.push(placed);
        }
        for (text, own) in &read.texts {
            self.placed.texts.push((text, transform.after(own), layer(&text.layer)));
        }
        if self.placed.entities.len() + self.placed.texts.len() > MAX_PLACED {
            return Err(Error::Block {
                line: read.inserts.first().map_or(0, |i| i.line),
                reason: "block references place more than a million entities".into(),
            });
        }
        for insert in &read.inserts {
            let layer = layer(&insert.layer);
            self.insert(insert, transform, &layer, color(insert.color))?;
        }
        Ok(())
    }

    fn insert(
        &mut self,
        insert: &'a Insert,
        outer: &Transform,
        layer: &str,
        color: Option<[u8; 3]>,
    ) -> Result<()> {
        let key = insert.block.to_uppercase();
        let block = self.blocks.get(&key).ok_or_else(|| Error::Block {
            line: insert.line,
            reason: format!("block {} is not defined", insert.block),
        })?;
        if block.external {
            self.placed.skipped.push(Skipped {
                entity: format!("INSERT of external reference {}", insert.block),
                line: insert.line,
            });
            return Ok(());
        }
        if self.stack.contains(&key) {
            return Err(Error::Block {
                line: insert.line,
                reason: format!("block {} places itself", insert.block),
            });
        }
        if self.stack.len() >= MAX_DEPTH {
            return Err(Error::Block {
                line: insert.line,
                reason: format!("blocks nest more than {MAX_DEPTH} deep"),
            });
        }
        if !self.used.contains(&key) {
            self.used.push(key.clone());
            self.placed.skipped.extend(block.read.skipped.iter().cloned());
            self.placed.notes.extend(block.read.notes.iter().cloned());
            self.placed.paper += block.read.paper;
            self.placed.invisible += block.read.invisible;
        }
        self.stack.push(key);
        let base = Transform::translation(-block.base);
        for copy in &insert.copies {
            let transform = outer.after(&copy.after(&base));
            self.read(&block.read, &transform, Some(layer), color)?;
        }
        self.stack.pop();
        Ok(())
    }
}
