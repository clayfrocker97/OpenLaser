// SPDX-License-Identifier: GPL-3.0-or-later

//! The parts a job cuts. A job opens from one or more library parts laid
//! out side by side, and more can be added to it later; its drawing is
//! theirs joined in order (`openlaser_library::JobDrawing`), attached to
//! the draft whenever its parts change.

use crate::coordinator::Coordinator;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::geometry::{Bounds, MAX_PLACED_CONTOURS, Placed, Point, Transform};
use openlaser_library::{Id, JobDrawing};
use std::ops::Range;
use std::sync::Arc;

/// The gap left between parts laid out side by side, in millimetres.
const GAP: f64 = 10.;

impl Coordinator {
    /// Starts a fresh job draft cutting one part.
    pub fn open_part(&mut self, part: &Id) -> Result<()> {
        self.open_parts(std::slice::from_ref(part))
    }

    /// Starts a fresh job draft cutting `parts`, laid out side by side in
    /// the order given. Unfinished setups remain available through pending
    /// changes, not through the part cards.
    pub fn open_parts(&mut self, parts: &[Id]) -> Result<()> {
        let sources = Arc::new(self.library.job_drawing(parts)?);
        if sources.contours() > MAX_PLACED_CONTOURS {
            return Err(Error::Request(format!(
                "these parts hold more than {MAX_PLACED_CONTOURS} contours; set them up in smaller jobs"
            )));
        }
        let correction = if self.any_dxf(parts) {
            self.mode.and_then(|mode| self.correction.active(mode))
        } else {
            None
        };
        let mut draft = Draft::new(sources.clone());
        draft.current.placed = arrange(&sources, 0..parts.len(), None, self.row_width(), 0..);
        draft.current.correction = correction;
        self.leave_draft();
        self.draft_generation += 1;
        self.draft = Some(draft);
        self.reprepare();
        Ok(())
    }

    /// Adds parts to the open job, beside what is on the sheet, as one
    /// undoable edit.
    pub fn add_parts(&mut self, parts: &[Id]) -> Result<()> {
        let draft =
            self.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        if parts.is_empty() {
            return Err(Error::Request("choose the parts to add".into()));
        }
        if let Some(id) = parts.iter().find(|id| draft.current.parts.contains(id)) {
            let name = self.library.part(id).map_or_else(|_| id.to_string(), |p| p.name.clone());
            return Err(Error::Request(format!(
                "{name} is already in this job; copy it on the sheet for more"
            )));
        }
        let all: Vec<Id> = draft.current.parts.iter().chain(parts).cloned().collect();
        let sources = Arc::new(self.library.job_drawing(&all)?);
        let added = sources.contours() - draft.source_count;
        if added > MAX_PLACED_CONTOURS.saturating_sub(draft.current.placed.len()) {
            return Err(Error::Request(format!(
                "the layout would exceed {MAX_PLACED_CONTOURS} contours"
            )));
        }
        let beside = crate::draft::place(draft.drawing()?, &draft.current.placed).bounds();
        let used: std::collections::BTreeSet<u32> =
            draft.current.placed.iter().map(|p| p.copy).collect();
        let copies = (0u32..).filter(move |copy| !used.contains(copy));
        let placed = arrange(
            &sources,
            draft.current.parts.len()..all.len(),
            beside,
            self.row_width(),
            copies,
        );
        let correction = (draft.job.is_none()
            && !draft.calibration
            && draft.current.correction.is_none()
            && self.any_dxf(parts))
        .then(|| {
            let laser = draft.current.recipe.as_ref().map(|r| r.laser).or(self.mode)?;
            self.correction.active(laser)
        })
        .flatten();
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        draft.add_parts(sources, placed)?;
        if correction.is_some() {
            draft.current.correction = correction;
        }
        self.reprepare();
        Ok(())
    }

    /// Attaches the drawing of the open draft's parts, when they changed.
    pub(crate) fn attach_draft(&mut self) -> Result<()> {
        let Some(mut draft) = self.draft.take() else { return Ok(()) };
        let attached = self.attach(&mut draft);
        self.draft = Some(draft);
        attached
    }

    /// Attaches the drawing of a draft's parts, unless it already has it.
    pub(crate) fn attach(&self, draft: &mut Draft) -> Result<()> {
        if draft.sources().is_none() {
            draft.attach(Arc::new(self.library.job_drawing(&draft.current.parts)?))?;
        }
        Ok(())
    }

    /// The saved job's name, or the name of the parts a new job cuts.
    pub(crate) fn draft_name(&self, draft: &Draft) -> String {
        if let Some(job) = draft.job.as_ref().and_then(|id| self.library.job(id).ok()) {
            return job.name.clone();
        }
        if let Some(base) = &draft.saved_base {
            return base.name.clone();
        }
        let names: Vec<&str> = draft
            .current
            .parts
            .iter()
            .filter_map(|id| self.library.part(id).ok())
            .map(|part| part.name.as_str())
            .collect();
        name(&names)
    }

    /// Whether any of the parts came from a DXF drawing, which matrix
    /// correction applies to.
    pub(crate) fn any_dxf(&self, parts: &[Id]) -> bool {
        parts
            .iter()
            .filter_map(|id| self.library.part(id).ok())
            .any(|part| part.file_name.to_ascii_lowercase().ends_with(".dxf"))
    }

    /// How wide a row of parts laid out side by side may grow: the bed's
    /// travel, when it is known.
    fn row_width(&self) -> Option<f64> {
        self.extent().map(|e| e[0][1] - e[0][0]).filter(|width| *width > 0.)
    }
}

/// A job's name from its parts' names: all of a few short names, or the
/// first and how many more.
#[must_use]
pub fn name(names: &[&str]) -> String {
    match names {
        [] => "job".into(),
        [one] => (*one).to_owned(),
        few if few.len() <= 3 && few.iter().map(|n| n.chars().count() + 3).sum::<usize>() <= 51 => {
            few.join(" + ")
        }
        [first, rest @ ..] => format!("{first} + {} more", rest.len()),
    }
}

/// Places the contours of the parts at `parts` in rows, each part `GAP`
/// from the one before and every part its own copy. The first row
/// continues to the right of `beside`, or of the first part where it is
/// drawn; a row wraps once it would grow wider than `width`, and the next
/// row starts above everything before it.
fn arrange(
    sources: &JobDrawing,
    parts: Range<usize>,
    beside: Option<Bounds>,
    width: Option<f64>,
    mut copies: impl Iterator<Item = u32>,
) -> Vec<Placed> {
    let mut shelf: Option<Shelf> = beside.map(|b| Shelf::after(b, width));
    let mut placed = Vec::new();
    for part in parts {
        let range = sources.range(part);
        let copy = copies.next().unwrap_or(u32::MAX);
        let offset = match (sources.part_drawing(part).and_then(|d| d.bounds()), &mut shelf) {
            (Some(bounds), Some(shelf)) => shelf.put(bounds),
            (Some(bounds), None) => {
                shelf = Some(Shelf::after(bounds, width));
                Point::new(0., 0.)
            }
            (None, _) => Point::new(0., 0.),
        };
        let transform = Transform::translation(offset);
        placed.extend(range.map(|source| Placed { source, copy, transform }));
    }
    placed
}

/// Rows of parts, filled left to right and stacked upwards.
struct Shelf {
    left: f64,
    limit: Option<f64>,
    next: f64,
    row: f64,
    top: f64,
}

impl Shelf {
    /// The rows continue to the right of what `bounds` holds.
    fn after(bounds: Bounds, width: Option<f64>) -> Self {
        Self {
            left: bounds.min.x,
            limit: width.map(|w| bounds.min.x + w),
            next: bounds.max.x + GAP,
            row: bounds.min.y,
            top: bounds.max.y,
        }
    }

    /// Where a part of `bounds` moves by to take the next place.
    fn put(&mut self, bounds: Bounds) -> Point {
        let size = bounds.max - bounds.min;
        if self.limit.is_some_and(|limit| self.next + size.x > limit) && self.next > self.left {
            self.next = self.left;
            self.row = self.top + GAP;
        }
        let offset = Point::new(self.next - bounds.min.x, self.row - bounds.min.y);
        self.next += size.x + GAP;
        self.top = self.top.max(self.row + size.y);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::{Contour, Curve, Drawing};

    fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Contour {
        let corners = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        Contour {
            layer: "0".into(),
            curves: (0..4)
                .map(|i| Curve::Line {
                    start: Point::from(corners[i]),
                    end: Point::from(corners[(i + 1) % 4]),
                })
                .collect(),
        }
    }

    fn sources(parts: Vec<Vec<Contour>>) -> JobDrawing {
        JobDrawing::new(
            parts
                .into_iter()
                .enumerate()
                .map(|(i, contours)| {
                    (Id::from(format!("p{i}").as_str()), Arc::new(Drawing { contours }))
                })
                .collect(),
        )
    }

    fn bounds_of(sources: &JobDrawing, placed: &[Placed], part: usize) -> Bounds {
        let range = sources.range(part);
        let mine: Vec<_> = placed.iter().filter(|p| range.contains(&p.source)).copied().collect();
        crate::draft::place(sources.drawing(), &mine).bounds().unwrap()
    }

    /// The first part stays where it is drawn; the others follow it in a
    /// row 10 mm apart, each its own copy, until the bed's width wraps them
    /// onto a row above.
    #[test]
    fn parts_open_side_by_side_and_wrap_at_the_bed() {
        let joined = sources(vec![
            vec![rectangle(5., 5., 100., 50.), rectangle(20., 20., 10., 10.)],
            vec![rectangle(-300., 40., 80., 80.)],
            vec![rectangle(0., 0., 60., 30.)],
        ]);
        let placed = arrange(&joined, 0..3, None, Some(250.), 0..);
        assert_eq!(placed.len(), 4);
        assert_eq!(placed.iter().map(|p| p.copy).collect::<Vec<_>>(), [0, 0, 1, 2]);
        assert_eq!(placed[0].transform, Transform::IDENTITY);
        let [a, b, c] = [0, 1, 2].map(|part| bounds_of(&joined, &placed, part));
        assert_eq!((b.min.x, b.min.y), (a.max.x + GAP, a.min.y));
        // The third part would end at x = 265, past the 250 mm bed from x = 5.
        assert_eq!((c.min.x, c.min.y), (a.min.x, b.max.y + GAP));
        let unbounded = arrange(&joined, 0..3, None, None, 0..);
        let c = bounds_of(&joined, &unbounded, 2);
        assert!((c.min.y - a.min.y).abs() < 1e-9, "without a known bed, one row");
    }

    /// Added parts continue beside the layout already on the sheet, with
    /// copies it does not use.
    #[test]
    fn added_parts_go_beside_the_sheet() {
        let joined =
            sources(vec![vec![rectangle(0., 0., 10., 10.)], vec![rectangle(0., 0., 20., 5.)]]);
        let sheet = Bounds { min: Point::new(-50., 100.), max: Point::new(150., 300.) };
        let placed = arrange(&joined, 1..2, Some(sheet), None, [7].into_iter());
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].copy, 7);
        let b = bounds_of(&joined, &placed, 1);
        assert_eq!((b.min.x, b.min.y), (160., 100.));
    }

    #[test]
    fn names_show_a_few_parts_or_count_the_rest() {
        assert_eq!(name(&[]), "job");
        assert_eq!(name(&["Bracket"]), "Bracket");
        assert_eq!(name(&["Bracket", "Plate", "Gusset"]), "Bracket + Plate + Gusset");
        assert_eq!(name(&["A", "B", "C", "D"]), "A + 3 more");
        let long = "A very long part name for a bracket";
        assert_eq!(name(&[long, long]), format!("{long} + 1 more"));
    }
}
