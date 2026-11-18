// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! RME's "latter" control protocol: the 32-bit DSP words that the Fireface
//! UCX, 802, the UFX family and the UCX II exchange with TotalMix (bulk OUT
//! `0x0c` / IN `0x8d` on the UCX), the same words as RME's FireWire
//! "latter" Firefaces.
//!
//! This module is the transport-independent half of a future backend for
//! those units: encoding and decoding words, the mixer coefficient, the
//! meter packet, and the safety rules every writer must follow. It sends
//! nothing. No TuxMix backend drives a latter unit yet (see
//! `crate::profile::ControlProtocol::Latter`).
//!
//! Sources, re-implemented here rather than copied:
//! - realbogart/fireface-ucx-linux (MIT), `include/ucx_proto.h` and
//!   `docs/PROTOCOL.md`: the word layout, parity, the mixer coefficient,
//!   the dump request, the meter packet, the flash registers and the write
//!   pacing. Checked on a Fireface UCX by its author.
//! - OroboroModular/rme-fireface-ucx-linux-driver, `re/NOTES.md`: the same
//!   coefficient encoding measured independently (`0x9000` = 0 dB,
//!   `0xf000` = phase-inverted unity).
//!
//! The register numbers below are the UCX's. The other units share the
//! word format; their register maps differ in places (oscmix's
//! `device_ff*.c` has one per unit).

/// Bit 31: odd parity over the whole word.
pub const PARITY: u32 = 0x8000_0000;
/// Bit 30: the word is a mixer gain, not a per-channel parameter.
pub const MIXER: u32 = 0x4000_0000;

/// Writing [`ControlProtocol::Latter`]'s `refresh` value here makes the
/// unit send every register (1636 on the UCX). The dump carries no mixer
/// gains: the unit never reports those, so a backend must keep its own
/// copy and push it.
///
/// [`ControlProtocol::Latter`]: crate::profile::ControlProtocol::Latter
pub const REG_REFRESH: u16 = 0x3f99;
/// Polled with values 0-15 in turn (`0x3f00000N`) so the unit keeps
/// reporting register changes, front panel included.
pub const REG_POLL: u16 = 0x3f00;
/// Stores the whole state to a setup slot in flash. Never sent.
pub const REG_FLASH_STORE: u16 = 0x3e06;
/// Next to the flash store, guarded by a magic value in TotalMix, purpose
/// unknown. Never sent.
pub const REG_FLASH_NEAR: u16 = 0x3e05;

/// Vendor requests that program the unit's flash (FLASH_WRITE,
/// FLASH_STATUS). Never sent.
pub const FORBIDDEN_REQUESTS: [u8; 2] = [0x18, 0x19];
/// Vendor request CLOCK: on the UCX it detunes the sample clock until a
/// power cycle (realbogart). yonie's UCX II proof of concept sends it in
/// its init sequence, so it may be model-specific; not established. A
/// backend must not send it without a measurement on the model it drives.
pub const CAUTION_REQUEST_CLOCK: u8 = 0x1b;

/// Mixer source index of playback channel 0 (inputs start at 0).
pub const MIX_PLAYBACK: u16 = 0x20;
/// Mixer sources per output in the register space.
pub const MIX_STEP: u16 = 64;
/// Linear mixer gain of 0 dB.
pub const MIX_UNITY: i32 = 32768;
/// Largest linear mixer gain TotalMix writes (+6 dB).
pub const MIX_MAX: i32 = 65536;

/// `w` with its parity bit set so that the whole word has odd parity.
pub const fn with_parity(w: u32) -> u32 {
    let w = w & !PARITY;
    if w.count_ones() % 2 == 1 {
        w
    } else {
        w | PARITY
    }
}

/// Whether `w` has the odd parity the unit expects and sends.
pub const fn parity_ok(w: u32) -> bool {
    w.count_ones() % 2 == 1
}

/// The word that writes `value` to register `reg` (bits 30:16; a mixer
/// register carries bit 14 of `reg`, which lands on [`MIXER`]).
pub const fn word(reg: u16, value: u16) -> u32 {
    with_parity(((reg as u32 & 0x7fff) << 16) | value as u32)
}

/// The register and value of a word from the unit, or `None` when its
/// parity is wrong.
pub const fn decode(w: u32) -> Option<(u16, u16)> {
    if !parity_ok(w) {
        return None;
    }
    Some((((w >> 16) & 0x7fff) as u16, w as u16))
}

/// The register of parameter `param` on channel `channel` (inputs first,
/// then outputs: on the UCX channels 0-17 are inputs, 18-35 outputs).
pub const fn channel_reg(channel: u8, param: u8) -> u16 {
    ((channel as u16) << 8) | param as u16
}

/// The register of the mixer gain from `src` (an input, or
/// [`MIX_PLAYBACK`] + a playback channel) to output channel `out`.
pub const fn mix_reg(out: u16, src: u16) -> u16 {
    (MIXER >> 16) as u16 | (out * MIX_STEP + src)
}

/// The 16-bit coefficient of a linear mixer gain ([`MIX_UNITY`] = 0 dB,
/// negative = phase inverted). Small gains use a 15-bit signed value;
/// from 0.5 up, bit 15 switches to a scale 8 times coarser.
pub const fn mix_coef(gain: i32) -> u16 {
    if (gain + 0x4000) as u32 > 0x7fff {
        0x8000 | ((gain >> 3) & 0x7fff) as u16
    } else {
        (gain & 0x7fff) as u16
    }
}

/// The linear gain of a mixer coefficient: the inverse of [`mix_coef`],
/// to its precision.
pub const fn mix_gain(coef: u16) -> i32 {
    // Sign-extend the 15-bit value (bit 14 is its sign).
    let v = ((coef as i32) << 17) >> 17;
    if coef & 0x8000 != 0 {
        v * 8
    } else {
        v
    }
}

/// The word that sets the mixer gain from `src` to `out`.
pub const fn mix_word(out: u16, src: u16, gain: i32) -> u32 {
    word(mix_reg(out, src), mix_coef(gain))
}

/// The word that asks for a full register dump.
pub const fn refresh_word(refresh: u16) -> u32 {
    word(REG_REFRESH, refresh)
}

/// The `n`th poll word (`n` cycles 0-15).
pub const fn poll_word(n: u8) -> u32 {
    word(REG_POLL, (n & 0xf) as u16)
}

/// Whether a writer must refuse `w`: the flash registers.
pub const fn is_forbidden_word(w: u32) -> bool {
    let reg = ((w >> 16) & 0x7fff) as u16;
    reg == REG_FLASH_STORE || reg == REG_FLASH_NEAR
}

/// Whether a writer must refuse vendor request `request`.
pub const fn is_forbidden_request(request: u8) -> bool {
    request == FORBIDDEN_REQUESTS[0] || request == FORBIDDEN_REQUESTS[1]
}

/// Meter packets: 20 channels per bank, `u64` RMS power at 0x00, `u32`
/// peaks at 0xa0, the bank tag at 0xf0.
pub mod meters {
    /// Bytes in one meter packet.
    pub const PACKET_LEN: usize = 0xf4;
    /// Channels in one packet.
    pub const CHANNELS: usize = 20;
    const PEAK_AT: usize = 0xa0;
    const TAG_AT: usize = 0xf0;

    /// What a meter packet measures, from its tag.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Bank {
        /// Inputs, after the DSP.
        InputsPost,
        /// Inputs, before the DSP.
        InputsPre,
        /// Playback channels.
        Playback,
        /// Outputs, before their volume.
        OutputsPre,
        /// Outputs, after their volume.
        OutputsPost,
    }

    impl Bank {
        pub const fn from_tag(tag: u32) -> Option<Self> {
            match tag {
                0x1111_1111 => Some(Self::InputsPost),
                0x5555_5555 => Some(Self::InputsPre),
                0x2222_2222 => Some(Self::Playback),
                0x3333_3333 => Some(Self::OutputsPre),
                0x6666_6666 => Some(Self::OutputsPost),
                _ => None,
            }
        }
    }

    /// One decoded meter packet.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Packet {
        pub bank: Bank,
        /// Peak per channel, dBFS (`None` below the meter's range).
        pub peak_db: [Option<f32>; CHANNELS],
        /// RMS per channel, dBFS (`None` below the meter's range).
        pub rms_db: [Option<f32>; CHANNELS],
    }

    /// Peak in dBFS of a raw peak value (full scale 2^27; the low 4 bits
    /// are flags, not level).
    pub fn peak_db(raw: u32) -> Option<f32> {
        let v = raw & !0xf;
        (v != 0).then(|| 20.0 * (v as f32 / (1u32 << 27) as f32).log10())
    }

    /// RMS in dBFS of a raw power value (full scale 2^55).
    pub fn rms_db(raw: u64) -> Option<f32> {
        (raw != 0).then(|| 10.0 * (raw as f64 / (1u64 << 55) as f64).log10() as f32)
    }

    /// The packet in `buf`, or `None` when it is short or its tag unknown.
    pub fn decode(buf: &[u8]) -> Option<Packet> {
        if buf.len() < PACKET_LEN {
            return None;
        }
        let u32_at = |i: usize| u32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
        let u64_at = |i: usize| u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
        let bank = Bank::from_tag(u32_at(TAG_AT))?;
        let mut peak = [None; CHANNELS];
        let mut rms = [None; CHANNELS];
        for ch in 0..CHANNELS {
            rms[ch] = rms_db(u64_at(ch * 8));
            peak[ch] = peak_db(u32_at(PEAK_AT + ch * 4));
        }
        Some(Packet {
            bank,
            peak_db: peak,
            rms_db: rms,
        })
    }
}

/// Write pacing. The UCX buffers 128 words and drains about 12 per
/// millisecond; past that, every later transfer on the bulk OUT endpoint
/// times out for 20 s to over 2 minutes. The reference driver writes in
/// bursts of at most 63 words and paces them at 0.5 ms per word, far under
/// the drain rate. A writer must also keep reading the bulk IN endpoint
/// while it writes, or the unit's report queue fills and stalls large
/// writes.
#[derive(Debug, Clone)]
pub struct Pacer {
    /// Words that may still go out now.
    budget: f64,
    /// Time of the last refill, in microseconds on the caller's clock.
    last_us: u64,
}

impl Pacer {
    /// Most words in one transfer.
    pub const MAX_BURST: usize = 63;
    /// Words the pacer lets out per millisecond.
    pub const WORDS_PER_MS: f64 = 2.0;

    /// A pacer that starts with a full burst available at `now_us`.
    pub fn new(now_us: u64) -> Self {
        Self {
            budget: Self::MAX_BURST as f64,
            last_us: now_us,
        }
    }

    fn refill(&mut self, now_us: u64) {
        let dt_ms = now_us.saturating_sub(self.last_us) as f64 / 1000.0;
        self.budget = (self.budget + dt_ms * Self::WORDS_PER_MS).min(Self::MAX_BURST as f64);
        self.last_us = now_us.max(self.last_us);
    }

    /// How many of `wanted` words may be sent at `now_us`; the caller sends
    /// that many and comes back later for the rest.
    pub fn take(&mut self, now_us: u64, wanted: usize) -> usize {
        self.refill(now_us);
        let n = wanted
            .min(self.budget.floor() as usize)
            .min(Self::MAX_BURST);
        self.budget -= n as f64;
        n
    }

    /// Microseconds from `now_us` until at least one word may be sent.
    pub fn wait_us(&mut self, now_us: u64) -> u64 {
        self.refill(now_us);
        if self.budget >= 1.0 {
            0
        } else {
            ((1.0 - self.budget) * 1000.0 / Self::WORDS_PER_MS).ceil() as u64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_has_odd_parity() {
        for (reg, value) in [(0, 0), (0x3f99, 0x0812), (0x4000, 0x9000), (0x7fff, 0xffff)] {
            let w = word(reg, value);
            assert!(parity_ok(w), "{w:#010x}");
            assert_eq!(decode(w), Some((reg, value)));
        }
        // One bit flipped anywhere is caught.
        let w = word(0x1208, 1);
        for bit in 0..32 {
            assert_eq!(decode(w ^ (1 << bit)), None, "bit {bit}");
        }
    }

    #[test]
    fn the_dump_request_is_the_documented_word() {
        // realbogart: UCX_DSP_DUMP_REQUEST 0x3f990812 (before parity).
        assert_eq!(refresh_word(0x0812) & !PARITY, 0x3f99_0812);
        assert_eq!(poll_word(3) & !PARITY, 0x3f00_0003);
        assert_eq!(poll_word(19) & !PARITY, 0x3f00_0003, "n wraps at 16");
    }

    /// The coefficients both sources measured: 0x9000 = 0 dB,
    /// 0xf000 = phase-inverted unity (Oroboro `re/NOTES.md`, realbogart
    /// `UCX_DSP_MIX_UNITY`).
    #[test]
    fn mixer_coefficients_match_the_measurements() {
        assert_eq!(mix_coef(MIX_UNITY), 0x9000);
        assert_eq!(mix_coef(-MIX_UNITY), 0xf000);
        assert_eq!(mix_coef(0), 0);
        assert_eq!(mix_coef(MIX_MAX), 0xa000);
        assert_eq!(mix_gain(0x9000), MIX_UNITY);
        assert_eq!(mix_gain(0xf000), -MIX_UNITY);
        // 0x9fec: Oroboro's +6 dB reading, 1.995 x unity.
        let g = mix_gain(0x9fec) as f64 / MIX_UNITY as f64;
        assert!((g - 1.995).abs() < 0.001, "{g}");
    }

    #[test]
    fn mixer_coefficients_round_trip() {
        // Below 0.5 the fine scale is exact.
        for g in [-0x4000, -1, 1, 100, 0x3fff] {
            assert_eq!(mix_gain(mix_coef(g)), g, "{g}");
        }
        // Above it, the coarse scale keeps the gain to within one step of 8.
        for g in [0x4000, 20000, MIX_UNITY, 50000, MIX_MAX, -20000] {
            assert!((mix_gain(mix_coef(g)) - g).abs() < 8, "{g}");
        }
    }

    #[test]
    fn mixer_registers_carry_the_mixer_bit() {
        // Output 1 (index 0), input 3 (index 2).
        let w = mix_word(0, 2, MIX_UNITY);
        assert_eq!(w & MIXER, MIXER);
        assert_eq!(decode(w), Some((0x4002, 0x9000)));
        // Playback 1 to output 2: 0x4000 + 64 + 0x20.
        assert_eq!(mix_reg(1, MIX_PLAYBACK), 0x4060);
        // The FX send bus (outputs 0x1e/0x1f on the UCX).
        assert_eq!(mix_reg(0x1e, 0), 0x4780);
    }

    #[test]
    fn channel_registers() {
        // realbogart PROTOCOL.md: ch 18 param 8 = 0x1208 (Line Out ref level).
        assert_eq!(channel_reg(18, 0x08), 0x1208);
        assert_eq!(channel_reg(24, 0x08), 0x1808);
    }

    #[test]
    fn flash_is_refused() {
        assert!(is_forbidden_word(word(REG_FLASH_STORE, 0x0910)));
        assert!(is_forbidden_word(word(REG_FLASH_NEAR, 0)));
        assert!(!is_forbidden_word(word(REG_REFRESH, 0x0812)));
        assert!(!is_forbidden_word(mix_word(0, 0, MIX_UNITY)));
        assert!(is_forbidden_request(0x18) && is_forbidden_request(0x19));
        assert!(!is_forbidden_request(0x10));
    }

    #[test]
    fn meter_packets_decode() {
        let mut buf = vec![0u8; meters::PACKET_LEN];
        // Channel 0: full-scale peak (flag bits set, ignored), RMS -6 dB.
        buf[0xa0..0xa4].copy_from_slice(&((1u32 << 27) | 0xf).to_le_bytes());
        buf[0..8].copy_from_slice(&((1u64 << 55) / 4).to_le_bytes());
        // Channel 1: -20 dBFS peak.
        buf[0xa4..0xa8].copy_from_slice(&((1u32 << 27) / 10).to_le_bytes());
        buf[0xf0..0xf4].copy_from_slice(&0x2222_2222u32.to_le_bytes());
        let p = meters::decode(&buf).unwrap();
        assert_eq!(p.bank, meters::Bank::Playback);
        assert!(p.peak_db[0].unwrap().abs() < 0.01);
        assert!((p.rms_db[0].unwrap() + 6.02).abs() < 0.01);
        assert!((p.peak_db[1].unwrap() + 20.0).abs() < 0.01);
        assert_eq!(p.peak_db[2], None, "silence is below range");
        // Unknown tag, short packet.
        buf[0xf0..0xf4].copy_from_slice(&0u32.to_le_bytes());
        assert!(meters::decode(&buf).is_none());
        assert!(meters::decode(&buf[..100]).is_none());
    }

    #[test]
    fn the_pacer_never_overruns_the_unit() {
        let mut p = Pacer::new(0);
        assert_eq!(p.take(0, 200), Pacer::MAX_BURST);
        assert_eq!(p.take(0, 10), 0);
        assert_eq!(p.wait_us(0), 500);
        assert_eq!(p.take(500, 10), 1);
        // Over any second, at most a burst plus 2 words per ms go out:
        // far below the 12 per ms the UCX drains, so its 128-word buffer
        // never fills.
        let mut p = Pacer::new(0);
        let mut sent = 0;
        for t in (0..1_000_000).step_by(100) {
            sent += p.take(t, 1000);
        }
        assert!(sent <= Pacer::MAX_BURST + 2000, "{sent}");
        assert!(sent >= 1990, "{sent}");
    }
}
