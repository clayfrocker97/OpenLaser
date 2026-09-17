// SPDX-License-Identifier: GPL-3.0-or-later

//! One length-checked wire-layout table, including exact headers and reserved
//! words. Semantic field validation remains shared with record construction.

use super::{Record, RecordError};
use crate::requests::{LaserChannel, OutputBank};

pub(super) fn record(words: &[u32], at: usize) -> Result<(Record, usize), RecordError> {
    let header = *words.get(at).ok_or(RecordError::Truncated { at })?;
    let payload_bytes = usize::try_from(header >> 16).unwrap_or(usize::MAX);
    if !payload_bytes.is_multiple_of(4) {
        return Err(RecordError::Unaligned { at, header });
    }
    let end = at + 1 + payload_bytes / 4;
    let body = words.get(at + 1..end).ok_or(RecordError::Truncated { at })?;
    let unknown = RecordError::Unknown { at, header };
    let decoded = match (header, body) {
        (0x0008_0bb8, &[first, second]) => {
            let a = first.to_le_bytes();
            let b = second.to_le_bytes();
            if b[1] != 0 {
                return Err(unknown);
            }
            Record::Move {
                dx: a[0].cast_signed(),
                dy: a[1].cast_signed(),
                laser: [a[2], a[3], b[0], b[2], b[3]],
            }
        }
        (0x0000_0bb9, []) => Record::Barrier,
        (0x0004_0bba, &[tag]) => Record::Item(tag),
        (0x0008_270f, &[18, value]) => {
            Record::ContourControl(u16::try_from(value).map_err(|_| unknown)?)
        }
        (0x0010_270f, &[selector, frequency, power, enabled]) => {
            pwm(selector, frequency, power, enabled, unknown)?
        }
        (0x000c_270f, &[selector @ (2 | 13), mask, values]) => Record::Outputs {
            bank: if selector == 2 { OutputBank::Standard } else { OutputBank::Extended },
            mask: u16::try_from(mask).map_err(|_| unknown)?,
            values: u16::try_from(values).map_err(|_| unknown)?,
        },
        (0x000c_270f, &[4, channel @ (0 | 1), value]) => {
            Record::Analog { channel: u8::try_from(channel + 1).map_err(|_| unknown)?, value }
        }
        (0x0008_07d1, &[mode, timeout_ms]) if mode & 0xffff_ff00 == 0x0300_0000 => {
            Record::WaitStatus {
                selector: u8::try_from(mode & 0xff).map_err(|_| unknown)?,
                timeout_ms,
            }
        }
        (0x0008_07d1, &[millis, 3000]) => Record::Delay { millis },
        (0x0008_0067, &[speed_tenths, height_microns]) => {
            Record::HeightAbsolute { speed_tenths, height_microns }
        }
        (0x0008_0068, &[speed_tenths, height]) => Record::Follow { speed_tenths, height },
        (0x0008_006d, &[speed_tenths, delta]) => {
            Record::Lift { speed_tenths, delta_microns: delta.cast_signed() }
        }
        (0x0010_0069, &[follow_speed_tenths, height, lift_speed_tenths, lift]) => {
            Record::PierceHeight {
                follow_speed_tenths,
                height,
                lift_speed_tenths,
                lift_microns: lift.cast_signed(),
            }
        }
        (0x0010_006c, &[speed_tenths, descent, cut_height_microns, 1]) => Record::PierceRamp {
            speed_tenths,
            descent_microns: descent.cast_signed(),
            cut_height_microns,
        },
        (0x0018_006a, &[speed_tenths, lift, hold_ms, 0, down, target]) => Record::FrogJump {
            speed_tenths,
            lift,
            hold_ms,
            down_speed_tenths: u16::try_from(down).map_err(|_| unknown)?,
            target,
        },
        (0x000c_0076, &[4, value, height]) => Record::CrashProtect { value, height },
        _ => return Err(unknown),
    };
    decoded.validate().map_err(|_| unknown)?;
    Ok((decoded, end - at))
}

fn pwm(
    selector: u32,
    frequency: u32,
    power: u32,
    enabled: u32,
    unknown: RecordError,
) -> Result<Record, RecordError> {
    let channel = match selector {
        3 => LaserChannel::Primary,
        17 => LaserChannel::Secondary,
        _ => return Err(unknown),
    };
    match (u16::try_from(frequency), u8::try_from(power)) {
        (Ok(frequency), Ok(power)) if enabled == u32::from(power != 0) => {
            Ok(Record::Pwm { channel, frequency, power })
        }
        _ => Err(unknown),
    }
}
