// SPDX-License-Identifier: GPL-3.0-or-later

//! Physical units as newtypes, so a speed can never be handed to a field
//! that wants a length. Geometry stays in plain millimetre floats; these
//! wrap the values operators enter and jobs persist. They serialise as bare
//! numbers.

use serde::{Deserialize, Serialize};

macro_rules! unit {
    ($(#[$doc:meta])* $name:ident($inner:ty), $suffix:literal) => {
        $(#[$doc])*
        #[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $inner);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} {}", self.0, $suffix)
            }
        }
    };
}

unit!(
    /// A length.
    Millimeters(f64),
    "mm"
);
unit!(
    /// A speed.
    MmPerSecond(f64),
    "mm/s"
);
unit!(
    /// An angle.
    Degrees(f64),
    "°"
);
unit!(
    /// A share of full output.
    Percent(f64),
    "%"
);
unit!(
    /// A duration.
    Milliseconds(u32),
    "ms"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Units print with their suffix and serialise as bare numbers, so a
    /// job file reads naturally and a wrong unit is a type error, not a
    /// runtime surprise.
    #[test]
    fn units_display_and_serialise_as_numbers() {
        assert_eq!(Millimeters(2.5).to_string(), "2.5 mm");
        assert_eq!(Milliseconds(300).to_string(), "300 ms");
        assert_eq!(serde_json::to_string(&MmPerSecond(12.)).unwrap(), "12.0");
        assert_eq!(serde_json::from_str::<Degrees>("45").unwrap(), Degrees(45.));
        assert!(Percent(10.) < Percent(20.));
    }
}
