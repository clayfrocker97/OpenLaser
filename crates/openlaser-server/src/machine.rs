// SPDX-License-Identifier: GPL-3.0-or-later

//! The machine commands: admission under the lock, the long part in a task
//! of its own, the outcome as a note.

use crate::coordinator::{Held, Shared};
use crate::draft;
use crate::preflight::{PreflightConfirmation, PreflightIntent};
use crate::{Error, Result};
use openlaser_controller::lease::Lease;
use openlaser_controller::operations::motion::Request;
use openlaser_controller::operations::outputs::Plan;
use openlaser_controller::session::Configuration;
use openlaser_controller::streaming::{Ending, Program};
use openlaser_core::LaserMode;
use openlaser_xml::bindings::Manual;
use serde::Deserialize;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Once the socket is open: binds the machine files under the controller's
/// scale and reads the axis parameters back. The problem to report when
/// either fails; the connection stands regardless.
pub(crate) async fn after_connect(shared: &Shared) -> Option<String> {
    let mode = {
        let c = shared.lock().await;
        c.draft
            .as_ref()
            .and_then(|d| d.current.recipe.as_ref())
            .map(|r| r.laser)
            .or(c.mode)
            .unwrap_or(LaserMode::Fiber)
    };
    match configure(shared, mode, false).await {
        Ok(problem) => {
            refresh(shared).await;
            problem
        }
        Err(error) => {
            shared.lock().await.bindings_error = Some(error.to_string());
            Some(format!("the machine files did not bind: {error}"))
        }
    }
}

/// One reserved configuration transition, from immutable XML through
/// controller acknowledgement and fresh parameter verification.
async fn configure(shared: &Shared, mode: LaserMode, switch: bool) -> Result<Option<String>> {
    let epoch = shared.ensure_running()?;
    let requested = Instant::now();
    let (owner, bundle, scale) = {
        let mut c = shared.lock().await;
        let bundle = c.bundle.clone().ok_or_else(|| Error::Refused("no machine files".into()))?;
        let scale = c.scale()?;
        (c.reserve()?, bundle, scale)
    };
    let outcome = async {
        let bound =
            work(move || openlaser_xml::bindings::bind(&bundle, mode, scale).map_err(Error::from))
                .await?;
        if shared.ensure_running()? != epoch {
            return Err(Error::Refused("configuration was cancelled".into()));
        }
        shared.machine.configure(crate::bindings::controller(&bound)).await?;
        shared.lock().await.install_bindings(Some(bound));
        if switch {
            shared.machine.switch_mode_at(requested).await?;
        }
        let problem = apply_parameters(shared).await;
        // Connecting can invalidate an in-flight restored preview's revision.
        // Finish any still-pending geometry under the newly installed bindings.
        prepare(shared).await?;
        Ok(problem)
    }
    .await;
    let mut c = shared.lock().await;
    c.release_reservation(owner);
    outcome
}

/// Reads the axis parameters back and records whether they match the
/// machine files; the reason when they do not, or cannot be read.
pub(crate) async fn verify_parameters(
    shared: &Shared,
    machine: &openlaser_controller::Machine,
) -> Option<String> {
    let verified = machine.read_parameters().await;
    let mut coordinator = shared.lock().await;
    let problem = match verified {
        Ok(verified) => {
            let mismatches = coordinator.bundle.as_ref().map_or_else(Vec::new, |bundle| {
                parameter_mismatches(
                    bundle,
                    &verified,
                    coordinator.mode.unwrap_or(LaserMode::Fiber),
                )
            });
            coordinator.accepted = machine
                .state()
                .configuration
                .filter(|c| c.verified == verified && mismatches.is_empty());
            (!mismatches.is_empty()).then(|| {
                format!(
                    "controller settings differ from the machine files: {}",
                    mismatches.join(", ")
                )
            })
        }
        Err(error) => {
            coordinator.accepted = None;
            Some(format!("the parameters could not be read: {error}"))
        }
    };
    coordinator.parameter_problem.clone_from(&problem);
    problem
}

/// Imports one of the vendor's files; a connected machine is bound and
/// verified against the new files.
pub async fn import_file(shared: &Shared, file_name: &str, bytes: &[u8]) -> Result<()> {
    import_file_reviewed(shared, file_name, bytes, None).await
}

/// Imports only if the backup still matches the reviewed source hash.
pub async fn import_file_reviewed(
    shared: &Shared,
    file_name: &str,
    bytes: &[u8],
    expected: Option<&str>,
) -> Result<()> {
    let epoch = shared.ensure_running()?;
    let (owner, import) = reserve_import(shared, file_name, bytes, expected).await?;
    let outcome = run_import(shared, import, epoch).await;
    let mut c = shared.lock().await;
    c.release_reservation(owner);
    drop(c);
    if outcome.is_ok() {
        refresh(shared).await;
    }
    outcome
}

/// What an import needs, captured under the lock when it is reserved.
struct Import {
    file_name: String,
    bytes: Vec<u8>,
    dir: std::path::PathBuf,
    mode: Option<LaserMode>,
    scale: i32,
    connected: bool,
    /// The bindings to restore on the controller if the commit fails.
    previous: Option<openlaser_xml::bindings::Bindings>,
}

/// Reserves the machine workflow for an import, refusing one reviewed
/// against a backup that has since changed.
async fn reserve_import(
    shared: &Shared,
    file_name: &str,
    bytes: &[u8],
    expected: Option<&str>,
) -> Result<(u64, Import)> {
    let mut c = shared.lock().await;
    if expected
        .is_some_and(|hash| hash != c.files.backup.as_ref().map_or("", |f| f.sha256.as_str()))
    {
        return Err(Error::Refused("machine backup changed; review pending changes again".into()));
    }
    let dir = c.config.data_dir.join("machine");
    let connected = c.connected();
    let scale = if connected { c.scale()? } else { crate::bindings::OFFLINE_SCALE };
    let previous = c.bound.clone();
    let import = Import {
        file_name: file_name.to_owned(),
        bytes: bytes.to_vec(),
        dir,
        mode: c.mode,
        scale,
        connected,
        previous,
    };
    Ok((c.reserve()?, import))
}

/// Stages, binds, commits and installs the imported files, then verifies
/// the controller against them and prunes superseded backups.
async fn run_import(shared: &Shared, import: Import, epoch: u64) -> Result<()> {
    let Import { file_name, bytes, dir, mode, scale, connected, previous } = import;
    let name = file_name.clone();
    let candidate =
        work(move || crate::machine_files::stage(dir, &name, &bytes, mode, scale)).await?;
    if shared.ensure_running()? != epoch {
        return Err(Error::Refused("import was cancelled".into()));
    }
    if connected {
        shared.machine.configure(crate::bindings::controller(&candidate.bound)).await?;
    }
    let candidate = commit_import(shared, candidate, connected, previous.as_ref()).await?;
    let prune = candidate.pruning();
    {
        let mut c = shared.lock().await;
        c.install_files(candidate.loaded);
        c.install_bindings(connected.then_some(candidate.bound));
    }
    let problem = if connected { apply_parameters(shared).await } else { None };
    // The active reference and published state now agree. Pruning cannot
    // delete an unrelated file, and failure does not undo the import.
    tokio::task::spawn_blocking(prune)
        .await
        .map_err(|e| Error::Refused(format!("backup cleanup task: {e}")))?;
    let mut c = shared.lock().await;
    match problem {
        None => c.note(format!("Imported {file_name}"), false),
        Some(problem) => c.note(format!("Imported {file_name}; {problem}"), true),
    }
    Ok(())
}

/// Makes the staged files the active backup. If that fails after the
/// controller was configured from them, the `previous` bindings go back
/// on the controller; if even that fails, no bindings are kept.
async fn commit_import(
    shared: &Shared,
    candidate: crate::machine_files::Candidate,
    connected: bool,
    previous: Option<&openlaser_xml::bindings::Bindings>,
) -> Result<crate::machine_files::Candidate> {
    let committed = work(move || {
        candidate.commit()?;
        Ok(candidate)
    })
    .await;
    let error = match committed {
        Ok(candidate) => return Ok(candidate),
        Err(error) => error,
    };
    if connected
        && let Some(bound) = previous
        && let Err(rollback) = shared.machine.configure(crate::bindings::controller(bound)).await
    {
        let mut c = shared.lock().await;
        c.install_bindings(None);
        return Err(Error::Refused(format!(
            "{error}; restoring the previous bindings failed: {rollback}"
        )));
    }
    shared.lock().await.accepted = None;
    Err(error)
}

/// Fastest table jog, in millimetres per second, a request may ask for.
const MAX_TABLE_SPEED_MM_S: f64 = 100.;
/// Longest single XY jog step, in millimetres.
const MAX_JOG_STEP_MM: f64 = 1000.;
/// Shortest and longest gas test, in milliseconds.
const GAS_TEST_MS: std::ops::RangeInclusive<u64> = 50..=2000;
/// How long a timed gas test's valves stay open without a lease renewal.
/// Timed tests are not renewed from the UI; this is the controller's
/// safety net if the server stops talking to it mid-test.
const GAS_TEST_LEASE: Duration = Duration::from_secs(5);

/// The fields whose controller banks do not match the machine files.
fn parameter_mismatches(
    bundle: &openlaser_xml::Bundle,
    verified: &openlaser_controller::session::Verified,
    mode: LaserMode,
) -> Vec<String> {
    let Some(document) = bundle
        .document(openlaser_xml::Kind::Backup)
        .or_else(|| bundle.document(openlaser_xml::Kind::Hardware))
    else {
        return Vec::new();
    };
    crate::parameter_settings::mismatches(document, verified, mode)
        .unwrap_or_else(|error| vec![format!("cannot compare the backup parameters: {error}")])
}

async fn apply_parameters(shared: &Shared) -> Option<String> {
    match crate::parameter_settings::initialize(shared).await {
        Ok(()) => verify_parameters(shared, &shared.machine).await,
        Err(error) => {
            let problem = format!("machine initialization failed: {error}");
            let mut c = shared.lock().await;
            c.accepted = None;
            c.parameter_problem = Some(problem.clone());
            Some(problem)
        }
    }
}

/// Explicit settings read or write, serialized with the machine workflow.
pub async fn parameters(shared: &Shared, write: bool, expected: Option<&str>) -> Result<()> {
    let owner = {
        let mut c = shared.lock().await;
        if write && expected != c.files.backup.as_ref().map(|f| f.sha256.as_str()) {
            return Err(Error::Refused(
                "the backup changed; reload settings before writing".into(),
            ));
        }
        c.reserve()?
    };
    let problem = if write {
        apply_parameters(shared).await
    } else {
        verify_parameters(shared, &shared.machine).await
    };
    let mut c = shared.lock().await;
    if c.operation == Some(owner) {
        c.operation = None;
    }
    if let Some(problem) = problem {
        c.note(&problem, true);
        Err(Error::Refused(problem))
    } else {
        c.note(
            if write {
                "Machine settings written and verified"
            } else {
                "Controller settings read and matched"
            },
            false,
        );
        drop(c);
        refresh(shared).await;
        Ok(())
    }
}

/// Disconnects; whatever runs is stopped first.
pub async fn disconnect(shared: &Shared) -> Result<()> {
    let machine = shared.machine.clone();
    machine.disconnect().await?;
    let mut coordinator = shared.lock().await;
    coordinator.bound = None;
    coordinator.bindings_changed();
    coordinator.accepted = None;
    coordinator.parameter_problem = None;
    coordinator.held = None;
    coordinator.link.set(crate::document::LinkPhase::Idle, "");
    coordinator.note("Disconnected", false);
    Ok(())
}

/// Starts a long operation and notes its outcome when it ends.
fn spawn<T: Send + 'static>(
    shared: &Shared,
    label: &'static str,
    operation: impl std::future::Future<Output = std::result::Result<T, openlaser_controller::Error>>
    + Send
    + 'static,
) {
    let shared = Arc::clone(shared);
    tokio::spawn(async move {
        let outcome = operation.await;
        shared.lock().await.outcome(label, outcome);
    });
}

use std::sync::Arc;

/// Go Origin.
pub async fn home(shared: &Shared) -> Result<()> {
    let requested = Instant::now();
    let epoch = shared.ensure_running()?;
    select_setup_mode(shared).await?;
    // An alarm present at connection can prevent initialization. Once the
    // operator has recovered it, finish that setup before establishing Home.
    if shared.machine.state().configuration.is_none() {
        let expected = shared.lock().await.files.backup.as_ref().map(|file| file.sha256.clone());
        parameters(shared, true, expected.as_deref()).await?;
    }
    let revision = {
        let mut c = shared.lock().await;
        c.idle()?;
        if c.held.is_none() {
            if let Some(draft) = &mut c.draft
                && crate::placement::is_head(draft)
            {
                draft.current.sheet_offset = None;
                draft.capture_epoch = None;
                if draft.current.correction.is_some() {
                    draft.compiled = None;
                }
            }
            c.draft_changed();
        }
        c.draft.as_ref().map(|_| c.document().draft_revision)
    };
    if let Some(revision) = revision {
        compile_automatically(shared, revision).await?;
    }
    if shared.ensure_running()? != epoch {
        return Err(Error::Refused("Home was cancelled".into()));
    }
    let machine = shared.machine.clone();
    spawn(shared, "Home", async move { machine.home_at(false, requested).await });
    Ok(())
}

/// Head calibration.
pub async fn calibrate(shared: &Shared) -> Result<()> {
    shared.ensure_running()?;
    select_setup_mode(shared).await?;
    let context = {
        let mut c = shared.lock().await;
        c.idle()?;
        let context = crate::setup::Calibration::capture(&c);
        c.calibration = None;
        c.publish();
        context
    };
    let shared = shared.clone();
    tokio::spawn(async move {
        let context = match context {
            Ok(context) => context,
            Err(error) => {
                shared.lock().await.note(format!("Calibration failed: {error}"), true);
                return;
            }
        };
        let outcome = shared.machine.calibrate().await;
        let mut c = shared.lock().await;
        if outcome.is_ok() {
            c.calibration = Some(context);
        }
        c.outcome("Calibration", outcome);
    });
    Ok(())
}

/// Establish the job's source before the operator establishes machine setup.
async fn select_setup_mode(shared: &Shared) -> Result<()> {
    let revision = {
        let c = shared.lock().await;
        c.idle()?;
        c.draft.as_ref().map(|_| c.document().draft_revision)
    };
    if let Some(revision) = revision {
        select_draft_mode(shared, revision).await?;
    }
    Ok(())
}

/// A jog request from the UI.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct JogRequest {
    /// 0 for X, 1 for Y.
    pub axis: usize,
    /// The direction: positive or negative.
    pub positive: bool,
    /// The step in millimetres, or `None` for a held jog to the limit.
    pub step_mm: Option<f64>,
    /// Fast or slow.
    pub fast: bool,
    /// For a diagonal jog, the direction of the other axis. Both axes travel
    /// the same distance, so the head moves at 45° and stops at whichever
    /// limit comes first.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub diagonal: Option<bool>,
}

/// A held W/table jog at an operator-selected speed.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct TableRequest {
    /// Positive or negative W travel.
    pub positive: bool,
    /// A speed above zero and at most 100 mm/s.
    pub speed_mm_s: f64,
}

/// The configured lifting table, sharing the jog's lease and cleanup.
pub async fn table(shared: &Shared, request: TableRequest, lease: Lease) -> Result<()> {
    shared.ensure_running()?;
    let requested = Instant::now();
    let (motion, limits) = {
        let c = shared.lock().await;
        table_request(&c, request)?
    };
    let completed = shared.machine.start_table(motion, limits, Some(lease), requested).await?;
    spawn(shared, "Table jog", completed.finished());
    Ok(())
}

impl TableRequest {
    fn validate(&self) -> Result<()> {
        if !(self.speed_mm_s.is_finite()
            && self.speed_mm_s > 0.
            && self.speed_mm_s <= MAX_TABLE_SPEED_MM_S)
        {
            return Err(Error::Request("table speed must be above 0 and at most 100 mm/s".into()));
        }
        Ok(())
    }
}

fn table_request(c: &crate::Coordinator, request: TableRequest) -> Result<(Request, [i32; 2])> {
    c.idle()?;
    let verified = c.measured_configuration()?.verified;
    let bound = c.bound()?;
    let table = bound
        .table
        .ok_or_else(|| Error::Refused("the lifting-table branch is not configured".into()))?;
    request.validate()?;
    let feedback =
        c.machine.state().feedback.ok_or_else(|| Error::Refused("no table feedback".into()))?;
    if feedback.age_ms > crate::coordinator::FRESH_FEEDBACK_MS || !feedback.table_stationary {
        return Err(Error::Refused("wait for fresh stationary table feedback".into()));
    }
    let extent =
        table_extent(table.maximum_mm, &verified.banks[4], verified.scale, feedback.table_mm)?;
    let scale = verified.scale;
    let (travel, limits) =
        table_travel(request.positive, feedback.table_mm, extent, &verified.banks[4], scale)?;
    let speed = bound.jog.speed_word(request.speed_mm_s, scale)?;
    if speed > 1_000_000 {
        return Err(Error::Refused("the table speed cannot be represented".into()));
    }
    Ok((
        Request {
            delta: [travel, 0],
            speed,
            acceleration: table.acceleration,
            held: true,
            deceleration: bound.jog.deceleration,
        },
        limits,
    ))
}

fn table_extent(maximum_mm: f64, bank: &[u32], scale: i32, current: f64) -> Result<[f64; 2]> {
    let travel = if bank[0] & (1 << 5) == 0 { [0., maximum_mm] } else { [-maximum_mm, 0.] };
    let extent = within_bank(travel, bank, scale);
    if extent[0] >= extent[1] {
        return Err(Error::Refused(
            "the controller and machine file have no shared table travel range".into(),
        ));
    }
    if current < extent[0] || current > extent[1] {
        return Err(Error::Refused("the table is outside its configured travel".into()));
    }
    Ok(extent)
}

fn table_travel(
    positive: bool,
    current: f64,
    extent: [f64; 2],
    bank: &[u32],
    scale: i32,
) -> Result<(i32, [i32; 2])> {
    let delta = if positive { extent[1] - current } else { extent[0] - current };
    let travel = openlaser_xml::bindings::coordinate(delta, scale)?;
    let minimum = openlaser_xml::bindings::coordinate(extent[0], scale)?;
    let maximum = openlaser_xml::bindings::coordinate(extent[1], scale)?;
    let [distance, pulses] = [bank[9], bank[10]];
    let full_pulse = distance != 0
        && pulses != 0
        && u64::from(travel.unsigned_abs()) * u64::from(pulses) >= u64::from(distance);
    if !full_pulse || minimum < -100_000_000 || maximum > 100_000_000 {
        return Err(Error::Refused(
            "the table is at its limit or travel cannot be represented".into(),
        ));
    }
    Ok((travel, [minimum, maximum]))
}

/// The complete stationary pulse, including the output-off tail, fits one frame.
pub async fn pulse(shared: &Shared, duration_ms: u32, power: u8) -> Result<()> {
    let epoch = shared.ensure_running()?;
    let requested = Instant::now();
    let (owner, binding, pulse) = {
        let mut c = shared.lock().await;
        c.idle()?;
        if c.machine
            .state()
            .program
            .as_ref()
            .is_some_and(|p| p.state == openlaser_controller::state::ProgramState::Held)
        {
            return Err(Error::Refused("finish or stop the held job before a pulse".into()));
        }
        let configuration = c.acceptance()?;
        let binding = Execution::new(&c, &configuration, [0., 0.])?;
        let bundle = c
            .bundle
            .as_ref()
            .ok_or_else(|| Error::Refused("import machine settings first".into()))?;
        let pulse = openlaser_xml::bindings::pulse(bundle, configuration.mode, power)?;
        (c.reserve()?, binding, pulse)
    };
    let result = (|| {
        let program = openlaser_compiler::pulse::program(duration_ms, &pulse)?;
        let seconds = program.seconds;
        let upload = binding.program(&program, [1., 1.])?;
        if upload.blocks.len() != 1 {
            return Err(Error::Refused("the whole pulse must fit one controller frame".into()));
        }
        let execution = Arc::new(crate::document::ExecutionView {
            id: owner,
            frame: false,
            name: "Stationary laser pulse".into(),
            material: None,
            origin: binding.position,
            sheet_offset: [0., 0.],
            compiled: Arc::new(crate::document::Compiled {
                dry_run: false,
                seconds,
                plan: Vec::new(),
                pierces: vec![binding.position],
                blocks: 1,
                moves: Vec::new(),
                usage: crate::gas::Usage::default(),
                pass_usage: Arc::default(),
            }),
        });
        Ok(Built { program: upload, held: None, execution, fresh: false })
    })();
    start_run(shared, owner, epoch, requested, result, "Laser pulse").await
}

/// Jogs X or Y, or both at 45°: a step, or a held move until released.
pub async fn jog(shared: &Shared, request: JogRequest, lease: Option<Lease>) -> Result<()> {
    shared.ensure_running()?;
    let requested = Instant::now();
    if request.step_mm.is_none() && lease.is_none() {
        return Err(Error::Request("a held jog needs a press identity".into()));
    }
    let (machine, motion) = {
        let coordinator = shared.lock().await;
        coordinator.idle()?;
        let verified = coordinator.jog_parameters()?;
        let bound = coordinator.bound()?;
        let scale = verified.scale;
        let position = coordinator.position()?;
        if request.axis > 1 {
            return Err(Error::Request("the axis must be 0 or 1".into()));
        }
        let step = match request.step_mm {
            Some(step) if step.is_finite() && step > 0. && step <= MAX_JOG_STEP_MM => step,
            Some(_) => return Err(Error::Request("the step must be up to 1000 mm".into())),
            None => 1.,
        };
        let delta_mm = if request.positive { step } else { -step };
        let held = request.step_mm.is_none();
        let mut jog = bound.jog;
        jog.extent[request.axis] =
            within_bank(jog.extent[request.axis], &verified.banks[request.axis], scale);
        jog.soft_limit = true;
        let state = coordinator.machine.state();
        let mut delta = [0, 0];
        if let Some(other_positive) = request.diagonal {
            if state.xy_recovery {
                return Err(Error::Refused(
                    "limit recovery moves one axis at a time; jog straight off the limit".into(),
                ));
            }
            let other = 1 - request.axis;
            let other_mm = if other_positive { step } else { -step };
            let first = jog.travel(request.axis, position[request.axis], delta_mm, held, scale)?;
            let second = jog.travel(other, position[other], other_mm, held, scale)?;
            // Equal distances keep the move at 45°; the nearer limit ends it.
            let distance = first.unsigned_abs().min(second.unsigned_abs()).cast_signed();
            delta[request.axis] = distance * first.signum();
            delta[other] = distance * second.signum();
        } else {
            delta[request.axis] = if state.xy_recovery {
                if let Some(blocked) =
                    &state.xy_jog_blocked[request.axis][usize::from(request.positive)]
                {
                    return Err(Error::Refused(blocked.clone()));
                }
                openlaser_xml::bindings::coordinate(delta_mm.clamp(-1., 1.), scale)?
            } else {
                jog.travel(request.axis, position[request.axis], delta_mm, held, scale)?
            };
        }
        let fast = usize::from(request.fast);
        let motion = Request {
            delta,
            speed: jog.speed_word(jog.speed[fast], scale)?,
            acceleration: jog.acceleration[fast],
            held,
            deceleration: jog.deceleration,
        };
        (coordinator.machine.clone(), motion)
    };
    let completed = machine.start_travel(motion, lease, requested).await?;
    spawn(shared, "Jog", completed.finished());
    Ok(())
}

/// `extent` narrowed to the soft limits in an axis bank, words 1 and 2, so
/// a held jog never asks for a target the controller itself would refuse.
fn within_bank(extent: [f64; 2], bank: &[u32], scale: i32) -> [f64; 2] {
    let (Some(lower), Some(upper), true) = (bank.get(1), bank.get(2), scale > 0) else {
        return extent;
    };
    let mm = |word: u32| f64::from(word.cast_signed()) / f64::from(scale);
    [extent[0].max(mm(*lower)), extent[1].min(mm(*upper))]
}

/// Moves to the draft's job origin.
pub async fn go_origin(shared: &Shared, fast: bool) -> Result<()> {
    go_position(shared, Position::Origin, fast).await
}

/// Moves to a bed point through the same admitted, laser-off travel as Go origin.
pub async fn go_bed_point(shared: &Shared, point: openlaser_library::Anchor) -> Result<()> {
    go_position(shared, Position::Bed(point), false).await
}

/// Moves to machine XY coordinates through the normal admitted travel path.
pub async fn go_xy(shared: &Shared, target: [f64; 2]) -> Result<()> {
    go_position(shared, Position::Xy(target), false).await
}

pub(crate) async fn go_postflight(shared: &Shared, id: &str, step: usize) -> Result<()> {
    go_position(shared, Position::Postflight { id, step }, false).await
}

#[derive(Clone, Copy)]
enum Position<'a> {
    Origin,
    Bed(openlaser_library::Anchor),
    Xy([f64; 2]),
    Postflight { id: &'a str, step: usize },
}

async fn go_position(shared: &Shared, point: Position<'_>, fast: bool) -> Result<()> {
    shared.ensure_running()?;
    let requested = Instant::now();
    let completed = {
        let coordinator = shared.lock().await;
        let Some(motion) = positioning_request(&coordinator, point, fast)? else { return Ok(()) };
        // Keep reviewed state fixed until the controller owns this move.
        // The completion wait runs outside the coordinator lock.
        coordinator.machine.start_positioning(motion, requested).await?
    };
    spawn(
        shared,
        match point {
            Position::Origin => "Go to origin",
            _ => "Move to position",
        },
        completed.finished(),
    );
    Ok(())
}

fn positioning_request(
    coordinator: &crate::coordinator::Coordinator,
    point: Position<'_>,
    fast: bool,
) -> Result<Option<Request>> {
    coordinator.idle()?;
    coordinator.acceptance()?;
    let bound = coordinator.bound()?;
    let scale = coordinator.scale()?;
    let state = coordinator.machine.state();
    let feedback = state
        .feedback
        .filter(|f| {
            f.age_ms <= crate::coordinator::FRESH_FEEDBACK_MS && f.stationary && f.head.command == 0
        })
        .ok_or_else(|| Error::Refused("wait for fresh, stationary machine feedback".into()))?;
    if !state.session.homed || feedback.referenced != [true, true] {
        return Err(Error::Refused("home the machine first".into()));
    }
    let position = feedback.position_mm;
    let origin = position_target(coordinator, point)?;
    if !origin
        .iter()
        .zip(bound.jog.extent)
        .all(|(v, [min, max])| v.is_finite() && *v >= min && *v <= max)
    {
        return Err(Error::Request("the XY target is outside the bed".into()));
    }
    let delta = positioning_delta(origin, position, scale)?;
    if delta == [0, 0] {
        return Ok(None);
    }
    let fast = usize::from(fast);
    Ok(Some(Request {
        delta,
        speed: bound.jog.speed_word(bound.jog.speed[fast], scale)?,
        acceleration: bound.jog.position_acceleration[fast],
        held: false,
        deceleration: bound.jog.deceleration,
    }))
}

fn position_target(
    coordinator: &crate::coordinator::Coordinator,
    point: Position<'_>,
) -> Result<[f64; 2]> {
    match point {
        Position::Bed(point) => coordinator.bed_point(point),
        Position::Xy(target) => Ok(target),
        Position::Postflight { id, step } => coordinator.postflight_target(id, step),
        Position::Origin => coordinator
            .draft
            .as_ref()
            .and_then(crate::draft::Draft::origin)
            .ok_or_else(|| Error::Refused("nothing prepared".into())),
    }
}

fn positioning_delta(origin: [f64; 2], position: [f64; 3], scale: i32) -> Result<[i32; 2]> {
    let mut delta = [0; 2];
    for axis in 0..2 {
        let target = openlaser_xml::bindings::coordinate(origin[axis], scale)?;
        let current =
            openlaser_xml::bindings::coordinate((position[axis] * f64::from(scale)).round(), 1)?;
        delta[axis] = target
            .checked_sub(current)
            .ok_or_else(|| Error::Refused("the target is out of range".into()))?;
    }
    Ok(delta)
}

/// Ends a held jog or a held output.
pub async fn release(shared: &Shared, lease: Lease) -> Result<()> {
    shared.machine.release_owned(Some(lease)).await?;
    Ok(())
}

/// Renews a held jog's or output's lease.
pub fn heartbeat(shared: &Shared, lease: Lease) -> Result<()> {
    shared.ensure_running()?;
    shared.machine.heartbeat_owned(lease)?;
    Ok(())
}

/// A manual output request from the UI.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputRequest {
    /// The red pointer.
    Pointer,
    /// The shutter.
    Shutter,
    /// A gas at a pressure.
    Gas {
        /// The selector, 0 to 5.
        selector: u8,
        /// The pressure in bar.
        pressure: f64,
    },
    /// The head jogged while held.
    HeadJog {
        /// Up or down.
        up: bool,
        /// Fast or slow.
        fast: bool,
    },
}

/// Holds a manual output on until released.
pub async fn outputs(shared: &Shared, request: OutputRequest, lease: Lease) -> Result<()> {
    shared.ensure_running()?;
    let requested = Instant::now();
    let (machine, plan) = {
        let coordinator = shared.lock().await;
        coordinator.idle()?;
        let outputs = &coordinator.bound()?.outputs;
        let (name, manual): (&str, Manual) = match request {
            OutputRequest::Pointer => ("pointer", outputs.pointer()?),
            OutputRequest::Shutter => ("shutter", outputs.shutter()?),
            OutputRequest::Gas { selector, pressure } => ("gas", outputs.gas(selector, pressure)?),
            OutputRequest::HeadJog { up, fast } => ("head", outputs.head_jog(up, fast)?),
        };
        let plan = Plan {
            name: name.into(),
            on: manual.on,
            off: manual.off,
            lease: Duration::from_millis(manual.lease_ms),
            duration: None,
        };
        (coordinator.machine.clone(), plan)
    };
    let completed = machine.start_outputs(plan, Some(lease), requested).await?;
    spawn(shared, "Output", completed.finished());
    Ok(())
}

/// A bounded checklist gas test, using the same admission and cleanup as
/// held outputs. There is no renewable lease or laser output in this command.
pub async fn gas_test(
    shared: &Shared,
    selector: u8,
    pressure: f64,
    duration_ms: u64,
) -> Result<()> {
    shared.ensure_running()?;
    // A fixed-pressure valve (including CO2 High Air) has no pressure command.
    if !(selector <= crate::gas::MAX_GAS_SELECTOR
        && (0. ..=crate::gas::MAX_GAS_PRESSURE_BAR).contains(&pressure)
        && GAS_TEST_MS.contains(&duration_ms))
    {
        return Err(Error::Request("a gas test needs 0–5, 0–100 bar and 50–2000 ms".into()));
    }
    timed_gas(
        shared,
        selector,
        pressure,
        Duration::from_millis(duration_ms),
        "gas test",
        "Gas test",
    )
    .await
}

/// The 60-second flow test that calibrates a gas's consumption estimate
/// (Settings → Gas costs). It is the gas test's bounded valve plan with a
/// fixed, longer duration; Stop ends it like any output.
pub async fn gas_calibration(shared: &Shared, selector: u8, pressure: f64) -> Result<()> {
    shared.ensure_running()?;
    if !(selector <= crate::gas::MAX_GAS_SELECTOR
        && (0. ..=crate::gas::MAX_GAS_PRESSURE_BAR).contains(&pressure))
    {
        return Err(Error::Request("a gas flow test needs 0–5 and 0–100 bar".into()));
    }
    let duration = crate::gas::CALIBRATION_DURATION;
    timed_gas(shared, selector, pressure, duration, "gas flow test", "Gas flow test").await
}

async fn timed_gas(
    shared: &Shared,
    selector: u8,
    pressure: f64,
    duration: Duration,
    name: &str,
    label: &'static str,
) -> Result<()> {
    let requested = Instant::now();
    let plan = {
        let coordinator = shared.lock().await;
        coordinator.idle()?;
        let manual = coordinator.bound()?.outputs.gas(selector, pressure)?;
        Plan {
            name: name.into(),
            on: manual.on,
            off: manual.off,
            lease: GAS_TEST_LEASE,
            duration: Some(duration),
        }
    };
    let completed = shared.machine.start_outputs(plan, None, requested).await?;
    spawn(shared, label, completed.finished());
    Ok(())
}

/// Imports host process values while idle and invalidates compiled previews.
/// The original bytes and their hash are committed before the values change.
pub async fn import_soft(shared: &Shared, name: &str, bytes: &[u8]) -> Result<()> {
    import_soft_reviewed(shared, name, bytes, None).await
}

/// Imports only if the INI still matches the reviewed source hash.
pub async fn import_soft_reviewed(
    shared: &Shared,
    name: &str,
    bytes: &[u8],
    expected: Option<&str>,
) -> Result<()> {
    shared.ensure_running()?;
    let candidate = crate::soft_settings::SoftSettings::parse(name, bytes)?;
    let mut coordinator = shared.lock().await;
    coordinator.idle()?;
    if expected.is_some_and(|hash| hash != coordinator.soft.view().sha256.as_deref().unwrap_or(""))
    {
        return Err(Error::Refused(
            "process settings changed; review pending changes again".into(),
        ));
    }
    if coordinator
        .machine
        .state()
        .program
        .as_ref()
        .is_some_and(|p| p.state == openlaser_controller::state::ProgramState::Held)
    {
        return Err(Error::Refused(
            "finish or stop the held job before importing process settings".into(),
        ));
    }
    candidate.save(&coordinator.config.data_dir)?;
    coordinator.soft = candidate;
    if let Some(draft) = &mut coordinator.draft {
        draft.compiled = None;
    }
    coordinator.bindings_changed();
    coordinator.draft_changed();
    coordinator.note("Process settings imported", false);
    drop(coordinator);
    refresh(shared).await;
    Ok(())
}

/// Switches the laser mode. The XY reference carries over while the
/// controller keeps reporting it; parameters are verified again.
pub async fn switch_mode(shared: &Shared, mode: LaserMode) -> Result<()> {
    shared.ensure_running()?;
    {
        let mut c = shared.lock().await;
        if c.mode == Some(mode) {
            return Ok(());
        }
        if !c.connected() {
            c.idle()?;
            c.mode = Some(mode);
            c.install_bindings(None);
            c.postflight = None;
            c.publish();
            return Ok(());
        }
    }
    let problem = configure(shared, mode, true).await?;
    let mut c = shared.lock().await;
    match problem {
        None => c.note("Mode switch done", false),
        Some(problem) => c.note(format!("Mode switched; {problem}"), true),
    }
    Ok(())
}

/// Aligns a loaded job with its recipe, retaining the revision boundary across
/// the asynchronous controller switch. Concurrent edits must compile again.
pub(crate) async fn select_draft_mode(shared: &Shared, revision: u64) -> Result<u64> {
    let mode = {
        let c = shared.lock().await;
        c.check_draft(revision)?;
        let mode = c.draft.as_ref().and_then(|d| d.current.recipe.as_ref()).map(|r| r.laser);
        mode.filter(|mode| Some(*mode) != c.mode)
    };
    let Some(mode) = mode else { return Ok(revision) };
    switch_mode(shared, mode).await?;
    let c = shared.lock().await;
    c.check_draft(revision + 1)?;
    Ok(revision + 1)
}

/// Relieves one alarm row.
pub fn relieve(shared: &Shared, id: Option<u32>) -> Result<()> {
    let machine = shared.machine.clone();
    let label = if id.is_some_and(|id| {
        openlaser_protocol::alarms::relief(id) == openlaser_protocol::alarms::Relief::HeadHome
    }) {
        "Head home"
    } else {
        "Alarm reset"
    };
    let shared = Arc::clone(shared);
    tokio::spawn(async move {
        let rows = machine.state().alarms;
        let result = machine.relieve(id).await;
        let mut coordinator = shared.lock().await;
        coordinator.history.reset(&rows, id, &result);
        coordinator.outcome(label, result);
    });
    Ok(())
}

/// The stop sequence.
pub async fn stop(shared: &Shared) -> Result<()> {
    shared.stop_epoch.fetch_add(1, Ordering::AcqRel);
    let machine = shared.machine.clone();
    machine.stop().await?;
    Ok(())
}

/// Pauses the running program.
pub async fn hold(shared: &Shared) -> Result<()> {
    let machine = shared.machine.clone();
    machine.hold().await?;
    Ok(())
}

/// Runs the exact artifact accepted by the latest compile.
pub async fn run(shared: &Shared) -> Result<()> {
    run_reviewed(shared, None).await
}

/// Runs after checking a fresh operator review against these exact inputs.
pub async fn run_reviewed(
    shared: &Shared,
    confirmation: Option<&PreflightConfirmation>,
) -> Result<()> {
    // Placement capture would re-pin the held job's origin: refuse first.
    shared.lock().await.not_held()?;
    crate::placement::prepare(shared).await?;
    let epoch = shared.ensure_running()?;
    let requested = Instant::now();
    let (owner, compiled, sheet_offset, binding, name, material, origin, sheet) = {
        let mut coordinator = shared.lock().await;
        coordinator.not_held()?;
        coordinator.check_preflight(PreflightIntent::Run, epoch, confirmation)?;
        let draft =
            coordinator.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let compiled =
            draft.compiled.clone().ok_or_else(|| Error::Refused("compile the job first".into()))?;
        let configuration = compiled.configuration.ok_or_else(|| {
            Error::Refused("this is an offline preview; connect and compile again".into())
        })?;
        let sheet_offset = draft.sheet_offset()?;
        let binding = Execution::new(&coordinator, &configuration, sheet_offset)?;
        let name = coordinator.draft_name(draft);
        let material = draft.current.recipe.as_ref().map(crate::document::MaterialView::from);
        let origin = draft.origin().ok_or_else(|| Error::Refused("no job origin".into()))?;
        let sheet = if compiled.dry_run { None } else { coordinator.sheet_plan(draft)? };
        let owner = coordinator.reserve()?;
        (owner, compiled, sheet_offset, binding, name, material, origin, sheet)
    };
    let result = work(move || {
        let (program, view) =
            binding.job(&compiled.job, compiled.dry_run, compiled.view.plan.clone())?;
        let held = Arc::new(Held {
            sheet,
            job: compiled.job.clone(),
            configuration: binding.configuration,
            dry_run: compiled.dry_run,
            sheet_offset,
            view: view.clone(),
        });
        let execution = Arc::new(crate::document::ExecutionView {
            id: owner,
            frame: false,
            name,
            material,
            origin,
            sheet_offset,
            compiled: view,
        });
        Ok(Built { program, held: Some(held), execution, fresh: true })
    })
    .await;
    start_run(shared, owner, epoch, requested, result, "Job").await
}

/// Resumes only the immutable remainder belonging to the settled held run.
pub async fn resume(shared: &Shared) -> Result<()> {
    resume_reviewed(shared, None).await
}

/// Resumes with a fresh gas confirmation for the immutable held program.
pub async fn resume_reviewed(
    shared: &Shared,
    confirmation: Option<&PreflightConfirmation>,
) -> Result<()> {
    let epoch = shared.ensure_running()?;
    let requested = Instant::now();
    let (owner, recovery, binding, pierce) = {
        let mut c = shared.lock().await;
        c.check_preflight(PreflightIntent::Resume, epoch, confirmation)?;
        let recovery =
            c.recovery.clone().ok_or_else(|| Error::Refused("no retained job".into()))?;
        if !recovery.can_resume() {
            return Err(Error::Refused(
                "select a restart point and prepare stopped-job recovery first".into(),
            ));
        }
        let binding = Execution::new(&c, &recovery.configuration, recovery.original.sheet_offset)?;
        let pierce = c.bound()?.resume_pierce && !recovery.original.dry_run;
        (c.reserve()?, recovery, binding, pierce)
    };
    let result = work(move || {
        let (job, plan) = recovery.remainder(pierce)?;
        let (program, view) = binding.job_with_return(
            &job,
            recovery.original.dry_run,
            plan,
            recovery.return_position(),
        )?;
        let held = Arc::new(Held {
            sheet: recovery.original.sheet.clone(),
            job: Arc::new(job),
            configuration: recovery.configuration,
            dry_run: recovery.original.dry_run,
            sheet_offset: recovery.original.sheet_offset,
            view: view.clone(),
        });
        let execution = Arc::new(crate::document::ExecutionView {
            id: owner,
            compiled: view,
            ..(*recovery.execution).clone()
        });
        Ok(Built { program, held: Some(held), execution, fresh: false })
    })
    .await;
    start_run(shared, owner, epoch, requested, result, "Job").await
}

/// Move to the chosen restart point with the laser off and the head retracted.
pub async fn move_restart(shared: &Shared, revision: u64) -> Result<()> {
    shared.ensure_running()?;
    let requested = Instant::now();
    let motion = {
        let c = shared.lock().await;
        c.idle()?;
        let recovery =
            c.recovery.as_ref().ok_or_else(|| Error::Refused("no retained job".into()))?;
        if recovery.revision != revision || !recovery.can_resume() {
            return Err(Error::Refused("review and prepare the selected restart first".into()));
        }
        let binding = Execution::new(&c, &recovery.configuration, recovery.original.sheet_offset)?;
        let point = recovery
            .view()
            .position
            .ok_or_else(|| Error::Refused("select a restart point".into()))?;
        if point
            .iter()
            .enumerate()
            .any(|(axis, p)| *p < binding.extent[axis][0] || *p > binding.extent[axis][1])
        {
            return Err(Error::Refused("the restart point is outside travel limits".into()));
        }
        let bound = c.bound()?;
        let scale = c.scale()?;
        let delta = [
            openlaser_xml::bindings::coordinate(point[0] - binding.position[0], scale)?,
            openlaser_xml::bindings::coordinate(point[1] - binding.position[1], scale)?,
        ];
        if delta == [0, 0] {
            return Ok(());
        }
        Request {
            delta,
            speed: bound.jog.speed_word(bound.jog.speed[0], scale)?,
            acceleration: bound.jog.position_acceleration[0],
            held: false,
            deceleration: bound.jog.deceleration,
        }
    };
    let completed = shared.machine.start_positioning(motion, requested).await?;
    spawn(shared, "Move to restart", completed.finished());
    Ok(())
}

/// The machine-coordinate binding, captured once before expensive work.
#[derive(Clone, Copy)]
struct Execution {
    configuration: Configuration,
    position: [f64; 2],
    sheet_offset: [f64; 2],
    extent: [[f64; 2]; 2],
    co2_pwm_type: u8,
}

impl Execution {
    fn new(
        coordinator: &crate::Coordinator,
        configuration: &Configuration,
        sheet_offset: [f64; 2],
    ) -> Result<Self> {
        if coordinator.acceptance()? != *configuration {
            return Err(Error::Refused("the machine configuration changed; compile again".into()));
        }
        let state = coordinator.machine.state();
        if !state.session.homed {
            return Err(Error::Refused("run Go Origin first".into()));
        }
        if let Some(blocked) = state.blocked {
            return Err(Error::Refused(format!("alarms are active: {blocked}")));
        }
        let feedback =
            state.feedback.ok_or_else(|| Error::Refused("no machine feedback".into()))?;
        if !feedback.stationary || feedback.age_ms > crate::coordinator::FRESH_FEEDBACK_MS {
            return Err(Error::Refused("fresh stationary feedback is required".into()));
        }
        let bound = coordinator.bound()?;
        let extent = std::array::from_fn(|axis| {
            within_bank(
                bound.jog.extent[axis],
                &configuration.verified.banks[axis],
                configuration.verified.scale,
            )
        });
        Ok(Self {
            configuration: *configuration,
            position: [feedback.position_mm[0], feedback.position_mm[1]],
            sheet_offset,
            extent,
            co2_pwm_type: if bound.mode == LaserMode::Co2 { bound.co2_control_type } else { 0 },
        })
    }

    fn current(self) -> [f64; 2] {
        [self.position[0] - self.sheet_offset[0], self.position[1] - self.sheet_offset[1]]
    }

    fn job(
        self,
        job: &openlaser_compiler::program::Job,
        dry_run: bool,
        plan: Vec<crate::document::PassView>,
    ) -> Result<(Program, Arc<crate::document::Compiled>)> {
        self.job_with_return(job, dry_run, plan, None)
    }

    fn job_with_return(
        self,
        job: &openlaser_compiler::program::Job,
        dry_run: bool,
        plan: Vec<crate::document::PassView>,
        return_to: Option<[f64; 2]>,
    ) -> Result<(Program, Arc<crate::document::Compiled>)> {
        if job.settings.interpolation_cycle_us != self.configuration.verified.cycle_us {
            return Err(Error::Refused(
                "the program and controller interpolation cycles differ".into(),
            ));
        }
        let z_units_per_mm = u32::try_from(self.configuration.verified.scale).ok();
        let program = job.program_with_return(
            openlaser_compiler::program::Binding { current: self.current(), z_units_per_mm },
            return_to.map(|p| [p[0] - self.sheet_offset[0], p[1] - self.sheet_offset[1]]),
        )?;
        let upload = self.program(&program, job.settings.counts_per_mm)?;
        let view = Arc::new(draft::summary(job, &program, upload.blocks.len(), dry_run, plan));
        Ok((upload, view))
    }

    fn program(
        self,
        program: &openlaser_compiler::program::Program,
        counts: [f64; 2],
    ) -> Result<Program> {
        crate::envelope::validate(program, self.position, self.sheet_offset, self.extent, counts)?;
        Ok(Program {
            prepare_head: program.records.iter().any(|r| {
                matches!(
                    r,
                    openlaser_protocol::records::Record::Follow { .. }
                        | openlaser_protocol::records::Record::HeightAbsolute { .. }
                        | openlaser_protocol::records::Record::PierceHeight { .. }
                        | openlaser_protocol::records::Record::PierceRamp { .. }
                        | openlaser_protocol::records::Record::FrogJump { .. }
                        | openlaser_protocol::records::Record::Lift { .. }
                )
            }),
            position: self.position,
            configuration: self.configuration,
            blocks: draft::blocks(program)?,
            mode: self.configuration.mode,
            co2_pwm_type: self.co2_pwm_type,
            expected_seconds: program.seconds,
        })
    }
}

async fn work<T: Send + 'static>(build: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(build)
        .await
        .map_err(|e| Error::Refused(format!("build task: {e}")))?
}

struct Built {
    fresh: bool,
    program: Program,
    held: Option<Arc<Held>>,
    execution: Arc<crate::document::ExecutionView>,
}

/// One reservation owns construction, controller admission and completion.
async fn start_run(
    shared: &Shared,
    owner: u64,
    epoch: u64,
    requested: Instant,
    result: Result<Built>,
    label: &'static str,
) -> Result<()> {
    let result = async {
        let Built { program, held, execution, fresh } = result?;
        {
            let mut coordinator = shared.lock().await;
            if shared.ensure_running()? != epoch || coordinator.operation != Some(owner) {
                return Err(Error::Refused("the operation was cancelled".into()));
            }
            if coordinator.acceptance()? != program.configuration {
                return Err(Error::Refused(
                    "the machine configuration changed during construction".into(),
                ));
            }
            if let Some(plan) =
                held.as_ref().filter(|h| !h.dry_run).and_then(|h| h.sheet.as_deref())
            {
                coordinator.sheet_store.begin(plan)?;
            }
        }
        let running = shared.machine.start_run_at(program, requested).await?;
        {
            let mut coordinator = shared.lock().await;
            coordinator.run_started(owner, fresh, held, execution);
        }
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            let outcome = running.finished().await;
            let mut coordinator = shared.lock().await;
            coordinator.run_finished(owner, label, outcome);
        });
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        let mut coordinator = shared.lock().await;
        if coordinator.operation == Some(owner) {
            coordinator.operation = None;
            coordinator.note(error.to_string(), true);
        }
    }
    result
}

impl crate::Coordinator {
    fn run_started(
        &mut self,
        owner: u64,
        fresh: bool,
        held: Option<Arc<Held>>,
        execution: Arc<crate::document::ExecutionView>,
    ) {
        if self.operation == Some(owner) {
            if let Some(current) = &held {
                if fresh {
                    if let Some(draft) = &mut self.draft {
                        draft.capture_used = true;
                    }
                    let hash =
                        self.files.backup.as_ref().map(|f| f.sha256.clone()).unwrap_or_default();
                    self.recovery = Some(crate::resume::Recovery::new(
                        current.clone(),
                        execution.clone(),
                        hash,
                    ));
                    self.begin_gas_run(execution.id);
                } else if let Some(recovery) = &mut self.recovery {
                    recovery.dispatched(current.clone(), execution.id);
                }
            }
            self.held = held;
            self.completed_sheet = None;
            self.postflight = None;
            self.execution_changed(Some(execution));
        }
    }

    fn run_finished(
        &mut self,
        owner: u64,
        label: &str,
        outcome: openlaser_controller::Result<Ending>,
    ) {
        if self.operation != Some(owner) {
            return;
        }
        self.operation = None;
        // Framing owns an immutable copy of the job preview while it
        // moves. Return to the draft once motion has settled, including
        // when the frame was stopped or failed.
        if self.execution.as_ref().is_some_and(|execution| execution.frame) {
            self.execution_changed(None);
        }
        if !matches!(outcome, Ok(Ending::Held))
            && let Err(error) = self.finish_sheet(matches!(outcome, Ok(Ending::Completed)))
        {
            self.sheet_persistence_error = Some(error.to_string());
        }
        match outcome {
            Ok(Ending::Held) if self.held.is_some() => {
                self.note("Paused; Resume returns to the saved position", false);
            }
            other => match other {
                Ok(Ending::Completed) => {
                    self.record_gas(true);
                    self.completed_postflight();
                    self.held = None;
                    self.note(format!("{label} finished"), false);
                }
                Ok(_) => {
                    self.record_gas(false);
                    self.completed_postflight();
                    self.held = None;
                    self.note(format!("{label} stopped"), false);
                }
                Err(error) => {
                    self.record_gas(false);
                    self.note(format!("{label} failed: {error}"), true);
                }
            },
        }
    }
}

/// Frames the compiled process envelope (including leads and cleaning), with
/// the laser off. Initial approach and return are checked separately too.
pub async fn frame(shared: &Shared) -> Result<()> {
    shared.lock().await.not_held()?;
    crate::placement::prepare(shared).await?;
    let epoch = shared.ensure_running()?;
    let requested = Instant::now();
    let (owner, compiled, binding, settings, name, material, origin) = {
        let mut coordinator = shared.lock().await;
        coordinator.not_held()?;
        let draft =
            coordinator.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let compiled = draft
            .compiled
            .clone()
            .ok_or_else(|| Error::Refused("compile before framing the executed envelope".into()))?;
        let configuration = compiled
            .configuration
            .ok_or_else(|| Error::Refused("connect and compile again".into()))?;
        let binding = Execution::new(&coordinator, &configuration, draft.sheet_offset()?)?;
        let name = coordinator.draft_name(draft);
        let material = draft.current.recipe.as_ref().map(crate::document::MaterialView::from);
        let origin = draft.origin().ok_or_else(|| Error::Refused("no job origin".into()))?;
        let settings = coordinator.bound()?.frame;
        if ((settings.cadence_ms * 1000.).round() - f64::from(configuration.verified.cycle_us))
            .abs()
            > f64::EPSILON
        {
            return Err(Error::Refused("frame and controller interpolation cycles differ".into()));
        }
        let owner = coordinator.reserve()?;
        (owner, compiled, binding, settings, name, material, origin)
    };
    let result = work(move || {
        let scale = u32::try_from(binding.configuration.verified.scale)
            .map_err(|_| Error::Refused("invalid height-controller scale".into()))?;
        let bounds = crate::envelope::process_bounds(&compiled.job, scale)?;
        let program = openlaser_compiler::frame::program(bounds, binding.current(), &settings)?;
        let upload = binding.program(&program, settings.counts_per_mm)?;
        let execution = Arc::new(crate::document::ExecutionView {
            id: owner,
            frame: true,
            name,
            material,
            origin,
            sheet_offset: binding.sheet_offset,
            compiled: compiled.view.clone(),
        });
        Ok(Built { program: upload, held: None, execution, fresh: false })
    })
    .await;
    start_run(shared, owner, epoch, requested, result, "Frame").await
}

/// Prepares a pending draft revision outside the document lock.
pub async fn prepare(shared: &Shared) -> Result<()> {
    let Some(input) = shared.lock().await.preparation()? else { return Ok(()) };
    prepare_input(shared, input).await.map(|_| ())
}

pub(crate) async fn prepare_input(
    shared: &Shared,
    mut input: crate::coordinator::Preparation,
) -> Result<std::sync::Arc<crate::document::DraftView>> {
    let input = work(move || {
        input.draft.prepare(&input.drawing);
        input.draft.settle(input.extent);
        Ok(input)
    })
    .await?;
    let mut coordinator = shared.lock().await;
    coordinator.prepared(input)?;
    coordinator
        .document()
        .draft
        .clone()
        .ok_or_else(|| Error::Refused("the draft was closed".into()))
}

/// Compiles outside the control lock and installs only into its input revision.
pub async fn compile(shared: &Shared, dry_run: bool) -> Result<()> {
    prepare(shared).await?;
    let revision = shared
        .lock()
        .await
        .document()
        .draft
        .as_ref()
        .map(|d| d.revision)
        .ok_or_else(|| Error::Refused("open a part first".into()))?;
    compile_revision(shared, dry_run, revision).await
}

/// Rebuild after a non-authoring transition such as connection or homing.
/// Preparation errors belong to the draft; they do not undo the transition.
pub(crate) async fn refresh(shared: &Shared) {
    let (input, revision) = {
        let c = shared.lock().await;
        (c.preparation(), c.document().draft_revision)
    };
    let revision = match input {
        Ok(Some(input)) => match prepare_input(shared, input).await {
            Ok(draft) => draft.revision,
            Err(_) => return,
        },
        Ok(None) => revision,
        Err(_) => return,
    };
    let _ = compile_automatically(shared, revision).await;
}

/// Build an accepted edit without requiring a separate operator action.
/// Missing inputs wait for the next edit; stale work cannot replace a newer draft.
pub(crate) async fn compile_automatically(shared: &Shared, revision: u64) -> Result<()> {
    let dry_run =
        {
            let c = shared.lock().await;
            c.check_draft(revision)?;
            let Some(draft) = &c.draft else { return Ok(()) };
            if draft.prepared.is_none() || draft.current.recipe.is_none() || c.bundle.is_none() {
                return Ok(());
            }
            if draft.compiled.as_ref().is_some_and(|p| {
                p.dry_run == draft.dry_run && p.configuration == c.acceptance().ok()
            }) {
                return Ok(());
            }
            draft.dry_run
        };
    build_revision(shared, dry_run, revision, true).await
}

pub(crate) async fn compile_revision(shared: &Shared, dry_run: bool, revision: u64) -> Result<()> {
    build_revision(shared, dry_run, revision, false).await
}

async fn build_revision(
    shared: &Shared,
    dry_run: bool,
    revision: u64,
    automatic: bool,
) -> Result<()> {
    shared.ensure_running()?;
    let revision = select_draft_mode(shared, revision).await?;
    let inputs = {
        let mut coordinator = shared.lock().await;
        coordinator.check_draft(revision)?;
        if let Some(draft) = &mut coordinator.draft
            && draft.dry_run != dry_run
        {
            draft.dry_run = dry_run;
            draft.compiled = None;
            coordinator.draft_changed();
            coordinator.queue_draft();
        }
        match coordinator.compile_inputs(dry_run) {
            Ok(inputs) => inputs,
            Err(error) => {
                if let Some(draft) = &mut coordinator.draft {
                    draft.compiled = None;
                    draft.error = Some(error.to_string());
                }
                coordinator.draft_changed();
                return if automatic { Ok(()) } else { Err(error) };
            }
        }
    };
    let stamp = inputs.stamp;
    let result = work(move || {
        let prepared = inputs.prepared.shifted(inputs.settings.contour_shift)?;
        let prepared =
            crate::correction::prepare(&prepared, inputs.correction.as_ref(), inputs.placement)?;
        let mut compiled = draft::compile(
            &prepared,
            &inputs.settings,
            inputs.film.as_ref(),
            dry_run,
            inputs.scale,
        )?;
        compiled.configuration = inputs.configuration;
        Ok(compiled)
    })
    .await;
    let mut c = shared.lock().await;
    c.check_compile(stamp)?;
    let outcome = c.compiled(stamp, result);
    if automatic { Ok(()) } else { outcome }
}
