// SPDX-License-Identifier: GPL-3.0-or-later

//! How much assist gas a nozzle passes at a pressure: an estimate for
//! costing, never a machine command.
//!
//! A cutting nozzle is a short orifice fed from the head's pressure
//! chamber. With the chamber above about 1.9 times the surrounding
//! absolute pressure, the flow in the throat is choked and the standard
//! volume flow grows linearly with the absolute chamber pressure:
//!
//! ```text
//! Q = Cd · (π/4) d² · P₀ · √(k / R T₀) · (2 / (k + 1))^((k+1) / 2(k-1)) / ρ_std
//! ```
//!
//! For air at 20 °C, a discharge coefficient `Cd` of 0.8 and `d` in
//! millimetres this is 7.4 L/min per mm² per bar absolute, the coefficient
//! the approved mockup uses. Below the choke point the flow follows the
//! isentropic subsonic law, which is the choked flow scaled by
//! [`subsonic_ratio`]. The standard volume of a different gas scales with
//! `√(M_air / M_gas)` at the same mass-flow law, so nitrogen passes about
//! 2 % more litres than air and oxygen about 5 % fewer.
//!
//! A double-layer nozzle's inner and outer streams are taken to pass 87.5 %
//! of a single nozzle of the same exit diameter, as the mockup assumes.
//! This and the discharge coefficient are not measured on a machine: every
//! gas has a calibration factor from the 60-second test that multiplies
//! the estimate.

use super::{GasKind, Nozzle, NozzleType};
use openlaser_core::units::{Bar, LitersPerMinute};

/// Standard litres per minute per mm² of nozzle diameter squared per bar
/// absolute, for air through a choked orifice with a discharge coefficient
/// of 0.8.
pub const CHOKED_COEFFICIENT: f64 = 7.4;

/// The flow of a double-layer nozzle relative to a single one.
pub const DOUBLE_NOZZLE: f64 = 0.875;

/// The surrounding pressure a gauge reading is relative to, in bar.
pub const ATMOSPHERE_BAR: f64 = 1.013_25;

/// The ratio of specific heats of the diatomic assist gases.
const K: f64 = 1.4;

/// The absolute to surrounding pressure ratio at which the throat chokes.
#[must_use]
pub fn critical_ratio() -> f64 {
    f64::midpoint(K, 1.).powf(K / (K - 1.))
}

/// The subsonic flow as a share of the choked flow at the same absolute
/// pressure; 1 at and above the choke point, 0 with no pressure.
#[must_use]
pub fn subsonic_ratio(absolute_bar: f64) -> f64 {
    if !absolute_bar.is_finite() || absolute_bar <= ATMOSPHERE_BAR {
        return 0.;
    }
    let r = ATMOSPHERE_BAR / absolute_bar;
    if r <= 1. / critical_ratio() {
        return 1.;
    }
    let subsonic = (2. / (K - 1.) * (r.powf(2. / K) - r.powf((K + 1.) / K))).sqrt();
    let choked = (2. / (K + 1.)).powf((K + 1.) / (2. * (K - 1.)));
    (subsonic / choked).clamp(0., 1.)
}

/// The standard volume of `gas` relative to air at the same orifice
/// conditions: `√(M_air / M_gas)`.
#[must_use]
pub fn gas_factor(gas: GasKind) -> f64 {
    const AIR: f64 = 28.965;
    match gas {
        GasKind::Air => 1.,
        GasKind::Nitrogen => (AIR / 28.014).sqrt(),
        GasKind::Oxygen => (AIR / 31.998).sqrt(),
    }
}

/// The uncalibrated flow of `gas` through `nozzle` at gauge `pressure`.
///
/// ```
/// use openlaser_core::units::{Bar, Millimeters};
/// use openlaser_server::gas::{GasKind, Nozzle, NozzleType, flow};
/// let nozzle = Nozzle { diameter: Millimeters(1.5), kind: NozzleType::Single };
/// let air = flow::estimate(GasKind::Air, nozzle, Bar(12.)).0;
/// assert!((air - 7.4 * 1.5 * 1.5 * 13.013).abs() < 0.1);
/// ```
#[must_use]
pub fn estimate(gas: GasKind, nozzle: Nozzle, pressure: Bar) -> LitersPerMinute {
    let d = nozzle.diameter.0;
    if !d.is_finite() || d <= 0. || !pressure.0.is_finite() || pressure.0 <= 0. {
        return LitersPerMinute(0.);
    }
    let absolute = pressure.0 + ATMOSPHERE_BAR;
    let layers = match nozzle.kind {
        NozzleType::Single => 1.,
        NozzleType::Double => DOUBLE_NOZZLE,
    };
    LitersPerMinute(
        CHOKED_COEFFICIENT * d * d * absolute * layers * subsonic_ratio(absolute) * gas_factor(gas),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlaser_core::units::Millimeters;

    const SINGLE: Nozzle = Nozzle { diameter: Millimeters(1.), kind: NozzleType::Single };

    #[test]
    fn the_coefficient_is_a_choked_air_orifice_with_a_discharge_coefficient_of_0_8() {
        // ṁ/(A·P₀) for air at 293.15 K, divided by the standard density.
        let (r, t, rho) = (287.05_f64, 293.15_f64, 1.2041_f64);
        let per_pa = (K / (r * t)).sqrt() * (2. / (K + 1.)).powf((K + 1.) / (2. * (K - 1.)));
        let area = std::f64::consts::PI / 4. * 1e-6;
        let litres_per_minute_per_bar = per_pa * area * 1e5 / rho * 1000. * 60.;
        assert!((0.8 * litres_per_minute_per_bar - CHOKED_COEFFICIENT).abs() < 0.05);
    }

    #[test]
    fn choked_flow_is_linear_in_absolute_pressure_and_subsonic_flow_falls_to_zero() {
        assert!((critical_ratio() - 1.893).abs() < 1e-3);
        let at = |bar| estimate(GasKind::Air, SINGLE, Bar(bar)).0;
        assert!((at(10.) - CHOKED_COEFFICIENT * (10. + ATMOSPHERE_BAR)).abs() < 1e-9);
        assert!((at(20.) / at(10.) - 21.013_25 / 11.013_25).abs() < 1e-9);
        // Continuous at the choke point, and below it less than linear.
        let choke = ATMOSPHERE_BAR * critical_ratio() - ATMOSPHERE_BAR;
        assert!((at(choke - 1e-6) - at(choke + 1e-6)).abs() < 1e-3);
        assert!(at(0.3) < CHOKED_COEFFICIENT * 1.313_25);
        assert!(at(0.3) > 0.);
        for none in [0., -1., f64::NAN] {
            assert!(at(none).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn nozzle_size_type_and_gas_scale_the_flow() {
        let base = estimate(GasKind::Air, SINGLE, Bar(12.)).0;
        let wide = Nozzle { diameter: Millimeters(2.), ..SINGLE };
        assert!((estimate(GasKind::Air, wide, Bar(12.)).0 / base - 4.).abs() < 1e-9);
        let double = Nozzle { kind: NozzleType::Double, ..SINGLE };
        assert!((estimate(GasKind::Air, double, Bar(12.)).0 / base - 0.875).abs() < 1e-9);
        let n2 = estimate(GasKind::Nitrogen, SINGLE, Bar(12.)).0 / base;
        let o2 = estimate(GasKind::Oxygen, SINGLE, Bar(12.)).0 / base;
        assert!((n2 - 1.017).abs() < 1e-3 && (o2 - 0.951).abs() < 1e-3);
    }
}
