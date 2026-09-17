// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded bisection shared by profile construction and endpoint refinement.

use crate::{Error, Result, finite};

pub(super) fn solve(
    evaluate: impl Fn(f64) -> Result<f64>,
    left: f64,
    right: f64,
    tolerance: f64,
    exhausted: &'static str,
) -> Result<f64> {
    let (at_left, at_right) = (evaluate(left)?, evaluate(right)?);
    // Endpoints use strict tolerance; interior candidates include it.
    if at_left.abs() < tolerance {
        return Ok(left);
    }
    if at_right.abs() < tolerance {
        return Ok(right);
    }
    if finite(at_left * at_right)? > 0. {
        // Both native searches choose the right endpoint on a tie.
        return Ok(if at_right.abs() <= at_left.abs() { right } else { left });
    }
    let (mut below, mut above) = if at_left <= at_right { (left, right) } else { (right, left) };
    for _ in 0..super::ROOT_ITERATIONS {
        let midpoint = finite(finite(below + above)? * 0.5)?;
        let value = evaluate(midpoint)?;
        if -tolerance <= value && value <= tolerance {
            return Ok(midpoint);
        }
        if value > 0. {
            above = midpoint;
        } else {
            below = midpoint;
        }
    }
    Err(Error::Budget(exhausted))
}
