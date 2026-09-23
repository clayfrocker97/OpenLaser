// SPDX-License-Identifier: GPL-3.0-or-later

//! The alarm monitor: the rows an operator sees, kept over time.
//!
//! Controller and head bits become rows through the protocol crate's
//! catalogue. Host rules add rows for the machine's own inputs: doors and
//! other custom inputs, the cooling water and laser source warnings, and
//! gas feedback qualified by its valve. The vendor's retention rules decide
//! when a row leaves: most rows leave with their bit, custom rows may latch
//! until relieved, and gas rows are retained but never block.

use crate::snapshot::Snapshot;
use crate::state::AlarmView;
use openlaser_protocol::alarms::{
    self, CustomInput, DetailCache, Source, custom_is_armed, gas_update,
};
use std::collections::BTreeMap;
use std::time::Instant;

/// An ordered alarm transition sent to an optional history consumer.
#[derive(Clone, Debug)]
pub struct Observation {
    /// Observation time in seconds since the epoch.
    pub at: u64,
    /// Current rows; ages are zeroed so time alone does not create transitions.
    pub alarms: Vec<AlarmView>,
    /// Current connection failure, when present.
    pub fault: Option<String>,
}

/// The vendor's id for a head that has not found its reference.
pub const HEAD_REFERENCE: u32 = 45;
/// The vendor's id for the head reference warning bit, relieved by the head
/// search like [`HEAD_REFERENCE`].
pub const HEAD_REFERENCE_WARNING: u32 = 99;
/// The special group 1 bit that can accompany head faults. Its native detail
/// lookup aliases the optional autofocus cache; it has no unique Z label.
const SPECIAL_GROUP1_BIT: u8 = 24;

/// A host input rule, bound from the vendor's `DI` parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The vendor's id: 1000 to 1015 for custom inputs, 150 to 155 for gas
    /// feedback, 56 for cooling water, 60 for the laser source.
    pub id: u32,
    /// The label shown for the row.
    pub label: String,
    /// The digital input, 1 to 24.
    pub input: u8,
    /// Whether the input trips when low.
    pub active_low: bool,
    /// Whether the rule is armed only while a job runs.
    pub run_only: bool,
    /// Whether the row stays until relieved, even after the input recovers.
    pub latch: bool,
    /// For gas feedback: the valve output that qualifies the input.
    pub gas_valve: Option<u8>,
}

impl Rule {
    const fn is_gas(&self) -> bool {
        self.gas_valve.is_some()
    }

    fn same_binding(&self, other: &Self) -> bool {
        self.id == other.id
            && self.input == other.input
            && self.active_low == other.active_low
            && self.run_only == other.run_only
            && self.latch == other.latch
            && self.gas_valve == other.gas_valve
    }
}

/// One row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The vendor's id, when it emits one.
    pub id: Option<u32>,
    /// Where the row came from.
    pub source: String,
    /// The label.
    pub label: String,
    /// Whether the row blocks operations.
    pub blocking: bool,
    /// Whether the row is latched.
    pub latched: bool,
    /// Whether the cause is present now.
    pub active: bool,
    /// When the row appeared.
    pub since: Instant,
}

/// Row keys, ordered so the rows list is stable: controller group bits,
/// head bits, the head reference, axis details, then host rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Group1(u8),
    Group2(u8),
    Head(u8),
    HeadReference,
    Axis(u8, u8),
    Rule(u32),
}

/// What an operation may see past when it asks whether it is blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Concession {
    /// Nothing.
    None,
    /// The head reference row: Go Origin and the mode switch establish or
    /// invalidate the reference themselves.
    HeadReference,
    /// Cooling water and the laser source are needed for processing, not
    /// for a laser-off setup action such as head calibration.
    ProcessOnly,
    /// Missing-reference conditions allowed while homing or positioning:
    /// both head reference rows
    /// and, while the head feedback permits it, group 1 bit 24.
    HeadHome {
        /// Whether the head word carries a fault other than the reference
        /// warning.
        other_head_faults: bool,
    },
    /// Manual homing, positioning and setup also tolerate process-only inputs.
    ManualPositioning {
        /// Any head fault beyond the missing-reference warning.
        other_head_faults: bool,
    },
    /// A manual X/Y jog whose raw feedback permits directional recovery.
    XyJog {
        /// Any head fault beyond the missing-reference warning.
        other_head_faults: bool,
        /// Only known X/Y limits may be conceded, after direction validation.
        allow_limits: bool,
    },
    /// An authenticated manual Z move, away from only its opposite limit,
    /// or up from a nozzle touching the plate.
    HeadJog {
        /// Negative native travel moves the head up.
        up: bool,
        /// Eligible head feedback accompanies bit 24 in this snapshot.
        accompanied: bool,
    },
}

impl Key {
    const fn of(source: Source) -> Self {
        match source {
            Source::Group1 { bit } => Self::Group1(bit),
            Source::Group2 { bit } => Self::Group2(bit),
            Source::Head { bit } => Self::Head(bit),
            Source::AxisDetail { axis, bit } => Self::Axis(axis, bit),
        }
    }
}

/// The monitor.
#[derive(Debug, Default)]
pub struct Monitor {
    rules: Vec<Rule>,
    head_enabled: bool,
    rows: BTreeMap<Key, Row>,
    /// The vendor's operation state the input rules are armed against: 2
    /// while a program runs, 3 while one is held, 4 during a manual
    /// operation, 0 otherwise.
    pub operation: u32,
    previous_main_class: u32,
    pause_requested: bool,
}

impl Monitor {
    /// A monitor with the machine's input rules.
    #[must_use]
    pub fn new(rules: Vec<Rule>, head_enabled: bool) -> Self {
        Self { rules, head_enabled, ..Self::default() }
    }

    /// Replaces the rules; rows of rules that no longer exist leave.
    pub fn configure(&mut self, rules: Vec<Rule>, head_enabled: bool) {
        self.rows.retain(|key, row| {
            let Key::Rule(id) = key else { return true };
            let Some(new) = rules.iter().find(|rule| rule.id == *id) else { return false };
            let retain = self.rules.iter().any(|old| old.same_binding(new));
            if retain {
                row.label.clone_from(&new.label);
            }
            retain
        });
        self.rules = rules;
        self.head_enabled = head_enabled;
    }

    /// Updates every row from one snapshot.
    pub fn observe(&mut self, snapshot: &Snapshot, details: &DetailCache) {
        let now = Instant::now();
        let status = &snapshot.status;
        let observation =
            alarms::decode(status.alarm_group_1(), status.alarm_group_2(), &details.axis_details());
        let head = alarms::head_alarms(snapshot.head.alarm_word(), self.head_enabled);
        let mut present = Vec::new();
        for alarm in observation.alarms.iter().chain(&head) {
            let key = Key::of(alarm.source);
            present.push(key);
            self.rows.entry(key).or_insert_with(|| Row {
                id: alarm.id,
                source: describe(alarm.source),
                label: alarm.label.map_or_else(|| describe(alarm.source), str::to_owned),
                blocking: true,
                latched: false,
                active: true,
                since: now,
            });
        }
        if self.head_enabled && !snapshot.head.referenced() {
            present.push(Key::HeadReference);
            self.rows.entry(Key::HeadReference).or_insert_with(|| Row {
                id: Some(HEAD_REFERENCE),
                source: "head".into(),
                label: "Head requires Home".into(),
                blocking: true,
                latched: false,
                active: true,
                since: now,
            });
        }
        let group_1 = status.alarm_group_1();
        self.rows.retain(|key, _| match key {
            // A detail row stays while its axis summary is still set.
            Key::Axis(axis, _) => present.contains(key) || group_1 & (1 << axis) != 0,
            Key::Rule(_) => true,
            _ => present.contains(key),
        });
        self.previous_main_class =
            if self.blocked().is_some() { 5 } else { snapshot.class() as u32 };
        self.evaluate_rules(snapshot, now);
    }

    fn evaluate_rules(&mut self, snapshot: &Snapshot, now: Instant) {
        let status = &snapshot.status;
        let class = snapshot.class() as u32;
        for rule in &self.rules {
            let key = Key::Rule(rule.id);
            let level = status.input(rule.input);
            let tripped = level != rule.active_low;
            let previous = self.rows.get(&key);
            let present = previous.is_some();
            let previous_active = previous.is_some_and(|row| row.active);
            let active = if let Some(valve) = rule.gas_valve {
                let update = gas_update(present, status.output(valve), !tripped, class);
                self.pause_requested |= update.pause;
                update.row_present
            } else {
                let armed =
                    custom_is_armed(rule.run_only, self.previous_main_class, self.operation);
                if (1000..=1015).contains(&rule.id) {
                    CustomInput {
                        tripped,
                        armed,
                        previous_active,
                        row_present: present,
                        only_manual_relief: rule.latch,
                    }
                    .active()
                } else {
                    armed && tripped
                }
            };
            let latched = rule.latch && (previous.is_some_and(|row| row.latched) || active);
            if active || latched {
                let row = self.rows.entry(key).or_insert_with(|| Row {
                    id: Some(rule.id),
                    source: format!("input {}", rule.input),
                    label: rule.label.clone(),
                    blocking: !rule.is_gas(),
                    latched,
                    active,
                    since: now,
                });
                row.active = active;
                row.latched = latched;
            } else {
                self.rows.remove(&key);
            }
        }
    }

    /// The vendor removes custom and gas rows once a common reset is
    /// acknowledged; a still-tripped input reasserts on the next poll.
    pub fn common_reset_acknowledged(&mut self) {
        self.rows.retain(|_, row| !row.id.is_some_and(|id| matches!(id, 1000..=1015 | 150..=158)));
        self.pause_requested = false;
    }

    /// Whether a gas feedback row was just inserted while a program ran,
    /// which the vendor answers with a pause. Reading it consumes it.
    pub fn take_pause_request(&mut self) -> bool {
        std::mem::take(&mut self.pause_requested)
    }

    /// Why operations are blocked, if a blocking row is present.
    #[must_use]
    pub fn blocked(&self) -> Option<String> {
        self.blocked_for(Concession::None)
    }

    /// As [`Monitor::blocked`] for an operation granted `concession`.
    #[must_use]
    pub fn blocked_for(&self, concession: Concession) -> Option<String> {
        let conceded = |key: &Key, row: &Row| {
            if matches!(key, Key::Rule(56 | 60))
                && matches!(
                    concession,
                    Concession::ProcessOnly
                        | Concession::ManualPositioning { .. }
                        | Concession::XyJog { .. }
                        | Concession::HeadJog { .. }
                )
            {
                return true;
            }
            if matches!(concession, Concession::XyJog { allow_limits: true, .. })
                && matches!(key, Key::Group1(0 | 1) | Key::Axis(0 | 1, 0..=3))
            {
                return true;
            }
            match concession {
                Concession::None | Concession::ProcessOnly => false,
                Concession::HeadReference => row.id == Some(HEAD_REFERENCE),
                Concession::HeadHome { other_head_faults }
                | Concession::ManualPositioning { other_head_faults }
                | Concession::XyJog { other_head_faults, .. } => {
                    matches!(row.id, Some(HEAD_REFERENCE | HEAD_REFERENCE_WARNING))
                        || (*key == Key::Group1(SPECIAL_GROUP1_BIT) && !other_head_faults)
                }
                Concession::HeadJog { up, accompanied } => {
                    matches!(key, Key::HeadReference | Key::Head(12))
                        || matches!(
                            (up, key),
                            (false, Key::Head(0 | 2)) | (true, Key::Head(1 | 3 | 5))
                        )
                        || (*key == Key::Group1(SPECIAL_GROUP1_BIT) && accompanied)
                }
            }
        };
        let labels: Vec<&str> = self
            .rows
            .iter()
            .filter(|(key, row)| row.blocking && !conceded(key, row))
            .map(|(_, row)| row.label.as_str())
            .collect();
        (!labels.is_empty()).then(|| labels.join("; "))
    }

    /// The rows, in a stable order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.values()
    }

    /// The rows as published state.
    #[must_use]
    pub fn views(&self) -> Vec<AlarmView> {
        self.rows
            .values()
            .map(|row| AlarmView {
                relief: {
                    let moves_axes =
                        row.id.is_some_and(|id| alarms::relief(id) == alarms::Relief::HeadHome);
                    crate::state::ReliefView {
                        label: if moves_axes { "Home head" } else { "Reset" }.into(),
                        moves_axes,
                    }
                },
                id: row.id,
                source: row.source.clone(),
                label: row.label.clone(),
                blocking: row.blocking,
                latched: row.latched,
                active: row.active,
                age_seconds: row.since.elapsed().as_secs(),
            })
            .collect()
    }
}

fn describe(source: Source) -> String {
    match source {
        Source::Group1 { bit } => format!("controller group 1 bit {bit}"),
        Source::Group2 { bit } => format!("controller group 2 bit {bit}"),
        Source::AxisDetail { axis, bit } => format!("axis {axis} detail bit {bit}"),
        Source::Head { bit } => format!("head bit {bit}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::tests::snapshot_with;

    fn door(latch: bool, run_only: bool) -> Rule {
        Rule {
            id: 1000,
            label: "Door alarm".into(),
            input: 4,
            active_low: true,
            run_only,
            latch,
            gas_valve: None,
        }
    }

    #[test]
    fn process_inputs_block_jobs_but_not_manual_motion_and_names_cannot_bypass() {
        let mut rules = vec![door(false, false)];
        rules[0].label = "Cooling-water alarm".into();
        for (id, input, label) in [(56, 3, "Water"), (60, 5, "Laser source")] {
            rules.push(Rule {
                id,
                input,
                label: label.into(),
                active_low: true,
                run_only: false,
                latch: false,
                gas_valve: None,
            });
        }
        let details = DetailCache::default();
        let mut monitor = Monitor::new(rules, true);
        let manual = [
            Concession::ProcessOnly,
            Concession::ManualPositioning { other_head_faults: false },
            Concession::HeadJog { up: false, accompanied: false },
            Concession::XyJog { other_head_faults: false, allow_limits: false },
        ];
        monitor.observe(&snapshot_with(&[(4, 1 << 3)], &[], &[(2, 0x8000_0001)]), &details);
        assert!(monitor.blocked().is_some());
        assert!(
            monitor.blocked_for(Concession::HeadHome { other_head_faults: false }).is_some(),
            "a job's startup head search cannot bypass process alarms"
        );
        for concession in manual {
            assert!(monitor.blocked_for(concession).is_none());
        }
        assert!(monitor.views().iter().all(|row| row.active && row.blocking));
        monitor.observe(&snapshot_with(&[], &[], &[(2, 0x8000_0001)]), &details);
        for concession in manual {
            assert!(
                monitor.blocked_for(concession).is_some(),
                "the custom input named Water is still a motion interlock"
            );
        }
    }

    /// A controller bit makes a blocking row that leaves when the bit
    /// clears; a head bit likewise, only when the head is enabled.
    #[test]
    fn controller_and_head_bits_come_and_go_with_their_rows() {
        let details = DetailCache::default();
        let referenced = (2, 0x8000_0000);
        let mut monitor = Monitor::new(vec![], true);
        monitor.observe(&snapshot_with(&[(6, 1 << 30)], &[], &[referenced]), &details);
        assert!(monitor.blocked().unwrap().contains("Emergency"));
        assert_eq!(monitor.rows().next().unwrap().id, Some(8030));
        monitor.observe(&snapshot_with(&[], &[], &[referenced]), &details);
        assert!(monitor.blocked().is_none());
        monitor.observe(&snapshot_with(&[], &[], &[referenced, (1, 1 << 8)]), &details);
        assert_eq!(monitor.rows().next().unwrap().id, Some(41));
        let mut disabled = Monitor::new(vec![], false);
        disabled.observe(&snapshot_with(&[], &[], &[(1, 1 << 8)]), &details);
        assert!(disabled.blocked().is_none());
    }

    /// An unreferenced head blocks with the vendor's row 45, which Go Origin
    /// and the mode switch see past; the head-only search also sees past the
    /// reference warning and the group 1 head summary bit, but not past a
    /// summary bit backed by another head fault.
    #[test]
    fn the_head_reference_row_is_conceded_to_the_operations_that_establish_it() {
        let details = DetailCache::default();
        let mut monitor = Monitor::new(vec![], true);
        monitor.observe(&snapshot_with(&[], &[], &[]), &details);
        assert_eq!(monitor.blocked().as_deref(), Some("Head requires Home"));
        assert_eq!(monitor.blocked_for(Concession::HeadReference), None);
        monitor.observe(&snapshot_with(&[(6, 1 << 24)], &[], &[(1, 1 << 12)]), &details);
        assert!(monitor.blocked_for(Concession::HeadReference).is_some());
        assert_eq!(monitor.blocked_for(Concession::HeadHome { other_head_faults: false }), None);
        assert!(monitor.blocked_for(Concession::HeadHome { other_head_faults: true }).is_some());
        monitor.observe(&snapshot_with(&[], &[], &[(2, 0x8000_0000)]), &details);
        assert_eq!(monitor.blocked(), None);
    }

    /// A door input trips when low, blocks, and leaves when it recovers;
    /// with manual relief it latches until a common reset is acknowledged.
    #[test]
    fn custom_inputs_follow_polarity_and_latching() {
        let details = DetailCache::default();
        let mut monitor = Monitor::new(vec![door(false, false)], false);
        monitor.observe(&snapshot_with(&[(4, 0)], &[], &[]), &details);
        assert_eq!(monitor.blocked().as_deref(), Some("Door alarm"));
        monitor.observe(&snapshot_with(&[(4, 1 << 3)], &[], &[]), &details);
        assert!(monitor.blocked().is_none());
        let mut latching = Monitor::new(vec![door(true, false)], false);
        latching.observe(&snapshot_with(&[(4, 0)], &[], &[]), &details);
        latching.observe(&snapshot_with(&[(4, 1 << 3)], &[], &[]), &details);
        assert!(latching.blocked().is_some());
        assert!(latching.rows().next().unwrap().latched);
        latching.common_reset_acknowledged();
        latching.observe(&snapshot_with(&[(4, 1 << 3)], &[], &[]), &details);
        assert!(latching.blocked().is_none());
    }

    /// A run-only input is ignored while idle and armed while a program
    /// runs or is held.
    #[test]
    fn run_only_rules_arm_with_the_operation() {
        let details = DetailCache::default();
        let mut monitor = Monitor::new(vec![door(false, true)], false);
        monitor.observe(&snapshot_with(&[(4, 0)], &[], &[]), &details);
        assert!(monitor.blocked().is_none());
        monitor.operation = 3;
        monitor.observe(&snapshot_with(&[(4, 0)], &[], &[]), &details);
        assert_eq!(monitor.blocked().as_deref(), Some("Door alarm"));
    }

    /// Gas feedback with the valve on inserts a non-blocking row and asks
    /// for a pause only while a program runs; with the valve off the row is
    /// left alone.
    #[test]
    fn gas_feedback_is_qualified_by_its_valve() {
        let details = DetailCache::default();
        let rule = Rule {
            id: 151,
            label: "Low oxygen".into(),
            input: 12,
            active_low: true,
            run_only: false,
            latch: false,
            gas_valve: Some(7),
        };
        let mut monitor = Monitor::new(vec![rule], false);
        monitor.observe(&snapshot_with(&[(4, 0), (5, 1 << 6), (19, 1)], &[], &[]), &details);
        assert!(monitor.blocked().is_none());
        assert_eq!(monitor.rows().count(), 1);
        assert!(monitor.take_pause_request());
        assert!(!monitor.take_pause_request());
        monitor.observe(&snapshot_with(&[(4, 0), (5, 0)], &[], &[]), &details);
        assert_eq!(monitor.rows().count(), 1, "the valve off retains the row");
        monitor.observe(&snapshot_with(&[(4, 1 << 11), (5, 1 << 6)], &[], &[]), &details);
        assert_eq!(monitor.rows().count(), 0);
    }

    #[test]
    fn rule_identity_survives_reordering_and_metadata_edits_but_not_rebinding() {
        let details = DetailCache::default();
        let original = door(true, false);
        let mut gas = original.clone();
        gas.id = 151;
        gas.label = "Low gas".into();
        gas.gas_valve = Some(26);
        gas.input = 12;
        gas.latch = false;
        let mut monitor = Monitor::new(vec![original.clone(), gas.clone()], false);
        let tripped = snapshot_with(&[(22, 1 << 15), (19, 1)], &[], &[]);
        monitor.observe(&tripped, &details);
        assert!(monitor.take_pause_request(), "extended valve qualifies gas feedback");
        let since = monitor.rows().find(|row| row.id == Some(1000)).unwrap().since;
        let mut renamed = original;
        renamed.label = "Door interlock".into();
        monitor.configure(vec![gas, renamed.clone()], false);
        let door = monitor.rows().find(|row| row.id == Some(1000)).unwrap();
        assert_eq!(door.label, "Door interlock");
        assert!(door.latched);
        assert_eq!(door.since, since);
        renamed.input = 5;
        monitor.configure(vec![renamed], false);
        assert_eq!(monitor.rows().count(), 0);
        monitor.observe(&tripped, &details);
        let row = monitor.rows().next().unwrap();
        assert_eq!(row.source, "input 5");
        assert!(row.active, "the currently active cause reasserts under its new binding");
    }
}
