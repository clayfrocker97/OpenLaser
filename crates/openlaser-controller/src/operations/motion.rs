// SPDX-License-Identifier: GPL-3.0-or-later

//! A jog or a positioning move: one native relative move that owns the
//! whole distance, MainApp `0x005b_7460` and `0x005b_c110`. A held jog is
//! the same move to the travel limit, decelerated to a stop when the
//! operator lets go or the heartbeat lapses. Completion is a host
//! judgement: stationary feedback within one pulse of the target.

use super::{
    Operation, Step, admit_alarms, admit_idle, admit_positioning,
    home::{self, Home},
};
use crate::Bindings;
use crate::alarms::Concession;
use crate::session::Verified;
use crate::snapshot::Snapshot;
use crate::state::OperationKind;
use openlaser_protocol::requests::{self, Write};
use std::time::{Duration, Instant};

/// How long a held jog survives without a heartbeat.
pub const HEARTBEAT: Duration = Duration::from_millis(350);
/// Watchdog headroom beyond the expected duration of a move.
const WATCHDOG_MARGIN: Duration = Duration::from_secs(10);
/// A stopped jog must settle before the host gives up its reference.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Even a renewed recovery press has a finite lifetime.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// One move, in controller units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The relative travel on X and Y.
    pub delta: [i32; 2],
    /// The speed word.
    pub speed: u32,
    /// The acceleration word.
    pub acceleration: u32,
    /// Whether the move is held and ends when released.
    pub held: bool,
    /// The deceleration word for the stop.
    pub deceleration: u32,
}

impl Request {
    /// The vendor's request for this move: a single-axis jog or an XY move.
    pub fn write(&self) -> Result<Write, String> {
        let write = match self.delta {
            [dx, 0] => requests::jog(0, self.speed, self.acceleration, dx),
            [0, dy] => requests::jog(1, self.speed, self.acceleration, dy),
            delta => requests::move_xy(delta, self.speed, self.acceleration),
        };
        write.map_err(|e| e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Start,
    Retracting,
    Moving,
    Stopping,
    Done,
}

/// The move.
#[derive(Debug)]
pub struct Motion {
    request: Request,
    tolerance: [u32; 2],
    limits: [(i32, i32); 2],
    head_enabled: bool,
    target: [i32; 2],
    phase: Phase,
    sent: Option<Instant>,
    heartbeat: Instant,
    watchdog: Duration,
    axes: [usize; 2],
    retraction: Option<Home>,
    jog: Option<(usize, bool)>,
    parameters: Verified,
    recovery_deadline: Option<Instant>,
}

impl Motion {
    /// A move under the verified parameter banks, whose words 1 and 2 are
    /// the soft limits and whose words 9 and 10 give the pulse resolution.
    pub fn new(
        request: Request,
        verified: &Verified,
        head_enabled: bool,
        now: Instant,
    ) -> Result<Self, String> {
        if request.delta == [0, 0] {
            return Err("the move has no distance".into());
        }
        let mut tolerance = [0; 2];
        let mut limits = [(0, 0); 2];
        for axis in 0..2 {
            let bank = verified.banks[axis];
            let (distance, pulses) = (bank[9], bank[10]);
            if distance == 0
                || pulses == 0
                || distance > i32::MAX.cast_unsigned()
                || pulses > i32::MAX.cast_unsigned()
            {
                return Err("the axis parameters do not give a pulse resolution".into());
            }
            tolerance[axis] = distance.div_ceil(pulses);
            limits[axis] = (bank[1].cast_signed(), bank[2].cast_signed());
            if limits[axis].0 >= limits[axis].1 {
                return Err("the axis parameters do not give a travel range".into());
            }
        }
        let watchdog = watchdog(&request)?;
        let jog = match request.delta {
            [delta, 0] => Some((0, delta > 0)),
            [0, delta] => Some((1, delta > 0)),
            _ => None,
        };
        Ok(Self {
            request,
            tolerance,
            limits,
            head_enabled,
            target: [0; 2],
            phase: Phase::Start,
            sent: None,
            heartbeat: now,
            watchdog,
            axes: [0, 1],
            retraction: None,
            jog,
            parameters: *verified,
            recovery_deadline: None,
        })
    }

    /// A discrete XY position uses the existing head search before travel
    /// when needed, without opening a gap for another controller operation.
    pub fn positioning(
        request: Request,
        verified: &Verified,
        bindings: &Bindings,
        now: Instant,
    ) -> Result<Self, String> {
        if request.held {
            return Err("positioning cannot be held".into());
        }
        let mut motion = Self::new(request, verified, bindings.head_enabled, now)?;
        motion.jog = None;
        motion.retraction = bindings.head_enabled.then(|| Home::new(bindings, true));
        Ok(motion)
    }

    /// The configured lifting table uses axis record 4, independently of Z.
    pub fn table(
        request: Request,
        verified: &Verified,
        head_enabled: bool,
        limits: [i32; 2],
        now: Instant,
    ) -> Result<Self, String> {
        if request.delta[1] != 0 || limits[0] >= limits[1] {
            return Err("invalid table travel".into());
        }
        let mut table = *verified;
        table.banks[0] = verified.banks[4];
        table.banks[0][1] = limits[0].cast_unsigned();
        table.banks[0][2] = limits[1].cast_unsigned();
        let mut motion = Self::new(request, &table, head_enabled, now)?;
        motion.axes = [4, 1];
        motion.jog = None;
        motion.parameters = *verified;
        Ok(motion)
    }

    /// Manual recovery belongs only to a single-axis jog, never an absolute
    /// position, diagonal move, table move or compiled program.
    pub(crate) const fn jog_direction(&self) -> Option<(usize, bool)> {
        self.jog
    }

    /// Recovery can precede session initialization. Its measured parameters
    /// are checked again before the first write and throughout the move.
    pub(crate) fn recovering_limit(&self, snapshot: &Snapshot) -> bool {
        self.jog.is_some() && (self.recovery_deadline.is_some() || snapshot.xy_limit_alarms())
    }

    pub(crate) fn concession(&self, snapshot: &Snapshot) -> Concession {
        self.jog.map_or(
            Concession::ManualPositioning { other_head_faults: home::other_head_faults(snapshot) },
            |(axis, positive)| {
                xy_concession(snapshot, axis, positive, self.recovery_deadline.is_some())
            },
        )
    }

    fn positions(&self, snapshot: &Snapshot) -> [i32; 2] {
        self.axes.map(|axis| snapshot.axes.axis(axis).position)
    }

    fn stationary(&self, snapshot: &Snapshot) -> bool {
        self.axes.iter().all(|&axis| {
            let a = snapshot.axes.axis(axis);
            a.speed == 0 && a.phase() == 0
        })
    }

    /// Renews a held jog's lease.
    pub fn heartbeat(&mut self, now: Instant) {
        self.heartbeat = now;
    }

    /// Whether the move is held.
    #[must_use]
    pub const fn held(&self) -> bool {
        self.request.held
    }

    pub(crate) fn expired(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Start | Phase::Moving)
            && ((self.held() && now.saturating_duration_since(self.heartbeat) > HEARTBEAT)
                || self.recovery_deadline.is_some_and(|deadline| now >= deadline))
    }

    /// Ends a held jog: the writes to send now.
    pub fn release(&mut self, now: Instant) -> Vec<Write> {
        match self.phase {
            Phase::Start | Phase::Retracting => self.phase = Phase::Done,
            Phase::Moving => {
                self.phase = Phase::Stopping;
                self.sent = Some(now);
                return vec![requests::rapid_stop(self.request.deceleration)];
            }
            Phase::Stopping | Phase::Done => {}
        }
        Vec::new()
    }

    fn at_target(&self, snapshot: &Snapshot) -> bool {
        let position = self.positions(snapshot);
        (0..2).all(|axis| {
            (i64::from(position[axis]) - i64::from(self.target[axis])).unsigned_abs()
                <= u64::from(self.tolerance[axis])
        })
    }

    fn target(&self, snapshot: &Snapshot) -> Result<[i32; 2], String> {
        let mut result = [0; 2];
        for (axis, position) in self.positions(snapshot).into_iter().enumerate() {
            let target = position
                .checked_add(self.request.delta[axis])
                .ok_or("the target is out of range")?;
            let (low, high) = self.limits[axis];
            let allowed = if self.recovery_deadline.is_some() {
                self.request.delta[axis] == 0
                    || recovery_target(
                        position,
                        target,
                        (low, high),
                        snapshot.axes.axis(axis).referenced(),
                    )
            } else {
                (low..=high).contains(&target)
            };
            if !allowed {
                return Err("the target is outside the soft limits".into());
            }
            result[axis] = target;
        }
        Ok(result)
    }

    fn retract(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        let home = self.retraction.as_mut().ok_or("head retraction is unavailable")?;
        match home.step(snapshot, blocked, now)? {
            Step::Finish(writes) => {
                self.retraction = None;
                self.phase = Phase::Start;
                Ok(Step::Send(writes))
            }
            step => Ok(step),
        }
    }

    fn start(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        if self.expired(now) {
            self.phase = Phase::Done;
            return Ok(Step::Finish(Vec::new()));
        }
        if let Some((axis, positive)) = self.jog {
            admit_xy_jog(snapshot, blocked, axis, positive)?;
            if snapshot.xy_limit_alarms() {
                self.bound_recovery(snapshot, axis, now)?;
            }
        } else {
            admit_positioning(snapshot, blocked)?;
        }
        self.target = self.target(snapshot)?;
        if self.head_enabled && snapshot.head.referenced() && !snapshot.head.at_upper_reference() {
            if self.retraction.is_some() {
                self.phase = Phase::Retracting;
                return self.retract(snapshot, blocked, now);
            }
            return Err("retract the head to its upper reference before moving".into());
        }
        self.phase = Phase::Moving;
        self.sent = Some(now);
        let write = if self.axes[0] == 4 {
            requests::jog(4, self.request.speed, self.request.acceleration, self.request.delta[0])
                .map_err(|e| e.to_string())?
        } else {
            self.request.write()?
        };
        Ok(Step::Send(vec![write]))
    }

    fn bound_recovery(
        &mut self,
        snapshot: &Snapshot,
        axis: usize,
        now: Instant,
    ) -> Result<(), String> {
        let scale = self.parameters.scale;
        if scale <= 0 || self.tolerance[axis] > scale.cast_unsigned() {
            return Err("the axis resolution cannot represent a 1 mm recovery move".into());
        }
        if snapshot.parameters() != Ok(self.parameters) {
            return Err("the axis parameters changed before recovery".into());
        }
        self.request.delta[axis] = self.request.delta[axis].clamp(-scale, scale);
        self.request.speed = self.request.speed.min(scale.cast_unsigned());
        self.recovery_deadline = Some(now + RECOVERY_TIMEOUT);
        Ok(())
    }
}

/// A referenced axis may approach its valid range from outside. Before
/// referencing, a physical limit can be anywhere in the reported coordinates;
/// the separately bounded recovery pulse does not rely on that origin.
fn recovery_target(current: i32, target: i32, (low, high): (i32, i32), referenced: bool) -> bool {
    if !referenced {
        true
    } else if current < low {
        target > current && target <= high
    } else if current > high {
        target < current && target >= low
    } else {
        (low..=high).contains(&target)
    }
}

pub(crate) fn xy_concession(
    snapshot: &Snapshot,
    axis: usize,
    positive: bool,
    recovering: bool,
) -> Concession {
    Concession::XyJog {
        other_head_faults: home::other_head_faults(snapshot),
        allow_limits: snapshot.xy_jog_alarms_clear(axis, positive, recovering),
    }
}

pub(crate) fn admit_xy_jog(
    snapshot: &Snapshot,
    blocked: Option<&str>,
    axis: usize,
    positive: bool,
) -> Result<(), String> {
    admit_alarms(snapshot.xy_jog_alarms_clear(axis, positive, false), blocked)?;
    if !snapshot.outputs_off(0) {
        return Err("outputs are on".into());
    }
    admit_idle(snapshot)?;
    if snapshot.xy_limit_alarms()
        && (!snapshot.stationary() || snapshot.status.fifo_activity() != 0)
    {
        return Err("wait for stationary axes and an idle FIFO before recovering a limit".into());
    }
    Ok(())
}

fn watchdog(request: &Request) -> Result<Duration, String> {
    let seconds = f64::from(request.delta.iter().map(|d| d.unsigned_abs()).max().unwrap_or(0))
        / f64::from(request.speed.max(1))
        + 2. * f64::from(request.speed) / f64::from(request.acceleration.max(1));
    if !seconds.is_finite() || seconds > 86_400. {
        return Err("the move would take too long to watch".into());
    }
    Ok(Duration::from_secs_f64(seconds * 3.) + WATCHDOG_MARGIN)
}

impl Motion {
    fn advance_moving(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        let clear =
            if let Some((axis, positive)) = self.jog.filter(|_| self.recovery_deadline.is_some()) {
                self.check_recovery(snapshot, axis)?;
                snapshot.xy_jog_alarms_clear(axis, positive, true)
            } else if let Some((axis, positive)) = self.jog {
                !snapshot.xy_limit_alarms() && snapshot.xy_jog_alarms_clear(axis, positive, false)
            } else {
                snapshot.positioning_alarms_clear()
            };
        if !clear || blocked.is_some() {
            return Err("an alarm interrupted the move".into());
        }
        let sent = self.sent.unwrap_or(now);
        if now.duration_since(sent) > self.watchdog {
            return Err("the move did not finish in time".into());
        }
        if self.expired(now) {
            return Ok(Step::Send(self.release(now)));
        }
        if snapshot.taken > sent && self.stationary(snapshot) && self.at_target(snapshot) {
            self.phase = Phase::Done;
            return Ok(Step::Finish(Vec::new()));
        }
        Ok(Step::Wait)
    }

    fn check_recovery(&self, snapshot: &Snapshot, axis: usize) -> Result<(), String> {
        if snapshot.parameters() != Ok(self.parameters) {
            return Err("the axis parameters changed during recovery".into());
        }
        if !snapshot.outputs_off(0)
            || !snapshot.head_idle()
            || snapshot.status.fifo_activity() != 0
            || snapshot
                .axes
                .all()
                .iter()
                .enumerate()
                .any(|(index, record)| index != axis && (record.speed != 0 || record.phase() != 0))
        {
            return Err("unexpected output or motion interrupted recovery".into());
        }
        Ok(())
    }

    fn advance_stopping(&mut self, snapshot: &Snapshot, now: Instant) -> Result<Step, String> {
        let sent = self.sent.unwrap_or(now);
        if now.duration_since(sent) > STOP_TIMEOUT {
            return Err("the jog did not stop in time".into());
        }
        if snapshot.taken > sent && self.stationary(snapshot) {
            self.phase = Phase::Done;
            Ok(Step::Finish(Vec::new()))
        } else {
            Ok(Step::Wait)
        }
    }
}

impl Operation for Motion {
    fn kind(&self) -> OperationKind {
        OperationKind::Motion
    }

    fn phase(&self) -> &'static str {
        match self.phase {
            Phase::Start => "starting",
            Phase::Retracting => "raising head",
            Phase::Moving => "moving",
            Phase::Stopping => "stopping",
            Phase::Done => "done",
        }
    }

    fn step(
        &mut self,
        snapshot: &Snapshot,
        blocked: Option<&str>,
        now: Instant,
    ) -> Result<Step, String> {
        match self.phase {
            Phase::Start => self.start(snapshot, blocked, now),
            Phase::Retracting => self.retract(snapshot, blocked, now),
            Phase::Moving => self.advance_moving(snapshot, blocked, now),
            Phase::Stopping => self.advance_stopping(snapshot, now),
            Phase::Done => Ok(Step::Finish(Vec::new())),
        }
    }

    fn cancel(&self) -> Vec<Write> {
        vec![requests::rapid_stop(self.request.deceleration)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::{bindings, snapshot_with, verified};

    fn request(delta: [i32; 2], held: bool) -> Request {
        Request { delta, speed: 50_000, acceleration: 5999, held, deceleration: 5999 }
    }

    fn recovery_snapshot(status: &[(usize, u32)], axes: &[(usize, u32)]) -> Snapshot {
        let mut snapshot = snapshot_with(status, axes, &[(2, 0x8000_0001)]);
        let mut runtime = [0; 120];
        for (axis, bank) in verified().banks.iter().enumerate() {
            runtime[50 + axis * 14..50 + (axis + 1) * 14].copy_from_slice(bank);
        }
        snapshot.runtime = openlaser_protocol::feedback::Runtime::decode(&runtime).unwrap();
        snapshot
    }

    #[test]
    fn every_xy_limit_permits_only_its_bounded_away_jog() {
        for axis in 0..2 {
            for positive_limit in [false, true] {
                for software in [false, true] {
                    let now = Instant::now();
                    let bit = u32::from(!positive_limit) + 2 * u32::from(software);
                    let position: i32 = if positive_limit { 1_000_000 } else { -500 };
                    let snapshot = recovery_snapshot(
                        &[(6, 1 << axis)],
                        &[
                            (axis * 10, 0x0200_c000 | (1 << bit)),
                            (axis * 10 + 2, position.cast_unsigned()),
                        ],
                    );
                    let mut delta = [0; 2];
                    delta[axis] = if positive_limit { -500_000 } else { 500_000 };
                    let mut away =
                        Motion::new(request(delta, true), &verified(), true, now).unwrap();
                    let write = requests::jog(
                        u8::try_from(axis).unwrap(),
                        1000,
                        5999,
                        delta[axis].clamp(-1000, 1000),
                    )
                    .unwrap();
                    assert_eq!(away.step(&snapshot, None, now).unwrap(), Step::Send(vec![write]));
                    delta[axis] = -delta[axis];
                    let mut toward =
                        Motion::new(request(delta, true), &verified(), true, now).unwrap();
                    assert!(toward.step(&snapshot, None, now).is_err());
                }
            }
        }
    }

    #[test]
    fn recovery_handles_corners_and_unknown_origins_without_opening_other_operations() {
        let now = Instant::now();
        let corner = recovery_snapshot(&[(6, 3)], &[(0, 1), (10, 2)]);
        assert!(corner.xy_jog_alarms_clear(0, false, false));
        assert!(corner.xy_jog_alarms_clear(1, true, false));
        let mut moving =
            Motion::new(request([-100_000, 0], false), &verified(), true, now).unwrap();
        assert!(
            matches!(moving.step(&corner, None, now).unwrap(), Step::Send(_)),
            "an unreferenced origin cannot trap the axis at its physical limit"
        );
        let mut positioning =
            Motion::positioning(request([-1000, 0], false), &verified(), &bindings(), now).unwrap();
        assert!(positioning.step(&corner, None, now).is_err());
        let mut diagonal =
            Motion::new(request([-1000, 1000], true), &verified(), true, now).unwrap();
        assert!(diagonal.step(&corner, None, now).is_err());
        let outside = recovery_snapshot(&[(6, 1)], &[(0, 0xc001), (2, 1_002_500)]);
        let mut inward = Motion::new(request([-100_000, 0], true), &verified(), true, now).unwrap();
        assert!(matches!(inward.step(&outside, None, now).unwrap(), Step::Send(_)));
        for (status, axes) in [
            (vec![(6, 1)], vec![(0, 3)]),
            (vec![(6, 1)], vec![(0, 1 | 16)]),
            (vec![(6, 3)], vec![(0, 1), (10, 16)]),
            (vec![(6, (1 << 30) | 1)], vec![(0, 1)]),
            (vec![(6, 1), (7, 1)], vec![(0, 1)]),
            (vec![(6, 3)], vec![(0, 1)]),
            (vec![(6, 1), (5, 1)], vec![(0, 1)]),
            (vec![(6, 1)], vec![(0, 1), (11, 1)]),
        ] {
            let snapshot = recovery_snapshot(&status, &axes);
            let mut motion =
                Motion::new(request([-1000, 0], true), &verified(), true, now).unwrap();
            assert!(motion.step(&snapshot, None, now).is_err());
        }
    }

    #[test]
    fn recovery_never_expands_and_new_limits_interrupt_an_ordinary_jog() {
        let now = Instant::now();
        let limit = recovery_snapshot(&[(6, 1)], &[(0, 1), (2, 10_000)]);
        let mut motion = Motion::new(request([-100_000, 0], true), &verified(), true, now).unwrap();
        motion.step(&limit, None, now).unwrap();
        let moving =
            recovery_snapshot(&[], &[(0, 0x0301_0000), (1, (-1000i32).cast_unsigned()), (2, 9600)]);
        motion.heartbeat(now + Duration::from_millis(400));
        assert_eq!(
            motion.step(&moving, None, now + Duration::from_millis(500)).unwrap(),
            Step::Wait
        );
        assert!(motion.expired(now + Duration::from_millis(751)));
        motion.heartbeat(now + Duration::from_secs(2));
        assert!(motion.expired(now + Duration::from_secs(2)));
        assert_eq!(
            motion.step(&moving, None, now + Duration::from_secs(2)).unwrap(),
            Step::Send(vec![requests::rapid_stop(5999)])
        );
        let idle = recovery_snapshot(&[], &[(2, 10_000)]);
        let mut normal = Motion::new(request([-10_000, 0], true), &verified(), true, now).unwrap();
        normal.step(&idle, None, now).unwrap();
        assert!(normal.step(&limit, None, now + Duration::from_millis(10)).is_err());
        let detail_without_summary = recovery_snapshot(&[], &[(0, 1), (2, 10_000)]);
        assert!(
            normal.step(&detail_without_summary, None, now + Duration::from_millis(10)).is_err()
        );
        let mut blocked = Motion::new(request([-1000, 0], true), &verified(), true, now).unwrap();
        assert!(blocked.step(&limit, Some("Door"), now).is_err());
    }

    #[test]
    fn positioning_reuses_head_home_and_checks_limits_before_any_write() {
        let now = Instant::now();
        let mut bindings = bindings();
        bindings.head_enabled = true;
        let lowered = snapshot_with(&[], &[], &[(2, 0x8000_0000), (6, 5000)]);
        let mut outside =
            Motion::positioning(request([2_000_000, 0], false), &verified(), &bindings, now)
                .unwrap();
        assert!(outside.step(&lowered, None, now).is_err());
        let mut motion =
            Motion::positioning(request([1000, 0], false), &verified(), &bindings, now).unwrap();
        let first = motion.step(&lowered, None, now).unwrap();
        assert!(
            matches!(first, Step::Send(ref writes) if writes.contains(&requests::head_home()) && !writes.contains(&request([1000, 0], false).write().unwrap()))
        );
        assert_eq!(motion.phase(), "raising head");
        assert_eq!(motion.step(&lowered, None, now).unwrap(), Step::Wait);
        assert!(motion.step(&lowered, Some("door open"), now).is_err());
        let upper = snapshot_with(&[], &[], &[(2, 0x8000_0001)]);
        assert_eq!(motion.step(&upper, None, now).unwrap(), Step::Send(Vec::new()));
        assert!(
            matches!(motion.step(&upper, None, now).unwrap(), Step::Send(ref writes) if writes.contains(&request([1000, 0], false).write().unwrap()))
        );
    }

    /// A step move sends one jog, waits while the axis moves, and finishes
    /// on stationary feedback within a pulse of the target.
    #[test]
    fn a_step_finishes_at_its_target() {
        let now = Instant::now();
        let mut motion = Motion::new(request([1000, 0], false), &verified(), false, now).unwrap();
        let idle = snapshot_with(&[], &[(2, 500)], &[]);
        let step = motion.step(&idle, None, now).unwrap();
        let expected = Write { address: 101, words: vec![3, 0, 50_000, 5999, 59_990, 1000] };
        assert_eq!(step, Step::Send(vec![expected]));
        let moving = snapshot_with(&[], &[(0, 0x0301_0000), (1, 40_000), (2, 900)], &[]);
        assert_eq!(motion.step(&moving, None, now).unwrap(), Step::Wait);
        let mut arrived = snapshot_with(&[], &[(2, 1499)], &[]);
        arrived.taken = now + Duration::from_millis(1);
        assert_eq!(motion.step(&arrived, None, now).unwrap(), Step::Finish(Vec::new()));
    }

    /// A held jog runs to the limit and stops when released or when the
    /// heartbeat lapses; a target past the limit is refused, as is a move
    /// with the head down.
    #[test]
    fn held_jogs_stop_on_release_or_lapse() {
        let now = Instant::now();
        let idle = snapshot_with(&[], &[], &[]);
        let mut held = Motion::new(request([0, 999_500], true), &verified(), false, now).unwrap();
        assert!(matches!(held.step(&idle, None, now).unwrap(), Step::Send(_)));
        assert_eq!(held.release(now), vec![requests::rapid_stop(5999)]);
        let mut stopped = idle;
        stopped.taken = now + Duration::from_millis(1);
        assert_eq!(held.step(&stopped, None, stopped.taken).unwrap(), Step::Finish(Vec::new()));
        let mut lapsed = Motion::new(request([0, 999_500], true), &verified(), false, now).unwrap();
        lapsed.step(&idle, None, now).unwrap();
        let moving = snapshot_with(&[], &[(11, 5000)], &[]);
        assert_eq!(
            lapsed.step(&moving, None, now + Duration::from_millis(100)).unwrap(),
            Step::Wait
        );
        lapsed.heartbeat(now + Duration::from_millis(300));
        assert_eq!(
            lapsed.step(&moving, None, now + Duration::from_millis(600)).unwrap(),
            Step::Wait
        );
        assert_eq!(
            lapsed.step(&moving, None, now + Duration::from_secs(1)).unwrap(),
            Step::Send(vec![requests::rapid_stop(5999)])
        );
        let mut beyond =
            Motion::new(request([2_000_000, 0], false), &verified(), false, now).unwrap();
        assert!(beyond.step(&idle, None, now).is_err());
        let mut head_down = Motion::new(request([100, 0], false), &verified(), true, now).unwrap();
        assert!(
            head_down
                .step(&snapshot_with(&[], &[], &[(2, 0x8000_0000), (6, 5000)]), None, now)
                .is_err()
        );
        let mut head_up = Motion::new(request([100, 0], false), &verified(), true, now).unwrap();
        assert!(head_up.step(&snapshot_with(&[], &[], &[(2, 0x8000_0001)]), None, now).is_ok());
    }

    /// Release and lease expiry also end a jog that has not sent its move.
    #[test]
    fn a_released_or_expired_jog_never_starts() {
        let now = Instant::now();
        let idle = snapshot_with(&[], &[], &[]);
        let mut released = Motion::new(request([1000, 0], true), &verified(), false, now).unwrap();
        assert!(released.release(now).is_empty());
        released.heartbeat(now);
        assert_eq!(released.step(&idle, None, now).unwrap(), Step::Finish(Vec::new()));

        let mut expired = Motion::new(request([1000, 0], true), &verified(), false, now).unwrap();
        assert_eq!(
            expired.step(&idle, None, now + HEARTBEAT + Duration::from_millis(1)).unwrap(),
            Step::Finish(Vec::new())
        );
    }

    /// Old idle feedback cannot complete a stop; fresh motion eventually
    /// times out instead of leaving a jog active forever.
    #[test]
    fn stopping_needs_fresh_feedback_and_has_a_deadline() {
        let now = Instant::now();
        let mut idle = snapshot_with(&[], &[], &[]);
        idle.taken = now;
        let mut motion = Motion::new(request([1000, 0], true), &verified(), false, now).unwrap();
        motion.step(&idle, None, now).unwrap();
        let released = now + Duration::from_millis(1);
        motion.release(released);
        assert_eq!(motion.step(&idle, None, released).unwrap(), Step::Wait);
        let moving = snapshot_with(&[], &[(1, 500)], &[]);
        assert!(
            motion.step(&moving, None, released + STOP_TIMEOUT + Duration::from_millis(1)).is_err()
        );
    }
}
