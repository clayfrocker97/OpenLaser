// SPDX-License-Identifier: GPL-3.0-or-later

//! Composing a program: the process commands around every pass's motion.
//!
//! MainApp `0x0045_FC70` schedules the passes, `0x0045_E350` starts a cut
//! (gas, peak, follow, laser on, delay), `0x0045_DD00` pierces stage by
//! stage, `0x0045_DB30` ends a cut, `0x0045_E6D0` plans the head and the
//! gas between contours, `0x0045_F4C0` finishes a pre-pierce point, and
//! NCModule `0x1004_3B10` opens and closes the program. The records are
//! emitted in the vendor's order and tagged into sections so the controller
//! can report progress and the interface can preview.

use crate::cut::{self, Piece, StopEvent};
use crate::head::JumpProfile;
use crate::motion::{self, Movement, Output, Plan, Sample};
use crate::pass::{self, Pass, PassKind};
use crate::settings::{Hardware, LeadRole, Overrides, PierceStage, Settings};
use crate::{Error, MAX_SAMPLES, Point, Result, float, residue, to_i32, to_u8, to_u32, travel};
use openlaser_protocol::records::{PulsedFields, Record};
use openlaser_protocol::requests::LaserChannel;
use std::ops::Range;
use std::sync::Arc;

/// Host memory bound for records, including process commands in addition to
/// interpolation samples. Some stages emit two records per timed sample.
pub const MAX_RECORDS: usize = 2 * MAX_SAMPLES;

/// What a program is bound to at compile time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binding {
    /// Where the head is, in drawing coordinates, when the program starts.
    pub current: Point,
    /// The height controller's units per millimetre, from the controller's
    /// configuration; required when the head is used.
    pub z_units_per_mm: Option<u32>,
}

/// What a section of the program does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The item tag before anything runs.
    CheckpointStart,
    /// Outputs and PWM zeroed before the first contour.
    Start,
    /// The item tag before a pass's travel.
    CheckpointTravel,
    /// Crash protection written before the head moves.
    HeadTravelProtection,
    /// The head jump between contours.
    FrogJump,
    /// The head stays down for a short travel.
    KeepHeadHeight,
    /// The head retracts before travelling.
    RetractBeforeTravel,
    /// Crash protection written for the travel.
    TravelProtection,
    /// Crash protection written after the head retracts.
    BeforeTravelProtection,
    /// Rapid travel to the pass's start.
    Travel,
    /// The head tracing the job's bounds with the laser off.
    Frame,
    /// Waiting for the jump's descent.
    FrogJumpComplete,
    /// The CO2 contour control that opens a contour.
    ContourControlStart,
    /// The commands opening a pierce stage.
    PierceStart(u8),
    /// A pierce stage.
    Pierce(u8),
    /// The smooth-pierce handoff.
    PierceSmooth,
    /// The gradual descent within a pierce stage.
    PierceGradual,
    /// Stationary records padding a bolt drill.
    BoltPadding,
    /// The PWM ramp of a bolt drill.
    BoltRamp,
    /// Commands opening residue cleaning.
    ResidueStart,
    /// The cleaning spiral.
    ResidueSpiral,
    /// The return from the spiral.
    ResidueReturn,
    /// Commands closing residue cleaning.
    ResidueEnd,
    /// The item tag after a pre-pierce point.
    CheckpointPoint,
    /// Gas off after a pre-pierce point, unless the recipe keeps it.
    PointEnd,
    /// The item tag before the cut.
    CheckpointCut,
    /// Gas, peak, follow and laser on.
    CutStart,
    /// A cooling stop.
    Cooling,
    /// Cutting motion.
    Cut,
    /// A film pass's motion.
    Film,
    /// Joint motion.
    Joint,
    /// The lead-in.
    LeadIn,
    /// The lead-out.
    LeadOut,
    /// Laser off, and gas off unless it is kept, after the contour.
    CutEnd,
    /// The item tag after the last pass.
    CheckpointFinished,
    /// Outputs zeroed and the head parked.
    End,
    /// The barrier closing the program.
    EndBarrier,
}

impl Kind {
    /// Whether the section is part of piercing, cleaning or the cut start.
    #[must_use]
    pub const fn is_process(self) -> bool {
        matches!(
            self,
            Self::PierceStart(_)
                | Self::Pierce(_)
                | Self::PierceSmooth
                | Self::PierceGradual
                | Self::BoltPadding
                | Self::BoltRamp
                | Self::ResidueStart
                | Self::ResidueSpiral
                | Self::ResidueReturn
                | Self::ResidueEnd
                | Self::CutStart
        )
    }

    /// The section kind of a cutting piece.
    fn of_piece(piece: &Piece) -> Self {
        if piece.process.cool_ms > 0 {
            Self::Cooling
        } else if piece.process.joint {
            Self::Joint
        } else {
            match piece.process.lead {
                Some(LeadRole::Entry) => Self::LeadIn,
                Some(LeadRole::Exit) => Self::LeadOut,
                None => Self::Cut,
            }
        }
    }
}

/// A run of records with one purpose.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// What the section does.
    pub kind: Kind,
    /// The pass it belongs to, if any.
    pub pass: Option<usize>,
    /// Its records within the program.
    pub records: Range<usize>,
    /// Its predictable duration; feedback waits add an unknown amount.
    pub seconds: f64,
    /// The sampled path, for motion sections.
    pub points: Arc<[Point]>,
    /// How many controller feedback waits it contains.
    pub waits: usize,
}

/// A complete program, ready to partition into upload blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    /// Every record in order.
    pub records: Vec<Record>,
    /// The sections, covering the records in order.
    pub sections: Vec<Section>,
    /// The predictable duration in seconds.
    pub seconds: f64,
    /// The number of interpolation samples.
    pub samples: usize,
}

/// One pass, planned.
#[derive(Clone, Debug, PartialEq)]
pub struct Compiled {
    /// Which pass over which contour.
    pub pass: Pass,
    /// The prepared source geometry; execution pieces remain authoritative on resume.
    pub contour: cut::Contour,
    /// The recipe the pass runs with.
    pub settings: Settings,
    /// The resolved overrides.
    pub overrides: Overrides,
    /// The pieces in order; none for a point.
    pub pieces: Vec<Piece>,
    /// The events between pieces, one more than the pieces.
    pub events: Vec<Option<StopEvent>>,
    /// Where the pass starts.
    pub start: Point,
    /// Where it ends; a point ends where it starts.
    pub end: Point,
    /// Whether a cut pierces before cutting: not after a preliminary pass
    /// the recipe does not repeat, and not always when resumed.
    pub initial_pierce: bool,
    /// Execution distance already removed by previous continuations.
    pub consumed: f64,
    /// Requested cooling points excluded by the native endpoint tolerance.
    pub omitted_cooling: usize,
}

impl Compiled {
    /// `pass` planned over `contour` under `settings` within `budget`
    /// samples.
    fn planned(
        pass: Pass,
        contour: &cut::Contour,
        settings: &Settings,
        budget: usize,
        prepared: bool,
    ) -> Result<Self> {
        let motion = if prepared {
            cut::plan_prepared(contour, settings, budget)?
        } else {
            cut::plan(contour, settings, budget)?
        };
        let start = motion
            .pieces
            .first()
            .and_then(|p| p.plan.points.first())
            .copied()
            .unwrap_or_else(|| contour.segments[0].curve.point(0.));
        let end = motion.pieces.last().and_then(|p| p.plan.points.last()).copied().unwrap_or(start);
        Ok(Self {
            pass,
            consumed: 0.,
            omitted_cooling: motion.omitted_cooling,
            contour: contour.clone(),
            settings: settings.clone(),
            overrides: motion.overrides,
            pieces: motion.pieces,
            events: motion.events,
            start,
            end,
            initial_pierce: true,
        })
    }

    /// A pre-pierce point at the prepared machining start of `contour`,
    /// the start of its lead-in when it has one: piercing without a cut.
    fn point(pass: Pass, contour: &cut::Contour, settings: &Settings) -> Result<Self> {
        let first = contour.segments.first().ok_or(Error::Invalid("a point needs a contour"))?;
        let start = first.curve.point(0.);
        let length = contour.length();
        Ok(Self {
            pass,
            consumed: 0.,
            omitted_cooling: 0,
            contour: contour.clone(),
            settings: settings.clone(),
            overrides: contour.overrides.resolved(settings, length),
            pieces: Vec::new(),
            events: vec![None],
            start,
            end: start,
            initial_pierce: true,
        })
    }

    /// The interpolation samples of the motion.
    fn samples(&self) -> usize {
        self.pieces.iter().map(|p| p.samples.len()).sum()
    }

    /// The cutting duration.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.pieces.iter().map(Piece::seconds).sum()
    }
}

/// Whether a recipe moves the head at all.
fn uses_head(s: &Settings) -> bool {
    !s.no_follow || s.absolute_cut_height.is_some() || !s.pierce.is_empty()
}

/// The film process of a job: its recipe, bound on its own, and per
/// prepared contour the runs of bare geometry the film pass traces.
#[derive(Clone, Copy, Debug)]
pub struct Film<'a> {
    /// The film recipe.
    pub settings: &'a Settings,
    /// Per prepared contour, its film geometry split where it breaks.
    pub runs: &'a [Vec<cut::Contour>],
}

/// A compiled job: every pass planned, ready to bind into a program.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// The job-level recipe: the program's opening and closing use it.
    pub settings: Settings,
    /// The passes in order.
    pub passes: Vec<Compiled>,
}

impl Job {
    /// Plans every contour under `settings`, with no film process.
    pub fn compile(settings: &Settings, contours: &[cut::Contour]) -> Result<Self> {
        Self::schedule(settings, contours, None)
    }

    /// Plans the job's passes, MainApp `0x0045_FC70`: preliminary
    /// piercing points and film passes in their batches before the cuts,
    /// within the sample budget. A film pass carries the gas-retention
    /// flags of the last cut before it, as the vendor copies them into its
    /// bank-11 scratch (`0x0045_E6D0`); a cut after its preliminary point
    /// pierces again only when the recipe repeats the piercing.
    pub fn schedule(
        settings: &Settings,
        contours: &[cut::Contour],
        film: Option<Film<'_>>,
    ) -> Result<Self> {
        Self::schedule_geometry(settings, contours, film, false, None)
    }

    /// Plans geometry already prepared by the host, preserving its lines
    /// and arcs instead of repeating native curve reduction. Corner rounding,
    /// motion planning, process commands and wire records use the same pipeline.
    pub fn schedule_prepared(
        settings: &Settings,
        contours: &[cut::Contour],
        film: Option<Film<'_>>,
    ) -> Result<Self> {
        Self::schedule_geometry(settings, contours, film, true, None)
    }

    /// Plans prepared geometry whose contours each run under their own
    /// recipe, as drawing layers choose them; `processes` holds one per
    /// contour. The job's own recipe opens and closes the program, and only
    /// its contours carry film and preliminary piercing.
    pub fn schedule_layered(
        settings: &Settings,
        contours: &[cut::Contour],
        film: Option<Film<'_>>,
        processes: &[&Settings],
    ) -> Result<Self> {
        if processes.len() != contours.len() {
            return Err(Error::Invalid("every contour needs its recipe"));
        }
        Self::schedule_geometry(settings, contours, film, true, Some(processes))
    }

    fn schedule_geometry(
        settings: &Settings,
        contours: &[cut::Contour],
        film: Option<Film<'_>>,
        prepared: bool,
        processes: Option<&[&Settings]>,
    ) -> Result<Self> {
        let process = |i: usize| processes.map_or(settings, |p| p[i]);
        let own = |i: usize| std::ptr::eq(process(i), settings);
        let film = match (settings.with_film, film) {
            (false, _) => None,
            (true, None) => {
                return Err(Error::Invalid("Resolve a separate film process before compiling"));
            }
            (true, Some(film)) if film.runs.len() != contours.len() => {
                return Err(Error::Invalid("film geometry must match the ordered contours"));
            }
            (true, Some(film)) => Some(film),
        };
        let filmed: Vec<bool> = (0..contours.len())
            .map(|i| own(i) && film.is_some_and(|f| !f.runs[i].is_empty()))
            .collect();
        let pierced: Vec<bool> =
            (0..contours.len()).map(|i| own(i) && settings.pre_pierce.is_some()).collect();
        let plan = pass::schedule(
            contours.len(),
            &filmed,
            settings.film_batch,
            &pierced,
            settings.pre_pierce.map_or(1, |p| p.batch),
        )?;
        let repierce = settings.pre_pierce.is_none_or(|p| p.repeat_before_cut);
        let mut passes: Vec<Compiled> = Vec::with_capacity(plan.len());
        let mut total = 0usize;
        let mut retention = None;
        for planned in plan {
            let contour = &contours[planned.source];
            let ordinal = passes.len();
            let pass = Pass { ordinal, ..planned };
            match pass.kind {
                PassKind::PrePierce => {
                    passes.push(Compiled::point(pass, contour, process(planned.source))?);
                }
                PassKind::Film => {
                    let film = film.ok_or(Error::Invalid("a film pass needs the film process"))?;
                    let mut bound = film.settings.clone();
                    if let Some((keep_after, short_keep)) = retention {
                        bound.keep_gas_after = keep_after;
                        bound.short_gas_keep = short_keep;
                    }
                    for run in &film.runs[pass.source] {
                        let pass = Pass { ordinal: passes.len(), ..pass };
                        let budget = MAX_SAMPLES.saturating_sub(total);
                        let compiled = Compiled::planned(pass, run, &bound, budget, prepared)?;
                        total += compiled.samples();
                        passes.push(compiled);
                    }
                }
                PassKind::Cut => {
                    let cutting = process(planned.source);
                    retention = Some((cutting.keep_gas_after, cutting.short_gas_keep));
                    let budget = MAX_SAMPLES.saturating_sub(total);
                    let mut compiled = Compiled::planned(pass, contour, cutting, budget, prepared)?;
                    compiled.initial_pierce = !own(planned.source) || repierce;
                    total += compiled.samples();
                    passes.push(compiled);
                }
            }
        }
        Ok(Self { settings: settings.clone(), passes })
    }

    /// Continues from a fraction of the compiled execution path. Preparation
    /// and smoothing are never repeated. The interrupted pass is retimed
    /// from rest on its retained polyline; later passes remain unchanged.
    pub fn continuation(&self, at: usize, fraction: f64, resume_pierce: bool) -> Result<Self> {
        crate::continuation::remaining(self, at, fraction, resume_pierce, None)
    }

    /// Retimes an interrupted cut from its exact settled feedback position.
    /// The position must be within 0.2 mm of the selected retained path.
    pub fn continuation_at(
        &self,
        at: usize,
        fraction: f64,
        resume_pierce: bool,
        position: Point,
    ) -> Result<Self> {
        if fraction >= 1. {
            return Err(Error::Invalid("the interrupted pass has no remaining motion"));
        }
        crate::continuation::remaining(self, at, fraction, resume_pierce, Some(position))
    }

    /// The program for this job, bound to the head's current position and
    /// the controller's height scale.
    pub fn program(&self, binding: Binding) -> Result<Program> {
        self.program_with_return(binding, None)
    }

    /// Returns cold to a saved pause point before continuing. This also
    /// preserves a pause in a travel move, before reaching the next cut.
    pub fn program_with_return(
        &self,
        binding: Binding,
        return_to: Option<Point>,
    ) -> Result<Program> {
        if binding.current.iter().chain(return_to.iter().flatten()).any(|v| !v.is_finite()) {
            return Err(Error::Invalid("the program needs a finite initial position"));
        }
        let hardware = self.settings.hardware.as_ref();
        let z = hardware.is_some_and(|h| h.z_enabled);
        let z_scale = if z {
            binding.z_units_per_mm.filter(|v| *v > 0).ok_or(Error::Invalid(
                "a height-controlled program needs the controller's Z scale",
            ))?
        } else {
            1
        };
        let mut build = Build {
            hardware,
            z,
            z_scale,
            program: Builder::default(),
            state: Outputs::default(),
            current: binding.current,
            residuals: [0.; 2],
        };
        build.open(&self.settings)?;
        if let Some(position) = return_to {
            build.return_to(position, &self.settings)?;
        }
        for (index, pass) in self.passes.iter().enumerate() {
            let previous = index.checked_sub(1).map(|i| &self.passes[i]);
            build.pass(index, previous, self.passes.get(index + 1), pass)?;
        }
        build.close(&self.settings, self.passes.last())?;
        Ok(build.program.finish())
    }
}

/// The record encoding a recipe's motion uses.
fn output_for(s: &Settings) -> Output {
    s.hardware.as_ref().and_then(|h| h.analog_motion).map_or(Output::Pulsed, Output::Analog)
}

/// The records and sections of a program as it is assembled.
#[derive(Default)]
pub(crate) struct Builder {
    records: Vec<Record>,
    sections: Vec<Section>,
    seconds: f64,
    samples: usize,
}

impl Entry {
    fn for_cut(c: &Compiled) -> Result<Self> {
        let starts_in_joint = c.pieces.first().is_some_and(|p| p.process.joint);
        // The cut start reads the fixed layer power before its on-delay;
        // speed modulation and edge PWM belong to the timed writer. An
        // initial joint enters at the joint's reduced power.
        let initial_power = if starts_in_joint {
            let first = c.pieces.first().and_then(|p| p.samples.first());
            f64::from(first.ok_or(Error::Invalid("an initial joint has no samples"))?.power)
        } else {
            c.pieces.first().and_then(|p| p.process.power).unwrap_or(f64::from(c.settings.power))
        };
        Ok(Entry { starts_in_joint, initial_power, perform_pierce: c.initial_pierce })
    }
}

impl Builder {
    /// Check the complete program before a producer allocates its output.
    fn allow(&self, samples: usize, records: usize) -> Result<()> {
        if samples > MAX_SAMPLES.saturating_sub(self.samples) {
            return Err(Error::Budget(
                "the program exceeds 32 million samples; split the layout into smaller jobs",
            ));
        }
        if records > MAX_RECORDS.saturating_sub(self.records.len()) {
            return Err(Error::Budget(
                "the program exceeds 64 million records; split the layout into smaller jobs",
            ));
        }
        Ok(())
    }

    /// Sampled points available to a movement: N points emit N-1 samples
    /// and N records, including its barrier. The motion codec caps N too.
    pub(crate) fn motion_points(&self) -> usize {
        (MAX_SAMPLES.saturating_sub(self.samples) + 1)
            .min(MAX_RECORDS.saturating_sub(self.records.len()))
    }

    pub(crate) fn append(
        &mut self,
        kind: Kind,
        pass: Option<usize>,
        records: Vec<Record>,
        seconds: f64,
    ) -> Result<&mut Section> {
        self.allow(0, records.len())?;
        let start = self.records.len();
        self.records.extend(records);
        self.seconds += seconds;
        self.sections.push(Section {
            kind,
            pass,
            records: start..self.records.len(),
            seconds,
            points: Arc::default(),
            waits: 0,
        });
        let at = self.sections.len() - 1;
        Ok(&mut self.sections[at])
    }

    /// An item tag on its own.
    fn checkpoint(&mut self, kind: Kind, pass: Option<usize>, tag: u32) -> Result<()> {
        self.append(kind, pass, vec![Record::Item(tag)], 0.)?;
        Ok(())
    }

    /// Process commands: a barrier first, zero delays dropped as the
    /// converter drops them, every record validated.
    fn commands(&mut self, kind: Kind, pass: Option<usize>, cmds: Vec<Record>) -> Result<()> {
        let count = cmds.iter().filter(|r| !matches!(r, Record::Delay { millis: 0 })).count();
        self.allow(0, count + 1)?;
        let mut seconds = 0.;
        let mut waits = 0;
        let mut records = vec![Record::Barrier];
        for record in cmds {
            match record {
                Record::Delay { millis: 0 } => continue,
                Record::Delay { millis } => seconds += f64::from(millis) / 1000.,
                Record::WaitStatus { .. } => waits += 1,
                _ => {}
            }
            record.validate()?;
            records.push(record);
        }
        self.append(kind, pass, records, seconds)?.waits = waits;
        Ok(())
    }

    pub(crate) fn motion(
        &mut self,
        kind: Kind,
        pass: Option<usize>,
        movement: Movement,
        points: Arc<[Point]>,
    ) -> Result<()> {
        self.allow(movement.steps, movement.records.len())?;
        self.samples += movement.steps;
        self.append(kind, pass, movement.records, movement.seconds)?.points = points;
        Ok(())
    }

    pub(crate) fn finish(self) -> Program {
        let Self { records, sections, seconds, samples } = self;
        Program { records, sections, seconds, samples }
    }
}

/// Output state carried through one program build. The vendor resets its
/// cached requests at the start of every build.
#[derive(Default)]
struct Outputs {
    /// The last gas selected, if any.
    gas: Option<u8>,
    /// Whether that gas is on now.
    gas_on: bool,
    last_peak: Option<f64>,
    following: bool,
}

impl Outputs {
    /// Every gas output off, MainApp `0x0045_D7C0`.
    fn close_gas(&mut self, h: &Hardware, cmds: &mut Vec<Record>) {
        gas_off(h, cmds);
        self.gas_on = false;
    }
}

/// A relative lift followed by the height controller's completion wait.
fn retraction(s: &Settings) -> Result<[Record; 2]> {
    Ok([
        Record::Lift {
            speed_tenths: to_u32(s.z_follow_speed * 10.)?,
            delta_microns: to_i32(s.retract_height * -1000.)?,
        },
        Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.follow_ms },
    ])
}

/// One program build in progress.
struct Build<'a> {
    hardware: Option<&'a Hardware>,
    z: bool,
    z_scale: u32,
    program: Builder,
    state: Outputs,
    current: Point,
    residuals: [f64; 2],
}

impl Build<'_> {
    fn return_to(&mut self, position: Point, s: &Settings) -> Result<()> {
        self.program.checkpoint(Kind::CheckpointTravel, Some(0), 0x8000_0000)?;
        if (0..2).any(|axis| travel::axis_in_domain((self.current[axis] - position[axis]).abs())) {
            let (plan, movement) = travel_movement(
                [self.current, position],
                s,
                self.residuals,
                0.,
                output_for(s),
                self.program.motion_points(),
            )?;
            self.residuals = movement.residuals;
            self.program.motion(Kind::Travel, Some(0), movement, plan.points)?;
        }
        self.current = position;
        Ok(())
    }

    /// The opening checkpoint and the outputs zeroed before the first contour.
    fn open(&mut self, s: &Settings) -> Result<()> {
        let mut start: Vec<Record> =
            stationary_pwm(self.hardware, 0, s.frequency).into_iter().collect();
        if self.hardware.is_none() {
            start.push(Record::Pwm {
                channel: LaserChannel::Secondary,
                frequency: s.frequency,
                power: 0,
            });
        }
        if let Some(h) = self.hardware {
            for port in [h.laser, h.gate].into_iter().flatten() {
                start.push(Record::output(port, false)?);
            }
            gas_off(h, &mut start);
        }
        self.program.checkpoint(Kind::CheckpointStart, None, u32::MAX)?;
        self.program.commands(Kind::Start, None, start)
    }

    /// One pass: the head and travel to its start, then the point or the cut.
    fn pass(
        &mut self,
        index: usize,
        previous: Option<&Compiled>,
        next: Option<&Compiled>,
        c: &Compiled,
    ) -> Result<()> {
        let s = &c.settings;
        let output = output_for(s);
        self.program.checkpoint(
            Kind::CheckpointTravel,
            Some(index),
            0x8000_0000 | crate::index(index)?,
        )?;
        let (extra_travel, wait_after) = self.head_travel(index, previous, c)?;
        if (0..2).any(|axis| travel::axis_in_domain((self.current[axis] - c.start[axis]).abs())) {
            let (plan, movement) = travel_movement(
                [self.current, c.start],
                s,
                self.residuals,
                extra_travel,
                output,
                self.program.motion_points(),
            )?;
            self.residuals = movement.residuals;
            self.program.motion(Kind::Travel, Some(index), movement, plan.points)?;
        }
        if wait_after {
            let wait = Record::WaitStatus { selector: 3, timeout_ms: s.timeouts.follow_ms };
            self.program.commands(Kind::FrogJumpComplete, Some(index), vec![wait])?;
        }
        if c.pass.kind == PassKind::PrePierce {
            self.point(index, c, output)?;
        } else {
            self.cut(index, c, next, output)?;
        }
        self.current = c.end;
        Ok(())
    }

    /// The head's move between passes, with the crash protection written
    /// around it. Returns the extra travel time a jump needs and whether
    /// the jump's descent must be waited for after the travel. Between the
    /// points of one pre-pierce batch the head stays down when the recipe
    /// asks; the batch's last point retracts like any other.
    fn head_travel(
        &mut self,
        index: usize,
        previous: Option<&Compiled>,
        c: &Compiled,
    ) -> Result<(f64, bool)> {
        let s = &c.settings;
        let retract = previous.map_or(s, |p| &p.settings);
        let keeps_down = previous.is_some_and(|p| {
            p.pass.kind == PassKind::PrePierce
                && c.pass.kind == PassKind::PrePierce
                && p.pass.batch == c.pass.batch
                && p.settings.pre_pierce.is_some_and(|pre| pre.keep_down)
        });
        let transition = if self.z {
            head_transition(previous, c, self.state.following, self.current, self.z_scale)?
        } else {
            Transition::Retract
        };
        let protection =
            self.hardware.and_then(|h| h.crash_height.map(|height| (h.crash_resume_value, height)));
        let insert_protection = previous.is_some() && s.head_travel.protect_insertions;
        if insert_protection && let Some((value, height)) = protection {
            let protect = Record::CrashProtect { value, height };
            self.program.commands(Kind::HeadTravelProtection, Some(index), vec![protect])?;
        }
        let retracts = matches!(transition, Transition::Retract);
        let wait_after = matches!(transition, Transition::Jump { wait_after: true, .. });
        let extra_travel = self.emit_transition(index, transition)?;
        self.retract_for_transfer(index, retract, retracts, keeps_down)?;
        self.transfer_protection(index, s, protection, retracts, insert_protection)?;
        Ok((extra_travel, wait_after))
    }

    fn emit_transition(&mut self, index: usize, transition: Transition) -> Result<f64> {
        Ok(match transition {
            Transition::Jump { command, extra_travel_seconds, .. } => {
                self.program.commands(Kind::FrogJump, Some(index), vec![command])?;
                self.state.following = false;
                extra_travel_seconds
            }
            Transition::KeepHeight => {
                self.program.append(Kind::KeepHeadHeight, Some(index), vec![], 0.)?;
                0.
            }
            Transition::Retract => {
                self.state.following = false;
                0.
            }
        })
    }

    fn retract_for_transfer(
        &mut self,
        index: usize,
        retract: &Settings,
        retracts: bool,
        keeps_down: bool,
    ) -> Result<()> {
        if index > 0 && self.z && retracts && !keeps_down && uses_head(retract) {
            // The vendor retracts with a relative lift, then waits for status 2.
            // Its transfer producer only does this after lowering the head.
            // Startup clearance belongs to the controller's referenced head
            // preparation; a relative lift from home would exceed Z travel.
            self.program.commands(
                Kind::RetractBeforeTravel,
                Some(index),
                retraction(retract)?.into(),
            )?;
        }
        Ok(())
    }

    fn transfer_protection(
        &mut self,
        index: usize,
        s: &Settings,
        protection: Option<(u32, u32)>,
        retracts: bool,
        insert_protection: bool,
    ) -> Result<()> {
        if self.z
            && uses_head(s)
            && retracts
            && let Some((value, height)) = protection
        {
            let protect = Record::CrashProtect { value, height };
            self.program.commands(Kind::TravelProtection, Some(index), vec![protect])?;
        }
        if insert_protection && let Some((_, height)) = protection {
            let protect = Record::CrashProtect { value: 0, height };
            self.program.commands(Kind::BeforeTravelProtection, Some(index), vec![protect])?;
        }
        Ok(())
    }

    /// A pre-pierce point, MainApp `0x0045_F4C0`: the piercing stages and
    /// residue cleaning at the prepared start with no cut, then the item
    /// tag and the gas off, unless the recipe keeps it during processing.
    /// The stages close their own gas; only cleaning leaves it on.
    fn point(&mut self, index: usize, c: &Compiled, output: Output) -> Result<()> {
        let s = &c.settings;
        let cut_start = CutStart {
            settings: s,
            hardware: self.hardware,
            z: self.z,
            z_scale: self.z_scale,
            index,
            output,
            on_delay_ms: s.on_delay_ms,
            position: c.start,
        };
        cut_start.pierce(&mut self.program, &mut self.state, &mut self.residuals)?;
        self.program.checkpoint(Kind::CheckpointPoint, Some(index), crate::index(index)?)?;
        if let Some(h) = self.hardware
            && self.state.gas_on
            && !s.keep_gas_after
        {
            let mut cmds = vec![];
            self.state.close_gas(h, &mut cmds);
            self.program.commands(Kind::PointEnd, Some(index), cmds)?;
        }
        Ok(())
    }

    /// The cut itself: its start sequence, its pieces with the stops
    /// between them, and its end sequence.
    fn cut(
        &mut self,
        index: usize,
        c: &Compiled,
        next: Option<&Compiled>,
        output: Output,
    ) -> Result<()> {
        let s = &c.settings;
        let h = self.hardware;
        let mut cut_start = CutStart {
            settings: s,
            hardware: h,
            z: self.z,
            z_scale: self.z_scale,
            index,
            output,
            on_delay_ms: c.overrides.on_delay_ms.unwrap_or(s.on_delay_ms),
            position: c.start,
        };
        self.contour_control(index)?;
        let entry = Entry::for_cut(c)?;
        cut_start.append(&mut self.program, &mut self.state, &mut self.residuals, entry)?;
        if h.is_none() {
            self.program.checkpoint(Kind::CheckpointCut, Some(index), crate::index(index)?)?;
        }
        self.cut_pieces(index, c, &mut cut_start, output)?;
        self.cut_end(index, c, next)
    }

    fn contour_control(&mut self, index: usize) -> Result<()> {
        if let Some(hardware) = self.hardware.filter(|h| h.co2_contour_control) {
            let value = match hardware.laser {
                Some(9) => 9,
                Some(10) => 10,
                _ => 1001,
            };
            self.program.commands(
                Kind::ContourControlStart,
                Some(index),
                vec![Record::ContourControl(value)],
            )?;
        }
        Ok(())
    }

    fn cut_pieces(
        &mut self,
        index: usize,
        c: &Compiled,
        cut_start: &mut CutStart<'_>,
        output: Output,
    ) -> Result<()> {
        let s = &c.settings;
        if c.events.len() != c.pieces.len() + 1 {
            return Err(Error::Invalid("a contour needs one event per piece boundary"));
        }
        for (piece_index, piece) in c.pieces.iter().enumerate() {
            if let Some(event) = c.events[piece_index] {
                cut_start.append_stop(
                    &mut self.program,
                    &mut self.state,
                    &mut self.residuals,
                    event,
                )?;
            }
            self.program.allow(piece.samples.len(), piece.samples.len() + 1)?;
            let movement =
                motion::frame(&piece.plan, &piece.samples, s.scales(), self.residuals, output)?;
            self.residuals = movement.residuals;
            let kind =
                if c.pass.kind == PassKind::Film { Kind::Film } else { Kind::of_piece(piece) };
            self.program.motion(kind, Some(index), movement, piece.plan.points.clone())?;
            cut_start.position = *piece
                .plan
                .points
                .last()
                .ok_or(Error::Invalid("a cutting piece has no endpoint"))?;
        }
        if let Some(event) = c.events[c.pieces.len()] {
            cut_start.append_stop(
                &mut self.program,
                &mut self.state,
                &mut self.residuals,
                event,
            )?;
        }
        Ok(())
    }

    /// Laser off, PWM zeroed and gas off after a contour. The gas stays on
    /// when the recipe keeps it during processing, or when the next pass
    /// keeps it over a transfer shorter than the machine's short-transfer
    /// distance (MainApp `0x0045_E6D0`, strictly shorter); the last pass
    /// always closes it.
    fn cut_end(&mut self, index: usize, c: &Compiled, next: Option<&Compiled>) -> Result<()> {
        let s = &c.settings;
        let h = self.hardware;
        let co2_pwm = h.is_some_and(|h| h.co2_contour_control);
        let mut cmds = vec![];
        if co2_pwm {
            cmds.push(Record::ContourControl(0));
        }
        cmds.push(Record::Delay { millis: c.overrides.off_delay_ms.unwrap_or(s.off_before_ms) });
        if !co2_pwm {
            cmds.extend(stationary_pwm(h, 0, s.frequency));
        }
        if let Some(h) = h {
            if let Some(port) = h.laser {
                cmds.push(Record::output(port, false)?);
            }
            // The vendor closes the laser output before the secondary PWM record.
            if co2_pwm {
                cmds.extend(stationary_pwm(Some(h), 0, s.frequency));
            }
            cmds.push(Record::Delay { millis: s.off_after_ms });
            if !keeps_gas(c, next) {
                self.state.close_gas(h, &mut cmds);
            }
        }
        self.program.commands(Kind::CutEnd, Some(index), cmds)
    }

    /// The closing checkpoint, the outputs zeroed, the head parked and the
    /// final barrier.
    fn close(&mut self, s: &Settings, last: Option<&Compiled>) -> Result<()> {
        self.program.checkpoint(Kind::CheckpointFinished, None, u32::MAX - 1)?;
        let mut end: Vec<Record> =
            stationary_pwm(self.hardware, 0, s.frequency).into_iter().collect();
        if let Some(h) = self.hardware {
            gas_off(h, &mut end);
            for port in [h.laser, h.gate].into_iter().flatten() {
                end.push(Record::output(port, false)?);
            }
            let final_settings = last.map_or(s, |c| &c.settings);
            if self.z && uses_head(final_settings) {
                end.extend(retraction(final_settings)?);
            }
            if let Some(channel) = h.peak_channel.filter(|_| !h.keep_peak_output) {
                end.push(Record::Analog { channel, value: 0 });
            }
        }
        self.program.commands(Kind::End, None, end)?;
        // The converter closes every program with a barrier.
        self.program.append(Kind::EndBarrier, None, vec![Record::Barrier], 0.)?;
        Ok(())
    }
}

/// Whether the gas stays on after `c` into `next`, MainApp `0x0045_E6D0`:
/// the recipe keeps it during processing, or the next pass keeps it over
/// a transfer shorter than the machine's short-transfer distance. A film
/// pass next reads the current recipe's flag, as the vendor's scratch bank
/// does; the distance runs between the prepared machining positions.
fn keeps_gas(c: &Compiled, next: Option<&Compiled>) -> bool {
    let Some(next) = next else { return false };
    let s = &c.settings;
    let short_keep = if next.pass.kind == PassKind::Film {
        s.short_gas_keep
    } else {
        next.settings.short_gas_keep
    };
    let transfer = (next.start[0] - c.end[0]).hypot(next.start[1] - c.end[1]);
    s.keep_gas_after
        || (short_keep && s.head_travel.short_transfer.is_some_and(|limit| transfer < limit))
}

/// How a cut begins.
#[derive(Clone, Copy)]
struct Entry {
    starts_in_joint: bool,
    initial_power: f64,
    perform_pierce: bool,
}

/// The cut-start sequence, shared by the initial cut, re-pierces and
/// pre-pierce points.
struct CutStart<'a> {
    settings: &'a Settings,
    hardware: Option<&'a Hardware>,
    z: bool,
    z_scale: u32,
    index: usize,
    output: Output,
    on_delay_ms: u32,
    /// Current execution position, including the prepared shift/placement.
    position: Point,
}

impl CutStart<'_> {
    /// A stop between pieces: a re-pierce, a cooling dwell, or both.
    fn append_stop(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        residuals: &mut [f64; 2],
        event: StopEvent,
    ) -> Result<()> {
        if event.repierce {
            if self.settings.pierce.is_empty() {
                return Err(Error::Invalid(
                    "re-piercing after a joint needs configured pierce stages",
                ));
            }
            let entry = Entry {
                starts_in_joint: false,
                initial_power: f64::from(self.settings.power),
                perform_pierce: true,
            };
            self.append(program, outputs, residuals, entry)?;
        }
        if event.cool_ms > 0 {
            let mut cmds: Vec<Record> =
                stationary_pwm(self.hardware, 0, self.settings.frequency).into_iter().collect();
            if let Some(hardware) = self.hardware.filter(|h| h.analog_motion.is_some()) {
                // Analog motion carries the laser gate separately from the
                // level, so cooling must switch the output off itself. The
                // next sample restores its own gate.
                let port = hardware
                    .laser
                    .ok_or(Error::Invalid("analog cooling needs an assigned laser output"))?;
                cmds.push(Record::output(port, false)?);
            }
            cmds.push(Record::Delay { millis: event.cool_ms });
            program.commands(Kind::Cooling, Some(self.index), cmds)?;
        }
        Ok(())
    }

    /// Piercing and residue cleaning when the cut pierces, then the cut
    /// start proper. Nothing is emitted without process hardware.
    fn append(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        residuals: &mut [f64; 2],
        entry: Entry,
    ) -> Result<()> {
        let Some(h) = self.hardware else { return Ok(()) };
        if !entry.starts_in_joint && entry.perform_pierce {
            self.pierce(program, outputs, residuals)?;
        }
        program.checkpoint(Kind::CheckpointCut, Some(self.index), crate::index(self.index)?)?;
        self.start_cut(program, outputs, h, entry)
    }

    /// The piercing stages in execution order, then residue cleaning.
    /// Nothing is emitted without process hardware.
    fn pierce(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        residuals: &mut [f64; 2],
    ) -> Result<()> {
        let Some(h) = self.hardware else { return Ok(()) };
        let mut prior_height = 0.;
        for (stage_index, stage) in self.settings.pierce.iter().enumerate() {
            self.pierce_stage(program, outputs, stage, stage_index == 0, &mut prior_height)?;
        }
        if let Some(r) = &self.settings.residue {
            self.residue_pass(program, outputs, residuals, h, r)?;
        }
        Ok(())
    }

    /// One pierce stage: gas and peak, the head at the stage height, laser
    /// on, the dwell or bolt drill or gradual descent, laser off, gas off.
    fn pierce_stage(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        stage: &PierceStage,
        first: bool,
        prior_height: &mut f64,
    ) -> Result<()> {
        let (s, index) = (self.settings, self.index);
        let h = self.hardware.ok_or(Error::Invalid("piercing needs process hardware"))?;
        outputs.following = false;
        let mut cmds = vec![];
        gas_on(h, stage.gas, stage.pressure, outputs, &mut cmds)?;
        peak(h, stage.peak_current, s.timeouts.analog_minimum, &mut outputs.last_peak, &mut cmds)?;
        if self.z {
            self.head_to_stage(&mut cmds, h, stage, first, *prior_height)?;
            *prior_height = stage.height;
        }
        for port in [h.gate, h.laser].into_iter().flatten() {
            cmds.push(Record::output(port, true)?);
        }
        cmds.extend(stationary_pwm(Some(h), stage.power, stage.frequency));
        if s.smooth_pierce {
            // The smooth producer hands straight into the cut: no dwell,
            // shutdown, gas-off or residue cleaning.
            return program.commands(Kind::PierceSmooth, Some(index), cmds);
        }
        cmds.push(Record::Delay { millis: stage.before_off_ms });
        self.pierce_dwell(program, &mut cmds, stage, prior_height)?;
        if let Some(port) = h.laser {
            cmds.push(Record::output(port, false)?);
        }
        cmds.extend(stationary_pwm(Some(h), 0, stage.frequency));
        cmds.push(Record::Delay { millis: stage.after_off_ms });
        outputs.close_gas(h, &mut cmds);
        program.commands(Kind::Pierce(stage.index), Some(index), cmds)
    }

    fn pierce_dwell(
        &self,
        program: &mut Builder,
        cmds: &mut Vec<Record>,
        stage: &PierceStage,
        prior_height: &mut f64,
    ) -> Result<()> {
        let index = self.index;
        if stage.gradual && stage.gradual_ms > 0 {
            self.gradual_stage(program, cmds, stage, prior_height)?;
        } else if let Some(bolt) = stage.bolt {
            program.commands(Kind::PierceStart(stage.index), Some(index), std::mem::take(cmds))?;
            self.bolt_drill(program, stage, bolt, true)?;
        } else {
            cmds.push(Record::Delay { millis: stage.duration_ms });
        }
        Ok(())
    }

    fn gradual_stage(
        &self,
        program: &mut Builder,
        cmds: &mut Vec<Record>,
        stage: &PierceStage,
        prior_height: &mut f64,
    ) -> Result<()> {
        let (s, index) = (self.settings, self.index);
        program.commands(Kind::PierceStart(stage.index), Some(index), std::mem::take(cmds))?;
        let target = self.next_stage_height(stage)?;
        cmds.push(gradual_pierce(stage.height, target, stage.gradual_ms, stage.index == 0)?);
        if let Some(bolt) = stage.bolt {
            program.commands(Kind::PierceGradual, Some(index), std::mem::take(cmds))?;
            self.bolt_drill(program, stage, bolt, false)?;
            // The vendor waits after the stationary PWM ramp.
            cmds.push(Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.section_drill_ms });
        } else {
            let timeout_ms = s.timeouts.section_drill_ms + stage.gradual_ms;
            cmds.push(Record::WaitStatus { selector: 2, timeout_ms });
        }
        *prior_height = target;
        Ok(())
    }

    /// The head's move to a stage: the first stage goes to its pierce
    /// height and lifts the rest, later stages lift relative to the last.
    fn head_to_stage(
        &self,
        cmds: &mut Vec<Record>,
        h: &Hardware,
        stage: &PierceStage,
        first: bool,
        prior_height: f64,
    ) -> Result<()> {
        let s = self.settings;
        if first {
            let maximum = if s.smooth_pierce { 10. } else { h.direct_drill_max };
            let base = if stage.height > maximum { 1. } else { stage.height };
            cmds.push(Record::PierceHeight {
                follow_speed_tenths: to_u32(s.z_follow_speed * 10.)?,
                height: to_u32(base * 1000.)? | s.vibration_word,
                lift_speed_tenths: to_u32(s.z_up_speed * 10.)?,
                lift_microns: to_i32((stage.height - base) * 1000.)?,
            });
            cmds.push(Record::WaitStatus { selector: 4, timeout_ms: s.timeouts.section_drill_ms });
        } else {
            cmds.push(Record::Lift {
                speed_tenths: to_u32(s.z_up_speed * 10.)?,
                delta_microns: to_i32((stage.height - prior_height) * -1000.)?,
            });
            cmds.push(Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.section_drill_ms });
        }
        Ok(())
    }

    /// Where a gradual stage descends to: the cut height from the final
    /// stage, the next stage's height otherwise.
    fn next_stage_height(&self, stage: &PierceStage) -> Result<f64> {
        if stage.index == 0 {
            return Ok(self.settings.cut_height);
        }
        self.settings
            .pierce
            .iter()
            .find(|next| next.index + 1 == stage.index)
            .map(|next| next.height)
            .ok_or(Error::Invalid("a gradual pierce stage has no next stage"))
    }

    /// The residue-cleaning pass: gas and peak, the head at cleaning
    /// height, laser on, the spiral and its return, laser off.
    fn residue_pass(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        residuals: &mut [f64; 2],
        h: &Hardware,
        r: &crate::settings::Residue,
    ) -> Result<()> {
        let (s, index) = (self.settings, self.index);
        outputs.following = false;
        let mut cmds = Vec::new();
        let gas_delay = gas_select(h, r.gas, r.pressure, outputs, &mut cmds)?;
        peak(h, r.peak_current, s.timeouts.analog_minimum, &mut outputs.last_peak, &mut cmds)?;
        if self.z {
            cmds.push(Record::Lift {
                speed_tenths: to_u32(s.z_up_speed * 10.)?,
                delta_microns: to_i32(r.height * -1000.)?,
            });
            cmds.push(Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.section_drill_ms });
        }
        cmds.push(Record::Delay { millis: gas_delay });
        for port in [h.gate, h.laser].into_iter().flatten() {
            cmds.push(Record::output(port, true)?);
        }
        cmds.extend(stationary_pwm(Some(h), r.power, r.frequency));
        program.commands(Kind::ResidueStart, Some(index), cmds)?;
        let anchor = self.position;
        for (i, (plan, movement)) in
            residue_movements(s, *residuals, self.output, program.motion_points())?
                .into_iter()
                .enumerate()
        {
            *residuals = movement.residuals;
            let points = plan.points.iter().map(|p| [p[0] + anchor[0], p[1] + anchor[1]]).collect();
            let kind = if i == 0 { Kind::ResidueSpiral } else { Kind::ResidueReturn };
            program.motion(kind, Some(index), movement, points)?;
        }
        let mut cmds = Vec::new();
        if let Some(port) = h.laser {
            cmds.push(Record::output(port, false)?);
        }
        cmds.extend(stationary_pwm(Some(h), 0, r.frequency));
        program.commands(Kind::ResidueEnd, Some(index), cmds)
    }

    /// Gas and peak for the cut, crash protection, the follow, laser on and
    /// the on-delay.
    fn start_cut(
        &self,
        program: &mut Builder,
        outputs: &mut Outputs,
        h: &Hardware,
        entry: Entry,
    ) -> Result<()> {
        let s = self.settings;
        let mut cmds = vec![];
        gas_on(h, s.gas, s.pressure, outputs, &mut cmds)?;
        peak(h, s.peak_current, s.timeouts.analog_minimum, &mut outputs.last_peak, &mut cmds)?;
        if let Some(height) = h.crash_height {
            cmds.push(Record::CrashProtect { value: 0, height });
        }
        self.follow(&mut cmds, outputs)?;
        outputs.following =
            self.z && (s.absolute_cut_height.is_some() || (!s.no_follow && !s.fixed_height));
        for port in [h.gate, h.laser].into_iter().flatten() {
            cmds.push(Record::output(port, true)?);
        }
        cmds.extend(stationary_pwm(Some(h), to_u8(entry.initial_power.round())?, s.frequency));
        if !entry.starts_in_joint {
            cmds.push(Record::Delay { millis: self.on_delay_ms });
        }
        program.commands(Kind::CutStart, Some(self.index), cmds)
    }

    /// The head's move into the cut: an absolute height, or a follow unless
    /// the head is already following; a fixed height follows then lifts.
    fn follow(&self, cmds: &mut Vec<Record>, outputs: &Outputs) -> Result<()> {
        let s = self.settings;
        if !self.z {
            return Ok(());
        }
        if let Some(height) = s.absolute_cut_height {
            cmds.push(Record::HeightAbsolute {
                speed_tenths: to_u32(s.z_follow_speed * 10.)?,
                height_microns: to_u32(height * 1000.)?,
            });
            cmds.push(Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.follow_ms });
        } else if !s.no_follow && !outputs.following {
            let target = if s.fixed_height {
                1000
            } else {
                to_u32(s.cut_height.max(0.5) * f64::from(self.z_scale))?
            };
            if target > 65535 {
                return Err(Error::Invalid("the cut height exceeds the 16-bit follow target"));
            }
            cmds.push(Record::Follow {
                speed_tenths: to_u32(s.z_follow_speed * 10.)?,
                height: target | if s.fixed_height { 0 } else { s.vibration_word },
            });
            cmds.push(Record::WaitStatus { selector: 3, timeout_ms: s.timeouts.follow_ms });
            if s.fixed_height {
                cmds.push(Record::Lift {
                    speed_tenths: to_u32(s.z_follow_speed * 10.)?,
                    delta_microns: to_i32((s.cut_height + 1.) * -1000.)?,
                });
                cmds.push(Record::WaitStatus { selector: 2, timeout_ms: s.timeouts.follow_ms });
            }
        }
        Ok(())
    }

    /// A bolt drill, MainApp `0x0045_DC50` and `0x0045_1540`: stationary
    /// padding records first, then one PWM request per cycle ramping to the
    /// target. A gradual stage skips the padding.
    fn bolt_drill(
        &self,
        program: &mut Builder,
        stage: &PierceStage,
        target: (u8, u16),
        padding: bool,
    ) -> Result<()> {
        let (index, cycle_us) = (self.index, self.settings.interpolation_cycle_us);
        let hardware =
            self.hardware.ok_or(Error::Invalid("bolt drilling needs process hardware"))?;
        if cycle_us == 0 || !1000u32.is_multiple_of(cycle_us) {
            return Err(Error::Invalid(
                "bolt drilling needs an interpolation cycle dividing one millisecond",
            ));
        }
        if hardware.analog_motion.is_some() {
            return Err(Error::Invalid("bolt drilling needs a PWM process output"));
        }
        let count = (1000 / cycle_us)
            .checked_mul(stage.duration_ms)
            .ok_or(Error::Budget("bolt drill samples overflow"))?;
        let count =
            usize::try_from(count).map_err(|_| Error::Budget("bolt drill samples overflow"))?;
        if count == 0 {
            return Ok(());
        }
        let padding_samples = if padding { count } else { 0 };
        program.allow(padding_samples, count + padding_samples + usize::from(padding))?;
        if padding {
            let still = Record::pulsed(0, 0, PulsedFields { power: 0, frequency: 5000, tail: 0 });
            let mut records = vec![still; count];
            records.push(Record::Barrier);
            program.samples += count;
            program.append(
                Kind::BoltPadding,
                Some(index),
                records,
                float(count) * f64::from(cycle_us) * 1e-6,
            )?;
        }
        let mut records = Vec::with_capacity(count);
        for step in 1..=count {
            let fraction = float(step) / float(count);
            let power =
                f64::from(stage.power) + (f64::from(target.0) - f64::from(stage.power)) * fraction;
            let frequency = f64::from(stage.frequency)
                + (f64::from(target.1) - f64::from(stage.frequency)) * fraction;
            let record = stationary_pwm(
                Some(hardware),
                to_u8(power.trunc())?,
                crate::to_u16(frequency.trunc())?,
            )
            .ok_or(Error::Invalid("bolt drilling needs stationary PWM"))?;
            record.validate()?;
            records.push(record);
        }
        program.append(Kind::BoltRamp, Some(index), records, 0.)?;
        Ok(())
    }
}

/// The vendor's stage handoff, NCModule `0x1004_4AFC`: earlier stages descend
/// to the next stage's height; the final stage hands over to cut height.
fn gradual_pierce(
    start_height: f64,
    target_height: f64,
    duration_ms: u32,
    final_stage: bool,
) -> Result<Record> {
    if !start_height.is_finite()
        || !target_height.is_finite()
        || start_height < 0.
        || target_height < 0.
        || duration_ms == 0
    {
        return Err(Error::Invalid("a pierce ramp needs finite heights and a positive duration"));
    }
    let descent = start_height - target_height;
    let speed = (descent.abs() / (f64::from(duration_ms) * 0.001)).clamp(0.1, 200.);
    let speed_tenths = to_u32(speed * 10.)?;
    let descent_microns = to_i32((descent * 1000.).trunc())?;
    Ok(if final_stage {
        Record::PierceRamp {
            speed_tenths,
            descent_microns,
            cut_height_microns: to_u32((target_height * 1000.).trunc())?,
        }
    } else {
        Record::Lift { speed_tenths, delta_microns: descent_microns }
    })
}

/// Every gas valve and proportional channel off.
fn gas_off(h: &Hardware, cmds: &mut Vec<Record>) {
    for port in h.gas_ports.iter().chain(h.ratio_ports.iter()).flatten() {
        if let Ok(record) = Record::output(*port, false) {
            cmds.push(record);
        }
    }
    for channel in h.ratio_channels.iter().flatten() {
        cmds.push(Record::Analog { channel: *channel, value: 0 });
    }
}

/// Selects a gas and waits the delay it needs.
fn gas_on(
    h: &Hardware,
    gas: u8,
    pressure: f64,
    outputs: &mut Outputs,
    cmds: &mut Vec<Record>,
) -> Result<()> {
    if !h.gas_enabled {
        return Ok(());
    }
    let delay = gas_select(h, gas, pressure, outputs, cmds)?;
    cmds.push(Record::Delay { millis: delay });
    Ok(())
}

/// Opens the selected valve and its proportional pressure, MainApp
/// `0x0045_D4C0`, and returns the delay to wait. A route that is on and
/// unchanged is left open and configured again; any other selection
/// clears every gas output first.
fn gas_select(
    h: &Hardware,
    gas: u8,
    pressure: f64,
    outputs: &mut Outputs,
    cmds: &mut Vec<Record>,
) -> Result<u32> {
    if !h.gas_enabled {
        return Ok(0);
    }
    let retained = outputs.gas_on && outputs.gas == Some(gas);
    if !retained {
        gas_off(h, cmds);
    }
    let i = usize::from(gas);
    let j = i % 3;
    if let Some(port) = h.gas_ports.get(i).copied().flatten() {
        cmds.push(Record::output(port, true)?);
    }
    if let Some(port) = h.ratio_ports[j] {
        cmds.push(Record::output(port, true)?);
    }
    if let Some(channel) = h.ratio_channels[j] {
        let value = gas_pressure_word(h, j, pressure)?;
        cmds.push(Record::Analog { channel, value });
    }
    let changed = outputs.gas.is_some_and(|previous| previous != gas);
    let wait = gas_delay(h, outputs.gas.is_none(), !outputs.gas_on, changed, 0);
    outputs.gas = Some(gas);
    outputs.gas_on = true;
    Ok(wait)
}

fn gas_pressure_word(h: &Hardware, j: usize, pressure: f64) -> Result<u32> {
    if !pressure.is_finite() || pressure < 0. || pressure > h.gas_max_pressure[j] {
        return Err(Error::Invalid("gas pressure exceeds the configured analog range"));
    }
    let value = match &h.gas_calibration[j] {
        Some(calibration) => to_u32(calibration.voltage(pressure)? * 1000.)?,
        None => to_u32(pressure * (10_000. / h.gas_max_pressure[j]))?,
    };
    if value > 10_000 {
        return Err(Error::Invalid("gas pressure exceeds the analog maximum"));
    }
    Ok(value)
}

/// The wait after a gas selection, MainApp `0x0045_D8B0`: the first-gas
/// allowance once per program, the base delay when the gas was off, the
/// change allowance when the selector changed, less the `overlap_ms` a
/// caller credits, never below zero. The callers here credit none.
fn gas_delay(h: &Hardware, first: bool, was_off: bool, changed: bool, overlap_ms: u32) -> u32 {
    let mut wait = 0;
    if first {
        wait += h.first_gas_delay_ms;
    }
    if was_off {
        wait += h.gas_delay_ms;
    }
    if changed {
        wait += h.change_gas_delay_ms;
    }
    wait.saturating_sub(overlap_ms)
}

/// A stationary PWM record, or none under CO2 analog control, whose motion
/// writer carries the level itself (NCModule `0x1004_3FB9`).
fn stationary_pwm(hardware: Option<&Hardware>, power: u8, frequency: u16) -> Option<Record> {
    if hardware.is_some_and(|h| h.analog_motion.is_some()) {
        return None;
    }
    let channel = if hardware.is_some_and(|h| h.secondary_pwm) {
        LaserChannel::Secondary
    } else {
        LaserChannel::Primary
    };
    Some(Record::Pwm { channel, frequency, power })
}

/// The peak current request. A repeated request is suppressed, except under
/// analog motion, whose writer shares the channel; the first request is
/// always emitted, even at zero.
fn peak(
    h: &Hardware,
    power: f64,
    minimum: u32,
    last: &mut Option<f64>,
    cmds: &mut Vec<Record>,
) -> Result<()> {
    if h.analog_motion.is_none() && *last == Some(power) {
        return Ok(());
    }
    if let Some(channel) = h.peak_channel {
        let value = to_u32(power * h.peak_factor)?;
        cmds.push(Record::Analog {
            channel,
            value: if value > 0 { value.max(minimum) } else { 0 },
        });
    }
    *last = Some(power);
    Ok(())
}

/// The residue spiral and the return to its origin, in local coordinates.
fn residue_movements(
    s: &Settings,
    residuals: [f64; 2],
    output: Output,
    max_points: usize,
) -> Result<Vec<(Plan, Movement)>> {
    let Some(r) = &s.residue else { return Ok(Vec::new()) };
    let spiral = residue::Spiral::new(r.radius, r.turns, r.speed)?;
    let plan = spiral.plan(
        crate::planner::Settings {
            acceleration: s.acceleration,
            acceleration_time: s.acceleration_time,
            spline_accuracy: s.spline_accuracy,
            speed: r.speed,
            corner_speed_floor: 0.,
            interval_ms: s.cadence_ms(),
            slow_start_length: 0.,
            slow_start_speed: s.speed,
            slow_end_length: 0.,
            slow_end_speed: s.speed,
        },
        max_points.min(1_000_000),
    )?;
    let sample = Sample { analog: r.power, power: r.power, frequency: r.frequency, tail: 0xffff };
    let outward =
        motion::frame(&plan, &vec![sample; plan.points.len() - 1], s.scales(), residuals, output)?;
    // The spiral writer emits samples one through N without an endpoint and
    // returns to the origin with a separately prepared travel.
    let mut back = s.clone();
    back.frequency = 0;
    let (return_plan, inward) = travel_movement(
        [[r.radius, 0.], [0., 0.]],
        &back,
        outward.residuals,
        0.,
        output,
        max_points.saturating_sub(outward.records.len()),
    )?;
    Ok(vec![(plan, outward), (return_plan, inward)])
}

/// Rapid travel with zero power, stretched when the head needs more time.
fn travel_movement(
    route: [Point; 2],
    s: &Settings,
    residuals: [f64; 2],
    extra_seconds: f64,
    output: Output,
    max_points: usize,
) -> Result<(Plan, Movement)> {
    let [start, end] = route;
    let length = (end[0] - start[0]).abs().max((end[1] - start[1]).abs());
    let settings = travel_settings(s, length, extra_seconds)?;
    let plan = travel::plan(start, end, settings, max_points.min(1_000_000))?;
    let sample = Sample { analog: 0, power: 0, frequency: s.frequency, tail: 0 };
    let movement =
        motion::frame(&plan, &vec![sample; plan.points.len() - 1], s.scales(), residuals, output)?;
    Ok((plan, movement))
}

/// Travel settings, slowed when the head jump needs `extra` seconds more
/// than the travel would take, MainApp `0x0045_E6D0`.
fn travel_settings(s: &Settings, length: f64, extra: f64) -> Result<travel::Settings> {
    let mut settings = travel::Settings {
        speed: s.travel_speed.min(s.native_speed_cap.unwrap_or(f64::INFINITY)),
        acceleration: s.travel_acceleration,
        acceleration_time: s.travel_acceleration_time,
        cadence_ms: s.cadence_ms(),
    };
    let jerk = 2. * settings.acceleration / settings.acceleration_time;
    if extra > 0.
        && f64::from(travel::duration_millis(length, settings.speed, settings.acceleration, jerk)?)
            * 0.001
            < extra
    {
        let step = (extra / 4.).max(25.);
        let jerk = (((length / 2.) / step) / step / step).max(10000.);
        settings.speed = ((length / 2.) / step).max(20.);
        settings.acceleration = ((length / 4.) / step / step).max(2000.);
        settings.acceleration_time = 2. * settings.acceleration / jerk;
    }
    Ok(settings)
}

/// How the head moves to the next pass.
enum Transition {
    Retract,
    KeepHeight,
    Jump { command: Record, wait_after: bool, extra_travel_seconds: f64 },
}

fn word(value: f64, maximum: u32) -> Result<u32> {
    if !value.is_finite() || value < 0. || value.trunc() > f64::from(maximum) {
        return Err(Error::Invalid("a head jump field is out of range"));
    }
    to_u32(value)
}

/// Whether the head may stay down between two passes: a short move at
/// the same cut height into an ordinary cut, or a pre-pierced one, that
/// does not pierce.
fn keeps_height(before: &Settings, next: &Compiled, distance: f64) -> bool {
    let s = &next.settings;
    !s.no_follow
        && (s.machining_kind == 0 || s.pre_pierce.is_some())
        && (before.cut_height - s.cut_height).abs() < 0.01
        && s.head_travel.keep_height_below.is_some_and(|limit| distance < limit)
        && !s.smooth_pierce
        && (!next.initial_pierce || s.pierce.is_empty())
}

/// Whether a jump is ruled out and the head must retract instead; the
/// vendor never jumps into a film pass.
fn must_retract(before: &Settings, next: &Compiled) -> bool {
    let s = &next.settings;
    before.machining_kind == 5
        || next.pass.kind == PassKind::Film
        || s.layer >= 11
        || s.no_follow
        || matches!(s.machining_kind, 1 | 5)
        || !s.head_travel.frog_jump
        || s.hardware.as_ref().is_none_or(|h| !h.z_enabled || h.crash_height.is_some())
}

/// The follow target the jump lands on, in millimetres. A pass whose
/// piercing already ran as a preliminary point, and is not repeated,
/// lands at the cut height like a plain cut.
fn jump_target(next: &Compiled) -> Result<f64> {
    let s = &next.settings;
    let pre_pierced = s.pre_pierce.is_some_and(|p| !p.repeat_before_cut);
    if s.machining_kind == 0 || (pre_pierced && !matches!(s.machining_kind, 1 | 5)) {
        return if s.smooth_pierce {
            s.pierce
                .first()
                .map(|stage| stage.height)
                .ok_or(Error::Invalid("smooth piercing needs a stage height"))
        } else {
            Ok(s.cut_height)
        };
    }
    let target = s
        .head_travel
        .jump_pierce_height
        .ok_or(Error::Invalid("frog jumping needs a pierce target"))?;
    let too_high = s.hardware.as_ref().is_some_and(|h| target > h.direct_drill_max);
    Ok(if too_high { 1. } else { target })
}

/// Inter-contour head planning, MainApp `0x0045_E6D0`. Cached head state is
/// local to one program build, so a resumed first contour never assumes
/// the head is following.
fn head_transition(
    previous: Option<&Compiled>,
    next: &Compiled,
    following: bool,
    current: Point,
    z_scale: u32,
) -> Result<Transition> {
    let Some(previous) = previous else { return Ok(Transition::Retract) };
    let before = &previous.settings;
    let s = &next.settings;
    if !following {
        return Ok(Transition::Retract);
    }
    let delta = [next.start[0] - current[0], next.start[1] - current[1]];
    if keeps_height(before, next, delta[0].hypot(delta[1])) {
        return Ok(Transition::KeepHeight);
    }
    if must_retract(before, next) {
        return Ok(Transition::Retract);
    }
    let minimum = s
        .head_travel
        .minimum_height
        .ok_or(Error::Invalid("frog jumping needs a minimum height"))?;
    let maximum = before.retract_height.max(minimum);
    let speed = s.z_follow_speed;
    let profile = JumpProfile::from_follow_speed(speed)?;
    let length = delta[0].abs().max(delta[1].abs());
    // The head planner returns zero below 0.005 mm, independently of the
    // travel producer's own domain floor.
    let travel_ms = if length < 0.005 {
        0
    } else {
        travel::duration_millis(
            length,
            s.travel_speed.min(s.native_speed_cap.unwrap_or(f64::INFINITY)),
            s.travel_acceleration,
            2. * s.travel_acceleration / s.travel_acceleration_time,
        )?
    };
    let requested = f64::from(travel_ms) * 0.001 + 0.1;
    let minimum_time = profile.motion_seconds(minimum)?.iter().sum::<f64>();
    let plan = profile.solve(requested.max(minimum_time), maximum)?;
    // The vendor truncates the height to whole millimetres before scaling.
    // Its fractional-minimum undershoot is not reproduced.
    let lift = plan.height.trunc().max(minimum.ceil());
    let speed_word = word(speed * 10., u32::from(u16::MAX))?;
    if speed_word == 0 {
        return Err(Error::Invalid("the head jump speed rounds to zero"));
    }
    let target_word = word(jump_target(next)? * f64::from(z_scale), u32::from(u16::MAX))?;
    let command = Record::FrogJump {
        speed_tenths: speed_word,
        lift: word(lift * f64::from(z_scale), i32::MAX.cast_unsigned())?,
        hold_ms: word(plan.hold_seconds * 1000., 600_000)?,
        down_speed_tenths: crate::to_u16(f64::from(speed_word))?,
        // The jump descriptor keeps the previous contour's vibration word.
        target: target_word | before.vibration_word,
    };
    // The vendor subtracts the raw added time here; only a positive value
    // ever reaches the travel producer.
    let extra_travel_seconds =
        if requested < minimum_time { (minimum_time - 100.).max(0.) } else { 0. };
    Ok(Transition::Jump { command, wait_after: true, extra_travel_seconds })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tagged(
        processes: Vec<SegmentProcess>,
        curves: Vec<Segment>,
    ) -> Vec<openlaser_core::toolpath::PreparedSegment<Segment>> {
        assert_eq!(curves.len(), processes.len());
        curves
            .into_iter()
            .zip(processes)
            .map(|(c, p)| openlaser_core::toolpath::PreparedSegment::new(c, p))
            .collect()
    }

    use crate::cut::{Contour, Segment};
    use crate::settings::{PrePierce, SegmentProcess};

    fn line(start: Point, end: Point) -> Contour {
        Contour {
            segments: tagged(vec![SegmentProcess::default()], vec![Segment::Line { start, end }]),
            overrides: Overrides::default(),
        }
    }

    /// A line led into by a 2 mm lead-in and out of by a 2 mm lead-out.
    fn led(start: Point, end: Point) -> Contour {
        let lead = |role| SegmentProcess { lead: Some(role), ..SegmentProcess::default() };
        Contour {
            segments: tagged(
                vec![lead(LeadRole::Entry), SegmentProcess::default(), lead(LeadRole::Exit)],
                vec![
                    Segment::Line { start: [start[0] - 2., start[1] + 1.], end: start },
                    Segment::Line { start, end },
                    Segment::Line { start: end, end: [end[0] + 2., end[1] + 1.] },
                ],
            ),
            overrides: Overrides::default(),
        }
    }

    fn settings() -> Settings {
        crate::cut::tests::settings()
    }

    /// The fiber process hardware the gas and head tests run with.
    fn hardware() -> Hardware {
        Hardware {
            analog_motion: None,
            gas_enabled: true,
            gas_ports: [Some(11), Some(12), Some(13), Some(14), Some(15), Some(16)],
            ratio_ports: [Some(20), Some(21), Some(22)],
            ratio_channels: [Some(1), Some(2), Some(2)],
            gas_max_pressure: [10., 11., 12.],
            gas_calibration: [None, None, None],
            gate: None,
            laser: Some(3),
            peak_channel: None,
            peak_factor: 100.,
            keep_peak_output: false,
            secondary_pwm: false,
            co2_contour_control: false,
            z_enabled: true,
            direct_drill_max: 5.,
            gas_delay_ms: 20,
            first_gas_delay_ms: 30,
            change_gas_delay_ms: 40,
            crash_height: None,
            crash_resume_value: 0,
        }
    }

    /// One ordinary pierce stage at `index`.
    fn stage(index: u8) -> PierceStage {
        PierceStage {
            index,
            height: 3.,
            power: 30,
            frequency: 1000,
            peak_current: 80.,
            gas: 1,
            pressure: 2.,
            duration_ms: 50,
            gradual: false,
            gradual_ms: 0,
            before_off_ms: 0,
            after_off_ms: 0,
            bolt: None,
        }
    }

    /// A one-stage fiber recipe with process hardware.
    fn piercing() -> Settings {
        let mut s = settings();
        s.hardware = Some(hardware());
        s.machining_kind = 2;
        s.pierce = vec![stage(0)];
        s
    }

    fn bound() -> Binding {
        Binding { current: [0., 0.], z_units_per_mm: Some(1000) }
    }

    /// Cleaning uses the actual first pierce position even when there is
    /// no approach travel, and follows the execution position at repierces.
    #[test]
    fn cleaning_is_anchored_at_initial_and_joint_pierces() {
        let mut s = piercing();
        s.residue = Some(crate::settings::Residue {
            height: 2.,
            speed: 10.,
            gas: 1,
            pressure: 2.,
            peak_current: 80.,
            power: 20,
            frequency: 1000,
            radius: 0.2,
            turns: 1,
        });
        let start = [70., 80.];
        let mut contour = line(start, [75., 80.]);
        contour.segments.extend(tagged(
            [
                SegmentProcess { joint: true, power: Some(0.), ..SegmentProcess::default() },
                SegmentProcess { repierce: true, ..SegmentProcess::default() },
            ]
            .into(),
            [
                Segment::Line { start: [75., 80.], end: [77., 80.] },
                Segment::Line { start: [77., 80.], end: [80., 80.] },
            ]
            .into(),
        ));
        let job = Job::compile(&s, &[contour]).unwrap();
        let program = job.program(Binding { current: start, ..bound() }).unwrap();
        let cleaning: Vec<_> = program
            .sections
            .iter()
            .enumerate()
            .filter(|(_, section)| section.kind == Kind::ResidueSpiral)
            .collect();
        assert_eq!(cleaning.len(), 2);
        assert_eq!(cleaning[0].1.points[0], start);
        assert!(
            !program.sections[..cleaning[0].0].iter().any(|section| section.kind == Kind::Travel)
        );
        let previous = program.sections[..cleaning[1].0]
            .iter()
            .rev()
            .find(|section| matches!(section.kind, Kind::Cut | Kind::Joint))
            .unwrap();
        assert_eq!(cleaning[1].1.points.first(), previous.points.last());
        for (_, section) in cleaning {
            assert!(section.points.iter().all(|point| point[0] > 69.7 && point[1] > 79.7));
        }
    }

    /// A contour on a layer with its own recipe runs under that recipe and
    /// pierces with it; contours on the job's recipe plan as they always
    /// did, so a job of one recipe is unchanged.
    #[test]
    fn layered_contours_run_under_their_own_recipe() {
        let s = piercing();
        let contours = [line([10., 10.], [20., 10.]), line([30., 10.], [40., 10.])];
        let plain = Job::schedule_prepared(&s, &contours, None).unwrap();
        assert_eq!(Job::schedule_layered(&s, &contours, None, &[&s, &s]).unwrap(), plain);
        let mut engrave = piercing();
        engrave.peak_current = 9.;
        engrave.pierce.clear();
        let layered = Job::schedule_layered(&s, &contours, None, &[&engrave, &s]).unwrap();
        assert_eq!(layered.passes[0].settings, engrave);
        assert!(layered.passes[0].initial_pierce);
        assert_eq!(layered.passes[1], plain.passes[1]);
        assert_eq!(layered.settings, s);
        assert!(Job::schedule_layered(&s, &contours, None, &[&s]).is_err());
    }

    /// Padding samples and PWM records have distinct counts. The complete
    /// bolt drill is refused before either output vector is allocated.
    #[test]
    fn motion_and_bolt_expansion_share_the_program_budget() {
        let mut s = piercing();
        s.interpolation_cycle_us = 500;
        let mut stage = stage(0);
        stage.duration_ms = u32::try_from(MAX_SAMPLES).unwrap();
        let cut_start = CutStart {
            settings: &s,
            hardware: s.hardware.as_ref(),
            z: true,
            z_scale: 1000,
            index: 0,
            output: Output::Pulsed,
            on_delay_ms: 0,
            position: [0.; 2],
        };
        let mut program = Builder::default();
        assert!(matches!(
            cut_start.bolt_drill(&mut program, &stage, (50, 1000), true),
            Err(Error::Budget(_))
        ));
        assert!(program.records.is_empty() && program.samples == 0);
        program.samples = 8_000_000;
        assert!(program.allow(1, 1).is_ok(), "long jobs can pass the former whole-job cap");
        program.samples = MAX_SAMPLES - 2;
        assert_eq!(program.motion_points(), 3);
        assert!(
            travel_movement(
                [[0.; 2], [10., 0.]],
                &s,
                [0.; 2],
                0.,
                Output::Pulsed,
                program.motion_points()
            )
            .is_err()
        );
        assert!(program.records.is_empty());
    }

    fn records<'a>(program: &'a Program, section: &Section) -> &'a [Record] {
        &program.records[section.records.clone()]
    }

    /// Whether any record switches a gas output or pressure channel off.
    fn closes_gas(records: &[Record]) -> bool {
        let ports = [11, 12, 13, 14, 15, 16, 20, 21, 22];
        records.iter().any(|r| {
            ports.iter().any(|p| Record::output(*p, false).ok().as_ref() == Some(r))
                || matches!(r, Record::Analog { value: 0, .. })
        })
    }

    fn section(program: &Program, pass: usize, kind: Kind) -> &Section {
        program.sections.iter().find(|s| s.pass == Some(pass) && s.kind == kind).unwrap()
    }

    /// A dry-run job with two lines opens with the checkpoint and start
    /// sections, travels to the second line, tags each cut, and closes with
    /// the finished checkpoint, the end commands and the final barrier.
    #[test]
    fn dry_run_program_has_the_vendor_layout() {
        let mut dry = settings();
        dry.dry_run = true;
        let job =
            Job::compile(&dry, &[line([0., 0.], [5., 0.]), line([20., 0.], [25., 0.])]).unwrap();
        let program = job.program(Binding { current: [0., 0.], z_units_per_mm: None }).unwrap();
        let kinds: Vec<Kind> = program.sections.iter().map(|s| s.kind).collect();
        assert_eq!(kinds[0], Kind::CheckpointStart);
        assert_eq!(kinds[1], Kind::Start);
        assert!(kinds.contains(&Kind::Travel));
        assert_eq!(kinds.iter().filter(|k| **k == Kind::CheckpointCut).count(), 2);
        assert_eq!(
            kinds[kinds.len() - 3..],
            [Kind::CheckpointFinished, Kind::End, Kind::EndBarrier]
        );
        assert_eq!(program.records[0], Record::Item(u32::MAX));
        assert_eq!(program.records[program.records.len() - 1], Record::Barrier);
        let items: Vec<u32> = program
            .records
            .iter()
            .filter_map(|r| if let Record::Item(tag) = r { Some(*tag) } else { None })
            .collect();
        assert_eq!(items, vec![u32::MAX, 0x8000_0000, 0, 0x8000_0001, 1, u32::MAX - 1]);
        assert!(program.samples > 0);
        assert!(program.seconds > 0.);
    }

    /// Without hardware the cut end still zeroes PWM after the off delay,
    /// and the sections cover every record exactly once.
    #[test]
    fn sections_partition_the_records() {
        let job = Job::compile(&settings(), &[line([0., 0.], [5., 0.])]).unwrap();
        let program = job.program(Binding { current: [0., 0.], z_units_per_mm: None }).unwrap();
        let mut at = 0;
        for section in &program.sections {
            assert_eq!(section.records.start, at);
            at = section.records.end;
        }
        assert_eq!(at, program.records.len());
        let cut_end = program.sections.iter().find(|s| s.kind == Kind::CutEnd).unwrap();
        let records = &program.records[cut_end.records.clone()];
        assert_eq!(records[0], Record::Barrier);
        assert!(records.contains(&Record::Pwm {
            channel: LaserChannel::Primary,
            frequency: 2000,
            power: 0
        }));
    }

    /// A gradual final stage hands over to the cut height with a pierce
    /// ramp; an earlier stage lifts relatively.
    #[test]
    fn gradual_pierce_selects_ramp_or_lift() {
        let ramp = gradual_pierce(3., 1., 500, true).unwrap();
        assert_eq!(
            ramp,
            Record::PierceRamp {
                speed_tenths: 40,
                descent_microns: 2000,
                cut_height_microns: 1000
            }
        );
        let lift = gradual_pierce(3., 2., 500, false).unwrap();
        assert_eq!(lift, Record::Lift { speed_tenths: 20, delta_microns: 1000 });
        assert!(gradual_pierce(3., 1., 0, true).is_err());
    }

    /// Pre-piercing in batches of two and a film pass on every contour
    /// schedule points, films and cuts in the vendor's order with
    /// contiguous ordinals; the cuts do not pierce again, the points sit at
    /// the lead-in start with no motion, and the film passes trace the bare
    /// geometry under the film recipe, which keeps its own power.
    #[test]
    fn passes_are_scheduled_with_their_own_processes() {
        use PassKind::{Cut, Film as F, PrePierce as P};
        let mut s = piercing();
        s.power = 77;
        s.pre_pierce = Some(PrePierce { batch: 2, repeat_before_cut: false, keep_down: false });
        s.with_film = true;
        s.film_batch = 0;
        let mut film_settings = settings();
        film_settings.power = 12;
        let contours =
            vec![led([0., 0.], [5., 0.]), led([20., 0.], [25., 0.]), led([40., 0.], [45., 0.])];
        let runs: Vec<Vec<Contour>> = contours
            .iter()
            .map(|c| vec![Contour { segments: vec![c.segments[1].clone()], ..c.clone() }])
            .map(|mut run| {
                run[0].segments[0].process = SegmentProcess::default();
                run
            })
            .collect();
        let film = Film { settings: &film_settings, runs: &runs };
        let job = Job::schedule(&s, &contours, Some(film)).unwrap();
        let kinds: Vec<(usize, PassKind)> =
            job.passes.iter().map(|c| (c.pass.source, c.pass.kind)).collect();
        assert_eq!(
            kinds,
            vec![(0, P), (1, P), (0, F), (1, F), (2, F), (0, Cut), (1, Cut), (2, P), (2, Cut)]
        );
        assert!(job.passes.iter().enumerate().all(|(i, c)| c.pass.ordinal == i));
        let point = &job.passes[0];
        assert!(point.pieces.is_empty());
        assert_eq!(point.start, [-2., 1.]);
        assert_eq!(point.end, point.start);
        let film_pass = &job.passes[2];
        assert_eq!(film_pass.settings.power, 12);
        assert_eq!(film_pass.contour.segments.len(), 1);
        assert_eq!(film_pass.start, [0., 0.]);
        let cut = &job.passes[5];
        assert_eq!(cut.settings.power, 77);
        assert!(!cut.initial_pierce);
        assert_eq!(cut.contour.segments.len(), 3);
        assert!(Job::schedule(&s, &contours, None).is_err(), "the film process is missing");
        s.pre_pierce = Some(PrePierce { batch: 1, repeat_before_cut: true, keep_down: false });
        s.with_film = false;
        let job = Job::schedule(&s, &contours, None).unwrap();
        assert!(job.passes.iter().filter(|c| c.pass.kind == Cut).all(|c| c.initial_pierce));
    }

    /// A pre-pierce point emits its piercing stage at the prepared start
    /// with no cut start, cut motion or cut end, tags itself when done, and
    /// the following cut skips its piercing. Between the points of one
    /// batch the head stays down when the recipe keeps it, and the batch's
    /// last point retracts before the cut.
    #[test]
    fn a_point_pierces_without_cutting_and_keeps_the_head_down() {
        let mut s = piercing();
        s.pre_pierce = Some(PrePierce { batch: 3, repeat_before_cut: false, keep_down: true });
        let contours =
            vec![led([0., 0.], [5., 0.]), led([20., 0.], [25., 0.]), led([40., 0.], [45., 0.])];
        let job = Job::compile(&s, &contours).unwrap();
        let program = job.program(bound()).unwrap();
        let kinds = |pass: usize| -> Vec<Kind> {
            program.sections.iter().filter(|s| s.pass == Some(pass)).map(|s| s.kind).collect()
        };
        assert_eq!(
            kinds(0),
            vec![Kind::CheckpointTravel, Kind::Travel, Kind::Pierce(0), Kind::CheckpointPoint]
        );
        assert!(!kinds(1).contains(&Kind::RetractBeforeTravel), "kept down between points");
        assert!(!kinds(2).contains(&Kind::RetractBeforeTravel));
        assert!(kinds(3).contains(&Kind::RetractBeforeTravel), "the batch's last point retracts");
        assert_eq!(
            kinds(3).iter().filter(|k| matches!(k, Kind::Pierce(_))).count(),
            0,
            "the cut does not pierce again"
        );
        assert!(kinds(3).contains(&Kind::CutStart) && kinds(3).contains(&Kind::CutEnd));
        let travel = section(&program, 0, Kind::Travel);
        assert_eq!(travel.points.last().copied(), Some([-2., 1.]));
        let items: Vec<u32> = program
            .records
            .iter()
            .filter_map(|r| if let Record::Item(tag) = r { Some(*tag) } else { None })
            .collect();
        assert_eq!(&items[1..5], [0x8000_0000, 0, 0x8000_0001, 1]);
    }

    /// The wait after a gas selection follows the vendor's arithmetic: the
    /// first gas adds its allowance, a gas that was off adds the base
    /// delay, a changed selector adds the change allowance, and a retained
    /// unchanged gas waits for nothing.
    #[test]
    fn gas_delays_follow_the_vendor_arithmetic() {
        let h = hardware();
        assert_eq!(gas_delay(&h, true, true, false, 0), 50);
        assert_eq!(gas_delay(&h, false, false, false, 0), 0);
        assert_eq!(gas_delay(&h, false, false, true, 5), 35);
        assert_eq!(gas_delay(&h, false, true, true, 5), 55);
        assert_eq!(gas_delay(&h, false, false, true, 100), 0);
    }

    /// Gas kept over a short transfer stays open: the cut end closes only
    /// the laser, the next cut start neither clears nor waits for the gas,
    /// and the program end still closes everything. At the threshold the
    /// gas closes, and the last cut always closes it.
    #[test]
    fn gas_stays_open_across_a_qualifying_transfer() {
        let mut s = piercing();
        s.pierce.clear();
        s.machining_kind = 0;
        s.short_gas_keep = true;
        s.head_travel.short_transfer = Some(10.);
        let near = vec![line([0., 0.], [5., 0.]), line([14.99, 0.], [20., 0.])];
        let program = Job::compile(&s, &near).unwrap().program(bound()).unwrap();
        let end = records(&program, section(&program, 0, Kind::CutEnd));
        assert!(!closes_gas(end), "gas kept over 9.99 mm");
        assert!(end.contains(&Record::output(3, false).unwrap()), "laser off between cuts");
        let start = records(&program, section(&program, 1, Kind::CutStart));
        assert!(!closes_gas(start));
        let delays: Vec<u32> = start
            .iter()
            .filter_map(|r| if let Record::Delay { millis } = r { Some(*millis) } else { None })
            .collect();
        assert_eq!(delays, vec![9], "the on-delay alone: no reopen delay");
        assert!(closes_gas(records(&program, section(&program, 1, Kind::CutEnd))), "last cut");
        assert!(closes_gas(records(
            &program,
            program.sections.iter().find(|s| s.kind == Kind::End).unwrap()
        )));
        let at = vec![line([0., 0.], [5., 0.]), line([15., 0.], [20., 0.])];
        let program = Job::compile(&s, &at).unwrap().program(bound()).unwrap();
        assert!(closes_gas(records(&program, section(&program, 0, Kind::CutEnd))), "strict");
        let start = records(&program, section(&program, 1, Kind::CutStart));
        assert!(start.contains(&Record::Delay { millis: 20 }), "the base delay after gas off");
    }

    /// Gas kept during processing changes route without closing at the cut
    /// end: the next selection closes the old route first, opens the new
    /// one, and waits the change allowance alone.
    #[test]
    fn a_kept_gas_change_closes_the_old_route_first() {
        let mut s = piercing();
        s.pierce.clear();
        s.machining_kind = 0;
        s.keep_gas_after = true;
        let job =
            Job::compile(&s, &[line([0., 0.], [5., 0.]), line([50., 0.], [55., 0.])]).unwrap();
        let mut job = job;
        job.passes[1].settings.gas = 3;
        let program = job.program(bound()).unwrap();
        assert!(!closes_gas(records(&program, section(&program, 0, Kind::CutEnd))));
        let start = records(&program, section(&program, 1, Kind::CutStart));
        let off = start.iter().position(|r| *r == Record::output(12, false).unwrap()).unwrap();
        let on = start.iter().position(|r| *r == Record::output(14, true).unwrap()).unwrap();
        assert!(off < on, "the old route closes before the new one opens");
        assert!(start.contains(&Record::Delay { millis: 40 }));
        assert!(!start.contains(&Record::Delay { millis: 60 }));
    }

    /// The first selection of a program waits for the first-gas allowance
    /// and the base delay; a pierce stage closes its gas, so the cut start
    /// after it waits the base delay again.
    #[test]
    fn stages_close_their_gas_and_the_cut_reopens_it() {
        let s = piercing();
        let program =
            Job::compile(&s, &[line([0., 0.], [5., 0.])]).unwrap().program(bound()).unwrap();
        let pierce = records(&program, section(&program, 0, Kind::Pierce(0)));
        assert!(pierce.contains(&Record::Delay { millis: 50 }), "first gas: 30 + 20");
        assert!(closes_gas(pierce));
        let start = records(&program, section(&program, 0, Kind::CutStart));
        assert!(start.contains(&Record::Delay { millis: 20 }));
    }

    /// A continuation from half way along a cut trims that cut, pierces
    /// again only when told, keeps the passes after it with their original
    /// ordinals, and never replays a finished preliminary point; a point
    /// resumes only whole, and nothing remains after the last pass.
    #[test]
    fn a_continuation_keeps_pass_identity() {
        let mut s = piercing();
        s.pre_pierce = Some(PrePierce { batch: 2, repeat_before_cut: false, keep_down: false });
        let contours = vec![line([0., 0.], [10., 0.]), line([20., 0.], [30., 0.])];
        let job = Job::compile(&s, &contours).unwrap();
        let kinds: Vec<PassKind> = job.passes.iter().map(|c| c.pass.kind).collect();
        assert_eq!(kinds, [PassKind::PrePierce, PassKind::PrePierce, PassKind::Cut, PassKind::Cut]);
        let rest = job.continuation(2, 0.5, true).unwrap();
        assert_eq!(rest.passes.len(), 2);
        assert_eq!(rest.passes[0].pass.ordinal, 2);
        assert!((rest.passes[0].start[0] - 5.).abs() < 1e-7);
        assert_eq!(rest.passes[0].start[1], 0.);
        assert!(rest.passes[0].initial_pierce, "pierces again on resume");
        assert!(!rest.passes[1].initial_pierce, "the untouched cut was pre-pierced");
        assert_eq!(rest.passes[1].pass.ordinal, 3);
        assert!(rest.passes.iter().all(|c| c.pass.kind == PassKind::Cut), "no replayed points");
        assert!(!job.continuation(2, 0.5, false).unwrap().passes[0].initial_pierce);
        assert!(job.continuation(0, 0.5, true).is_err(), "half a point");
        assert_eq!(job.continuation(0, 1., true).unwrap().passes.len(), 3);
        assert_eq!(job.continuation(1, 0., true).unwrap().passes.len(), 3);
        assert!(job.continuation(3, 1., true).is_err(), "finished");
        let program = rest.program(bound()).unwrap();
        let items: Vec<u32> = program
            .records
            .iter()
            .filter_map(|r| if let Record::Item(tag) = r { Some(*tag) } else { None })
            .collect();
        assert_eq!(items, vec![u32::MAX, 0x8000_0000, 0, 0x8000_0001, 1, u32::MAX - 1]);
    }

    #[test]
    fn resume_returns_cold_to_the_exact_saved_coordinate_and_keeps_the_remainder() {
        let job = Job::compile(
            &settings(),
            &[line([70., 80.], [110., 80.]), line([120., 80.], [130., 80.])],
        )
        .unwrap();
        let saved = [80., 80.001];
        let rest = job.continuation_at(0, 0.25, false, saved).unwrap();
        assert_eq!(rest.passes[0].start, saved);
        for axis in 0..2 {
            assert!((rest.passes[0].end[axis] - job.passes[0].end[axis]).abs() < 1e-10);
        }
        assert_eq!(rest.passes[1], job.passes[1]);
        assert_eq!(job.passes[0].start, [70., 80.], "original geometry is immutable");
        let parked = [20., 30.];
        let program =
            rest.program_with_return(Binding { current: parked, ..bound() }, Some(saved)).unwrap();
        let travel = section(&program, 0, Kind::Travel);
        assert_eq!(travel.points.first().copied(), Some(parked));
        assert_eq!(travel.points.last().copied(), Some(saved));
        let cut = section(&program, 0, Kind::Cut);
        assert_eq!(cut.points.first().copied(), Some(saved));
        assert!(travel.records.end <= cut.records.start);
        for record in &program.records[..travel.records.end] {
            match record {
                Record::Move { laser, .. } => assert_eq!(Record::pulsed_fields(*laser).power, 0),
                Record::Pwm { power, .. } => assert_eq!(*power, 0),
                Record::Outputs { values, .. } => assert_eq!(*values, 0),
                _ => (),
            }
        }
        let pulses = records(&program, travel).iter().fold([0_i32; 2], |mut p, record| {
            if let Record::Move { dx, dy, .. } = record {
                p[0] += i32::from(*dx);
                p[1] += i32::from(*dy);
            }
            p
        });
        for axis in 0..2 {
            assert!(
                (parked[axis] + f64::from(pulses[axis]) / job.settings.counts_per_mm[axis]
                    - saved[axis])
                    .abs()
                    <= 0.5 / job.settings.counts_per_mm[axis]
            );
        }
        assert!(
            rest.passes[0]
                .pieces
                .iter()
                .flat_map(|p| p.plan.points.iter())
                .all(|p| p[0] >= saved[0])
        );
        assert!(job.continuation_at(0, 0.25, false, [80., 80.3]).is_err());
        assert!(job.continuation_at(0, 1., false, saved).is_err());
    }

    #[test]
    fn a_pause_during_travel_returns_to_that_waypoint_before_the_cut_start() {
        let job = Job::compile(&settings(), &[line([70., 80.], [110., 80.])]).unwrap();
        let parked = [20., 30.];
        let saved = [50., 60.];
        let program =
            job.program_with_return(Binding { current: parked, ..bound() }, Some(saved)).unwrap();
        let travel: Vec<_> = program.sections.iter().filter(|s| s.kind == Kind::Travel).collect();
        assert_eq!(travel.len(), 2);
        assert_eq!(travel[0].points.last().copied(), Some(saved));
        assert_eq!(travel[1].points.first().copied(), Some(saved));
        assert_eq!(travel[1].points.last().copied(), Some(job.passes[0].start));
        for segment in travel {
            for record in records(&program, segment) {
                if let Record::Move { laser, .. } = record {
                    assert_eq!(Record::pulsed_fields(*laser).power, 0);
                }
            }
        }
    }

    #[test]
    fn repeated_continuations_keep_the_consumed_distance_and_later_passes() {
        let job = Job::compile(
            &settings(),
            &[line([70., 80.], [110., 80.]), line([120., 80.], [130., 80.])],
        )
        .unwrap();
        let first = job.continuation(0, 0.25, false).unwrap();
        let second = first.continuation(0, 0.5, false).unwrap();
        assert!((first.passes[0].start[0] - 80.).abs() < 1e-7);
        assert!((second.passes[0].start[0] - 95.).abs() < 1e-7);
        assert!((second.passes[0].consumed - 25.).abs() < 1e-7);
        assert_eq!(second.passes[1], job.passes[1], "untouched pass retains its entire plan");
        for rest in [&first, &second] {
            let pass = &rest.passes[0];
            let points = &pass.pieces[0].plan.points;
            assert!(points.windows(2).all(|p| p[1][0] >= p[0][0]));
            assert_eq!(points[0], points[1], "restart uses the native initial zero step");
            assert!((pass.end[0] - 110.).abs() < 1e-7);
        }
    }

    /// Previewing and resuming a long job must not duplicate every sample;
    /// retiming the interrupted pass must still leave the original untouched.
    #[test]
    fn preview_and_continuation_share_immutable_samples() {
        let job =
            Job::compile(&settings(), &[line([0., 0.], [40., 0.]), line([50., 0.], [60., 0.])])
                .unwrap();
        let program = job.program(bound()).unwrap();
        let original = &job.passes[0].pieces[0];
        assert!(Arc::ptr_eq(&section(&program, 0, Kind::Cut).points, &original.plan.points));
        let rest = job.continuation(0, 0.25, false).unwrap();
        assert_eq!(original.plan.points[0], [0.; 2]);
        assert!((rest.passes[0].start[0] - 10.).abs() < 1e-7);
        assert!(!Arc::ptr_eq(&rest.passes[0].pieces[0].plan.points, &original.plan.points));
        let later = &job.passes[1].pieces[0];
        let retained = &rest.passes[1].pieces[0];
        assert!(Arc::ptr_eq(&retained.plan.points, &later.plan.points));
        assert!(Arc::ptr_eq(&retained.plan.distances, &later.plan.distances));
        assert!(Arc::ptr_eq(&retained.samples, &later.samples));
    }

    #[test]
    fn continuation_preserves_arcs_rounded_corners_and_later_process_events() {
        let curves = vec![
            Segment::Line { start: [70., 80.], end: [90., 80.] },
            Segment::Line { start: [90., 80.], end: [90., 100.] },
            Segment::Arc {
                center: [100., 100.],
                radius: 10.,
                start_angle: std::f64::consts::PI,
                sweep: -std::f64::consts::FRAC_PI_2,
            },
            Segment::Line { start: [100., 110.], end: [120., 110.] },
        ];
        let processes = vec![
            SegmentProcess::default(),
            SegmentProcess::default(),
            SegmentProcess {
                joint: true,
                power: Some(20.),
                speed: Some(10.),
                ..SegmentProcess::default()
            },
            SegmentProcess { repierce: true, cool_ms: 100, ..SegmentProcess::default() },
        ];
        let contour =
            Contour { segments: tagged(processes, curves), overrides: Overrides::default() };
        let job = Job::compile(&piercing(), &[contour]).unwrap();
        for fraction in [0.1, 0.265, 0.6, 0.8] {
            let rest = job.continuation(0, fraction, true).unwrap();
            assert!(rest.passes[0].consumed > 0.);
            assert!(rest.passes[0].start.iter().all(|v| v.is_finite()));
            assert!((rest.passes[0].end[0] - job.passes[0].end[0]).abs() < 1e-7);
            let program = rest
                .program(Binding { current: rest.passes[0].start, z_units_per_mm: Some(1000) })
                .unwrap();
            assert!(program.samples > 0);
            if fraction < 0.6 {
                assert!(rest.passes[0].pieces.iter().any(|piece| piece.process.cool_ms == 100));
                assert!(rest.passes[0].events.iter().flatten().any(|event| event.repierce));
            }
        }
    }

    #[test]
    fn endpoint_cooling_omissions_are_counted_without_inventing_events() {
        let points = [0., 0.1, 0.2, 9.8, 9.9, 10.];
        let segments = points
            .windows(2)
            .map(|p| {
                openlaser_core::toolpath::PreparedSegment::new(
                    Segment::Line { start: [p[0], 0.], end: [p[1], 0.] },
                    SegmentProcess { cool_ms: 100, ..SegmentProcess::default() },
                )
            })
            .collect();
        let job =
            Job::compile(&settings(), &[Contour { segments, overrides: Overrides::default() }])
                .unwrap();
        let pass = &job.passes[0];
        let retained = pass.pieces.iter().filter(|piece| piece.process.cool_ms != 0).count();
        assert_eq!(pass.omitted_cooling, 3);
        assert_eq!(retained, 2);
    }
}
