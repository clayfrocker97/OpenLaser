// SPDX-License-Identifier: GPL-3.0-or-later

//! The vendor's parameter files as M-Laser writes them: the machine backup,
//! the split hardware, manual and layer files, and the host's `ipAdd.ini`.
//!
//! Two jobs. First, lossless retention: a [`Document`] keeps its original
//! bytes, every attribute is readable by path, and an edit produces a new
//! document with only that value changed. Second, binding: a [`Bundle`] of
//! documents becomes the compiler's [`Settings`](openlaser_compiler::settings::Settings)
//! for one laser and layer through the vendor's own rules, which live in
//! [`recipe`], the controller's axis parameter banks through [`parameters`],
//! and the host's stop, home, mode, alarm and jog bindings through
//! [`bindings`]. Attribute names are resolved by full path, never by name
//! alone.
//!
//! The binding rules are the vendor's: which attribute feeds which field,
//! how it is scaled, what is refused. Where the vendor host tolerates a
//! value silently, this crate refuses it and says why.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        reason = "tests compare exact vendor values"
    )
)]

mod attrs;
pub mod bindings;
pub mod document;
pub mod initialization;
pub mod layer_file;
pub mod parameters;
pub mod recipe;

pub use document::{Bundle, Document, Kind};

/// Why a file could not be read or bound.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Invalid XML syntax, with the parser's source position and explanation.
    #[error("XML at byte {offset}: {reason}")]
    Syntax {
        /// Byte offset in the original file.
        offset: usize,
        /// The XML parser's explanation.
        reason: String,
    },
    /// The bytes are not the XML subset the vendor writes.
    #[error("XML at byte {offset}: {reason}")]
    Malformed {
        /// Byte offset of the problem.
        offset: usize,
        /// What was found.
        reason: &'static str,
    },
    /// A construct the vendor never writes and this parser refuses.
    #[error("XML construct not accepted: {0}")]
    Forbidden(&'static str),
    /// The file is too large or too deep to be a parameter file.
    #[error("XML exceeds the {0} limit")]
    Limit(&'static str),
    /// A path appears twice, so a read or edit would be ambiguous.
    #[error("XML path {0} appears more than once")]
    Duplicate(String),
    /// A group or attribute the binding needs is not there.
    #[error("missing setting {0}")]
    Missing(String),
    /// A setting has a value the vendor's rules do not accept.
    #[error("setting {field}: {reason}")]
    Invalid {
        /// The attribute, with its group.
        field: String,
        /// What is wrong with it.
        reason: String,
    },
    /// A combination of settings the compiler cannot run.
    #[error("{0}")]
    Unsupported(String),
    /// The compiler refused the bound values.
    #[error(transparent)]
    Compiler(#[from] openlaser_compiler::Error),
    /// A bound value does not make a valid controller request.
    #[error(transparent)]
    Request(#[from] openlaser_protocol::requests::RequestError),
    /// A bound value does not make a valid controller sequence.
    #[error(transparent)]
    Sequence(#[from] openlaser_protocol::sequences::SequenceError),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// A validated whole number as the integer it is, or `None` for anything
/// fractional, negative or beyond 32 bits.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "range checked before the cast"
)]
pub(crate) fn whole(value: f64) -> Option<u32> {
    (value.is_finite() && value.fract() == 0. && (0. ..=f64::from(u32::MAX)).contains(&value))
        .then_some(value as u32)
}
