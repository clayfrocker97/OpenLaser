// SPDX-License-Identifier: GPL-3.0-or-later

//! The only crate that talks to the machine.
//!
//! One task owns the UDP socket. Commands arrive on a channel, live state is
//! published through a watch. Every tick the task reads the controller's
//! feedback blocks, feeds the alarm monitor and advances the one operation
//! that may be active: home, calibration, a jog, a mode switch, alarm
//! relief, a manual output, or a running program. Operations are state
//! machines over feedback that return the writes to send, so the vendor's
//! sequences are visible in one place per operation and testable without a
//! socket.
//!
//! Stop and release bypass the normal command queue and are checked between
//! exchanges, each bounded by [`Config::reply_timeout`] (500 ms by default).
//! Shutdown attempts every required OFF/stop/clear write even if an earlier
//! acknowledgement is lost or the requester is gone. An uncertain enabling
//! write is never retried.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests build known feedback and compare exact model values"
    )
)]

pub mod alarm_text;
pub mod alarms;
pub mod bindings;
pub mod config;
pub mod lease;
pub mod link;
pub mod machine;
pub mod operations;
pub mod session;
pub mod simulator;
pub mod snapshot;
pub mod state;
pub mod streaming;

pub use bindings::Bindings;
pub use config::Config;
pub use machine::Machine;
pub use simulator::Simulator;
pub use state::State;

/// Why a command was not carried out.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// There is no connection to the controller.
    #[error("the machine is not connected")]
    Disconnected,
    /// The machine bindings have not been configured.
    #[error("the machine is not configured")]
    Unconfigured,
    /// The controller did not answer, or answered wrongly. A write may or
    /// may not have been applied.
    #[error("{0}")]
    Link(String),
    /// The machine's current state does not admit the command.
    #[error("{0}")]
    Refused(String),
    /// The operation started but did not complete.
    #[error("{0}")]
    Failed(String),
    /// Another operation is active.
    #[error("{0} is active")]
    Busy(&'static str),
    /// The controller task is no longer running.
    #[error("the controller task has stopped")]
    Gone,
}

impl From<link::LinkError> for Error {
    fn from(error: link::LinkError) -> Self {
        Self::Link(error.to_string())
    }
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;
