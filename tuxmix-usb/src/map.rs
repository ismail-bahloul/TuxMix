//! Address map of the RME Babyface Pro FS proprietary protocol.
//!
//! All addresses were reverse-engineered from USB captures of TotalMix FX
//! driving the device (see `tools/usbdump/PROTOCOL.md` for the full RE
//! report). This module turns channel/output/source identifiers into the
//! register addresses used by the vendor requests.
//!
//! # Register map (16-bit crosspoints, `bReq = 0x12`)
//!
//! ```text
//! L register = 0x0034 + 0x0034 * out + src_idx
//! R register = 0x004E + 0x0034 * out + src_idx
//! ```
//!
//! A second "low map" mirroring the AN1/2 submix is written in sync:
//!
//! ```text
//! L register = 0x0000 + src_idx
//! R register = 0x001A + src_idx
//! ```
//!
//! # Output master faders
//!
//! ```text
//! 16-bit: 0x03E0 + 2 * out   (bReq 0x12)
//! 8-bit:  0x0004 + 2 * out   (bReq 0x1A)
//! ```
//!
//! # Source index space
//!
//! | Source | idx (L) | idx (R) |
//! |---|---|---|
//! | AN1 / AN2 / AN3 / AN4 | 0 / 1 / 2 / 3 | same |
//! | AS1/2 | 4 | 5 |
//! | ADAT3/4 | 6 | 7 |
//! | ADAT5/6 | 8 | 9 |
//! | ADAT7/8 | 10 | 11 |
//! | Playback N (1-6) | 12 + 2·(N-1) | 13 + 2·(N-1) |
//!
//! Mono sources use the same index on both L and R; stereo pairs use the
//! even index on L and the odd index on R.

/// A hardware input source (analog, AS or ADAT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    An1,
    An2,
    An3,
    An4,
    As12,
    Adat34,
    Adat56,
    Adat78,
}

impl Input {
    /// Left source index.
    pub fn index_l(self) -> usize {
        match self {
            Input::An1 => 0,
            Input::An2 => 1,
            Input::An3 => 2,
            Input::An4 => 3,
            Input::As12 => 4,
            Input::Adat34 => 6,
            Input::Adat56 => 8,
            Input::Adat78 => 10,
        }
    }

    /// Right source index (same as L for mono inputs).
    pub fn index_r(self) -> usize {
        match self {
            Input::An1 | Input::An2 | Input::An3 | Input::An4 => self.index_l(),
            Input::As12 => 5,
            Input::Adat34 => 7,
            Input::Adat56 => 9,
            Input::Adat78 => 11,
        }
    }

    /// Mic preamp index (0-3) for the gain/48V/PAD registers; `None` for
    /// inputs without a preamp (AS/ADAT).
    pub fn preamp_index(self) -> Option<usize> {
        match self {
            Input::An1 => Some(0),
            Input::An2 => Some(1),
            Input::An3 => Some(2),
            Input::An4 => Some(3),
            _ => None,
        }
    }
}

/// A software playback source (6 stereo pairs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback(pub usize);

impl Playback {
    /// Left source index (12 + 2·(n-1)).
    pub fn index_l(self) -> usize {
        12 + 2 * (self.0 - 1)
    }

    /// Right source index (13 + 2·(n-1)).
    pub fn index_r(self) -> usize {
        13 + 2 * (self.0 - 1)
    }
}

/// A physical output pair (strip order in TotalMix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    An12,
    Ph34,
    As12,
    Adat34,
    Adat56,
    Adat78,
}

impl Output {
    /// Strip index (0-5), used for the master faders.
    pub fn index(self) -> usize {
        match self {
            Output::An12 => 0,
            Output::Ph34 => 1,
            Output::As12 => 2,
            Output::Adat34 => 3,
            Output::Adat56 => 4,
            Output::Adat78 => 5,
        }
    }

    /// Crosspoint-map register block for this output.
    ///
    /// HARDWARE-VERIFIED 2026-08-24 (kernel driver): the crosspoint
    /// map lists the Phones FIRST — block 0 (0x0034) feeds the output
    /// whose MASTER is 0x03E2/0x0006 (PH3/4), block 1 (0x0068) feeds
    /// the 0x03E0/0x0004 master (AN1/2).  The master map has AN1/2
    /// first.  See PROTOCOL.md "Crosspoint address map".
    fn crosspoint_block(self) -> usize {
        match self {
            Output::An12 => 1,
            Output::Ph34 => 0,
            Output::As12 => 2,
            Output::Adat34 => 3,
            Output::Adat56 => 4,
            Output::Adat78 => 5,
        }
    }
}

/// A mixer source: a hardware input or a software playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Input(Input),
    Playback(Playback),
}

impl Source {
    /// Left source index in the register space.
    pub fn index_l(self) -> usize {
        match self {
            Source::Input(i) => i.index_l(),
            Source::Playback(p) => p.index_l(),
        }
    }

    /// Right source index in the register space.
    pub fn index_r(self) -> usize {
        match self {
            Source::Input(i) => i.index_r(),
            Source::Playback(p) => p.index_r(),
        }
    }
}

/// Register address of a crosspoint (L side) in the standard map.
pub fn crosspoint_l(out: Output, src: Source) -> usize {
    0x0034 + 0x0034 * out.crosspoint_block() + src.index_l()
}

/// Register address of a crosspoint (R side) in the standard map.
pub fn crosspoint_r(out: Output, src: Source) -> usize {
    0x004E + 0x0034 * out.crosspoint_block() + src.index_r()
}

/// Low-map mirror of [`crosspoint_l`] (AN1/2 submix, written in sync).
/// Base `0x0000` kept explicit (not just `src.index_l()`) to match its
/// sibling functions' `BASE + offset` shape and `PROTOCOL.md`'s own
/// documented formula, which is written the same way.
#[allow(clippy::identity_op)]
pub fn low_map_l(src: Source) -> usize {
    0x0000 + src.index_l()
}

/// Low-map mirror of [`crosspoint_r`].
pub fn low_map_r(src: Source) -> usize {
    0x001A + src.index_r()
}

/// 16-bit output master register (L side).
pub fn master_16_l(out: Output) -> usize {
    0x03E0 + 2 * out.index()
}

/// 16-bit output master register (R side).
pub fn master_16_r(out: Output) -> usize {
    0x03E1 + 2 * out.index()
}

/// 8-bit output master register (L side), written with `bReq = 0x1A`.
pub fn master_8_l(out: Output) -> usize {
    0x0004 + 2 * out.index()
}

/// 8-bit output master register (R side).
pub fn master_8_r(out: Output) -> usize {
    0x0005 + 2 * out.index()
}

/// Mic preamp gain register (0-3). Base `0x0000` kept explicit for the
/// same reason as [`low_map_l`].
#[allow(clippy::identity_op)]
pub fn gain_register(mic: usize) -> usize {
    0x0000 + mic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crosspoints_match_captured_addresses() {
        // Crosspoint-map output order CORRECTED 2026-08-24 (hardware):
        // block 0 = PH3/4 (Phones), block 1 = AN1/2.  The old cap_sweep
        // labels ("AN1 into AN1/2 wrote 0x0034") were mis-attributed.
        assert_eq!(
            crosspoint_l(Output::Ph34, Source::Input(Input::An1)),
            0x0034
        );
        assert_eq!(
            crosspoint_r(Output::Ph34, Source::Input(Input::An1)),
            0x004E
        );
        // AN1 into AN1/2 (block 1 = base 0x68).
        assert_eq!(
            crosspoint_l(Output::An12, Source::Input(Input::An1)),
            0x0068
        );
        assert_eq!(
            crosspoint_r(Output::An12, Source::Input(Input::An1)),
            0x0082
        );
        // AS1/2 input (idx 4/5) into PH3/4: 0x0038 / 0x0053.
        assert_eq!(
            crosspoint_l(Output::Ph34, Source::Input(Input::As12)),
            0x0038
        );
        assert_eq!(
            crosspoint_r(Output::Ph34, Source::Input(Input::As12)),
            0x0053
        );
        // ADAT7/8 base (block 5) confirmed by the scene load: 0x0138 / 0x0152.
        assert_eq!(
            crosspoint_l(Output::Adat78, Source::Input(Input::An1)),
            0x0138
        );
        assert_eq!(
            crosspoint_r(Output::Adat78, Source::Input(Input::An1)),
            0x0152
        );
        // Playbacks 3-6 into AN1/2 (block 1): 0x0078/0x0093, 0x007A/0x0095, ...
        assert_eq!(
            crosspoint_l(Output::An12, Source::Playback(Playback(3))),
            0x0078
        );
        assert_eq!(
            crosspoint_r(Output::An12, Source::Playback(Playback(3))),
            0x0093
        );
        assert_eq!(
            crosspoint_l(Output::An12, Source::Playback(Playback(6))),
            0x007E
        );
        assert_eq!(
            crosspoint_r(Output::An12, Source::Playback(Playback(6))),
            0x0099
        );
    }

    #[test]
    fn low_map_matches_captured_pairs() {
        // cap_srcmap.pcap: AN1 → (0x0000, 0x001A), AS1/2 → (0x0004, 0x001F),
        // playback 1 → (0x000C, 0x0027).
        assert_eq!(low_map_l(Source::Input(Input::An1)), 0x0000);
        assert_eq!(low_map_r(Source::Input(Input::An1)), 0x001A);
        assert_eq!(low_map_l(Source::Input(Input::As12)), 0x0004);
        assert_eq!(low_map_r(Source::Input(Input::As12)), 0x001F);
        assert_eq!(low_map_l(Source::Playback(Playback(1))), 0x000C);
        assert_eq!(low_map_r(Source::Playback(Playback(1))), 0x0027);
    }

    #[test]
    fn master_registers_match_captured_addresses() {
        // cap_as_test.pcap: AS1/2 master = 0x03E4/0x03E5 + 0x0008/0x0009.
        assert_eq!(master_16_l(Output::As12), 0x03E4);
        assert_eq!(master_16_r(Output::As12), 0x03E5);
        assert_eq!(master_8_l(Output::As12), 0x0008);
        assert_eq!(master_8_r(Output::As12), 0x0009);
        // PH3/4 master = 0x03E2/0x03E3 + 0x0006/0x0007.
        assert_eq!(master_16_l(Output::Ph34), 0x03E2);
        assert_eq!(master_8_l(Output::Ph34), 0x0006);
    }

    #[test]
    fn playback_indices_are_12_based() {
        assert_eq!(Playback(1).index_l(), 12);
        assert_eq!(Playback(1).index_r(), 13);
        assert_eq!(Playback(6).index_l(), 22);
        assert_eq!(Playback(6).index_r(), 23);
    }
}
