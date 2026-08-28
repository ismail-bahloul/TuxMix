//! Babyface Pro FS implementation of [`RmeDevice`] over the proprietary
//! USB backend (`tuxmix-usb`).
//!
//! This is the TotalMix-class driver: it opens the device via libusb and
//! drives the DSP mixer with vendor control requests, exactly like
//! TotalMix FX does on Windows. The protocol was reverse-engineered from
//! USB captures — see `tools/usbdump/PROTOCOL.md`.
//!
//! # Status / caveats
//!
//! - Volume mapping is **calibrated** from cap_calib.pcap (2026-08-22):
//!   crosspoint faders use the measured 70-point curve (-inf..+6 dB,
//!   -20 dB = 0x0243, 0 dB = 0x16A0); the output master uses the
//!   exponential fit (0 dB = 0x2000, +6 dB = 0x4000). See
//!   `tools/usbdump/CALIBRATION.md`.
//! - Gain is calibrated: raw 0-20 ≈ 0-65 dB (3.25 dB per raw step); the
//!   UI model tracks gain in **dB** (0-65), converted to raw on write.
//! - Sensitivity, SPDIF and clock-source controls are not mapped in the
//!   protocol yet and return errors.
//! - Solo is written to the global solo registers (the per-channel solo
//!   mapping is unresolved in the RE).

use crate::channel::{
    ChannelId, ChannelType, InputChannel, OutputChannel, PlaybackChannel, Sensitivity,
};
use crate::curves::{fader_db_to_raw, fader_raw_to_db, FADER_CURVE, FADER_MUTE_RAW};
use crate::device::{DeviceSettings, RmeDevice};
use crate::error::Error;
use crate::panel::{PanelDriver, PanelEvent, PanelState, SelectState};
use crate::scene::Scene;
use tuxmix_usb::device::BabyfaceUsb;
use tuxmix_usb::map::{Input, Output, Playback, Source};

/// Output-master value at 0 dB (exponential fit, cap_calib.pcap:
/// 0 dB = 0x2000, +6 dB = 0x4000 = 2×0x2000 → +6.02 dB).
const MASTER_0DB: u16 = 0x2000;
/// Output-master value at +6 dB (the fader top).
const MASTER_TOP_RAW: u16 = 0x4000;
/// Preamp gain max in dB for Mic inputs (raw 0-20 ≈ 0-65 dB).
const GAIN_DB_MAX: u32 = 65;
/// Preamp gain max in dB for Instrument inputs (kernel control 0-9 dB,
/// raw = dB×2 = 0-18 — cap_gain34.pcap, 2026-08-26).
const GAIN_DB_MAX_INSTR: u32 = 9;
/// Calibrated preamp-gain step: 65 dB over 20 raw steps.
const GAIN_DB_PER_STEP: f32 = 3.25;

/// Ref-level codes (Instr 3/4) stored in [`InputChannel::ref_level`]
/// and passed to [`RmeDevice::set_ref_level`]. 0 = unset (a scene
/// without a ref level doesn't write it). LABELED from cap_reflevel2
/// (2026-08-24): +4dBu = (0x17 0x000F, 0x21 0x0000), -10dBV =
/// (0x0003, 0x0000), Boost = (0x0003, 0x0003).
pub const REF_PLUS_4DBU: u16 = 1;
pub const REF_MINUS_10DBV: u16 = 2;
pub const REF_BOOST: u16 = 3;

/// Calibrated output-master curve: exponential with exact doubling per
/// +6 dB (0 dB = 0x2000, +6 dB = 0x4000 — matches the master sweep and
/// scene loads). `db <= -64` → mute (0x0000).
fn master_db_to_raw(db: f32) -> u16 {
    if db <= -64.0 {
        return 0;
    }
    let raw = MASTER_0DB as f32 * 2f32.powf(db / 6.0);
    raw.round().clamp(0.0, MASTER_TOP_RAW as f32) as u16
}

/// The 8-bit output-master register is the REAL volume
/// (HARDWARE-VERIFIED 2026-08-24, kernel driver): 0.5 dB per step,
/// 0xF3 = 0 dB, 0x73 = -64 dB (silence), 0xFF = +6 dB, mute = 0x3B.
/// The 16-bit register is a companion kept in sync.  Writing a
/// constant 0xF3 (the old code) left the master volume stuck at 0 dB.
fn master_8bit(db: f32) -> u8 {
    // 0xF3 = 243 = 0 dB, 0x73 = 115 = -64 dB, 0xFF = 255 = +6 dB.
    (243.0 + 2.0 * db).round().clamp(115.0, 255.0) as u8
}

/// The Babyface Pro FS driven over the proprietary USB protocol.
pub struct BabyfaceProUsb {
    dev: BabyfaceUsb,
    inputs: Vec<InputChannel>,
    playbacks: Vec<PlaybackChannel>,
    outputs: Vec<OutputChannel>,
    settings: DeviceSettings,
    panel: PanelDriver,
    /// Current MIX-monitoring crosspoint value (selected input →
    /// selected output), adjusted by the front-panel wheel.
    mix_raw: u16,
    /// AN1/2 input-strip stereo link (0x17 wIdx=0x1000): true = linked
    /// (the device's default from TotalMix), false = split. Not
    /// persisted (a hardware state; the device keeps its own).
    input_link: bool,
}

// ── topology helpers ────────────────────────────────────────────────

/// The protocol source for a core input channel index. Stereo inputs
/// (AS/ADAT) map both core channels (L/R) to the same pair — the
/// protocol writes both pair registers together (TotalMix keeps the
/// pair balanced via pan).
fn input_source(idx: usize) -> Result<Source, Error> {
    Ok(Source::Input(match idx {
        0 => Input::An1,
        1 => Input::An2,
        2 => Input::An3,
        3 => Input::An4,
        4 | 5 => Input::As12,
        6 | 7 => Input::Adat34,
        8 | 9 => Input::Adat56,
        10 | 11 => Input::Adat78,
        _ => return Err(Error::InvalidChannel(format!("Input {idx}"))),
    }))
}

/// The protocol source for a core playback channel index (12 channels,
/// 6 stereo pairs). Both channels of a pair map to the same source.
fn playback_source(idx: usize) -> Result<Source, Error> {
    if idx >= 12 {
        return Err(Error::InvalidChannel(format!("Playback {idx}")));
    }
    Ok(Source::Playback(Playback(idx / 2 + 1)))
}

/// The protocol output for a core output channel index (6 stereo pairs).
fn output_for(idx: usize) -> Result<Output, Error> {
    Ok(match idx {
        0 => Output::An12,
        1 => Output::Ph34,
        2 => Output::As12,
        3 => Output::Adat34,
        4 => Output::Adat56,
        5 => Output::Adat78,
        _ => return Err(Error::InvalidChannel(format!("Output {idx}"))),
    })
}

// ── scale helpers (calibrated — cap_calib.pcap, 2026-08-22) ─────────

/// f32 volume (0.0-1.0, where 1.0 = 0 dB — the GUI's `20·log10(vol)`
/// model) → raw 16-bit crosspoint-fader value via the calibrated curve.
/// Linear volume (0..2) → raw fader code. Calibrated: 1.0 = 0 dB =
/// 0x16A0, 2.0 = +6 dB = 0x2D41 (the fader's top, like TotalMix),
/// 0.0 = -inf.
pub fn volume_to_raw(v: f32) -> u16 {
    let db = 20.0 * v.clamp(f32::EPSILON, 2.0).log10();
    fader_db_to_raw(db)
}

/// raw 16-bit crosspoint-fader value → f32 volume (0.0-1.0, 1.0 = 0 dB).
pub fn raw_to_volume(raw: u16) -> f32 {
    if raw <= FADER_MUTE_RAW || raw < FADER_CURVE[0].1 {
        return 0.0; // -inf (the GUI renders vol 0 as "-inf dB")
    }
    let db = fader_raw_to_db(raw);
    10f32.powf(db / 20.0)
}

/// dB of preamp gain → raw protocol value (calibrated: raw 0-20 ≈
/// 0-65 dB, 3.25 dB per raw step, rounded).  MIC inputs only — see
/// [`BabyfaceProUsb::gain_to_raw`] for the per-input dispatch.
pub fn gain_db_to_raw(db: f32) -> u8 {
    (db / GAIN_DB_PER_STEP).round().clamp(0.0, 20.0) as u8
}

/// raw protocol value → dB of preamp gain (calibrated inverse).
pub fn raw_to_gain_db(raw: u8) -> f32 {
    (raw as f32 * GAIN_DB_PER_STEP).min(65.0)
}

/// Decode the `0x17` status readback into the preamp state: byte 0
/// mirrors the state register (base 0x0C — bit0/1 = 48V AN1/2,
/// bit4/5 = PAD AN1/2), byte 2 = clock state (0x40 Internal / 0x80
/// optical no-lock). Verified 2026-08-23 (cap_padpan.pcap + live
/// readback tracks the front-panel P48 LEDs: 0x0C/0x0D/0x1D).
pub fn preamp_state_from_readback(st: [u8; 4]) -> (bool, bool, bool, bool) {
    (
        st[0] & 0x01 != 0,
        st[0] & 0x02 != 0,
        st[0] & 0x10 != 0,
        st[0] & 0x20 != 0,
    )
}

impl BabyfaceProUsb {
    /// Discover and open the Babyface Pro FS on the USB bus.
    pub fn open() -> Result<Self, Error> {
        let dev = BabyfaceUsb::open()?;
        let inputs = vec![
            InputChannel::new(0, "AN1", ChannelType::Mic, 6),
            InputChannel::new(1, "AN2", ChannelType::Mic, 6),
            InputChannel::new(2, "AN3", ChannelType::Instrument, 6),
            InputChannel::new(3, "AN4", ChannelType::Instrument, 6),
            InputChannel::new(4, "AS1", ChannelType::Line, 6),
            InputChannel::new(5, "AS2", ChannelType::Line, 6),
            InputChannel::new(6, "ADAT3", ChannelType::ADAT, 6),
            InputChannel::new(7, "ADAT4", ChannelType::ADAT, 6),
            InputChannel::new(8, "ADAT5", ChannelType::ADAT, 6),
            InputChannel::new(9, "ADAT6", ChannelType::ADAT, 6),
            InputChannel::new(10, "ADAT7", ChannelType::ADAT, 6),
            InputChannel::new(11, "ADAT8", ChannelType::ADAT, 6),
        ];
        let playbacks: Vec<PlaybackChannel> = (0..12)
            .map(|i| PlaybackChannel::new(i, &format!("PB{}", i + 1), 6))
            .collect();
        let outputs = vec![
            OutputChannel::new(0, "AN1/2"),
            OutputChannel::new(1, "PH3/4"),
            OutputChannel::new(2, "AS1/2"),
            OutputChannel::new(3, "ADAT3/4"),
            OutputChannel::new(4, "ADAT5/6"),
            OutputChannel::new(5, "ADAT7/8"),
        ];
        let settings = DeviceSettings {
            clock_source: "Internal".into(),
            // Fireface USB Settings dropdown: Internal / Optical In.
            clock_sources: vec!["Internal".into(), "Optical In".into()],
            spdif_optical: false,
            spdif_emphasis: false,
            spdif_professional: false,
            spdif_enabled: false,
            pitch_percent: 0.0,
            ms_proc: false,
            an12: false,
            dim: false,
            fx_send_db: None,
            width: 0.0,
            sample_rate: 48_000,
        };
        let mut dev = Self {
            dev,
            inputs,
            playbacks,
            outputs,
            settings,
            panel: PanelDriver::new(),
            mix_raw: 0,
            input_link: true, // the device's linked default (TotalMix)
        };
        // Gains start at 0 dB on the preamps (no readback exists for
        // them — TotalMix re-applies its own saved gains; we have the
        // shared auto.json). The UI model is in dB (0-65 Mic, 0-18
        // Instrument), converted to raw on write.
        for i in 0..4 {
            dev.inputs[i].gain = Some(0);
            dev.inputs[i].gain_max = Some(match dev.inputs[i].channel_type {
                ChannelType::Mic => GAIN_DB_MAX,
                ChannelType::Instrument => GAIN_DB_MAX_INSTR,
                _ => 0,
            });
        }
        // The preamp state (48V/PAD) IS readable: 0x17 byte 0 mirrors
        // the state register and the device persists it across power
        // cycles (verified 2026-08-23 — the init burst doesn't clear
        // it either). Sync the UI to the real hardware state instead of
        // assuming everything is off.
        if let Ok(st) = dev.dev.read_status(0x17) {
            let (ph1, ph2, pad1, pad2) = preamp_state_from_readback(st);
            dev.inputs[0].phantom = ph1;
            dev.inputs[1].phantom = ph2;
            dev.inputs[0].pad = pad1;
            dev.inputs[1].pad = pad2;
        }
        // Keep the audio session running (init + state-restore + trigger
        // + interrupt URBs + arm — see tuxmix-usb `start_streaming`).
        // Needed for the meters (computed from the IN stream) and,
        // later, for real audio I/O.
        dev.dev.start_streaming()?;
        Ok(dev)
    }

    /// The 48V/PAD bits only (no base/ref bits) composed from ALL
    /// inputs. Callers that write the preamp register decide which
    /// base/ref value goes on top (`PREAMP_BASE` for preamp writes;
    /// the ref-level states for `set_ref_level`).
    fn preamp_bits(&self) -> u16 {
        use tuxmix_usb::protocol::{PREAMP_48V_MIC1, PREAMP_PAD_BIT};
        let mut state = 0;
        for i in 0..4 {
            if self.inputs[i].phantom {
                state |= PREAMP_48V_MIC1 << i;
            }
            if self.inputs[i].pad {
                state |= PREAMP_PAD_BIT << i;
            }
        }
        state
    }

    /// Write the preamp STATE byte composed from ALL inputs (48V + PAD
    /// bits) + the commit — WITHOUT the gain writes, so toggling 48V/
    /// PAD on one mic never clobbers the other gains (they have no
    /// readback; we can't restore them). Verified on hardware: 48V
    /// engages with just `0x17 state 0x003F` + `0x21` (p48d_test.c).
    fn write_preamp_state(&mut self) -> Result<(), Error> {
        use tuxmix_usb::protocol::PREAMP_BASE;
        let state = self.preamp_bits() | PREAMP_BASE;
        let reqs = tuxmix_usb::protocol::set_preamp_state(state);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    /// Write ONE mic's gain register only (no state write) — the
    /// protocol allows a single 0x1A write (listentest.c does exactly
    /// this), so changing one gain never clobbers 48V/PAD or the other
    /// gains. The stored value is in dB; converted to the 5-bit raw
    /// here (per-input law).
    fn write_gain(&mut self, idx: usize) -> Result<(), Error> {
        let raw = Self::gain_to_raw(
            self.inputs[idx].channel_type,
            self.inputs[idx].gain.unwrap_or(0),
        );
        let mut cycle = 0u8;
        let reqs = tuxmix_usb::protocol::set_gain(idx, raw, &mut cycle);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    /// Per-input gain (dB) → raw protocol code.  TWO calibrated laws
    /// (cap_calib.pcap + cap_gain34.pcap, 2026-08-26): Mic AN1/2 =
    /// raw 0-20 = 0-65 dB (3.25 dB/step); Instrument AN3/4 = raw 0-18
    /// = 0-9 dB (0.5 dB/step — matches the kernel control 0-9 dB,
    /// raw = dB×2).
    fn gain_to_raw(ct: ChannelType, gain: u32) -> u8 {
        let g = gain as f32;
        match ct {
            ChannelType::Mic => gain_db_to_raw(g),
            ChannelType::Instrument => (g * 2.0).round().clamp(0.0, 18.0) as u8,
            _ => 0,
        }
    }

    /// Write the FULL preamp block (state byte composed from ALL inputs
    /// + all four gains) — for whole-scene application only, where the
    /// complete state is known.
    fn write_preamp_block(&mut self) -> Result<(), Error> {
        use tuxmix_usb::protocol::PREAMP_BASE;
        let state = self.preamp_bits() | PREAMP_BASE;
        let gain = [
            Self::gain_to_raw(
                self.inputs[0].channel_type,
                self.inputs[0].gain.unwrap_or(0),
            ),
            Self::gain_to_raw(
                self.inputs[1].channel_type,
                self.inputs[1].gain.unwrap_or(0),
            ),
            Self::gain_to_raw(
                self.inputs[2].channel_type,
                self.inputs[2].gain.unwrap_or(0),
            ),
            Self::gain_to_raw(
                self.inputs[3].channel_type,
                self.inputs[3].gain.unwrap_or(0),
            ),
        ];
        let mut reqs = tuxmix_usb::protocol::set_preamp_state(state);
        let mut cycle = 0u8;
        for (m, g) in gain.iter().enumerate() {
            reqs.extend(tuxmix_usb::protocol::set_gain(m, *g, &mut cycle));
        }
        self.dev.send_all(&reqs)?;
        Ok(())
    }
}

impl RmeDevice for BabyfaceProUsb {
    fn model_name(&self) -> &str {
        "Babyface Pro FS (USB)"
    }

    fn output_pair_count(&self) -> usize {
        self.outputs.len()
    }

    fn open() -> Result<Self, Error> {
        Self::open()
    }

    fn inputs(&self) -> &[InputChannel] {
        &self.inputs
    }

    fn inputs_mut(&mut self) -> &mut [InputChannel] {
        &mut self.inputs
    }

    fn playbacks(&self) -> &[PlaybackChannel] {
        &self.playbacks
    }

    fn playbacks_mut(&mut self) -> &mut [PlaybackChannel] {
        &mut self.playbacks
    }

    fn outputs(&self) -> &[OutputChannel] {
        &self.outputs
    }

    fn outputs_mut(&mut self) -> &mut [OutputChannel] {
        &mut self.outputs
    }

    fn settings(&self) -> &DeviceSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut DeviceSettings {
        &mut self.settings
    }

    fn set_volume(&mut self, channel: ChannelId, output: usize, volume: f32) -> Result<(), Error> {
        let out = output_for(output)?;
        let src = match channel {
            ChannelId::Input(i) => input_source(i)?,
            ChannelId::Playback(c) => playback_source(c)?,
            ChannelId::Output(o) => {
                self.outputs[o].volume = volume;
                // Output masters use their own (exponential) curve;
                // the 8-bit register is the REAL volume (0.5 dB/step,
                // 0xF3 = 0 dB). 2.0 = +6 dB, the master's top.
                let db = 20.0 * volume.clamp(f32::EPSILON, 2.0).log10();
                return Ok(self.dev.set_output_master(
                    out,
                    master_db_to_raw(db),
                    master_8bit(db),
                )?);
            }
        };
        let raw = volume_to_raw(volume);
        self.dev.set_volume(out, src, raw)?;
        // Keep the AN1/2 low-map mirror in sync (TotalMix writes both).
        if out == Output::An12 {
            self.dev.set_low_map_volume(src, raw)?;
        }
        // Update the local state.
        match channel {
            ChannelId::Input(i) => self.inputs[i].volumes[output] = volume,
            ChannelId::Playback(c) => self.playbacks[c].volumes[output] = volume,
            _ => {}
        }
        Ok(())
    }

    fn volume(&self, channel: ChannelId, output: usize) -> Result<f32, Error> {
        match channel {
            ChannelId::Input(i) => Ok(self.inputs[i].volumes[output]),
            ChannelId::Playback(c) => Ok(self.playbacks[c].volumes[output]),
            ChannelId::Output(o) => Ok(self.outputs[o].volume),
        }
    }

    fn set_pan(&mut self, channel: ChannelId, output: usize, pan: i8) -> Result<(), Error> {
        let out = output_for(output)?;
        let balance = (pan.clamp(-100, 100) as f32) / 100.0;
        let (src, fixed_is_left) = match channel {
            ChannelId::Input(i) => {
                // Mono inputs have no pan (observed: no writes emitted).
                if (0..4).contains(&i) {
                    return Ok(());
                }
                (input_source(i)?, i % 2 == 0)
            }
            ChannelId::Playback(c) => (playback_source(c)?, c % 2 == 0),
            ChannelId::Output(_) => return Ok(()),
        };
        let fixed_volume = match channel {
            ChannelId::Input(i) => self.inputs[i].volumes[output],
            ChannelId::Playback(c) => self.playbacks[c].volumes[output],
            _ => 0.0,
        };
        self.dev.set_balance(
            out,
            src,
            balance,
            volume_to_raw(fixed_volume),
            fixed_is_left,
        )?;
        match channel {
            ChannelId::Input(i) => self.inputs[i].pans[output] = pan,
            ChannelId::Playback(c) => self.playbacks[c].pans[output] = pan,
            _ => {}
        }
        Ok(())
    }

    fn pan(&self, channel: ChannelId, output: usize) -> Result<i8, Error> {
        match channel {
            ChannelId::Input(i) => Ok(self.inputs[i].pans[output]),
            ChannelId::Playback(c) => Ok(self.playbacks[c].pans[output]),
            ChannelId::Output(_) => Ok(0),
        }
    }

    fn set_mute(&mut self, channel: ChannelId, mute: bool) -> Result<(), Error> {
        match channel {
            ChannelId::Output(o) => {
                self.outputs[o].mute = mute;
                // Unmute restores the current volume (the 8-bit
                // register is the real output volume).
                let db = 20.0 * self.outputs[o].volume.clamp(f32::EPSILON, 2.0).log10();
                let reqs = tuxmix_usb::protocol::set_output_master_mute(
                    output_for(o)?,
                    mute,
                    master_db_to_raw(db),
                    master_8bit(db),
                );
                self.dev.send_all(&reqs).map_err(Into::into)
            }
            ChannelId::Input(i) => {
                self.inputs[i].mute = mute;
                // cap_mute2.pcap (2026-08-24): muting an input strip
                // zeroes its crosspoints — the low-map pair (0x1000 =
                // the linked -6 dB default on restore) + the standard
                // crosspoints into EVERY output (global mute). The
                // model keeps the unmuted volumes for the restore.
                let src = input_source(i)?;
                let low = if mute {
                    0
                } else {
                    volume_to_raw(self.inputs[i].volumes[0])
                };
                self.dev.set_low_map_volume(src, low)?;
                for o in 0..self.outputs.len() {
                    let raw = if mute {
                        0
                    } else {
                        volume_to_raw(self.inputs[i].volumes[o])
                    };
                    let out = output_for(o)?;
                    self.dev.set_volume(out, src, raw)?;
                }
                Ok(())
            }
            ChannelId::Playback(c) => {
                self.playbacks[c].mute = mute;
                // Same: zero/restore the strip's crosspoints into all
                // outputs (the capture: PB1-5's out0 pairs 0x0000 /
                // 0x2000 = the active marker).
                let src = Source::Playback(Playback(c / 2 + 1));
                for o in 0..self.outputs.len() {
                    let raw = if mute {
                        0
                    } else {
                        volume_to_raw(self.playbacks[c].volumes[o])
                    };
                    let out = output_for(o)?;
                    self.dev.set_volume(out, src, raw)?;
                }
                Ok(())
            }
        }
    }

    fn mute(&self, channel: ChannelId) -> Result<bool, Error> {
        match channel {
            ChannelId::Input(i) => Ok(self.inputs[i].mute),
            ChannelId::Playback(c) => Ok(self.playbacks[c].mute),
            ChannelId::Output(o) => Ok(self.outputs[o].mute),
        }
    }

    fn set_solo(&mut self, channel: ChannelId, solo: bool) -> Result<(), Error> {
        // cap_solo2.pcap (2026-08-24): solo = MUTE-THE-OTHERS — the
        // soloed strip's AN1/2 low map goes to the "active" value
        // (0x2000 playbacks / 0x1000 linked inputs) and every OTHER
        // strip's crosspoints go 0x0000 (in the capture, visible as
        // the low-map writes of the AN1/2 submix strips); un-solo
        // restores every strip from the model, respecting its own
        // mute button. Outputs have no solo in TotalMix.
        //
        // (The old "global solo registers 0x0004/0x001F/0x000C/0x0027"
        // were AS1/2's + PB1's LOW MAPS — writing them muted those two
        // strips on every solo toggle. Removed.)
        let (solo_i, solo_c) = match channel {
            ChannelId::Input(i) => (Some(i), None),
            ChannelId::Playback(c) => (None, Some(c)),
            ChannelId::Output(_) => return Ok(()),
        };
        // The soloed strip: set its flag and compute its low-map marker
        // (or restore it from the model on un-solo). The low map is the
        // AN1/2 submix mirror TotalMix keeps in sync.
        let (soloed, marker) = match channel {
            ChannelId::Input(i) => {
                let src = input_source(i)?;
                self.inputs[i].solo = solo;
                let raw = if solo {
                    if self.input_link {
                        0x1000
                    } else {
                        0x2000
                    }
                } else {
                    volume_to_raw(self.inputs[i].volumes[0])
                };
                (src, raw)
            }
            ChannelId::Playback(c) => {
                let src = playback_source(c)?;
                self.playbacks[c].solo = solo;
                let raw = if solo {
                    0x2000
                } else {
                    volume_to_raw(self.playbacks[c].volumes[0])
                };
                (src, raw)
            }
            ChannelId::Output(_) => unreachable!(),
        };
        self.dev.set_low_map_volume(soloed, marker)?;
        // The others: mute (solo ON) or restore (solo OFF) their low
        // maps and crosspoints into every output. Exclusive solo (the
        // TotalMix default): soloing a strip clears the other strips'
        // solo flags so a later un-solo restores them cleanly.
        for j in 0..self.inputs.len() {
            if solo_i == Some(j) {
                continue;
            }
            if solo {
                self.inputs[j].solo = false;
            }
            let ch = &self.inputs[j];
            let low = if solo || ch.mute {
                0
            } else {
                volume_to_raw(ch.volumes[0])
            };
            let src = input_source(j)?;
            self.dev.set_low_map_volume(src, low)?;
            for o in 0..self.outputs.len() {
                let raw = if solo || ch.mute {
                    0
                } else {
                    volume_to_raw(ch.volumes[o])
                };
                let out = output_for(o)?;
                self.dev.set_volume(out, src, raw)?;
            }
        }
        for j in 0..self.playbacks.len() {
            if solo_c == Some(j) {
                continue;
            }
            if solo {
                self.playbacks[j].solo = false;
            }
            let ch = &self.playbacks[j];
            let low = if solo || ch.mute {
                0
            } else {
                volume_to_raw(ch.volumes[0])
            };
            let src = playback_source(j)?;
            self.dev.set_low_map_volume(src, low)?;
            for o in 0..self.outputs.len() {
                let raw = if solo || ch.mute {
                    0
                } else {
                    volume_to_raw(ch.volumes[o])
                };
                let out = output_for(o)?;
                self.dev.set_volume(out, src, raw)?;
            }
        }
        Ok(())
    }

    fn solo(&self, channel: ChannelId) -> Result<bool, Error> {
        match channel {
            ChannelId::Input(i) => Ok(self.inputs[i].solo),
            ChannelId::Playback(c) => Ok(self.playbacks[c].solo),
            ChannelId::Output(o) => Ok(self.outputs[o].solo),
        }
    }

    fn set_phantom(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        if self.inputs[idx].channel_type != ChannelType::Mic {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no 48V phantom power"
            )));
        }
        self.inputs[idx].phantom = on;
        self.write_preamp_state()
    }

    fn set_pad(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        if self.inputs[idx].channel_type != ChannelType::Mic {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no pad switch"
            )));
        }
        self.inputs[idx].pad = on;
        self.write_preamp_state()
    }

    fn set_gain(&mut self, idx: usize, gain: u32) -> Result<(), Error> {
        if !matches!(
            self.inputs[idx].channel_type,
            ChannelType::Mic | ChannelType::Instrument
        ) {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no gain control"
            )));
        }
        // Gain is tracked in dB (0-65 Mic / 0-18 Instrument) — the raw
        // conversion happens in `write_gain`. 1 dB steps, like TotalMix.
        let max = self.inputs[idx].gain_max.unwrap_or(GAIN_DB_MAX);
        self.inputs[idx].gain = Some(gain.min(max));
        self.write_gain(idx)
    }

    fn set_sensitivity(&mut self, _idx: usize, _sensitivity: Sensitivity) -> Result<(), Error> {
        Err(Error::InvalidChannel(
            "Sensitivity is not mapped in the USB protocol yet".into(),
        ))
    }

    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), Error> {
        self.settings.pitch_percent = pitch_percent.clamp(-5.0, 5.0);
        let reqs = tuxmix_usb::protocol::set_pitch(pitch_percent.clamp(-5.0, 5.0));
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> Result<(), Error> {
        // Mid-session SET_INTERFACE(5, alt) + stream restart at the new
        // frame layout (validated on Linux, ratetest.c). Unsupported
        // rates error; the UI shows the supported list.
        self.dev.set_sample_rate(rate)?;
        self.settings.sample_rate = self.dev.sample_rate();
        Ok(())
    }

    // ── §9 controls (decoded on Windows 2026-08-23, see PROTOCOL.md) ──

    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), Error> {
        output_for(out)?; // validate the index
        self.outputs[out].loopback = on;
        // TotalMix ALWAYS writes the full 30-channel map on every toggle
        // (cap_loopback_off.pcap, 2026-08-23): 0x0001 on the active
        // channels, 0x0000 on the rest. A partial (2-channel-only) OFF
        // does NOT reliably disengage the loopback — the Linux probe
        // showed it needs the full 30-channel clear. Build the map from
        // the current state of ALL outputs so toggling one strip does
        // not clear another strip's loopback.
        let mut reqs = Vec::with_capacity(30);
        for c in 0..30u16 {
            let active =
                (c as usize / 2) < self.outputs.len() && self.outputs[c as usize / 2].loopback;
            reqs.push(tuxmix_usb::protocol::set_loopback(c, active));
        }
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_an12(&mut self, on: bool) -> Result<(), Error> {
        self.settings.an12 = on;
        // Compose with the stereo-link state: 0x1400 = linked + copy,
        // but a SPLIT pair must stay split (0x1000 alone, no 0x0400).
        let reqs = tuxmix_usb::protocol::set_input_link(self.input_link, on);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_input_link(&mut self, linked: bool) -> Result<(), Error> {
        self.input_link = linked;
        let reqs = tuxmix_usb::protocol::set_input_link(linked, self.settings.an12);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_trim(&mut self, idx: usize, db: f32) -> Result<(), Error> {
        let src = input_source(idx)?;
        // cap_trim2.pcap + cap_trim3/4.pcap (2026-08-24): the low map
        // = the trim ALONE on the master curve (0x2000 = 0 dB, 0 =
        // -inf); the standard map = fader × trim — the fader-curve
        // value of the SUMMED dB (regression on 188 labeled pairs:
        // standard_raw = 0x16A0 · 10^((fader_dB + trim_dB)/20), exact
        // to 3 decimals). The old ×27/256 placeholder is gone.
        let db = db.clamp(-65.0, 6.0);
        let trim_raw = master_db_to_raw(db);
        let fader_db = 20.0 * self.inputs[idx].volumes[0].clamp(f32::EPSILON, 2.0).log10();
        let standard_raw = fader_db_to_raw(fader_db + db);
        let reqs = tuxmix_usb::protocol::set_trim(src, trim_raw, standard_raw);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_ms_proc(&mut self, on: bool) -> Result<(), Error> {
        self.settings.ms_proc = on;
        // Engaged → mute the AN2 crosspoints; disengaged → restore the
        // current AN2→AN1/2 fader value (capture: 0x068E → 0x0000 →
        // 0x068E). The low map mirrors out0, so one value covers both.
        let value = if on {
            0x0000
        } else {
            volume_to_raw(self.inputs[1].volumes[0])
        };
        let reqs = tuxmix_usb::protocol::set_ms_proc(value);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_phase(&mut self, idx: usize, invert: bool) -> Result<(), Error> {
        let src = input_source(idx)?;
        self.inputs[idx].phase = invert;
        // Negate (bitwise-NOT) the current fader value of EVERY output
        // the input routes into; out0 also gets the low-map mirror
        // (matching the capture: 0x0EA0 → 0xF15F).
        for o in 0..self.outputs.len() {
            let raw = volume_to_raw(self.inputs[idx].volumes[o]);
            let value = if invert { !raw } else { raw };
            if o == 0 {
                let reqs = tuxmix_usb::protocol::set_phase(src, value);
                self.dev.send_all(&reqs)?;
            } else {
                let out = output_for(o)?;
                self.dev
                    .send_all(&[tuxmix_usb::protocol::VendorRequest::new(
                        0x12,
                        value,
                        tuxmix_usb::map::crosspoint_l(out, src) as u16,
                    )])?;
            }
        }
        Ok(())
    }

    fn set_fx_send(&mut self, db: f32) -> Result<(), Error> {
        let db = db.clamp(-65.0, 0.0);
        self.settings.fx_send_db = Some(db);
        // cap_fx3.pcap (2026-08-24 re-fit): the send follows the
        // CALIBRATED CROSSPOINT FADER curve at 0.5-dB steps in the
        // -62..-24 dB region (0x0003=-62, 0x000B=-54, 0x001D=-46,
        // 0x0029=-43, 0x0122=-26, 0x016D=-24 = FADER_CURVE exactly),
        // but the slider TOP is **0x1000 = 0 dB display** — NOT the
        // fader's 0 dB (0x16A0) — so the raw is clamped to 0x1000
        // (writes above it would exceed the send's register max).
        // Slider bottom (-inf) = 0x0000. Top region + 0.5-dB grid:
        // a slow labeled sweep would pin them exactly.
        let raw = if db <= -65.0 {
            0
        } else {
            fader_db_to_raw(db).min(0x1000)
        };
        let reqs = tuxmix_usb::protocol::set_fx_send(raw);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_stereo_split(&mut self, pb: usize, split: bool) -> Result<(), Error> {
        if pb >= self.playbacks.len() {
            return Err(Error::InvalidChannel(format!("Playback {pb}")));
        }
        // The two mono channels of a strip share the split state.
        self.playbacks[pb].split = split;
        self.playbacks[pb ^ 1].split = split;
        let reqs = tuxmix_usb::protocol::set_stereo_split(Playback(pb / 2 + 1), split);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_width(&mut self, width: f32) -> Result<(), Error> {
        self.settings.width = width.clamp(-1.0, 1.0);
        let reqs = tuxmix_usb::protocol::set_width(width);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_ref_level(&mut self, idx: usize, raw: u16) -> Result<(), Error> {
        if self.inputs[idx].channel_type != ChannelType::Instrument {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no ref-level switch"
            )));
        }
        self.inputs[idx].ref_level = raw;
        // LABELED pairs (cap_reflevel2.pcap, 2026-08-24 — started at
        // +4dBu = 0x0F, clicks -10dBV/Boost/+4dBu): the 0x21 carries
        // part of the code (NOT always the 0x0000 commit).
        let (state, commit) = match raw {
            REF_MINUS_10DBV => (0x0003, 0x0000),
            REF_BOOST => (0x0003, 0x0003),
            _ => (0x000F, 0x0000), // +4dBu (also the 0/unset fallback)
        };
        let reqs = tuxmix_usb::protocol::set_ref_level(state, commit);
        self.dev.send_all(&reqs)?;
        Ok(())
    }

    fn set_spdif_enabled(&mut self, _enabled: bool) -> Result<(), Error> {
        Err(Error::InvalidChannel(
            "SPDIF control is not mapped in the USB protocol yet".into(),
        ))
    }

    fn set_spdif_emphasis(&mut self, _enabled: bool) -> Result<(), Error> {
        Err(Error::InvalidChannel(
            "SPDIF control is not mapped in the USB protocol yet".into(),
        ))
    }

    fn set_spdif_professional(&mut self, _enabled: bool) -> Result<(), Error> {
        Err(Error::InvalidChannel(
            "SPDIF control is not mapped in the USB protocol yet".into(),
        ))
    }

    fn set_clock_source(&mut self, source: &str) -> Result<(), Error> {
        if !self.settings.clock_sources.iter().any(|s| s == source) {
            return Err(Error::InvalidChannel(format!(
                "Unknown clock source: {}",
                source
            )));
        }
        // The keepalive 0x10 0x05CF word bit 2 = clock Optical (cap_clk:
        // the 0x17 readback byte 2 flips 0x40 → 0x80 no-lock).
        self.dev.set_clock_optical(source == "Optical In")?;
        self.settings.clock_source = source.to_string();
        Ok(())
    }

    fn capture_scene(&self) -> Scene {
        Scene {
            name: "capture".into(),
            model: self.model_name().into(),
            inputs: self.inputs.clone(),
            playbacks: self.playbacks.clone(),
            outputs: self.outputs.clone(),
            settings: self.settings.clone(),
        }
    }

    fn apply_scene(&mut self, scene: &Scene) -> Result<(), Error> {
        scene.check_compatible(self.model_name())?;
        // Write every crosspoint (standard + low map), then the masters
        // and the preamp block — mirroring a TotalMix scene load.
        for (i, inp) in scene.inputs.iter().enumerate() {
            let src = input_source(i)?;
            for (o, v) in inp.volumes.iter().enumerate() {
                let out = output_for(o)?;
                let raw = volume_to_raw(*v);
                self.dev.set_volume(out, src, raw)?;
                if out == Output::An12 {
                    self.dev.set_low_map_volume(src, raw)?;
                }
            }
        }
        for (c, pb) in scene.playbacks.iter().enumerate() {
            let src = playback_source(c)?;
            for (o, v) in pb.volumes.iter().enumerate() {
                let out = output_for(o)?;
                let raw = volume_to_raw(*v);
                self.dev.set_volume(out, src, raw)?;
                if out == Output::An12 {
                    self.dev.set_low_map_volume(src, raw)?;
                }
            }
        }
        for (o, out) in scene.outputs.iter().enumerate() {
            let out_p = output_for(o)?;
            // Output masters use their own (exponential) curve; the
            // 8-bit register is the REAL volume.
            let db = 20.0 * out.volume.clamp(f32::EPSILON, 1.0).log10();
            let raw = master_db_to_raw(db);
            self.dev.set_output_master(out_p, raw, master_8bit(db))?;
            if out.mute {
                self.dev.set_output_master_mute(out_p, true, 0, 0)?;
            }
        }
        // Adopt the new state.
        self.inputs = scene.inputs.clone();
        self.playbacks = scene.playbacks.clone();
        self.outputs = scene.outputs.clone();
        self.settings = scene.settings.clone();
        // `gain_max` is a hardware property — re-derive it so a scene
        // saved by an older/gain-raw build (or a different model) can't
        // override the UI's gain range with a stale value.
        for i in 0..self.inputs.len().min(4) {
            self.inputs[i].gain_max = Some(match self.inputs[i].channel_type {
                ChannelType::Mic => GAIN_DB_MAX,
                ChannelType::Instrument => GAIN_DB_MAX_INSTR,
                _ => 0,
            });
        }
        self.write_preamp_block()?;
        // §9 mixer states (decoded on Windows 2026-08-23) — applied
        // AFTER the preamp block so set_ref_level composes on top of the
        // just-written 48V/PAD bits. Loopback is written unconditionally
        // (its flag map is cleared by the stream init); the others only
        // when set, since their writes would otherwise clobber fader
        // values or flip unset states. State is collected first (the
        // setters borrow self mutably).
        let loopback: Vec<bool> = self.outputs.iter().map(|o| o.loopback).collect();
        for (o, on) in loopback.into_iter().enumerate() {
            self.set_loopback(o, on)?;
        }
        if self.settings.an12 {
            self.set_an12(true)?;
        }
        if self.settings.ms_proc {
            self.set_ms_proc(true)?;
        }
        if let Some(db) = self.settings.fx_send_db {
            self.set_fx_send(db)?;
        }
        if self.settings.width != 0.0 {
            self.set_width(self.settings.width)?;
        }
        let inp_states: Vec<(usize, bool, u16)> = self
            .inputs
            .iter()
            .enumerate()
            .map(|(i, inp)| (i, inp.phase, inp.ref_level))
            .collect();
        for (i, phase, ref_level) in inp_states {
            if phase {
                self.set_phase(i, true)?;
            }
            if ref_level != 0 {
                self.set_ref_level(i, ref_level)?;
            }
        }
        let splits: Vec<usize> = self
            .playbacks
            .iter()
            .enumerate()
            .filter(|(c, pb)| c % 2 == 0 && pb.split)
            .map(|(c, _)| c)
            .collect();
        for c in splits {
            self.set_stereo_split(c, true)?;
        }
        Ok(())
    }

    fn poll_events(&mut self) -> Result<(), Error> {
        // No ALSA events in the USB backend; keep the interrupt URBs
        // moving so the stream and the meters stay live.
        self.dev.pump(std::time::Duration::from_millis(20));
        self.panel_tick()
    }

    fn meters(&self) -> Option<Vec<f32>> {
        self.dev.input_peaks().map(|p| p.to_vec())
    }
}

impl BabyfaceProUsb {
    /// Poll the front-panel state (`0x17` readback) and act on the
    /// events like TotalMix does — the panel is HOST-DRIVEN (see
    /// `panel.rs`). Called from `poll_events` at the UI tick rate.
    fn panel_tick(&mut self) -> Result<(), Error> {
        let st = match self.dev.read_status(0x17) {
            Ok(s) => s,
            Err(_) => return Ok(()), // transient; the next poll retries
        };
        let ps = PanelState::decode(st);
        let events = self.panel.feed(ps);
        for ev in events {
            match ev {
                PanelEvent::MixPressed => {
                    // TotalMix ack for the MIX press (cap_mix.pcap).
                    // After this the device flips byte2 into fader mode
                    // (0x00+n wheel counter) and the wheel adjusts the
                    // monitoring level.
                    self.dev.send(&tuxmix_usb::protocol::VendorRequest::new(
                        0x17, 0x8480, 0x8C80,
                    ))?;
                    // Start the monitoring level at the selected
                    // input→output crosspoint's current value so the
                    // first wheel click doesn't jump from -inf. The
                    // SELECT-chosen channel is the reference (LEFT if
                    // nothing is selected).
                    let (_, o) = Self::mix_crosspoint(self.panel.in_sel, self.panel.out_sel);
                    let sel = Self::select_targets(self.panel.select, self.panel.in_sel);
                    let i = if self.panel.in_sel == 2 {
                        4 // Opt: the AS1/2 pair
                    } else if sel.is_empty() {
                        0 // nothing selected; the wheel won't move anyway
                    } else {
                        sel[0]
                    };
                    self.mix_raw = volume_to_raw(self.inputs[i].volumes[o]);
                }
                PanelEvent::MixReleased => {
                    self.dev.send(&tuxmix_usb::protocol::VendorRequest::new(
                        0x17, 0x0400, 0x8000,
                    ))?;
                    self.dev.send(&tuxmix_usb::protocol::VendorRequest::new(
                        0x17, 0x0400, 0x8080,
                    ))?;
                }
                PanelEvent::Wheel { delta } => self.panel_wheel(ps, delta)?,
                PanelEvent::DimPressed => {
                    // DIM needs the Main-Out mapping; logged only.
                }
                PanelEvent::Button { code } => {
                    if code == 0x42 {
                        // SET = 48V phantom toggle on the IN-selected
                        // mic(s). The hardware only does this in
                        // standalone mode (online, TotalMix never
                        // reacts), but TuxMix IS the host: we write
                        // the preamp state and the P48 LEDs follow it
                        // (verified on hardware).
                        self.panel_set_phantom(ps)?;
                    }
                    // Other flashes (IN/OUT/SELECT) are logged by the
                    // driver but act only through the state they set.
                }
            }
        }
        Ok(())
    }

    /// The model input indices the SELECT state targets on the
    /// IN-selected pair (manual §5.1/§5.3: SELECT steps left/right/both,
    /// then the wheel changes the gain or the monitoring level).
    /// `in_sel` 0 = Ch1/2 (AN1/AN2), 1 = Ch3/4 (AN3/AN4); Opt (2) has
    /// no preamp and one shared crosspoint pair. `SelectState::None`
    /// (deselected) targets NOTHING — with no channel selected the wheel
    /// must not move any gain (hardware-verified 2026-08-24).
    fn select_targets(select: SelectState, in_sel: usize) -> &'static [usize] {
        match (select, in_sel) {
            (SelectState::Left, 0) => &[0],
            (SelectState::Right, 0) => &[1],
            (SelectState::Both, 0) => &[0, 1],
            (SelectState::Left, 1) => &[2],
            (SelectState::Right, 1) => &[3],
            (SelectState::Both, 1) => &[2, 3],
            _ => &[], // Opt (2) / SelectState::None (nothing selected)
        }
    }

    /// The mic indices a SET press affects for the current panel state
    /// (empty = no phantom target — Opt/Ch3/4 have no phantom, and
    /// SELECT None means no channel is chosen). Pure, for tests.
    fn set_phantom_targets(ps: PanelState, select: SelectState) -> &'static [usize] {
        if ps.mix_mode() || ps.out_mode() || ps.in_sel() != 0 {
            return &[];
        }
        match select {
            SelectState::Left => &[0],
            SelectState::Right => &[1],
            SelectState::Both => &[0, 1],
            SelectState::None => &[],
        }
    }

    /// Toggle 48V on the SELECT-chosen mic(s) of the IN-selected pair
    /// (host-side emulation of the standalone SET function).
    fn panel_set_phantom(&mut self, ps: PanelState) -> Result<(), Error> {
        let mics = Self::set_phantom_targets(ps, self.panel.select);
        if mics.is_empty() {
            return Ok(());
        }
        let on = !self.inputs[mics[0]].phantom;
        for &m in mics {
            self.inputs[m].phantom = on;
        }
        self.write_preamp_state()
    }

    /// The current front-panel state, for the UI to follow: (MIX
    /// engaged, IN selection, OUT selection). The GUI/TUI sync their
    /// selected submix to `out_sel` like TotalMix highlights the
    /// panel's current submix.
    pub fn panel_selection(&self) -> (bool, usize, usize) {
        (self.panel.mix_mode, self.panel.in_sel, self.panel.out_sel)
    }

    /// The model input/output indices of the MIX-monitoring crosspoint
    /// for a given panel IN/OUT selection (shared by the MixPressed
    /// init and the wheel).
    fn mix_crosspoint(in_sel: usize, out_sel: usize) -> (usize, usize) {
        let i = match in_sel {
            1 => 2, // AN3 (Ch3/4 pair)
            2 => 4, // AS1 (Opt = the optical input)
            _ => 0, // AN1
        };
        let o = match out_sel {
            1 => 1, // Phones
            2 => 5, // ADAT7/8 (Opt = the optical output)
            _ => 0, // AN1/2
        };
        (i, o)
    }

    /// One wheel click (or a small run) in the active panel mode.
    fn panel_wheel(&mut self, ps: PanelState, delta: i8) -> Result<(), Error> {
        use tuxmix_usb::map::Output as MapOutput;
        if self.panel.mix_mode {
            // Monitoring level: the SELECT-chosen channel(s) of the
            // selected input → selected output crosspoint, ±0.5 dB per
            // click on the fader curve (cap_mix.pcap wrote 0x12
            // 0x0034/0x004E — the standard map only, no low-map mirror).
            // The mode + selection come from the LATCHED panel state:
            // byte2 is the 0x00+n fader counter here, so the IN
            // selection is not readable from the readback.
            let out = match self.panel.out_sel {
                1 => MapOutput::Ph34,
                2 => MapOutput::Adat78, // Opt = the optical output
                _ => MapOutput::An12,
            };
            let db = fader_raw_to_db(self.mix_raw);
            let new_db = (db + 0.5 * delta as f32).clamp(-65.0, 6.0);
            self.mix_raw = if new_db <= -65.0 {
                0
            } else {
                fader_db_to_raw(new_db)
            };
            let mut targets: Vec<usize> = match self.panel.in_sel {
                1 => Self::select_targets(self.panel.select, 1).to_vec(),
                2 => vec![4], // Opt: the AS1/2 pair (one crosspoint)
                _ => Self::select_targets(self.panel.select, 0).to_vec(),
            };
            // Mirror the change into the local model so the UI fader
            // follows the wheel (the raw USB write alone would leave the
            // GUI showing the stale value).
            let o = match self.panel.out_sel {
                1 => 1,
                2 => 5,
                _ => 0,
            };
            for i in targets.drain(..) {
                self.inputs[i].volumes[o] = raw_to_volume(self.mix_raw);
                let src = input_source(i)?;
                self.dev.set_volume(out, src, self.mix_raw)?;
            }
        } else if ps.out_mode() {
            // Output level: the selected OUT's master fader, ±0.5 dB
            // per click (cap_set2.pcap: the wheel writes only the
            // 16-bit master 0x03E0+2·out, no 8-bit companion mid-run).
            let o = match ps.out_sel() {
                1 => 1, // Phones
                2 => 5, // Opt = ADAT7/8
                _ => 0,
            };
            let db = 20.0 * self.outputs[o].volume.clamp(f32::EPSILON, 2.0).log10();
            let new_db = (db + 0.5 * delta as f32).clamp(-65.0, 6.0);
            let raw = master_db_to_raw(new_db);
            self.outputs[o].volume = 10f32.powf(new_db / 20.0);
            self.dev
                .set_output_master(output_for(o)?, raw, master_8bit(new_db))?;
        } else {
            // Gain mode: the SELECT-chosen channel(s) of the
            // IN-selected pair, ±1 dB per click (manual §5.1: SELECT
            // steps left/right/both, then the wheel changes the gain).
            // Opt has no preamp. The write targets the PANEL's "ADC
            // gain" registers (0x1A 0x000A+mic — cap_select.pcap
            // 2026-08-24), NOT the GUI's 0x0000+mic; the model tracks
            // the dB either way (relationship of the two families:
            // Linux check).
            if ps.in_sel() != 2 {
                let mics = Self::select_targets(self.panel.select, ps.in_sel());
                for &mic in mics {
                    let g = self.inputs[mic].gain.unwrap_or(0) as f32;
                    let max = self.inputs[mic].gain_max.unwrap_or(GAIN_DB_MAX) as f32;
                    let new_g = (g + 1.0 * delta as f32).clamp(0.0, max);
                    self.inputs[mic].gain = Some(new_g as u32);
                    let raw = Self::gain_to_raw(self.inputs[mic].channel_type, new_g as u32);
                    let reqs = tuxmix_usb::protocol::set_panel_gain(mic, raw);
                    self.dev.send_all(&reqs)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scale_calibrated() {
        // Calibrated from cap_calib.pcap: 1.0 = 0 dB = 0x16A0, 0.0 = -inf.
        assert_eq!(volume_to_raw(0.0), 0x0000);
        assert_eq!(volume_to_raw(1.0), 0x16A0);
        // 2.0 = +6 dB = 0x2D41 (fader top, TotalMix range).
        assert_eq!(volume_to_raw(2.0), 0x2D41);
        assert!((raw_to_volume(0x2D41) - 2.0).abs() < 0.05);
        // -6 dB ≈ 0x0B51 (interpolated between the table's -7/-6 points).
        assert!((volume_to_raw(0.5) as i32 - 0x0B51).abs() <= 1);
        assert!((raw_to_volume(0x16A0) - 1.0).abs() < 1e-3);
        // Mute raw is -inf.
        assert!(raw_to_volume(0x0000) < 1e-6);
        // The calibrated -20 dB anchor (scene-load cross-check).
        assert_eq!(fader_db_to_raw(-20.0), 0x0243);
        assert!((fader_raw_to_db(0x0243) + 20.0).abs() < 0.1);
        // Top: +6 dB = 0x2D41.
        assert_eq!(fader_db_to_raw(6.0), 0x2D41);
        // Master curve: 0 dB = 0x2000, +6 = 0x4000.
        assert_eq!(master_db_to_raw(0.0), 0x2000);
        assert_eq!(master_db_to_raw(6.0), 0x4000);
        assert_eq!(master_db_to_raw(-65.0), 0x0000);
    }

    #[test]
    fn gain_scale_calibrated() {
        // Calibrated: raw 0-20 ≈ 0-65 dB (3.25 dB/step).
        assert_eq!(gain_db_to_raw(0.0), 0);
        assert_eq!(gain_db_to_raw(65.0), 20);
        assert_eq!(gain_db_to_raw(35.0), 11); // 35/3.25 ≈ 10.8
        assert!((raw_to_gain_db(20) - 65.0).abs() < 1e-3);
        assert!((raw_to_gain_db(17) - 55.25).abs() < 1e-3);
    }

    #[test]
    fn instr_gain_scale_calibrated() {
        // Instrument AN3/4 (cap_gain34.pcap, 2026-08-26): 0-9 dB,
        // 0.5 dB/step = raw 0-18 (raw = dB×2, matches the kernel
        // control 0-9 dB).
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Instrument, 0), 0);
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Instrument, 4), 8);
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Instrument, 9), 18);
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Instrument, 99), 18); // clamped
                                                                                  // The mic law is untouched: 0-65 dB, raw 0-20.
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Mic, 0), 0);
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Mic, 65), 20);
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Mic, 35), 11); // 35/3.25 ≈ 10.8
                                                                           // Line/SPDIF/ADAT inputs have no preamp gain.
        assert_eq!(BabyfaceProUsb::gain_to_raw(ChannelType::Line, 40), 0);
    }

    #[test]
    fn preamp_readback_decodes_state_byte() {
        // 0x17 readback byte 0 mirrors the preamp state (0x0C base).
        assert_eq!(
            preamp_state_from_readback([0x0C, 0x01, 0x40, 0x40]),
            (false, false, false, false)
        );
        // 48V AN1 on.
        assert_eq!(
            preamp_state_from_readback([0x0D, 0x01, 0x40, 0x40]),
            (true, false, false, false)
        );
        // 48V AN1 + PAD AN1 (cap_padpan.pcap).
        assert_eq!(
            preamp_state_from_readback([0x1D, 0x01, 0x40, 0x40]),
            (true, false, true, false)
        );
        // PAD AN2 only (bit 0x20) would read 0x2C.
        assert_eq!(
            preamp_state_from_readback([0x2C, 0x01, 0x40, 0x40]),
            (false, false, false, true)
        );
    }

    #[test]
    fn set_phantom_targets_follow_in_and_select() {
        use crate::panel::{PanelState, SelectState};
        // Ch1/2 selected (byte2 0x4x) + SELECT = Left -> AN1 only.
        let ps = PanelState::decode([0x0C, 0x05, 0x4A, 0x40]);
        assert_eq!(
            BabyfaceProUsb::set_phantom_targets(ps, SelectState::Left),
            &[0]
        );
        assert_eq!(
            BabyfaceProUsb::set_phantom_targets(ps, SelectState::Right),
            &[1]
        );
        assert_eq!(
            BabyfaceProUsb::set_phantom_targets(ps, SelectState::Both),
            &[0, 1]
        );
        // SELECT = None -> no channel -> nothing.
        assert!(BabyfaceProUsb::set_phantom_targets(ps, SelectState::None).is_empty());
        // Ch3/4 (0x5x) and Opt (0x6x) have no phantom on this unit.
        let ps34 = PanelState::decode([0x0C, 0x05, 0x5A, 0x40]);
        assert!(BabyfaceProUsb::set_phantom_targets(ps34, SelectState::Both).is_empty());
        let psopt = PanelState::decode([0x0C, 0x05, 0x6A, 0x40]);
        assert!(BabyfaceProUsb::set_phantom_targets(psopt, SelectState::Both).is_empty());
        // OUT and MIX modes are not a 48V context.
        let psout = PanelState::decode([0x0C, 0x05, 0x8A, 0x40]);
        assert!(BabyfaceProUsb::set_phantom_targets(psout, SelectState::Both).is_empty());
        let psmix = PanelState::decode([0x8C, 0x85, 0x0A, 0x44]);
        assert!(BabyfaceProUsb::set_phantom_targets(psmix, SelectState::Both).is_empty());
    }

    #[test]
    fn select_targets_follow_select_and_in_pair() {
        use crate::panel::SelectState;
        // Ch1/2 (in_sel 0): Left -> AN1, Right -> AN2, Both -> both.
        assert_eq!(BabyfaceProUsb::select_targets(SelectState::Left, 0), &[0]);
        assert_eq!(BabyfaceProUsb::select_targets(SelectState::Right, 0), &[1]);
        assert_eq!(
            BabyfaceProUsb::select_targets(SelectState::Both, 0),
            &[0, 1]
        );
        // SELECT None (deselected): nothing moves (hardware-verified).
        assert!(BabyfaceProUsb::select_targets(SelectState::None, 0).is_empty());
        assert!(BabyfaceProUsb::select_targets(SelectState::None, 1).is_empty());
        // Ch3/4 (in_sel 1): Left -> AN3, Right -> AN4.
        assert_eq!(BabyfaceProUsb::select_targets(SelectState::Left, 1), &[2]);
        assert_eq!(BabyfaceProUsb::select_targets(SelectState::Right, 1), &[3]);
        assert_eq!(
            BabyfaceProUsb::select_targets(SelectState::Both, 1),
            &[2, 3]
        );
        // Opt (in_sel 2): no preamp targets.
        assert!(BabyfaceProUsb::select_targets(SelectState::Both, 2).is_empty());
    }

    #[test]
    fn input_topology_maps_to_protocol_sources() {
        assert_eq!(input_source(0).unwrap(), Source::Input(Input::An1));
        assert_eq!(input_source(3).unwrap(), Source::Input(Input::An4));
        // Both AS channels map to the AS1/2 pair.
        assert_eq!(input_source(4).unwrap(), Source::Input(Input::As12));
        assert_eq!(input_source(5).unwrap(), Source::Input(Input::As12));
        assert_eq!(input_source(6).unwrap(), Source::Input(Input::Adat34));
        assert_eq!(input_source(11).unwrap(), Source::Input(Input::Adat78));
        assert!(input_source(12).is_err());
    }

    #[test]
    fn playback_topology_maps_to_pairs() {
        // 12 core playbacks -> 6 stereo pairs (protocol indices 12-23).
        assert_eq!(playback_source(0).unwrap(), Source::Playback(Playback(1)));
        assert_eq!(playback_source(1).unwrap(), Source::Playback(Playback(1)));
        assert_eq!(playback_source(10).unwrap(), Source::Playback(Playback(6)));
        assert_eq!(playback_source(11).unwrap(), Source::Playback(Playback(6)));
        assert!(playback_source(12).is_err());
    }

    #[test]
    fn output_topology_maps_to_strip_order() {
        assert_eq!(output_for(0).unwrap(), Output::An12);
        assert_eq!(output_for(1).unwrap(), Output::Ph34);
        assert_eq!(output_for(2).unwrap(), Output::As12);
        assert_eq!(output_for(5).unwrap(), Output::Adat78);
        assert!(output_for(6).is_err());
    }

    #[test]
    fn open_fails_without_hardware() {
        // Only meaningful on a machine with no accessible Babyface: with
        // the udev rule the test user can open the real device (and
        // dropping it mid-stream segfaults), so skip when it's present.
        let present = std::fs::read_dir("/sys/bus/usb/devices")
            .map(|rd| {
                rd.flatten().any(|e| {
                    let p = e.path();
                    let vid = std::fs::read_to_string(p.join("idVendor")).unwrap_or_default();
                    let pid = std::fs::read_to_string(p.join("idProduct")).unwrap_or_default();
                    vid.trim() == "2a39" && pid.trim() == "3fc0"
                })
            })
            .unwrap_or(false);
        if present {
            return;
        }
        assert!(BabyfaceProUsb::open().is_err());
    }
}
