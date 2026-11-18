// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! The Babyface Pro FS's hardware meters: bulk IN `0x85` on interface 1.
//!
//! The unit streams a 480-byte packet about every millisecond, unasked,
//! while it is in proprietary mode: forty `u64` RMS powers, then forty
//! `u32` peaks. The interface carries nothing the audio driver uses, so a
//! libusb client can claim it while `snd-usb-babyface-pro` runs.
//!
//! The scales are the ones the Fireface UCX uses (see
//! [`crate::latter::meters`]): a peak is `20·log10(v / 2^27)` dBFS (its low
//! four bits are flags), a power is `10·log10(v / 2^55)` dBFS. Checked on a
//! Babyface Pro FS with a -50 dBFS, 1 kHz tone played through a PipeWire
//! sink at volume 0.8 (-5.84 dB): the playback slot read -55.8 dB peak and
//! -58.8 dB RMS, the 3.0 dB between them being a sine's crest factor.
//!
//! The forty slots, as found with a tone on one playback channel at a time
//! and with an FX send on (the FX bus is the one slot that moves then):
//!
//! | slots | what |
//! |---|---|
//! | 0-11  | hardware inputs: AN1-4, AS1/2, ADAT3-8 (the capture order) |
//! | 12-25 | playback words: PB1-12, then two that carry nothing |
//! | 26-37 | hardware outputs: AN1/2, PH3/4, AS1/2, ADAT3-8 |
//! | 38-39 | the FX bus, left and right |
//!
//! Not checked: that inputs 4-11 read what is plugged in (nothing was on
//! the digital inputs), and the meaning of the two silent playback words.
//! The slot of an output is its position after the playback words, so a
//! tone on PB1 shows on the left channel of every output pair it is routed
//! to, which is how the output slots were told apart.

use crate::latter::meters::{peak_db, rms_db};

/// USB interface that carries the meters.
pub const INTERFACE: u8 = 1;
/// Bulk IN endpoint of the meters.
pub const ENDPOINT: u8 = 0x85;
/// Bytes in a packet.
pub const PACKET_LEN: usize = 480;
/// Meter slots in a packet.
pub const SLOTS: usize = 40;

const PEAKS_AT: usize = SLOTS * 8;

pub const INPUTS: std::ops::Range<usize> = 0..12;
pub const PLAYBACK: std::ops::Range<usize> = 12..26;
pub const OUTPUTS: std::ops::Range<usize> = 26..38;
pub const FX: std::ops::Range<usize> = 38..40;

/// One decoded packet. `None` is "below the meter's range".
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub peak_db: [Option<f32>; SLOTS],
    pub rms_db: [Option<f32>; SLOTS],
}

impl Packet {
    /// The packet in `buf`, or `None` unless it is exactly
    /// [`PACKET_LEN`] bytes.
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() != PACKET_LEN {
            return None;
        }
        let mut p = Packet {
            peak_db: [None; SLOTS],
            rms_db: [None; SLOTS],
        };
        for slot in 0..SLOTS {
            let rms = u64::from_le_bytes(buf[slot * 8..slot * 8 + 8].try_into().unwrap());
            let peak = u32::from_le_bytes(
                buf[PEAKS_AT + slot * 4..PEAKS_AT + slot * 4 + 4]
                    .try_into()
                    .unwrap(),
            );
            p.rms_db[slot] = rms_db(rms);
            p.peak_db[slot] = peak_db(peak);
        }
        Some(p)
    }

    /// Peak of hardware input `i` (device order), dBFS.
    pub fn input_peak(&self, i: usize) -> Option<f32> {
        self.peak_db.get(INPUTS.start + i).copied().flatten()
    }

    /// Peak of playback word `i`, dBFS.
    pub fn playback_peak(&self, i: usize) -> Option<f32> {
        self.peak_db.get(PLAYBACK.start + i).copied().flatten()
    }

    /// Peak of hardware output `i`, dBFS.
    pub fn output_peak(&self, i: usize) -> Option<f32> {
        self.peak_db.get(OUTPUTS.start + i).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real packet, read from a Babyface Pro FS while a -50 dBFS 1 kHz
    /// tone played on PB1 through a sink at volume 0.8.
    const TONE_ON_PB1: &str = concat!(
        "13852f010000000092310100000000005dc2000000000000adbf00000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "e17358f90a0000001f000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000009f839e8e010000002c02000000000000bb1e46c502000000b3ee120000000000",
        "227a68c002000000130613000000000010c9b0b000000000c6e51200000000003075a0b000000000dce2120000000000",
        "b281b6b0000000001fe61200000000001f000000000000001f00000000000000602c000070020000a0020000b0020000",
        "0000000000000000000000000000000000000000000000000000000000000000e0500300000000000000000000000000",
        "00000000000000000000000000000000000000000000000000000000000000000000000000000000f03f010040000000",
        "80b00100400b000040af0100400b000020db0000300b000020db0000300b000030db0000300b00000000000000000000"
    );

    fn from_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn a_real_packet_decodes_to_the_measured_levels() {
        let p = Packet::decode(&from_hex(TONE_ON_PB1)).unwrap();
        // PB1: -50 dBFS tone through a sink at 0.8 (0.8^3 = -5.84 dB).
        let pb1 = p.playback_peak(0).unwrap();
        assert!((pb1 - -55.8).abs() < 0.1, "{pb1}");
        let rms = p.rms_db[PLAYBACK.start].unwrap();
        assert!((rms - -58.8).abs() < 0.1, "{rms}");
        // A sine's peak is 3.0 dB above its RMS.
        assert!(((pb1 - rms) - 3.0).abs() < 0.1);
        // The other playback words are silent, as are the FX bus and the
        // two spare playback words.
        assert_eq!(p.playback_peak(1), None);
        assert_eq!(p.peak_db[FX.start], None);
        assert_eq!(p.playback_peak(12), None);
        assert_eq!(p.playback_peak(13), None);
        // PB1 is routed to the left of the output pairs: output 0 carries it.
        assert!(p.output_peak(0).unwrap() < pb1);
        // An analog input with nothing plugged in sits at its noise floor.
        assert!(p.input_peak(0).unwrap() < -70.0);
    }

    #[test]
    fn only_whole_packets_decode() {
        assert!(Packet::decode(&[0; PACKET_LEN]).is_some());
        assert!(Packet::decode(&[0; PACKET_LEN - 1]).is_none());
        assert!(Packet::decode(&[0; PACKET_LEN + 1]).is_none());
        let silent = Packet::decode(&[0; PACKET_LEN]).unwrap();
        assert!(silent
            .peak_db
            .iter()
            .chain(&silent.rms_db)
            .all(Option::is_none));
    }

    #[test]
    fn the_slots_cover_the_packet_once() {
        assert_eq!(INPUTS.end, PLAYBACK.start);
        assert_eq!(PLAYBACK.end, OUTPUTS.start);
        assert_eq!(OUTPUTS.end, FX.start);
        assert_eq!(FX.end, SLOTS);
        assert_eq!(PEAKS_AT + SLOTS * 4, PACKET_LEN);
    }

    #[test]
    fn full_scale_reads_zero_db() {
        let mut buf = vec![0u8; PACKET_LEN];
        buf[PEAKS_AT..PEAKS_AT + 4].copy_from_slice(&(1u32 << 27).to_le_bytes());
        buf[..8].copy_from_slice(&((1u64 << 55) / 2).to_le_bytes());
        let p = Packet::decode(&buf).unwrap();
        assert!(p.peak_db[0].unwrap().abs() < 0.001);
        assert!((p.rms_db[0].unwrap() + 3.01).abs() < 0.01);
    }
}
