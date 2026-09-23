// SPDX-License-Identifier: GPL-3.0-or-later

//! What the task publishes: everything a user interface needs to render the
//! machine, as plain data.

use crate::session::Quality;
use openlaser_core::LaserMode;
use openlaser_protocol::records::Checkpoint;
use serde::Serialize;

/// The connection to the controller.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Connection {
    /// No socket.
    #[default]
    Disconnected,
    /// A socket with a complete snapshot behind it.
    Connected {
        /// The connection count.
        epoch: u64,
        /// The endpoint.
        endpoint: String,
    },
    /// The connection was lost; an explicit reconnect is required.
    Faulted {
        /// Why.
        reason: String,
    },
}

/// The controller's identity.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Identity {
    /// The product id; 103 for the MCC100.
    pub product_id: u32,
    /// The program version; 20177 for firmware 201.77.
    pub program_version: u32,
}

impl From<openlaser_protocol::feedback::Identity> for Identity {
    fn from(identity: openlaser_protocol::feedback::Identity) -> Self {
        Self { product_id: identity.product_id, program_version: identity.program_version }
    }
}

/// The head controller as last read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct HeadView {
    /// Whether the head holds a reference.
    pub referenced: bool,
    /// The head height in millimetres.
    pub height_mm: f64,
    /// The command in progress, zero when idle.
    pub command: u32,
    /// The head status byte.
    pub status: u8,
}

/// The FIFO as last read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FifoView {
    /// Free space in bytes.
    pub free: u32,
    /// Total capacity in bytes, once read.
    pub capacity: Option<u32>,
    /// The activity byte: 1 while a program runs.
    pub activity: u8,
    /// The stamp of the last accepted upload.
    pub stamp: u32,
    /// The running item tag and progress within it.
    pub item: i32,
    /// Progress within the running item.
    pub progress: i32,
}

/// The feedback as last read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Feedback {
    /// X, Y and Z in millimetres.
    pub position_mm: [f64; 3],
    /// W/table position from axis record 4, separate from the head.
    pub table_mm: f64,
    /// Whether the table reports zero speed and phase.
    pub table_stationary: bool,
    /// The controller's units per millimetre.
    pub scale: i32,
    /// The interpolation cycle in microseconds.
    pub cycle_us: u32,
    /// Whether X and Y report a reference.
    pub referenced: [bool; 2],
    /// Whether X and Y are stationary.
    pub stationary: bool,
    /// The head.
    pub head: HeadView,
    /// The 24 digital inputs.
    pub inputs: u32,
    /// The standard output bank.
    pub outputs: u16,
    /// The extended output bank.
    pub extended_outputs: u32,
    /// The FIFO.
    pub fifo: FifoView,
    /// The two controller alarm aggregates.
    pub alarm_groups: [u32; 2],
    /// How old the snapshot is, in milliseconds.
    pub age_ms: u64,
}

/// The authority facts of the connection.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionView {
    /// Whether Go Origin established the XY reference on this connection.
    pub homed: bool,
    /// The mode applied on this connection.
    pub mode: Option<LaserMode>,
    /// Whether the parameter banks were read back and matched.
    pub parameters_verified: bool,
    /// The head calibration made on this connection.
    pub calibration: Option<Quality>,
}

/// One alarm row.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AlarmView {
    /// Whether the cause is present, distinct from retained reset state.
    pub active: bool,
    /// The actual relief offered for this row, shown before activation.
    pub relief: ReliefView,
    /// The vendor's id, when it emits one.
    pub id: Option<u32>,
    /// Where the row came from.
    pub source: String,
    /// The vendor's label, or the host rule's.
    pub label: String,
    /// The plain name operators read, such as "Emergency stop pressed".
    pub title: String,
    /// One sentence on how to clear the row.
    pub fix: String,
    /// Whether the row blocks operations.
    pub blocking: bool,
    /// Whether the row stays until relieved even when the cause is gone.
    pub latched: bool,
    /// Seconds since the row appeared.
    pub age_seconds: u64,
}

/// What relieving an alarm does.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReliefView {
    /// Operator-facing action name.
    pub label: String,
    /// Whether the action can command axis movement.
    pub moves_axes: bool,
}

/// What kind of operation is active.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Go Origin, or the head-only reference search.
    Home,
    /// Head height calibration.
    Calibrate,
    /// A jog or a positioning move.
    Motion,
    /// The laser mode switch.
    ModeSwitch,
    /// Alarm relief.
    Relief,
    /// A manual output held on.
    Outputs,
    /// A program.
    Program,
    /// Reading or applying the controller's parameter banks.
    Parameters,
}

impl OperationKind {
    /// The kind as named in messages.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "the reference search",
            Self::Calibrate => "the calibration",
            Self::Motion => "a move",
            Self::ModeSwitch => "the mode switch",
            Self::Relief => "alarm relief",
            Self::Outputs => "a manual output",
            Self::Program => "a program",
            Self::Parameters => "the parameter operation",
        }
    }
}

/// The active operation.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationView {
    /// Its kind.
    pub kind: OperationKind,
    /// Its phase, as a short label.
    pub phase: String,
    /// Seconds since it started.
    pub age_seconds: u64,
}

/// Where a program stands.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    /// Blocks are being uploaded and executed.
    Running,
    /// Every block is uploaded; execution continues.
    Finishing,
    /// Held after a pause; a continuation may be run.
    Held,
    /// Finished normally.
    Completed,
    /// Stopped by request.
    Stopped,
    /// Ended by a failure.
    Failed,
}

/// The last program.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramView {
    /// Where it stands.
    pub state: ProgramState,
    /// Whether feedback was captured after FIFO start was acknowledged.
    pub started: bool,
    /// Blocks acknowledged by the controller.
    pub uploaded: usize,
    /// Blocks in the program.
    pub total: usize,
    /// The execution checkpoint captured at the pause or stop.
    pub checkpoint: Option<CheckpointView>,
    /// Why it stopped early, if it did.
    pub error: Option<String>,
}

/// An execution checkpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CheckpointView {
    /// The item tag being executed.
    pub item: i32,
    /// Progress within the item.
    pub progress: i32,
    /// X and Y where the machine stopped, in millimetres.
    pub position_mm: [f64; 2],
}

impl CheckpointView {
    pub(crate) fn new(checkpoint: Checkpoint, position_mm: [f64; 2]) -> Self {
        Self { item: checkpoint.item, progress: checkpoint.progress, position_mm }
    }
}

/// The published state.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    /// Changes on alarm transitions or a controller link fault.
    pub alarm_revision: u64,
    /// Accepted configuration for host program construction, omitted from UI JSON.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub configuration: Option<crate::session::Configuration>,
    /// Last complete measurements for settings diagnostics, without authority.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub observed_parameters: Option<crate::session::Verified>,
    /// Changes with every publication.
    pub revision: u64,
    /// The connection.
    pub connection: Connection,
    /// The controller's identity, once read.
    pub identity: Option<Identity>,
    /// The mode the bindings are for, once configured.
    pub configured_mode: Option<LaserMode>,
    /// The feedback, while connected.
    pub feedback: Option<Feedback>,
    /// The authority facts.
    pub session: SessionView,
    /// The alarm rows.
    pub alarms: Vec<AlarmView>,
    /// Why operations are refused right now, if they are.
    pub blocked: Option<String>,
    /// Alarms blocking homing or laser-off manual positioning. Missing
    /// reference and process-only notices are excluded.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub motion_blocked: Option<String>,
    /// Laser-off setup, without a missing-reference concession.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub setup_blocked: Option<String>,
    /// X then Y, each negative then positive. Includes directional recovery.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub xy_jog_blocked: [[Option<String>; 2]; 2],
    /// A known X/Y limit requires a bounded recovery pulse.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub xy_recovery: bool,
    /// Manual Z admission, down then up, including directional limit recovery.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub head_jog_blocked: [Option<String>; 2],
    /// A known directional Z limit requires bounded recovery presses.
    #[serde(skip)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub head_recovery: bool,
    /// The active operation.
    pub operation: Option<OperationView>,
    /// The last program.
    pub program: Option<ProgramView>,
    /// The last failure, until the next successful command.
    pub last_error: Option<String>,
}
