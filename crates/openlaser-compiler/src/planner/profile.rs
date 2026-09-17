// SPDX-License-Identifier: GPL-3.0-or-later

//! One seven-phase velocity profile over a span, CADModule `0x1010_64B0`, and
//! its compact one-transition variant, `0x1010_7D20`.
//!
//! A profile accelerates from its entry speed to a peak and decelerates to
//! its exit speed. Each ramp is jerk-limited: a jerk phase, a constant
//! acceleration phase, a jerk phase. The cruise between the ramps takes
//! whatever length they leave over, so its duration can come out slightly
//! negative when a root search stopped on distance error. The sampler admits
//! exactly that residual and nothing more.
//!
//! The arithmetic is the vendor's, operand for operand. The iteration
//! ceiling on the root search, the finite-value checks and the cruise
//! tolerance are ours: the vendor has none of them and carries on with
//! whatever its arithmetic produced. Names, helpers and layout are ours too.

use crate::{Error, Result, finite};

/// The six words a profile is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Span length.
    pub length: f64,
    /// Speed at the span's start.
    pub entry: f64,
    /// The highest speed allowed inside the span.
    pub ceiling: f64,
    /// Speed at the span's end.
    pub exit: f64,
    /// Acceleration.
    pub acceleration: f64,
    /// Jerk.
    pub jerk: f64,
    /// First and last node index the span covers.
    pub span: [u32; 2],
}

impl Input {
    fn check(&self) -> Result<()> {
        for v in [self.length, self.entry, self.ceiling, self.exit, self.acceleration, self.jerk] {
            finite(v)?;
        }
        Ok(())
    }
}

/// One phase: how long it lasts and how far it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phase {
    /// Duration in seconds.
    pub duration: f64,
    /// Distance in millimetres.
    pub distance: f64,
}

/// Index of the entry ramp's first jerk phase.
pub const ENTRY_JERK_IN: usize = 0;
/// Index of the constant acceleration phase.
pub const ACCELERATE: usize = 1;
/// Index of the entry ramp's second jerk phase.
pub const ENTRY_JERK_OUT: usize = 2;
/// Index of the cruise.
pub const CRUISE: usize = 3;
/// Index of the exit ramp's first jerk phase.
pub const EXIT_JERK_IN: usize = 4;
/// Index of the constant deceleration phase.
pub const DECELERATE: usize = 5;
/// Index of the exit ramp's second jerk phase.
pub const EXIT_JERK_OUT: usize = 6;

/// The timing of one ramp: its two equal jerk phases and the constant
/// acceleration phase between them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Ramp {
    jerk_time: f64,
    constant_time: f64,
}

/// A ramp that reaches `delta` more speed: a full ramp when the acceleration
/// limit is reached, a triangular one otherwise.
fn ramp(delta: f64, acceleration: f64, jerk: f64) -> Ramp {
    if acceleration * acceleration / jerk <= delta {
        full_ramp(delta, acceleration, jerk)
    } else {
        triangular_ramp(delta, jerk)
    }
}

/// A ramp whose middle runs at the acceleration limit.
fn full_ramp(delta: f64, acceleration: f64, jerk: f64) -> Ramp {
    let t = acceleration / jerk;
    Ramp { jerk_time: t, constant_time: (delta - jerk * t * t) / acceleration }
}

/// A ramp that never reaches the acceleration limit.
fn triangular_ramp(delta: f64, jerk: f64) -> Ramp {
    Ramp { jerk_time: (delta / jerk).sqrt(), constant_time: 0. }
}

/// Which construction path the vendor took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// The ceiling is reached with a cruise in between.
    CeilingReached,
    /// Both ramps reach constant acceleration but no cruise.
    BothFullRamps,
    /// One full ramp, one triangular, peak found by root search.
    MixedRoot,
    /// Both ramps triangular, peak found by root search.
    BothTriangularRoot,
    /// A span under 0.01 mm: the vendor's literal two-ramp formula.
    TinySpan,
    /// The endpoints cannot be joined within the span; the vendor reports
    /// failure and leaves the span uncovered.
    EndpointAdjustment,
    /// Compact: constant speed.
    CompactConstant,
    /// Compact: one transition between the endpoints.
    CompactTransition,
}

/// A constructed profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub(super) length: f64,
    pub(super) entry: f64,
    pub(super) peak: f64,
    pub(super) exit: f64,
    pub(super) acceleration: f64,
    pub(super) jerk: f64,
    pub(super) phases: [Phase; 7],
    span: [u32; 2],
    /// The construction path taken.
    pub branch: Branch,
    /// Whether the vendor's builder reported success.
    pub native_return: bool,
    cruise_tolerance: f64,
}

impl Profile {
    /// Span length.
    #[must_use]
    pub const fn length(&self) -> f64 {
        self.length
    }

    /// Speed at the span's start.
    #[must_use]
    pub const fn entry(&self) -> f64 {
        self.entry
    }

    /// The peak speed.
    #[must_use]
    pub const fn peak(&self) -> f64 {
        self.peak
    }

    /// Speed at the span's end.
    #[must_use]
    pub const fn exit(&self) -> f64 {
        self.exit
    }

    /// Acceleration.
    #[must_use]
    pub const fn acceleration(&self) -> f64 {
        self.acceleration
    }

    /// Jerk.
    #[must_use]
    pub const fn jerk(&self) -> f64 {
        self.jerk
    }

    /// The seven phases, entry ramp first.
    #[must_use]
    pub const fn phases(&self) -> &[Phase; 7] {
        &self.phases
    }

    /// First and last node index the span covers.
    #[must_use]
    pub const fn span(&self) -> [u32; 2] {
        self.span
    }

    /// The seven phase durations.
    #[must_use]
    pub fn durations(&self) -> [f64; 7] {
        self.phases.map(|p| p.duration)
    }

    /// The seven phase distances.
    #[must_use]
    pub fn distances(&self) -> [f64; 7] {
        self.phases.map(|p| p.distance)
    }

    /// The twenty words in the vendor's record layout: length, entry, peak,
    /// exit, acceleration, jerk, the seven durations, the seven distances.
    #[must_use]
    pub fn words(&self) -> [f64; 20] {
        let mut words = [0.; 20];
        words[..6].copy_from_slice(&[
            self.length,
            self.entry,
            self.peak,
            self.exit,
            self.acceleration,
            self.jerk,
        ]);
        words[6..13].copy_from_slice(&self.durations());
        words[13..20].copy_from_slice(&self.distances());
        words
    }

    /// What a merge rebuilds the profile from: the constructed peak stands
    /// in for the ceiling, and the acceleration and jerk are the profile's,
    /// which an endpoint adjustment may have rewritten.
    #[must_use]
    pub const fn rebuild_input(&self) -> Input {
        Input {
            length: self.length,
            entry: self.entry,
            ceiling: self.peak,
            exit: self.exit,
            acceleration: self.acceleration,
            jerk: self.jerk,
            span: self.span,
        }
    }

    /// The entry ramp's duration, summed in the vendor's order.
    #[must_use]
    pub fn entry_ramp_time(&self) -> f64 {
        let d = self.durations();
        d[ACCELERATE] + d[ENTRY_JERK_IN] + d[ENTRY_JERK_OUT]
    }

    /// The exit ramp's duration, summed in the vendor's order.
    #[must_use]
    pub fn exit_ramp_time(&self) -> f64 {
        let d = self.durations();
        d[DECELERATE] + d[EXIT_JERK_IN] + d[EXIT_JERK_OUT]
    }

    /// The entry ramp's distance, summed in the vendor's order.
    #[must_use]
    pub fn entry_ramp_distance(&self) -> f64 {
        let x = self.distances();
        x[ACCELERATE] + x[ENTRY_JERK_IN] + x[ENTRY_JERK_OUT]
    }

    /// How far below zero the cruise duration may sit and still be sampled.
    pub(crate) const fn cruise_tolerance(&self) -> f64 {
        self.cruise_tolerance
    }

    /// A profile is built as if it accelerates; one that runs from a faster
    /// to a slower speed is the mirror image.
    fn mirrored(mut self) -> Self {
        for (a, b) in [
            (ENTRY_JERK_IN, EXIT_JERK_OUT),
            (ACCELERATE, DECELERATE),
            (ENTRY_JERK_OUT, EXIT_JERK_IN),
        ] {
            self.phases.swap(a, b);
        }
        self
    }

    fn finish(self, residual: f64) -> Result<Self> {
        let profile = if self.entry > self.exit { self.mirrored() } else { self };
        if profile.words().iter().any(|v| !v.is_finite()) {
            return Err(Error::NonFinite);
        }
        // The admitted residual plus round-off, as a duration at peak speed.
        let magnitude =
            profile.length.abs().max(profile.distances().iter().map(|v| v.abs()).sum::<f64>());
        let cruise_tolerance = (residual + 64. * f64::EPSILON * magnitude) / profile.peak;
        Ok(Self { cruise_tolerance, ..profile })
    }
}

/// The distance a change of speed takes, CADModule `0x1010_62C0`: a
/// triangular ramp when the acceleration limit is not reached, a full ramp
/// otherwise, and nothing for a decrease.
pub(crate) fn transition_distance(from: f64, to: f64, acceleration: f64, jerk: f64) -> f64 {
    let sum = from + to;
    let delta = to - from;
    if delta < 0. {
        0.
    } else if delta < acceleration * acceleration / jerk {
        (delta / jerk).sqrt() * sum
    } else {
        sum * (delta / acceleration + acceleration / jerk) * 0.5
    }
}

/// The distance-balance a peak speed must satisfy, as the vendor's two
/// root callbacks express it.
#[derive(Clone, Copy)]
enum Callback {
    /// Both ramps triangular.
    BothTriangular,
    /// A full ramp up from the low speed, a triangular one down to the high.
    FullThenTriangular,
}

/// The midpoint root search of CADModule `0x1010_6330`. The vendor's loop
/// has no iteration ceiling.
struct Root {
    callback: Callback,
    low: f64,
    high: f64,
    acceleration: f64,
    jerk: f64,
    length: f64,
    tolerance: f64,
}

impl Root {
    /// The distance the two ramps to and from peak speed `x` cover, less
    /// the span length.
    fn value(&self, x: f64) -> Result<f64> {
        let (low, high, acceleration, jerk) = (self.low, self.high, self.acceleration, self.jerk);
        let sum = match self.callback {
            Callback::BothTriangular => {
                (low + x) * ((x - low) / jerk).sqrt() + (high + x) * ((x - high) / jerk).sqrt()
            }
            Callback::FullThenTriangular => {
                (high + x) * ((x - high) / jerk).sqrt()
                    + (0.5 * (low + x)) * ((x - low) / acceleration + acceleration / jerk)
            }
        };
        finite(sum - self.length)
    }

    fn solve(&self, left: f64, right: f64) -> Result<f64> {
        super::root::solve(
            |value| self.value(value),
            left,
            right,
            self.tolerance,
            "profile root search did not converge",
        )
    }
}

/// The peak, the two ramps and how they were found.
struct Shape {
    branch: Branch,
    native_return: bool,
    peak: f64,
    acceleration: f64,
    jerk: f64,
    up: Ramp,
    down: Ramp,
}

/// Selects the construction branch and builds the ramps.
#[allow(clippy::too_many_lines, reason = "the vendor's decision tree, one branch per arm")]
fn shape(input: Input, low: f64, high: f64) -> Result<Shape> {
    let Input { length, acceleration, jerk, .. } = input;
    let ceiling = input.ceiling.max(high);
    let a2 = acceleration * acceleration;
    let root = |callback, left, right| {
        Root {
            callback,
            low,
            high,
            acceleration,
            jerk,
            length,
            tolerance: (ceiling / 1000.).min(0.001),
        }
        .solve(left, right)
    };
    let shape = |branch, peak, up, down| Shape {
        branch,
        native_return: true,
        peak,
        acceleration,
        jerk,
        up,
        down,
    };
    if transition_distance(high, ceiling, acceleration, jerk)
        + transition_distance(low, ceiling, acceleration, jerk)
        < length
    {
        let up = ramp(ceiling - low, acceleration, jerk);
        let down = ramp(ceiling - high, acceleration, jerk);
        return Ok(shape(Branch::CeilingReached, ceiling, up, down));
    }
    let upper = a2 / jerk + high;
    if length
        > ((upper + high) * acceleration) / jerk
            + transition_distance(low, upper, acceleration, jerk)
    {
        // The vendor's exponentiation loop yields the fourth power of the
        // acceleration here, not an ambient operand.
        let radicand = a2 * a2
            + ((high * high + low * low) * jerk - (high + low) * a2
                + 2. * acceleration * jerk * length)
                * (2. * jerk);
        let peak = (radicand.sqrt() - a2) / (2. * jerk);
        let up = full_ramp(peak - low, acceleration, jerk);
        let down = full_ramp(peak - high, acceleration, jerk);
        return Ok(shape(Branch::BothFullRamps, peak, up, down));
    }
    if transition_distance(low, high, acceleration, jerk) <= length {
        let lower = (a2 / jerk + low).max(high);
        if transition_distance(high, lower, acceleration, jerk)
            + transition_distance(low, lower, acceleration, jerk)
            <= length
        {
            let peak = root(Callback::FullThenTriangular, lower, upper)?;
            let t = acceleration / jerk;
            let up = Ramp {
                jerk_time: t,
                constant_time: ((peak - t * jerk * t * 0.5) - (0.5 * jerk * t * t + low))
                    / acceleration,
            };
            let down = triangular_ramp(peak - high, jerk);
            return Ok(shape(Branch::MixedRoot, peak, up, down));
        }
        if length < 0.01 {
            // The vendor's literal instruction sequence: the two constant
            // ramps together overshoot the span and the cruise absorbs it.
            let peak = (acceleration * length + low * low + high * high).sqrt();
            let up = Ramp { jerk_time: 0., constant_time: (peak - low) / acceleration };
            let down = Ramp { jerk_time: 0., constant_time: (peak - high) / acceleration };
            return Ok(shape(Branch::TinySpan, peak, up, down));
        }
        let peak = root(Callback::BothTriangular, high, lower)?;
        let up = triangular_ramp(peak - low, jerk);
        let down = triangular_ramp(peak - high, jerk);
        return Ok(shape(Branch::BothTriangularRoot, peak, up, down));
    }
    // The span is too short to join the endpoints: the vendor rewrites the
    // acceleration and jerk, reports failure and leaves the exit ramp empty.
    let acceleration = ((high - low) * (high + low)) / length;
    let jerk = (acceleration * (high + low)) / length;
    let peak = (acceleration * acceleration) / jerk + low;
    Ok(Shape {
        branch: Branch::EndpointAdjustment,
        native_return: false,
        peak,
        acceleration,
        jerk,
        up: Ramp { jerk_time: acceleration / jerk, constant_time: 0. },
        down: Ramp::default(),
    })
}

/// Builds the full profile for `input`.
pub fn construct(input: Input) -> Result<Profile> {
    input.check()?;
    if input.length <= 0.
        || input.ceiling <= 0.
        || input.acceleration <= 0.
        || input.jerk <= 0.
        || input.entry < 0.
        || input.exit < 0.
    {
        return Err(Error::Invalid("profile input outside the positive domain"));
    }
    let low = input.entry.min(input.exit);
    let high = input.entry.max(input.exit);
    let Shape { branch, native_return, peak, acceleration, jerk, up, down } =
        shape(input, low, high)?;
    let (t1, ta1) = (up.jerk_time, up.constant_time);
    let (t2, ta2) = (down.jerk_time, down.constant_time);
    let x0 = t1 * low + (jerk * t1 * t1 * t1) / 6.;
    let x1 = ta1 * acceleration * ta1 * 0.5 + (jerk * t1 * t1 * 0.5 + low) * ta1;
    let x2 = peak * t1 - (jerk * t1 * t1 * t1) / 6.;
    let x4 = peak * t2 - (jerk * t2 * t2 * t2) / 6.;
    let x5 = (peak - t2 * jerk * t2 * 0.5) * ta2 - acceleration * ta2 * ta2 * 0.5;
    let x6 = t2 * high + (jerk * t2 * t2 * t2) / 6.;
    let x3 = (((((input.length - x0) - x1) - x2) - x4) - x5) - x6;
    let phases = [
        Phase { duration: t1, distance: x0 },
        Phase { duration: ta1, distance: x1 },
        Phase { duration: t1, distance: x2 },
        Phase { duration: x3 / peak, distance: x3 },
        Phase { duration: t2, distance: x4 },
        Phase { duration: ta2, distance: x5 },
        Phase { duration: t2, distance: x6 },
    ];
    // The root search stops on absolute distance error on either side of the
    // root; the tiny-span formula overshoots by an exact amount. Admit that
    // much negative cruise and nothing else.
    let residual = match branch {
        Branch::MixedRoot | Branch::BothTriangularRoot => {
            (input.ceiling.max(high) / 1000.).min(0.001)
        }
        Branch::TinySpan => (low * low + high * high) / (2. * acceleration),
        _ => 0.,
    };
    Profile {
        length: input.length,
        entry: input.entry,
        peak,
        exit: input.exit,
        acceleration,
        jerk,
        phases,
        span: input.span,
        branch,
        native_return,
        cruise_tolerance: 0.,
    }
    .finish(residual)
}

/// Builds the compact one-transition profile for `input`. The result's
/// `native_return` is the vendor's changed-endpoint flag.
pub fn construct_compact(input: Input) -> Result<Profile> {
    input.check()?;
    if input.length <= 0. || input.entry < 0. || input.exit < 0. || input.jerk <= 0. {
        return Err(Error::Invalid("compact profile input outside the positive domain"));
    }
    let Input { length, entry, exit, .. } = input;
    let low = entry.min(exit);
    let high = entry.max(exit);
    let sum = high + low;
    let delta = high - low;
    let changed = delta >= 0.001;
    // A transition keeps the input jerk, halving it when the span allows,
    // and derives its own acceleration; a constant profile has no ramp.
    let (acceleration, jerk, up, cruise) = if !changed {
        (sum, input.jerk, Ramp::default(), Phase { duration: length / low, distance: length })
    } else if (delta / input.jerk).sqrt() * sum < length {
        let half = input.jerk * 0.5;
        let jerk = if (delta / half).sqrt() * sum < length { half } else { input.jerk };
        let acceleration =
            (jerk * length) / sum - ((length / sum) * (length / sum) - delta / jerk).sqrt() * jerk;
        let t = acceleration / jerk;
        let up = Ramp { jerk_time: t, constant_time: (delta - t * t * jerk) / acceleration };
        (acceleration, jerk, up, Phase::default())
    } else {
        let acceleration = (delta * sum) / length;
        let jerk = (acceleration * sum) / length;
        let up = Ramp { jerk_time: acceleration / jerk, constant_time: 0. };
        (acceleration, jerk, up, Phase::default())
    };
    let (t, ta) = (up.jerk_time, up.constant_time);
    let x0 = (jerk * t * t * t) / 6. + t * low;
    let x1 = sum * 0.5 * ta;
    let x2 = t * high - (jerk * t * t * t) / 6.;
    let phases = [
        Phase { duration: t, distance: x0 },
        Phase { duration: ta, distance: x1 },
        Phase { duration: t, distance: x2 },
        cruise,
        Phase::default(),
        Phase::default(),
        Phase::default(),
    ];
    Profile {
        length,
        entry,
        peak: input.ceiling.max(high),
        exit,
        acceleration,
        jerk,
        phases,
        span: input.span,
        branch: if changed { Branch::CompactTransition } else { Branch::CompactConstant },
        native_return: changed,
        cruise_tolerance: 0.,
    }
    .finish(0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(length: f64, entry: f64, exit: f64) -> Input {
        Input { length, entry, ceiling: 100., exit, acceleration: 100., jerk: 1000., span: [2, 7] }
    }

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    /// The exact record of every construction branch, pinned from the
    /// implementation the vendor captures verify. The arithmetic is IEEE
    /// exact, so any change to a ramp or distance formula shows up as the
    /// word that moved.
    #[test]
    fn every_branch_builds_its_pinned_record() {
        let cases = [
            (
                input(200., 0., 0.),
                Branch::CeilingReached,
                true,
                [
                    200.0,
                    0.0,
                    100.0,
                    0.0,
                    100.0,
                    1000.0,
                    0.1,
                    0.9,
                    0.1,
                    0.8999999999999999,
                    0.1,
                    0.9,
                    0.1,
                    0.16666666666666666,
                    45.0,
                    9.833333333333334,
                    89.99999999999999,
                    9.833333333333334,
                    45.0,
                    0.16666666666666666,
                ],
            ),
            (
                input(100., 0., 0.),
                Branch::BothFullRamps,
                true,
                [
                    100.0,
                    0.0,
                    95.12492197250393,
                    0.0,
                    100.0,
                    1000.0,
                    0.1,
                    0.8512492197250393,
                    0.1,
                    -2.480132312760684e-17,
                    0.1,
                    0.8512492197250393,
                    0.1,
                    0.16666666666666666,
                    40.48750780274961,
                    9.345825530583728,
                    -2.3592239273284576e-15,
                    9.345825530583728,
                    40.4875078027496,
                    0.16666666666666666,
                ],
            ),
            (
                input(4., 0., 15.),
                Branch::MixedRoot,
                true,
                [
                    4.0,
                    0.0,
                    17.44873046875,
                    15.0,
                    100.0,
                    1000.0,
                    0.1,
                    0.0744873046875,
                    0.1,
                    -2.530442919904328e-5,
                    0.049484648819103486,
                    0.0,
                    0.049484648819103486,
                    0.16666666666666666,
                    0.649854451417923,
                    1.5782063802083333,
                    -0.00044153016475967366,
                    0.8432485383688293,
                    0.0,
                    0.7624654935030076,
                ],
            ),
            (
                input(1., 0., 0.),
                Branch::BothTriangularRoot,
                true,
                [
                    1.0,
                    0.0,
                    6.298828125,
                    0.0,
                    100.0,
                    1000.0,
                    0.07936515687000184,
                    0.0,
                    0.07936515687000184,
                    2.9376182476954816e-5,
                    0.07936515687000184,
                    0.0,
                    0.07936515687000184,
                    0.08331791370630076,
                    0.0,
                    0.4165895685315038,
                    0.00018503552439097515,
                    0.4165895685315038,
                    0.0,
                    0.08331791370630076,
                ],
            ),
            (
                input(0.001, 0., 0.),
                Branch::TinySpan,
                true,
                [
                    0.001,
                    0.0,
                    0.31622776601683794,
                    0.0,
                    100.0,
                    1000.0,
                    0.0,
                    0.0031622776601683794,
                    0.0,
                    0.0,
                    0.0,
                    0.0031622776601683794,
                    0.0,
                    0.0,
                    0.0005,
                    0.0,
                    0.0,
                    0.0,
                    0.0005,
                    0.0,
                ],
            ),
            (
                input(0.001, 3., 3.),
                Branch::TinySpan,
                true,
                [
                    0.001,
                    3.0,
                    4.254409477236529,
                    3.0,
                    100.0,
                    1000.0,
                    0.0,
                    0.012544094772365294,
                    0.0,
                    -0.02115452226250208,
                    0.0,
                    0.012544094772365294,
                    0.0,
                    0.0,
                    0.04549999999999999,
                    0.0,
                    -0.09,
                    0.0,
                    0.0455,
                    0.0,
                ],
            ),
            (
                input(0.01, 0., 10.),
                Branch::EndpointAdjustment,
                false,
                [
                    0.01,
                    0.0,
                    10.0,
                    10.0,
                    10000.0,
                    10000000.0,
                    0.001,
                    0.0,
                    0.001,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0016666666666666668,
                    0.0,
                    0.008333333333333333,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            ),
        ];
        for (input, branch, native_return, expected) in cases {
            let profile = construct(input).unwrap();
            assert_eq!(profile.branch, branch);
            assert_eq!(profile.native_return, native_return, "{branch:?}");
            assert_eq!(profile.words(), expected, "{branch:?}");
            assert_eq!(profile.span(), [2, 7]);
        }
    }

    /// A long span from rest to rest has the analytic phases: 0.1 s jerk
    /// phases, 0.9 s constant acceleration, and the textbook distances.
    #[test]
    fn full_cruise_profile_has_the_native_phase_values() {
        let profile = construct(input(200., 0., 0.)).unwrap();
        for (actual, expected) in
            profile.durations().iter().zip([0.1, 0.9, 0.1, 0.9, 0.1, 0.9, 0.1])
        {
            near(*actual, expected);
        }
        let x = profile.distances();
        near(x[ENTRY_JERK_IN], 1. / 6.);
        near(x[ACCELERATE], 45.);
        near(x[ENTRY_JERK_OUT], 59. / 6.);
        near(x[CRUISE], 90.);
    }

    /// Reversing the endpoints mirrors the ramps, keeps the cruise, and
    /// gives exactly the pinned records in both orientations.
    #[test]
    fn reversed_endpoints_mirror_the_ramps() {
        let forward = construct(input(200., 2., 15.)).unwrap();
        let reverse = construct(input(200., 15., 2.)).unwrap();
        for (a, b) in [
            (ENTRY_JERK_IN, EXIT_JERK_OUT),
            (ACCELERATE, DECELERATE),
            (ENTRY_JERK_OUT, EXIT_JERK_IN),
        ] {
            assert_eq!(forward.phases()[a], reverse.phases()[b]);
            assert_eq!(forward.phases()[b], reverse.phases()[a]);
        }
        assert_eq!(forward.phases()[CRUISE], reverse.phases()[CRUISE]);
        assert_eq!(
            forward.words(),
            [
                200.0,
                2.0,
                100.0,
                15.0,
                100.0,
                1000.0,
                0.1,
                0.88,
                0.1,
                0.9029499999999997,
                0.1,
                0.75,
                0.1,
                0.3666666666666667,
                44.879999999999995,
                9.833333333333334,
                90.29499999999997,
                9.833333333333334,
                43.125,
                1.6666666666666667,
            ]
        );
        assert_eq!(
            reverse.words(),
            [
                200.0,
                15.0,
                100.0,
                2.0,
                100.0,
                1000.0,
                0.1,
                0.75,
                0.1,
                0.9029499999999997,
                0.1,
                0.88,
                0.1,
                1.6666666666666667,
                43.125,
                9.833333333333334,
                90.29499999999997,
                9.833333333333334,
                44.879999999999995,
                0.3666666666666667,
            ]
        );
    }

    /// The vendor's word layout puts the six inputs first, then durations,
    /// then distances.
    #[test]
    fn words_follow_the_vendor_layout() {
        let profile = construct(input(200., 2., 15.)).unwrap();
        let words = profile.words();
        assert_eq!(words[..4], [200., 2., profile.peak(), 15.]);
        assert_eq!(words[6..13], profile.durations());
        assert_eq!(words[13..20], profile.distances());
    }

    /// Compact profiles pinned exactly: the constant case, a long
    /// transition in both orientations (which halves the jerk), and a short
    /// transition that derives its own acceleration and jerk. A compact
    /// profile from rest to rest is not admitted.
    #[test]
    fn compact_profiles_build_their_pinned_records() {
        let cases = [
            (
                input(20., 10., 10.),
                Branch::CompactConstant,
                false,
                [
                    20.0, 10.0, 100.0, 10.0, 20.0, 1000.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0,
                    0.0, 0.0, 20.0, 0.0, 0.0, 0.0,
                ],
            ),
            (
                input(20., 2., 10.),
                Branch::CompactTransition,
                true,
                [
                    20.0,
                    2.0,
                    100.0,
                    10.0,
                    2.4034659892569152,
                    500.0,
                    0.00480693197851383,
                    3.3237194693762966,
                    0.00480693197851383,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.009623119942914449,
                    19.94231681625778,
                    0.04806006379925151,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            ),
            (
                input(20., 10., 2.),
                Branch::CompactTransition,
                true,
                [
                    20.0,
                    10.0,
                    100.0,
                    2.0,
                    2.4034659892569152,
                    500.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.00480693197851383,
                    3.3237194693762966,
                    0.00480693197851383,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.04806006379925151,
                    19.94231681625778,
                    0.009623119942914449,
                ],
            ),
            (
                input(0.01, 2., 10.),
                Branch::CompactTransition,
                true,
                [
                    0.01,
                    2.0,
                    100.0,
                    10.0,
                    9600.0,
                    11520000.0,
                    0.0008333333333333334,
                    0.0,
                    0.0008333333333333334,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.002777777777777778,
                    0.0,
                    0.007222222222222222,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            ),
        ];
        for (input, branch, native_return, expected) in cases {
            let profile = construct_compact(input).unwrap();
            assert_eq!(profile.branch, branch);
            assert_eq!(profile.native_return, native_return, "{branch:?}");
            assert_eq!(profile.words(), expected, "{branch:?} {input:?}");
        }
        assert!(construct_compact(input(20., 0., 0.)).is_err());
    }

    /// A zero jerk is outside the domain.
    #[test]
    fn singular_inputs_are_not_admitted() {
        let mut singular = input(1., 0., 0.);
        singular.jerk = 0.;
        assert!(construct(singular).is_err());
    }
}
