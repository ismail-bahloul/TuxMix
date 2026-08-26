//! Babyface Pro (FS) implementation of the [`RmeDevice`] trait.

use alsa::mixer::SelemChannelId;
use log::info;

use crate::channel::OutputChannel;
use crate::channel::*;
use crate::device::{DeviceSettings, RmeDevice};
use crate::error::Error;
use crate::mixer::AlsaMixer;
use crate::profile::DeviceProfile;
use crate::profiles::babyface_pro::PROFILE;
use crate::scene::Scene;

// ── Helpers ────────────────────────────────────────────────────
//
// NOTE: these two functions encode the ALSA selem naming grammar
// observed on the Babyface Pro FS specifically (`"<Type>-<Name>-
// <Output>"`, `" 48V"`/`" PAD"`/`" Sens."` suffixes, `"Clock Mode"`).
// This is *not* confirmed to be shared by other RME models — if you're
// porting this file for a second device, re-verify against real
// `amixer scontents` output before assuming these patterns transfer.

fn selem_name(ch_type: &str, ch_name: &str, out_name: &str) -> String {
    format!("{}-{}-{}", ch_type, ch_name, out_name)
}

fn ch_type_str(ct: ChannelType) -> &'static str {
    match ct {
        ChannelType::Mic => "Mic",
        ChannelType::Instrument => "Line",
        ChannelType::Line | ChannelType::SPDIF | ChannelType::ADAT => "Line",
    }
}

/// A mono input/playback's route into a stereo output pair is really
/// *two* independent ALSA volumes (one per crosspoint) — there is no
/// "pan" control on the hardware. TotalMix's single volume+pan fader is
/// a UI convenience over that pair, so this decodes raw L/R volumes
/// (0..=65536 each) into the same convention: the louder side sets
/// volume, and how far the quieter side is attenuated sets pan.
/// `encode_volume_pan` is the exact inverse, so round-tripping through
/// both is lossless (mod integer rounding).
fn decode_volume_pan(l_raw: i64, r_raw: i64) -> (f32, i8) {
    let l = l_raw as f32 / 65536.0;
    let r = r_raw as f32 / 65536.0;
    let volume = l.max(r);
    let pan = if volume > 0.0 {
        (((r - l) / volume) * 100.0).round().clamp(-100.0, 100.0) as i8
    } else {
        0
    };
    (volume, pan)
}

fn encode_volume_pan(volume: f32, pan: i8) -> (i64, i64) {
    let v = volume.clamp(0.0, 1.0);
    let p = (pan.clamp(-100, 100) as f32) / 100.0;
    let (l, r) = if p <= 0.0 {
        (v, v * (1.0 + p))
    } else {
        (v * (1.0 - p), v)
    };
    ((l * 65536.0) as i64, (r * 65536.0) as i64)
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
}

impl BabyfacePro {
    /// Match ALSA mixer elements to our channel model.
    fn attach_mixer_elements(&mut self) {
        let mono = SelemChannelId::mono();

        for (name, selem) in self.mixer.iter_selems() {
            // ── Global: Sample Clock Source ──────────────────
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

            // ── Global: SPDIF (IEC958) format flags ──────────
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
            // The raw ALSA control is named "IEC958 Switch"; amixer's
            // simple-mixer view strips the well-known " Switch" suffix,
            // so it shows up here as just "IEC958" (confirmed via
            // `amixer controls`' numid listing — this is the standard
            // generic ALSA S/PDIF output-enable switch, not something
            // RME-specific or ambiguous).
            if name == "IEC958" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_enabled = v != 0;
                }
                continue;
            }

            // ── Phantom 48V & PAD for Mic inputs ────────────
            for i in 0..self.inputs.len() {
                if self.inputs[i].channel_type != ChannelType::Mic {
                    continue;
                }
                let expected_48v = format!("Mic-{} 48V", self.inputs[i].name);
                if name == expected_48v {
                    if let Ok(v) = selem.get_playback_switch(mono) {
                        self.inputs[i].phantom = v != 0;
                    }
                    break;
                }

                let expected_pad = format!("Mic-{} PAD", self.inputs[i].name);
                if name == expected_pad {
                    if let Ok(v) = selem.get_playback_switch(mono) {
                        self.inputs[i].pad = v != 0;
                    }
                    break;
                }
            }

            // ── Sensitivity for Instrument inputs ───────────
            for i in 0..self.inputs.len() {
                if self.inputs[i].channel_type != ChannelType::Instrument {
                    continue;
                }
                let expected_sens = format!("Line-{} Sens.", self.inputs[i].name);
                if name == expected_sens {
                    if let Ok(item) = selem.get_enum_item(mono) {
                        self.inputs[i].sensitivity = Some(if item == 0 {
                            Sensitivity::Minus10dBV
                        } else {
                            Sensitivity::Plus4dBu
                        });
                    }
                    break;
                }
            }

            // ── Preamp Gain for Mic and Instrument inputs ───
            for i in 0..self.inputs.len() {
                if !matches!(
                    self.inputs[i].channel_type,
                    ChannelType::Mic | ChannelType::Instrument
                ) {
                    continue;
                }
                let ct = ch_type_str(self.inputs[i].channel_type);
                let expected_gain = format!("{}-{} Gain", ct, self.inputs[i].name);
                if name == expected_gain {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        let (_, max) = selem.get_playback_volume_range();
                        self.inputs[i].gain = Some(v as u32);
                        self.inputs[i].gain_max = Some(max as u32);
                    }
                    break;
                }
            }
        }

        // ── Per-output volume+pan for inputs and playbacks ──────
        //
        // Deliberately *not* folded into the streaming scan above: each
        // output pair needs both the left and right crosspoint read
        // together to decode a real pan (see `decode_volume_pan`) — a
        // streaming match on individual selem names can only ever see
        // one side at a time, silently discarding whichever one it
        // read first. Direct `find_selem` lookups fetch both sides
        // deterministically instead.
        for i in 0..self.inputs.len() {
            let ct = ch_type_str(self.inputs[i].channel_type);
            let name = self.inputs[i].name.clone();
            for (out_idx, pair) in self.profile.outputs.iter().enumerate() {
                let l = self
                    .mixer
                    .find_selem(&selem_name(ct, &name, pair.left), 0)
                    .and_then(|s| s.get_playback_volume(mono).ok());
                let r = self
                    .mixer
                    .find_selem(&selem_name(ct, &name, pair.right), 0)
                    .and_then(|s| s.get_playback_volume(mono).ok());
                if let (Some(l), Some(r)) = (l, r) {
                    let (volume, pan) = decode_volume_pan(l, r);
                    self.inputs[i].volumes[out_idx] = volume;
                    self.inputs[i].pans[out_idx] = pan;
                }
            }
        }

        for i in 0..self.playbacks.len() {
            let ch_name = self.playbacks[i].name[4..].to_string(); // strip "PCM "
            for (out_idx, pair) in self.profile.outputs.iter().enumerate() {
                let l = self
                    .mixer
                    .find_selem(&selem_name("PCM", &ch_name, pair.left), 0)
                    .and_then(|s| s.get_playback_volume(mono).ok());
                let r = self
                    .mixer
                    .find_selem(&selem_name("PCM", &ch_name, pair.right), 0)
                    .and_then(|s| s.get_playback_volume(mono).ok());
                if let (Some(l), Some(r)) = (l, r) {
                    let (volume, pan) = decode_volume_pan(l, r);
                    self.playbacks[i].volumes[out_idx] = volume;
                    self.playbacks[i].pans[out_idx] = pan;
                }
            }
        }

        // Each individual output channel (`self.outputs`, 12 entries —
        // AN1/AN2/PH3/PH4/.../ADAT7/ADAT8) has no pan concept — it's a
        // single hardware-level volume, not a routed crosspoint pair — so
        // read both sides of each pair deterministically by name rather
        // than relying on streaming-scan iteration order. Both sides,
        // *not* just `pair.left`: unlike a crosspoint's volume+pan, a
        // pair's two output channels aren't linked, so the left side's
        // level says nothing about the right side's.
        for (i, pair) in self.profile.outputs.iter().enumerate() {
            for (out_name, ch_idx) in [(pair.left, i * 2), (pair.right, i * 2 + 1)] {
                if let Some(selem) = self.mixer.find_selem(&format!("Main-Out {}", out_name), 0) {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        self.outputs[ch_idx].volume = (v as f32) / 65536.0;
                    }
                }
            }
        }

        info!(
            "Attached {} inputs, {} playbacks, clock: {}",
            self.inputs.len(),
            self.playbacks.len(),
            self.settings.clock_source
        );
    }

    /// Write both crosspoints of an input/playback's route into an
    /// output pair from a volume+pan pair — shared by `set_volume` and
    /// `set_pan` so that setting one never flattens the other (writing
    /// only `volume` to both sides, ignoring the channel's current pan,
    /// is exactly the bug this replaced).
    fn write_crosspoint(
        &self,
        ch_type: &str,
        ch_name: &str,
        output: usize,
        volume: f32,
        pan: i8,
    ) -> Result<(), Error> {
        let pair = &self.profile.outputs[output];
        let (l_raw, r_raw) = encode_volume_pan(volume, pan);
        let mono = SelemChannelId::mono();
        if let Some(selem) = self
            .mixer
            .find_selem(&selem_name(ch_type, ch_name, pair.left), 0)
        {
            selem.set_playback_volume(mono, l_raw)?;
        }
        if let Some(selem) = self
            .mixer
            .find_selem(&selem_name(ch_type, ch_name, pair.right), 0)
        {
            selem.set_playback_volume(mono, r_raw)?;
        }
        Ok(())
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
                fx_send_db: None,
                width: 0.0,
                sample_rate: 48_000,
            },
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
        let vol_raw = (vol_clamped * 65536.0) as i64;
        let mono = SelemChannelId::mono();

        // Output channels use a different ALSA naming scheme ("Main-Out
        // <name>") and `idx` (not the `output` submix-bus param, which
        // callers always pass as 0) selects which physical *channel* to
        // write — `self.outputs` is one entry per individual output
        // channel (12, AN1/AN2/PH3/PH4/.../ADAT7/ADAT8), but
        // `self.profile.outputs` is one entry per *pair* (6): `idx / 2`
        // finds the pair, `idx % 2` picks left vs right within it. Indexing
        // `profile.outputs` with the raw channel `idx` directly (the
        // previous version of this code) panics for any `idx >= 6`
        // (`ADAT3 OUT` on) and, worse, silently writes the *wrong* pair's
        // ALSA elements — both channels of it — for `idx` 2 through 5.
        if let ChannelId::Output(idx) = channel {
            let out = self
                .outputs
                .get_mut(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let pair = self
                .profile
                .outputs
                .get(idx / 2)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let out_name = if idx % 2 == 0 { pair.left } else { pair.right };
            let elem_name = format!("Main-Out {}", out_name);
            if let Some(selem) = self.mixer.find_selem(&elem_name, 0) {
                selem.set_playback_volume(mono, vol_raw)?;
            }
            out.volume = vol_clamped;
            return Ok(());
        }

        if output >= self.profile.output_pair_count() {
            return Err(Error::InvalidChannel(format!("Output {}", output)));
        }

        let (ch_type, ch_name, pan) = match channel {
            ChannelId::Input(idx) => {
                let inp = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                (
                    ch_type_str(inp.channel_type),
                    inp.name.clone(),
                    inp.pans[output],
                )
            }
            ChannelId::Playback(idx) => {
                let pb = self
                    .playbacks
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                ("PCM", pb.name[4..].to_string(), pb.pans[output])
            }
            ChannelId::Output(_) => unreachable!("Output handled above"),
        };

        self.write_crosspoint(ch_type, &ch_name, output, vol_clamped, pan)?;

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

        let (ch_type, ch_name, volume) = match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                (
                    ch_type_str(ch.channel_type),
                    ch.name.clone(),
                    ch.volumes[output],
                )
            }
            ChannelId::Playback(idx) => {
                let ch = self
                    .playbacks
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                ("PCM", ch.name[4..].to_string(), ch.volumes[output])
            }
            ChannelId::Output(_) => return Err(Error::InvalidChannel("Output has no pan".into())),
        };

        self.write_crosspoint(ch_type, &ch_name, output, volume, pan)?;

        match channel {
            ChannelId::Input(idx) => self.inputs[idx].pans[output] = pan,
            ChannelId::Playback(idx) => self.playbacks[idx].pans[output] = pan,
            ChannelId::Output(_) => unreachable!("Output handled above"),
        }
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
        let elem_name = format!("Mic-{} 48V", inp.name);
        if let Some(selem) = self.mixer.find_selem(&elem_name, 0) {
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
        let elem_name = format!("Mic-{} PAD", inp.name);
        if let Some(selem) = self.mixer.find_selem(&elem_name, 0) {
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
        let ct = ch_type_str(inp.channel_type);
        let elem_name = format!("{}-{} Gain", ct, inp.name);
        let clamped = inp.gain_max.map_or(gain, |max| gain.min(max));
        if let Some(selem) = self.mixer.find_selem(&elem_name, 0) {
            selem.set_playback_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.gain = Some(clamped);
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

    fn set_pitch(&mut self, _pitch_percent: f32) -> Result<(), Error> {
        // The class-compliant ALSA path has no varispeed control; the
        // 0x1B DDS quad is proprietary-USB only.
        Err(Error::InvalidChannel(
            "Pitch/varispeed is not available on the ALSA class-compliant backend".into(),
        ))
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

    #[test]
    fn decode_center_pan_from_equal_raw_volumes() {
        let (volume, pan) = decode_volume_pan(32768, 32768);
        assert!((volume - 0.5).abs() < 1e-4);
        assert_eq!(pan, 0);
    }

    #[test]
    fn decode_hard_left_from_silent_right() {
        let (volume, pan) = decode_volume_pan(65536, 0);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, -100);
    }

    #[test]
    fn decode_hard_right_from_silent_left() {
        let (volume, pan) = decode_volume_pan(0, 65536);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, 100);
    }

    #[test]
    fn decode_silence_defaults_to_center_pan() {
        let (volume, pan) = decode_volume_pan(0, 0);
        assert_eq!(volume, 0.0);
        assert_eq!(pan, 0);
    }

    #[test]
    fn encode_then_decode_round_trips_for_a_range_of_volume_pan_pairs() {
        for vol_pct in [0, 10, 25, 50, 75, 100] {
            for pan in [-100, -50, -25, 0, 25, 50, 100] {
                let volume = vol_pct as f32 / 100.0;
                let (l, r) = encode_volume_pan(volume, pan);
                let (decoded_vol, decoded_pan) = decode_volume_pan(l, r);
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
        let (l, r) = encode_volume_pan(0.75, 0);
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
