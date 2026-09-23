// SPDX-License-Identifier: GPL-3.0-or-later

//! Calibration storage and compiler adaptation. The coordinate model lives in
//! `openlaser-correction`; this module never sends machine commands.

use crate::coordinator::Coordinator;
use crate::draft::{Draft, Prepared};
use crate::{Error, Result};
use openlaser_compiler::cut;
use openlaser_core::LaserMode;
use openlaser_core::geometry::{Bounds, Curve, Point};
use openlaser_core::nesting::{NestSettings, NestStock, Nesting};
use openlaser_core::toolpath::PreparedSegment;
use openlaser_correction::{Map, Measurement, Profile};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    bed: Option<Bounds>,
    measurements: [Option<Measurement>; 9],
    active: Option<Profile>,
    enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Store {
    version: u32,
    revision: u64,
    fiber: Record,
    co2: Record,
}

impl Default for Store {
    fn default() -> Self {
        Self { version: 1, revision: 0, fiber: Record::default(), co2: Record::default() }
    }
}

impl Store {
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let bytes = match std::fs::read(root.join("correction.json")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(Error::Refused(format!("matrix correction: {e}"))),
        };
        let store: Self = serde_json::from_slice(&bytes).map_err(problem)?;
        if store.version != 1 {
            return Err(Error::Refused("unsupported matrix correction file".into()));
        }
        for record in [&store.fiber, &store.co2] {
            if let Some(profile) = &record.active {
                profile.validate().map_err(problem)?;
            }
            validate_measurements(&record.measurements)?;
            if record.enabled && record.active.is_none() {
                return Err(Error::Refused("enabled correction has no measured profile".into()));
            }
        }
        Ok(store)
    }

    fn record(&self, mode: LaserMode) -> &Record {
        match mode {
            LaserMode::Fiber => &self.fiber,
            LaserMode::Co2 => &self.co2,
        }
    }

    fn record_mut(&mut self, mode: LaserMode) -> &mut Record {
        match mode {
            LaserMode::Fiber => &mut self.fiber,
            LaserMode::Co2 => &mut self.co2,
        }
    }

    pub(crate) fn active(&self, mode: LaserMode) -> Option<Profile> {
        let record = self.record(mode);
        record.enabled.then(|| record.active.clone()).flatten()
    }
}

fn problem(error: impl std::fmt::Display) -> Error {
    Error::Refused(format!("matrix correction: {error}"))
}

fn validate_measurements(measurements: &[Option<Measurement>; 9]) -> Result<()> {
    for m in measurements.iter().flatten() {
        Measurement::from_edges([m.x, m.y], [m.x, m.y]).map_err(problem)?;
    }
    Ok(())
}

pub(crate) fn bed(c: &Coordinator) -> Result<Bounds> {
    let extent =
        c.extent().ok_or_else(|| Error::Refused("load machine dimensions first".into()))?;
    Ok(Bounds {
        min: Point::new(extent[0][0], extent[1][0]),
        max: Point::new(extent[0][1], extent[1][1]),
    })
}

/// Current measurements and the profile that new DXF jobs will receive.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct CorrectionView {
    /// Concurrency token for saving this form.
    pub revision: u64,
    /// Laser being calibrated.
    pub mode: LaserMode,
    /// Current machine dimensions.
    pub bed: Bounds,
    /// Coupon centres in bottom-to-top row order.
    pub positions: [[f64; 2]; 9],
    /// Measurements for this bed; unfinished cells are null.
    pub measurements: [Option<Measurement>; 9],
    /// Applied snapshot, which may belong to an older bed.
    pub active: Option<Profile>,
    /// Automatically apply the active snapshot to new DXF jobs.
    pub enabled: bool,
}

/// Save a partial measurement set, apply a complete one, or pause future use.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CorrectionChange {
    /// Retain progress; `apply` also creates and enables a validated profile.
    Measure {
        /// Current form revision.
        revision: u64,
        /// Which laser was measured.
        mode: LaserMode,
        /// Nine effective X/Y measurements, with empty cells allowed while editing.
        measurements: Box<[Option<Measurement>; 9]>,
        /// Enable after validating all nine cells.
        apply: bool,
    },
    /// Enable or disable correction for future DXF jobs.
    Enable {
        /// Current form revision.
        revision: u64,
        /// Which laser to change.
        mode: LaserMode,
        /// Whether to use the active profile.
        enabled: bool,
    },
}

impl Coordinator {
    /// The matrix settings for one laser.
    pub fn correction_view(&self, mode: LaserMode) -> Result<CorrectionView> {
        let bed = bed(self)?;
        let profile = Profile { bed, measurements: [Measurement { x: 100., y: 100. }; 9] };
        let positions = profile.positions().map_err(problem)?.map(Into::into);
        let record = self.correction.record(mode);
        Ok(CorrectionView {
            revision: self.correction.revision,
            mode,
            bed,
            positions,
            measurements: if record.bed == Some(bed) { record.measurements } else { [None; 9] },
            active: record.active.clone(),
            enabled: record.enabled,
        })
    }

    /// Write first, then publish new defaults. Existing drafts keep their snapshot.
    pub fn save_correction(&mut self, change: CorrectionChange) -> Result<()> {
        let mut next = self.correction.clone();
        let expected = match &change {
            CorrectionChange::Measure { revision, .. }
            | CorrectionChange::Enable { revision, .. } => *revision,
        };
        if expected != next.revision {
            return Err(Error::Refused("matrix settings changed; reopen this page".into()));
        }
        let bed = bed(self)?;
        match change {
            CorrectionChange::Measure { mode, measurements, apply, .. } => {
                let measurements = *measurements;
                validate_measurements(&measurements)?;
                let record = next.record_mut(mode);
                record.bed = Some(bed);
                record.measurements = measurements;
                if apply {
                    let values =
                        measurements.into_iter().collect::<Option<Vec<_>>>().ok_or_else(|| {
                            Error::Request("enter X and Y for all nine squares".into())
                        })?;
                    let profile = Profile {
                        bed,
                        measurements: values
                            .try_into()
                            .map_err(|_| Error::Request("nine measurements are required".into()))?,
                    };
                    profile.validate().map_err(problem)?;
                    record.active = Some(profile);
                    record.enabled = true;
                }
            }
            CorrectionChange::Enable { mode, enabled, .. } => {
                let record = next.record_mut(mode);
                if enabled && record.active.as_ref().is_none_or(|p| p.bed != bed) {
                    return Err(Error::Refused(
                        "measure and save this bed before enabling correction".into(),
                    ));
                }
                record.enabled = enabled;
            }
        }
        next.revision += 1;
        openlaser_library::atomic_write(
            &self.config.data_dir.join("correction.json"),
            &serde_json::to_vec_pretty(&next).map_err(problem)?,
        )?;
        self.correction = next;
        self.publish();
        Ok(())
    }

    /// Load the nine-square job into Setup, without enabling motion.
    pub fn correction_coupon(&mut self, mode: LaserMode) -> Result<()> {
        let bed = bed(self)?;
        let drawing = openlaser_correction::coupon_drawing(bed).map_err(problem)?;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"{} {} {} {}\"><title>{mode:?} matrix calibration</title>",
            bed.width(),
            bed.height(),
            bed.min.x,
            bed.min.y,
            bed.width(),
            bed.height()
        );
        for contour in &drawing.contours {
            let p = contour.curves[0].point(0.);
            write!(svg, "<rect x=\"{}\" y=\"{}\" width=\"100\" height=\"100\" fill=\"none\" stroke=\"black\"/>", p.x, p.y).map_err(problem)?;
        }
        svg.push_str("</svg>");
        self.queue_draft();
        let part = self.library.add_part("Matrix calibration.svg", svg.as_bytes(), drawing)?;
        let sources = self.library.job_drawing(std::slice::from_ref(&part.id))?;
        let mut draft = Draft::new(std::sync::Arc::new(sources));
        draft.calibration = true;
        draft.placement =
            Some(openlaser_library::placement::Placement::Fixed { origin: bed.min.into() });
        draft.sheet_offset = Some([0., 0.]);
        draft.nesting = Some(Nesting {
            stock: NestStock::Rectangle { bounds: bed },
            settings: NestSettings::default(),
        });
        self.draft_generation += 1;
        self.draft = Some(draft);
        self.library_changed();
        self.reprepare();
        Ok(())
    }
}

/// Invert prepared geometry after its recipe shift and final placement. Source
/// intervals and processes follow each piece; point events occur only once.
pub(crate) fn prepare(
    prepared: &Prepared,
    profile: Option<&Profile>,
    placement: [f64; 2],
) -> Result<Prepared> {
    let Some(profile) = profile else {
        return Ok(prepared.clone());
    };
    let map = Map::new(profile).map_err(problem)?;
    if map.is_identity() {
        return Ok(prepared.clone());
    }
    let placement = Point::from(placement);
    let mut count = 0usize;
    let mut correct = |contour: &cut::Contour| -> Result<cut::Contour> {
        let mut segments = Vec::new();
        for segment in &contour.segments {
            let curve = match segment.curve {
                cut::Segment::Line { start, end } => {
                    Curve::Line { start: start.into(), end: end.into() }
                }
                cut::Segment::Arc { center, radius, start_angle, sweep } => {
                    Curve::Arc { center: center.into(), radius, start_angle, sweep }
                }
            };
            for (index, piece) in
                map.curve(curve, placement, 0.002).map_err(problem)?.into_iter().enumerate()
            {
                count += 1;
                if count > 2_000_000 {
                    return Err(Error::Refused(
                        "corrected job exceeds two million path segments".into(),
                    ));
                }
                let mut process = segment.process.clone();
                if index > 0 {
                    process.cool_ms = 0;
                    process.repierce = false;
                }
                let source = segment.source.map(|mut source| {
                    let start = source.start;
                    let span = source.end - start;
                    source.start = start + span * piece.from;
                    source.end = start + span * piece.to;
                    source
                });
                segments.push(PreparedSegment {
                    curve: cut::Segment::Line { start: piece.start.into(), end: piece.end.into() },
                    process,
                    source,
                });
            }
        }
        Ok(cut::Contour { segments, overrides: contour.overrides.clone() })
    };
    let contours = prepared.contours.iter().map(&mut correct).collect::<Result<Vec<_>>>()?;
    let film = prepared
        .film
        .iter()
        .map(|runs| runs.iter().map(&mut correct).collect())
        .collect::<Result<Vec<_>>>()?;
    Ok(Prepared { contours, film, ..prepared.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::geometry::{Contour, Drawing};
    use openlaser_core::toolpath::{SegmentProcess, SourceInterval};

    #[test]
    #[allow(clippy::float_cmp, reason = "subdivision must preserve exact source boundaries")]
    fn subdividing_correction_retains_source_intervals_without_repeating_process_events() {
        let drawing = Drawing {
            contours: vec![Contour {
                layer: "Cut".into(),
                curves: vec![Curve::Line {
                    start: Point::new(100., 100.),
                    end: Point::new(200., 100.),
                }],
            }],
        };
        let mut draft = Draft::of(drawing.clone());
        draft.prepare(&drawing);
        let mut prepared = (*draft.prepared.unwrap()).clone();
        let process = SegmentProcess {
            cool_ms: 120,
            repierce: true,
            joint: true,
            power: Some(15.),
            speed: Some(4.),
            ..SegmentProcess::default()
        };
        prepared.contours[0].segments[0].process = process.clone();
        // A reversed interval must remain reversed after adaptive subdivision.
        prepared.contours[0].segments[0].source =
            Some(SourceInterval { contour: 7, start: 0.8, end: 0.2 });
        prepared.film = vec![vec![prepared.contours[0].clone()]];
        let profile = Profile {
            bed: Bounds { min: Point::ORIGIN, max: Point::new(1000., 1000.) },
            measurements: [Measurement { x: 101., y: 99. }; 9],
        };
        let result = prepare(&prepared, Some(&profile), [30., 40.]).unwrap();
        let map = Map::new(&profile).unwrap();
        for contour in [&result.contours[0], &result.film[0][0]] {
            let segments = &contour.segments;
            assert!(segments.len() > 10);
            assert_eq!(segments.iter().filter(|s| s.process.repierce).count(), 1);
            assert_eq!(segments.iter().map(|s| s.process.cool_ms).sum::<u32>(), 120);
            assert_eq!(segments[0].process, process);
            assert!(segments.iter().all(|s| s.process.joint
                && s.process.power == Some(15.)
                && s.process.speed == Some(4.)));
            assert_eq!(segments[0].source.unwrap().start, 0.8);
            assert!((segments.last().unwrap().source.unwrap().end - 0.2).abs() < 1e-12);
            for pair in segments.windows(2) {
                assert_eq!(pair[0].source.unwrap().end, pair[1].source.unwrap().start);
                assert_eq!(pair[0].source.unwrap().contour, 7);
            }
            let placement = Point::new(30., 40.);
            let first = Point::from(segments[0].curve.point(0.)) + placement;
            let last = Point::from(segments.last().unwrap().curve.point(1.)) + placement;
            assert!(map.forward(first).distance(Point::new(130., 140.)) < 1e-8);
            assert!(map.forward(last).distance(Point::new(230., 140.)) < 1e-8);
        }
    }
}
