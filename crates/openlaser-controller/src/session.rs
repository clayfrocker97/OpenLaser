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

/// The words of an axis bank that only bound travel: the soft limit pair.
const LIMIT_WORDS: [usize; 2] = [1, 2];

impl Verified {
    /// Whether an XY reference established under `self` still holds under
    /// `other`: the same coordinate scale and the same X and Y banks, apart
    /// from their soft limits, which bound travel but move no coordinate.
    #[must_use]
    pub fn keeps_reference(&self, other: &Self) -> bool {
        self.scale == other.scale
            && self.banks[..2].iter().zip(&other.banks[..2]).all(|(a, b)| {
                (0..PARAMETER_BANK_WORDS)
                    .all(|word| LIMIT_WORDS.contains(&word) || a[word] == b[word])
            })
    }
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
    /// The axis configuration Go Origin established the reference under. A
    /// later configuration keeps the reference only if it
    /// [keeps](Verified::keeps_reference) this one's coordinates.
    pub reference: Option<Verified>,
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

    /// New bindings or a new mode: the parameters, the mode and the
    /// calibration must be established again. The XY reference is physical
    /// and carries over; the task drops it as soon as the controller stops
    /// reporting it or the configuration stops keeping it.
    pub(crate) fn rebind(&mut self) {
        let (homed, reference) = (self.homed, self.reference);
        self.invalidate();
        (self.homed, self.reference) = (homed, reference);
    }

    /// A mode was applied: the parameters and the calibration must be
    /// established again, while the physical XY reference carries over.
    pub fn mode_applied(&mut self, mode: LaserMode) {
        self.rebind();
        self.mode = Some((mode, self.epoch));
    }

    /// Whether the reference Go Origin established holds under `verified`.
    #[must_use]
    pub fn reference_holds(&self, verified: &Verified) -> bool {
        self.reference.is_some_and(|reference| reference.keeps_reference(verified))
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
    /// parameters and the calibration but keeps the reference and the mode.
    #[test]
    fn facts_expire_with_their_epoch() {
        let mut session = Session::default();
        session.connected();
        session.homed = Some(session.epoch);
        session.calibration = Some((Quality::Good, session.epoch));
        session.mode_applied(LaserMode::Co2);
        assert!(session.is_homed());
        assert_eq!(session.calibration(), None);
        assert_eq!(session.applied_mode(), Some(LaserMode::Co2));
        session.connected();
        assert!(!session.is_homed());
        assert_eq!(session.applied_mode(), None);
        assert_eq!(Quality::from_status_byte(0x11), Some(Quality::Good));
        assert_eq!(Quality::from_status_byte(0x13), None);
    }

    /// Soft limits and every other axis may change under a reference; an X
    /// or Y coordinate word or the coordinate scale may not.
    #[test]
    fn only_xy_coordinates_bind_the_reference() {
        let homed = Verified {
            scale: 1000,
            cycle_us: 250,
            banks: [[7; PARAMETER_BANK_WORDS]; AXIS_COUNT],
            system: [0; 26],
            pwm: [0; 2],
        };
        let mut other = homed;
        other.banks[0][1] = 1;
        other.banks[1][2] = 1;
        other.banks[3][10] = 1;
        other.system[5] = 1;
        other.pwm = [9, 9];
        assert!(homed.keeps_reference(&other));
        let mut geared = homed;
        geared.banks[1][10] = 8;
        assert!(!homed.keeps_reference(&geared));
        assert!(!homed.keeps_reference(&Verified { scale: 100, ..homed }));
        let mut session = Session::default();
        session.connected();
        (session.homed, session.reference) = (Some(session.epoch), Some(homed));
        session.rebind();
        assert!(session.is_homed() && session.reference_holds(&other));
        assert!(!session.reference_holds(&geared));
    }
}
