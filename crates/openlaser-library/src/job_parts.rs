// SPDX-License-Identifier: GPL-3.0-or-later

//! The parts a job cuts. A job's drawing is its parts' drawings joined in
//! order, so every contour index in a job, placed or referenced by its
//! stock, names one contour of one part. Adding a part appends its contours
//! and never renumbers those before it.

use crate::{Error, Id, Library, Result};
use openlaser_core::geometry::Drawing;
use std::ops::Range;
use std::sync::Arc;

/// The most parts one job can cut, as many as one nesting search places.
pub const MAX_PARTS: usize = 500;

/// A job's drawing: its parts' drawings joined in order.
#[derive(Clone, Debug)]
pub struct JobDrawing {
    parts: Vec<(Id, Arc<Drawing>)>,
    /// Where each part's contours start in the joined drawing.
    starts: Vec<usize>,
    drawing: Arc<Drawing>,
}

impl JobDrawing {
    /// Joins the parts' drawings in order. One part shares its drawing.
    #[must_use]
    pub fn new(parts: Vec<(Id, Arc<Drawing>)>) -> Self {
        let mut starts = Vec::with_capacity(parts.len());
        let mut count = 0;
        for (_, drawing) in &parts {
            starts.push(count);
            count += drawing.contours.len();
        }
        let drawing = match parts.as_slice() {
            [(_, only)] => only.clone(),
            _ => Arc::new(Drawing {
                contours: parts.iter().flat_map(|(_, d)| d.contours.iter().cloned()).collect(),
            }),
        };
        Self { parts, starts, drawing }
    }

    /// The parts, in the order their contours follow each other.
    #[must_use]
    pub fn parts(&self) -> impl ExactSizeIterator<Item = &Id> {
        self.parts.iter().map(|(id, _)| id)
    }

    /// Whether this is the drawing of exactly `parts`, in that order.
    #[must_use]
    pub fn is_of(&self, parts: &[Id]) -> bool {
        self.parts.len() == parts.len() && self.parts().zip(parts).all(|(a, b)| a == b)
    }

    /// The joined drawing.
    #[must_use]
    pub const fn drawing(&self) -> &Arc<Drawing> {
        &self.drawing
    }

    /// Every contour of every part.
    #[must_use]
    pub fn contours(&self) -> usize {
        self.drawing.contours.len()
    }

    /// The drawing of the part at `index`, as the library holds it.
    #[must_use]
    pub fn part_drawing(&self, index: usize) -> Option<&Arc<Drawing>> {
        self.parts.get(index).map(|(_, drawing)| drawing)
    }

    /// The contours of the part at `index`.
    #[must_use]
    pub fn range(&self, index: usize) -> Range<usize> {
        let start = self.starts.get(index).copied().unwrap_or(self.contours());
        let end = self.starts.get(index + 1).copied().unwrap_or(self.contours());
        start..end
    }

    /// Which part a joined contour belongs to.
    #[must_use]
    pub fn part_of(&self, source: usize) -> Option<usize> {
        if source >= self.contours() {
            return None;
        }
        Some(self.starts.partition_point(|&start| start <= source) - 1)
    }

    /// The drawing of only the parts `keep` marks, and where each of this
    /// drawing's contours lies in it; the others' contours map to nothing.
    #[must_use]
    pub fn retain(&self, keep: &[bool]) -> (Self, Vec<Option<usize>>) {
        let mut map = vec![None; self.contours()];
        let mut next = 0;
        let mut parts = Vec::new();
        for (index, part) in self.parts.iter().enumerate() {
            if keep.get(index).copied().unwrap_or(true) {
                for (source, slot) in self.range(index).zip(&mut map[self.range(index)]) {
                    *slot = Some(next + source - self.starts[index]);
                }
                next += part.1.contours.len();
                parts.push(part.clone());
            }
        }
        (Self::new(parts), map)
    }
}

/// Checks a job's parts: one to [`MAX_PARTS`], each once, each in the
/// library. Their total contour count.
pub(crate) fn contour_count(parts: &[Id], library: &Library) -> Result<usize> {
    if parts.is_empty() || parts.len() > MAX_PARTS {
        return Err(Error::Invalid(format!("a job cuts 1 to {MAX_PARTS} parts")));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut count = 0;
    for id in parts {
        if !seen.insert(id) {
            return Err(Error::Invalid(format!("the job lists part {id} twice")));
        }
        count += library.part(id)?.drawing.contours.len();
    }
    Ok(count)
}

impl Library {
    /// The drawing of a job cutting `parts`.
    pub fn job_drawing(&self, parts: &[Id]) -> Result<JobDrawing> {
        contour_count(parts, self)?;
        let parts = parts
            .iter()
            .map(|id| Ok((id.clone(), self.part(id)?.drawing.clone())))
            .collect::<Result<Vec<_>>>()?;
        Ok(JobDrawing::new(parts))
    }
}

/// The stored form of a job's parts, as serde `with`: one part as a bare
/// id, which is how every job was stored before a job could cut several,
/// so older builds keep reading single-part jobs; several as a list.
pub mod stored_parts {
    use crate::Id;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Writes one part as its id and several as a list.
    pub fn serialize<S: Serializer>(parts: &[Id], serializer: S) -> Result<S::Ok, S::Error> {
        match parts {
            [one] => one.serialize(serializer),
            several => several.serialize(serializer),
        }
    }

    /// Reads a bare id as one part, or a list.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Id>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged, expecting = "a part id or a list of part ids")]
        enum Stored {
            One(Id),
            Several(Vec<Id>),
        }
        Ok(match Stored::deserialize(deserializer)? {
            Stored::One(id) => vec![id],
            Stored::Several(ids) => ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::{Contour, Curve, Point};

    fn lines(count: usize) -> Arc<Drawing> {
        Arc::new(Drawing {
            contours: (0..count)
                .map(|i| {
                    let y = f64::from(u32::try_from(i).unwrap());
                    Contour {
                        layer: "0".into(),
                        curves: vec![Curve::Line {
                            start: Point::new(0., y),
                            end: Point::new(1., y),
                        }],
                    }
                })
                .collect(),
        })
    }

    #[test]
    fn parts_join_in_order_and_one_part_shares_its_drawing() {
        let only = lines(2);
        let single = JobDrawing::new(vec![(Id::from("a"), only.clone())]);
        assert!(Arc::ptr_eq(single.drawing(), &only));
        let joined = JobDrawing::new(vec![
            (Id::from("a"), lines(2)),
            (Id::from("b"), lines(3)),
            (Id::from("c"), lines(1)),
        ]);
        assert_eq!(joined.contours(), 6);
        assert_eq!((joined.range(0), joined.range(1), joined.range(2)), (0..2, 2..5, 5..6));
        assert_eq!(
            (0..7).map(|s| joined.part_of(s)).collect::<Vec<_>>(),
            [Some(0), Some(0), Some(1), Some(1), Some(1), Some(2), None]
        );
        assert!(joined.is_of(&[Id::from("a"), Id::from("b"), Id::from("c")]));
        assert!(!joined.is_of(&[Id::from("a"), Id::from("b")]));
        let (kept, map) = joined.retain(&[true, false, true]);
        assert!(kept.is_of(&[Id::from("a"), Id::from("c")]));
        assert_eq!(map, [Some(0), Some(1), None, None, None, Some(2)]);
        assert_eq!(kept.drawing().contours[2], joined.drawing().contours[5]);
    }

    #[test]
    fn one_part_is_stored_as_before_and_several_as_a_list() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Item {
            #[serde(rename = "part", with = "stored_parts")]
            parts: Vec<Id>,
        }
        let one = Item { parts: vec![Id::from("a")] };
        assert_eq!(serde_json::to_string(&one).unwrap(), r#"{"part":"a"}"#);
        let several = Item { parts: vec![Id::from("a"), Id::from("b")] };
        assert_eq!(serde_json::to_string(&several).unwrap(), r#"{"part":["a","b"]}"#);
        for item in [one, several] {
            let text = serde_json::to_string(&item).unwrap();
            assert_eq!(serde_json::from_str::<Item>(&text).unwrap(), item);
        }
        assert!(serde_json::from_str::<Item>(r#"{"part":3}"#).is_err());
    }
}
