// SPDX-License-Identifier: GPL-3.0-or-later

//! Durable sheet history. A run captures its stock before admission; remaining
//! material becomes nestable only after an operator saves an inspected remnant.

use crate::coordinator::Coordinator;
use crate::document::PassKind;
use crate::draft::Draft;
use crate::{Error, Result};
use openlaser_core::LaserMode;
use openlaser_core::geometry::{Bounds, Contour, Curve, Drawing, Point, Transform};
use openlaser_core::nesting::{NestSettings, NestStock, Nesting};
use openlaser_library::Id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CutArea {
    outline: Contour,
    passes: Vec<usize>,
}

/// Immutable sheet geometry and source metadata belonging to one original run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SheetPlan {
    id: Id,
    job: Option<Id>,
    name: String,
    mode: LaserMode,
    material: String,
    thickness_mm: f64,
    outline: Contour,
    existing: Vec<Contour>,
    areas: Vec<CutArea>,
    parent: Option<String>,
    boundary_known: bool,
    correction: Option<openlaser_correction::Profile>,
    #[serde(default)]
    clearance: f64,
}

/// Material state, independent of any later job edits.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SheetState {
    /// An admitted or interrupted admission; completion is not yet established.
    Cutting,
    /// The controller finished the job, or an operator marked a saved job cut.
    Completed,
    /// A stopped or failed run; planned part areas stay reserved conservatively.
    Interrupted,
    /// An operator inspected and saved the remaining sheet for nesting.
    Remnant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    version: u32,
    revision: u64,
    at: u64,
    state: SheetState,
    plan: SheetPlan,
    removed: Vec<Contour>,
    reported: bool,
}

/// An inspected sheet or completion record, with display geometry only.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct SheetView {
    /// Stable sheet identity.
    pub id: String,
    /// Concurrency token for edits.
    pub revision: u64,
    /// Completed, interrupted or inspected state.
    pub state: SheetState,
    /// Already reserved by a later cut; cannot be selected as stock again.
    pub used: bool,
    /// Operator's name.
    pub name: String,
    /// Original saved job, when one exists.
    pub job: Option<Id>,
    /// Process used on this sheet.
    pub mode: LaserMode,
    /// Material name.
    pub material: String,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
    /// Recording time in Unix seconds.
    pub at: u64,
    /// Marked cut by an operator instead of a controller completion.
    pub reported: bool,
    /// Whether a stock outline was specified before the job.
    pub boundary_known: bool,
    /// Outer stock extent.
    pub bounds: Bounds,
    /// Outer boundary for the sheet viewer.
    pub outline: Vec<[f64; 2]>,
    /// Reserved removed regions for the viewer.
    pub cutouts: Vec<Vec<[f64; 2]>>,
    /// Inherited minimum margin for prior kerf and leads, in millimetres.
    pub clearance: f64,
}

/// A page of sheet history, newest first.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct SheetPage {
    /// Records on this page.
    pub items: Vec<SheetView>,
    /// Cursor for older records.
    pub next: Option<String>,
}

/// Operator confirmation and optional corrected rectangular stock size.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRemnant {
    /// Revision being inspected.
    pub revision: u64,
    /// Name on the stock card.
    pub name: String,
    /// Replace an inferred boundary with the actual rectangular sheet bounds.
    pub bounds: Option<Bounds>,
}

pub(crate) struct Store {
    root: PathBuf,
    records: BTreeMap<String, Record>,
}

fn fail(error: impl std::fmt::Display) -> Error {
    Error::Refused(format!("sheet library: {error}"))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Store {
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let root = root.join("sheets");
        std::fs::create_dir_all(&root).map_err(fail)?;
        let mut records = BTreeMap::new();
        for entry in std::fs::read_dir(&root).map_err(fail)? {
            let path = entry.map_err(fail)?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let record: Record =
                serde_json::from_slice(&std::fs::read(&path).map_err(fail)?).map_err(fail)?;
            if record.version != 1
                || path.file_stem().and_then(|s| s.to_str()) != Some(record.plan.id.as_str())
            {
                return Err(fail("invalid sheet identity or version"));
            }
            validate(&record)?;
            records.insert(record.plan.id.to_string(), record);
        }
        Ok(Self { root, records })
    }

    fn write(&mut self, record: Record) -> Result<()> {
        validate(&record)?;
        let id = record.plan.id.to_string();
        openlaser_library::atomic_write(
            &self.root.join(format!("{id}.json")),
            &serde_json::to_vec_pretty(&record).map_err(fail)?,
        )?;
        self.records.insert(id, record);
        Ok(())
    }

    fn used(&self, id: &str) -> bool {
        self.records.values().any(|r| r.plan.parent.as_deref() == Some(id))
    }

    pub(crate) fn view(&self, id: &str) -> Result<SheetView> {
        let record = self
            .records
            .get(id)
            .ok_or_else(|| Error::Missing("that sheet does not exist".into()))?;
        let p = &record.plan;
        Ok(SheetView {
            id: id.into(),
            revision: record.revision,
            state: record.state,
            used: self.used(id),
            name: p.name.clone(),
            job: p.job.clone(),
            mode: p.mode,
            material: p.material.clone(),
            thickness_mm: p.thickness_mm,
            at: record.at,
            reported: record.reported,
            boundary_known: p.boundary_known,
            bounds: p.outline.bounds().ok_or_else(|| fail("empty sheet"))?,
            outline: display(&p.outline),
            cutouts: p.existing.iter().chain(&record.removed).map(display).collect(),
            clearance: p.clearance,
        })
    }

    pub(crate) fn page(&self, before: Option<&str>, remnants: bool) -> Result<SheetPage> {
        let ids: Vec<_> = self
            .records
            .iter()
            .rev()
            .filter(|(id, r)| {
                before.is_none_or(|b| id.as_str() < b)
                    && (!remnants || (r.state == SheetState::Remnant && !self.used(id)))
            })
            .take(25)
            .map(|(id, _)| id.as_str())
            .collect();
        let next = (ids.len() > 24).then(|| ids[23].to_owned());
        Ok(SheetPage {
            items: ids.into_iter().take(24).map(|id| self.view(id)).collect::<Result<_>>()?,
            next,
        })
    }

    /// Resolve a ready remnant into the current drawing's stock coordinate frame.
    pub(crate) fn stock(&self, id: &str, at: Point) -> Result<NestStock> {
        let record = self
            .records
            .get(id)
            .ok_or_else(|| Error::Missing("that remnant does not exist".into()))?;
        if record.state != SheetState::Remnant || self.used(id) {
            return Err(Error::Refused("this sheet is not available as a remnant".into()));
        }
        let shift = Transform::translation(
            at - record.plan.outline.bounds().ok_or_else(|| fail("empty sheet"))?.min,
        );
        Ok(NestStock::Remnant {
            reference: id.into(),
            name: record.plan.name.clone(),
            clearance: record.plan.clearance,
            outline: shift.contour(&record.plan.outline),
            cutouts: record
                .plan
                .existing
                .iter()
                .chain(&record.removed)
                .map(|c| shift.contour(c))
                .collect(),
        })
    }

    pub(crate) fn begin(&mut self, plan: &SheetPlan) -> Result<()> {
        if let Some(record) = self.records.get(plan.id.as_str()) {
            let mut record = record.clone();
            record.state = SheetState::Cutting;
            record.revision += 1;
            return self.write(record);
        }
        if let Some(parent) = &plan.parent
            && self.used(parent)
        {
            return Err(Error::Refused("this remnant was already reserved by another job".into()));
        }
        self.write(Record {
            version: 1,
            revision: 0,
            at: now(),
            state: SheetState::Cutting,
            plan: plan.clone(),
            removed: plan.areas.iter().map(|a| a.outline.clone()).collect(),
            reported: false,
        })
    }

    pub(crate) fn finish(
        &mut self,
        plan: &SheetPlan,
        completed: bool,
        executed: &[usize],
    ) -> Result<()> {
        let mut record = self
            .records
            .get(plan.id.as_str())
            .cloned()
            .ok_or_else(|| fail("missing admitted sheet"))?;
        record.revision += 1;
        record.state = if completed { SheetState::Completed } else { SheetState::Interrupted };
        if completed {
            record.removed = plan
                .areas
                .iter()
                .filter(|a| a.passes.iter().any(|pass| executed.contains(pass)))
                .map(|a| a.outline.clone())
                .collect();
        }
        self.write(record)
    }

    pub(crate) fn save(&mut self, id: &str, change: &SaveRemnant) -> Result<()> {
        let mut record = self
            .records
            .get(id)
            .cloned()
            .ok_or_else(|| Error::Missing("that sheet does not exist".into()))?;
        if record.revision != change.revision {
            return Err(Error::Refused("the sheet changed; inspect its current state".into()));
        }
        if self.used(id) || !matches!(record.state, SheetState::Completed | SheetState::Remnant) {
            return Err(Error::Refused(
                "only a completed, available sheet can become a remnant".into(),
            ));
        }
        let name = change.name.trim();
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return Err(Error::Request("give the remnant a name of 1–120 characters".into()));
        }
        if let Some(bounds) = change.bounds {
            record.plan.outline = rectangle(bounds)?;
            record.plan.boundary_known = true;
        }
        if !record.plan.boundary_known {
            return Err(Error::Request("enter the actual sheet width and height".into()));
        }
        record.plan.name = name.into();
        record.state = SheetState::Remnant;
        record.revision += 1;
        self.write(record)
    }
}

fn validate(record: &Record) -> Result<()> {
    let p = &record.plan;
    if p.id.as_str().is_empty()
        || !p.id.as_str().bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || p.name.len() > 480
        || !p.thickness_mm.is_finite()
        || p.thickness_mm < 0.
        || !p.clearance.is_finite()
        || p.clearance < 0.
    {
        return Err(fail("invalid sheet metadata"));
    }
    let mut curves = 0usize;
    for contour in std::iter::once(&p.outline).chain(&p.existing).chain(&record.removed) {
        curves += contour.curves.len();
        if curves > 1_000_000
            || !contour.is_closed()
            || contour.curves.iter().any(|c| !c.is_valid())
        {
            return Err(fail("invalid or excessive sheet geometry"));
        }
    }
    Ok(())
}

fn display(contour: &Contour) -> Vec<[f64; 2]> {
    crate::draft::outline(&Drawing { contours: vec![contour.clone()] }, 256)
        .into_iter()
        .flatten()
        .collect()
}

fn rectangle(bounds: Bounds) -> Result<Contour> {
    crate::nesting::stock(
        &Drawing::default(),
        &Nesting { stock: NestStock::Rectangle { bounds }, settings: NestSettings::default() },
    )
}

/// Open cuts remove narrow traces too. Retain analytic bands so future parts
/// cannot cross a slit; the inherited kerf and lead margin expands these bands.
fn trace_area(curve: &Curve) -> Contour {
    const PAD: f64 = 0.01;
    let curves = match *curve {
        Curve::Line { start, end } => {
            let delta = end - start;
            if delta.norm() == 0. {
                vec![Curve::circle(start, PAD)]
            } else {
                let along = delta * (PAD / delta.norm());
                let across = along.perpendicular();
                let points = [
                    start - along - across,
                    end + along - across,
                    end + along + across,
                    start - along + across,
                ];
                (0..4).map(|i| Curve::Line { start: points[i], end: points[(i + 1) % 4] }).collect()
            }
        }
        Curve::Arc { center, radius, start_angle, sweep } => {
            let extra = (PAD / radius).min(1.).asin();
            if radius <= PAD || sweep.abs() + 2. * extra >= std::f64::consts::TAU {
                vec![Curve::circle(center, radius + PAD)]
            } else {
                let start_angle = start_angle - extra * sweep.signum();
                let sweep = sweep + 2. * extra * sweep.signum();
                let outer = Curve::Arc { center, radius: radius + PAD, start_angle, sweep };
                let inner = Curve::Arc {
                    center,
                    radius: radius - PAD,
                    start_angle: start_angle + sweep,
                    sweep: -sweep,
                };
                vec![
                    outer,
                    Curve::Line { start: outer.end(), end: inner.start() },
                    inner,
                    Curve::Line { start: inner.end(), end: outer.start() },
                ]
            }
        }
    };
    Contour { layer: "Previous open cut".into(), curves }
}

fn footprints(contours: &[Contour], group: &[usize]) -> Vec<Contour> {
    let outer = group
        .iter()
        .filter_map(|i| contours.get(*i))
        .filter(|c| c.is_closed())
        .max_by(|a, b| a.signed_area().abs().total_cmp(&b.signed_area().abs()));
    outer.map_or_else(
        || {
            group
                .iter()
                .flat_map(|i| {
                    contours[*i].curves.iter().filter(|curve| curve.length() > 0.).map(trace_area)
                })
                .collect()
        },
        |outer| vec![outer.clone()],
    )
}

fn command_shift(draft: &Draft) -> Result<[f64; 2]> {
    let Some(recipe) = &draft.current.recipe else { return Ok([0., 0.]) };
    let shift = if draft.calibration {
        [0., 0.]
    } else if let Some(compiled) = &draft.compiled {
        compiled.shift
    } else if recipe.attributes.get("EnableContourShift").is_some_and(|v| v == "1") {
        let axis = |name| {
            recipe.attributes.get(name).map_or(Ok(0.), |value| value.parse::<f64>().map_err(fail))
        };
        [axis("ContourShiftXDist")?, axis("ContourShiftYDist")?]
    } else {
        [0., 0.]
    };
    Ok(shift)
}

fn previous_clearance(draft: &Draft, prepared: &crate::draft::Prepared) -> f64 {
    (draft.current.features.kerf.as_ref().map_or(0., |k| k.width.0)
        + prepared
            .contours
            .iter()
            .map(|c| {
                c.segments
                    .iter()
                    .filter(|s| s.process.lead.is_some())
                    .map(|s| s.curve.length())
                    .sum::<f64>()
            })
            .fold(0., f64::max))
    .max(draft.current.nesting.as_ref().map_or(0., |n| match &n.stock {
        NestStock::Remnant { clearance, .. } => *clearance,
        _ => 0.,
    }))
}

impl Coordinator {
    /// Snapshot the active stock, outer part regions and open cut traces.
    pub(crate) fn sheet_plan(&self, draft: &Draft) -> Result<Option<Arc<SheetPlan>>> {
        let Some(prepared) = &draft.prepared else { return Ok(None) };
        let Some(recipe) = &draft.current.recipe else { return Ok(None) };
        let drawing = draft.drawing()?;
        let machining_shift = command_shift(draft)?;
        let prepared = prepared.shifted(machining_shift)?;
        let moved = Transform::translation(Point::from(machining_shift));
        let placed = Drawing {
            contours: crate::draft::place(drawing, &draft.current.placed)
                .contours
                .iter()
                .map(|c| moved.contour(c))
                .collect(),
        };
        let outline = if let Some(nesting) = &draft.current.nesting {
            crate::nesting::stock(drawing, nesting)?
        } else {
            let extent =
                self.extent().ok_or_else(|| Error::Refused("no sheet dimensions".into()))?;
            let zero = Point::from(draft.current.sheet_offset.unwrap_or([0., 0.]));
            rectangle(Bounds {
                min: Point::new(extent[0][0], extent[1][0]) - zero,
                max: Point::new(extent[0][1], extent[1][1]) - zero,
            })?
        };
        let shift = Transform::translation(
            Point::ORIGIN - outline.bounds().ok_or_else(|| fail("empty sheet"))?.min,
        );
        let mut areas = Vec::new();
        // Authored groups and bridges may join several disconnected outer
        // shapes. Recover material regions from geometry, not selection groups.
        for group in openlaser_prep::groups(&placed.contours) {
            let passes: Vec<_> = draft
                .compiled
                .as_ref()
                .map(|c| {
                    c.view
                        .plan
                        .iter()
                        .filter(|p| {
                            p.kind == PassKind::Cut
                                && p.instances.iter().any(|instance| {
                                    group.iter().any(|i| {
                                        draft.current.placed[*i].source == instance.source
                                            && draft.current.placed[*i].copy == instance.copy
                                    })
                                })
                        })
                        .map(|p| p.ordinal)
                        .collect()
                })
                .unwrap_or_default();
            for outline in footprints(&placed.contours, &group) {
                areas.push(CutArea { outline: shift.contour(&outline), passes: passes.clone() });
            }
        }
        if areas.is_empty() || prepared.contours.is_empty() {
            return Ok(None);
        }
        let name = self.draft_name(draft);
        let parent = draft.current.nesting.as_ref().and_then(|n| {
            if let NestStock::Remnant { reference, .. } = &n.stock {
                Some(reference.clone())
            } else {
                None
            }
        });
        Ok(Some(Arc::new(SheetPlan {
            id: Id::generate(),
            job: draft.job.clone(),
            name,
            mode: recipe.laser,
            material: recipe.name.clone(),
            thickness_mm: recipe.thickness_mm,
            outline: shift.contour(&outline),
            existing: draft
                .current
                .nesting
                .as_ref()
                .map(|n| crate::nesting::cutouts(n).iter().map(|c| shift.contour(c)).collect())
                .unwrap_or_default(),
            areas,
            parent,
            boundary_known: draft.current.nesting.is_some(),
            correction: draft.current.correction.clone(),
            // Reserve a conservative band around nominal removed regions.
            // It follows saved stock through future jobs and cannot be reduced
            // by choosing a smaller margin in the nesting form.
            clearance: previous_clearance(draft, &prepared),
        })))
    }

    pub(crate) fn finish_sheet(&mut self, completed: bool) -> Result<()> {
        let Some(held) = self.held.as_ref().filter(|h| !h.dry_run) else { return Ok(()) };
        let Some(plan) = held.sheet.clone() else { return Ok(()) };
        if let Some(recovery) = &mut self.recovery {
            recovery.observe(&self.machine.state(), self.execution.as_deref());
        }
        let executed = self
            .recovery
            .as_ref()
            .map(|r| {
                r.view()
                    .steps
                    .into_iter()
                    .filter(|s| s.pass.kind == PassKind::Cut && !s.executed.is_empty())
                    .map(|s| s.pass.ordinal)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.sheet_store.finish(&plan, completed, &executed)?;
        if completed {
            self.completed_sheet = Some(plan.id.to_string());
        }
        Ok(())
    }

    /// Record the operator's statement that a saved job was cut previously.
    pub fn report_cut_sheet(&mut self, job: &Id) -> Result<String> {
        let saved = self.library.job(job)?.clone();
        let sources = Arc::new(self.library.job_drawing(&saved.parts)?);
        let mut draft = Draft::new(sources.clone());
        draft.adopt(&saved);
        if draft.current.placed.is_empty() {
            draft.current.placed = openlaser_core::geometry::Placed::all(sources.contours());
        }
        draft.prepare(sources.drawing());
        let plan = self
            .sheet_plan(&draft)?
            .ok_or_else(|| Error::Refused("the job has no cutting paths".into()))?;
        let id = plan.id.to_string();
        if plan.parent.as_ref().is_some_and(|parent| self.sheet_store.used(parent)) {
            return Err(Error::Refused("this remnant was already reserved by another job".into()));
        }
        self.sheet_store.write(Record {
            version: 1,
            revision: 0,
            at: now(),
            state: SheetState::Completed,
            removed: plan.areas.iter().map(|a| a.outline.clone()).collect(),
            plan: (*plan).clone(),
            reported: true,
        })?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, size: f64) -> Contour {
        rectangle(Bounds { min: Point::new(x, x), max: Point::new(x + size, x + size) }).unwrap()
    }

    #[test]
    fn open_cut_traces_exclude_slits_without_discarding_the_inside_of_an_arc() {
        let stock = rect(0., 100.);
        let line =
            trace_area(&Curve::Line { start: Point::new(10., 10.), end: Point::new(90., 90.) });
        let arc = trace_area(&Curve::Arc {
            center: Point::new(50., 50.),
            radius: 30.,
            start_angle: 0.,
            sweep: std::f64::consts::PI,
        });
        for outline in [&line, &arc] {
            assert!(outline.is_closed());
            assert!(outline.curves.iter().all(Curve::is_valid));
        }
        let check = |cutout, points: &[[f64; 2]]| {
            openlaser_nest::check_region_polylines(&stock, &[cutout], [points], 0.)
        };
        assert!(check(line.clone(), &[[45., 50.], [55., 50.]]).is_err());
        assert!(check(line, &[[10., 80.], [20., 80.]]).is_ok());
        assert!(check(arc.clone(), &[[50., 75.], [50., 85.]]).is_err());
        assert!(check(arc, &[[45., 60.], [55., 60.]]).is_ok());
    }

    #[test]
    fn restart_keeps_reservations_and_completion_excludes_only_untouched_parts() {
        let root = std::env::temp_dir().join(format!("openlaser-sheet-store-{}", Id::generate()));
        let plan = SheetPlan {
            id: Id::generate(),
            job: None,
            name: "Sheet".into(),
            mode: LaserMode::Fiber,
            material: "Steel".into(),
            thickness_mm: 1.,
            outline: rect(0., 100.),
            existing: vec![rect(10., 5.)],
            areas: vec![
                CutArea { outline: rect(30., 10.), passes: vec![1, 2] },
                CutArea { outline: rect(60., 10.), passes: vec![3] },
            ],
            parent: None,
            boundary_known: false,
            correction: None,
            clearance: 1.,
        };
        let id = plan.id.as_str();
        let mut store = Store::open(&root).unwrap();
        store.begin(&plan).unwrap();
        store.finish(&plan, false, &[1]).unwrap();
        drop(store);
        let mut store = Store::open(&root).unwrap();
        assert_eq!(store.view(id).unwrap().state, SheetState::Interrupted);
        assert_eq!(
            store.view(id).unwrap().cutouts.len(),
            3,
            "interrupted work keeps every planned area reserved"
        );
        assert!(store.stock(id, Point::ORIGIN).is_err());
        store.begin(&plan).unwrap();
        store.finish(&plan, true, &[1]).unwrap();
        assert_eq!(
            store.view(id).unwrap().cutouts.len(),
            2,
            "part with any cut is reserved; untouched skipped part is retained"
        );
        let view = store.view(id).unwrap();
        assert!(
            store
                .save(
                    id,
                    &SaveRemnant {
                        revision: view.revision,
                        name: "Remainder".into(),
                        bounds: None
                    }
                )
                .is_err()
        );
        store
            .save(
                id,
                &SaveRemnant {
                    revision: view.revision,
                    name: "Remainder".into(),
                    bounds: Some(Bounds { min: Point::ORIGIN, max: Point::new(90., 90.) }),
                },
            )
            .unwrap();
        drop(store);
        let mut store = Store::open(&root).unwrap();
        assert_eq!(store.page(None, true).unwrap().items.len(), 1);
        let stock = store.stock(id, Point::new(100., 200.)).unwrap();
        if let NestStock::Remnant { outline, cutouts, .. } = stock {
            assert_eq!(outline.bounds().unwrap().min, Point::new(100., 200.));
            assert_eq!(cutouts.len(), 2);
            assert_eq!(cutouts[0].bounds().unwrap().min, Point::new(110., 210.));
        } else {
            panic!("expected remnant")
        }
        let mut child = plan.clone();
        child.id = Id::generate();
        child.parent = Some(id.into());
        store.begin(&child).unwrap();
        assert!(Store::open(&root).unwrap().stock(id, Point::ORIGIN).is_err());
        let mut duplicate = child;
        duplicate.id = Id::generate();
        assert!(store.begin(&duplicate).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
