//! Mock implementation of [`RmeDevice`] for development and testing.
//!
//! [`MockBabyfacePro`] simulates a Babyface Pro FS entirely in memory,
//! without any ALSA or hardware interaction.
//!
//! Use `--mock` to run the TUI/GUI without a physical device:
//! ```bash
//! cargo run -p tuxmix-gui -- --mock
//! ```

use rand::Rng;

use crate::channel::*;
use crate::device::{DeviceSettings, RmeDevice};
use crate::error::Error;
use crate::profiles::babyface_pro::PROFILE;
use crate::scene::Scene;

// ── Main struct ─────────────────────────────────────────────────

/// A simulated Babyface Pro FS that works without any hardware.
///
/// All state is kept in memory. Every operation succeeds immediately.
/// Use this to develop or test UI code without a physical RME device.
/// Topology (channel counts/names/types) comes from the same
/// [`PROFILE`] as [`crate::BabyfacePro`], so the two can't drift apart
/// the way the old hand-duplicated const tables did.
pub struct MockBabyfacePro {
    model_name: String,
    inputs: Vec<InputChannel>,
    playbacks: Vec<PlaybackChannel>,
    outputs: Vec<OutputChannel>,
    settings: DeviceSettings,
    input_meters: Vec<f32>,
    playback_meters: Vec<f32>,
    tick: u64,
}

impl MockBabyfacePro {
    fn update_meters(&mut self) {
        let mut rng = rand::thread_rng();
        self.tick += 1;

        for v in &mut self.input_meters {
            let target = rng.gen_range(0.0..0.95);
            *v += (target - *v) * 0.05;
            *v = v.clamp(0.0, 1.0);
        }
        for v in &mut self.playback_meters {
            let target = rng.gen_range(0.0..0.85);
            *v += (target - *v) * 0.03;
            *v = v.clamp(0.0, 1.0);
        }
    }

    pub fn input_meter(&self, idx: usize) -> f32 {
        self.input_meters.get(idx).copied().unwrap_or(0.0)
    }

    pub fn playback_meter(&self, idx: usize) -> f32 {
        self.playback_meters.get(idx).copied().unwrap_or(0.0)
    }

    pub fn input_meters(&self) -> &[f32] {
        &self.input_meters
    }

    pub fn playback_meters(&self) -> &[f32] {
        &self.playback_meters
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

impl RmeDevice for MockBabyfacePro {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn output_pair_count(&self) -> usize {
        PROFILE.output_pair_count()
    }

    fn open() -> Result<Self, Error> {
        // Same topology as the real device, plus a mock-only demo
        // default: the first two (Mic) inputs start with phantom power
        // on, so the UI has something interesting to show immediately.
        let mut inputs = PROFILE.build_inputs();
        for (i, ch) in inputs.iter_mut().enumerate() {
            ch.phantom = i < 2;
            // Ranges matching the kernel driver's controls (2026-08-26:
            // AN1/2 mic gain 0-65 dB, AN3/4 instrument 0-9 dB — the
            // instrument raw is dB×2 = 0-18, cap_gain34.pcap).
            ch.gain_max = match ch.channel_type {
                ChannelType::Mic => Some(65),
                ChannelType::Instrument => Some(9),
                _ => None,
            };
            ch.gain = ch.gain_max.map(|_| 0);
        }

        Ok(Self {
            model_name: format!("{} (mock)", PROFILE.model_name),
            inputs,
            playbacks: PROFILE.build_playbacks(),
            outputs: PROFILE.build_outputs(),
            settings: DeviceSettings {
                clock_source: "Internal".into(),
                // Mocked from the real Babyface Pro FS's own enum items
                // (see `babyface.rs::attach_mixer_elements`), so the
                // Quick Control clock picker has something real to show
                // in --mock too.
                clock_sources: vec!["Internal".into(), "AutoSync".into()],
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
            input_meters: vec![0.0; PROFILE.input_count()],
            playback_meters: vec![0.0; PROFILE.output_pair_count() * 2],
            tick: 0,
        })
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
        let vol = volume.clamp(0.0, 1.0);
        match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get_mut(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                ch.volumes
                    .get_mut(output)
                    .map(|v| *v = vol)
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))?;
            }
            ChannelId::Playback(idx) => {
                let ch = self
                    .playbacks
                    .get_mut(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                ch.volumes
                    .get_mut(output)
                    .map(|v| *v = vol)
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))?;
            }
            ChannelId::Output(idx) => {
                let ch = self
                    .outputs
                    .get_mut(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
                ch.volume = vol;
            }
        }
        Ok(())
    }

    fn volume(&self, channel: ChannelId, output: usize) -> Result<f32, Error> {
        match channel {
            ChannelId::Input(idx) => self
                .inputs
                .get(idx)
                .and_then(|c| c.volumes.get(output).copied())
                .ok_or_else(|| Error::InvalidChannel(format!("Channel {}", idx))),
            ChannelId::Playback(idx) => self
                .playbacks
                .get(idx)
                .and_then(|c| c.volumes.get(output).copied())
                .ok_or_else(|| Error::InvalidChannel(format!("Channel {}", idx))),
            ChannelId::Output(idx) => self
                .outputs
                .get(idx)
                .map(|c| c.volume)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx))),
        }
    }

    fn set_pan(&mut self, channel: ChannelId, output: usize, pan: i8) -> Result<(), Error> {
        let pan = pan.clamp(-100, 100);
        match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get_mut(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                ch.pans[output] = pan;
            }
            ChannelId::Playback(idx) => {
                let ch = self
                    .playbacks
                    .get_mut(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                ch.pans[output] = pan;
            }
            ChannelId::Output(_) => return Err(Error::InvalidChannel("Output has no pan".into())),
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
        inp.sensitivity = Some(sensitivity);
        Ok(())
    }

    fn set_spdif_enabled(&mut self, enabled: bool) -> Result<(), Error> {
        self.settings.spdif_enabled = enabled;
        Ok(())
    }

    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), Error> {
        self.settings.pitch_percent = pitch_percent.clamp(-5.0, 5.0);
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) -> Result<(), Error> {
        self.settings.sample_rate = rate;
        Ok(())
    }

    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), Error> {
        let o = self
            .outputs
            .get_mut(out)
            .ok_or_else(|| Error::InvalidChannel(format!("Output {out}")))?;
        o.loopback = on;
        Ok(())
    }

    fn set_an12(&mut self, on: bool) -> Result<(), Error> {
        self.settings.an12 = on;
        Ok(())
    }

    fn set_ms_proc(&mut self, on: bool) -> Result<(), Error> {
        self.settings.ms_proc = on;
        Ok(())
    }

    fn set_phase(&mut self, idx: usize, invert: bool) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {idx}")))?;
        inp.phase = invert;
        Ok(())
    }

    fn set_fx_send(&mut self, db: f32) -> Result<(), Error> {
        self.settings.fx_send_db = Some(db.clamp(-65.0, 0.0));
        Ok(())
    }

    fn set_stereo_split(&mut self, pb: usize, split: bool) -> Result<(), Error> {
        if pb >= self.playbacks.len() {
            return Err(Error::InvalidChannel(format!("Playback {pb}")));
        }
        self.playbacks[pb].split = split;
        self.playbacks[pb ^ 1].split = split;
        Ok(())
    }

    fn set_width(&mut self, width: f32) -> Result<(), Error> {
        self.settings.width = width.clamp(-1.0, 1.0);
        Ok(())
    }

    fn set_ref_level(&mut self, idx: usize, raw: u16) -> Result<(), Error> {
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {idx}")))?;
        if inp.channel_type != ChannelType::Instrument {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no ref-level switch"
            )));
        }
        inp.ref_level = raw;
        Ok(())
    }

    fn set_spdif_emphasis(&mut self, enabled: bool) -> Result<(), Error> {
        self.settings.spdif_emphasis = enabled;
        Ok(())
    }

    fn set_spdif_professional(&mut self, enabled: bool) -> Result<(), Error> {
        self.settings.spdif_professional = enabled;
        Ok(())
    }

    fn set_clock_source(&mut self, source: &str) -> Result<(), Error> {
        if !self.settings.clock_sources.iter().any(|s| s == source) {
            return Err(Error::InvalidChannel(format!(
                "Unknown clock source: {}",
                source
            )));
        }
        self.settings.clock_source = source.to_string();
        Ok(())
    }

    fn capture_scene(&self) -> Scene {
        Scene {
            name: "Untitled".into(),
            model: self.model_name.clone(),
            inputs: self.inputs.clone(),
            playbacks: self.playbacks.clone(),
            outputs: self.outputs.clone(),
            settings: self.settings.clone(),
        }
    }

    fn apply_scene(&mut self, scene: &Scene) -> Result<(), Error> {
        scene.check_compatible(&self.model_name)?;
        self.inputs = scene.inputs.clone();
        self.playbacks = scene.playbacks.clone();
        self.outputs = scene.outputs.clone();
        self.settings = scene.settings.clone();
        Ok(())
    }

    fn poll_events(&mut self) -> Result<(), Error> {
        self.update_meters();
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_creates_correct_number_of_channels() {
        let dev = MockBabyfacePro::open().unwrap();
        assert_eq!(dev.inputs().len(), 12);
        assert_eq!(dev.playbacks().len(), 12);
    }

    #[test]
    fn test_output_pair_count() {
        let dev = MockBabyfacePro::open().unwrap();
        assert_eq!(dev.output_pair_count(), 6);
    }

    #[test]
    fn test_each_channel_has_per_output_volumes() {
        let dev = MockBabyfacePro::open().unwrap();
        for ch in dev.inputs() {
            assert_eq!(ch.volumes.len(), 6);
            assert_eq!(ch.pans.len(), 6);
        }
        for ch in dev.playbacks() {
            assert_eq!(ch.volumes.len(), 6);
            assert_eq!(ch.pans.len(), 6);
        }
    }

    #[test]
    fn test_per_output_volume() {
        let mut dev = MockBabyfacePro::open().unwrap();
        // Set volume for input 0 towards output 2 only
        dev.set_volume(ChannelId::Input(0), 2, 0.3).unwrap();
        assert!((dev.volume(ChannelId::Input(0), 2).unwrap() - 0.3).abs() < 1e-6);
        // Other outputs should be unchanged (default 0 dB = 1.0)
        assert!((dev.volume(ChannelId::Input(0), 0).unwrap() - 1.0).abs() < 1e-6);
        assert!((dev.volume(ChannelId::Input(0), 5).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_per_output_pan() {
        let mut dev = MockBabyfacePro::open().unwrap();
        dev.set_pan(ChannelId::Playback(0), 3, 50).unwrap();
        assert_eq!(dev.pan(ChannelId::Playback(0), 3).unwrap(), 50);
        assert_eq!(dev.pan(ChannelId::Playback(0), 0).unwrap(), 0);
    }

    #[test]
    fn test_volume_clamps_to_range() {
        let mut dev = MockBabyfacePro::open().unwrap();
        dev.set_volume(ChannelId::Input(0), 0, 1.5).unwrap();
        assert!((dev.volume(ChannelId::Input(0), 0).unwrap() - 1.0).abs() < 1e-6);
        dev.set_volume(ChannelId::Input(0), 0, -0.5).unwrap();
        assert!((dev.volume(ChannelId::Input(0), 0).unwrap() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_invalid_channel_returns_error() {
        let mut dev = MockBabyfacePro::open().unwrap();
        let result = dev.set_volume(ChannelId::Input(99), 0, 0.5);
        assert!(result.is_err());
        let result = dev.set_volume(ChannelId::Input(0), 99, 0.5);
        assert!(result.is_err());
    }

    #[test]
    fn test_capture_and_apply_scene() {
        let mut dev = MockBabyfacePro::open().unwrap();
        dev.set_volume(ChannelId::Input(0), 0, 0.25).unwrap();
        dev.set_pan(ChannelId::Playback(0), 1, 50).unwrap();

        let scene = dev.capture_scene();
        assert!((scene.inputs[0].volumes[0] - 0.25).abs() < 1e-6);
        assert_eq!(scene.playbacks[0].pans[1], 50);

        let mut dev2 = MockBabyfacePro::open().unwrap();
        dev2.apply_scene(&scene).unwrap();
        assert!((dev2.volume(ChannelId::Input(0), 0).unwrap() - 0.25).abs() < 1e-6);
        assert_eq!(dev2.pan(ChannelId::Playback(0), 1).unwrap(), 50);
    }

    #[test]
    fn test_poll_events_updates_meters() {
        let mut dev = MockBabyfacePro::open().unwrap();
        for v in dev.input_meters() {
            assert_eq!(*v, 0.0);
        }
        dev.poll_events().unwrap();
        let has_movement = dev.input_meters().iter().any(|v| *v > 0.0);
        assert!(has_movement);
    }

    #[test]
    fn test_meter_ranges() {
        let mut dev = MockBabyfacePro::open().unwrap();
        for _ in 0..100 {
            dev.poll_events().unwrap();
        }
        for v in dev.input_meters() {
            assert!(*v >= 0.0 && *v <= 1.0, "Meter {} out of range", v);
        }
        for v in dev.playback_meters() {
            assert!(*v >= 0.0 && *v <= 1.0, "Meter {} out of range", v);
        }
    }

    #[test]
    fn test_mute_solo_toggle() {
        let mut dev = MockBabyfacePro::open().unwrap();
        assert!(!dev.mute(ChannelId::Input(0)).unwrap());
        assert!(!dev.solo(ChannelId::Playback(0)).unwrap());

        dev.set_mute(ChannelId::Input(0), true).unwrap();
        assert!(dev.mute(ChannelId::Input(0)).unwrap());
        assert!(!dev.mute(ChannelId::Input(1)).unwrap()); // other channel unchanged

        dev.set_solo(ChannelId::Playback(0), true).unwrap();
        assert!(dev.solo(ChannelId::Playback(0)).unwrap());

        dev.set_mute(ChannelId::Input(99), true).unwrap_err();
        dev.set_solo(ChannelId::Playback(99), true).unwrap_err();
    }

    #[test]
    fn test_mute_solo_in_scene() {
        let mut dev = MockBabyfacePro::open().unwrap();
        dev.set_mute(ChannelId::Input(0), true).unwrap();
        dev.set_solo(ChannelId::Playback(0), true).unwrap();

        let scene = dev.capture_scene();
        assert!(scene.inputs[0].mute);
        assert!(scene.playbacks[0].solo);

        let mut dev2 = MockBabyfacePro::open().unwrap();
        dev2.apply_scene(&scene).unwrap();
        assert!(dev2.mute(ChannelId::Input(0)).unwrap());
        assert!(dev2.solo(ChannelId::Playback(0)).unwrap());
    }

    #[test]
    fn test_serialize_scene_to_json() {
        let dev = MockBabyfacePro::open().unwrap();
        let scene = dev.capture_scene();
        let json = scene.to_json().unwrap();
        let restored = Scene::from_json(&json).unwrap();
        assert_eq!(restored.inputs.len(), scene.inputs.len());
    }

    #[test]
    fn test_first_inputs_have_phantom() {
        let dev = MockBabyfacePro::open().unwrap();
        assert!(dev.inputs()[0].phantom);
        assert!(dev.inputs()[1].phantom);
        assert!(!dev.inputs()[2].phantom);
    }

    #[test]
    fn test_captured_scene_is_tagged_with_model() {
        let dev = MockBabyfacePro::open().unwrap();
        let scene = dev.capture_scene();
        assert_eq!(scene.model, dev.model_name());
    }

    #[test]
    fn test_apply_scene_rejects_model_mismatch() {
        let mut dev = MockBabyfacePro::open().unwrap();
        let mut scene = dev.capture_scene();
        scene.model = "Some Other RME Device".into();

        let err = dev.apply_scene(&scene).unwrap_err();
        assert!(matches!(err, Error::SceneModelMismatch { .. }));
    }

    #[test]
    fn test_apply_scene_accepts_legacy_scene_with_no_model() {
        let mut dev = MockBabyfacePro::open().unwrap();
        let mut scene = dev.capture_scene();
        scene.model = String::new(); // simulates a scene saved before `model` existed
        scene.inputs[0].volumes[0] = 0.42;

        dev.apply_scene(&scene).unwrap();
        assert!((dev.volume(ChannelId::Input(0), 0).unwrap() - 0.42).abs() < 1e-6);
    }
}
