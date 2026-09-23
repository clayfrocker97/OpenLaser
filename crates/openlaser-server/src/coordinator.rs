// SPDX-License-Identifier: GPL-3.0-or-later

//! The coordinator: the library, the draft, the machine files and the
//! controller task, behind one lock, publishing one document.
//!
//! Commands that finish quickly run under the lock. Commands that take
//! seconds, a reference search or a program, are started here and finish
//! in a task of their own; their progress shows in the controller's state
//! and their outcome in the message.

use crate::bindings::{self, Files};
use crate::connect;
use crate::document::{
    BindingsView, Document, DraftView, ExecutionView, FeatureSource, FilesView, Gate, JobView,
    LibraryView, Message, OutputsView, PartView, PickView, Readiness, RecipeView, Revisions,
    RuleView, SkippedView,
};
use crate::draft::{self, Binder, Draft};
use crate::{Error, Result, recipes};
use openlaser_compiler::program::Job as CompiledJob;
use openlaser_compiler::settings::Settings;
use openlaser_controller::Machine;
use openlaser_controller::alarms::Rule;
use openlaser_controller::session::Configuration;
use openlaser_controller::state::Connection;
use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_core::geometry::Transform;
use openlaser_library::{Id, Library};
use openlaser_protocol::registers::{AXIS_COUNT, PARAMETER_BANK_WORDS};
use openlaser_xml::Bundle;
use openlaser_xml::bindings as vendor;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, MutexGuard, watch};

/// Why a new run or frame is refused while a job is held.
const HELD: &str = "a job is paused; resume or stop it first";

/// How the server is set up.
#[derive(Clone, Debug)]
pub struct Config {
    /// Where the HTTP server listens.
    pub listen: SocketAddr,
    /// Where the library lives.
    pub data_dir: PathBuf,
    /// How to reach the controller.
    pub machine: openlaser_controller::Config,
    /// The vendor files.
    pub files: Files,
    /// The operating mode to start in; the files' saved mode otherwise.
    pub mode: Option<LaserMode>,
    /// Bypass the CO2 height controller and its crash protection. This is
    /// independent of manually setting optical focus; fiber Z remains enabled.
    pub co2_manual_focus: bool,
    /// Where development builds read the UI; embedded builds ignore this path.
    pub ui_dir: PathBuf,
    /// The computer's network, for the connect workflow; none when the
    /// controller is the simulator on loopback.
    pub network: Option<Arc<dyn openlaser_network::os::Os>>,
}

/// What the last run left behind for a continuation.
#[derive(Clone, Debug)]
pub struct Held {
    /// Immutable stock and cut areas belonging to this original run.
    pub sheet: Option<Arc<crate::stock_store::SheetPlan>>,
    /// The job as run, every pass with its own geometry and process.
    pub job: Arc<CompiledJob>,
    /// The exact machine configuration used for this execution.
    pub configuration: Configuration,
    /// Whether it was a dry run.
    pub dry_run: bool,
    /// The sheet offset: what the machine adds to a drawing coordinate.
    pub sheet_offset: [f64; 2],
    /// The exact dispatched program view and physical pass identities.
    pub view: Arc<crate::document::Compiled>,
}

/// The coordinator.
pub struct Coordinator {
    pub(crate) sheet_store: crate::stock_store::Store,
    pub(crate) sheet_persistence_error: Option<String>,
    pub(crate) completed_sheet: Option<String>,
    pub(crate) correction: crate::correction::Store,
    pub(crate) fonts: crate::font_store::Store,
    pub(crate) nesting_task: Option<crate::nesting::Task>,
    /// Original execution and its recovery choices, retained in this session.
    pub recovery: Option<crate::resume::Recovery>,
    recovery_view: Option<Arc<crate::resume::RecoveryView>>,
    pub(crate) queued_draft: Option<Arc<draft::AuthoringState>>,
    pub(crate) draft_store: crate::workspace::Store,
    /// An authoring persistence failure, kept visible until saving works.
    pub persistence_error: Option<String>,
    /// Operator checklist defaults, never completed checks.
    pub preflight: crate::preflight::PreflightPreferences,
    pub(crate) postflight: Option<crate::postflight::Pending>,
    /// Preserved host process overrides.
    pub soft: crate::soft_settings::SoftSettings,
    /// How long every screen's held controls must be held.
    pub hold: crate::touch::HoldTimes,
    /// Gas prices, the run history and the run being recorded.
    pub gas: crate::gas::Store,
    /// Read-only historical alarm recording.
    pub history: crate::alarm_history::History,
    /// The configuration.
    pub config: Config,
    /// The library.
    pub library: Library,
    /// The controller task.
    pub machine: Machine,
    /// The vendor files.
    pub bundle: Option<Arc<Bundle>>,
    /// The vendor files as the settings page lists them.
    pub files: Arc<FilesView>,
    /// The bindings for the operating mode, once the controller's scale is
    /// known.
    pub bound: Option<vendor::Bindings>,
    /// Why the files could not be read or bound.
    pub bindings_error: Option<String>,
    /// The operating mode.
    pub mode: Option<LaserMode>,
    /// Configuration measured on this connection and matched to the files.
    pub accepted: Option<Configuration>,
    /// Successful head calibration, scoped to its material and connection.
    pub(crate) calibration: Option<crate::setup::Calibration>,
    /// Why the most recent parameter read could not match the machine files.
    pub(crate) parameter_problem: Option<String>,
    /// The job being set up.
    pub draft: Option<Draft>,
    /// The last run, for a continuation.
    pub held: Option<Arc<Held>>,
    /// Projection of the last admitted execution; no mutable recipe authority.
    pub execution: Option<Arc<ExecutionView>>,
    /// Owner of the operation being built or executed.
    pub(crate) operation: Option<u64>,
    operation_serial: u64,
    compile_serial: u64,
    /// The computer's side of the machine link.
    pub link: connect::Link,
    library_view: Arc<LibraryView>,
    draft_view: Option<Arc<DraftView>>,
    pub(crate) draft_generation: u64,
    binding_view: Option<Arc<BindingsView>>,
    revisions: Revisions,
    message: Option<Message>,
    revision: u64,
    publisher: watch::Sender<Document>,
}

/// The simulator's units per millimetre.
const SIMULATOR_SCALE: i32 = 1000;

/// The five axis parameter banks.
pub type Banks = [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT];

/// The coordinator as the API shares it.
pub type Shared = Arc<SharedState>;

/// Control delivery is independent of the document and geometry lock.
pub struct SharedState {
    pub(crate) font_import: Mutex<()>,
    /// Direct handle for stop, release and lease renewal.
    pub machine: Machine,
    coordinator: Mutex<Coordinator>,
    /// Counts Stop presses and the shutdown. Work that is built off the lock
    /// records it first and gives up if it has moved on when it returns.
    pub(crate) stop_epoch: AtomicU64,
    /// Once set, no new work is admitted.
    pub(crate) closing: watch::Sender<bool>,
}

impl SharedState {
    /// Access to short document transactions.
    pub async fn lock(&self) -> MutexGuard<'_, Coordinator> {
        self.coordinator.lock().await
    }

    /// Refuses new work once the server is shutting down; otherwise the
    /// current stop epoch, for the caller to compare again after awaiting.
    pub(crate) fn ensure_running(&self) -> Result<u64> {
        if *self.closing.borrow() {
            return Err(Error::Refused("the server is shutting down".into()));
        }
        Ok(self.stop_epoch.load(Ordering::Acquire))
    }
}

/// Identifies the inputs of a compile, including competing compile requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileStamp {
    draft: u64,
    binding: u64,
    serial: u64,
}

impl Coordinator {
    /// Opens the library, reads the machine files and spawns the controller
    /// task. Machine files that fail to read are reported, not fatal.
    #[allow(
        clippy::too_many_lines,
        reason = "one constructor keeps the coordinator's owned state and startup tasks together"
    )]
    pub fn start(config: Config) -> std::result::Result<Shared, String> {
        let library = Library::open(&config.data_dir).map_err(|e| e.to_string())?;
        for skipped in library.skipped() {
            tracing::warn!(file = %skipped.file, reason = %skipped.reason, "library file left out");
        }
        let sheet_store =
            crate::stock_store::Store::open(&config.data_dir).map_err(|e| e.to_string())?;
        let correction =
            crate::correction::Store::open(&config.data_dir).map_err(|e| e.to_string())?;
        let fonts = crate::font_store::Store::open(&config.data_dir).map_err(|e| e.to_string())?;
        let preflight = crate::preflight::PreflightPreferences::open(&config.data_dir)
            .map_err(|e| e.to_string())?;
        let soft = crate::soft_settings::SoftSettings::open(&config.data_dir)
            .map_err(|e| e.to_string())?;
        let (machine, observations) = Machine::spawn_with_alarm_events(config.machine.clone());
        let history = crate::alarm_history::History::start(&config.data_dir, observations)
            .map_err(|e| e.to_string())?;
        let mut history_changes = history.watch();
        let draft_store = crate::workspace::Store::new(&config.data_dir);
        let mut draft_requests = draft_store.requests();
        let mut draft_status = draft_store.watch();
        let (publisher, _) = watch::channel(Document::default());
        let mut coordinator = Self {
            calibration: None,
            sheet_store,
            sheet_persistence_error: None,
            completed_sheet: None,
            correction,
            fonts,
            nesting_task: None,
            recovery: None,
            recovery_view: None,
            queued_draft: None,
            draft_store: draft_store.clone(),
            persistence_error: None,
            preflight,
            postflight: None,
            soft,
            hold: crate::touch::HoldTimes::open(&config.data_dir),
            gas: crate::gas::Store::open(&config.data_dir),
            history,
            mode: config.mode,
            link: connect::Link::open(&config),
            config,
            library,
            machine: machine.clone(),
            bundle: None,
            files: Arc::default(),
            bound: None,
            bindings_error: None,
            accepted: None,
            parameter_problem: None,
            draft: None,
            held: None,
            operation: None,
            operation_serial: 0,
            execution: None,
            compile_serial: 0,
            library_view: Arc::default(),
            draft_view: None,
            draft_generation: 0,
            binding_view: None,
            revisions: Revisions::default(),
            message: None,
            revision: 0,
            publisher,
        };
        coordinator.load_files();
        coordinator.name_gases();
        coordinator.revisions.draft = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or_default();
        coordinator.draft_generation = coordinator.revisions.draft;
        if let Err(error) = coordinator.restore_active() {
            coordinator.persistence_error = Some(error.to_string());
        }
        let restored = coordinator.preparation().map_err(|error| error.to_string())?;
        coordinator.library_changed();
        let shared = Arc::new(SharedState {
            font_import: Mutex::new(()),
            machine: machine.clone(),
            coordinator: Mutex::new(coordinator),
            stop_epoch: AtomicU64::new(0),
            closing: watch::channel(false).0,
        });
        let mut closing = shared.closing.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = draft_requests.changed() => if result.is_err() { break; },
                    _ = closing.changed() => break,
                }
                let _ = draft_store.flush().await;
            }
        });
        if let Some(input) = restored {
            // Capture only the startup draft. A delayed task must not take
            // over preparation of a new edit made after start returned.
            let target = Arc::clone(&shared);
            let revision = input.revision;
            tokio::spawn(async move {
                let outcome = async {
                    let draft = crate::machine::prepare_input(&target, input).await?;
                    crate::machine::compile_automatically(&target, draft.revision).await
                }
                .await;
                if let Err(error) = outcome {
                    let mut c = target.lock().await;
                    if c.revisions.draft == revision {
                        c.note(format!("Restored draft: {error}"), true);
                    }
                }
            });
        }
        // Every change of the controller's state republishes the document.
        let forward = Arc::clone(&shared);
        tokio::spawn(async move {
            let mut states = machine.watch();
            let mut closing = forward.closing.subscribe();
            loop {
                tokio::select! {
                    result = states.changed() => if result.is_err() { break; },
                    result = history_changes.changed() => if result.is_err() { break; },
                    result = draft_status.changed() => {
                        if result.is_err() { break; }
                        let error = draft_status.borrow_and_update().clone();
                        forward.lock().await.persistence_error = error;
                    },
                    _ = closing.changed() => break,
                }
                forward.lock().await.publish();
            }
        });
        Ok(shared)
    }

    /// A receiver that wakes on every publication.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Document> {
        self.publisher.subscribe()
    }

    /// Builds and publishes the document.
    pub fn publish(&mut self) {
        self.revision += 1;
        let machine = self.machine.state();
        self.update_calibration(&machine);
        // A held run has already released its completion future. Stopping
        // it must still end the job and offer postflight. Do this on the
        // state publication path, never on the urgent Stop delivery path.
        if self.operation.is_none()
            && machine.operation.is_none()
            && self.held.is_some()
            && machine
                .program
                .as_ref()
                .is_some_and(|p| p.state == openlaser_controller::state::ProgramState::Stopped)
        {
            if let Err(error) = self.finish_sheet(false) {
                self.sheet_persistence_error = Some(error.to_string());
            }
            self.record_gas(false);
            self.completed_postflight();
            self.held = None;
            self.message = Some(Message {
                id: self.revision,
                text: "Job stopped".into(),
                error: false,
                at: openlaser_library::now(),
            });
        }
        if let Some(recovery) = &mut self.recovery {
            recovery.observe(&machine, self.execution.as_deref());
            if self
                .recovery_view
                .as_ref()
                .is_none_or(|v| v.revision != recovery.revision || v.id != recovery.execution.id)
            {
                self.recovery_view = Some(Arc::new(recovery.view()));
                self.revisions.recovery += 1;
            }
        }
        let can_resume = self.operation.is_none()
            && self.recovery.as_ref().is_some_and(crate::resume::Recovery::can_resume);
        let readiness = self.readiness(&machine, can_resume);
        let progress = self.execution.as_ref().and_then(|execution| execution.progress(&machine));
        let document = Document {
            completed_sheet: self.completed_sheet.clone(),
            postflight: self.postflight_notice(),
            recovery: self.recovery_view.clone(),
            persistence_error: self
                .sheet_persistence_error
                .clone()
                .or_else(|| self.persistence_error.clone()),
            preflight_revision: self.preflight.revision,
            alarm_history: self.history.status(),
            soft: self.soft.view(),
            hold: self.hold,
            gas: self.gas.view(self.revisions.draft, self.draft_view.as_deref()),
            revision: self.revision,
            calibration: self.calibration_view(&machine),
            machine,
            mode: self.mode,
            bindings: self.binding_view.clone(),
            bindings_error: self.bindings_error.clone(),
            files: self.files.clone(),
            library: self.library_view.clone(),
            draft: self.draft_view.clone(),
            draft_revision: self.revisions.draft,
            execution: self.execution.clone(),
            progress,
            can_resume,
            readiness,
            message: self.message.clone(),
            link: self.link.view(&self.config),
            revisions: self.revisions,
        };
        self.publisher.send_replace(document);
    }

    /// The current document.
    #[must_use]
    pub fn document(&self) -> Document {
        self.publisher.borrow().clone()
    }

    /// Records a note for the operator and publishes.
    pub fn note(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            tracing::warn!(%text);
        } else {
            tracing::info!(%text);
        }
        self.message =
            Some(Message { id: self.revision + 1, text, error, at: openlaser_library::now() });
        self.publish();
    }

    /// Records an outcome: the label on success, the error otherwise.
    pub fn outcome<T>(
        &mut self,
        label: &str,
        result: std::result::Result<T, impl std::fmt::Display>,
    ) {
        match result {
            Ok(_) => self.note(format!("{label} done"), false),
            Err(error) => self.note(format!("{label}: {error}"), true),
        }
    }

    /// Whether the controller is connected with a complete snapshot.
    #[must_use]
    pub fn connected(&self) -> bool {
        matches!(self.machine.state().connection, Connection::Connected { .. })
    }

    /// What the server would accept now.
    fn head_jog_gate(&self, gate: Gate) -> Gate {
        if self.bound.as_ref().is_some_and(|b| b.outputs.head_jog_tenths.is_some()) {
            gate
        } else {
            Gate::closed("no manual Z control configured")
        }
    }

    fn xy_jog_gate(&self, gate: Gate) -> Gate {
        if !gate.ok {
            return gate;
        }
        self.jog_parameters().map_or_else(|error| Gate::closed(error.to_string()), |_| Gate::open())
    }

    fn readiness(&self, machine: &openlaser_controller::State, can_resume: bool) -> Readiness {
        use openlaser_controller::state::ProgramState;
        let connected = matches!(machine.connection, Connection::Connected { .. });
        let running = machine
            .program
            .as_ref()
            .is_some_and(|p| matches!(p.state, ProgramState::Running | ProgramState::Finishing));
        let base = |blocked: &Option<String>| self.operation_gate(machine, blocked.as_deref());
        let motion = |referenced: bool, blocked: &Option<String>| -> Gate {
            let gate = base(blocked);
            if !gate.ok {
                return gate;
            }
            if referenced && !machine.session.homed {
                return Gate::closed("home XY first");
            }
            let parameters =
                if referenced { self.acceptance() } else { self.measured_configuration() };
            if let Err(error) = parameters {
                return Gate::closed(error.to_string());
            }
            Gate::open()
        };
        let draft = self.draft.as_ref();
        let compile = match draft {
            None => Gate::closed("open a part first"),
            Some(d) if d.current.recipe.is_none() => Gate::closed("choose a material first"),
            Some(d) if d.prepared.is_none() => {
                Gate::closed(d.error.clone().unwrap_or_else(|| "nothing prepared".into()))
            }
            Some(_) => Gate::open(),
        };
        let run = {
            let gate = motion(true, &machine.blocked);
            match draft {
                _ if !gate.ok => gate,
                _ if self.held.is_some() => Gate::closed(HELD),
                Some(d) if d.compiled.is_none() => d
                    .error
                    .as_ref()
                    .map_or_else(|| compile.clone(), |error| Gate::closed(error.clone())),
                Some(_) => Gate::open(),
                None => Gate::closed("open a part first"),
            }
        };
        let frame = run.clone();
        Readiness {
            connect: if connected {
                Gate::closed("already connected")
            } else if self.link.busy() {
                Gate::closed("connecting")
            } else {
                Gate::open()
            },
            home: base(&machine.motion_blocked),
            calibrate: if self.bound.as_ref().is_some_and(|b| b.head_enabled) {
                base(&machine.setup_blocked)
            } else {
                Gate::closed("no head controller")
            },
            jog: motion(false, &machine.motion_blocked),
            xy_jog: std::array::from_fn(|axis| {
                std::array::from_fn(|direction| {
                    self.xy_jog_gate(base(&machine.xy_jog_blocked[axis][direction]))
                })
            }),
            xy_recovery: machine.xy_recovery,
            head_up: self.head_jog_gate(base(&machine.head_jog_blocked[1])),
            head_down: self.head_jog_gate(base(&machine.head_jog_blocked[0])),
            head_recovery: machine.head_recovery,
            position: motion(true, &machine.motion_blocked),
            outputs: base(&machine.blocked),
            set_origin: self.origin_gate(machine),
            compile,
            run,
            resume: if can_resume {
                motion(true, &machine.blocked)
            } else {
                Gate::closed("nothing is held")
            },
            hold: if running { Gate::open() } else { Gate::closed("no program is running") },
            stop: if connected { Gate::open() } else { Gate::closed("connect the machine first") },
            mode: base(&machine.motion_blocked),
            frame,
        }
    }

    fn operation_gate(&self, machine: &openlaser_controller::State, blocked: Option<&str>) -> Gate {
        if !matches!(machine.connection, Connection::Connected { .. }) {
            return Gate::closed("connect the machine first");
        }
        if self.bound.is_none() {
            return Gate::closed(
                self.bindings_error
                    .clone()
                    .unwrap_or_else(|| "the machine files are not bound".into()),
            );
        }
        if let Some(operation) = &machine.operation {
            return Gate::closed(format!("{} is active", operation.kind.label()));
        }
        if self.operation.is_some() {
            return Gate::closed("an operation is being prepared");
        }
        if let Some(blocked) = blocked {
            return Gate::closed(format!("alarms are active: {blocked}"));
        }
        Gate::open()
    }

    pub(crate) fn library_changed(&mut self) {
        self.revisions.library += 1;
        self.library_view = Arc::new(LibraryView {
            folders: self.library.folders().to_vec(),
            parts: self.library.parts().map(PartView::new).collect(),
            recipes: self.library.recipes().map(RecipeView::new).collect(),
            jobs: self
                .library
                .jobs()
                .filter_map(|job| {
                    self.library.job_drawing(&job.parts).ok().map(|d| JobView::new(job, &d))
                })
                .collect(),
            skipped: self
                .library
                .skipped()
                .iter()
                .map(|s| SkippedView { file: s.file.clone(), reason: s.reason.clone() })
                .collect(),
        });
        self.publish();
    }

    pub(crate) fn draft_changed(&mut self) {
        self.queue_draft();
        self.revisions.draft += 1;
        self.draft_view = self.draft.as_ref().map(|draft| {
            let mut view = draft.view();
            view.sheets = self.sheet_navigation(draft);
            view.name = self.draft_name(draft);
            view.revision = self.revisions.draft;
            view.generation = self.draft_generation;
            Arc::new(view)
        });
        self.publish();
    }

    pub(crate) fn bindings_changed(&mut self) {
        self.revisions.bindings += 1;
        self.binding_view = self.bound.as_ref().map(|b| Arc::new(bindings_view(b)));
    }

    pub(crate) fn execution_changed(&mut self, execution: Option<Arc<ExecutionView>>) {
        self.execution = execution;
        self.revisions.execution += 1;
        self.publish();
    }

    /// Refuses a geometry request made against another draft revision.
    pub fn check_draft(&self, revision: u64) -> Result<()> {
        if self.draft.is_none() || self.revisions.draft != revision {
            return Err(Error::Refused("the draft changed; refresh the edit or pick".into()));
        }
        Ok(())
    }

    // Machine files -------------------------------------------------------------

    /// Reads the machine backup: the configured one, replaced by an
    /// import. A file that fails to read is reported, not fatal.
    fn load_files(&mut self) {
        let loaded = self
            .config
            .files
            .clone()
            .with_dir(&self.config.data_dir.join("machine"))
            .and_then(|files| bindings::load(&files));
        match loaded {
            Ok(loaded) => self.install_files(loaded),
            Err(error) => {
                self.revisions.files += 1;
                self.files =
                    Arc::new(FilesView { error: Some(error.clone()), ..FilesView::default() });
                self.bundle = None;
                self.bindings_error = Some(error);
            }
        }
        if self.mode.is_none() {
            self.mode = self.bundle.as_ref().and_then(|b| bindings::saved_mode(b));
        }
    }

    pub(crate) fn install_files(&mut self, loaded: bindings::Loaded) {
        let manual = self.config.co2_manual_focus;
        self.files = Arc::new(FilesView {
            backup: Some(loaded.file),
            banks: bindings::banks(&loaded.bundle),
            gases: bindings::gases(&loaded.bundle),
            capabilities: [LaserMode::Fiber, LaserMode::Co2]
                .into_iter()
                .filter_map(|mode| {
                    bindings::capabilities(&loaded.bundle, mode, mode == LaserMode::Co2 && manual)
                })
                .collect(),
            extent: vendor::extent(&loaded.bundle).ok(),
            error: None,
        });
        self.bundle = Some(Arc::new(loaded.bundle));
        self.bindings_error = None;
        self.revisions.files += 1;
    }

    pub(crate) fn install_bindings(&mut self, bound: Option<vendor::Bindings>) {
        self.mode = bound.as_ref().map(|b| b.mode).or(self.mode);
        self.bound = bound;
        self.accepted = None;
        self.parameter_problem = None;
        self.held = None;
        self.bindings_changed();
        if let Some(draft) = &mut self.draft {
            draft.compiled = None;
        }
        self.draft_changed();
    }

    /// The controller's units per millimetre.
    pub fn scale(&self) -> Result<i32> {
        self.machine
            .state()
            .feedback
            .map(|f| f.scale)
            .ok_or_else(|| Error::Refused("connect the machine first".into()))
    }

    /// The head's position in machine millimetres.
    pub fn position(&self) -> Result<[f64; 3]> {
        self.machine
            .state()
            .feedback
            .map(|f| f.position_mm)
            .ok_or_else(|| Error::Refused("connect the machine first".into()))
    }

    /// The bindings, once bound.
    pub fn bound(&self) -> Result<&vendor::Bindings> {
        self.bound
            .as_ref()
            .ok_or_else(|| Error::Refused("the machine is not bound; connect first".into()))
    }

    /// What a simulated plant should answer so it behaves like a machine
    /// M-Laser set up from these files: the host input rules, and the axis
    /// banks over `current` with the files' parameters and the XY travel
    /// limits the vendor's mode switch writes.
    pub fn simulated_plant(&self, current: Banks) -> Result<(Vec<Rule>, Banks)> {
        let bundle =
            self.bundle.as_ref().ok_or_else(|| Error::Refused("no machine files".into()))?;
        let bound = vendor::bind(bundle, self.mode.unwrap_or(LaserMode::Fiber), SIMULATOR_SCALE)?;
        let document = bundle
            .document(openlaser_xml::Kind::Backup)
            .or_else(|| bundle.document(openlaser_xml::Kind::Hardware))
            .ok_or_else(|| Error::Refused("no hardware file".into()))?;
        let plan = openlaser_xml::parameters::Plan::from_document(document, SIMULATOR_SCALE)?;
        let mut banks = current;
        for (axis, bank) in banks.iter_mut().enumerate() {
            let merged = plan.merge(u8::try_from(axis).unwrap_or(0), bank)?;
            bank.copy_from_slice(&merged);
        }
        for (bank, (lower, upper)) in banks.iter_mut().zip(bound.mode_switch.xy_limits) {
            bank[1] = lower.cast_unsigned();
            bank[2] = upper.cast_unsigned();
        }
        let mut rules = bindings::controller(&bound).rules;
        // The simulated wiring survives a mode change just like physical wiring.
        // Seed inactive inputs for the other supported mode as well.
        let other = match bound.mode {
            LaserMode::Fiber => LaserMode::Co2,
            LaserMode::Co2 => LaserMode::Fiber,
        };
        if let Ok(other) = vendor::bind(bundle, other, SIMULATOR_SCALE) {
            for rule in bindings::controller(&other).rules {
                if !rules.iter().any(|existing| existing.id == rule.id) {
                    rules.push(rule);
                }
            }
        }
        Ok((rules, banks))
    }

    // Library -------------------------------------------------------------------

    /// Imports a DXF or SVG as a part, preserving open paths.
    pub fn import_part(&mut self, file_name: &str, bytes: &[u8]) -> Result<PartView> {
        let import = crate::imports::part(
            file_name,
            bytes,
            &self.fonts.fonts,
            &crate::imports::ImportOptions::default(),
        )?;
        crate::imports::require_geometry(&import)?;
        self.import_drawing(file_name, bytes, import)
    }

    /// Persists an already parsed import and publishes its library entry.
    pub(crate) fn import_drawing(
        &mut self,
        file_name: &str,
        bytes: &[u8],
        import: crate::imports::Import,
    ) -> Result<PartView> {
        let notices = import.notices();
        let part = self.library.add_part(file_name, bytes, import.drawing)?;
        let view = PartView::new(&part);
        self.library_changed();
        if !notices.is_empty() {
            self.note(format!("Imported {}. {}", part.name, notices.join(" ")), false);
        }
        Ok(view)
    }

    /// Copies a part.
    pub fn duplicate_part(&mut self, id: &Id) -> Result<PartView> {
        let copy = self.library.duplicate_part(id)?;
        self.library_changed();
        Ok(PartView::new(&copy))
    }

    /// What simplifying a part's drawing did; with `save`, the result is
    /// kept as a new part beside it, and the part itself stays as it is for
    /// the jobs that use it.
    pub fn simplified_part(
        &mut self,
        id: &Id,
        result: openlaser_prep::simplify::Simplified,
        save: bool,
    ) -> Result<crate::document::SimplifyView> {
        let part = self.library.part(id)?;
        let base = format!("{} simplified", part.name.chars().take(100).collect::<String>());
        // Another simplification of the same part is numbered, not a namesake.
        let name = (1..=999)
            .map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") })
            .find(|name| !self.library.parts().any(|p| &p.name == name))
            .unwrap_or(base);
        let mut view = crate::document::SimplifyView {
            curves_before: result.curves_before,
            curves_after: result.curves_after,
            contours_before: part.drawing.contours.len(),
            contours_after: result.drawing.contours.len(),
            repeats: result.repeats,
            specks: result.specks,
            part: None,
        };
        if save {
            if !result.changed() {
                return Err(Error::Refused(
                    "the drawing is already as simple as this tolerance allows".into(),
                ));
            }
            view.part = Some(self.library.add_derived_part(id, &name, result.drawing)?.id);
            self.library_changed();
        }
        Ok(view)
    }

    /// Renames, moves or stars a part.
    pub fn update_part(&mut self, id: &Id, change: ItemChange) -> Result<()> {
        self.library.update_part(id, |part| {
            apply_change(&mut part.tags, change.tags);
            apply_change(&mut part.notes, change.notes);
            apply_change(&mut part.quantity, change.quantity);
            apply_change(&mut part.name, change.name);
            apply_change(&mut part.folder, change.folder);
            apply_change(&mut part.favourite, change.favourite);
        })?;
        self.library_changed();
        Ok(())
    }

    /// Removes a part.
    pub fn remove_part(&mut self, id: &Id) -> Result<()> {
        self.library.remove_part(id)?;
        if self.draft.as_ref().is_some_and(|d| d.current.parts.contains(id)) {
            self.draft = None;
            self.draft_changed();
        }
        self.library_changed();
        Ok(())
    }

    /// Makes a folder.
    pub fn add_folder(&mut self, name: &str, parent: Option<Id>) -> Result<Id> {
        let folder = self.library.add_folder(name, parent)?;
        self.library_changed();
        Ok(folder.id)
    }

    /// Renames or stars a folder.
    pub fn update_folder(&mut self, id: &Id, change: ItemChange) -> Result<()> {
        self.library.update_folder(id, |folder| {
            apply_change(&mut folder.name, change.name);
            apply_change(&mut folder.parent, change.folder);
            apply_change(&mut folder.favourite, change.favourite);
        })?;
        self.library_changed();
        Ok(())
    }

    /// Removes an empty folder.
    pub fn remove_folder(&mut self, id: &Id) -> Result<()> {
        self.library.remove_folder(id)?;
        self.library_changed();
        Ok(())
    }

    /// Adds a recipe: its values from a layer bank of the machine files or
    /// copied from a recipe of the same laser, with the gas selection the
    /// operator chose.
    pub fn add_recipe(&mut self, new: &NewRecipe) -> Result<RecipeView> {
        let mut recipe = match &new.values {
            Values::Bank(bank) => {
                let bundle = self
                    .bundle
                    .as_ref()
                    .ok_or_else(|| Error::Refused("no machine files".into()))?;
                recipes::from_bank(bundle, new.laser, *bank, &new.name, new.thickness_mm)?
            }
            Values::Recipe(id) => {
                let source = self.library.recipe(id)?;
                if source.laser != new.laser {
                    return Err(Error::Request("the source recipe is for the other laser".into()));
                }
                let mut copy = source.clone();
                if copy.name != new.name {
                    copy.photo = None;
                }
                copy.name.clone_from(&new.name);
                copy.thickness_mm = new.thickness_mm;
                copy.source_sha256 = None;
                copy.file_name = None;
                copy.favourite = false;
                copy.tags.clear();
                copy
            }
        };
        if let Some(gas) =
            if new.laser == LaserMode::Co2 { Some(openlaser_xml::recipe::CO2_GAS) } else { new.gas }
        {
            if gas > 5 {
                return Err(Error::Request("the gas selection is 0 to 5".into()));
            }
            recipe.attributes.insert("CutGasType".into(), gas.to_string());
            recipe.gas = recipes::gas_name(&gas.to_string());
        }
        let recipe = self.library.add_recipe(recipe)?;
        self.library_changed();
        Ok(RecipeView::new(&recipe))
    }

    /// Imports a vendor recipe file, keeping the file by its hash. A file
    /// already in the library, under any name, answers with its recipe and
    /// `true`.
    pub fn import_recipe(&mut self, file_name: &str, bytes: &[u8]) -> Result<(RecipeView, bool)> {
        if let Some(existing) = self.library.recipe_by_source(&openlaser_library::sha256(bytes)) {
            return Ok((RecipeView::new(existing), true));
        }
        self.import_recipe_as(
            &RecipeImport { name: file_name.to_owned(), ..RecipeImport::default() },
            bytes,
        )
    }

    /// What a vendor recipe file would add, without saving anything.
    pub fn preview_recipe(&self, file_name: &str, bytes: &[u8]) -> Result<recipes::RecipePreview> {
        let recipe = recipes::from_file(file_name, bytes)?;
        let sha256 = openlaser_library::sha256(bytes);
        let existing = self.library.recipe_by_source(&sha256).map(|r| r.id.clone());
        Ok(recipes::RecipePreview::new(&recipe, sha256, existing))
    }

    /// Imports a vendor recipe file as the operator reviewed it: under the
    /// material and thickness they chose, with the head setup they
    /// confirmed, as a new recipe or over the one it replaces. Without
    /// `keep_both` or `replace`, a file already imported under the same
    /// material, thickness and gas answers with that recipe and `true`.
    pub fn import_recipe_as(
        &mut self,
        options: &RecipeImport,
        bytes: &[u8],
    ) -> Result<(RecipeView, bool)> {
        let mut recipe = recipes::from_file(&options.name, bytes)?;
        if let Some(material) = &options.material {
            recipe.name.clone_from(material);
        }
        if let Some(thickness) = options.thickness_mm {
            if !(thickness.is_finite() && thickness >= 0.) {
                return Err(Error::Request("the thickness must be 0 or more".into()));
            }
            recipe.thickness_mm = thickness;
        }
        if options.setup {
            let setup = recipes::setup::HeadSetup {
                nozzle_diameter_mm: options.nozzle_diameter_mm.clone(),
                nozzle: options.nozzle,
                focus_mm: options.focus_mm.clone(),
                lens_mm: options.lens_mm.clone(),
            };
            setup.check().map_err(Error::Request)?;
            setup.apply(&mut recipe.attributes, true);
        }
        let sha256 = self.library.keep_original(bytes)?;
        if options.replace.is_none()
            && !options.keep_both
            && let Some(existing) = self
                .library
                .recipes()
                .find(|r| r.source_sha256.as_ref() == Some(&sha256) && r.key() == recipe.key())
        {
            return Ok((RecipeView::new(existing), true));
        }
        recipe.source_sha256 = Some(sha256);
        let saved = if let Some(id) = &options.replace {
            if self.library.recipe(id)?.laser != recipe.laser {
                return Err(Error::Request("the replaced recipe is for the other laser".into()));
            }
            self.library.update_recipe(id, |old| {
                let openlaser_library::Recipe { id, photo, favourite, film, created, .. } =
                    old.clone();
                *old = openlaser_library::Recipe { id, photo, favourite, film, created, ..recipe };
            })?
        } else {
            self.library.add_recipe(recipe)?
        };
        self.library_changed();
        Ok((RecipeView::new(&saved), false))
    }

    /// Keeps a sample cut photo for a recipe. The vendor's library names
    /// every photo `.png` and stores most of them as JPEG, so the bytes
    /// decide.
    pub fn set_photo(&mut self, id: &Id, bytes: &[u8]) -> Result<()> {
        if image_type(bytes).is_none() {
            return Err(Error::Request("a photo must be a PNG or JPEG".into()));
        }
        let sha256 = self.library.add_photo(bytes)?;
        self.library.update_recipe(id, |recipe| recipe.photo = Some(sha256))?;
        self.library_changed();
        Ok(())
    }

    /// A photo's bytes and their media type.
    pub fn photo(&self, sha256: &str) -> Result<(&'static str, Vec<u8>)> {
        let bytes = self.library.photo(sha256)?;
        let media_type = image_type(&bytes).unwrap_or("application/octet-stream");
        Ok((media_type, bytes))
    }

    /// Changes a recipe. The film process it refers to must be another
    /// recipe for the same laser.
    pub fn update_recipe(&mut self, id: &Id, change: RecipeChange) -> Result<()> {
        let saved = self.library.recipe(id)?;
        if let Some(expected) = &change.expected_attributes {
            for (key, value) in &change.attributes {
                let base = expected.get(key).ok_or_else(|| {
                    Error::Request("a staged attribute needs its saved base".into())
                })?;
                if saved.attributes.get(key) != base.as_ref()
                    && saved.attributes.get(key) != Some(value)
                {
                    return Err(Error::Refused(format!(
                        "{key} changed elsewhere; review pending edits"
                    )));
                }
            }
        }
        if let (Some(value), Some(base)) = (&change.film, &change.expected_film)
            && &saved.film != base
            && &saved.film != value
        {
            return Err(Error::Refused(
                "film process changed elsewhere; review pending edits".into(),
            ));
        }
        let mut attributes = self.library.recipe(id)?.attributes.clone();
        openlaser_xml::recipe::edit_attributes(&mut attributes, &change.attributes)?;
        if let Some(Some(film)) = &change.film {
            let laser = self.library.recipe(id)?.laser;
            let process = self.library.recipe(film)?;
            if process.laser != laser {
                return Err(Error::Request("the film process is for the other laser".into()));
            }
            if process.id == *id {
                return Err(Error::Request("a recipe cannot be its own film process".into()));
            }
        }
        self.library.update_recipe(id, |recipe| {
            apply_change(&mut recipe.film, change.film);
            apply_change(&mut recipe.name, change.name);
            apply_change(&mut recipe.thickness_mm, change.thickness_mm);
            apply_change(&mut recipe.gas, change.gas);
            apply_change(&mut recipe.favourite, change.favourite);
            apply_change(&mut recipe.note, change.note);
            if let Some(value) = change.attributes.get("CutGasType") {
                recipe.gas = recipes::gas_name(value);
            }
            recipe.attributes = attributes;
        })?;
        self.library_changed();
        Ok(())
    }

    /// Recipes that showed a bare selector before the selections had
    /// names take their names now.
    fn name_gases(&mut self) {
        let stale: Vec<(Id, String)> = self
            .library
            .recipes()
            .filter(|r| r.gas.starts_with("Controller gas "))
            .filter_map(|r| r.attributes.get("CutGasType").map(|s| (r.id.clone(), s.clone())))
            .collect();
        for (id, selector) in stale {
            let name = recipes::gas_name(&selector);
            if let Err(error) = self.library.update_recipe(&id, |r| r.gas = name) {
                tracing::warn!(%error, "a recipe's gas was not renamed");
            }
        }
    }

    /// Adds a thickness to a material: a copy of `id` with its values,
    /// for the operator to tune.
    pub fn duplicate_recipe(&mut self, id: &Id, thickness_mm: f64) -> Result<RecipeView> {
        let mut copy = self.library.recipe(id)?.clone();
        copy.thickness_mm = thickness_mm;
        copy.source_sha256 = None;
        copy.file_name = None;
        copy.favourite = false;
        let copy = self.library.add_recipe(copy)?;
        self.library_changed();
        Ok(RecipeView::new(&copy))
    }

    /// Adds bundled recipes once per collection, preserving saved edits and
    /// keeping deleted defaults from returning on later launches.
    pub fn seed_defaults(&mut self) {
        let marker = self.config.data_dir.join("defaults-seeded");
        if !marker.exists() && self.library.recipes().next().is_none() {
            let (name, bytes) = crate::defaults::CO2;
            if let Err(error) = self.import_recipe(name, bytes) {
                tracing::warn!(%name, %error, "a bundled recipe was not imported");
            }
            if let Err(error) = std::fs::write(&marker, b"") {
                tracing::warn!(%error, "the defaults marker was not written");
            }
        }
        let metals = self.config.data_dir.join("metal-library-2026-07-21-seeded");
        if metals.exists() {
            return;
        }
        let mut complete = true;
        for (name, bytes) in crate::defaults::METALS {
            if let Err(error) = self.import_recipe(name, bytes) {
                complete = false;
                tracing::warn!(%name, %error, "a bundled recipe was not imported");
            }
        }
        if complete && let Err(error) = std::fs::write(&metals, b"") {
            tracing::warn!(%error, "the metal library marker was not written");
        }
    }

    /// Removes a recipe.
    pub fn remove_recipe(&mut self, id: &Id) -> Result<()> {
        self.library.remove_recipe(id)?;
        self.library_changed();
        Ok(())
    }

    /// Renames, moves or stars a job.
    pub fn update_job(&mut self, id: &Id, change: ItemChange) -> Result<()> {
        self.library.update_job(id, |job| {
            apply_change(&mut job.tags, change.tags);
            apply_change(&mut job.notes, change.notes);
            apply_change(&mut job.quantity, change.quantity);
            apply_change(&mut job.name, change.name);
            apply_change(&mut job.folder, change.folder);
            apply_change(&mut job.favourite, change.favourite);
        })?;
        self.library_changed();
        Ok(())
    }

    /// Copies a saved job without changing the open draft.
    pub fn duplicate_job(&mut self, id: &Id) -> Result<JobView> {
        let copy = self.library.duplicate_job(id)?;
        self.library_changed();
        Ok(JobView::new(&copy, &self.library.job_drawing(&copy.parts)?))
    }

    /// Undoes or redoes saved library data; machine execution is independent.
    pub fn undo_saved(&mut self, back: bool, revision: u64) -> Result<()> {
        self.library.undo_saved(back, revision)?;
        if self
            .draft
            .as_ref()
            .is_some_and(|d| d.current.parts.iter().any(|p| self.library.part(p).is_err()))
        {
            self.draft = None;
            self.draft_changed();
        }
        self.library_changed();
        Ok(())
    }

    /// Removes a job.
    pub fn remove_job(&mut self, id: &Id) -> Result<()> {
        self.library.remove_job(id)?;
        self.library_changed();
        Ok(())
    }

    // Draft ---------------------------------------------------------------------

    /// Opens a saved job as the draft.
    pub fn open_job(&mut self, id: &Id) -> Result<()> {
        let job = self.library.job(id)?.clone();
        let sources = Arc::new(self.library.job_drawing(&job.parts)?);
        if job.placed.iter().any(|p| p.source >= sources.contours()) {
            return Err(Error::Refused("the job's placement does not fit its parts".into()));
        }
        let mut draft = Draft::new(sources);
        draft.saved_base = Some(job.clone());
        draft.job = Some(job.id.clone());
        draft.current.recipe = Some(job.recipe.clone());
        draft.current.film.clone_from(&job.film);
        draft.current.features = job.features.clone();
        draft.current.grouping = job.grouping.clone();
        if !job.placed.is_empty() {
            draft.current.placed.clone_from(&job.placed);
        }
        draft.current.sheet_offset = job.sheet_offset;
        draft.current.placement.clone_from(&job.placement);
        draft.current.anchor = job.anchor;
        draft.current.preflight = job.preflight.clone();
        draft.current.nesting.clone_from(&job.nesting);
        draft.current.correction.clone_from(&job.correction);
        draft.calibration = job.calibration;
        draft.current.feature_source =
            Some(FeatureSource { job: job.id.clone(), name: job.name.clone(), at: job.updated });
        self.leave_draft();
        let draft = self.retained(&format!("job-{id}"))?.unwrap_or(draft);
        self.draft_generation += 1;
        self.draft = Some(draft);
        self.reprepare();
        Ok(())
    }

    /// Chooses the draft's recipe, with the film process it refers to as
    /// it is now; the features prefill from the last job on that recipe.
    pub fn set_recipe(&mut self, id: &Id) -> Result<()> {
        let recipe = self.library.recipe(id)?.clone();
        let film = recipe.film.as_ref().and_then(|film| self.library.recipe(film).ok().cloned());
        let source = self
            .library
            .last_job_on(&recipe.key())
            .map(|job| (job.id.clone(), job.name.clone(), job.updated, job.features.reusable()));
        let dxf = self.draft.as_ref().is_some_and(|d| self.any_dxf(&d.current.parts));
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        draft.remember();
        if draft.job.is_none() && !draft.calibration && dxf {
            draft.current.correction = self.correction.active(recipe.laser);
        }
        draft.current.recipe = Some(recipe);
        draft.current.film = film;
        if let Some((job, name, at, features)) = source {
            draft.current.features = features;
            draft.current.feature_source = Some(FeatureSource { job, name, at });
        }
        self.reprepare();
        Ok(())
    }

    /// Sets the draft's features.
    pub fn set_features(&mut self, features: Features) -> Result<()> {
        let draft = self.draft_mut()?;
        draft.remember();
        draft.current.features = features;
        self.reprepare();
        Ok(())
    }

    /// Copies machining defaults; picked locations stay with the saved job.
    pub fn copy_features(&mut self, job: &Id) -> Result<()> {
        let job = self.library.job(job)?.clone();
        let draft = self.draft_mut()?;
        draft.remember();
        draft.current.features = job.features.reusable();
        draft.current.feature_source =
            Some(FeatureSource { job: job.id, name: job.name, at: job.updated });
        self.reprepare();
        Ok(())
    }

    /// The draft, or the refusal to work without one.
    fn draft_mut(&mut self) -> Result<&mut Draft> {
        self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))
    }

    /// Moves, turns or mirrors placed contours: `matrix` applied after
    /// whatever they already have; none puts them back where the drawing
    /// has them.
    pub fn transform(&mut self, contours: &[usize], matrix: Option<Transform>) -> Result<()> {
        if matrix.is_some_and(|m| !m.is_similarity()) {
            return Err(Error::Request("use a finite uniform scale, move, turn or mirror".into()));
        }
        let draft = self.draft_mut()?;
        if contours.iter().any(|&i| i >= draft.current.placed.len()) {
            return Err(Error::Request("no such contour".into()));
        }
        draft.transform(contours, matrix)?;
        self.reprepare();
        Ok(())
    }

    /// Groups or ungroups the current selection; machining joins remain atomic.
    pub fn set_grouped(&mut self, contours: &[usize], together: bool) -> Result<()> {
        let draft = self.draft_mut()?;
        if draft.preparing {
            return Err(Error::Refused("wait for the current edit to finish".into()));
        }
        if contours.is_empty() || contours.iter().any(|&i| i >= draft.current.placed.len()) {
            return Err(Error::Request("select contours to group or ungroup".into()));
        }
        draft.set_grouped(contours, together);
        self.reprepare();
        Ok(())
    }

    /// Puts drawing contours on the sheet as one more copy, each under its
    /// transform; their indices.
    pub fn add(&mut self, contours: &[(usize, Transform)]) -> Result<Vec<usize>> {
        self.add_with_leads(contours, &[])
    }

    /// Paste local lead settings without changing the original instances.
    pub fn add_with_leads(
        &mut self,
        contours: &[(usize, Transform)],
        leads: &[openlaser_core::features::LeadOverride],
    ) -> Result<Vec<usize>> {
        self.add_grouped(contours, leads, &openlaser_core::grouping::Grouping::default())
    }

    /// Pastes contours with independent lead settings and authored grouping.
    pub fn add_grouped(
        &mut self,
        contours: &[(usize, Transform)],
        leads: &[openlaser_core::features::LeadOverride],
        grouping: &openlaser_core::grouping::Grouping,
    ) -> Result<Vec<usize>> {
        grouping.validate(contours.len()).map_err(Error::Request)?;
        let draft = self.draft_mut()?;
        if contours.len()
            > openlaser_core::geometry::MAX_PLACED_CONTOURS
                .saturating_sub(draft.current.placed.len())
        {
            return Err(Error::Request("the layout exceeds 100000 contours".into()));
        }
        let count = draft.drawing()?.contours.len();
        if contours.is_empty() {
            return Err(Error::Request("nothing to add".into()));
        }
        if contours.iter().any(|&(source, _)| source >= count) {
            return Err(Error::Request("no such contour".into()));
        }
        if contours.iter().any(|(_, m)| !m.is_similarity()) {
            return Err(Error::Request("use a finite uniform scale, move, turn or mirror".into()));
        }
        if leads.iter().any(|lead| lead.location.contour >= contours.len()) {
            return Err(Error::Request("a copied lead has no contour".into()));
        }
        let added = self.draft_mut()?.add_grouped(contours, leads, grouping);
        self.reprepare();
        Ok(added)
    }

    /// Takes contours off the sheet; the features lose what lay on them.
    pub fn remove(&mut self, contours: &[usize]) -> Result<()> {
        let draft =
            self.draft.as_mut().ok_or_else(|| Error::Refused("open a part first".into()))?;
        if contours.iter().any(|&i| i >= draft.current.placed.len()) {
            return Err(Error::Request("no such contour".into()));
        }
        if (0..draft.current.placed.len()).all(|i| contours.contains(&i)) {
            return Err(Error::Refused("keep at least one contour".into()));
        }
        let drawing = draft.drawing()?.clone();
        draft.remove(&drawing, contours)?;
        draft.prune_parts();
        self.reprepare();
        Ok(())
    }

    /// Takes back the last edit to the parts, the placement or the features.
    pub fn undo(&mut self) -> Result<()> {
        self.step(true)
    }

    /// Does the last edit undone again.
    pub fn redo(&mut self) -> Result<()> {
        self.step(false)
    }

    /// Steps through the history, attaching the parts' drawing when the
    /// step changes them; a step whose parts no longer fit is taken back.
    fn step(&mut self, back: bool) -> Result<()> {
        let draft = self.draft_mut()?;
        if back { draft.undo() } else { draft.redo() }?;
        if let Err(error) = self.attach_draft() {
            let draft = self.draft_mut()?;
            if back { draft.redo() } else { draft.undo() }?;
            return Err(error);
        }
        self.reprepare();
        Ok(())
    }

    /// A tap on the drawing, snapped to a contour of the placed part.
    pub fn pick(&self, at: [f64; 2], tolerance: f64, bridging: bool) -> Result<Option<PickView>> {
        self.pick_staged(at, tolerance, bridging, None)
    }

    /// A pick against an unsaved feature session, without changing the draft.
    pub fn pick_staged(
        &self,
        at: [f64; 2],
        tolerance: f64,
        bridging: bool,
        features: Option<Features>,
    ) -> Result<Option<PickView>> {
        let draft =
            self.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        if !(at.iter().all(|v| v.is_finite()) && tolerance.is_finite() && tolerance >= 0.) {
            return Err(Error::Request("the pick is not finite".into()));
        }
        let mut staged = draft.clone();
        if let Some(features) = features {
            staged.current.features = features;
        }
        draft::pick(draft.drawing()?, &staged, at, tolerance, bridging)
    }

    /// Puts the layout's anchor point at `origin`, in machine coordinates.
    pub fn set_origin(&mut self, origin: [f64; 2]) -> Result<()> {
        self.set_origin_at(origin, None)
    }

    /// Updates the origin and optional job anchor as one undoable edit.
    pub fn set_origin_at(
        &mut self,
        origin: [f64; 2],
        anchor: Option<openlaser_library::Anchor>,
    ) -> Result<()> {
        if !origin.iter().all(|v| v.is_finite()) {
            return Err(Error::Request("the origin is not finite".into()));
        }
        let draft = self.draft_mut()?;
        let anchor = anchor.unwrap_or(draft.current.anchor);
        let bounds = crate::placement::reference_bounds(draft)
            .ok_or_else(|| Error::Refused("nothing prepared".into()))?;
        let dock = anchor.on(bounds.min.into(), bounds.max.into());
        draft.remember();
        draft.current.anchor = anchor;
        draft.current.sheet_offset = Some([origin[0] - dock[0], origin[1] - dock[1]]);
        draft.current.placement = Some(openlaser_library::placement::Placement::Fixed { origin });
        if draft.current.correction.is_some() {
            draft.compiled = None;
        }
        self.draft_changed();
        Ok(())
    }

    /// Chooses which point of the layout the origin stands for; the layout
    /// stays where it is.
    pub fn set_anchor(&mut self, anchor: openlaser_library::Anchor) -> Result<()> {
        self.draft_mut()?.remember();
        self.draft_mut()?.current.anchor = anchor;
        self.draft_changed();
        Ok(())
    }

    /// Puts the layout's anchor point where the head is now.
    pub fn origin_here(&mut self) -> Result<()> {
        self.origin_here_at(None)
    }

    /// Sets an optional anchor at the head using fresh, stationary feedback.
    pub fn origin_here_at(&mut self, anchor: Option<openlaser_library::Anchor>) -> Result<()> {
        self.idle()?;
        let state = self.machine.state();
        let feedback = state
            .feedback
            .filter(|f| f.age_ms <= 1000 && f.stationary && f.head.command == 0)
            .ok_or_else(|| {
                Error::Refused("set the origin with fresh feedback and the axes stationary".into())
            })?;
        self.set_origin_at([feedback.position_mm[0], feedback.position_mm[1]], anchor)
    }

    /// One of the nine points on the known bed bounds.
    pub fn bed_point(&self, point: openlaser_library::Anchor) -> Result<[f64; 2]> {
        let extent = self
            .extent()
            .ok_or_else(|| Error::Refused("import machine settings so the bed is known".into()))?;
        Ok(point.on([extent[0][0], extent[1][0]], [extent[0][1], extent[1][1]]))
    }

    /// Marks this revision for preparation; the HTTP workflow takes the
    /// immutable inputs out of the lock and conditionally installs the result.
    pub(crate) fn reprepare(&mut self) {
        if let Some(draft) = &mut self.draft {
            draft.preparing = true;
            draft.prepared = None;
            draft.compiled = None;
            draft.preview = None;
            draft.error = Some("preparing geometry".into());
            self.draft_changed();
        }
    }

    pub(crate) fn preparation(&self) -> Result<Option<Preparation>> {
        let Some(draft) = self.draft.as_ref().filter(|d| d.preparing) else { return Ok(None) };
        Ok(Some(Preparation {
            revision: self.revisions.draft,
            draft: draft.clone(),
            drawing: draft.drawing()?.clone(),
            extent: self.extent(),
        }))
    }

    pub(crate) fn prepared(&mut self, work: Preparation) -> Result<()> {
        self.check_draft(work.revision)?;
        self.draft = Some(work.draft);
        self.draft_changed();
        Ok(())
    }

    /// The travel of X and Y: the bindings' once connected, the backup's
    /// before.
    pub(crate) fn extent(&self) -> Option<[[f64; 2]; 2]> {
        self.bound.as_ref().map(|b| b.jog.extent).or(self.files.extent)
    }

    /// What a compile needs, taken out so the work can run off the lock:
    /// the prepared geometry, the recipe bound, the film process bound on
    /// its own when the recipe removes film, and the scale.
    pub fn compile_inputs(&mut self, dry_run: bool) -> Result<CompileInputs> {
        self.compile_serial += 1;
        let draft =
            self.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        let prepared = draft.prepared.clone().ok_or_else(|| {
            Error::Refused(draft.error.clone().unwrap_or_else(|| "nothing prepared".into()))
        })?;
        let recipe = draft
            .current
            .recipe
            .as_ref()
            .ok_or_else(|| Error::Refused("choose a material first".into()))?;
        if self.mode.is_some_and(|mode| mode != recipe.laser) {
            return Err(Error::Refused(
                "the recipe is for the other laser; switch mode first".into(),
            ));
        }
        let bundle =
            self.bundle.as_ref().ok_or_else(|| Error::Refused("no machine files".into()))?;
        let scale = self.scale().or_else(|_| {
            // Offline, plan against the files' own geometry; the run rebinds.
            openlaser_xml::recipe::machine(bundle)
                .and_then(|m| vendor::coordinate(m.counts_per_mm[0].round(), 1))
                .map_err(Error::from)
        })?;
        let manual_focus = recipe.laser == LaserMode::Co2 && self.config.co2_manual_focus;
        let binder =
            Binder { dry_run, manual_focus, film: false, scale, timeouts: self.soft.timeouts };
        let settings = draft::settings(bundle, recipe, binder)?;
        if let Some(profile) = &draft.current.correction {
            let bed = crate::correction::bed(self)?;
            if profile.bed != bed {
                return Err(Error::Refused(
                    "this job's correction was measured on a different bed".into(),
                ));
            }
        }
        let film = match (&draft.current.film, settings.with_film) {
            (Some(film), true) => {
                if film.laser != recipe.laser {
                    return Err(Error::Refused("the film process is for the other laser".into()));
                }
                Some(draft::settings(bundle, film, Binder { film: true, ..binder })?)
            }
            _ => None,
        };
        Ok(CompileInputs {
            correction: if draft.calibration {
                None
            } else {
                draft.current.sheet_offset.and(draft.current.correction.clone())
            },
            placement: draft.current.sheet_offset.unwrap_or([0., 0.]),
            stamp: self.compile_stamp(),
            configuration: self.acceptance().ok(),
            prepared,
            settings,
            film,
            scale,
        })
    }

    pub(crate) fn preflight_stamp(&self) -> String {
        format!(
            "{:?}/{}/{:?}/{:?}",
            self.compile_stamp(),
            self.operation_serial,
            self.execution.as_ref().map(|e| e.id),
            self.recovery.as_ref().map(|r| r.revision)
        )
    }

    fn compile_stamp(&self) -> CompileStamp {
        CompileStamp {
            draft: self.revisions.draft,
            binding: self.revisions.bindings,
            serial: self.compile_serial,
        }
    }

    pub(crate) fn check_compile(&self, stamp: CompileStamp) -> Result<()> {
        if self.compile_stamp() == stamp {
            Ok(())
        } else {
            Err(Error::Refused("compilation was superseded by a newer draft or settings".into()))
        }
    }

    /// Installs only the result of the latest compile over these exact inputs.
    pub fn compiled(
        &mut self,
        stamp: CompileStamp,
        result: Result<draft::CompiledJob>,
    ) -> Result<()> {
        self.check_compile(stamp)?;
        let result = result.and_then(|compiled| {
            if let Some(draft) = &self.draft
                && let Some(nesting) = &draft.current.nesting
            {
                let stock = crate::nesting::stock(draft.drawing()?, nesting)?;
                let map = draft
                    .current
                    .correction
                    .as_ref()
                    .filter(|_| draft.current.sheet_offset.is_some() && !draft.calibration)
                    .map(openlaser_correction::Map::new)
                    .transpose()
                    .map_err(|e| Error::Refused(e.to_string()))?;
                let zero = openlaser_core::geometry::Point::from(
                    draft.current.sheet_offset.unwrap_or([0., 0.]),
                );
                let physical: Vec<Vec<[f64; 2]>> = compiled
                    .view
                    .moves
                    .iter()
                    .filter(|m| m.kind != crate::document::PathKind::Travel)
                    .map(|m| {
                        m.points
                            .iter()
                            .map(|p| {
                                map.as_ref().map_or(*p, |map| {
                                    (map.forward(openlaser_core::geometry::Point::from(*p) + zero)
                                        - zero)
                                        .into()
                                })
                            })
                            .collect()
                    })
                    .collect();
                openlaser_nest::check_region_polylines(
                    &stock,
                    crate::nesting::cutouts(nesting),
                    physical.iter().map(Vec::as_slice),
                    nesting.margin(),
                )
                .map_err(|e| Error::Refused(e.to_string()))?;
            }
            Ok(compiled)
        });
        let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
        if let Some(draft) = &mut self.draft {
            match result {
                Ok(compiled) => {
                    draft.compiled = Some(Arc::new(compiled));
                    draft.error = None;
                }
                Err(error) => {
                    draft.compiled = None;
                    draft.error = Some(error.to_string());
                }
            }
        }
        self.draft_changed();
        if self.operation.is_none() && self.held.is_none() {
            self.execution_changed(None);
        }
        outcome
    }

    /// The freshly read controller configuration for manual positioning.
    /// Matching a backup is a separate requirement for compiled job geometry.
    pub(crate) fn measured_configuration(&self) -> Result<Configuration> {
        self.machine.state().configuration.ok_or_else(|| {
            Error::Refused(
                "the controller's axis parameters are unavailable; reconnect to read them".into(),
            )
        })
    }

    /// Limit recovery can precede initialization, using the measured units
    /// solely for a bounded manual pulse. This does not authorize a job.
    pub(crate) fn jog_parameters(&self) -> Result<openlaser_controller::session::Verified> {
        let state = self.machine.state();
        if state.xy_recovery {
            state
                .observed_parameters
                .ok_or_else(|| Error::Refused("read the axis parameters before recovery".into()))
        } else {
            self.measured_configuration().map(|configuration| configuration.verified)
        }
    }

    /// The matched configuration only while the controller still accepts it.
    pub fn acceptance(&self) -> Result<Configuration> {
        self.accepted
            .filter(|accepted| self.machine.state().configuration == Some(*accepted))
            .ok_or_else(|| {
                Error::Refused(self.parameter_problem.clone().unwrap_or_else(|| {
                    "read and match the axis parameters to the machine files first".into()
                }))
            })
    }

    pub(crate) fn idle(&self) -> Result<()> {
        if self.operation.is_some() || self.machine.state().operation.is_some() {
            return Err(Error::Refused("another operation is active".into()));
        }
        Ok(())
    }

    /// A held job keeps its origin, sheet record and cut history until it is
    /// resumed or stopped, so no new run or frame may replace it.
    pub(crate) fn not_held(&self) -> Result<()> {
        if self.held.is_some() {
            return Err(Error::Refused(HELD.into()));
        }
        Ok(())
    }

    /// Releases only the reservation that still owns the workflow.
    pub(crate) fn release_reservation(&mut self, owner: u64) {
        if self.operation == Some(owner) {
            self.operation = None;
            self.publish();
        }
    }

    pub(crate) fn reserve(&mut self) -> Result<u64> {
        self.idle()?;
        self.operation_serial += 1;
        self.operation = Some(self.operation_serial);
        self.publish();
        Ok(self.operation_serial)
    }
}

pub(crate) struct Preparation {
    revision: u64,
    pub draft: Draft,
    pub drawing: Arc<openlaser_core::geometry::Drawing>,
    pub extent: Option<[[f64; 2]; 2]>,
}

/// What a compile takes.
pub struct CompileInputs {
    /// Frozen correction applied after shifting into the final bed position.
    pub correction: Option<openlaser_correction::Profile>,
    /// Drawing origin in machine coordinates.
    pub placement: [f64; 2],
    /// Revision that may accept this result.
    pub stamp: CompileStamp,
    /// Live configuration, absent for an offline preview.
    pub configuration: Option<Configuration>,
    /// The prepared geometry.
    pub prepared: Arc<draft::Prepared>,
    /// The recipe as bound.
    pub settings: Settings,
    /// The film process as bound, when the recipe removes film.
    pub film: Option<Settings>,
    /// The controller's scale.
    pub scale: i32,
}

/// The media type of a PNG or JPEG, from its first bytes.
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// A rename, a move to a folder, or a star.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, optional_fields))]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ItemChange {
    /// Operator tags.
    pub tags: Option<Vec<String>>,
    /// Operator notes.
    pub notes: Option<String>,
    /// Requested production quantity.
    pub quantity: Option<u32>,
    /// The new name.
    pub name: Option<String>,
    /// The new folder; `Some(None)` moves to the root.
    #[serde(default, deserialize_with = "double_option")]
    #[allow(clippy::option_option, reason = "absent leaves alone, null clears")]
    pub folder: Option<Option<Id>>,
    /// Starred or not.
    pub favourite: Option<bool>,
}

/// An absent field preserves its saved value; a present null can clear an
/// optional value. All library metadata edits use the same patch semantics.
fn apply_change<T>(saved: &mut T, next: Option<T>) {
    if let Some(next) = next {
        *saved = next;
    }
}

/// Where a new recipe's values come from.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Values {
    /// A layer bank of the machine files, 1 to 11.
    Bank(u8),
    /// A recipe already in the library.
    Recipe(Id),
}

/// A recipe to add.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Deserialize)]
pub struct NewRecipe {
    /// The material.
    pub name: String,
    /// Which laser.
    pub laser: LaserMode,
    /// The sheet thickness.
    pub thickness_mm: f64,
    /// Where its values come from.
    pub values: Values,
    /// The gas selection, 0 to 5, when it should differ from the source's.
    pub gas: Option<u8>,
}

/// How to save an imported vendor recipe file, after the operator
/// reviewed its preview. Sent as the query of the upload.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, optional_fields))]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RecipeImport {
    /// The file's name, which the material, thickness and gas are read from.
    pub name: String,
    /// The material to file it under instead of the one in the name.
    pub material: Option<String>,
    /// The sheet thickness instead of the one in the name.
    pub thickness_mm: Option<f64>,
    /// The recipe it replaces: its values, note and file change, while its
    /// id, photo, star and film process stay.
    pub replace: Option<Id>,
    /// Adds it even when the same file is already in the library under
    /// the same material, thickness and gas.
    #[serde(default)]
    pub keep_both: bool,
    /// Whether the four setup values below are confirmed: each present one
    /// is kept and each absent one cleared. Otherwise what the file's note
    /// and names say is kept.
    #[serde(default)]
    pub setup: bool,
    /// Confirmed nozzle bore in millimetres.
    pub nozzle_diameter_mm: Option<String>,
    /// Confirmed nozzle construction.
    pub nozzle: Option<recipes::setup::NozzleKind>,
    /// Confirmed manual focus offset in millimetres.
    pub focus_mm: Option<String>,
    /// Confirmed lens focal length in millimetres.
    pub lens_mm: Option<String>,
}

/// A change to a recipe.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export, optional_fields))]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RecipeChange {
    /// Saved values when these staged attributes were first edited.
    pub expected_attributes: Option<BTreeMap<String, Option<String>>>,
    /// Saved film choice when the staged edit began.
    #[serde(default, deserialize_with = "double_option")]
    #[allow(clippy::option_option, reason = "absent leaves alone, null is an absent film")]
    pub expected_film: Option<Option<Id>>,
    /// The material.
    pub name: Option<String>,
    /// The sheet thickness.
    pub thickness_mm: Option<f64>,
    /// The assist gas.
    pub gas: Option<String>,
    /// Starred or not.
    pub favourite: Option<bool>,
    /// The note.
    pub note: Option<String>,
    /// The film process; `Some(None)` clears it.
    #[serde(default, deserialize_with = "double_option")]
    #[allow(clippy::option_option, reason = "absent leaves alone, null clears")]
    pub film: Option<Option<Id>>,
    /// Attribute values to replace.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

/// `null` clears, absence leaves alone.
#[allow(clippy::option_option, reason = "absent leaves alone, null clears")]
fn double_option<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<Id>>, D::Error> {
    Option::<Id>::deserialize(deserializer).map(Some)
}

fn bindings_view(bound: &vendor::Bindings) -> BindingsView {
    let outputs = &bound.outputs;
    let mut gas = [false; 6];
    for (selector, assigned) in gas.iter_mut().enumerate() {
        *assigned = outputs.hardware.gas_enabled && outputs.hardware.gas_ports[selector].is_some();
    }
    BindingsView {
        mode: bound.mode,
        head_enabled: bound.head_enabled,
        jog_speed: bound.jog.speed,
        table_maximum_mm: bound.table.map(|t| t.maximum_mm),
        extent: bound.jog.extent,
        outputs: OutputsView {
            pointer: outputs.pointer_port != 0,
            pointer_port: outputs.pointer_port,
            shutter: outputs.shutter_port != 0,
            shutter_port: outputs.shutter_port,
            gas,
            head_jog: outputs.head_jog_tenths.is_some(),
        },
        rules: bound
            .rules
            .iter()
            .map(|r| RuleView { id: r.id, label: r.label.clone(), input: r.input })
            .collect(),
    }
}

/// The draft view of a document.
#[must_use]
pub fn draft_view(coordinator: &Coordinator) -> Option<DraftView> {
    coordinator.draft_view.as_deref().cloned()
}
