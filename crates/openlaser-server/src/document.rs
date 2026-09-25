// SPDX-License-Identifier: GPL-3.0-or-later

//! What the server publishes: one document the UI renders from, pushed on
//! every change.

use openlaser_core::LaserMode;
use openlaser_core::features::Features;
use openlaser_core::geometry::{Bounds, Placed};
use openlaser_library::{Anchor, Folder, Id, Job, JobDrawing, Part, Recipe};
use serde::Serialize;
use std::sync::Arc;

/// The whole state, as JSON.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, Serialize)]
pub struct Document {
    /// Most recent completed sheet; history geometry loads on demand.
    pub completed_sheet: Option<String>,
    /// Retained original execution and recovery choices.
    pub recovery: Option<Arc<crate::resume::RecoveryView>>,
    /// Retained authoring edits that could not be written to disk.
    pub persistence_error: Option<String>,
    /// Revision of persisted preflight defaults.
    pub preflight_revision: u64,
    /// Pending checklist for the last successfully completed job.
    pub postflight: Option<crate::postflight::PostflightNotice>,
    /// Alarm recording health; historical rows load only when requested.
    pub alarm_history: crate::alarm_history::HistoryStatus,
    /// Active host process overrides.
    pub soft: crate::soft_settings::SoftView,
    /// How long held controls must be held, the same on every screen.
    pub hold: crate::touch::HoldTimes,
    /// Sheet sizes saved for the stock chooser.
    pub sheet_sizes: Vec<crate::sheet_sizes::SheetSize>,
    /// Gas prices and the current job's estimate.
    pub gas: Arc<crate::gas::GasView>,
    /// Changes with every publication.
    pub revision: u64,
    /// The controller task's state.
    pub machine: openlaser_controller::State,
    /// Whether head calibration applies to the current material.
    pub calibration: crate::setup::CalibrationView,
    /// The operating mode the host is set to.
    pub mode: Option<LaserMode>,
    /// The machine files, once bound.
    pub bindings: Option<Arc<BindingsView>>,
    /// Why the machine files could not be bound.
    pub bindings_error: Option<String>,
    /// The machine files.
    pub files: Arc<FilesView>,
    /// The library.
    pub library: Arc<LibraryView>,
    /// The job being set up.
    pub draft: Option<Arc<DraftView>>,
    /// Changes even when the draft is cleared.
    pub draft_revision: u64,
    /// The immutable view of the last admitted program.
    pub execution: Option<Arc<ExecutionView>>,
    /// Progress interpreted against that program's pass identities.
    pub progress: Option<ProgressView>,
    /// Whether a held program can be resumed.
    pub can_resume: bool,
    /// Which commands the server would accept now, and why not otherwise.
    pub readiness: Readiness,
    /// The last thing worth telling the operator.
    pub message: Option<Message>,
    /// The computer's side of the machine link.
    pub link: LinkView,
    /// The revisions of the big sections, published with the document
    /// they describe so a stream never pairs one with another's.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub revisions: Revisions,
}

/// Change counters for the document's big sections.
///
/// Each counter only ever grows, and grows whenever its section is rebuilt.
/// The coordinator owns them (`Coordinator::revisions`) and publishes a copy
/// with every [`Document`]. Readers compare two copies for equality; the
/// numbers themselves mean nothing beyond "changed or not", except the draft
/// counter, which clients also send back to name the draft they edited.
///
/// Readers common to all counters: the event stream in `api.rs` remembers
/// the copy it last sent, and `Patch` leaves out every section whose
/// counter is unchanged since then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Revisions {
    /// Counts rebuilds of the recovery view: the original execution's
    /// progress and the operator's recovery choices. Bumped by
    /// `Coordinator::publish` when the recovery's own revision or execution
    /// id differs from the view last built. Read only by `Patch`.
    pub recovery: u64,
    /// Counts rebuilds of the library view: parts, recipes, jobs, folders.
    /// Bumped by `Coordinator::library_changed` after any library edit.
    /// Read only by `Patch`.
    pub library: u64,
    /// Counts changes of the draft: every edit, preparation, compilation and
    /// open or close. Bumped by `Coordinator::draft_changed`, and seeded at
    /// start-up from the wall clock in milliseconds so a client holding a
    /// number from an earlier server run never matches a new draft. Read by
    /// `Patch`; sent as [`Document::draft_revision`] and
    /// [`DraftView::revision`]; checked by `Coordinator::check_draft` to
    /// refuse edits made against an older draft; part of the compile stamp
    /// that discards superseded compiles; and carried by background
    /// preparation so its result is dropped if the draft moved on.
    pub draft: u64,
    /// Counts changes of the bound controller settings. Bumped by
    /// `Coordinator::bindings_changed` on connect, disconnect and import.
    /// Read by `Patch` and part of the compile stamp, so a compile made
    /// under older bindings is discarded.
    pub bindings: u64,
    /// Counts changes of the imported machine files view. Bumped when the
    /// files are installed or fail to load (`install_files`, `load_files`).
    /// Read only by `Patch`.
    pub files: u64,
    /// Counts changes of the last admitted program's view. Bumped by
    /// `Coordinator::execution_changed` when a run or frame is admitted or
    /// its view is cleared. Read only by `Patch`.
    pub execution: u64,
}

/// A borrowed stream patch. Unchanged sections are never traversed by serde.
pub(crate) struct Patch<'a> {
    pub document: &'a Document,
    pub previous: Option<Revisions>,
}

impl Serialize for Patch<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let d = self.document;
        let mut map = serializer.serialize_map(None)?;
        if self.previous.is_none_or(|p| p.recovery != d.revisions.recovery) {
            map.serialize_entry("recovery", &d.recovery)?;
        }
        map.serialize_entry("revision", &d.revision)?;
        map.serialize_entry("persistence_error", &d.persistence_error)?;
        map.serialize_entry("preflight_revision", &d.preflight_revision)?;
        map.serialize_entry("postflight", &d.postflight)?;
        map.serialize_entry("completed_sheet", &d.completed_sheet)?;
        map.serialize_entry("alarm_history", &d.alarm_history)?;
        map.serialize_entry("soft", &d.soft)?;
        map.serialize_entry("hold", &d.hold)?;
        map.serialize_entry("sheet_sizes", &d.sheet_sizes)?;
        map.serialize_entry("gas", &d.gas)?;
        map.serialize_entry("machine", &d.machine)?;
        map.serialize_entry("calibration", &d.calibration)?;
        map.serialize_entry("mode", &d.mode)?;
        map.serialize_entry("bindings_error", &d.bindings_error)?;
        map.serialize_entry("can_resume", &d.can_resume)?;
        map.serialize_entry("readiness", &d.readiness)?;
        map.serialize_entry("message", &d.message)?;
        map.serialize_entry("link", &d.link)?;
        map.serialize_entry("progress", &d.progress)?;
        map.serialize_entry("draft_revision", &d.draft_revision)?;
        if self.previous.is_none_or(|p| p.bindings != d.revisions.bindings) {
            map.serialize_entry("bindings", &d.bindings)?;
        }
        if self.previous.is_none_or(|p| p.files != d.revisions.files) {
            map.serialize_entry("files", &d.files)?;
        }
        if self.previous.is_none_or(|p| p.library != d.revisions.library) {
            map.serialize_entry("library", &d.library)?;
        }
        if self.previous.is_none_or(|p| p.draft != d.revisions.draft) {
            map.serialize_entry("draft", &d.draft)?;
        }
        if self.previous.is_none_or(|p| p.execution != d.revisions.execution) {
            map.serialize_entry("execution", &d.execution)?;
        }
        map.end()
    }
}

/// Whether one command would be accepted.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Gate {
    /// Whether it would.
    pub ok: bool,
    /// Why not, when not.
    pub reason: Option<String>,
}

impl Gate {
    /// Allowed.
    #[must_use]
    pub const fn open() -> Self {
        Self { ok: true, reason: None }
    }

    /// Refused for `reason`.
    #[must_use]
    pub fn closed(reason: impl Into<String>) -> Self {
        Self { ok: false, reason: Some(reason.into()) }
    }
}

/// The gates the UI enables its controls from. The server checks again
/// when the command arrives.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Readiness {
    /// Connect to the controller.
    pub connect: Gate,
    /// Go Origin.
    pub home: Gate,
    /// Head calibration.
    pub calibrate: Gate,
    /// Manual X/Y or configured table jogging using the controller's limits.
    pub jog: Gate,
    /// X then Y, each negative then positive, including limit recovery.
    pub xy_jog: [[Gate; 2]; 2],
    /// X/Y presses are bounded to 1 mm at no more than 1 mm/s during recovery.
    pub xy_recovery: bool,
    /// Manual Z up, including recovery from a lower limit.
    pub head_up: Gate,
    /// Manual Z down, including recovery from an upper limit.
    pub head_down: Gate,
    /// Z presses are bounded to 1 mm at no more than 1 mm/s during recovery.
    pub head_recovery: bool,
    /// Move to an absolute job origin or bed point under matched parameters.
    pub position: Gate,
    /// Hold a manual output.
    pub outputs: Gate,
    /// Set the job origin where the head is.
    pub set_origin: Gate,
    /// Compile the draft.
    pub compile: Gate,
    /// Run the compiled draft.
    pub run: Gate,
    /// Resume the held program.
    pub resume: Gate,
    /// Hold the running program.
    pub hold: Gate,
    /// Stop whatever runs.
    pub stop: Gate,
    /// Switch the laser mode.
    pub mode: Gate,
    /// Trace the job's bounds with the laser off.
    pub frame: Gate,
}

/// A note for the operator, with when it was raised.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Message {
    /// Unique publication id, including notes raised within the same second.
    pub id: u64,
    /// The text.
    pub text: String,
    /// Whether it reports a failure.
    pub error: bool,
    /// Seconds since the epoch.
    pub at: u64,
}

/// The computer's side of the machine link: where the connect workflow
/// stands and the route it uses.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkView {
    /// The step in progress, or how the last attempt ended.
    pub phase: LinkPhase,
    /// What the step is doing, or why the attempt failed.
    pub detail: String,
    /// The adapter towards the machine, once found.
    pub adapter: Option<AdapterView>,
    /// The address spoken from, once known.
    pub host: Option<String>,
    /// The computer's address for the adapter, `ip/prefix`.
    pub computer: String,
    /// The controller's endpoint, `ip:port`.
    pub controller: String,
    /// Adapter selected for the next connection; empty means automatic.
    pub remembered_adapter: String,
    /// The adapters to choose from when more than one could be the
    /// machine's.
    pub choices: Vec<AdapterView>,
}

/// A step of the connect workflow.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkPhase {
    /// No attempt in progress; the connection state says the rest.
    #[default]
    Idle,
    /// Reading the computer's adapters.
    Inspecting,
    /// Giving the adapter its address, with the system's prompt.
    Configuring,
    /// Reaching the controller.
    Connecting,
    /// Binding the machine files and reading the parameters back.
    Reading,
    /// The last attempt failed; the detail says why.
    Failed,
}

/// An adapter as the operator sees it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdapterView {
    /// The interface name.
    pub name: String,
    /// What the computer calls the hardware.
    pub description: String,
    /// Its addresses.
    pub addresses: Vec<String>,
}

/// The bound machine, as the UI needs it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BindingsView {
    /// The laser the bindings are for.
    pub mode: LaserMode,
    /// Whether a head controller is configured.
    pub head_enabled: bool,
    /// Slow and fast jog speeds in millimetres per second.
    pub jog_speed: [f64; 2],
    /// The lifting table's positive travel limit, when that branch is configured.
    pub table_maximum_mm: Option<f64>,
    /// The travel of X and Y in millimetres, lower then upper.
    pub extent: [[f64; 2]; 2],
    /// Which manual outputs are assigned.
    pub outputs: OutputsView,
    /// The host input rules, for the alarm list.
    pub rules: Vec<RuleView>,
}

/// Which manual outputs the machine has.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OutputsView {
    /// A red pointer.
    pub pointer: bool,
    /// Selected source's red-light output, zero when unassigned.
    pub pointer_port: u8,
    /// A shutter or laser gate.
    pub shutter: bool,
    /// Selected source's gate output, zero when unassigned.
    pub shutter_port: u8,
    /// Gas selectors with a valve: low air, oxygen, nitrogen, high air,
    /// oxygen, nitrogen.
    pub gas: [bool; 6],
    /// A head that can be jogged.
    pub head_jog: bool,
}

/// One host input rule.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleView {
    /// The vendor's id.
    pub id: u32,
    /// The label.
    pub label: String,
    /// The input.
    pub input: u8,
}

/// The machine files as loaded, and the layer banks they hold.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FilesView {
    /// The machine backup, once read.
    pub backup: Option<FileView>,
    /// Why it could not be read.
    pub error: Option<String>,
    /// The layer banks it holds.
    pub banks: Vec<BankView>,
    /// The gas selections with a valve, 0 to 5.
    pub gases: Vec<u8>,
    /// What each laser's process can do, for the recipe editor.
    pub capabilities: Vec<CapabilitiesView>,
    /// The travel of X and Y in millimetres, lower then upper, as the
    /// backup gives it; the bed is drawn from this before connecting.
    pub extent: Option<[[f64; 2]; 2]>,
}

/// What one laser's process can do on this machine, read from the files:
/// the recipe editor offers only what is wired, and shows the operating
/// settings the passes depend on.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapabilitiesView {
    /// Which laser.
    pub laser: LaserMode,
    /// Whether the head's height is controlled: nozzle gap, piercing,
    /// cleaning and following need it.
    pub height_control: bool,
    /// The six gas selections, low air, oxygen, nitrogen, then high.
    pub gases: Vec<GasRouteView>,
    /// Whether the laser takes a peak output level.
    pub peak_output: bool,
    /// How many contours a pre-pierce batch takes (`SoftParam.GP.PreDrillMaxNum`).
    pub pre_pierce_batch: Option<u32>,
    /// How many contours a film batch takes; zero is the whole job
    /// (`ManuParam.MC.ClearUpFilmNum_Pre`).
    pub film_batch: Option<u32>,
    /// The short-transfer distance in millimetres (`ManuParam.FC.ShortNoUpMaxLength`).
    pub short_transfer_mm: Option<f64>,
    /// The gas delays in milliseconds: after a switch, before the first
    /// gas, and on a change.
    pub gas_delays_ms: [u32; 3],
}

/// One gas selection of the machine.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GasRouteView {
    /// The selector, 0 to 5.
    pub selector: u8,
    /// Its name.
    pub name: String,
    /// Whether a valve is wired to it.
    pub valve: bool,
    /// Whether its pressure is set electronically, through a proportional
    /// output; otherwise it is set at the regulator.
    pub pressure: bool,
}

/// One machine file.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileView {
    /// The file name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Its hash.
    pub sha256: String,
}

/// One layer bank of the machine files.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BankView {
    /// Which laser.
    pub laser: LaserMode,
    /// The bank, 1 to 11.
    pub bank: u8,
    /// The vendor's `LayerFileName`, often the material it was saved from.
    pub name: String,
    /// Whether the vendor disabled it.
    pub disabled: bool,
    /// Its headline values.
    pub summary: RecipeSummary,
}

/// The library, summarised.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, Serialize)]
pub struct LibraryView {
    /// Every folder.
    pub folders: Vec<Folder>,
    /// Every part.
    pub parts: Vec<PartView>,
    /// Every recipe.
    pub recipes: Vec<RecipeView>,
    /// Every job.
    pub jobs: Vec<JobView>,
    /// Full sheets on the rack.
    pub stock: Vec<crate::inventory::StockItem>,
    /// Remnants ready to nest on.
    pub remnants: Vec<crate::stock_store::SheetView>,
    /// Library files left out when it opened, untouched on disk.
    pub skipped: Vec<SkippedView>,
}

/// A library file that could not be loaded, and why.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkippedView {
    /// The file, relative to the data directory.
    pub file: String,
    /// Why it was left out.
    pub reason: String,
}

/// A part without its geometry.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PartView {
    /// Searchable operator tags.
    pub tags: Vec<String>,
    /// Operator notes.
    pub notes: String,
    /// Requested production quantity.
    pub quantity: u32,
    /// Its id.
    pub id: Id,
    /// Its name.
    pub name: String,
    /// Its folder.
    pub folder: Option<Id>,
    /// The file it came from.
    pub file_name: String,
    /// Its extent.
    pub bounds: Option<Bounds>,
    /// How many contours it has.
    pub contours: usize,
    /// A thumbnail: the contours as polylines in drawing coordinates.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub outline: Vec<Vec<[f64; 2]>>,
    /// The drawing's layers and how many contours each holds.
    pub layers: Vec<LayerView>,
    /// Whether it is starred.
    pub favourite: bool,
    /// When it last changed.
    pub updated: u64,
}

/// One drawing layer.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LayerView {
    /// Its name in the drawing.
    pub name: String,
    /// How many contours lie on it.
    pub contours: usize,
}

impl PartView {
    pub(crate) fn new(part: &Part) -> Self {
        let mut layers: Vec<LayerView> = Vec::new();
        for contour in &part.drawing.contours {
            match layers.iter_mut().find(|l| l.name == contour.layer) {
                Some(layer) => layer.contours += 1,
                None => layers.push(LayerView { name: contour.layer.clone(), contours: 1 }),
            }
        }
        Self {
            tags: part.tags.clone(),
            notes: part.notes.clone(),
            quantity: part.quantity,
            id: part.id.clone(),
            name: part.name.clone(),
            folder: part.folder.clone(),
            file_name: part.file_name.clone(),
            bounds: part.bounds(),
            contours: part.drawing.contours.len(),
            outline: crate::display_path::thumbnail(&crate::draft::outline(&part.drawing, 64)),
            layers,
            favourite: part.favourite,
            updated: part.updated,
        }
    }
}

/// A tap on the drawing snapped to a contour.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PickView {
    /// The contour and the fraction along it, as the features refer to it.
    pub spot: openlaser_core::features::Spot,
    /// The point on the contour, in drawing coordinates.
    pub point: [f64; 2],
    /// The placed contour whose local coordinates hold `point`.
    pub owner: usize,
}

/// A recipe with the fields the operator reads off a card.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecipeView {
    /// Its id.
    pub id: Id,
    /// The material.
    pub name: String,
    /// Which laser.
    pub laser: LaserMode,
    /// The sheet thickness.
    pub thickness_mm: f64,
    /// The assist gas.
    pub gas: String,
    /// The vendor layer bank.
    pub layer: u8,
    /// The key jobs share features by.
    pub key: String,
    /// The recipe the film pass runs with, when film removal is on.
    pub film: Option<Id>,
    /// The headline values every screen summarises the recipe with.
    pub summary: RecipeSummary,
    /// Every attribute, for the editor.
    pub attributes: std::collections::BTreeMap<String, String>,
    /// The vendor's note.
    pub note: String,
    /// The process words from the vendor's file name.
    pub tags: Vec<String>,
    /// The file it was imported from.
    pub file_name: Option<String>,
    /// The hash of its sample cut photo, served at `/api/photos/{hash}`.
    pub photo: Option<String>,
    /// Whether it is starred.
    pub favourite: bool,
    /// When it last changed.
    pub updated: u64,
}

/// The headline process values of a recipe, as the vendor stores them: the
/// one summary every screen shows. `peak` is the laser's power setting and
/// `duty` the part of each pulse period it is on; the interface names them
/// Power and Duty (see `ui/DESIGN.md`).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RecipeSummary {
    /// `CutSpeed` in millimetres per second.
    pub speed: Option<String>,
    /// Peak power in percent: `CutPeakCurrent`.
    pub peak: Option<String>,
    /// Duty cycle in percent: `CutPower` on a fiber laser, `CutDuty` on CO2.
    pub duty: Option<String>,
    /// `CutFreq` in hertz.
    pub frequency: Option<String>,
    /// `CutGasType`, the gas selection 0 to 5.
    pub gas: Option<String>,
    /// `CutAirPressure` in bar.
    pub pressure: Option<String>,
    /// `CutHeight`, the nozzle gap while cutting, in millimetres.
    pub height: Option<String>,
    /// The nozzle, focus and lens the head is set up with.
    pub setup: crate::recipes::setup::HeadSetup,
    /// The ordinary piercing stages the machining type runs, 0 for none.
    pub pierce_stages: u8,
    /// Whether smooth piercing replaces the stages.
    pub smooth_pierce: bool,
}

impl RecipeSummary {
    /// The headline values of a layer bank for `laser`.
    #[must_use]
    pub fn of(attributes: &std::collections::BTreeMap<String, String>, laser: LaserMode) -> Self {
        let get = |key: &str| attributes.get(key).cloned();
        let (duty, other) =
            if laser == LaserMode::Co2 { ("CutDuty", "CutPower") } else { ("CutPower", "CutDuty") };
        let kind =
            attributes.get("ManuType").and_then(|v| v.trim().parse::<u8>().ok()).unwrap_or(0);
        Self {
            speed: get("CutSpeed"),
            peak: get("CutPeakCurrent"),
            duty: get(duty).or_else(|| get(other)),
            frequency: get("CutFreq"),
            gas: get("CutGasType"),
            pressure: get("CutAirPressure"),
            height: get("CutHeight"),
            setup: crate::recipes::setup::HeadSetup::of(attributes),
            pierce_stages: openlaser_xml::recipe::MACHINING_KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map_or(0, |(_, stages)| *stages),
            smooth_pierce: attributes.get("EnableSmoothPierce").is_some_and(|v| v.trim() == "1"),
        }
    }

    /// A library recipe's summary: CO2 always cuts with High Air, whose
    /// pressure is not set electronically.
    #[must_use]
    pub fn of_recipe(recipe: &Recipe) -> Self {
        let mut summary = Self::of(&recipe.attributes, recipe.laser);
        if recipe.laser == LaserMode::Co2 {
            summary.gas = Some(openlaser_xml::recipe::CO2_GAS.to_string());
            summary.pressure = None;
        }
        summary
    }
}

impl RecipeView {
    pub(crate) fn new(recipe: &Recipe) -> Self {
        let summary = RecipeSummary::of_recipe(recipe);
        let gas = if recipe.laser == LaserMode::Co2 {
            crate::recipes::GAS[3].to_owned()
        } else {
            recipe.gas.clone()
        };
        Self {
            id: recipe.id.clone(),
            name: recipe.name.clone(),
            laser: recipe.laser,
            thickness_mm: recipe.thickness_mm,
            gas,
            layer: recipe.layer,
            key: recipe.key().encoded(),
            film: recipe.film.clone(),
            summary,
            attributes: recipe.attributes.clone(),
            note: recipe.note.clone(),
            tags: recipe.tags.clone(),
            file_name: recipe.file_name.clone(),
            photo: recipe.photo.clone(),
            favourite: recipe.favourite,
            updated: recipe.updated,
        }
    }
}

/// A saved job.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JobView {
    /// Membership in a saved sheet set.
    pub sheet: Option<openlaser_library::SheetInfo>,
    /// Extent of the saved placements, including scaling and removed contours.
    pub bounds: Option<Bounds>,
    /// Thumbnail of the saved placements.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub outline: Vec<Vec<[f64; 2]>>,
    /// Number of saved placed contours.
    pub contours: usize,
    /// Searchable operator tags.
    pub tags: Vec<String>,
    /// Operator notes.
    pub notes: String,
    /// Requested production quantity.
    pub quantity: u32,
    /// Its id.
    pub id: Id,
    /// Its name.
    pub name: String,
    /// Its folder.
    pub folder: Option<Id>,
    /// The parts it cuts, in the order of its drawing.
    pub parts: Vec<Id>,
    /// The recipe it was saved with.
    pub recipe: RecipeView,
    /// Which features are on.
    pub features_on: Vec<String>,
    /// Whether it is starred.
    pub favourite: bool,
    /// When it last changed.
    pub updated: u64,
}

impl JobView {
    /// The job, drawn from `drawing`, its parts' joined drawing.
    pub(crate) fn new(job: &Job, drawing: &JobDrawing) -> Self {
        let drawing = if job.placed.is_empty() {
            drawing.drawing().clone()
        } else {
            Arc::new(crate::draft::place(drawing.drawing(), &job.placed))
        };
        Self {
            sheet: job.sheet.clone(),
            bounds: drawing.bounds(),
            outline: crate::display_path::thumbnail(&crate::draft::outline(&drawing, 64)),
            contours: drawing.contours.len(),
            tags: job.tags.clone(),
            notes: job.notes.clone(),
            quantity: job.quantity,
            id: job.id.clone(),
            name: job.name.clone(),
            folder: job.folder.clone(),
            parts: job.parts.clone(),
            recipe: RecipeView::new(&job.recipe),
            features_on: features_on(&job.features),
            favourite: job.favourite,
            updated: job.updated,
        }
    }
}

/// The names of the features switched on.
#[must_use]
pub fn features_on(features: &Features) -> Vec<String> {
    let mut on = Vec::new();
    if features.leads.is_some() {
        on.push("leads".into());
    }
    if features.joints.is_some() {
        on.push("joints".into());
    }
    if features.cooling.is_some() {
        on.push("cooling".into());
    }
    if features.kerf.is_some() {
        on.push("kerf".into());
    }
    if features.bridges.is_some() {
        on.push("bridges".into());
    }
    if features.common.is_some() {
        on.push("common".into());
    }
    on
}

/// Where the features of a draft came from.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct FeatureSource {
    /// The job they were copied from.
    pub job: Id,
    /// Its name.
    pub name: String,
    /// When that job was saved.
    pub at: u64,
}

/// The job being set up.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DraftView {
    /// Working-copy identity, distinct from the source part.
    pub key: String,
    /// Selected run type, including while its preview is rebuilding.
    pub dry_run: bool,
    /// Saved positioning method and the current run's placement state.
    pub placement: crate::placement::PlacementView,
    /// Numbered sheets available without duplicating their canvas paths.
    pub sheets: Option<crate::sheets::SheetNavigation>,
    /// Frozen matrix correction, if applied to this job.
    pub correction: Option<openlaser_correction::Profile>,
    /// Nominal, uncorrected calibration coupons.
    pub calibration: bool,
    /// Stock reference polyline, excluded from cutting and hit selection.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub stock_outline: Vec<[f64; 2]>,
    /// Previously removed material in the stock.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub stock_cutouts: Vec<Vec<[f64; 2]>>,
    /// Stock reference and the last applied nesting settings.
    pub nesting: Option<openlaser_core::nesting::Nesting>,
    /// Operator checklist policy.
    pub preflight: openlaser_library::preflight::JobPreflight,
    /// Identifies this opening of a part or saved job, across its edits.
    pub generation: u64,
    /// Identity required when editing or picking geometry.
    pub revision: u64,
    /// The saved job's name, or the name of what it cuts.
    pub name: String,
    /// The parts it cuts, in the order of its drawing.
    pub parts: Vec<DraftPart>,
    /// The saved job it was opened from, if any.
    pub job: Option<Id>,
    /// The recipe, once chosen.
    pub recipe: Option<RecipeView>,
    /// The film process the recipe refers to, as snapshotted with it.
    pub film: Option<RecipeView>,
    /// The machining features.
    pub features: Features,
    /// Where they came from.
    pub feature_source: Option<FeatureSource>,
    /// The drawing's contours on the sheet, copies and all.
    pub placed: Vec<Placed>,
    /// The contours that move together: an outline with what it encloses,
    /// joined across bridges.
    pub groups: Vec<Vec<usize>>,
    /// The origin in machine coordinates: where the anchor point lies,
    /// once the layout is prepared.
    pub origin: Option<[f64; 2]>,
    /// The sheet offset: what the machine adds to a drawing coordinate,
    /// once the sheet is placed. Sent as `zero`.
    #[serde(rename = "zero")]
    pub sheet_offset: Option<[f64; 2]>,
    /// Which point of the placed part the origin stands for.
    pub anchor: Anchor,
    /// The anchor point in drawing coordinates, once prepared.
    pub dock: Option<[f64; 2]>,
    /// How many edits can be undone.
    pub past: usize,
    /// How many undone edits can be redone.
    pub future: usize,
    /// The prepared toolpath, for the canvas.
    pub preview: Option<Arc<Preview>>,
    /// The compiled program, once compiled. Sent without its moves, which
    /// are megabytes on a large job and change with every edit: the run
    /// page fetches them for the revision it shows.
    #[serde(serialize_with = "summary")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<CompiledSummary<'_>>"))]
    pub compiled: Option<Arc<Compiled>>,
    /// Why preparation or compilation failed.
    pub error: Option<String>,
    /// The job's layers, in the order the drawing first uses them.
    pub layers: Vec<crate::layers::DraftLayer>,
}

/// What simplifying a part's drawing does, and the part it made once saved.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SimplifyView {
    /// Lines and arcs before.
    pub curves_before: usize,
    /// Lines and arcs after.
    pub curves_after: usize,
    /// Contours before.
    pub contours_before: usize,
    /// Contours after.
    pub contours_after: usize,
    /// Contours dropped for repeating another on the same layer.
    pub repeats: usize,
    /// Contours dropped for being smaller than the tolerance.
    pub specks: usize,
    /// The simplified part, once saved.
    pub part: Option<Id>,
}

/// One part of the job being set up and its contours in the job's drawing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DraftPart {
    /// The library part.
    pub id: Id,
    /// Its first contour: placed contours with sources from `first` to
    /// `first + contours` are this part's.
    pub first: usize,
    /// How many contours its drawing has.
    pub contours: usize,
}

/// The prepared toolpath as polylines.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Preview {
    /// The contours in cutting order, as polylines in drawing coordinates.
    pub contours: Vec<PreviewContour>,
    /// The extent of the placed part.
    pub bounds: Option<Bounds>,
    /// Total cut length in millimetres.
    pub length_mm: f64,
    /// Preparation warnings.
    pub warnings: Vec<String>,
}

/// One prepared contour.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PreviewContour {
    /// Stable placed-source location for this contour's lead edits.
    pub lead_target: Option<openlaser_core::features::Spot>,
    /// Corresponding contour in repeated parts, before placement transforms.
    pub matching_contour: Option<usize>,
    /// Whether it is closed.
    pub closed: bool,
    /// The nesting depth: zero for an outer contour.
    pub depth: usize,
    /// The drawing layer it came from.
    pub layer: String,
    /// The drawing contours it was made from.
    pub sources: Vec<usize>,
    /// The path in cutting order, split where the process changes.
    pub paths: Vec<Path>,
    /// Where the cooling stops are.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub cooling: Vec<[f64; 2]>,
    /// The start of the cut, after the lead-in.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub start: [f64; 2],
}

/// A polyline and the process it is cut with.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Path {
    /// What the laser does along it.
    pub kind: PathKind,
    /// The polyline in drawing coordinates.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub points: Vec<[f64; 2]>,
}

/// What a path does, for the canvas layers.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// A rapid move between contours.
    Travel,
    /// Cutting.
    Cut,
    /// A film pass.
    Film,
    /// The lead-in.
    LeadIn,
    /// The lead-out.
    LeadOut,
    /// A micro-joint.
    Joint,
    /// A piece that begins with a cooling stop.
    Cooling,
    /// The residue cleaning spiral.
    Cleaning,
}

/// The compiled program, summarised.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Compiled {
    /// Whether it is a dry run.
    pub dry_run: bool,
    /// The predictable duration in seconds.
    pub seconds: f64,
    /// Every pass, including its original identity after a continuation.
    pub plan: Vec<PassView>,
    /// Where each pass pierces or starts, in drawing coordinates.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub pierces: Vec<[f64; 2]>,
    /// How many upload blocks.
    pub blocks: usize,
    /// The travel and cut moves for the run page, in drawing coordinates.
    pub moves: Vec<Move>,
    /// Laser time, gas time, pierces and cut length of one run.
    pub usage: crate::gas::Usage,
    /// The same split by pass, to count what a stopped run spent.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub pass_usage: Arc<crate::gas::JobUsage>,
}

/// The compiled program without its moves, as the draft sends it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Serialize)]
pub struct CompiledSummary<'a> {
    /// Whether it is a dry run.
    pub dry_run: bool,
    /// The predictable duration in seconds.
    pub seconds: f64,
    /// Every pass, including its original identity after a continuation.
    pub plan: &'a [PassView],
    /// Where each pass pierces or starts, in drawing coordinates.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub pierces: &'a [[f64; 2]],
    /// How many upload blocks.
    pub blocks: usize,
    /// Laser time, gas time, pierces and cut length of one run.
    pub usage: &'a crate::gas::Usage,
}

impl<'a> From<&'a Compiled> for CompiledSummary<'a> {
    fn from(compiled: &'a Compiled) -> Self {
        Self {
            dry_run: compiled.dry_run,
            seconds: compiled.seconds,
            plan: &compiled.plan,
            pierces: &compiled.pierces,
            blocks: compiled.blocks,
            usage: &compiled.usage,
        }
    }
}

#[allow(clippy::ref_option, reason = "serde's serialize_with takes the field by reference")]
fn summary<S: serde::Serializer>(
    compiled: &Option<Arc<Compiled>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    compiled.as_deref().map(CompiledSummary::from).serialize(serializer)
}

/// A physical contour instance before feature preparation.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct InstanceView {
    /// The contour in the source drawing.
    pub source: usize,
    /// The placed copy of that drawing.
    pub copy: u32,
}

/// One process in the admitted pass plan.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PassView {
    /// Its ordinal in the original job, preserved through continuation.
    pub ordinal: usize,
    /// What the process does.
    pub kind: PassKind,
    /// The physical instances from which this prepared path was made.
    pub instances: Vec<InstanceView>,
    /// Requested cooling points omitted by the native endpoint tolerance.
    pub omitted_cooling: usize,
}

/// The processes counted separately by execution progress.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PassKind {
    /// A preliminary point operation, without a cutting traversal.
    PrePierce,
    /// Film removal under its independently bound recipe.
    Film,
    /// A cutting pass.
    Cut,
}

/// The view belongs to the admitted operation, even if the draft changes.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecutionView {
    /// Admission identity.
    pub id: u64,
    /// Whether this is a frame, whose motion never turns on the laser.
    pub frame: bool,
    /// The part or saved job being executed.
    pub name: String,
    /// The material snapshot used by this program.
    pub material: Option<MaterialView>,
    /// The selected job anchor in machine coordinates at admission.
    pub origin: [f64; 2],
    /// The sheet offset captured at admission: what the machine adds to a
    /// drawing coordinate. Sent as `zero`.
    #[serde(rename = "zero")]
    pub sheet_offset: [f64; 2],
    /// The cut job's preview and pass identities. Framing keeps this same
    /// geometry visible while its separate laser-off motion runs.
    pub compiled: Arc<Compiled>,
}

/// The material identity shown beside an immutable execution.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MaterialView {
    /// The material name.
    pub name: String,
    /// Sheet thickness in millimetres.
    pub thickness_mm: f64,
    /// The assist gas.
    pub gas: String,
    /// The laser this recipe uses.
    pub laser: LaserMode,
    /// The recipe's headline values as the program was compiled with them.
    pub summary: RecipeSummary,
}

impl From<&Recipe> for MaterialView {
    fn from(recipe: &Recipe) -> Self {
        Self {
            name: recipe.name.clone(),
            thickness_mm: recipe.thickness_mm,
            gas: recipe.gas.clone(),
            laser: recipe.laser,
            summary: RecipeSummary::of_recipe(recipe),
        }
    }
}

/// Item-tag progress; the unverified native progress unit is not used.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ProgressView {
    /// Confirmed completed passes in this execution.
    pub completed: usize,
    /// The pass being approached or processed, when the tag names one.
    pub pass: Option<usize>,
    /// Whether its approach is active; for preliminary points the tag also
    /// covers the point process until its completion checkpoint.
    pub approaching: bool,
}

impl ExecutionView {
    pub(crate) fn progress(&self, state: &openlaser_controller::State) -> Option<ProgressView> {
        use openlaser_controller::state::ProgramState;
        let program = state.program.as_ref()?;
        if self.frame || !program.started {
            return None;
        }
        let count = self.compiled.plan.len();
        if program.state == ProgramState::Completed {
            return Some(ProgressView { completed: count, pass: None, approaching: false });
        }
        let item = program
            .checkpoint
            .map(|c| c.item)
            .or_else(|| state.feedback.as_ref().map(|f| f.fifo.item))?;
        let item = item.cast_unsigned();
        if item == u32::MAX - 1 {
            return Some(ProgressView { completed: count, pass: None, approaching: false });
        }
        let pass = usize::try_from(item & 0x7fff_ffff).ok()?;
        let current = self.compiled.plan.get(pass)?;
        let approaching = item & 0x8000_0000 != 0;
        let point_done = !approaching && current.kind == PassKind::PrePierce;
        Some(ProgressView {
            completed: pass + usize::from(point_done),
            pass: Some(pass),
            approaching,
        })
    }
}

/// A sampled motion section of the program.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Move {
    /// What the laser does along it.
    pub kind: PathKind,
    /// The pass, for progress.
    pub pass: Option<usize>,
    /// The polyline.
    #[serde(serialize_with = "crate::display_path::micrometres")]
    pub points: Vec<[f64; 2]>,
}

#[cfg(all(test, feature = "typescript"))]
#[test]
fn export_bindings_recipe_policy() {
    use openlaser_xml::recipe;
    let policy = serde_json::json!({
        "duration": { "aliases": recipe::DURATION_ALIASES, "unit": "ms", "min": 0, "max": recipe::MAX_STAGE_DURATION_MS },
        "stages": recipe::MACHINING_KINDS.map(|(_, count)| count),
        "stored_fields": recipe::STORED_FIELDS,
        "stored_stage_fields": recipe::STORED_STAGE_FIELDS,
    });
    let dir = std::path::PathBuf::from(std::env::var("TS_RS_EXPORT_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("recipe-policy.json"), serde_json::to_vec_pretty(&policy).unwrap())
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_draft_leaves_its_moves_out_and_a_patch_leaves_unchanged_sections_out() {
        let empty = openlaser_core::geometry::Drawing { contours: Vec::new() };
        let mut draft = crate::draft::Draft::of(empty).view();
        draft.compiled = Some(Arc::new(Compiled {
            dry_run: false,
            seconds: 1.,
            plan: vec![],
            pierces: vec![],
            blocks: 1,
            moves: vec![Move {
                kind: PathKind::Cut,
                pass: Some(0),
                points: vec![[12.345, 67.890]; 100_000],
            }],
            usage: crate::gas::Usage::default(),
            pass_usage: Arc::default(),
        }));
        let document = Document { draft: Some(Arc::new(draft)), ..Document::default() };
        let full = serde_json::to_vec(&Patch { document: &document, previous: None }).unwrap();
        // The draft carries the program's summary; its moves are fetched.
        assert!(full.len() < 100_000, "{}", full.len());
        let full: serde_json::Value = serde_json::from_slice(&full).unwrap();
        assert_eq!(full["draft"]["compiled"]["blocks"], 1);
        assert!(full["draft"]["compiled"].get("moves").is_none());
        let patch =
            serde_json::to_vec(&Patch { document: &document, previous: Some(document.revisions) })
                .unwrap();
        assert!(patch.len() < 10_000);
        let patch: serde_json::Value = serde_json::from_slice(&patch).unwrap();
        assert!(patch.get("draft").is_none());
        assert!(patch.get("library").is_none());
        assert!(patch.get("bindings").is_none());
        let cleared = Document {
            draft_revision: 1,
            revisions: Revisions { draft: 1, ..document.revisions },
            ..document.clone()
        };
        let cleared = Document { draft: None, ..cleared };
        let patch =
            serde_json::to_value(Patch { document: &cleared, previous: Some(document.revisions) })
                .unwrap();
        assert_eq!(patch.get("draft"), Some(&serde_json::Value::Null));
        assert_eq!(patch["draft_revision"], 1);
    }

    /// The first event is the whole document: every field a page reads must
    /// be in the hand-written patch, or a new field never reaches the screens.
    #[test]
    fn the_first_patch_carries_every_document_field() {
        let document = Document::default();
        let whole = serde_json::to_value(&document).unwrap();
        let first = serde_json::to_value(Patch { document: &document, previous: None }).unwrap();
        let missing: Vec<&String> = whole
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| first.get(key.as_str()).is_none())
            .collect();
        assert!(missing.is_empty(), "missing from the patch: {missing:?}");
    }
}
