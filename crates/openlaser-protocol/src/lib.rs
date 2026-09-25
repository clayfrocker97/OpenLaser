// SPDX-License-Identifier: GPL-3.0-or-later

//! Machine-exact. The MCC100 controller's wire contract, with no I/O.
//!
//! The frame codec, the catalogue of request shapes, the feedback decoders,
//! the alarm catalogue and the native record vocabulary of the FIFO stream.
//! Everything here reproduces what the vendor software does, byte for byte,
//! and every constant names the evidence it rests on.
//!
//! Do not improve this crate. Reproduce it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod alarms;
pub mod feedback;
mod frame;
pub mod records;
pub mod registers;
pub mod requests;
pub mod sequences;

pub use frame::{
    Direction, Frame, FrameError, Function, MAX_FRAME_BYTES, MAX_WORDS, crc16, refusal,
};
