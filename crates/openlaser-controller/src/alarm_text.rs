// SPDX-License-Identifier: GPL-3.0-or-later

//! Plain names and fixes for alarm rows.
//!
//! The vendor's labels ("Z software upper limit", "Bus Fault") and the raw
//! bits behind unlabelled rows ("controller group 1 bit 24") are accurate but
//! read like a register dump. Operators see the plain name and one line on
//! how to clear it; the vendor label and source stay in the row for details.
//! The wording is OpenLaser's own and changes no alarm behaviour.

use openlaser_protocol::alarms::Source;

/// What an operator reads for one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plain {
    /// A short name, such as "Emergency stop pressed".
    pub title: String,
    /// One sentence on what to do next.
    pub fix: String,
}

impl Plain {
    fn new(title: impl Into<String>, fix: impl Into<String>) -> Self {
        Self { title: title.into(), fix: fix.into() }
    }
}

/// The group 1 bit that accompanies head faults; see `alarms.rs`.
const HEAD_SUMMARY_BIT: u8 = 24;
/// The first group 1 bit that is a direct fault rather than an axis summary.
const FIRST_DIRECT_FAULT_BIT: u8 = 25;

/// Plain names for the axis summary bits. The vendor's X/Y jog recovery
/// concedes bits 0 and 1, which fixes those two; the rest are only numbered.
fn axis_name(axis: u8) -> String {
    match axis {
        0 => "X axis".into(),
        1 => "Y axis".into(),
        _ => format!("Axis {}", u32::from(axis) + 1),
    }
}

const RESET: &str = "Then press Reset.";

/// The plain text for a controller or head bit.
#[must_use]
pub fn for_source(source: Source) -> Plain {
    match source {
        Source::Group1 { bit: 25 } => Plain::new(
            "Controller bus fault",
            "Check the drive cables at the controller. Restart the controller if it comes back.",
        ),
        Source::Group1 { bit: 26 } => Plain::new(
            "Controller output fault",
            format!("Check the output wiring (valves, laser enable). {RESET}"),
        ),
        Source::Group1 { bit: 29 } => Plain::new(
            "Servo drive alarm",
            format!("Read the error code on the drive's display and clear it there. {RESET}"),
        ),
        Source::Group1 { bit: 30 } => Plain::new(
            "Emergency stop pressed",
            format!("Twist or pull the emergency stop button to release it. {RESET}"),
        ),
        Source::Group1 { bit: HEAD_SUMMARY_BIT } => Plain::new(
            "Head fault",
            "Home the head. If it comes back, check the other head alarms.",
        ),
        Source::Group1 { bit } if bit < FIRST_DIRECT_FAULT_BIT => Plain::new(
            format!("{} fault", axis_name(bit)),
            "The axis reports a limit or drive alarm; its cause is listed with it.",
        ),
        Source::Group1 { .. } => Plain::new(
            "Controller fault",
            "Press Reset. If it comes back, restart the controller and note the details below.",
        ),
        Source::Group2 { bit } => group_2(bit),
        Source::AxisDetail { axis, bit } => axis_detail(&axis_name(axis), bit),
        Source::Head { bit } => head(bit),
    }
}

fn group_2(bit: u8) -> Plain {
    match bit {
        5 => Plain::new(
            "Controller ran out of motion data",
            "Use a wired network link to the controller and close other busy programs.",
        ),
        7 => Plain::new(
            "Move too fast for the machine",
            "Lower the cutting or jog speed, then press Reset.",
        ),
        _ => Plain::new(
            "Controller refused a command",
            "Press Reset and try again. If it repeats, report it with the details below.",
        ),
    }
}

fn axis_detail(axis: &str, bit: u8) -> Plain {
    match bit {
        0 => Plain::new(
            format!("{axis} on its + limit switch"),
            format!("Jog the {axis} in the − direction, off the switch. {RESET}"),
        ),
        1 => Plain::new(
            format!("{axis} on its − limit switch"),
            format!("Jog the {axis} in the + direction, off the switch. {RESET}"),
        ),
        2 => Plain::new(
            format!("{axis} past its + travel limit"),
            format!("Jog the {axis} back in the − direction. {RESET}"),
        ),
        3 => Plain::new(
            format!("{axis} past its − travel limit"),
            format!("Jog the {axis} back in the + direction. {RESET}"),
        ),
        4 => Plain::new(
            format!("{axis} drive alarm"),
            format!("Read the error code on the drive's display and clear it there. {RESET}"),
        ),
        5 => Plain::new(
            format!("{axis} motors out of step"),
            "Home the machine. If it comes back, check the second drive of the gantry.",
        ),
        _ => Plain::new(
            format!("{axis} fault"),
            "Press Reset. If it comes back, report it with the details below.",
        ),
    }
}

fn head(bit: u8) -> Plain {
    let calibrate = "Clean the nozzle, then calibrate the head.";
    match bit {
        0 => Plain::new("Head on its top limit switch", format!("Jog the head down. {RESET}")),
        1 => Plain::new("Head on its bottom limit switch", format!("Jog the head up. {RESET}")),
        2 => Plain::new("Head past its top travel limit", format!("Jog the head down. {RESET}")),
        3 => Plain::new("Head past its bottom travel limit", format!("Jog the head up. {RESET}")),
        4 => Plain::new(
            "Head drive alarm",
            format!("Read the error code on the head drive and clear it there. {RESET}"),
        ),
        5 => Plain::new(
            "Nozzle touching the sheet",
            "Jog the head up, check the sheet lies flat, then press Reset.",
        ),
        6 => Plain::new(
            "Head encoder fault",
            "Home the head. If it comes back, check the head motor's encoder cable.",
        ),
        7 | 10 => Plain::new(
            "Head sensor cable fault",
            "Check the sensor cable and the nozzle connection, then home the head.",
        ),
        8 => Plain::new(
            "Head could not follow the sheet",
            "Check the sheet lies flat and slow the cut down, then press Reset.",
        ),
        9 => Plain::new("Head sensor signal too weak", calibrate),
        11 => Plain::new("Head controller not ready", "Restart the head controller."),
        12 => Plain::new("Head needs homing", "Home the head."),
        13 => Plain::new("No head sensor signal", format!("Check the sensor cable. {calibrate}")),
        14 => Plain::new(
            "Head sensor signal unstable",
            "Clean the nozzle and ceramic ring of spatter, then calibrate the head.",
        ),
        _ => Plain::new(
            "Head fault",
            "Home the head. If it comes back, report it with the details below.",
        ),
    }
}

/// The plain text for the missing head reference row.
#[must_use]
pub fn head_reference() -> Plain {
    Plain::new("Head needs homing", "Home the head.")
}

/// The plain text for a host input rule with the vendor's `id` and the
/// label the machine files give it.
#[must_use]
pub fn for_rule(id: u32, label: &str) -> Plain {
    match id {
        56 => Plain::new(
            "Cooling water fault",
            "Check the chiller is on, full and at temperature. Then press Reset.",
        ),
        60 => Plain::new(
            "Laser source alarm",
            "Read the alarm on the laser source and clear it there. Then press Reset.",
        ),
        150..=158 => Plain::new(
            format!("Low gas pressure ({label})"),
            "Open the cylinder valve or change the cylinder, and check the regulator.",
        ),
        _ => Plain::new(label, format!("Check the {}. {RESET}", label.to_lowercase())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_bits_get_plain_names() {
        assert_eq!(for_source(Source::Group1 { bit: 24 }).title, "Head fault");
        assert_eq!(for_source(Source::Group1 { bit: 30 }).title, "Emergency stop pressed");
        assert_eq!(for_source(Source::Group1 { bit: 3 }).title, "Axis 4 fault");
        assert_eq!(
            for_source(Source::AxisDetail { axis: 0, bit: 1 }).title,
            "X axis on its − limit switch"
        );
        assert_eq!(for_source(Source::Head { bit: 12 }).title, "Head needs homing");
    }

    #[test]
    fn every_bit_has_a_fix_and_no_register_words() {
        let sources = (0..32u8)
            .flat_map(|bit| [Source::Group1 { bit }, Source::Group2 { bit }, Source::Head { bit }])
            .chain(
                (0..25).flat_map(|axis| (0..32).map(move |bit| Source::AxisDetail { axis, bit })),
            );
        for source in sources {
            let plain = for_source(source);
            assert!(!plain.fix.is_empty(), "{source:?}");
            assert!(!plain.title.contains("bit"), "{source:?}: {}", plain.title);
            assert!(!plain.title.contains("group"), "{source:?}: {}", plain.title);
        }
    }

    #[test]
    fn custom_inputs_keep_their_label() {
        let door = for_rule(1000, "Door alarm");
        assert_eq!(door.title, "Door alarm");
        assert_eq!(door.fix, "Check the door alarm. Then press Reset.");
        assert_eq!(for_rule(56, "Water").title, "Cooling water fault");
    }
}
