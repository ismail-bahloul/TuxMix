//! Babyface Pro (FS) implementation of the [`RmeDevice`] trait.

use alsa::mixer::{Selem, SelemChannelId};
use log::info;

use crate::channel::OutputChannel;
use crate::channel::*;
use crate::curves::{fader_db_to_raw, fader_raw_to_db};
use crate::device::{DeviceSettings, RmeDevice};
use crate::error::Error;
use crate::mixer::AlsaMixer;
use crate::profile::DeviceProfile;
use crate::profiles::babyface_pro::PROFILE;
use crate::scene::Scene;

// ── Helpers ────────────────────────────────────────────────────
//
// The ALSA control names/indices below were cross-checked 2026-08-28
// against the real kernel driver (`babyface-pro-linux/tools/kernel/
// mixer.c`) and live `amixer -c FS scontents` output on real hardware
// — not guessed. The previous grammar this file used
// (`"<Type>-<Name>-<Output>"`, `" 48V"`/`" PAD"`/`" Sens."` suffixes)
// never matched anything the kernel driver actually creates, so every
// crosspoint/phantom/pad/gain read-write was a silent no-op.

/// One entry per `mixer.c`'s `bf_sources[14]` crosspoint source, in the
/// same order (source index = position in this array = `src` in the
/// kernel's `index = out*14 + src`). The first 4 (AN1-AN4) are true
/// mono sources: their control's front-left/front-right ALSA channels
/// write the source's level into the output pair's left/right side, so
/// TotalMix-style volume+pan applies (see `input_crosspoint_slot`). The
/// other 10 are hardware-linked stereo pairs (AS1/2, the ADAT pairs,
/// PB1-PB6): front-left/front-right are each physical channel's OWN
/// level into the output pair, with no cross-routing — there is no
/// ALSA-level way to route e.g. AS1 into an output's right side, so pan
/// does not apply to those.
const BF_SOURCES: [&str; 14] = [
    "AN1", "AN2", "AN3", "AN4", "AS1/2", "ADAT3/4", "ADAT5/6", "ADAT7/8", "PB1", "PB2", "PB3",
    "PB4", "PB5", "PB6",
];

/// `mixer.c`'s `out_names[6]` — the master-volume/mute ALSA control
/// name per output pair, in `self.profile.outputs` order (both use the
/// canonical AN1/2, PH3/4, ... order — hardware-verified in `mixer.c`
/// not to need reordering).
const PAIR_LABELS: [&str; 6] = ["AN1/2", "PH3/4", "AS1/2", "ADAT3/4", "ADAT5/6", "ADAT7/8"];

/// Maps a `PROFILE.inputs` index to its crosspoint source: which of
/// `BF_SOURCES` it is, and — for the non-mono (linked-pair) sources —
/// which single ALSA channel (front-left/front-right) is *this*
/// input's own level, since the pair's other channel belongs to the
/// sibling input (e.g. `AS1` is `BF_SOURCES[4]` front-left, `AS2` is
/// the same source's front-right).
fn input_crosspoint_slot(idx: usize) -> Option<(usize, SelemChannelId, bool)> {
    use SelemChannelId::{FrontLeft as L, FrontRight as R};
    Some(match idx {
        0 => (0, L, true),  // AN1 (Mic 1)
        1 => (1, L, true),  // AN2 (Mic 2)
        2 => (2, L, true),  // IN3 -> hardware source "AN3"
        3 => (3, L, true),  // IN4 -> hardware source "AN4"
        4 => (4, L, false), // AS1
        5 => (4, R, false), // AS2
        6 => (5, L, false), // ADAT3
        7 => (5, R, false), // ADAT4
        8 => (6, L, false), // ADAT5
        9 => (6, R, false), // ADAT6
        10 => (7, L, false), // ADAT7
        11 => (7, R, false), // ADAT8
        _ => return None,
    })
}

/// Same idea for `PROFILE.outputs`-derived playback channels: each of
/// the 6 `PBn` sources is one playback pair (`PCM <left>`/`PCM
/// <right>`), always linked (no mono PB source exists).
fn playback_crosspoint_slot(idx: usize) -> Option<(usize, SelemChannelId)> {
    if idx >= 12 {
        return None;
    }
    let src = 8 + idx / 2;
    let ch = if idx % 2 == 0 {
        SelemChannelId::FrontLeft
    } else {
        SelemChannelId::FrontRight
    };
    Some((src, ch))
}

/// EQ strips exist only on the 4 analog inputs (`eq.c`'s `names[4] =
/// {"AN1","AN2","AN3","AN4"}`), which happen to be exactly
/// `BF_SOURCES[0..4]`.
fn eq_strip_name(idx: usize) -> Option<&'static str> {
    if idx < 4 {
        Some(BF_SOURCES[idx])
    } else {
        None
    }
}

/// `eq.c`'s `bf_eq_type_texts` enum-item order: Off/Bell/Low Shelf/High Shelf.
fn eq_band_type_to_enum(t: EqBandType) -> u32 {
    match t {
        EqBandType::Off => 0,
        EqBandType::Bell => 1,
        EqBandType::LowShelf => 2,
        EqBandType::HighShelf => 3,
    }
}

fn eq_band_type_from_enum(v: u32) -> EqBandType {
    match v {
        1 => EqBandType::Bell,
        2 => EqBandType::LowShelf,
        3 => EqBandType::HighShelf,
        _ => EqBandType::Off,
    }
}

/// `eq.c`'s `bf_eq_slope_texts` enum-item order: 6/12/18/24 dB/oct.
fn eq_slope_to_enum(slope_db_oct: u8) -> u32 {
    match slope_db_oct {
        12 => 1,
        18 => 2,
        24 => 3,
        _ => 0, // 6 dB/oct, and the fallback for any other value
    }
}

fn eq_slope_from_enum(v: u32) -> u8 {
    match v {
        1 => 12,
        2 => 18,
        3 => 24,
        _ => 6,
    }
}

/// A mono input's route into a stereo output pair is really *two*
/// independent ALSA volumes (one per crosspoint channel) — there is no
/// separate "pan" control on the hardware. TotalMix's single
/// volume+pan fader is a UI convenience over that pair, so this
/// decodes raw front-left/front-right volumes into the same
/// convention: the louder side sets volume, and how far the quieter
/// side is attenuated sets pan. `max` is the ALSA control's own
/// reported full-scale (`get_playback_volume_range().1`) — the real
/// hardware ranges differ per control (crosspoints top out at 0x2d41,
/// master volume at 0x4000), so this must never be hardcoded.
/// `encode_volume_pan` is the exact inverse, so round-tripping through
/// both is lossless (mod integer rounding).
fn decode_volume_pan(l_raw: i64, r_raw: i64, max: f32) -> (f32, i8) {
    let l = l_raw as f32 / max;
    let r = r_raw as f32 / max;
    let volume = l.max(r);
    let pan = if volume > 0.0 {
        (((r - l) / volume) * 100.0).round().clamp(-100.0, 100.0) as i8
    } else {
        0
    };
    (volume, pan)
}

fn encode_volume_pan(volume: f32, pan: i8, max: f32) -> (i64, i64) {
    let v = volume.clamp(0.0, 1.0);
    let p = (pan.clamp(-100, 100) as f32) / 100.0;
    let (l, r) = if p <= 0.0 {
        (v, v * (1.0 + p))
    } else {
        (v * (1.0 - p), v)
    };
    ((l * max) as i64, (r * max) as i64)
}

// ── Main struct ────────────────────────────────────────────────

/// Babyface Pro (FS) device controller.
pub struct BabyfacePro {
    mixer: AlsaMixer,
    profile: &'static DeviceProfile,
    inputs: Vec<InputChannel>,
    playbacks: Vec<PlaybackChannel>,
    outputs: Vec<OutputChannel>,
    settings: DeviceSettings,
    /// AN1/2 input-strip stereo link state (`"AN1/2 Link"`).
    /// Not exposed via [`DeviceSettings`] — matches the USB backend,
    /// which also keeps this outside `DeviceSettings` (no getter exists
    /// in [`RmeDevice`] for it, only `set_input_link`).
    linked: bool,
}

impl BabyfacePro {
    /// Look up a crosspoint's ALSA element: `BF_SOURCES[src]` at
    /// `.index = output*14 + src`, exactly `mixer.c`'s
    /// `babyface_create_xpoints` layout.
    fn crosspoint_selem(&self, src: usize, output: usize) -> Option<Selem<'_>> {
        self.mixer
            .find_selem(BF_SOURCES[src], (output * 14 + src) as u32)
    }

    /// Match ALSA mixer elements to our channel model.
    ///
    /// Global controls this driver doesn't yet expose over ALSA
    /// (Sample Clock Source, IEC958/SPDIF, per-input Sensitivity — none
    /// of these exist anywhere in `mixer.c`/`main.c`/`panel.c`/`eq.c` as
    /// of 2026-08-28) are left scanned-for-but-unmatched here rather
    /// than removed: harmless no-ops today, and a smaller diff than
    /// ripping the fields out of the model in the same pass that fixes
    /// the crosspoint/master/phantom/pad/gain grammar.
    fn attach_mixer_elements(&mut self) {
        let mono = SelemChannelId::mono();

        for (name, selem) in self.mixer.iter_selems() {
            if name == "Sample Clock Source" {
                if let Ok(current) = selem.get_enum_item(mono) {
                    if let Ok(item_name) = selem.get_enum_item_name(current) {
                        self.settings.clock_source = item_name;
                    }
                }
                if let Ok(count) = selem.get_enum_items() {
                    self.settings.clock_sources = (0..count)
                        .filter_map(|i| selem.get_enum_item_name(i).ok())
                        .collect();
                }
                continue;
            }
            if name == "IEC958 Emphasis" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_emphasis = v != 0;
                }
                continue;
            }
            if name == "IEC958 Pro Mask" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_professional = v != 0;
                }
                continue;
            }
            if name == "IEC958" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_enabled = v != 0;
                }
                continue;
            }
        }

        // ── Phantom 48V & PAD — Mic 1/Mic 2 = inputs[0]/inputs[1],
        // sharing one ALSA name per switch, disambiguated by `.index`
        // (not by name — `mixer.c:1279-1311`).
        for i in 0..2 {
            let Some(inp) = self.inputs.get_mut(i) else {
                continue;
            };
            if let Some(selem) = self.mixer.find_selem("Phantom Power Mic 1", i as u32) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    inp.phantom = v != 0;
                }
            }
            if let Some(selem) = self.mixer.find_selem("Pad Mic 1", i as u32) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    inp.pad = v != 0;
                }
            }
        }

        // ── Preamp gain — inputs[0..4] (Mic1, Mic2, Instr3, Instr4) map
        // 1:1 to ALSA index 0..4 of "Mic 1" (a *capture* volume, not
        // playback — `mixer.c:1313-1326`).
        for i in 0..4 {
            let Some(inp) = self.inputs.get_mut(i) else {
                continue;
            };
            if let Some(selem) = self.mixer.find_selem("Mic 1", i as u32) {
                if let Ok(v) = selem.get_capture_volume(mono) {
                    let (_, max) = selem.get_capture_volume_range();
                    inp.gain = Some(v as u32);
                    inp.gain_max = Some(max as u32);
                }
            }
        }

        // ── Crosspoint volume (+ pan for the 4 true-mono sources) ──
        for i in 0..self.inputs.len() {
            let Some((src, ch, is_mono)) = input_crosspoint_slot(i) else {
                continue;
            };
            for out in 0..self.profile.output_pair_count() {
                let Some(selem) = self.crosspoint_selem(src, out) else {
                    continue;
                };
                let max = selem.get_playback_volume_range().1 as f32;
                if is_mono {
                    let l = selem.get_playback_volume(SelemChannelId::FrontLeft).ok();
                    let r = selem.get_playback_volume(SelemChannelId::FrontRight).ok();
                    if let (Some(l), Some(r)) = (l, r) {
                        let (volume, pan) = decode_volume_pan(l, r, max);
                        self.inputs[i].volumes[out] = volume;
                        self.inputs[i].pans[out] = pan;
                    }
                } else if let Ok(v) = selem.get_playback_volume(ch) {
                    self.inputs[i].volumes[out] = v as f32 / max;
                }
            }
        }

        for i in 0..self.playbacks.len() {
            let Some((src, ch)) = playback_crosspoint_slot(i) else {
                continue;
            };
            for out in 0..self.profile.output_pair_count() {
                let Some(selem) = self.crosspoint_selem(src, out) else {
                    continue;
                };
                let max = selem.get_playback_volume_range().1 as f32;
                if let Ok(v) = selem.get_playback_volume(ch) {
                    self.playbacks[i].volumes[out] = v as f32 / max;
                }
            }
        }

        // ── Output pair master volume + mute — one 2-channel ALSA
        // element per pair (`PAIR_LABELS[pair_idx]` @ index `pair_idx`),
        // front-left/front-right are the pair's two physical channels;
        // the mute switch is shared by both (`mixer.c`'s `bf_mute_get`
        // mirrors the same value into both ALSA channels), so both
        // `self.outputs` entries for the pair get the same mute state.
        for (pair_idx, label) in PAIR_LABELS.iter().enumerate() {
            let Some(selem) = self.mixer.find_selem(label, pair_idx as u32) else {
                continue;
            };
            let max = selem.get_playback_volume_range().1 as f32;
            let l = selem.get_playback_volume(SelemChannelId::FrontLeft).ok();
            let r = selem.get_playback_volume(SelemChannelId::FrontRight).ok();
            let muted = selem
                .get_playback_switch(SelemChannelId::FrontLeft)
                .map(|v| v == 0)
                .unwrap_or(false);
            if let Some(l) = l {
                if let Some(out) = self.outputs.get_mut(pair_idx * 2) {
                    out.volume = l as f32 / max;
                    out.mute = muted;
                }
            }
            if let Some(r) = r {
                if let Some(out) = self.outputs.get_mut(pair_idx * 2 + 1) {
                    out.volume = r as f32 / max;
                    out.mute = muted;
                }
            }
        }

        // ── Loopback — one bool per output pair.
        for pair_idx in 0..PAIR_LABELS.len() {
            if let Some(selem) = self.mixer.find_selem("Loopback", pair_idx as u32) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    let on = v != 0;
                    if let Some(o) = self.outputs.get_mut(pair_idx * 2) {
                        o.loopback = on;
                    }
                    if let Some(o) = self.outputs.get_mut(pair_idx * 2 + 1) {
                        o.loopback = on;
                    }
                }
            }
        }

        // ── Global switches / values (AN 1>2, AN1/2 Link, MS Processor,
        // Dim, Width, FX Send, Varispeed Pitch) — all real ALSA controls
        // (mixer.c:1016-1114), previously never wired up on this backend.
        if let Some(selem) = self.mixer.find_selem("AN 1>2", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.an12 = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("AN1/2 Link", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.linked = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("MS Processor", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.ms_proc = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("Dim", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.dim = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("Width", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.width = v as f32 / 100.0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("FX Send", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.fx_send_db = Some(if v <= 0 { -65.0 } else { fader_raw_to_db(v as u16) });
            }
        }
        if let Some(selem) = self.mixer.find_selem("Varispeed Pitch", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.pitch_percent = v as f32 / 10.0;
            }
        }

        // ── Hardware DSP EQ — analog inputs only (`eq_strip_name`).
        for i in 0..4 {
            let Some(strip) = eq_strip_name(i) else {
                continue;
            };
            let mut eq = InputEq::default();
            let mut found_any = false;
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Enable"), 0) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    eq.enabled = v != 0;
                    found_any = true;
                }
            }
            for band in 0..3 {
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Type", band + 1), 0)
                {
                    if let Ok(v) = selem.get_enum_item(mono) {
                        eq.bands[band].band_type = eq_band_type_from_enum(v);
                        found_any = true;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Freq", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].freq_hz = v.clamp(0, 20_000) as u16;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Q", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].q = v as f32 / 100.0;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Gain", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].gain_db = v as f32 / 10.0;
                    }
                }
            }
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Freq"), 0) {
                if let Ok(v) = selem.get_playback_volume(mono) {
                    eq.low_cut_freq_hz = v.clamp(0, 20_000) as u16;
                }
            }
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Slope"), 0) {
                if let Ok(v) = selem.get_enum_item(mono) {
                    eq.low_cut_slope_db_oct = eq_slope_from_enum(v);
                }
            }
            if found_any {
                self.inputs[i].eq = Some(eq);
            }
        }

        info!(
            "Attached {} inputs, {} playbacks, clock: {}",
            self.inputs.len(),
            self.playbacks.len(),
            self.settings.clock_source
        );
    }

    fn channel(&self, ch: ChannelId) -> Result<(&bool, &bool), Error> {
        match ch {
            ChannelId::Input(idx) => self
                .inputs
                .get(idx)
                .map(|c| (&c.mute, &c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx))),
            ChannelId::Playback(idx) => self
                .playbacks
                .get(idx)
                .map(|c| (&c.mute, &c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx))),
            ChannelId::Output(idx) => self
                .outputs
                .get(idx)
                .map(|c| (&c.mute, &c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx))),
        }
    }

    fn channel_mut(&mut self, ch: ChannelId) -> Result<(&mut bool, &mut bool), Error> {
        match ch {
            ChannelId::Input(idx) => self
                .inputs
                .get_mut(idx)
                .map(|c| (&mut c.mute, &mut c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx))),
            ChannelId::Playback(idx) => self
                .playbacks
                .get_mut(idx)
                .map(|c| (&mut c.mute, &mut c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx))),
            ChannelId::Output(idx) => self
                .outputs
                .get_mut(idx)
                .map(|c| (&mut c.mute, &mut c.solo))
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx))),
        }
    }
}

impl RmeDevice for BabyfacePro {
    fn model_name(&self) -> &str {
        self.profile.model_name
    }

    fn output_pair_count(&self) -> usize {
        self.profile.output_pair_count()
    }

    fn open() -> Result<Self, Error> {
        info!("Searching for RME Babyface Pro...");
        let profile = &PROFILE;
        let mixer = AlsaMixer::open_by_card_name(profile.card_substring)?;
        let mut device = Self {
            mixer,
            profile,
            inputs: profile.build_inputs(),
            playbacks: profile.build_playbacks(),
            outputs: profile.build_outputs(),
            settings: DeviceSettings {
                clock_source: "Internal".into(),
                clock_sources: Vec::new(),
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
            },
            linked: false,
        };
        device.attach_mixer_elements();
        Ok(device)
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
        let vol_clamped = volume.clamp(0.0, 1.0);

        // Output channels: one 2-channel "<pair label> Playback Volume"
        // ALSA element per pair (`PAIR_LABELS`), front-left/front-right
        // are the pair's two physical channels — `idx / 2` finds the
        // pair, `idx % 2` picks which channel of it.
        if let ChannelId::Output(idx) = channel {
            let out = self
                .outputs
                .get_mut(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let label = PAIR_LABELS
                .get(idx / 2)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let ch = if idx % 2 == 0 {
                SelemChannelId::FrontLeft
            } else {
                SelemChannelId::FrontRight
            };
            if let Some(selem) = self.mixer.find_selem(label, (idx / 2) as u32) {
                let max = selem.get_playback_volume_range().1 as f32;
                selem.set_playback_volume(ch, (vol_clamped * max) as i64)?;
            }
            out.volume = vol_clamped;
            return Ok(());
        }

        if output >= self.profile.output_pair_count() {
            return Err(Error::InvalidChannel(format!("Output {}", output)));
        }

        let (src, ch, is_mono) = match channel {
            ChannelId::Input(idx) => input_crosspoint_slot(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?,
            ChannelId::Playback(idx) => {
                let (src, ch) = playback_crosspoint_slot(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                (src, ch, false)
            }
            ChannelId::Output(_) => unreachable!("Output handled above"),
        };

        if let Some(selem) = self.crosspoint_selem(src, output) {
            let max = selem.get_playback_volume_range().1 as f32;
            if is_mono {
                // Preserve the channel's current pan: a mono source's
                // "volume" is really two independent ALSA channels, so
                // writing volume alone must re-encode with the cached
                // pan rather than flattening it to center.
                let pan = if let ChannelId::Input(idx) = channel {
                    self.inputs[idx].pans[output]
                } else {
                    0
                };
                let (l, r) = encode_volume_pan(vol_clamped, pan, max);
                selem.set_playback_volume(SelemChannelId::FrontLeft, l)?;
                selem.set_playback_volume(SelemChannelId::FrontRight, r)?;
            } else {
                selem.set_playback_volume(ch, (vol_clamped * max) as i64)?;
            }
        }

        match channel {
            ChannelId::Input(idx) => self.inputs[idx].volumes[output] = vol_clamped,
            ChannelId::Playback(idx) => self.playbacks[idx].volumes[output] = vol_clamped,
            ChannelId::Output(_) => unreachable!("Output handled above"),
        }
        Ok(())
    }

    fn volume(&self, channel: ChannelId, output: usize) -> Result<f32, Error> {
        match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                ch.volumes
                    .get(output)
                    .copied()
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))
            }
            ChannelId::Playback(idx) => {
                let ch = self
                    .playbacks
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                ch.volumes
                    .get(output)
                    .copied()
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))
            }
            ChannelId::Output(idx) => self
                .outputs
                .get(idx)
                .map(|c| c.volume)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx))),
        }
    }

    fn set_pan(&mut self, channel: ChannelId, output: usize, pan: i8) -> Result<(), Error> {
        let pan = pan.clamp(-100, 100);

        // Only the 4 true-mono sources (AN1-AN4) have an independent
        // left/right crosspoint pair to pan across — AS1/2, the ADAT
        // pairs and every PCM playback channel are hardware-linked
        // stereo sources with one ALSA channel each, no cross-routing
        // possible (see `BF_SOURCES`/`input_crosspoint_slot`).
        let (idx, volume) = match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                (idx, ch.volumes[output])
            }
            ChannelId::Playback(_) => {
                return Err(Error::InvalidChannel(
                    "Playback channels are linked stereo pairs and have no independent pan"
                        .into(),
                ));
            }
            ChannelId::Output(_) => return Err(Error::InvalidChannel("Output has no pan".into())),
        };
        let (src, _ch, is_mono) =
            input_crosspoint_slot(idx).ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if !is_mono {
            return Err(Error::InvalidChannel(format!(
                "Input {} is a linked stereo source and has no independent pan",
                idx
            )));
        }

        if let Some(selem) = self.crosspoint_selem(src, output) {
            let max = selem.get_playback_volume_range().1 as f32;
            let (l, r) = encode_volume_pan(volume, pan, max);
            selem.set_playback_volume(SelemChannelId::FrontLeft, l)?;
            selem.set_playback_volume(SelemChannelId::FrontRight, r)?;
        }

        self.inputs[idx].pans[output] = pan;
        Ok(())
    }

    fn pan(&self, channel: ChannelId, output: usize) -> Result<i8, Error> {
        match channel {
            ChannelId::Input(idx) => self
                .inputs
                .get(idx)
                .and_then(|c| c.pans.get(output).copied())
                .ok_or_else(|| Error::InvalidChannel(format!("Channel {}", idx))),
            ChannelId::Playback(idx) => self
                .playbacks
                .get(idx)
                .and_then(|c| c.pans.get(output).copied())
                .ok_or_else(|| Error::InvalidChannel(format!("Channel {}", idx))),
            ChannelId::Output(_) => Err(Error::InvalidChannel("Output has no pan".into())),
        }
    }

    fn set_mute(&mut self, channel: ChannelId, mute: bool) -> Result<(), Error> {
        // Output mute is a real hardware switch, but it's shared by the
        // whole pair (`mixer.c`'s `bf_mute_get` mirrors one flag onto
        // both ALSA channels) — muting one physical output channel
        // mutes its sibling too, so mirror that into both `self.outputs`
        // entries rather than leaving them inconsistent with hardware.
        // Input/Playback mute has no dedicated ALSA switch at all (the
        // kernel driver has none), so it stays in-memory only, same as
        // before.
        if let ChannelId::Output(idx) = channel {
            let pair_idx = idx / 2;
            let label = PAIR_LABELS
                .get(pair_idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            if let Some(selem) = self.mixer.find_selem(label, pair_idx as u32) {
                let unmuted = (!mute) as i32;
                selem.set_playback_switch(SelemChannelId::FrontLeft, unmuted)?;
                selem.set_playback_switch(SelemChannelId::FrontRight, unmuted)?;
            }
            if let Some(out) = self.outputs.get_mut(pair_idx * 2) {
                out.mute = mute;
            }
            if let Some(out) = self.outputs.get_mut(pair_idx * 2 + 1) {
                out.mute = mute;
            }
            return Ok(());
        }
        let ch = self.channel_mut(channel)?;
        *ch.0 = mute;
        Ok(())
    }

    fn mute(&self, channel: ChannelId) -> Result<bool, Error> {
        let ch = self.channel(channel)?;
        Ok(*ch.0)
    }

    fn set_solo(&mut self, channel: ChannelId, solo: bool) -> Result<(), Error> {
        let ch = self.channel_mut(channel)?;
        *ch.1 = solo;
        Ok(())
    }

    fn solo(&self, channel: ChannelId) -> Result<bool, Error> {
        let ch = self.channel(channel)?;
        Ok(*ch.1)
    }

    fn set_phantom(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if inp.channel_type != ChannelType::Mic {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no 48V phantom power",
                idx
            )));
        }
        // Mic 1/Mic 2 = inputs[0]/inputs[1], mapping 1:1 to ALSA `.index`
        // (both mics share the literal name "Phantom Power Mic 1").
        if let Some(selem) = self.mixer.find_selem("Phantom Power Mic 1", idx as u32) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        inp.phantom = on;
        Ok(())
    }

    fn set_pad(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if inp.channel_type != ChannelType::Mic {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no pad switch",
                idx
            )));
        }
        if let Some(selem) = self.mixer.find_selem("Pad Mic 1", idx as u32) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        inp.pad = on;
        Ok(())
    }

    fn set_gain(&mut self, idx: usize, gain: u32) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if !matches!(inp.channel_type, ChannelType::Mic | ChannelType::Instrument) {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no gain control",
                idx
            )));
        }
        let clamped = inp.gain_max.map_or(gain, |max| gain.min(max));
        // inputs[0..4] (Mic1, Mic2, Instr3, Instr4) map 1:1 to ALSA
        // index 0..4 of "Mic 1" — a *capture* volume, not playback.
        if let Some(selem) = self.mixer.find_selem("Mic 1", idx as u32) {
            selem.set_capture_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.gain = Some(clamped);
        Ok(())
    }

    fn set_eq_enabled(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Enable"), 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).enabled = on;
        Ok(())
    }

    fn set_eq_band_type(
        &mut self,
        idx: usize,
        band: usize,
        band_type: EqBandType,
    ) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Type", band + 1), 0)
        {
            selem.set_enum_item(SelemChannelId::mono(), eq_band_type_to_enum(band_type))?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].band_type = band_type;
        Ok(())
    }

    fn set_eq_band_freq(&mut self, idx: usize, band: usize, freq_hz: u16) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = freq_hz.min(20_000);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Freq", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].freq_hz = clamped;
        Ok(())
    }

    fn set_eq_band_q(&mut self, idx: usize, band: usize, q: f32) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = q.clamp(0.05, 10.0);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Q", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), (clamped * 100.0).round() as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].q = clamped;
        Ok(())
    }

    fn set_eq_band_gain(&mut self, idx: usize, band: usize, gain_db: f32) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = gain_db.clamp(-24.0, 24.0);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Gain", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), (clamped * 10.0).round() as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].gain_db = clamped;
        Ok(())
    }

    fn set_eq_low_cut_freq(&mut self, idx: usize, freq_hz: u16) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let clamped = freq_hz.min(20_000);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Freq"), 0) {
            selem.set_playback_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).low_cut_freq_hz = clamped;
        Ok(())
    }

    fn set_eq_low_cut_slope(&mut self, idx: usize, slope_db_oct: u8) -> Result<(), Error> {
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Low Cut Slope"), 0)
        {
            selem.set_enum_item(SelemChannelId::mono(), eq_slope_to_enum(slope_db_oct))?;
        }
        inp.eq.get_or_insert_with(InputEq::default).low_cut_slope_db_oct = slope_db_oct;
        Ok(())
    }

    fn set_sensitivity(&mut self, idx: usize, sensitivity: Sensitivity) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if inp.channel_type != ChannelType::Instrument {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no sensitivity switch",
                idx
            )));
        }
        let elem_name = format!("Line-{} Sens.", inp.name);
        let item = match sensitivity {
            Sensitivity::Minus10dBV => 0,
            Sensitivity::Plus4dBu => 1,
        };
        if let Some(selem) = self.mixer.find_selem(&elem_name, 0) {
            selem.set_enum_item(SelemChannelId::mono(), item)?;
        }
        inp.sensitivity = Some(sensitivity);
        Ok(())
    }

    fn set_spdif_enabled(&mut self, enabled: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("IEC958", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), enabled as i32)?;
        }
        self.settings.spdif_enabled = enabled;
        Ok(())
    }

    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), Error> {
        // "Varispeed Pitch" exists over ALSA too (mixer.c:1021-1030),
        // range -50..50 in 0.1%-steps (raw = percent * 10) — despite
        // this method's old comment claiming otherwise, this is not
        // USB-only.
        let clamped = pitch_percent.clamp(-5.0, 5.0);
        if let Some(selem) = self.mixer.find_selem("Varispeed Pitch", 0) {
            let raw = (clamped * 10.0).round() as i64;
            selem.set_playback_volume(SelemChannelId::mono(), raw)?;
        }
        self.settings.pitch_percent = clamped;
        Ok(())
    }

    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), Error> {
        if out >= PAIR_LABELS.len() {
            return Err(Error::InvalidChannel(format!("Output pair {}", out)));
        }
        if let Some(selem) = self.mixer.find_selem("Loopback", out as u32) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        if let Some(o) = self.outputs.get_mut(out * 2) {
            o.loopback = on;
        }
        if let Some(o) = self.outputs.get_mut(out * 2 + 1) {
            o.loopback = on;
        }
        Ok(())
    }

    fn set_ms_proc(&mut self, on: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("MS Processor", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.ms_proc = on;
        Ok(())
    }

    fn set_an12(&mut self, on: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("AN 1>2", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.an12 = on;
        Ok(())
    }

    fn set_input_link(&mut self, linked: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("AN1/2 Link", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), linked as i32)?;
        }
        self.linked = linked;
        Ok(())
    }

    fn set_dim(&mut self, on: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("Dim", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.dim = on;
        Ok(())
    }

    fn set_width(&mut self, width: f32) -> Result<(), Error> {
        let clamped = width.clamp(-1.0, 1.0);
        if let Some(selem) = self.mixer.find_selem("Width", 0) {
            let raw = (clamped * 100.0).round() as i64;
            selem.set_playback_volume(SelemChannelId::mono(), raw)?;
        }
        self.settings.width = clamped;
        Ok(())
    }

    fn set_fx_send(&mut self, db: f32) -> Result<(), Error> {
        let db = db.clamp(-65.0, 0.0);
        if let Some(selem) = self.mixer.find_selem("FX Send", 0) {
            let max = selem.get_playback_volume_range().1 as u16;
            let raw = if db <= -65.0 {
                0
            } else {
                fader_db_to_raw(db).min(max)
            };
            selem.set_playback_volume(SelemChannelId::mono(), raw as i64)?;
        }
        self.settings.fx_send_db = Some(db);
        Ok(())
    }

    fn set_spdif_emphasis(&mut self, enabled: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("IEC958 Emphasis", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), enabled as i32)?;
        }
        self.settings.spdif_emphasis = enabled;
        Ok(())
    }

    fn set_spdif_professional(&mut self, enabled: bool) -> Result<(), Error> {
        if let Some(selem) = self.mixer.find_selem("IEC958 Pro Mask", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), enabled as i32)?;
        }
        self.settings.spdif_professional = enabled;
        Ok(())
    }

    fn set_clock_source(&mut self, source: &str) -> Result<(), Error> {
        let selem = self
            .mixer
            .find_selem("Sample Clock Source", 0)
            .ok_or_else(|| Error::InvalidChannel("No clock source control".into()))?;
        let count = selem.get_enum_items().unwrap_or(0);
        for idx in 0..count {
            if selem
                .get_enum_item_name(idx)
                .map(|n| n == source)
                .unwrap_or(false)
            {
                selem.set_enum_item(SelemChannelId::mono(), idx)?;
                self.settings.clock_source = source.to_string();
                return Ok(());
            }
        }
        Err(Error::InvalidChannel(format!(
            "Unknown clock source: {}",
            source
        )))
    }

    fn capture_scene(&self) -> Scene {
        Scene {
            name: "Untitled".into(),
            model: self.profile.model_name.to_string(),
            inputs: self.inputs.clone(),
            playbacks: self.playbacks.clone(),
            outputs: self.outputs.clone(),
            settings: self.settings.clone(),
        }
    }

    fn apply_scene(&mut self, scene: &Scene) -> Result<(), Error> {
        scene.check_compatible(self.profile.model_name)?;
        for (i, saved) in scene.inputs.iter().enumerate() {
            for (out, &v) in saved.volumes.iter().enumerate() {
                self.set_volume(ChannelId::Input(i), out, v)?;
            }
        }
        for (i, saved) in scene.playbacks.iter().enumerate() {
            for (out, &v) in saved.volumes.iter().enumerate() {
                self.set_volume(ChannelId::Playback(i), out, v)?;
            }
        }
        for (i, saved) in scene.outputs.iter().enumerate() {
            self.set_volume(ChannelId::Output(i), 0, saved.volume)?;
        }
        self.inputs = scene.inputs.clone();
        self.playbacks = scene.playbacks.clone();
        self.outputs = scene.outputs.clone();
        self.settings = scene.settings.clone();
        Ok(())
    }

    fn poll_events(&mut self) -> Result<(), Error> {
        let _ = self.mixer.handle_events()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MAX: f32 = 65536.0;

    #[test]
    fn decode_center_pan_from_equal_raw_volumes() {
        let (volume, pan) = decode_volume_pan(32768, 32768, TEST_MAX);
        assert!((volume - 0.5).abs() < 1e-4);
        assert_eq!(pan, 0);
    }

    #[test]
    fn decode_hard_left_from_silent_right() {
        let (volume, pan) = decode_volume_pan(65536, 0, TEST_MAX);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, -100);
    }

    #[test]
    fn decode_hard_right_from_silent_left() {
        let (volume, pan) = decode_volume_pan(0, 65536, TEST_MAX);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, 100);
    }

    #[test]
    fn decode_silence_defaults_to_center_pan() {
        let (volume, pan) = decode_volume_pan(0, 0, TEST_MAX);
        assert_eq!(volume, 0.0);
        assert_eq!(pan, 0);
    }

    #[test]
    fn encode_then_decode_round_trips_for_a_range_of_volume_pan_pairs() {
        for vol_pct in [0, 10, 25, 50, 75, 100] {
            for pan in [-100, -50, -25, 0, 25, 50, 100] {
                let volume = vol_pct as f32 / 100.0;
                let (l, r) = encode_volume_pan(volume, pan, TEST_MAX);
                let (decoded_vol, decoded_pan) = decode_volume_pan(l, r, TEST_MAX);
                assert!(
                    (decoded_vol - volume).abs() < 0.01,
                    "volume {} pan {}: decoded volume {}",
                    volume,
                    pan,
                    decoded_vol
                );
                // At volume 0 every raw pair is (0, 0), so pan is
                // undecidable — decode_volume_pan defaults it to 0
                // regardless of what was encoded.
                if vol_pct > 0 {
                    assert!(
                        (decoded_pan - pan).abs() <= 1,
                        "volume {} pan {}: decoded pan {}",
                        volume,
                        pan,
                        decoded_pan
                    );
                }
            }
        }
    }

    #[test]
    fn encode_center_pan_gives_equal_left_and_right() {
        let (l, r) = encode_volume_pan(0.75, 0, TEST_MAX);
        assert_eq!(l, r);
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_pan_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let cid = ChannelId::Input(1); // AN2, verified silent (0%) before running this
        let orig_vol = dev.volume(cid, 0).unwrap();
        let orig_pan = dev.pan(cid, 0).unwrap();

        dev.set_volume(cid, 0, 0.5).unwrap();
        dev.set_pan(cid, 0, 50).unwrap();
        assert!((dev.volume(cid, 0).unwrap() - 0.5).abs() < 0.02);
        assert!((dev.pan(cid, 0).unwrap() - 50).abs() <= 1);

        // Re-open to force a fresh attach_mixer_elements() read from
        // hardware, proving the round trip survives a real re-read,
        // not just the in-memory field set above.
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!((dev2.volume(cid, 0).unwrap() - 0.5).abs() < 0.02);
        assert!((dev2.pan(cid, 0).unwrap() - 50).abs() <= 1);
        drop(dev2);

        dev.set_volume(cid, 0, orig_vol).unwrap();
        dev.set_pan(cid, 0, orig_pan).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_spdif_enabled_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let orig = dev.settings().spdif_enabled;

        dev.set_spdif_enabled(!orig).unwrap();
        assert_eq!(dev.settings().spdif_enabled, !orig);

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.settings().spdif_enabled, !orig);
        drop(dev2);

        dev.set_spdif_enabled(orig).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_spdif_emphasis_and_professional_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let orig_emph = dev.settings().spdif_emphasis;
        let orig_prof = dev.settings().spdif_professional;

        dev.set_spdif_emphasis(!orig_emph).unwrap();
        dev.set_spdif_professional(!orig_prof).unwrap();

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.settings().spdif_emphasis, !orig_emph);
        assert_eq!(dev2.settings().spdif_professional, !orig_prof);
        drop(dev2);

        dev.set_spdif_emphasis(orig_emph).unwrap();
        dev.set_spdif_professional(orig_prof).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_clock_source_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let orig = dev.settings().clock_source.clone();
        let sources = dev.settings().clock_sources.clone();
        let other = sources
            .iter()
            .find(|s| **s != orig)
            .expect("hardware exposes more than one clock source")
            .clone();

        dev.set_clock_source(&other).unwrap();
        assert_eq!(dev.settings().clock_source, other);

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.settings().clock_source, other);
        drop(dev2);

        dev.set_clock_source(&orig).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_output_volume_beyond_sixth_pair_does_not_panic_or_clobber_a_sibling() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        // idx=8 (ADAT5 OUT, the 9th individual output channel) is the
        // exact index reported crashing in the wild: `self.outputs` has
        // 12 individual-channel entries, but the old code indexed
        // `self.profile.outputs` (6 *pairs*) with the same raw `idx`,
        // panicking for any idx >= 6. idx=9 (ADAT6 OUT) is idx=8's pair
        // sibling — the old code, for idx in 2..=5, also silently wrote
        // *both* channels of the wrong pair, so round-tripping idx=8
        // alone and confirming idx=9 didn't move covers both failure
        // modes in one test.
        let target = ChannelId::Output(8);
        let sibling = ChannelId::Output(9);
        let orig_target = dev.volume(target, 0).unwrap();
        let orig_sibling = dev.volume(sibling, 0).unwrap();

        dev.set_volume(target, 0, 0.5).unwrap();
        assert!((dev.volume(target, 0).unwrap() - 0.5).abs() < 0.02);
        assert!(
            (dev.volume(sibling, 0).unwrap() - orig_sibling).abs() < 0.02,
            "writing Output(8) should not move Output(9)'s volume"
        );

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!((dev2.volume(target, 0).unwrap() - 0.5).abs() < 0.02);
        drop(dev2);

        dev.set_volume(target, 0, orig_target).unwrap();
    }
}
