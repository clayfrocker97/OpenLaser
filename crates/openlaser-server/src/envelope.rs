// SPDX-License-Identifier: GPL-3.0-or-later

//! Admission of planned and quantized XY motion in machine coordinates.

use crate::{Error, Result};
use openlaser_compiler::program::{Binding, Job, Kind, Program};
use openlaser_core::geometry::{Bounds, Point};
use openlaser_protocol::records::Record;

/// Planned geometry must fit to floating-point precision. Quantized motion
/// may differ by less than one encoder count because the writer truncates
/// and carries the residual between movements.
pub(crate) fn validate(
    program: &Program,
    position: [f64; 2],
    zero: [f64; 2],
    extent: [[f64; 2]; 2],
    counts: [f64; 2],
) -> Result<()> {
    if counts.iter().any(|c| !c.is_finite() || *c <= 0.) {
        return Err(Error::Refused("invalid encoder scale".into()));
    }
    let check = |point: [f64; 2], tolerance: [f64; 2]| -> Result<()> {
        for axis in 0..2 {
            if !point[axis].is_finite()
                || !extent[axis].iter().all(|v| v.is_finite())
                || extent[axis][0] > extent[axis][1]
                || point[axis] < extent[axis][0] - tolerance[axis]
                || point[axis] > extent[axis][1] + tolerance[axis]
            {
                return Err(Error::Refused(format!(
                    "the executed path leaves {} travel at {:.4} mm",
                    ["X", "Y"][axis],
                    point[axis]
                )));
            }
        }
        Ok(())
    };
    check(position, [1e-9; 2])?;
    for point in program.sections.iter().flat_map(|s| s.points.iter()) {
        check([point[0] + zero[0], point[1] + zero[1]], [1e-9; 2])?;
    }
    let mut pulses = [0i64; 2];
    for record in &program.records {
        if let Record::Move { dx, dy, .. } = record {
            pulses[0] += i64::from(*dx);
            pulses[1] += i64::from(*dy);
            // MAX_RECORDS bounds the accumulator well inside exact f64 integers.
            let displacement = pulses.map(|v| i32::try_from(v).map(f64::from));
            let [Ok(x), Ok(y)] = displacement else {
                return Err(Error::Refused("motion exceeds the pulse domain".into()));
            };
            check(
                [position[0] + x / counts[0], position[1] + y / counts[1]],
                counts.map(|c| 1. / c),
            )?;
        }
    }
    Ok(())
}

/// The process rectangle: cut, film, leads, joints, piercing and cleaning.
/// Travel from the current head position is deliberately not part of the
/// rectangle, but every frame leg is admitted through `validate`.
pub(crate) fn process_bounds(job: &Job, z_units_per_mm: u32) -> Result<Bounds> {
    let first = job.passes.first().ok_or_else(|| Error::Refused("no passes to frame".into()))?;
    let program =
        job.program(Binding { current: first.start, z_units_per_mm: Some(z_units_per_mm) })?;
    let mut bounds = Bounds::of(Point::from(first.start));
    for pass in &job.passes {
        bounds.include(pass.start.into());
    }
    for section in &program.sections {
        if section.kind != Kind::Travel {
            for point in section.points.iter() {
                bounds.include((*point).into());
            }
        }
    }
    Ok(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_compiler::program::Section;

    #[test]
    fn process_and_quantized_motion_are_both_checked() {
        let mut program = Program {
            records: vec![Record::Move { dx: 100, dy: 0, laser: [0; 5] }],
            sections: vec![Section {
                kind: Kind::ResidueSpiral,
                pass: Some(0),
                records: 0..1,
                seconds: 0.,
                points: [[0., 0.], [1., 0.]].into(),
                waits: 0,
            }],
            seconds: 0.,
            samples: 1,
        };
        let limits = [[0., 10.]; 2];
        assert!(validate(&program, [9., 5.], [9., 5.], limits, [100.; 2]).is_ok());
        assert!(validate(&program, [9., 5.], [9.001, 5.], limits, [100.; 2]).is_err());
        assert!(validate(&program, [11., 5.], [0.; 2], limits, [100.; 2]).is_err());
        program.sections[0].points = [[0.; 2]].into();
        assert!(validate(&program, [9.1, 5.], [0.; 2], limits, [100.; 2]).is_err());
    }
}
