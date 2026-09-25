// SPDX-License-Identifier: GPL-3.0-or-later

//! The head raise that ends the vendor's manual stop, which a pause and the
//! Stop button send (NCModule `0x10057240`).
//!
//! The stop before it has already stopped the FIFO (or the axes), switched
//! the laser, gas and outputs off and cancelled the head. On the first
//! feedback taken after those writes, as the vendor reads the head again
//! after its cancel, the raise decides: the head must be referenced, clear
//! of the Z value warning (head bit 12) and at least the safe height below
//! its origin, so the move can only go up. OpenLaser also leaves the head
//! where it is under an emergency stop or a Z fault that makes moving it
//! pointless or unsafe. What follows the stop, the laser mode for a fiber
//! head, goes out right after the retract request, or at once when there
//! is none. Nothing waits for the head to arrive: the head controller
//! finishes the move on its own, and the pause does not hang on a head
//! that stays busy.

use crate::snapshot::Snapshot;
use openlaser_protocol::requests::Write;
use openlaser_protocol::sequences;
use std::time::{Duration, Instant};

/// How long the raise may wait for the feedback it decides on.
const FEEDBACK_WAIT: Duration = Duration::from_millis(500);

/// Z faults under which the head stays where it is: the upper hardware and
/// software limits, the servo, the encoder, the FPGA and the vendor's own
/// exclusion, the Z value warning.
const STAY_BITS: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 11) | (1 << 12);
/// Controller group 1 bit 30, the emergency stop.
const EMERGENCY_STOP: u32 = 1 << 30;

/// A pending raise and the writes that follow it.
#[derive(Debug)]
pub struct Raise {
    request: Option<sequences::Raise>,
    after: Vec<Write>,
    stopped: Instant,
    done: bool,
}

impl Raise {
    /// A raise decided on feedback taken after `stopped`, followed by
    /// `after`, such as the fiber head's mode.
    #[must_use]
    pub fn new(raise: Option<sequences::Raise>, after: Vec<Write>, stopped: Instant) -> Self {
        Self { request: raise, after, stopped, done: false }
    }

    /// Whether the raise and the writes after it have been requested.
    #[must_use]
    pub const fn done(&self) -> bool {
        self.done
    }

    /// Advances on a snapshot: the writes to send now, possibly none.
    pub fn step(&mut self, snapshot: &Snapshot, now: Instant) -> Vec<Write> {
        if self.done {
            return Vec::new();
        }
        let mut writes = Vec::new();
        if snapshot.taken > self.stopped {
            match self.request {
                Some(raise) if may_raise(raise, snapshot) => {
                    tracing::info!(
                        from_mm = f64::from(snapshot.head.height()) / 1000.,
                        to_mm = f64::from(raise.height_thousandths) / 1000.,
                        "stop: raising the head"
                    );
                    writes.push(raise.write());
                }
                Some(_) => tracing::info!(
                    head_alarms = format_args!("{:#06x}", snapshot.head.alarm_word() & 0xffff),
                    height_mm = f64::from(snapshot.head.height()) / 1000.,
                    referenced = snapshot.head.referenced(),
                    "stop: the head stays where it is"
                ),
                None => {}
            }
        } else if now.saturating_duration_since(self.stopped) <= FEEDBACK_WAIT {
            return Vec::new();
        } else {
            tracing::warn!("stop: no fresh feedback to raise the head on");
        }
        self.done = true;
        writes.append(&mut self.after);
        writes
    }
}

/// Whether the head may rise to `raise` from this feedback: referenced,
/// free of the Z faults that keep it where it is and the emergency stop, and at
/// least the safe height below its origin, so the move goes up.
#[must_use]
pub fn may_raise(raise: sequences::Raise, snapshot: &Snapshot) -> bool {
    let head = &snapshot.head;
    head.referenced()
        && head.alarm_word() & STAY_BITS == 0
        && snapshot.status.alarm_group_1() & EMERGENCY_STOP == 0
        && i64::from(head.height()) >= i64::from(raise.height_thousandths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::snapshot_with;
    use openlaser_protocol::requests;

    const RAISE: sequences::Raise =
        sequences::Raise { speed_tenths: 1000, height_thousandths: 15_000 };

    fn gas() -> Vec<Write> {
        vec![requests::analog_output(1, 0).unwrap()]
    }

    /// A referenced head, `height` thousandths below its origin, with the
    /// head alarm word `alarms` and controller group 1 `group_1`.
    fn head(height: i32, alarms: u32, group_1: u32) -> Snapshot {
        snapshot_with(
            &[(6, group_1)],
            &[],
            &[(1, alarms), (2, 0x8000_0001), (6, height.cast_unsigned())],
        )
    }

    /// On feedback taken after the stop the retract goes out, then the gas,
    /// and nothing waits for the head to arrive.
    #[test]
    fn the_head_rises_before_the_gas_goes_off() {
        let stopped = Instant::now();
        let mut raise = Raise::new(Some(RAISE), gas(), stopped);
        let mut stale = head(19_500, 1 << 5, 0);
        stale.taken = stopped;
        assert!(raise.step(&stale, stopped).is_empty(), "feedback from before the stop");
        let mut touching = head(19_500, 1 << 5, 0);
        touching.taken = stopped + Duration::from_millis(1);
        let writes = raise.step(&touching, touching.taken);
        assert_eq!(writes[0], requests::head_retract(1000, 15_000));
        assert_eq!(writes[1..], gas()[..]);
        assert!(raise.done());
        assert!(raise.step(&touching, touching.taken).is_empty(), "requested once");
    }

    /// Above the safe height, unreferenced, under an emergency stop or a
    /// Z fault, the head stays and the gas goes off at once.
    #[test]
    fn the_head_stays_where_rising_is_not_safe() {
        let mut unreferenced = head(19_500, 0, 0);
        unreferenced.head = snapshot_with(&[], &[], &[(6, 19_500)]).head;
        let stopped = Instant::now();
        for mut snapshot in [
            head(10_000, 0, 0),
            unreferenced,
            head(19_500, 0, EMERGENCY_STOP),
            head(19_500, 1 << 12, 0),
            head(19_500, 1 << 4, 0),
            head(19_500, 1 << 0, 0),
        ] {
            snapshot.taken = stopped + Duration::from_millis(1);
            let mut raise = Raise::new(Some(RAISE), gas(), stopped);
            assert_eq!(raise.step(&snapshot, snapshot.taken), gas());
            assert!(raise.done());
        }
        let mut snapshot = head(19_500, 1 << 5, 0);
        snapshot.taken = stopped + Duration::from_millis(1);
        let mut none = Raise::new(None, gas(), stopped);
        assert_eq!(none.step(&snapshot, snapshot.taken), gas(), "no head configured");
    }

    /// Without fresh feedback the gas does not wait long.
    #[test]
    fn the_gas_does_not_wait_for_feedback() {
        let stopped = Instant::now();
        let mut raise = Raise::new(Some(RAISE), gas(), stopped);
        let mut stale = head(19_500, 0, 0);
        stale.taken = stopped;
        assert!(raise.step(&stale, stopped + Duration::from_millis(100)).is_empty());
        assert_eq!(raise.step(&stale, stopped + FEEDBACK_WAIT * 2), gas());
        assert!(raise.done());
    }
}
