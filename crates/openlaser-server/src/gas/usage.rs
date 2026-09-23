// SPDX-License-Identifier: GPL-3.0-or-later

//! Laser and gas time read back from a compiled program. The compiler's
//! records are only read: valve and laser outputs are followed through the
//! program's own output writes, so the times match what is uploaded.

use super::GasKind;
use openlaser_compiler::pass::PassKind;
use openlaser_compiler::program::{Job, Kind, Program, Section};
use openlaser_compiler::settings::{Hardware, Settings};
use openlaser_core::units::{Bar, Millimeters, Seconds};
use openlaser_protocol::records::Record;
use openlaser_protocol::requests::{OutputBank, OutputPort};
use serde::{Deserialize, Serialize};

/// Time one gas flowed at one pressure.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GasTime {
    /// Which gas.
    pub gas: GasKind,
    /// The regulated gauge pressure it flowed at.
    pub pressure: Bar,
    /// How long its valve was open.
    pub seconds: Seconds,
}

/// What a program, or a part of it, spends: laser time, gas time per gas
/// and pressure, pierces and cut length.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Time the laser output is on.
    pub laser: Seconds,
    /// Gas valve time, one entry per gas and pressure.
    pub gases: Vec<GasTime>,
    /// How many times the program pierces.
    pub pierces: u32,
    /// Length cut with the beam on, leads included.
    pub cut: Millimeters,
}

impl Usage {
    fn add_gas(&mut self, gas: GasKind, pressure: f64, seconds: f64) {
        if seconds <= 0. {
            return;
        }
        if let Some(line) =
            self.gases.iter_mut().find(|g| g.gas == gas && (g.pressure.0 - pressure).abs() < 1e-6)
        {
            line.seconds.0 += seconds;
        } else {
            self.gases.push(GasTime { gas, pressure: Bar(pressure), seconds: Seconds(seconds) });
        }
    }

    /// Adds `other` scaled by `share`; pierces count whole once any share ran.
    pub fn accumulate(&mut self, other: &Self, share: f64) {
        let share = if share.is_finite() { share.clamp(0., 1.) } else { 0. };
        if share <= 0. {
            return;
        }
        self.laser.0 += other.laser.0 * share;
        self.cut.0 += other.cut.0 * share;
        self.pierces += other.pierces;
        for line in &other.gases {
            self.add_gas(line.gas, line.pressure.0, line.seconds.0 * share);
        }
    }

    /// Total gas valve time, every gas together.
    #[must_use]
    pub fn gas_seconds(&self) -> f64 {
        self.gases.iter().map(|g| g.seconds.0).sum()
    }
}

/// A program's usage split by pass, so a stopped run can be counted by
/// the share of each pass it executed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobUsage {
    /// The whole program.
    pub total: Usage,
    /// Each pass, in the job's pass order.
    pub passes: Vec<Usage>,
    /// Records outside every pass, such as the opening and closing.
    pub common: Usage,
}

impl JobUsage {
    /// The usage of a run that executed `shares` of each pass (0 to 1).
    #[must_use]
    pub fn executed(&self, shares: &[f64]) -> Usage {
        let mut usage = Usage::default();
        if shares.iter().any(|s| *s > 0.) {
            usage.accumulate(&self.common, 1.);
        }
        for (pass, share) in self.passes.iter().zip(shares) {
            usage.accumulate(pass, *share);
        }
        usage
    }
}

/// One output port's bank and bit.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Port(OutputBank, u16);

impl Port {
    fn of(port: Option<u8>) -> Option<Self> {
        port.and_then(|p| OutputPort::new(p).ok()).map(|p| Self(p.bank, p.mask))
    }

    /// Its new state if `record` writes it.
    fn written(self, record: &Record) -> Option<bool> {
        match record {
            Record::Outputs { bank, mask, values } if *bank == self.0 && mask & self.1 != 0 => {
                Some(values & self.1 != 0)
            }
            _ => None,
        }
    }
}

/// Laser-on section kinds, used only when a program writes no PWM power.
const fn fires(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Pierce(_)
            | Kind::PierceSmooth
            | Kind::PierceGradual
            | Kind::BoltRamp
            | Kind::ResidueSpiral
            | Kind::Cut
            | Kind::LeadIn
            | Kind::LeadOut
            | Kind::Film
    )
}

/// The gas selector and pressure a section's process writes, from the
/// recipe values the compiler selected them from.
fn context(kind: Kind, settings: &Settings, stage: Option<u8>) -> (u8, f64) {
    let stage_of = |index: Option<u8>| {
        index
            .and_then(|i| settings.pierce.iter().find(|s| s.index == i))
            .map(|s| (s.gas, s.pressure))
    };
    let cutting = (settings.gas, settings.pressure);
    match kind {
        Kind::PierceStart(i) | Kind::Pierce(i) => stage_of(Some(i)).unwrap_or(cutting),
        Kind::PierceSmooth | Kind::PierceGradual | Kind::BoltPadding | Kind::BoltRamp => {
            stage_of(stage).unwrap_or(cutting)
        }
        Kind::ResidueStart | Kind::ResidueSpiral | Kind::ResidueReturn | Kind::ResidueEnd => {
            settings.residue.as_ref().map_or(cutting, |r| (r.gas, r.pressure))
        }
        _ => cutting,
    }
}

/// A count as a float; counts here are far below 2^52.
pub(super) fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

fn length(section: &Section) -> f64 {
    section.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

struct Valves {
    gas: [Option<Port>; 6],
    ratio: [Option<Port>; 3],
    laser: Option<Port>,
    gas_on: [bool; 6],
    ratio_on: [bool; 3],
    laser_on: bool,
    pwm_on: bool,
    pwm_seen: bool,
    pressure: [f64; 3],
}

impl Valves {
    fn new(h: &Hardware) -> Self {
        Self {
            gas: h.gas_ports.map(Port::of),
            ratio: h.ratio_ports.map(Port::of),
            laser: Port::of(h.laser),
            gas_on: [false; 6],
            ratio_on: [false; 3],
            laser_on: false,
            pwm_on: false,
            pwm_seen: false,
            pressure: [0.; 3],
        }
    }

    /// The beam is on while a nonzero PWM power is written and, when the
    /// machine has a laser output, that output is on. The gate alone only
    /// enables the source, so it is not counted.
    const fn firing(&self) -> bool {
        self.pwm_on && (self.laser.is_none() || self.laser_on)
    }

    fn open(&self, kind: usize) -> bool {
        (0..6).any(|i| i % 3 == kind && self.gas_on[i]) || self.ratio_on[kind]
    }

    /// Follows one record; `selected` is the section's gas and pressure.
    fn apply(&mut self, record: &Record, h: &Hardware, selected: (u8, f64)) {
        let selected_kind = usize::from(selected.0 % 3);
        let before = [self.open(0), self.open(1), self.open(2)];
        for (port, on) in self.gas.iter().zip(self.gas_on.iter_mut()) {
            if let Some(state) = port.and_then(|p| p.written(record)) {
                *on = state;
            }
        }
        for (port, on) in self.ratio.iter().zip(self.ratio_on.iter_mut()) {
            if let Some(state) = port.and_then(|p| p.written(record)) {
                *on = state;
            }
        }
        if let Some(state) = self.laser.and_then(|p| p.written(record)) {
            self.laser_on = state;
        }
        if let Record::Pwm { power, .. } = record {
            self.pwm_on = *power > 0;
            self.pwm_seen |= *power > 0;
        }
        let pressure_written = matches!(record, Record::Analog { channel, value } if *value > 0
            && h.ratio_channels[selected_kind] == Some(*channel));
        if pressure_written || (!before[selected_kind] && self.open(selected_kind)) {
            self.pressure[selected_kind] = selected.1;
        }
    }
}

/// The laser and gas time of `program`, compiled from `job`.
///
/// Gas time runs from the record that opens a valve to the record that
/// closes it, over the program's delays and motion; a valve kept open over
/// a short travel is counted over that travel. Waits for controller
/// feedback (head height, a jump's descent) have no fixed length and add
/// nothing, so real gas time is somewhat longer. A dry run spends nothing.
#[must_use]
pub fn of(job: &Job, program: &Program) -> JobUsage {
    let mut usage =
        JobUsage { passes: vec![Usage::default(); job.passes.len()], ..JobUsage::default() };
    let dry = job.settings.dry_run;
    let hardware = job.settings.hardware.as_ref().filter(|_| !dry);
    let mut valves = hardware.map(Valves::new);
    let mut stage = None;
    // Laser time by section kind, for programs that never write a PWM power.
    let mut fallback = vec![0.; job.passes.len() + 1];
    for section in &program.sections {
        let pass = section.pass.and_then(|p| job.passes.get(p));
        let settings = pass.map_or(&job.settings, |p| &p.settings);
        if let Kind::PierceStart(i) | Kind::Pierce(i) = section.kind {
            stage = Some(i);
        }
        let selected = context(section.kind, settings, stage);
        let records = program.records.get(section.records.clone()).unwrap_or_default();
        let moves = records.iter().filter(|r| matches!(r, Record::Move { .. })).count();
        let step = if moves > 0 { section.seconds / count(moves) } else { 0. };
        let bucket = match section.pass {
            Some(p) if p < usage.passes.len() => &mut usage.passes[p],
            _ => &mut usage.common,
        };
        if matches!(section.kind, Kind::Cut | Kind::LeadIn | Kind::LeadOut) {
            bucket.cut.0 += length(section);
        }
        let (Some(h), Some(valves)) = (hardware, valves.as_mut()) else { continue };
        if fires(section.kind) {
            fallback[section.pass.filter(|p| *p < job.passes.len()).unwrap_or(job.passes.len())] +=
                section.seconds;
        }
        for record in records {
            let dt = match record {
                Record::Move { .. } => step,
                Record::Delay { millis } => f64::from(*millis) / 1000.,
                _ => {
                    valves.apply(record, h, selected);
                    continue;
                }
            };
            if valves.firing() {
                bucket.laser.0 += dt;
            }
            for (kind, gas) in
                [GasKind::Air, GasKind::Oxygen, GasKind::Nitrogen].into_iter().enumerate()
            {
                if valves.open(kind) {
                    bucket.add_gas(gas, valves.pressure[kind], dt);
                }
            }
        }
    }
    if valves.as_ref().is_some_and(|v| !v.pwm_seen) {
        for (bucket, seconds) in usage.passes.iter_mut().chain([&mut usage.common]).zip(fallback) {
            bucket.laser = Seconds(seconds);
        }
    }
    if !dry {
        for (compiled, bucket) in job.passes.iter().zip(&mut usage.passes) {
            let pierces = match compiled.pass.kind {
                PassKind::PrePierce => true,
                PassKind::Cut => compiled.initial_pierce,
                PassKind::Film => false,
            };
            bucket.pierces = u32::from(pierces);
        }
    }
    let shares = vec![1.; usage.passes.len()];
    usage.total = usage.executed(&shares);
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executed_shares_scale_time_and_count_started_pierces() {
        let pass = Usage {
            laser: Seconds(10.),
            gases: vec![GasTime {
                gas: GasKind::Oxygen,
                pressure: Bar(0.8),
                seconds: Seconds(12.),
            }],
            pierces: 1,
            cut: Millimeters(100.),
        };
        let common = Usage {
            gases: vec![GasTime { gas: GasKind::Oxygen, pressure: Bar(0.8), seconds: Seconds(1.) }],
            ..Usage::default()
        };
        let job = JobUsage { total: Usage::default(), passes: vec![pass.clone(), pass], common };
        let half = job.executed(&[1., 0.5]);
        assert!((half.laser.0 - 15.).abs() < 1e-9);
        assert!((half.cut.0 - 150.).abs() < 1e-9);
        assert_eq!(half.pierces, 2);
        assert_eq!(half.gases.len(), 1);
        assert!((half.gas_seconds() - 19.).abs() < 1e-9);
        assert_eq!(job.executed(&[0., 0.]), Usage::default());
    }
}
