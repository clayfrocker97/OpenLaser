// SPDX-License-Identifier: GPL-3.0-or-later

//! The passes a job makes over its contours, MainApp `0x0045_FC70`:
//! preliminary piercing and film batches are collected independently and
//! inserted at their source-position boundaries, then every contour gets
//! its ordinary cut. A batch counts source positions, flagged or not. At a
//! coincident boundary the order is piercing, film, then the cut.
//!
//! Every pass has its own identity, so a finished film pass or pre-pierce
//! point never counts as a cut contour, and a resume names a pass, never a
//! source contour alone.

use crate::{Error, Result};

/// What a pass does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassKind {
    /// The piercing sequence at a contour's start, without its cut.
    PrePierce,
    /// The film process traced over a contour before its cut.
    Film,
    /// The contour cut.
    Cut,
}

/// One pass over one source position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    /// Its place in the program, unique.
    pub ordinal: usize,
    /// The prepared position it runs over.
    pub source: usize,
    /// What it does.
    pub kind: PassKind,
    /// The batch it belongs to, counted from zero per kind.
    pub batch: usize,
}

/// The most source positions a schedule takes.
const MAX_POSITIONS: usize = 100_000;

/// The schedule over `count` prepared positions in cutting order: `film`
/// and `pre` flag each position, `film_batch` of zero means the whole job,
/// `pre_batch` must be positive.
pub fn schedule(
    count: usize,
    film: &[bool],
    film_batch: usize,
    pre: &[bool],
    pre_batch: usize,
) -> Result<Vec<Pass>> {
    if film.len() != count || pre.len() != count {
        return Err(Error::Invalid("pass flags must match the ordered contours"));
    }
    if count > MAX_POSITIONS {
        return Err(Error::Budget("the schedule exceeds 100 000 contours"));
    }
    if pre_batch == 0 {
        return Err(Error::Invalid("PreDrillMaxNum must be positive"));
    }
    let film_size = if film_batch == 0 { count.max(1) } else { film_batch };
    let mut passes: Vec<Pass> = Vec::with_capacity(count);
    let push = |source: usize, kind: PassKind, batch: usize, passes: &mut Vec<Pass>| {
        passes.push(Pass { ordinal: passes.len(), source, kind, batch });
    };
    for at in 0..count {
        for (kind, size, flags) in
            [(PassKind::PrePierce, pre_batch, pre), (PassKind::Film, film_size, film)]
        {
            if at.is_multiple_of(size) {
                let end = at.saturating_add(size).min(count);
                for (source, _) in flags.iter().enumerate().take(end).skip(at).filter(|f| *f.1) {
                    push(source, kind, at / size, &mut passes);
                }
            }
        }
        push(at, PassKind::Cut, at / film_size, &mut passes);
    }
    Ok(passes)
}

/// The passes left after a checkpoint at `fraction` of the pass at `at`,
/// the first with that fraction: a program resumes by pass, never by an
/// ambiguous source contour. A point pass resumes only before or after
/// itself.
pub fn remaining(plan: &[Pass], at: usize, fraction: f64) -> Result<Vec<(Pass, f64)>> {
    if !fraction.is_finite() || !(0. ..=1.).contains(&fraction) {
        return Err(Error::Invalid("a checkpoint fraction runs from 0 to 1"));
    }
    let pass = plan.get(at).ok_or(Error::Invalid("the checkpoint is outside the program"))?;
    if pass.kind == PassKind::PrePierce && fraction != 0. && fraction != 1. {
        return Err(Error::Invalid(
            "a pre-pierce checkpoint must be before or after the point operation",
        ));
    }
    let start = at + usize::from(fraction == 1.);
    Ok(plan[start..]
        .iter()
        .enumerate()
        .map(|(i, p)| (*p, if i == 0 && start == at { fraction } else { 0. }))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(plan: &[Pass]) -> Vec<(usize, PassKind)> {
        plan.iter().map(|p| (p.source, p.kind)).collect()
    }

    /// Film batches count every position, flagged or not, and each batch's
    /// film passes precede its cuts; a batch of zero is the whole job.
    #[test]
    fn film_batches_count_positions_and_precede_their_cuts() {
        use PassKind::{Cut, Film};
        let flags = [true, false, true, true, false];
        let plan = schedule(5, &flags, 2, &[false; 5], 1).unwrap();
        assert_eq!(
            kinds(&plan),
            vec![(0, Film), (0, Cut), (1, Cut), (2, Film), (3, Film), (2, Cut), (3, Cut), (4, Cut)]
        );
        assert_eq!(plan.iter().map(|p| p.batch).collect::<Vec<_>>(), [0, 0, 0, 1, 1, 1, 1, 2]);
        let whole = schedule(3, &[true, false, true], 0, &[false; 3], 1).unwrap();
        assert_eq!(kinds(&whole), vec![(0, Film), (2, Film), (0, Cut), (1, Cut), (2, Cut)]);
        assert_eq!(schedule(3, &[false; 3], 0, &[false; 3], 1).unwrap().len(), 3);
    }

    /// Pre-pierce and film batches keep their own boundaries; where they
    /// coincide, piercing comes first, then film, then the cuts.
    #[test]
    fn independent_boundaries_keep_the_vendor_insertion_order() {
        use PassKind::{Cut, Film, PrePierce};
        let plan = schedule(4, &[true; 4], 3, &[true, false, true, true], 2).unwrap();
        assert_eq!(
            kinds(&plan),
            vec![
                (0, PrePierce),
                (0, Film),
                (1, Film),
                (2, Film),
                (0, Cut),
                (1, Cut),
                (2, PrePierce),
                (3, PrePierce),
                (2, Cut),
                (3, Film),
                (3, Cut)
            ]
        );
        assert!(plan.iter().enumerate().all(|(i, p)| p.ordinal == i));
    }

    /// One point is a valid batch of ten, a batch of zero is refused, and
    /// the flag lists must match the contours.
    #[test]
    fn a_single_point_batch_is_valid_and_zero_is_not() {
        let plan = schedule(1, &[false], 0, &[true], 10).unwrap();
        assert_eq!(kinds(&plan), vec![(0, PassKind::PrePierce), (0, PassKind::Cut)]);
        assert!(matches!(
            schedule(1, &[false], 0, &[true], 0),
            Err(Error::Invalid("PreDrillMaxNum must be positive"))
        ));
        assert!(schedule(2, &[false], 0, &[true; 2], 1).is_err());
    }

    /// A resume names a pass: from a cut at half way the remaining passes
    /// are that cut and the rest, never the completed preliminary passes;
    /// a point resumes only before or after itself, and the passes keep
    /// their original ordinals.
    #[test]
    fn resume_keeps_pass_identity() {
        let plan = schedule(2, &[false; 2], 0, &[true; 2], 2).unwrap();
        let rest = remaining(&plan, 2, 0.5).unwrap();
        assert_eq!(
            rest.iter().map(|(p, f)| (p.source, p.kind, *f)).collect::<Vec<_>>(),
            vec![(0, PassKind::Cut, 0.5), (1, PassKind::Cut, 0.)]
        );
        assert_eq!(rest[1].0.ordinal, 3);
        assert!(remaining(&plan, 0, 0.5).is_err(), "half a point");
        assert_eq!(remaining(&plan, 0, 1.).unwrap().len(), 3);
        assert_eq!(remaining(&plan, 0, 1.).unwrap()[0].1, 0.);
        assert_eq!(remaining(&plan, 0, 0.).unwrap().len(), 4);
        assert!(remaining(&plan, 3, 1.).unwrap().is_empty());
        assert!(remaining(&plan, 4, 0.).is_err());
        assert!(remaining(&plan, 0, f64::NAN).is_err());
    }
}
