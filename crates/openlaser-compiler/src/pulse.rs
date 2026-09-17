// SPDX-License-Identifier: GPL-3.0-or-later

//! A bounded stationary manual pulse. The complete enable, delay and disable
//! sequence is queued together; a host timer never holds the laser on.

use crate::program::{Kind, Program, Section};
use crate::{Error, Result};
use openlaser_protocol::records::Record;
use openlaser_protocol::requests::{LaserChannel, OutputPort};

/// Manual laser values resolved from the machine files by the XML binder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Primary Fiber PWM or the admitted secondary CO2 PWM branch.
    pub channel: LaserChannel,
    /// Queued PWM frequency, in hertz.
    pub frequency: u16,
    /// Requested PWM power, in percent.
    pub power: u8,
    /// Optional analog peak-current output, channel and encoded value.
    pub analog: Option<(u8, u32)>,
    /// Shutter output; zero when unassigned.
    pub gate: u8,
    /// Laser-enable output; zero when unassigned.
    pub output: u8,
}

fn digital(port: u8, on: bool) -> Result<Record> {
    let OutputPort { bank, mask } =
        OutputPort::new(port).map_err(|_| Error::Invalid("invalid manual laser output"))?;
    Ok(Record::Outputs { bank, mask, values: if on { mask } else { 0 } })
}

/// The pulse program, with zero motion and an unconditional output-off tail.
pub fn program(duration_ms: u32, s: &Settings) -> Result<Program> {
    if !(10..=1000).contains(&duration_ms) || !(1..=100).contains(&s.power) {
        return Err(Error::Invalid("a pulse needs 10–1000 ms and 1–100 percent"));
    }
    if s.frequency == 0 || (s.gate == 0 && s.output == 0) {
        return Err(Error::Invalid("a pulse needs a frequency and a laser or shutter output"));
    }
    let mut on = Vec::new();
    let mut off = Vec::new();
    if let Some((channel, value)) = s.analog {
        if !(1..=2).contains(&channel) {
            return Err(Error::Invalid("invalid pulse analog channel"));
        }
        on.push(Record::Analog { channel, value });
        off.push(Record::Analog { channel, value: 0 });
    }
    if s.gate != 0 {
        on.push(digital(s.gate, true)?);
    }
    on.push(Record::Pwm { channel: s.channel, frequency: s.frequency, power: s.power });
    if s.output != 0 {
        on.push(digital(s.output, true)?);
        off.insert(0, digital(s.output, false)?);
    }
    off.push(Record::Pwm { channel: LaserChannel::Primary, frequency: s.frequency, power: 0 });
    off.push(Record::Pwm { channel: LaserChannel::Secondary, frequency: s.frequency, power: 0 });
    if s.gate != 0 {
        off.push(digital(s.gate, false)?);
    }
    let mut records = vec![Record::Item(u32::MAX)];
    records.extend(off.clone());
    records.extend(on);
    records.push(Record::Delay { millis: duration_ms });
    records.extend(off);
    records.push(Record::Item(u32::MAX - 1));
    records.push(Record::Barrier);
    let seconds = f64::from(duration_ms) / 1000.;
    Ok(Program {
        sections: vec![Section {
            kind: Kind::CutStart,
            pass: None,
            records: 0..records.len(),
            seconds,
            points: std::sync::Arc::default(),
            waits: 0,
        }],
        records,
        seconds,
        samples: 0,
    })
}
