// SPDX-License-Identifier: GPL-3.0-or-later

//! `AutoCAD` colour indices as red, green and blue, and the short names a
//! layer of an entity's own colour takes.

/// An entity's colour: its layer's, its block reference's, or its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Color {
    /// The layer's colour.
    ByLayer,
    /// The colour of the reference that places its block.
    ByBlock,
    /// A colour of its own.
    Rgb([u8; 3]),
}

/// The colour of an `AutoCAD` colour index; the grey steps and the seven
/// named colours as `AutoCAD` shows them, and the rest from the hue wheel,
/// ten shades a hue.
#[must_use]
pub(crate) fn aci(index: u8) -> [u8; 3] {
    const NAMED: [[u8; 3]; 10] = [
        [0, 0, 0],
        [255, 0, 0],
        [255, 255, 0],
        [0, 255, 0],
        [0, 255, 255],
        [0, 0, 255],
        [255, 0, 255],
        [255, 255, 255],
        [128, 128, 128],
        [192, 192, 192],
    ];
    const GREYS: [u8; 6] = [51, 80, 105, 130, 190, 255];
    const LEVELS: [f64; 5] = [255., 165., 127., 76., 38.];
    match index {
        0..=9 => NAMED[usize::from(index)],
        250..=255 => [GREYS[usize::from(index - 250)]; 3],
        _ => {
            let at = index - 10;
            let hue = f64::from(at / 10) * 15.;
            let shade = at % 10;
            let level = LEVELS[usize::from(shade / 2)];
            let pure = hue_rgb(hue);
            let channel = |c: f64| {
                let value = if shade.is_multiple_of(2) {
                    c * level
                } else {
                    c * level + (1. - c) * level / 2.
                };
                // Rounded to a whole channel within 0 to 255.
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "0 to 255"
                )]
                let byte = value.round().clamp(0., 255.) as u8;
                byte
            };
            [channel(pure[0]), channel(pure[1]), channel(pure[2])]
        }
    }
}

/// A fully saturated colour at `hue` degrees, each channel 0 to 1.
fn hue_rgb(hue: f64) -> [f64; 3] {
    let sector = hue / 60.;
    let rise = sector.fract();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "0 to 5")]
    match sector.floor() as u8 % 6 {
        0 => [1., rise, 0.],
        1 => [1. - rise, 1., 0.],
        2 => [0., 1., rise],
        3 => [0., 1. - rise, 1.],
        4 => [rise, 0., 1.],
        _ => [1., 0., 1. - rise],
    }
}

/// A colour's short name: the named colours by name, others as hex.
#[must_use]
pub(crate) fn name(rgb: [u8; 3]) -> String {
    match rgb {
        [255, 0, 0] => "Red".into(),
        [255, 255, 0] => "Yellow".into(),
        [0, 255, 0] => "Green".into(),
        [0, 255, 255] => "Cyan".into(),
        [0, 0, 255] => "Blue".into(),
        [255, 0, 255] => "Magenta".into(),
        [255, 255, 255] => "White".into(),
        [0, 0, 0] => "Black".into(),
        [128, 128, 128] => "Grey".into(),
        [192, 192, 192] => "Light grey".into(),
        [r, g, b] => format!("#{r:02X}{g:02X}{b:02X}"),
    }
}

/// A colour from group 62 (an index; negative when the layer is off) or
/// 420 (a true colour), the true colour first.
pub(crate) fn from_groups(index: Option<f64>, true_color: Option<f64>) -> Color {
    if let Some(value) = true_color.filter(|v| (0. ..=16_777_215.).contains(v)) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "24 bits")]
        let value = value as u32;
        let [_, r, g, b] = value.to_be_bytes();
        return Color::Rgb([r, g, b]);
    }
    #[allow(clippy::cast_possible_truncation, reason = "a colour index is a small whole number")]
    match index.map(|v| v.abs() as i32) {
        Some(0) => Color::ByBlock,
        Some(n @ 1..=255) => Color::Rgb(aci(u8::try_from(n).unwrap_or(7))),
        _ => Color::ByLayer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_indices_match_autocads_table() {
        assert_eq!(aci(1), [255, 0, 0]);
        assert_eq!(aci(7), [255, 255, 255]);
        assert_eq!(aci(10), [255, 0, 0]);
        assert_eq!(aci(11), [255, 128, 128]);
        assert_eq!(aci(12), [165, 0, 0]);
        assert_eq!(aci(20), [255, 64, 0]);
        assert_eq!(aci(250), [51, 51, 51]);
        assert_eq!(name(aci(5)), "Blue");
        assert_eq!(name([18, 52, 86]), "#123456");
    }

    #[test]
    fn a_true_colour_wins_and_the_special_indices_defer() {
        assert_eq!(from_groups(Some(1.), Some(f64::from(0x0012_3456))), Color::Rgb([18, 52, 86]));
        assert_eq!(from_groups(Some(-3.), None), Color::Rgb([0, 255, 0]));
        assert_eq!(from_groups(Some(0.), None), Color::ByBlock);
        assert_eq!(from_groups(Some(256.), None), Color::ByLayer);
        assert_eq!(from_groups(None, None), Color::ByLayer);
    }
}
