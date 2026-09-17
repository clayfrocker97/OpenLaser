// SPDX-License-Identifier: GPL-3.0-or-later

//! What the operations need to know about this machine: the vendor's
//! parameters, already bound into the shapes the protocol crate sequences
//! take. Built from the vendor XML upstream; the controller only executes.

use crate::alarms::Rule;
use openlaser_core::LaserMode;
use openlaser_protocol::sequences::{DualDrive, ModeSwitch, Shutdown};

/// The outputs the vendor's Go Origin sequence drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HomeOutputs {
    /// `DO.ZFGoOriginDone`: switched off when the head search starts; zero
    /// for none.
    pub z_origin_done_port: u8,
    /// `DO.ManuSignal`: held on while the XY search runs; zero for none.
    pub manual_signal_port: u8,
}

/// Everything bound for one laser mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    /// The laser these bindings are for.
    pub mode: LaserMode,
    /// Whether a head controller is configured (`ZF.ZFType` nonzero).
    pub head_enabled: bool,
    /// The output-off policy and the rapid-stop deceleration.
    pub shutdown: Shutdown,
    /// The Go Origin outputs.
    pub home: HomeOutputs,
    /// The mode switch plan for `mode`.
    pub mode_switch: ModeSwitch,
    /// The XY limits restored by a dual-drive reset, when the machine has
    /// the ordinary two-axis route.
    pub dual_drive: Option<DualDrive>,
    /// The host input rules: doors, water, laser source and gas feedback.
    pub rules: Vec<Rule>,
    /// The port the controller must report at register 50108 before CO2
    /// output is enabled, when PWM synchronisation is on.
    pub co2_pwm_sync_port: Option<u8>,
}
