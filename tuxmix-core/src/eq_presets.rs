// SPDX-License-Identifier: MIT
//! TotalMix FX's EQ presets for the hardware 3-band EQ of an input strip.
//!
//! The fourteen factory presets of the strip's preset menu and its Reset,
//! read off TotalMix FX on a Babyface Pro (band types from the type
//! icons, since the wire carries filter coefficients, not a type). Every
//! preset uses a 24 dB/oct low cut and leaves the EQ and the low-cut
//! switches as they were; only the low cut's frequency differs.

use crate::channel::EqBandType::{self, Bell, HighShelf, LowShelf};

/// One band of a preset: type, frequency in Hz, Q and gain in dB.
pub type PresetBand = (EqBandType, u16, f32, f32);

/// An EQ preset: three bands and the low cut's frequency and slope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqPreset {
    pub name: &'static str,
    pub bands: [PresetBand; 3],
    /// The low cut's frequency, in Hz. Whether it is on is the strip's own.
    pub low_cut_hz: u16,
    /// The low cut's slope, in dB/octave.
    pub low_cut_slope: u8,
}

const fn p(name: &'static str, bands: [PresetBand; 3], low_cut_hz: u16) -> EqPreset {
    EqPreset {
        name,
        bands,
        low_cut_hz,
        low_cut_slope: 24,
    }
}

/// TotalMix FX's Reset: flat bands at 50 Hz (low shelf), 1 kHz (bell) and
/// 10 kHz (high shelf), Q 1.0, low cut at 20 Hz with a 12 dB/oct slope.
pub const RESET: EqPreset = EqPreset {
    name: "Reset",
    bands: [
        (LowShelf, 50, 1.0, 0.0),
        (Bell, 1000, 1.0, 0.0),
        (HighShelf, 10_000, 1.0, 0.0),
    ],
    low_cut_hz: 20,
    low_cut_slope: 12,
};

/// The fourteen factory presets, in TotalMix FX's menu order.
pub const FACTORY: [EqPreset; 14] = [
    p(
        "Modern Bass Treble",
        [
            (LowShelf, 100, 1.2, 5.0),
            (Bell, 1000, 1.0, 0.0),
            (HighShelf, 5000, 1.2, 5.0),
        ],
        20,
    ),
    p(
        "Classic Bass Treble",
        [
            (LowShelf, 200, 0.7, 5.0),
            (Bell, 1000, 1.0, 0.0),
            (HighShelf, 2500, 0.7, 5.0),
        ],
        20,
    ),
    p(
        "Cheap Phones Compensation",
        [
            (Bell, 77, 0.9, 13.0),
            (Bell, 3000, 1.0, -4.0),
            (Bell, 10_000, 0.7, 7.0),
        ],
        20,
    ),
    p(
        "Glitter",
        [
            (Bell, 77, 0.9, 0.0),
            (Bell, 3000, 1.0, 0.0),
            (Bell, 14_000, 0.7, 5.5),
        ],
        20,
    ),
    p(
        "Deep Bass Boost",
        [
            (Bell, 60, 0.7, 10.0),
            (Bell, 260, 1.0, -4.0),
            (HighShelf, 5000, 1.2, 0.0),
        ],
        20,
    ),
    p(
        "Telephone (LC+EQ)",
        [
            (LowShelf, 730, 1.2, -20.0),
            (Bell, 4500, 1.7, 15.5),
            (HighShelf, 5500, 1.4, -20.0),
        ],
        500,
    ),
    p(
        "Low Frequency Effects",
        [
            (LowShelf, 110, 1.0, 20.0),
            (Bell, 700, 0.7, -20.0),
            (HighShelf, 1160, 0.7, -20.0),
        ],
        20,
    ),
    p(
        "Bass Drum",
        [
            (Bell, 80, 0.7, 12.0),
            (Bell, 540, 0.7, -8.0),
            (Bell, 4000, 1.2, 16.0),
        ],
        20,
    ),
    p(
        "Cymbals (LC+EQ)",
        [
            (Bell, 119, 0.7, -18.0),
            (Bell, 1010, 0.7, -18.0),
            (Bell, 10_000, 0.7, 12.0),
        ],
        500,
    ),
    p(
        "Wah",
        [
            (LowShelf, 300, 0.7, -20.0),
            (Bell, 470, 1.4, 20.0),
            (HighShelf, 4000, 1.0, -20.0),
        ],
        20,
    ),
    p(
        "Live Vocal Mic (LC+EQ)",
        [
            (Bell, 120, 0.7, 3.0),
            (Bell, 6000, 1.0, 3.0),
            (HighShelf, 13_800, 1.9, -10.5),
        ],
        80,
    ),
    p(
        "Peak",
        [
            (LowShelf, 50, 1.0, 0.0),
            (Bell, 1000, 5.0, 20.0),
            (HighShelf, 10_000, 1.0, 0.0),
        ],
        20,
    ),
    p(
        "Notch",
        [
            (LowShelf, 50, 1.0, 0.0),
            (Bell, 1000, 5.0, -20.0),
            (HighShelf, 10_000, 1.0, 0.0),
        ],
        20,
    ),
    p(
        "High Frequency Cut",
        [
            (LowShelf, 50, 1.0, 0.0),
            (Bell, 1000, 1.0, 0.0),
            (HighShelf, 14_000, 1.6, -20.0),
        ],
        20,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factory_menu_has_totalmixs_fourteen_presets_in_order() {
        let names: Vec<_> = FACTORY.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), 14);
        assert_eq!(names[0], "Modern Bass Treble");
        assert_eq!(names[10], "Live Vocal Mic (LC+EQ)");
        assert_eq!(names[13], "High Frequency Cut");
    }

    #[test]
    fn every_value_is_inside_the_eqs_range() {
        for preset in FACTORY.iter().chain([&RESET]) {
            assert!([6, 12, 18, 24].contains(&preset.low_cut_slope));
            assert!((20..=20_000).contains(&preset.low_cut_hz));
            for &(_, freq, q, gain) in &preset.bands {
                assert!((20..=20_000).contains(&freq), "{}", preset.name);
                assert!((0.05..=10.0).contains(&q), "{}", preset.name);
                assert!((-24.0..=24.0).contains(&gain), "{}", preset.name);
            }
        }
    }

    #[test]
    fn the_presets_with_a_low_cut_name_it() {
        let lc: Vec<_> = FACTORY
            .iter()
            .filter(|p| p.low_cut_hz != 20)
            .map(|p| (p.name, p.low_cut_hz))
            .collect();
        assert_eq!(
            lc,
            [
                ("Telephone (LC+EQ)", 500),
                ("Cymbals (LC+EQ)", 500),
                ("Live Vocal Mic (LC+EQ)", 80)
            ]
        );
    }
}
