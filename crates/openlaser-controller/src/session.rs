// SPDX-License-Identifier: GPL-3.0-or-later

//! The facts that authorise operations, each with its own lifetime.
//!
//! A home reference, an applied mode, a verified parameter set and a
//! calibration are separate facts, never one `ready` flag. Each is stamped
//! with the connection epoch it was established in, so a reconnect
//! invalidates all of them at once and a mode switch invalidates the ones
//! it must.

use openlaser_core::LaserMode;
use openlaser_protocol::registers::{AXIS_COUNT, PARAMETER_BANK_WORDS};
use serde::Serialize;

/// The quality the head controller reports after a calibration.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Status byte `0x10`.
    Excellent,
    /// Status byte `0x11`.
    Good,
    /// Status byte `0x12`.
    Bad,
}

impl Quality {
    /// The quality a head status byte reports, if it reports one.
    #[must_use]
    pub const fn from_status_byte(byte: u8) -> Option<Self> {
        match byte {
            0x10 => Some(Self::Excellent),
            0x11 => Some(Self::Good),
            0x12 => Some(Self::Bad),
            _ => None,
        }
    }
}

/// The parameter banks read from the controller. Applying parameters also
/// checks this readback against the written plan; matching an imported
/// machine backup is a separate host-side check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The controller's coordinate divisor at the time.
    pub scale: i32,
    /// The interpolation cycle at the time.
    pub cycle_us: u32,
    /// The five banks as read.
    pub banks: [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT],
    /// System words, including alarm inputs, head routing and input filters.
    pub system: [u32; 26],
    /// The two PWM synchronization output routes.
    pub pwm: [u32; 2],
}

/// The connection, bindings and measured configuration a program was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Configuration {
    /// The connection that supplied the measurements.
    pub epoch: u64,
    /// Changes whenever the host installs bindings.
    pub binding: u64,
    /// The configured laser.
    pub mode: LaserMode,
    /// The complete accepted measurements.
    pub verified: Verified,
}

/// The authority state of the current connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Session {
    /// Counts connections; every fact below names the epoch it belongs to.
    pub epoch: u64,
    /// The epoch in which the XY reference was established by Go Origin.
    pub homed: Option<u64>,
    /// The mode applied to the controller and the epoch it was applied in.
    pub mode: Option<(LaserMode, u64)>,
    /// The verified parameter banks.
    pub parameters: Option<Verified>,
    /// The last head calibration and its epoch.
    pub calibration: Option<(Quality, u64)>,
}

impl Session {
    /// A new connection: nothing carries over.
    pub fn connected(&mut self) {
        self.epoch += 1;
        self.invalidate();
    }

    /// Discards authority after an uncertain operation or a lost link.
    pub(crate) fn invalidate(&mut self) {
        *self = Self { epoch: self.epoch, ..Self::default() };
    }

    /// A mode was applied: the reference, the parameters and the
    /// calibration must be established again.
    pub fn mode_applied(&mut self, mode: LaserMode) {
        self.invalidate();
        self.mode = Some((mode, self.epoch));
    }

    /// Whether the XY reference holds on this connection.
    #[must_use]
    pub fn is_homed(&self) -> bool {
        self.homed == Some(self.epoch)
    }

    /// The mode applied on this connection, if any.
    #[must_use]
    pub fn applied_mode(&self) -> Option<LaserMode> {
        self.mode.filter(|(_, epoch)| *epoch == self.epoch).map(|(mode, _)| mode)
    }

    /// The calibration made on this connection, if any.
    #[must_use]
    pub fn calibration(&self) -> Option<Quality> {
        self.calibration.filter(|(_, epoch)| *epoch == self.epoch).map(|(quality, _)| quality)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reconnect invalidates every fact, and a mode switch invalidates the
    /// reference, the parameters and the calibration but keeps the mode.
    #[test]
    fn facts_expire_with_their_epoch() {
        let mut session = Session::default();
        session.connected();
        session.homed = Some(session.epoch);
        session.calibration = Some((Quality::Good, session.epoch));
        session.mode_applied(LaserMode::Co2);
        assert!(!session.is_homed());
        assert_eq!(session.calibration(), None);
        assert_eq!(session.applied_mode(), Some(LaserMode::Co2));
        session.homed = Some(session.epoch);
        assert!(session.is_homed());
        session.connected();
        assert!(!session.is_homed());
        assert_eq!(session.applied_mode(), None);
        assert_eq!(Quality::from_status_byte(0x11), Some(Quality::Good));
        assert_eq!(Quality::from_status_byte(0x13), None);
    }
}
