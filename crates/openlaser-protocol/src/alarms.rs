// SPDX-License-Identifier: GPL-3.0-or-later

//! The alarm catalogue and the host rules the vendor applies to it.
//!
//! Alarms arrive as bits: two controller aggregates in the status block, a
//! detail word per axis, and the head alarm word. The vendor turns bits into
//! numbered rows with labels, decides whether a row pauses or stops a job,
//! and offers each row a relief action. All of that is reproduced here as
//! pure functions; keeping the rows over time is the controller crate's job.
//!
//! Evidence: MainApp `0x0059_3b90` to `0x0059_4340` (decode), `0x0059_3f5e`
//! (head rows), `0x0059_39e7` (dispatch), `0x0055_6040` (relief table),
//! `0x005e_5440` (retention), `0x005c_b840` (gas), `0x0059_65dd` (count);
//! NCModule `0x1003_91a0` (cache layout) and `0x1002_bd80` (head getter).

use crate::registers::{self, Block};

/// Where an alarm bit came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A bit of controller aggregate group 1.
    Group1 {
        /// The bit number.
        bit: u8,
    },
    /// A bit of controller aggregate group 2.
    Group2 {
        /// The bit number.
        bit: u8,
    },
    /// A bit of one axis detail word.
    AxisDetail {
        /// The axis index, 0 to 24 across the vendor's cache aliases.
        axis: u8,
        /// The bit number.
        bit: u8,
    },
    /// A bit of the head alarm word.
    Head {
        /// The bit number.
        bit: u8,
    },
}

/// One alarm row as the vendor would show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Alarm {
    /// The bit the row came from.
    pub source: Source,
    /// The vendor's label, if it emits one for this bit.
    pub label: Option<&'static str>,
    /// The vendor's numeric id, if it emits a row. A raw bit with no id still
    /// blocks operation.
    pub id: Option<u32>,
}

/// Everything decoded from one poll of the controller aggregates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observation {
    /// Group 1 as read.
    pub group_1: u32,
    /// Group 2 as read.
    pub group_2: u32,
    /// The rows, in the vendor's emission order.
    pub alarms: Vec<Alarm>,
    /// Axis summaries whose detail word was not available.
    pub missing_axis_details: Vec<u8>,
    /// Each raw detail word, including the axis reference and motion fields.
    pub axis_detail_words: Vec<(u8, u32)>,
}

const GROUP_2_LABELS: [&str; 8] = [
    "Illegal command",
    "The interpolation data content length is abnormal",
    "Axis control command execution exception",
    "FTC order execution exception",
    "PLC command execution exception",
    "FIFO starvation",
    "Immediate instruction exception",
    "Overspeed",
];

const AXIS_LABELS: [&str; 6] = [
    "Hard positive limit",
    "Hard negative limit",
    "Soft positive limit",
    "Soft negative limit",
    "Servo input alarm",
    "Dual drive alarm",
];

/// The head alarm word's low 16 bits: the vendor's id for each, or `None`
/// where it blocks without a row.
const HEAD_IDS: [Option<u32>; 16] = [
    Some(33),
    Some(34),
    Some(35),
    Some(36),
    Some(37),
    Some(38),
    Some(39),
    Some(40),
    Some(41),
    Some(42),
    Some(43),
    Some(81),
    Some(99),
    Some(100),
    Some(59),
    None,
];

const HEAD_LABELS: [&str; 16] = [
    "Z hardware upper limit",
    "Z hardware lower limit",
    "Z software upper limit",
    "Z software lower limit",
    "Z servo input alarm",
    "Cutting head touching plate",
    "Z encoder alarm",
    "Head signal wire alarm (bit 7)",
    "Head following error",
    "Head capacitance variation too small",
    "Head signal wire alarm (bit 10)",
    "Z FPGA not loaded",
    "Z axis value warning",
    "Z signal is zero",
    "Z signal abnormal fluctuation",
    "Unclassified Z alarm bit 15",
];

/// Decodes the two aggregates and the axis detail words into rows.
///
/// `axis_details` is indexed by axis: entry `axis` is `Some(word)` when the
/// detail word for that axis was read. Group 1 bits 0 to 24 summarise axes;
/// bits 25 to 31 are direct faults with ids 8025 to 8031. Axis detail bits get
/// id `8100 + 32 * axis + bit`; group 2 bits get `9000 + bit`.
#[must_use]
pub fn decode(group_1: u32, group_2: u32, axis_details: &[Option<u32>]) -> Observation {
    let mut observation = Observation { group_1, group_2, ..Default::default() };
    for bit in 0..32u8 {
        if group_1 & (1 << bit) == 0 {
            continue;
        }
        let label = match bit {
            25 => Some("Bus Fault"),
            26 => Some("Output Fault"),
            29 => Some("Servo input alarm"),
            30 => Some("Emergency stop alarm"),
            _ => None,
        };
        observation.alarms.push(Alarm {
            source: Source::Group1 { bit },
            label,
            id: (bit >= 25).then_some(8000 + u32::from(bit)),
        });
        if bit > 24 {
            continue;
        }
        match axis_details.get(usize::from(bit)).copied().flatten() {
            Some(word) => {
                observation.axis_detail_words.push((bit, word));
                // Real axis records share their first word with reference,
                // phase and state feedback. The native alarm loop reads only
                // its six fault bits; moving/referenced flags are not alarms.
                let faults = if bit < 5 { word & 0x3f } else { word };
                for fault in 0..32u8 {
                    if faults & (1 << fault) != 0 {
                        observation.alarms.push(Alarm {
                            source: Source::AxisDetail { axis: bit, bit: fault },
                            label: AXIS_LABELS.get(usize::from(fault)).copied(),
                            id: (fault < 6)
                                .then_some(8100 + 32 * u32::from(bit) + u32::from(fault)),
                        });
                    }
                }
            }
            None => observation.missing_axis_details.push(bit),
        }
    }
    for bit in 0..32u8 {
        if group_2 & (1 << bit) != 0 {
            observation.alarms.push(Alarm {
                source: Source::Group2 { bit },
                label: GROUP_2_LABELS.get(usize::from(bit)).copied(),
                id: (bit < 8).then_some(9000 + u32::from(bit)),
            });
        }
    }
    observation
}

/// The head alarm bits that matter: the low 16 bits, and only when a head is
/// configured. The upper 16 bits are masked by the vendor's getter.
#[must_use]
pub const fn head_word(raw: u32, configured: bool) -> u32 {
    if configured { raw & 0xffff } else { 0 }
}

/// The head rows for a raw head alarm word. Bit 15 blocks without an id.
#[must_use]
pub fn head_alarms(raw: u32, configured: bool) -> Vec<Alarm> {
    let word = head_word(raw, configured);
    (0..16u8)
        .filter(|bit| word & (1 << bit) != 0)
        .map(|bit| Alarm {
            source: Source::Head { bit },
            label: Some(HEAD_LABELS[usize::from(bit)]),
            id: HEAD_IDS[usize::from(bit)],
        })
        .collect()
}

// Axis detail cache ----------------------------------------------------------

/// A word in the vendor's alarm cache, addressed by cache group and index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CacheWord {
    /// The cache group: which block it was read from.
    pub group: u8,
    /// The word index within the group.
    pub index: usize,
}

/// Where the vendor reads the detail word for axis summary bit `axis`.
///
/// Only indices 0 to 4 are real axes. The vendor's loop runs to 24 and walks
/// into adjacent caches; reproducing that makes the same ids appear, it does
/// not mean more axes exist.
/// Bit 24 aliases group 10 word 20, in the optional autofocus property cache
/// filled by NCModule `0x1005_0030`. This is separate from the Z head alarm
/// word at 10000/1 and does not establish a controller wire address for it.
#[must_use]
pub const fn detail_word(axis: usize) -> Option<CacheWord> {
    let (group, index) = match axis {
        0..=4 => (3, axis * 10),
        5 => (1, 0),
        6..=12 => (4, (axis - 6) * 10 + 9),
        13..=14 => (5, (axis - 13) * 10 + 9),
        15..=16 => (7, (axis - 15) * 10 + 3),
        17..=20 => (8, (axis - 17) * 10 + 5),
        21 => (9, 4),
        22..=24 => (10, (axis - 22) * 10),
        _ => return None,
    };
    Some(CacheWord { group, index })
}

/// The vendor's alarm cache: the blocks it copies from the wire, by group.
/// An absent group reads as zero, as the vendor's zero-initialised storage
/// does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DetailCache {
    groups: [Vec<u32>; 11],
}

impl DetailCache {
    /// Records a block read, returning whether the vendor caches that block.
    /// The parameter block is stored as the first 14 words of each record.
    pub fn observe(&mut self, address: u32, words: &[u32]) -> bool {
        let group = match (address, words.len()) {
            (1000, 36) => 0,
            (5000, 1) => 1,
            (2000, 50) => 3,
            (50_200, 100) => {
                self.groups[4] = words
                    .chunks_exact(20)
                    .flat_map(|record| record[..14].iter().copied())
                    .collect();
                return true;
            }
            (50_000, 26) => 5,
            (10_000, 18) => 7,
            (11_000, 41) => 8,
            _ => return false,
        };
        self.groups[group] = words.to_vec();
        true
    }

    /// One cache word, zero when its group was never read.
    #[must_use]
    pub fn word(&self, word: CacheWord) -> u32 {
        self.groups
            .get(usize::from(word.group))
            .and_then(|group| group.get(word.index))
            .copied()
            .unwrap_or(0)
    }

    /// The detail word for axis summary bit `axis`, or `None` above 24.
    #[must_use]
    pub fn axis_detail(&self, axis: usize) -> Option<u32> {
        detail_word(axis).map(|word| self.word(word))
    }

    /// The detail words for axes 0 to 24, ready for [`decode`].
    #[must_use]
    pub fn axis_details(&self) -> Vec<Option<u32>> {
        (0..25).map(|axis| self.axis_detail(axis)).collect()
    }
}

/// The extra blocks to read when group 1 summarises axes beyond the five in
/// the standard poll: their detail words live in other caches.
#[must_use]
pub fn additional_reads(group_1: u32) -> Vec<Block> {
    let mut reads = Vec::new();
    if group_1 & (1 << 5) != 0 {
        reads.push(registers::DETAIL_GROUP_1);
    }
    if group_1 & 0x1fc0 != 0 {
        reads.push(registers::PARAMETERS);
    }
    if group_1 & 0x001e_0000 != 0 {
        reads.push(registers::DETAIL_GROUP_8);
    }
    reads
}

// Host rules -----------------------------------------------------------------

/// What the vendor does to a running job when a blocking alarm is reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Action {
    /// Nothing.
    #[default]
    None,
    /// Pause the job for explicit recovery.
    Pause,
    /// Stop the job.
    Stop,
}

/// The vendor's flags that decide what a blocking alarm does to the job.
#[allow(clippy::struct_excessive_bools, reason = "the vendor's flags, reproduced as it reads them")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dispatch {
    /// Whether the reported alarm blocks.
    pub blocking: bool,
    /// The vendor's operation state: 2 a running cut, 3 a paused job, 4 a
    /// manual operation, 11 a special state that never reacts.
    pub operation: u32,
    /// Whether the processing count has finished.
    pub processing_count_finished: bool,
    /// Set by the stationary timed-laser routine, not the ordinary cut flag.
    pub force_stop: bool,
    /// The operator's bypass flag.
    pub manual_bypass: bool,
}

impl Dispatch {
    /// What the vendor does to the job for these flags.
    #[must_use]
    pub const fn action(self) -> Action {
        if !self.blocking || matches!(self.operation, 3 | 11) {
            return Action::None;
        }
        if self.processing_count_finished {
            if self.operation == 2 || (self.operation == 4 && !self.manual_bypass) {
                Action::Stop
            } else {
                Action::None
            }
        } else if self.force_stop {
            Action::Stop
        } else if self.manual_bypass {
            Action::None
        } else {
            Action::Pause
        }
    }
}

/// The relief the vendor's alarm dialog offers for an id. A user action, never
/// permission to move or reconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relief {
    /// Re-establish the controller connection.
    Reconnect,
    /// The common alarm reset.
    CommonClear,
    /// Clear the FIFO, then the common reset.
    ClearFifoThenCommon,
    /// Bus reset, then the common reset.
    ResetBusThenCommon,
    /// Dual-drive reset with restored limits, then the home question.
    DualDriveAndHomeDecision,
    /// Reset the laser source.
    SourceReset,
    /// Head reference search only.
    HeadHome,
    /// Direct the operator to the pulse-equivalent setting.
    CheckPulseEquivalent,
    /// Autofocus reference, which this machine does not have.
    AutofocusHome,
    /// Unlock the laser.
    UnlockLaser,
    /// The autofocus home dialog, which this machine does not have.
    AutofocusHomeDialog,
    /// Dismiss a maintenance reminder row.
    ServiceItem,
    /// No direct handler; the controller aggregate tail may still clear it.
    NoDirectHandler,
}

/// The vendor's relief table by alarm id.
#[must_use]
pub const fn relief(id: u32) -> Relief {
    match id {
        0 | 58 | 61 | 62 | 82 => Relief::Reconnect,
        9..=12 => Relief::DualDriveAndHomeDecision,
        13 => Relief::ClearFifoThenCommon,
        8025 => Relief::ResetBusThenCommon,
        8100..=8799 if (id - 8100) % 32 == 5 => Relief::DualDriveAndHomeDecision,
        1..=8 | 14..=32 | 46..=55 | 150..=159 | 1000..=1015 | 8000..=9999 => Relief::CommonClear,
        44 => Relief::SourceReset,
        45 | 99 => Relief::HeadHome,
        63 => Relief::CheckPulseEquivalent,
        64..=79 => Relief::AutofocusHome,
        80 => Relief::UnlockLaser,
        102 => Relief::AutofocusHomeDialog,
        3000..=3199 => Relief::ServiceItem,
        _ => Relief::NoDirectHandler,
    }
}

/// After any relief handler the vendor sends a common reset if either
/// controller aggregate was set when the dialog opened, even when that
/// duplicates the handler's own reset. Do not deduplicate it.
#[must_use]
pub const fn relief_has_controller_tail(group_1: u32, group_2: u32) -> bool {
    group_1 != 0 || group_2 != 0
}

/// Rows the ordinary clear leaves in place: gas rows, maintenance rows, and
/// custom input rows when their relief is manual only.
#[must_use]
pub const fn retained_by_ordinary_clear(id: u32, only_manual_custom_relief: bool) -> bool {
    matches!(id, 150..=159 | 3000..=3199)
        || (matches!(id, 1000..=1015) && only_manual_custom_relief)
}

/// Whether a run-only custom input is armed: always when not run-only,
/// otherwise while a job runs or is paused.
#[must_use]
pub const fn custom_is_armed(run_only: bool, previous_main_class: u32, operation: u32) -> bool {
    !run_only || (previous_main_class != 0 && operation != 0) || operation == 3
}

/// The inputs to the vendor's custom-input retention rule.
#[allow(clippy::struct_excessive_bools, reason = "the vendor's flags, reproduced as it reads them")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CustomInput {
    /// Whether the physical input is tripped on this poll.
    pub tripped: bool,
    /// Whether the rule is armed; see [`custom_is_armed`].
    pub armed: bool,
    /// Whether the row was active on the previous poll.
    pub previous_active: bool,
    /// Whether the row is currently present.
    pub row_present: bool,
    /// Whether the row's relief is manual only.
    pub only_manual_relief: bool,
}

impl CustomInput {
    /// The row's active flag after this poll, including the vendor's
    /// no-update branch when a run-only input stays tripped while unarmed.
    #[must_use]
    pub const fn active(self) -> bool {
        if !self.tripped {
            self.row_present && self.only_manual_relief
        } else if self.armed {
            true
        } else {
            self.previous_active
        }
    }
}

/// The outcome of one gas feedback check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GasUpdate {
    /// Whether the gas alarm row is present after the check.
    pub row_present: bool,
    /// Whether this check pauses the job.
    pub pause: bool,
}

/// The vendor's gas rule: an off valve leaves the row as it is; healthy
/// feedback with the valve on removes it; only a newly inserted row pauses,
/// and only when the raw CNC class is 4.
#[must_use]
pub const fn gas_update(
    row_present: bool,
    valve_on: bool,
    input_healthy: bool,
    cnc_class: u32,
) -> GasUpdate {
    if !valve_on {
        return GasUpdate { row_present, pause: false };
    }
    GasUpdate {
        row_present: !input_healthy,
        pause: !input_healthy && !row_present && cnc_class == 4,
    }
}

/// The processing-count alarm: completed at or above planned, compared as
/// signed values, and the configured action is exactly 2.
#[must_use]
pub const fn processing_count_finished(completed: i32, planned: i32, action: i32) -> bool {
    completed >= planned && action == 2
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Group 1 direct faults get ids 8025 and up with the four known labels;
    /// axis summary bits emit their detail rows with ids 8100 + 32·axis + bit
    /// and keep the raw detail word; group 2 bits get 9000 + bit.
    #[test]
    fn aggregates_decode_into_the_vendor_rows() {
        let details = [Some(0x8000_0001), None];
        let observation = decode((1 << 30) | 1 | (1 << 1), 1 << 5, &details);
        assert_eq!(
            observation.alarms[0],
            Alarm { source: Source::Group1 { bit: 0 }, label: None, id: None }
        );
        assert_eq!(
            observation.alarms[1],
            Alarm {
                source: Source::AxisDetail { axis: 0, bit: 0 },
                label: Some("Hard positive limit"),
                id: Some(8100)
            }
        );
        assert_eq!(observation.alarms.len(), 5);
        assert_eq!(observation.alarms[2].source, Source::Group1 { bit: 1 });
        assert_eq!(
            observation.alarms[3],
            Alarm {
                source: Source::Group1 { bit: 30 },
                label: Some("Emergency stop alarm"),
                id: Some(8030)
            }
        );
        assert_eq!(
            observation.alarms[4],
            Alarm {
                source: Source::Group2 { bit: 5 },
                label: Some("FIFO starvation"),
                id: Some(9005)
            }
        );
        assert_eq!(observation.missing_axis_details, [1]);
        assert_eq!(observation.axis_detail_words, [(0, 0x8000_0001)]);
        assert_eq!(decode(0, 0, &[]), Observation::default());
    }

    #[test]
    fn axis_motion_and_reference_fields_are_not_alarm_bits() {
        for word in [0x0200_c001, 0x0301_c001, 0x0402_8001] {
            let observation = decode(1, 0, &[Some(word)]);
            assert_eq!(observation.alarms.len(), 2);
            assert_eq!(observation.alarms[1].id, Some(8100));
            assert_eq!(observation.axis_detail_words, [(0, word)]);
        }
        let unexplained = decode(1, 0, &[Some(0x0200_c000)]);
        assert_eq!(unexplained.alarms.len(), 1);
        assert_eq!(unexplained.alarms[0].source, Source::Group1 { bit: 0 });
    }

    /// Axis index 24 still emits its detail id, 8868 and up, even though the
    /// vendor's label and relief ranges stop at 8799.
    #[test]
    fn axis_twenty_four_keeps_its_id() {
        let mut details = vec![None; 25];
        details[24] = Some(0b10_0000);
        let observation = decode(1 << 24, 0, &details);
        assert_eq!(observation.alarms.len(), 2);
        assert_eq!(observation.alarms[1].id, Some(8868 + 5));
        assert_eq!(observation.alarms[1].label, Some("Dual drive alarm"));
    }

    /// Head bits follow the vendor's id table, bit 15 blocks without an id,
    /// the upper half of the word is ignored, and an unconfigured head yields
    /// nothing.
    #[test]
    fn head_rows_follow_the_id_table_and_low_half() {
        let ids = [33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 81, 99, 100, 59];
        for (bit, id) in ids.into_iter().enumerate() {
            let rows = head_alarms(1 << bit, true);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, Some(id), "bit {bit}");
            assert_eq!(rows[0].source, Source::Head { bit: u8::try_from(bit).unwrap() });
        }
        assert_eq!(head_alarms(0x8000, true)[0].id, None);
        assert!(head_alarms(0xffff_0000, true).is_empty());
        assert!(head_alarms(u32::MAX, false).is_empty());
        assert_eq!(head_alarms(1 << 13, true)[0].label, Some("Z signal is zero"));
    }

    /// Every alias index maps to the vendor object offset 0x120 + 40·axis, so
    /// the cache walk lands where the native loop lands.
    #[test]
    fn detail_aliases_land_on_the_native_offsets() {
        let bases = |group: u8| match group {
            1 => 0x1e8,
            3 => 0x120,
            4 => 0x1ec,
            5 => 0x304,
            7 => 0x36c,
            8 => 0x3b4,
            9 => 0x458,
            10 => 0x490,
            _ => unreachable!(),
        };
        for axis in 0..25 {
            let word = detail_word(axis).unwrap();
            assert_eq!(bases(word.group) + word.index * 4, 0x120 + axis * 40, "axis {axis}");
        }
        assert_eq!(detail_word(25), None);
    }

    /// The cache keeps only the 14 meaningful words of each parameter record,
    /// aliases beyond the real axes read from it, and unread groups are zero.
    #[test]
    fn cache_drops_parameter_padding_and_defaults_to_zero() {
        let mut cache = DetailCache::default();
        let words: Vec<u32> = (0..100).collect();
        assert!(cache.observe(50_200, &words));
        assert!(!cache.observe(60_001, &[0; 120]));
        for index in 0..70 {
            assert_eq!(
                cache.word(CacheWord { group: 4, index }),
                u32::try_from((index / 14) * 20 + index % 14).unwrap()
            );
        }
        for axis in 6..=12 {
            let index = (axis - 6) * 10 + 9;
            assert_eq!(
                cache.axis_detail(axis),
                Some(u32::try_from((index / 14) * 20 + index % 14).unwrap())
            );
        }
        for axis in 21..25 {
            assert_eq!(cache.axis_detail(axis), Some(0));
        }
        assert_eq!(cache.axis_details().len(), 25);
    }

    /// Only the summary bits that point outside the standard poll trigger
    /// extra reads, each to the block the vendor refreshes.
    #[test]
    fn additional_reads_follow_the_summary_bits() {
        assert!(additional_reads(0x1f).is_empty());
        assert_eq!(additional_reads(1 << 5), [registers::DETAIL_GROUP_1]);
        assert_eq!(additional_reads(1 << 7), [registers::PARAMETERS]);
        assert_eq!(additional_reads(1 << 18), [registers::DETAIL_GROUP_8]);
        assert_eq!(additional_reads(0x001e_1fe0).len(), 3);
    }

    /// Dispatch keeps the vendor's exceptions: paused and state 11 never
    /// react; a finished processing count stops only running and manual
    /// operations; force-stop stops; bypass silences; otherwise pause.
    #[test]
    fn dispatch_preserves_the_operation_exceptions() {
        let dispatch = |blocking, operation, finished, force, bypass| {
            Dispatch {
                blocking,
                operation,
                processing_count_finished: finished,
                force_stop: force,
                manual_bypass: bypass,
            }
            .action()
        };
        for operation in 0..=20 {
            assert_eq!(dispatch(false, operation, false, false, false), Action::None);
            let paused = matches!(operation, 3 | 11);
            assert_eq!(
                dispatch(true, operation, false, false, false),
                if paused { Action::None } else { Action::Pause }
            );
            assert_eq!(
                dispatch(true, operation, false, true, false),
                if paused { Action::None } else { Action::Stop }
            );
            assert_eq!(
                dispatch(true, operation, true, false, false),
                if matches!(operation, 2 | 4) { Action::Stop } else { Action::None }
            );
            assert_eq!(dispatch(true, operation, false, false, true), Action::None);
            assert_eq!(
                dispatch(true, operation, true, false, true),
                if operation == 2 { Action::Stop } else { Action::None }
            );
        }
    }

    /// The relief table: representative ids from every branch, including the
    /// dual-drive detail bit pattern and the unhandled Z signal row.
    #[test]
    fn relief_table_matches_the_dialog_dispatch() {
        assert_eq!(relief(0), Relief::Reconnect);
        assert_eq!(relief(57), Relief::NoDirectHandler);
        assert_eq!(relief(1), Relief::CommonClear);
        assert_eq!(relief(13), Relief::ClearFifoThenCommon);
        assert_eq!(relief(8025), Relief::ResetBusThenCommon);
        assert_eq!(relief(8105), Relief::DualDriveAndHomeDecision);
        assert_eq!(relief(8868 + 5), Relief::CommonClear);
        assert_eq!(relief(44), Relief::SourceReset);
        assert_eq!(relief(45), Relief::HeadHome);
        assert_eq!(relief(99), Relief::HeadHome);
        assert_eq!(relief(63), Relief::CheckPulseEquivalent);
        assert_eq!(relief(100), Relief::NoDirectHandler);
        assert_eq!(relief(151), Relief::CommonClear);
        assert_eq!(relief(3010), Relief::ServiceItem);
        assert!(!relief_has_controller_tail(0, 0));
        assert!(relief_has_controller_tail(1 << 24, 0));
    }

    /// Retention, arming, custom activity and gas rules reproduce their
    /// separate native paths, including the no-update branch.
    #[test]
    fn retention_and_input_rules_match_the_native_paths() {
        let custom_active = |tripped, armed, previous_active, row_present, only_manual_relief| {
            CustomInput { tripped, armed, previous_active, row_present, only_manual_relief }
                .active()
        };
        for id in 0..10_000 {
            let gas_or_service = matches!(id, 150..=159 | 3000..=3199);
            assert_eq!(retained_by_ordinary_clear(id, false), gas_or_service);
            assert_eq!(
                retained_by_ordinary_clear(id, true),
                gas_or_service || matches!(id, 1000..=1015)
            );
        }
        assert!(!custom_is_armed(true, 0, 2));
        assert!(custom_is_armed(true, 4, 2));
        assert!(custom_is_armed(true, 0, 3));
        assert!(!custom_is_armed(true, 5, 0));
        assert!(custom_is_armed(false, 0, 0));
        assert!(custom_active(false, false, true, true, true));
        assert!(!custom_active(false, false, true, true, false));
        assert!(custom_active(true, false, true, true, false));
        assert!(!custom_active(true, false, false, false, false));
        assert_eq!(gas_update(false, true, false, 4), GasUpdate { row_present: true, pause: true });
        assert_eq!(
            gas_update(true, false, false, 4),
            GasUpdate { row_present: true, pause: false }
        );
        assert_eq!(gas_update(true, true, true, 4), GasUpdate { row_present: false, pause: false });
        assert_eq!(gas_update(true, true, false, 4), GasUpdate { row_present: true, pause: false });
        assert_eq!(
            gas_update(false, true, false, 0),
            GasUpdate { row_present: true, pause: false }
        );
    }

    /// The processing count compares signed values and only action 2 finishes.
    #[test]
    fn processing_count_is_signed_and_needs_action_two() {
        for (completed, planned, met) in [
            (0, 0, true),
            (1, 2, false),
            (2, 2, true),
            (3, 2, true),
            (-1, 0, false),
            (0, -1, true),
            (i32::MIN, i32::MAX, false),
        ] {
            for action in -1..=4 {
                assert_eq!(
                    processing_count_finished(completed, planned, action),
                    met && action == 2
                );
            }
        }
    }
}
