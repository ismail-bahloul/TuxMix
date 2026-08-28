//! Encoding of the Babyface Pro FS proprietary control commands.
//!
//! Every command is a USB **vendor control transfer** on endpoint 0 with
//! no data phase (the value lives in `wValue`, the sub-address in
//! `wIndex`):
//!
//! ```text
//! bmRequestType = 0x40  (write: OUT, vendor, device)
//! bRequest      = register / command code
//! wValue        = value to write
//! wIndex        = sub-address / channel  (high bits: transaction counter)
//! wLength       = 0
//! ```
//!
//! Decoded command families (see `tools/usbdump/PROTOCOL.md`):
//!
//! | `bRequest` | Purpose |
//! |---|---|
//! | `0x12` | 16-bit volume faders (crosspoints, masters, low map) |
//! | `0x1A` | 8-bit registers (gain, master companions, mute state) |
//! | `0x17` | preamp state bitmask (48V, PAD) — `wIndex = 0x003F` |
//! | `0x21` | commit, follows every preamp write |
//! | `0x10 0x8000`/`0x1D`/`0x14 0xC000` | session start (arm) |
//! | `0x13 0xC000` | session stop (disarm) |
//! | `0x11/0x1C/0x1E/0x1F` | status register polling (reads) |

use crate::map::{self, Output, Playback, Source};

/// A single vendor control request (OUT direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorRequest {
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
}

impl VendorRequest {
    /// Build a request with the standard OUT bmRequestType (0x40).
    pub const fn new(b_request: u8, w_value: u16, w_index: u16) -> Self {
        VendorRequest {
            b_request,
            w_value,
            w_index,
        }
    }
}

/// Transaction counter embedded in the high bits of `wIndex`.
///
/// TotalMix cycles the two high bits of `wIndex` on every fader write
/// (0xC000 → 0x4000 → 0x8000 → 0x0000). The device likely uses it only
/// for ordering/idempotency, but we reproduce it for fidelity.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlagCounter(u8);

impl FlagCounter {
    /// The current flag bits (0xC000/0x4000/0x8000/0x0000).
    pub fn current(&self) -> u16 {
        match self.0 {
            0 => 0xC000,
            1 => 0x4000,
            2 => 0x8000,
            _ => 0x0000,
        }
    }

    /// Advance to the next flag value.
    pub fn advance(&mut self) {
        self.0 = (self.0 + 1) & 0x03;
    }
}

/// Preamp state register (`bReq = 0x17`, `wIndex = 0x003F`).
///
/// The 0x17 state byte layout (verified on hardware 2026-08-22 — the
/// two front-panel P48 LEDs follow the bits exactly):
///
/// - bits 0-1: 48V for Mic 1 (AN1) / Mic 2 (AN2) — `0x0D` lights the
///   left P48 LED, `0x0E` the right one, `0x0F` both. The write is a
///   FULL state (writing 0x0E turns AN1 off).
/// - bits 4-5: PAD for Mic 1 / Mic 2 (`0x10 << mic`) — verified by the
///   relay click + a ~4.3 dB noise-floor drop on AN2 (bit 5). Bits
///   6-7 presumed PAD Mic 3/4.
/// - bits 2-3 are a constant base (0x0C) present in every state —
///   role unknown (no visible LED), keep them set.
///
/// The PAD bits (0x10/0x20) toggle physical relays — the audible
/// "click" (user observation; 48V itself is silent).
pub const PREAMP_REGISTER: u16 = 0x003F;
pub const PREAMP_48V_MIC1: u16 = 0x0001;
pub const PREAMP_48V_MIC2: u16 = 0x0002;
pub const PREAMP_BASE: u16 = 0x000C;
pub const PREAMP_48V_ON: u16 = PREAMP_BASE | PREAMP_48V_MIC1; // 0x000D, Mic 1
pub const PREAMP_48V_OFF: u16 = PREAMP_BASE; // 0x000C
pub const PREAMP_PAD_BIT: u16 = 0x0010; // Mic 1 PAD (0x10 << mic for 2-4)

/// 16-bit fader volume for a crosspoint (both L and R, centered/mono).
///
/// Returns the pair of requests TotalMix sends for a fader move. The
/// `flag` is the transaction counter used for these two writes.
pub fn set_crosspoint_volume(
    out: Output,
    src: Source,
    volume: u16,
    flag: &mut FlagCounter,
) -> [VendorRequest; 2] {
    let f = flag.current();
    flag.advance();
    [
        VendorRequest::new(0x12, volume, (map::crosspoint_l(out, src) as u16) | f),
        VendorRequest::new(0x12, volume, (map::crosspoint_r(out, src) as u16) | f),
    ]
}

/// Low-map mirror of [`set_crosspoint_volume`] (AN1/2 submix shadow).
pub fn set_low_map_volume(src: Source, volume: u16, flag: &mut FlagCounter) -> [VendorRequest; 2] {
    let f = flag.current();
    flag.advance();
    [
        VendorRequest::new(0x12, volume, (map::low_map_l(src) as u16) | f),
        VendorRequest::new(0x12, volume, (map::low_map_r(src) as u16) | f),
    ]
}

/// Stereo balance (pan): varies ONE side of the crosspoint pair.
///
/// `balance` is -1.0 (full left) .. 1.0 (full right); the fixed side is
/// passed as `fixed_volume`, the varied side is attenuated by `|balance|`.
/// (Observed behavior: TotalMix writes only the varying register, the
/// other side is untouched.)
pub fn set_crosspoint_balance(
    out: Output,
    src: Source,
    balance: f32,
    fixed_volume: u16,
    fixed_is_left: bool,
    flag: &mut FlagCounter,
) -> Vec<VendorRequest> {
    let f = flag.current();
    flag.advance();
    let vary_reg = if fixed_is_left {
        map::crosspoint_r(out, src)
    } else {
        map::crosspoint_l(out, src)
    };
    // Linear-in-raw attenuation (CALIBRATION.md "Pan", cap_pan_stereo.pcap,
    // 2026-08-22, hardware-confirmed): the fader-curve taper referenced by
    // the old comment here (-40 dB = 0x0317, -20 dB = 0x139E) was a position
    // estimate CALIBRATION.md later found to be wrong (0x0317 really about
    // -21 dB, 0x139E about -1.5 dB on that curve) -- this formula was never
    // a placeholder waiting on that curve, it IS the confirmed pan law.
    let varied = (fixed_volume as f32 * (1.0 - balance.abs())) as u16;
    vec![VendorRequest::new(0x12, varied, (vary_reg as u16) | f)]
}

/// Output master fader: 8-bit companion + 16-bit value (both L and R).
pub fn set_output_master(
    out: Output,
    volume_16: u16,
    volume_8: u8,
    flag: &mut FlagCounter,
) -> Vec<VendorRequest> {
    let f = flag.current();
    flag.advance();
    vec![
        // 8-bit companion registers (bReq 0x1A, no flag bits observed).
        VendorRequest::new(0x1A, volume_8 as u16, map::master_8_l(out) as u16),
        VendorRequest::new(0x1A, volume_8 as u16, map::master_8_r(out) as u16),
        // 16-bit master registers.
        VendorRequest::new(0x12, volume_16, (map::master_16_l(out) as u16) | f),
        VendorRequest::new(0x12, volume_16, (map::master_16_r(out) as u16) | f),
    ]
}

/// Mute an output master: force the 16-bit volume to 0 and set the 8-bit
/// companion to the mute state (0x003B).  Unmute restores the caller's
/// current volume (8-bit + 16-bit — the 8-bit is the REAL volume,
/// hardware-verified 2026-08-24).
pub fn set_output_master_mute(
    out: Output,
    muted: bool,
    restore_16: u16,
    restore_8: u8,
) -> Vec<VendorRequest> {
    if muted {
        vec![
            VendorRequest::new(0x1A, 0x003B, map::master_8_l(out) as u16),
            VendorRequest::new(0x1A, 0x003B, map::master_8_r(out) as u16),
            VendorRequest::new(0x12, 0x0000, map::master_16_l(out) as u16),
            VendorRequest::new(0x12, 0x0000, map::master_16_r(out) as u16),
        ]
    } else {
        vec![
            VendorRequest::new(0x1A, restore_8 as u16, map::master_8_l(out) as u16),
            VendorRequest::new(0x1A, restore_8 as u16, map::master_8_r(out) as u16),
            VendorRequest::new(0x12, restore_16, map::master_16_l(out) as u16),
            VendorRequest::new(0x12, restore_16, map::master_16_r(out) as u16),
        ]
    }
}

/// Mic preamp gain (8-bit register, `bReq = 0x1A`).
///
/// `value` is the raw gain code (5 bits, 0-31 ≈ 0-62 dB in 2-dB steps);
/// the high bits (5-6) carry a 3-state transaction counter (0x20 → 0x00
/// → 0x40) observed in TotalMix's writes. `cycle` must be `&mut 0` to
/// start. (Verified on hardware: the IN-stream level rises with the raw
/// value; the raw→dB anchor is pending a Windows calibration capture
/// (see WINDOWS-CAPTURE-PLAN.md; sweep shows saturation at raw 23).)
pub fn set_gain(mic: usize, value: u8, cycle: &mut u8) -> Vec<VendorRequest> {
    let counter = match *cycle % 3 {
        0 => 0x20,
        1 => 0x00,
        _ => 0x40,
    };
    *cycle = (*cycle + 1) % 3;
    let v = ((value & 0x1F) as u16) | counter;
    vec![VendorRequest::new(0x1A, v, map::gain_register(mic) as u16)]
}

/// Preamp STATE write only: the full 48V/PAD state byte + the 0x21
/// commit, WITHOUT the gain writes. Toggling 48V/PAD on one mic must
/// not clobber the four gains (they have no readback, so we can't
/// restore them) — the state byte is composed from ALL inputs by the
/// caller. The gains are written individually via [`set_gain`].
pub fn set_preamp_state(state: u16) -> Vec<VendorRequest> {
    vec![
        VendorRequest::new(0x17, state, PREAMP_REGISTER),
        VendorRequest::new(0x21, 0x0000, 0x0000),
    ]
}

/// Pitch/varispeed write: the 4-bank 0x1B quad + the clock keepalive.
/// Hardware-verified 2026-08-23 (pitchformula.c): the DERIVED quad
/// (bank1 = round(DDS16×0.72562), bank2 = round(DDS16×2/3), bank3 =
/// 0x7CFF frac 0, all fraction bytes 0) produces the identical IN rate
/// as the captured verbatim quad — the fraction bytes don't matter and
/// no bank3 lookup is needed.
///
/// `pitch_percent` = +4.0 → DDS_24 = round(50000×256/(1+4/100)).
pub fn set_pitch(pitch_percent: f32) -> Vec<VendorRequest> {
    let dds24 = (50000.0 * 256.0 / (1.0 + pitch_percent / 100.0)).round() as u32;
    let dds16 = (dds24 >> 8) as u16;
    let frac = (dds24 & 0xFF) as u16;
    let b1 = (dds16 as f32 * 0.72562).round() as u16;
    let b2 = (dds16 as f32 * 2.0 / 3.0).round() as u16;
    vec![
        VendorRequest::new(0x1B, dds16, (frac << 8) | 0), // bank 0 = DDS 16.8
        VendorRequest::new(0x1B, b1, 0x0001),             // bank 1
        VendorRequest::new(0x1B, b2, 0x0002),             // bank 2
        VendorRequest::new(0x1B, 0x7CFF, 0x0003),         // bank 3 (0% value)
        VendorRequest::new(0x10, 0x0001, 0x05CF),         // clock keepalive
    ]
}

// ── Sample rate ────────────────────────────────────────────────
//
// A rate change is PURELY `SET_INTERFACE(5, alt)` — no vendor writes
// (validated 2026-08-22 on Linux, ratetest.c, and the cap_rates2
// sweep). The alt is a BANDWIDTH CLASS, not a 1:1 rate code, and the
// per-frame byte count drops as the rate rises (fewer active channels:
// 14 ch ≤ 64k, 10 ch at 88.2-128k, 8 ch at 176.4/192k).

/// The alt-setting + frame layout for a sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateAlt {
    /// Audio interface alt-setting (1/2/3) to `SET_INTERFACE(5, alt)`.
    pub alt: u8,
    /// Bytes per audio frame at this rate (the frame width the URB
    /// size is derived from: 256 frames per URB).
    pub frame_bytes: usize,
}

/// Map a sample rate to its alt-setting + frame bytes.
/// `None` = rate the device does not support.
///
/// | alt | rates (kHz) | frame | URB (256 fr) |
/// |---|---|---|---|
/// | 1 | 32/44.1/48/64/88.2 | 56 B (14 ch) | 14336 B |
/// | 2 | 96/128 | 40 B (10 ch) | 10240 B |
/// | 3 | 176.4/192 | 32 B (8 ch) | 8192 B |
pub fn rate_to_alt(rate: u32) -> Option<RateAlt> {
    match rate {
        32000 | 44100 | 48000 | 64000 | 88200 => Some(RateAlt {
            alt: 1,
            frame_bytes: 56,
        }),
        96000 | 128000 => Some(RateAlt {
            alt: 2,
            frame_bytes: 40,
        }),
        176400 | 192000 => Some(RateAlt {
            alt: 3,
            frame_bytes: 32,
        }),
        _ => None,
    }
}

/// URB size for a rate: 256 frames per URB (the Windows driver's
/// cadence — 14336 B at 48 kHz — validated at all three alts by
/// ratetest.c).
pub fn rate_urb_size(rate: u32) -> Option<usize> {
    rate_to_alt(rate).map(|ra| ra.frame_bytes * 256)
}

/// The FRONT-PANEL gain write (cap_select.pcap, 2026-08-24): the wheel
/// in gain mode writes `0x1A` wIdx **0x000A + mic** (the "ADC gain"
/// register family — NOT the 0x0000+mic the GUI uses), raw value in
/// bits 0-4, NO cycling counter. Both channels written together in
/// linked mode. Relationship to the 0x0000+mic gain: TBD (Linux
/// hardware check).
pub fn set_panel_gain(mic: usize, value: u8) -> Vec<VendorRequest> {
    vec![VendorRequest::new(
        0x1A,
        (value & 0x1F) as u16,
        (0x000A + mic) as u16,
    )]
}

/// The `0x10 wIdx=0x05CF` host settings-state word (FULLY DECODED
/// 2026-08-22, cap_clk/cap_opt/cap_eqr): bit 0 = clock Internal,
/// bit 2 = clock Optical, bit 6 = EQ for Record, bit 10 = Optical Out
/// SPDIF (0 = ADAT). The ~3 s keepalive doubles as this command word.
pub fn settings_word(clock_optical: bool, eq_record: bool, spdif_out: bool) -> u16 {
    let mut w = if clock_optical { 0x0004 } else { 0x0001 };
    if eq_record {
        w |= 0x0040;
    }
    if spdif_out {
        w |= 0x0400;
    }
    w
}

/// Build the keepalive request carrying a settings-state word.
pub fn settings_keepalive(word: u16) -> VendorRequest {
    VendorRequest::new(0x10, word, 0x05CF)
}

// ── §9 controls (decoded on Windows 2026-08-23, see PROTOCOL.md) ─────

/// Loopback flag for one output channel — NEW bReq `0x15` (cap_ctrl3.pcap).
/// `channel` = 0-29 (the full channel map; AN1/2 output = 0/1).
/// 0x0001 = ON, 0x0000 = OFF.
pub fn set_loopback(channel: u16, on: bool) -> VendorRequest {
    VendorRequest::new(0x15, if on { 0x0001 } else { 0x0000 }, channel)
}

/// AN 1>2 toggle (cap_ctrl2.pcap): `0x17` wIdx=0x1000 (a NEW register),
/// bit 0x1000 of wValue = the flag — 0x0400 (off) ↔ 0x1400 (on).
/// Followed by the 0x21 COMMIT: without it the write does not fully
/// apply (verified on hardware 2026-08-23, an12probe.c — OFF without
/// commit left the AN1→AN2 route half-live at ≈ -41 dBFS).
/// Input-strip stereo-link state for the AN1/2 pair (`0x17` wIdx=0x1000,
/// cap_an2.pcap): 0x0000 = split (individual AN1/AN2 buses), 0x0400 =
/// linked (default — TotalMix's state; the pair moves together),
/// + 0x1000 = the AN 1>2 copy mode (only composed with linked, the
/// cap shows 0x1400 = linked + copy). Followed by the 0x21 commit.
pub fn set_input_link(linked: bool, an12: bool) -> Vec<VendorRequest> {
    let mut v = if linked { 0x0400 } else { 0x0000 };
    if an12 {
        v |= 0x1000;
    }
    vec![
        VendorRequest::new(0x17, v, 0x1000),
        VendorRequest::new(0x21, 0x0000, 0x0000),
    ]
}

/// MS-proc write of the AN2 crosspoints (cap_ctrl2.pcap): low map
/// 0x0001 + standard AN2→out0 0x0035. 0x0000 when MS is engaged; the
/// saved fader value when disengaged (capture: 0x068E → 0x0000 → 0x068E).
/// BOTH sides are written (R = low 0x001B + standard 0x004F): the
/// original capture only showed the L side, but the R crosspoint kept
/// the signal audible when only 0x0001/0x0035 were muted (verified on
/// hardware 2026-08-23, an12test.c).
pub fn set_ms_proc(value: u16) -> [VendorRequest; 4] {
    [
        VendorRequest::new(0x12, value, 0x0001), // AN2 L low map
        VendorRequest::new(0x12, value, 0x0035), // AN2 L standard → out0
        VendorRequest::new(0x12, value, 0x001B), // AN2 R low map
        VendorRequest::new(0x12, value, 0x004F), // AN2 R standard → out0
    ]
}

/// Phase Ø toggle (cap_ctrl.pcap): NEGATE the crosspoint coefficient
/// (Q15) on both maps. The observed write is the bitwise NOT of the
/// current fader value (0x0EA0 → 0xF15F, 0x018B → 0xFE74 — both are
/// `!value`). Writes the L crosspoints of `src` (low map + AN1/2
/// standard map, matching the capture; the R side is untouched).
pub fn set_phase(src: Source, value: u16) -> [VendorRequest; 2] {
    let neg = !value;
    [
        VendorRequest::new(0x12, neg, map::low_map_l(src) as u16),
        VendorRequest::new(0x12, neg, map::crosspoint_l(Output::An12, src) as u16),
    ]
}

/// FX send (AN1/2 → reverb/echo) level (cap_fx2.pcap): `0x12`
/// wIdx=0x0138/0x0153 (L/R pair, same value = mono send to stereo FX),
/// ramping 0x000C → 0x1000 (max observed). `value` = the raw send level
/// (the exact curve vs dB is uncalibrated — see `BabyfaceProUsb::set_fx_send`).
/// FX send level in dB (-65..0, 0 = max send, ≤-65 = off). Curve
/// CONFIRMED 2026-08-24 (cap_fx3.pcap, full up/down drags): raw =
/// 0x1000·2^(dB/6), 0 dB = 0x1000, slider bottom (-inf) = 0x0000,
/// -65 dB ≈ 0x0003 (anchors -60→0x0004, -54→0x0007, -36→0x0041,
/// -24→0x00F4 match within drag-sampling noise).
pub fn set_fx_send(value: u16) -> [VendorRequest; 2] {
    [
        VendorRequest::new(0x12, value, 0x0138),
        VendorRequest::new(0x12, value, 0x0153),
    ]
}

/// Stereo split of a playback strip into the AN1/2 output (cap_ctrl3.pcap):
/// stereo pair = 0x1000 (-6 dB per side) on both crosspoints; split-mono
/// = 0x2000 (0 dB, own side) / 0x0000 (muted, other side). Both the low
/// map and the standard map are rewritten, like every other fader write.
/// (The exact "alternating per mono channel" bus mapping needs a
/// hardware check — this writes the literal captured pattern into out0.)
pub fn set_stereo_split(pb: Playback, split: bool) -> Vec<VendorRequest> {
    let (l, r) = if split {
        (0x2000, 0x0000)
    } else {
        (0x1000, 0x1000)
    };
    let src = Source::Playback(pb);
    vec![
        VendorRequest::new(0x12, l, map::low_map_l(src) as u16),
        VendorRequest::new(0x12, r, map::low_map_r(src) as u16),
        VendorRequest::new(0x12, l, map::crosspoint_l(Output::An12, src) as u16),
        VendorRequest::new(0x12, r, map::crosspoint_r(Output::An12, src) as u16),
    ]
}

/// Trim (T) write for the AN1/2 pair (cap_trim2.pcap + cap_trim3/4,
/// 2026-08-24): on a LINKED strip TotalMix writes ALL EIGHT registers —
/// the low map (0x0000/0x001A/0x0001/0x001B = AN1 L/R + AN2 L/R) with
/// the MASTER curve value (`trim_raw`, 0x2000 = 0 dB display, 0 =
/// -inf) + the standard map (0x0034/0x004E/0x0035/0x004F) with the
/// COMBINED gain (`standard_raw` = fader × trim on the fader curve).
///
/// The two maps carry DIFFERENT values: the low map is the trim alone
/// (master curve), the standard map is fader·trim. The relation is
/// pinned from 188 labeled pairs in cap_trim2 (regression):
/// `standard_raw = 0x16A0 · 10^((fader_dB + trim_dB)/20)` — i.e. the
/// fader-curve value of the summed dB. The old ×27/256 placeholder
/// (and the "same value per map" note) was wrong.
pub fn set_trim(src: Source, trim_raw: u16, standard_raw: u16) -> Vec<VendorRequest> {
    let l = map::low_map_l(src) as u16;
    let r = map::low_map_r(src) as u16;
    let cl = map::crosspoint_l(Output::An12, src) as u16;
    let cr = map::crosspoint_r(Output::An12, src) as u16;
    vec![
        VendorRequest::new(0x12, trim_raw, l),
        VendorRequest::new(0x12, trim_raw, r),
        VendorRequest::new(0x12, trim_raw, l + 1), // AN2 L low map (adjacent)
        VendorRequest::new(0x12, trim_raw, r + 1), // AN2 R low map
        VendorRequest::new(0x12, standard_raw, cl),
        VendorRequest::new(0x12, standard_raw, cr),
        VendorRequest::new(0x12, standard_raw, cl + 1), // AN2 L standard
        VendorRequest::new(0x12, standard_raw, cr + 1), // AN2 R standard
    ]
}

/// Width knob on the AN1/2 INPUT strip (cap_width2.pcap, 2026-08-23):
/// the knob writes the low-map AN1/AN2 crosspoints as MIRROR balance pairs
/// (L+R = 0x2000 each; AN2 = the mirror of AN1), 0x0000/0x001A (AN1
/// L/R) + 0x0001/0x001B (AN2 L/R). At neutral (w=0) all four = 0x1000.
/// (The old 0x00AE/0x00C8/0x0046/0x0060 pairs from cap_ctrl belong to
/// a different strip — mapping still TBD.)
pub fn set_width(width: f32) -> Vec<VendorRequest> {
    let w = width.clamp(-1.0, 1.0);
    let l = ((0x2000u32 as f32 * (1.0 + w) / 2.0).round() as u32) as u16;
    let r = 0x2000 - l;
    vec![
        VendorRequest::new(0x12, l, 0x0000), // AN1 L low map
        VendorRequest::new(0x12, r, 0x001A), // AN1 R low map
        VendorRequest::new(0x12, r, 0x0001), // AN2 L low map (mirror)
        VendorRequest::new(0x12, l, 0x001B), // AN2 R low map (mirror)
    ]
}

/// Ref level (Instr 3/4, cap_ctrl2.pcap): `0x17` wIdx=0x003F (the SAME
/// preamp state register) + `0x21` commit. Observed values cycle
/// 0x0000 ↔ 0x000C — 0x0C is also `PREAMP_BASE`, so the ref code lives
/// in bits 2-3 (the 3-state bit map is TBD: 0x0000 / 0x000C observed,
/// third state unknown). `state` = the FULL state word — compose the
/// 48V/PAD bits in, or the write clobbers them.
/// Ref level (Instr 3/4) write: the `0x17` preamp-register state +
/// the `0x21` value — the 0x21 is NOT always the 0x0000 "commit", it
/// carries part of the 3-state code. LABELED 2026-08-24
/// (cap_reflevel2.pcap, started at +4dBu = 0x0F):
///
/// | State | 0x17 state | 0x21 |
/// |---|---|---|
/// | +4dBu | 0x000F | 0x0000 |
/// | -10dBV | 0x0003 | 0x0000 |
/// | Boost | 0x0003 | 0x0003 |
///
/// The state bits overlap the 48V/PAD region of the shared preamp
/// register; composition with an active 48V is untested (Instr 3/4 has
/// no phantom, so it doesn't collide in practice).
pub fn set_ref_level(state: u16, commit: u16) -> Vec<VendorRequest> {
    vec![
        VendorRequest::new(0x17, state, PREAMP_REGISTER),
        VendorRequest::new(0x21, commit, 0x0000),
    ]
}

// ── EQ (bulk OUT ep 0x0A, 64-byte coefficient blocks) ─────────────
//
// The EQ runs on the device DSP; TotalMix uploads the coefficients as
// 64-byte BULK OUT blocks on ep 0x0A (interface 1, mps 512) — NOT
// vendor-control writes. Layout (see PROTOCOL.md "Bulk OUT ep 0x0A"):
//
// ```
// 0x00 : [ch 0/1] [slope 2^n-1 | 0] [ch 0/1] [0x80 EQ active]
// 0x04 : slot 1 (Low)  b0 b1 b2 a1   (4 × i32 Q1.31-ish)
// 0x14 : slot 2 (Mid)  b0 b1 b2 a1
// 0x24 : slot 3 (High) b0 b1 b2 a1
// 0x34 : 5th coeff SHARED by the 3 slots (0x08000000 = passthrough)
// 0x38 : low-cut freq word (0x04000000 = low cut off)
// 0x3C : 0
// ```

/// 64-byte EQ coefficient block size (bulk OUT ep 0x0A).
pub const EQ_BLOCK_LEN: usize = 64;

/// The low-cut frequency word that disables the low cut (cap_eq7:
/// byte1 = 0x00 + 0x38 = 0x04000000 when off).
pub const LOW_CUT_OFF: u32 = 0x0400_0000;

/// Header byte 1 for a low-cut slope: `2^n − 1` (n = poles),
/// 0 = off. 6/12/18/24 dB per octave → 1/3/7/15 (cap_eq7, verified).
pub fn low_cut_slope_byte(slope_db_per_oct: u8) -> u8 {
    match slope_db_per_oct {
        6 => 0x01,
        12 => 0x03,
        18 => 0x07,
        24 => 0x0F,
        _ => 0x00, // off / unsupported
    }
}

/// The 0x38 low-cut frequency word for `freq_hz` at `slope_db_per_oct`.
///
/// Fit on cap_eq9 (labeled 12 dB/oct sweep, all 9 points, max error
/// 0.003%): `0x38 = round(K·f/(1+c·f))` with K = 11508, c = 1/11656.
/// The slope compensation scales the POLE frequency so the composite
/// −3 dB stays constant (cap_eq7 measured factors, exact to ±1 LSB on
/// the captured points): 6 dB/oct ×1.5267, 12 ×1.0000, 18 ×0.8061,
/// 24 ×0.6977.
pub fn low_cut_freq_raw(freq_hz: f32, slope_db_per_oct: u8) -> u32 {
    let factor = match slope_db_per_oct {
        6 => 1.5267,
        12 => 1.0,
        18 => 0.8061,
        24 => 0.6977,
        _ => return LOW_CUT_OFF,
    };
    let f = freq_hz * factor;
    (11508.0 * f / (1.0 + f / 11656.0)).round() as u32
}

/// Build one 64-byte EQ coefficient block (bulk OUT ep 0x0A).
///
/// `channel_left` selects the channel byte (0 = left, 1 = right — the
/// block is written twice, once per channel). `bands` = the 3 band
/// slots (4 signed coeffs each); `shared` = the 5th coeff at 0x34
/// (0x08000000 = no band active); `low_cut_freq` = the 0x38 word from
/// [`low_cut_freq_raw`] or [`LOW_CUT_OFF`]; `slope` = the header byte 1
/// from [`low_cut_slope_byte`].
pub fn eq_block(
    channel_left: bool,
    bands: &[[i32; 4]; 3],
    shared: i32,
    low_cut_freq: u32,
    slope: u8,
) -> [u8; EQ_BLOCK_LEN] {
    let ch = if channel_left { 0u8 } else { 1u8 };
    let mut b = [0u8; EQ_BLOCK_LEN];
    b[0] = ch;
    b[1] = slope;
    b[2] = ch;
    b[3] = 0x80; // EQ engine active
    for (slot, base) in bands.iter().zip([0x04usize, 0x14, 0x24]) {
        for (k, v) in slot.iter().enumerate() {
            b[base + 4 * k..base + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    b[0x34..0x38].copy_from_slice(&shared.to_le_bytes());
    b[0x38..0x3C].copy_from_slice(&low_cut_freq.to_le_bytes());
    b
}

/// The two EQ blocks (left + right channel) for a low-cut change,
/// with no band EQ active (all-zero band slots).
///
/// `freq_hz = None` turns the low cut off. Band slots are preserved by
/// re-passing them once the band biquad formula is decoded (the block
/// always carries the FULL EQ state — a low-cut-only write zeroes the
/// bands).
pub fn set_low_cut(
    freq_hz: Option<f32>,
    slope_db_per_oct: u8,
    bands: &[[i32; 4]; 3],
    shared: i32,
) -> [[u8; EQ_BLOCK_LEN]; 2] {
    let (freq, slope) = match freq_hz {
        Some(f) => (
            low_cut_freq_raw(f, slope_db_per_oct),
            low_cut_slope_byte(slope_db_per_oct),
        ),
        None => (LOW_CUT_OFF, 0),
    };
    [
        eq_block(true, bands, shared, freq, slope),
        eq_block(false, bands, shared, freq, slope),
    ]
}

/// EQ band filter type (the 3-band channel EQ: Low/Mid/High).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqType {
    /// Parametric bell (peaking) — cap_eq8a/b/c, EXACT RBJ cookbook fit
    /// (verified to ~1 LSB on the labeled gain/Q/freq sweeps).
    Bell,
    /// Low shelf — cap_eq8d (shelf sweep) PINNED 2026-08-24: RBJ low
    /// shelf with α = sin(w0)/(2Q) (the S=1 shelf used before was ~4e-4
    /// off; the Q-driven α matches the stored words to ~1e-6 at ≤1 kHz,
    /// the residual is the shared high-freq warping — see eq_biquad.md).
    LowShelf,
    /// High shelf — same formula family as [`EqType::LowShelf`]
    /// (a1 sign mirrored per the RBJ cookbook; no labeled high-shelf
    /// capture yet to confirm to LSB).
    HighShelf,
}

/// The 5 stored words for one EQ band slot (c0..c3 @ slot + c4 @ 0x34).
///
/// The storage is NOT the biquad directly — it is the RBJ cookbook
/// biquad split as a NORMALIZED numerator / NORMALIZED denominator with
/// the leading numerator coeff stored separately (decoded 2026-08-24,
/// see `tools/usbdump/eq_biquad.md`):
///
/// ```text
/// H(z) = c4·(1 + c2·z⁻¹ + c3·z⁻²) / (1 + c0·z⁻¹ + c1·z⁻²)
///        c0 = a1′    c1 = a2′    c2 = b1′/b0′    c3 = b2′/b0′    c4 = b0′
/// ```
///
/// where `′` = RBJ coefficients normalized by a0, and each stored word
/// is the value ×2²⁷ (signed i32; the ×16 that confused earlier fits is
/// the 2³¹/2²⁷ scale). The device reconstructs `b0 = c4`, `b1 = c2·c4`,
/// `b2 = c3·c4`, `a1 = c0`, `a2 = c1`.
///
/// `gain_db = 0` → identity ([0,0,0,0, 0x0800_0000]), matching TotalMix
/// (0 dB band = all-zero slot + the neutral b0 = 2²⁷ word).
///
/// Verified: cap_eq8c gain sweep −20..+20 dB reproduces the EXACT
/// labeled peak/notch (200 Hz, ±0.000 dB); cap_eq8b Q sweep (0.7..5)
/// and cap_eq8a freq sweep (50..10 kHz) match to ~1 LSB at low freq.
pub fn eq_band_storage(eq_type: EqType, freq_hz: f32, q: f32, gain_db: f32, fs: f32) -> [i32; 5] {
    let s = (1i64 << 27) as f32;
    if gain_db.abs() < 1e-6 {
        return [0, 0, 0, 0, s.round() as i32]; // 0x08000000
    }
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * freq_hz / fs;
    let c = w0.cos();
    // RBJ cookbook. Bell: α = sin(w0)/(2Q). Shelves: SAME Q-driven α
    // (cap_eq8d shelf sweep 2026-08-24: the implied α from the stored
    // a1'/a2' = sin(w0)/(2Q) to ~1e-5, vs the old S=1 which was ~4e-4
    // off; the residual is the same high-freq warping as the bell).
    let (b0, b1, b2, a0, a1, a2) = match eq_type {
        EqType::Bell => {
            let alpha = w0.sin() / (2.0 * q);
            (
                1.0 + alpha * a,
                -2.0 * c,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * c,
                1.0 - alpha / a,
            )
        }
        EqType::LowShelf => {
            let alpha = w0.sin() / (2.0 * q);
            let sq = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * c + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - sq),
                (a + 1.0) + (a - 1.0) * c + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - sq,
            )
        }
        EqType::HighShelf => {
            let alpha = w0.sin() / (2.0 * q);
            let sq = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * c + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - sq),
                (a + 1.0) - (a - 1.0) * c + sq,
                -2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - sq,
            )
        }
    };
    let (b0n, b1n, b2n) = (b0 / a0, b1 / a0, b2 / a0);
    let (a1n, a2n) = (a1 / a0, a2 / a0);
    let r = |v: f32| (v * s).round() as i32;
    [r(a1n), r(a2n), r(b1n / b0n), r(b2n / b0n), r(b0n)]
}

/// The two EQ blocks (left + right) for a single-band change, keeping
/// the other two slots as passed.
///
/// `slot` = 0 (Low) / 1 (Mid) / 2 (High). `current` = the other slots'
/// stored words (pass the current values so they survive).
pub fn set_eq_band(
    slot: usize,
    eq_type: EqType,
    freq_hz: f32,
    q: f32,
    gain_db: f32,
    fs: f32,
    current: &[[i32; 4]; 3],
) -> [[u8; EQ_BLOCK_LEN]; 2] {
    let mut bands = *current;
    let w = eq_band_storage(eq_type, freq_hz, q, gain_db, fs);
    bands[slot] = [w[0], w[1], w[2], w[3]];
    let shared = w[4];
    let (freq, slope) = (LOW_CUT_OFF, 0); // preserve the low cut? caller re-sends
    [
        eq_block(true, &bands, shared, freq, slope),
        eq_block(false, &bands, shared, freq, slope),
    ]
}

/// Preamp state write for one mic: 48V + PAD bits, followed by the
/// commit request and all four gain writes (matching TotalMix, which
/// rewrites the whole block on every change).
///
/// `mic` is 0-3 (AN1-AN4); the 48V/PAD bits are `1 << mic` / `0x10 <<
/// mic` (bits for Mic 3/4 verified by extrapolation).
pub fn set_preamp(mic: usize, phantom: bool, pad: bool, gain: [u8; 4]) -> Vec<VendorRequest> {
    let mut state = PREAMP_BASE;
    if phantom {
        state |= PREAMP_48V_MIC1 << mic;
    }
    if pad {
        state |= PREAMP_PAD_BIT << mic;
    }
    let mut reqs = vec![VendorRequest::new(0x17, state, PREAMP_REGISTER)];
    reqs.push(VendorRequest::new(0x21, 0x0000, 0x0000));
    let mut cycle = 0u8;
    for (m, g) in gain.iter().enumerate() {
        reqs.extend(set_gain(m, *g, &mut cycle));
    }
    reqs
}

/// Session-start sequence (cap_audio.pcap): `0x10 0x0000 0x8000` +
/// `0x1D 0x0000 0x0000` right before the audio URBs begin (frames
/// 5807/5809, 8405/8407 …), then `0x14 0x0000 0xC000` right after
/// (frame 5829, 114 µs after the first URB). Sending the trailing
/// `0x13 0xC000` stop write (as the old `keepalive` did) disarms the
/// session — the device then never activates its clock (0x17 byte 2
/// bit 7 stays 0) and 48V never engages.
pub fn session_start() -> [VendorRequest; 3] {
    [
        VendorRequest::new(0x10, 0x0000, 0x8000),
        VendorRequest::new(0x1D, 0x0000, 0x0000),
        VendorRequest::new(0x14, 0x0000, 0xC000),
    ]
}

/// Session-stop write (cap_audio.pcap frame 7397, right after the last
/// audio URB of a session): `0x13 0x0000 0xC000`.
pub fn session_stop() -> VendorRequest {
    VendorRequest::new(0x13, 0x0000, 0xC000)
}

/// Superseded cold-start handshake (from `cap_coldstart.pcap`): 30
/// zeroing writes (`bReq=0x15`, `wIndex` 0-29) followed by a single
/// `bReq=0x17 wValue=0x0000 wIndex=0x8080`.
///
/// This was the cap_coldstart-based hypothesis for arming the analog
/// front end; the cap_coldplug-based [`streaming_init`] superseded it.
/// Kept so the `cold48v` example still compiles/tests it.
pub fn cold_init() -> Vec<VendorRequest> {
    let mut reqs: Vec<VendorRequest> = (0..30u16)
        .map(|i| VendorRequest::new(0x15, 0x0000, i))
        .collect();
    reqs.push(VendorRequest::new(0x17, 0x0000, 0x8080));
    reqs
}

/// Superseded one-off `bReq=0x17 wValue=0x0000 wIndex=0xF040` write,
/// observed exactly once in `cap_coldstart.pcap` after the *first*
/// preamp block of a session. Companion to [`cold_init`]; kept for the
/// `cold48v` example.
pub fn preamp_arm() -> VendorRequest {
    VendorRequest::new(0x17, 0x0000, 0xF040)
}

/// Audio-session init sequence, captured verbatim from a cold-plug of
/// the device with the RME driver (cap_coldplug.pcap). Sent by the
/// driver right after `SELECT_CONFIG` + `SET_INTERFACE(5, alt 1)`,
/// before the isochronous stream. Without it the device never treats a
/// stream as valid (streaming bit stays 0, 48V never engages):
///
/// - `0x16` x60: clear registers 0x00-0x1D and 0x20-0x3D
/// - `0x1B` x4: clock/sample-rate setup (values captured at 48 kHz;
///   they likely change with the sample rate)
/// - `0x1C 0x0000 0x0000`
/// - `0x10 0x0021 0x05FF`
/// - `0x17` + `0x21`: preamp state write (wIdx = 0x0000 here)
/// - `0x10 0x0000 0x3000` x2, `0x10 0x0800 0x0800` x3
pub fn streaming_init(phantom: bool) -> Vec<VendorRequest> {
    let mut reqs = Vec::with_capacity(57 + 4 + 1 + 2 + 5);
    for idx in 0..=0x3D {
        if idx == 0x1E || idx == 0x1F {
            continue;
        }
        reqs.push(VendorRequest::new(0x16, 0x0000, idx));
    }
    reqs.push(VendorRequest::new(0x1B, 0xC350, 0x0000));
    reqs.push(VendorRequest::new(0x1B, 0x8DB8, 0xD201));
    reqs.push(VendorRequest::new(0x1B, 0x8234, 0xD302));
    reqs.push(VendorRequest::new(0x1B, 0x7CFF, 0xF803));
    reqs.push(VendorRequest::new(0x1C, 0x0000, 0x0000));
    reqs.push(VendorRequest::new(0x10, 0x0021, 0x05FF));
    reqs.push(VendorRequest::new(
        0x17,
        if phantom {
            PREAMP_48V_ON
        } else {
            PREAMP_48V_OFF
        },
        0x0000,
    ));
    reqs.push(VendorRequest::new(0x21, 0x0000, 0x0000));
    reqs.push(VendorRequest::new(0x10, 0x0000, 0x3000));
    reqs.push(VendorRequest::new(0x10, 0x0000, 0x3000));
    reqs.push(VendorRequest::new(0x10, 0x0800, 0x0800));
    reqs.push(VendorRequest::new(0x10, 0x0800, 0x0800));
    reqs.push(VendorRequest::new(0x10, 0x0800, 0x0800));
    reqs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Input;

    #[test]
    fn fader_writes_match_captured_values() {
        // cap_sweep.pcap: AN1 fader into AN1/2 at 0x0317 (≈ -40 dB) with
        // the transaction flag cycling.
        let mut flag = FlagCounter::default();
        let reqs =
            set_crosspoint_volume(Output::An12, Source::Input(Input::An1), 0x0317, &mut flag);
        assert_eq!(reqs[0].b_request, 0x12);
        assert_eq!(reqs[0].w_value, 0x0317);
        assert_eq!(reqs[0].w_index, 0xC068); // AN1/2 = block 1 (base 0x68) | flag 0xC000
        assert_eq!(reqs[1].w_index, 0xC082);

        // Next write cycles the flag to 0x4000.
        let reqs =
            set_crosspoint_volume(Output::An12, Source::Input(Input::An1), 0x0317, &mut flag);
        assert_eq!(reqs[0].w_index, 0x4068);
    }

    #[test]
    fn master_mute_matches_captured_sequence() {
        // cap_gain_solo.pcap mute: 0x1A 0x003B on 0x0004/0x0005 + 0x12 0
        // on 0x03E0/0x03E1.
        let reqs = set_output_master_mute(Output::An12, true, 0, 0);
        assert_eq!(
            reqs,
            vec![
                VendorRequest::new(0x1A, 0x003B, 0x0004),
                VendorRequest::new(0x1A, 0x003B, 0x0005),
                VendorRequest::new(0x12, 0x0000, 0x03E0),
                VendorRequest::new(0x12, 0x0000, 0x03E1),
            ]
        );
    }

    #[test]
    fn preamp_sequence_matches_hardware_48v_pad() {
        // 48V + PAD: 0x17 wVal=0x001D wIdx=0x003F + 0x21 commit + gains
        // (0x0D = 48V on, 0x10 = PAD — mapping verified on hardware
        // 2026-08-22: the P48 LED follows 0x0D/0x0C exactly).
        let reqs = set_preamp(0, true, true, [0, 0, 0, 0]);
        assert_eq!(reqs[0], VendorRequest::new(0x17, 0x001D, 0x003F));
        assert_eq!(reqs[1], VendorRequest::new(0x21, 0x0000, 0x0000));
        // 48V only: 0x000D.
        let reqs = set_preamp(0, true, false, [0, 0, 0, 0]);
        assert_eq!(reqs[0], VendorRequest::new(0x17, 0x000D, 0x003F));
        // 48V off: 0x000C (capture: 0x001C = off + PAD).
        let reqs = set_preamp(0, false, false, [0, 0, 0, 0]);
        assert_eq!(reqs[0], VendorRequest::new(0x17, 0x000C, 0x003F));
    }

    #[test]
    fn gain_counter_cycles_20_00_40() {
        let mut cycle = 0u8;
        let r1 = set_gain(0, 0x0A, &mut cycle);
        assert_eq!(r1[0].w_value, 0x002A); // value 0x0A + counter 0x20
        let r2 = set_gain(0, 0x0A, &mut cycle);
        assert_eq!(r2[0].w_value, 0x000A); // counter 0x00
        let r3 = set_gain(0, 0x0A, &mut cycle);
        assert_eq!(r3[0].w_value, 0x004A); // counter 0x40
    }

    #[test]
    fn pitch_quad_matches_hardware_formula() {
        // 0%: DDS_24 = 12800000 = 0xC35000 → bank0 0xC350/frac 0x00, then
        // bank1 = round(50000×0.72562) = 0x8DB9, bank2 = round(50000×2/3)
        // = 0x8235, bank3 = the 0% value 0x7CFF, + clock keepalive.
        let reqs = set_pitch(0.0);
        assert_eq!(reqs.len(), 5);
        assert_eq!(reqs[0], VendorRequest::new(0x1B, 0xC350, 0x0000));
        assert_eq!(reqs[1], VendorRequest::new(0x1B, 0x8DB9, 0x0001));
        assert_eq!(reqs[2], VendorRequest::new(0x1B, 0x8235, 0x0002));
        assert_eq!(reqs[3], VendorRequest::new(0x1B, 0x7CFF, 0x0003));
        assert_eq!(reqs[4], VendorRequest::new(0x10, 0x0001, 0x05CF));

        // +4%: DDS_24 = round(12800000/1.04) — the f32 arithmetic in the
        // formula lands on 0xBBCCED (frac 0xED; the double answer is
        // 0xEC — either is fine, the fraction byte is a don't-care,
        // verified by rate measurement, pitchformula.c). bank1 0x8845 /
        // bank2 0x7D33 match the captured verbatim quad from cap_fus2.pcap
        // exactly.
        let reqs = set_pitch(4.0);
        assert_eq!(reqs[0].b_request, 0x1B);
        assert_eq!(reqs[0].w_value, 0xBBCC);
        assert_eq!(reqs[0].w_index & 0xFF00, 0xED00); // bank 0, frac don't-care
        assert_eq!(reqs[1], VendorRequest::new(0x1B, 0x8845, 0x0001));
        assert_eq!(reqs[2], VendorRequest::new(0x1B, 0x7D33, 0x0002));
        assert_eq!(reqs[3], VendorRequest::new(0x1B, 0x7CFF, 0x0003));

        // Clamped to ±5%.
        let hi = set_pitch(12.0);
        let lo = set_pitch(-12.0);
        assert_ne!(hi[0], lo[0]);
    }

    #[test]
    fn rate_to_alt_matches_captured_classes() {
        // cap_rates2 sweep: alt 1 = 32/44.1/48/64/88.2k, alt 2 = 96/128k,
        // alt 3 = 176.4/192k. Frame bytes shrink with the class (14/10/8
        // channels).
        for r in [32000, 44100, 48000, 64000, 88200] {
            assert_eq!(
                rate_to_alt(r),
                Some(RateAlt {
                    alt: 1,
                    frame_bytes: 56
                })
            );
        }
        for r in [96000, 128000] {
            assert_eq!(
                rate_to_alt(r),
                Some(RateAlt {
                    alt: 2,
                    frame_bytes: 40
                })
            );
        }
        for r in [176400, 192000] {
            assert_eq!(
                rate_to_alt(r),
                Some(RateAlt {
                    alt: 3,
                    frame_bytes: 32
                })
            );
        }
        assert_eq!(rate_to_alt(0), None);
        assert_eq!(rate_to_alt(48001), None);
    }

    #[test]
    fn rate_urb_sizes_match_ratetest() {
        // ratetest.c: frame_bytes × 256 frames per URB (14336/10240/8192).
        assert_eq!(rate_urb_size(48000), Some(14336));
        assert_eq!(rate_urb_size(96000), Some(10240));
        assert_eq!(rate_urb_size(192000), Some(8192));
        assert_eq!(rate_urb_size(12345), None);
    }

    #[test]
    fn panel_gain_writes_adc_gain_registers() {
        // cap_select.pcap (2026-08-24): the panel wheel writes 0x1A
        // wIdx 0x000A+mic, raw in bits 0-4, no cycling counter.
        assert_eq!(
            set_panel_gain(0, 0x07),
            vec![VendorRequest::new(0x1A, 0x0007, 0x000A)]
        );
        assert_eq!(
            set_panel_gain(1, 0x12),
            vec![VendorRequest::new(0x1A, 0x0012, 0x000B)]
        );
    }

    #[test]
    fn settings_word_matches_captured_keepalives() {
        // cap_clk/cap_opt/cap_eqr: default 0x0001, optical 0x0004,
        // SPDIF 0x0401, EQ-record 0x0041 — additive bits over the
        // clock word.
        assert_eq!(settings_word(false, false, false), 0x0001);
        assert_eq!(settings_word(true, false, false), 0x0004);
        assert_eq!(settings_word(false, false, true), 0x0401);
        assert_eq!(settings_word(false, true, false), 0x0041);
        assert_eq!(
            settings_keepalive(0x0004),
            VendorRequest::new(0x10, 0x0004, 0x05CF)
        );
    }

    #[test]
    fn loopback_writes_channel_flags() {
        // cap_ctrl3.pcap: 0x15 wIdx=channel 0-29, wVal 0x0001/0x0000.
        assert_eq!(
            set_loopback(0, true),
            VendorRequest::new(0x15, 0x0001, 0x0000)
        );
        assert_eq!(
            set_loopback(1, false),
            VendorRequest::new(0x15, 0x0000, 0x0001)
        );
        assert_eq!(set_loopback(29, true), VendorRequest::new(0x15, 0x0001, 29));
    }

    #[test]
    fn trim_writes_all_eight_linked_registers() {
        // cap_trim2.pcap (2026-08-24, linked AN1/2 strip): the low map
        // 4 regs (0x0000/0x001A/0x0001/0x001B) carry the TRIM (master
        // curve), the standard 4 (0x0034/0x004E/0x0035/0x004F) the
        // COMBINED fader×trim (fader curve). Trim 0 dB + fader 0 dB →
        // low 0x2000, standard 0x16A0 (both = 0 dB on their curves).
        let reqs = set_trim(Source::Input(Input::An1), 0x2000, 0x16A0);
        assert_eq!(reqs.len(), 8);
        assert_eq!(reqs[0], VendorRequest::new(0x12, 0x2000, 0x0000));
        assert_eq!(reqs[1], VendorRequest::new(0x12, 0x2000, 0x001A));
        assert_eq!(reqs[2], VendorRequest::new(0x12, 0x2000, 0x0001));
        assert_eq!(reqs[3], VendorRequest::new(0x12, 0x2000, 0x001B));
        assert_eq!(reqs[4], VendorRequest::new(0x12, 0x16A0, 0x0068));
        assert_eq!(reqs[5], VendorRequest::new(0x12, 0x16A0, 0x0082));
        assert_eq!(reqs[6], VendorRequest::new(0x12, 0x16A0, 0x0069));
        assert_eq!(reqs[7], VendorRequest::new(0x12, 0x16A0, 0x0083));
        // Trim -6 dB (0x1000) + fader 0 dB → standard = fader curve at
        // -6 dB (0x0B57), NOT a fixed ratio of the low value.
        let reqs = set_trim(Source::Input(Input::An1), 0x1000, 0x0B57);
        assert_eq!(reqs[4], VendorRequest::new(0x12, 0x0B57, 0x0068));
    }

    #[test]
    fn input_link_composes_split_linked_an12() {
        // cap_ctrl2/cap_an2.pcap: the 0x1000 register + 0x21 commit.
        // 0x0000 = split, 0x0400 = linked, +0x1000 = AN 1>2 copy.
        assert_eq!(
            set_input_link(true, true),
            vec![
                VendorRequest::new(0x17, 0x1400, 0x1000),
                VendorRequest::new(0x21, 0x0000, 0x0000),
            ]
        );
        assert_eq!(
            set_input_link(true, false),
            vec![
                VendorRequest::new(0x17, 0x0400, 0x1000),
                VendorRequest::new(0x21, 0x0000, 0x0000),
            ]
        );
        // Split must stay split even with the AN1>2 flag off/on.
        assert_eq!(
            set_input_link(false, false),
            vec![
                VendorRequest::new(0x17, 0x0000, 0x1000),
                VendorRequest::new(0x21, 0x0000, 0x0000),
            ]
        );
        assert_eq!(
            set_input_link(false, true),
            vec![
                VendorRequest::new(0x17, 0x1000, 0x1000),
                VendorRequest::new(0x21, 0x0000, 0x0000),
            ]
        );
    }

    #[test]
    fn ms_proc_writes_an2_crosspoints() {
        // cap_ctrl2.pcap: low map 0x0001 + standard AN2→out0 0x0035.
        // Both SIDES: R = low 0x001B + standard 0x004F (the L-only write
        // left the R crosspoint live — audible on hardware, an12test.c).
        assert_eq!(
            set_ms_proc(0x0000),
            [
                VendorRequest::new(0x12, 0x0000, 0x0001),
                VendorRequest::new(0x12, 0x0000, 0x0035),
                VendorRequest::new(0x12, 0x0000, 0x001B),
                VendorRequest::new(0x12, 0x0000, 0x004F),
            ]
        );
        assert_eq!(
            set_ms_proc(0x068E)[0],
            VendorRequest::new(0x12, 0x068E, 0x0001)
        );
        assert_eq!(
            set_ms_proc(0x068E)[3],
            VendorRequest::new(0x12, 0x068E, 0x004F)
        );
    }

    #[test]
    fn phase_negates_crosspoint_value() {
        // cap_ctrl.pcap: 0x0EA0 → 0xF15F and 0x018B → 0xFE74 — both
        // bitwise NOTs, written on the low map + the AN1/2 standard map.
        let reqs = set_phase(Source::Input(Input::An1), 0x0EA0);
        assert_eq!(
            reqs,
            [
                VendorRequest::new(0x12, 0xF15F, 0x0000),
                VendorRequest::new(0x12, 0xF15F, 0x0068),
            ]
        );
        let reqs = set_phase(Source::Input(Input::An1), 0x018B);
        assert_eq!(reqs[0], VendorRequest::new(0x12, 0xFE74, 0x0000));
    }

    #[test]
    fn fx_send_writes_0138_0153() {
        // cap_fx2.pcap: L/R pair, same value (mono send to stereo FX).
        assert_eq!(
            set_fx_send(0x1000),
            [
                VendorRequest::new(0x12, 0x1000, 0x0138),
                VendorRequest::new(0x12, 0x1000, 0x0153),
            ]
        );
    }

    #[test]
    fn stereo_split_rewrites_playback_crosspoints() {
        // cap_ctrl3.pcap: split-mono = 0x2000/0x0000, stereo = 0x1000
        // each — low map (PB1 = 0x000C/0x0027) + the AN1/2 standard
        // block (block 1: L = 0x0068+12 = 0x0074, R = 0x0082+13 = 0x008F).
        let split = set_stereo_split(Playback(1), true);
        assert_eq!(
            split,
            vec![
                VendorRequest::new(0x12, 0x2000, 0x000C),
                VendorRequest::new(0x12, 0x0000, 0x0027),
                VendorRequest::new(0x12, 0x2000, 0x0074),
                VendorRequest::new(0x12, 0x0000, 0x008F),
            ]
        );
        let stereo = set_stereo_split(Playback(1), false);
        assert!(stereo.iter().all(|r| r.w_value == 0x1000));
    }

    #[test]
    fn width_writes_low_map_mirror_pairs() {
        // cap_width2.pcap (AN1/2 input strip): low-map AN1/AN2 as mirror
        // balance pairs, L+R = 0x2000. At +0.75: L = 0x2000·1.75/2 =
        // 0x1C00, R = 0x0400; AN2 = the mirror.
        let reqs = set_width(0.75);
        assert_eq!(
            reqs,
            vec![
                VendorRequest::new(0x12, 0x1C00, 0x0000), // AN1 L
                VendorRequest::new(0x12, 0x0400, 0x001A), // AN1 R
                VendorRequest::new(0x12, 0x0400, 0x0001), // AN2 L (mirror)
                VendorRequest::new(0x12, 0x1C00, 0x001B), // AN2 R (mirror)
            ]
        );
        // Neutral width = -6 dB on both sides of both pairs.
        let neutral = set_width(0.0);
        assert!(neutral.iter().all(|r| r.w_value == 0x1000));
        // Clamped to ±1.
        assert_eq!(set_width(5.0)[0].w_value, 0x2000);
        assert_eq!(set_width(-5.0)[0].w_value, 0x0000);
    }

    #[test]
    fn ref_level_writes_labeled_pairs() {
        // cap_reflevel2.pcap (2026-08-24, labeled): +4dBu = 0x17 0x0F +
        // 0x21 0x00, -10dBV = 0x03/0x00, Boost = 0x03/0x03.
        assert_eq!(
            set_ref_level(0x000F, 0x0000),
            vec![
                VendorRequest::new(0x17, 0x000F, 0x003F),
                VendorRequest::new(0x21, 0x0000, 0x0000),
            ]
        );
        assert_eq!(
            set_ref_level(0x0003, 0x0000)[0],
            VendorRequest::new(0x17, 0x0003, 0x003F)
        );
        assert_eq!(
            set_ref_level(0x0003, 0x0003)[1],
            VendorRequest::new(0x21, 0x0003, 0x0000)
        );
    }

    #[test]
    fn low_cut_freq_matches_cap_eq9_labeled_sweep() {
        // cap_eq9.pcap: low cut ON, 12 dB/oct, labeled freqs 20..500 Hz.
        // The captured 0x38 words (settled groups 1-5, 7-10 — group 6 is
        // the leftover 100 Hz state from the previous session):
        let captured = [
            (20, 0x0003_8180),
            (30, 0x0005_411A),
            (50, 0x0008_BE02),
            (75, 0x000D_15DD),
            (100, 0x0011_68FE),
            (150, 0x001A_0131),
            (200, 0x0022_86D9),
            (300, 0x0033_5B73),
            (500, 0x0054_301C),
        ];
        for (hz, want) in captured {
            let got = low_cut_freq_raw(hz as f32, 12);
            // Fit tolerance: ±0.003% of the captured word (the model
            // K=11508 c=1/11656 reproduces all points to ≤30 LSB).
            let tol = (want as f32 * 0.0001) as u32 + 2;
            assert!(
                got.abs_diff(want) <= tol,
                "freq {hz} Hz: got 0x{got:08X}, want 0x{want:08X}"
            );
        }
    }

    #[test]
    fn low_cut_slope_compensation_matches_cap_eq7() {
        // cap_eq7.pcap: fixed freq (20 Hz), slope 6/12/18/24 dB/oct.
        // Header byte1 = 2^n−1; the 0x38 words are the pole-frequency
        // compensated so the composite −3 dB stays constant:
        let captured = [
            (6, 0x01, 0x0005_58FF),
            (12, 0x03, 0x0003_8180),
            (18, 0x07, 0x0002_D3B9),
            (24, 0x0F, 0x0002_7285),
        ];
        for (slope, byte1, want) in captured {
            assert_eq!(low_cut_slope_byte(slope), byte1);
            let got = low_cut_freq_raw(20.0, slope);
            assert!(
                got.abs_diff(want) <= 6,
                "slope {slope}: got 0x{got:08X}, want 0x{want:08X}"
            );
        }
        assert_eq!(low_cut_slope_byte(0), 0x00);
        assert_eq!(low_cut_slope_byte(8), 0x00); // unsupported
    }

    #[test]
    fn eq_band_storage_matches_cap_eq8c_gain_sweep() {
        // cap_eq8c: Low bell @200 Hz Q=0.7, labeled gain sweep. The
        // reconstructed response must peak/notch at EXACTLY 200 Hz with
        // EXACTLY the labeled gain (verified offline, eq_validate2.py).
        // Check the stored words against the captured ones for +6 dB.
        // cap_eq8c g10 (+6 dB) captured words (×2^27):
        //   c0=-0.1233246·16? NO — stored value = word/2^27, and the
        //   *value* ×16 = the normalized coeff. Assert the round-trip:
        //   eq_band_storage → response peak == +6 dB @ 200 Hz.
        let w = eq_band_storage(EqType::Bell, 200.0, 0.7, 6.0, 48000.0);
        // Sanity: c4 = b0' ≈ 1.013003 → word ≈ 1.013003 × 2^27.
        let s = (1i64 << 27) as f64;
        let b0 = w[4] as f64 / s;
        assert!((b0 - 1.013003).abs() < 1e-4, "b0' = {b0}");
        let a1 = w[0] as f64 / s;
        assert!((a1 - -1.973195).abs() < 1e-4, "a1' = {a1}");
        let a2 = w[1] as f64 / s;
        assert!((a2 - 0.973872).abs() < 1e-4, "a2' = {a2}");
        // c2 = b1'/b0' = a1'/b0' (peaking has b1 = a1 = -2c).
        let c2 = w[2] as f64 / s;
        assert!((c2 - a1 / b0).abs() < 1e-4, "c2 = {c2}");
        // 0 dB → identity [0,0,0,0, 0x08000000].
        let id = eq_band_storage(EqType::Bell, 200.0, 0.7, 0.0, 48000.0);
        assert_eq!(id, [0, 0, 0, 0, 1 << 27]);
        // Negative gain mirrors: c0↔c2, c1↔c3 relative to +gain.
        let pos = eq_band_storage(EqType::Bell, 200.0, 0.7, 6.0, 48000.0);
        let neg = eq_band_storage(EqType::Bell, 200.0, 0.7, -6.0, 48000.0);
        assert!((neg[0] as f64 / s - pos[2] as f64 / s).abs() < 1e-4);
        assert!((neg[1] as f64 / s - pos[3] as f64 / s).abs() < 1e-4);
    }

    #[test]
    fn eq_band_storage_matches_cap_eq8d_low_shelf() {
        // cap_eq8d (shelf sweep, 2026-08-24): the stored words for the
        // +6 dB / -6 dB @ 200 Hz Q=0.7 low shelves match the RBJ low
        // shelf (α = sin(w0)/(2Q)) to ~1e-6 (vs ~4e-4 with the old S=1).
        // Captured words (value = word/2^27):
        //   +6: c0=-1.968542 c1=+0.969020 c2=-1.955575 c3=+0.956522 c4=+1.006509
        //   -6: c0=-1.955575 c1=+0.956522 c2=-1.968542 c3=+0.969020 c4=+0.993533
        let s = (1i64 << 27) as f64;
        let p6 = eq_band_storage(EqType::LowShelf, 200.0, 0.7, 6.0, 48000.0);
        let cap_p6 = [-1.968542, 0.969020, -1.955575, 0.956522, 1.006509];
        for (w, cap) in p6.iter().zip(cap_p6.iter()) {
            assert!(
                (*w as f64 / s - cap).abs() < 1e-5,
                "+6 shelf word {w:08X} vs captured {cap}"
            );
        }
        let m6 = eq_band_storage(EqType::LowShelf, 200.0, 0.7, -6.0, 48000.0);
        let cap_m6 = [-1.955575, 0.956522, -1.968542, 0.969020, 0.993533];
        for (w, cap) in m6.iter().zip(cap_m6.iter()) {
            assert!(
                (*w as f64 / s - cap).abs() < 1e-5,
                "-6 shelf word {w:08X} vs captured {cap}"
            );
        }
    }

    #[test]
    fn eq_band_response_peaks_at_labeled_gain() {
        // Offline-verified: the reconstructed biquad (b0=c4, b1=c2·c4,
        // b2=c3·c4, a1=c0, a2=c1) peaks at the labeled freq with the
        // labeled gain. Re-derive here from eq_band_storage and check
        // the response at the peak.
        fn resp(b: &[f64; 5], f: f64) -> f64 {
            let w = 2.0 * std::f64::consts::PI * f / 48000.0;
            let (re, im) = (w.cos(), w.sin());
            let num = (b[0] + b[1] * re + b[2] * (re * re - im * im))
                .hypot(b[1] * im + b[2] * 2.0 * re * im);
            let den = (1.0 + b[3] * re + b[4] * (re * re - im * im))
                .hypot(b[3] * im + b[4] * 2.0 * re * im);
            num / den
        }
        let s = (1i64 << 27) as f64;
        for (gain, want) in [
            (-20.0, -20.0),
            (-6.0, -6.0),
            (3.0, 3.0),
            (10.0, 10.0),
            (20.0, 20.0),
        ] {
            let w = eq_band_storage(EqType::Bell, 200.0, 0.7, gain, 48000.0);
            let b = [
                w[4] as f64 / s,
                w[2] as f64 / s * w[4] as f64 / s,
                w[3] as f64 / s * w[4] as f64 / s,
                w[0] as f64 / s,
                w[1] as f64 / s,
            ];
            // |H| at 200 Hz == 10^(gain/20).
            let h = resp(&b, 200.0);
            let db = 20.0 * h.log10();
            assert!(
                (db - want).abs() < 0.01,
                "gain {gain}: |H(200)| = {db:.3} dB (want {want})"
            );
        }
    }

    #[test]
    fn eq_block_layout_matches_captured_low_cut() {
        // cap_eq9 group 1 settled block (20 Hz, 12 dB/oct, R channel):
        // 01 03 01 80 | 48×00 | 08 00 00 00 (0x34) | 80 81 03 00 (0x38)
        // | 00 00 00 00 (0x3C).
        let bands = [[0i32; 4]; 3];
        let blk = eq_block(false, &bands, 0x0800_0000, 0x0003_8180, 0x03);
        assert_eq!(&blk[0..4], &[0x01, 0x03, 0x01, 0x80]);
        assert!(blk[0x04..0x34].iter().all(|&b| b == 0));
        assert_eq!(
            u32::from_le_bytes(blk[0x34..0x38].try_into().unwrap()),
            0x0800_0000
        );
        assert_eq!(
            u32::from_le_bytes(blk[0x38..0x3C].try_into().unwrap()),
            0x0003_8180
        );
        assert!(blk[0x3C..0x40].iter().all(|&b| b == 0));

        // Left channel flips the channel bytes.
        let l = eq_block(true, &bands, 0x0800_0000, 0x0003_8180, 0x03);
        assert_eq!(&l[0..4], &[0x00, 0x03, 0x00, 0x80]);

        // Band slots land at their offsets (slot 2 @ 0x14).
        let mut b2 = [[0i32; 4]; 3];
        b2[1] = [0x1234_5678, -2, 3, -4];
        let blk = eq_block(true, &b2, 0x0800_0000, LOW_CUT_OFF, 0x00);
        assert_eq!(
            u32::from_le_bytes(blk[0x14..0x18].try_into().unwrap()),
            0x1234_5678
        );
        assert_eq!(i32::from_le_bytes(blk[0x18..0x1C].try_into().unwrap()), -2);

        // OFF: byte1 = 0, 0x38 = LOW_CUT_OFF.
        let [l, r] = set_low_cut(None, 12, &bands, 0x0800_0000);
        assert_eq!(l[1], 0x00);
        assert_eq!(
            u32::from_le_bytes(l[0x38..0x3C].try_into().unwrap()),
            LOW_CUT_OFF
        );
        assert_eq!(r[0], 0x01);
    }
}
