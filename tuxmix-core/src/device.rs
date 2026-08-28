use crate::channel::{ChannelId, EqBandType, InputChannel, OutputChannel, PlaybackChannel, Sensitivity};
use crate::error::Error;
use crate::scene::Scene;

/// Global device-level settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceSettings {
    /// Current clock source (e.g. "Internal", "AutoSync").
    pub clock_source: String,
    /// Every valid value for `clock_source`, as reported by the
    /// hardware's own enum control (not hardcoded — a different model
    /// could expose a different set). Empty until a real device has
    /// been attached; `#[serde(default)]` for old Scene JSON.
    #[serde(default)]
    pub clock_sources: Vec<String>,
    /// SPDIF optical mode (true = optical, false = coaxial). No
    /// matching ALSA control was found on the Babyface Pro FS (its
    /// SPDIF I/O appears to be optical-only, with no toggle) — this
    /// field is currently unmapped and always `false`.
    pub spdif_optical: bool,
    /// SPDIF emphasis (`IEC958 Emphasis`).
    pub spdif_emphasis: bool,
    /// SPDIF professional/consumer format flag (`IEC958 Pro Mask`).
    pub spdif_professional: bool,
    /// Whether the SPDIF output is actively transmitting (`IEC958
    /// Switch`, shown as `"IEC958"` in the simple-mixer view — the
    /// standard generic ALSA S/PDIF enable switch, not RME-specific).
    /// `#[serde(default)]` so scenes saved before this field existed
    /// still deserialize (same pattern as `Scene::model`).
    #[serde(default)]
    pub spdif_enabled: bool,
    /// Pitch/varispeed in percent (-5..+5). 0 = nominal. Not persisted
    /// (the pitch is a live clock setting; the auto-scene keeps 0).
    #[serde(default, skip_serializing)]
    pub pitch_percent: f32,
    /// MS-proc engaged (the AN2 "side" crosspoints are muted).
    #[serde(default)]
    pub ms_proc: bool,
    /// AN 1>2 engaged (0x17 wIdx=0x1000 flag).
    #[serde(default)]
    pub an12: bool,
    /// Dim engaged on the Phones output (an absolute -20 dB cut,
    /// independent of the Phones master's own volume). See
    /// [`RmeDevice::set_dim`].
    #[serde(default)]
    pub dim: bool,
    /// FX send level in dB (-65..0, None = unset). 0 dB = max send.
    /// `Option` so old scenes (no send) stay untouched on apply.
    #[serde(default)]
    pub fx_send_db: Option<f32>,
    /// Stereo width (-1..+1, 0 = normal). Which strip it affects is TBD.
    #[serde(default)]
    pub width: f32,
    /// Active sample rate in Hz (proprietary USB mode; the ALSA backend
    /// leaves it to the kernel driver). `#[serde(default = …)]` so old
    /// Scene JSON (and scenes from the ALSA backend) still deserialize.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
}

/// Serde default for [`DeviceSettings::sample_rate`].
fn default_sample_rate() -> u32 {
    48_000
}

/// A generic RME audio interface.
///
/// Each implementation maps to a specific hardware model and knows
/// how to discover and control its ALSA mixer elements.
///
/// The device exposes a matrix (submix) mixer: each input and playback
/// channel has its own volume and pan towards every hardware output pair.
pub trait RmeDevice {
    /// Human-readable model name (e.g. "Babyface Pro FS").
    fn model_name(&self) -> &str;

    /// Number of physical stereo output pairs on this device.
    fn output_pair_count(&self) -> usize;

    /// Attempt to detect the device on the ALSA bus and open a mixer handle.
    fn open() -> Result<Self, Error>
    where
        Self: Sized;

    /// Returns a reference to all hardware input channels.
    fn inputs(&self) -> &[InputChannel];

    /// Returns a mutable reference to all hardware input channels.
    fn inputs_mut(&mut self) -> &mut [InputChannel];

    /// Returns a reference to all software playback channels.
    fn playbacks(&self) -> &[PlaybackChannel];

    /// Returns a mutable reference to all software playback channels.
    fn playbacks_mut(&mut self) -> &mut [PlaybackChannel];

    /// Returns a reference to all physical output channels.
    fn outputs(&self) -> &[OutputChannel];

    /// Returns a mutable reference to all physical output channels.
    fn outputs_mut(&mut self) -> &mut [OutputChannel];

    /// Returns the current global device settings.
    fn settings(&self) -> &DeviceSettings;

    /// Returns a mutable reference to the global device settings.
    fn settings_mut(&mut self) -> &mut DeviceSettings;

    // ── Control operations (submix / matrix) ──────────────────────

    /// Set the volume (0.0 – 1.0) for a given channel into a specific output pair.
    fn set_volume(&mut self, channel: ChannelId, output: usize, volume: f32) -> Result<(), Error>;

    /// Get the volume (0.0 – 1.0) for a given channel into a specific output pair.
    fn volume(&self, channel: ChannelId, output: usize) -> Result<f32, Error>;

    /// Set the pan (-100 .. 100) for a given channel into a specific output pair.
    fn set_pan(&mut self, channel: ChannelId, output: usize, pan: i8) -> Result<(), Error>;

    /// Get the pan (-100 .. 100) for a given channel into a specific output pair.
    fn pan(&self, channel: ChannelId, output: usize) -> Result<i8, Error>;

    // ── Mute / Solo ────────────────────────────────────────────────

    /// Set mute state for a channel.
    fn set_mute(&mut self, channel: ChannelId, mute: bool) -> Result<(), Error>;

    /// Get mute state for a channel.
    fn mute(&self, channel: ChannelId) -> Result<bool, Error>;

    /// Set solo state for a channel.
    fn set_solo(&mut self, channel: ChannelId, solo: bool) -> Result<(), Error>;

    /// Get solo state for a channel.
    fn solo(&self, channel: ChannelId) -> Result<bool, Error>;

    // ── Preamp controls (input-only: 48V, pad, gain) ────────────────

    /// Set 48V phantom power for an input. Errors if the input isn't a
    /// Mic channel (the only type with a phantom power switch).
    fn set_phantom(&mut self, idx: usize, on: bool) -> Result<(), Error>;

    /// Set the -20dB pad for an input. Errors if the input type has no
    /// pad switch.
    fn set_pad(&mut self, idx: usize, on: bool) -> Result<(), Error>;

    /// Set the preamp gain (dB, 1 dB steps, see
    /// [`InputChannel::gain_max`]) for an input. Errors if the input
    /// type has no gain control.
    fn set_gain(&mut self, idx: usize, gain: u32) -> Result<(), Error>;

    /// Set the sample clock pitch/varispeed in percent (-5..+5, 0 = nominal).
    /// The 0x1B DDS quad shifts the device clock dynamically (the
    /// actual sample rate moves with the pitch).
    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), Error>;

    /// Set the active sample rate in Hz. The proprietary USB backend
    /// does a mid-session `SET_INTERFACE` + stream restart; the ALSA
    /// backend and the mock leave it to the system (no-op). Errors on
    /// unsupported rates (e.g. 50 kHz).
    fn set_sample_rate(&mut self, rate: u32) -> Result<(), Error> {
        let _ = rate;
        Ok(())
    }

    // ── §9 controls (decoded on Windows 2026-08-23; see PROTOCOL.md) ──
    // These default to an error so a backend that hasn't mapped them
    // fails loudly instead of silently doing nothing.

    /// Loopback on an output pair (bReq 0x15, one mono channel per side).
    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), Error> {
        let _ = (out, on);
        Err(Error::InvalidChannel(
            "Loopback is not supported on this backend".into(),
        ))
    }

    /// MS-proc engage: mutes the AN2 crosspoints, restores the saved
    /// fader on disengage.
    fn set_ms_proc(&mut self, on: bool) -> Result<(), Error> {
        let _ = on;
        Err(Error::InvalidChannel(
            "MS proc is not supported on this backend".into(),
        ))
    }

    /// AN 1>2 toggle (0x17 wIdx=0x1000 flag).
    fn set_an12(&mut self, on: bool) -> Result<(), Error> {
        let _ = on;
        Err(Error::InvalidChannel(
            "AN 1>2 is not supported on this backend".into(),
        ))
    }

    /// Dim the Phones output by a fixed -20 dB, independent of its
    /// current master volume (disengage restores the pre-dim level).
    fn set_dim(&mut self, on: bool) -> Result<(), Error> {
        let _ = on;
        Err(Error::InvalidChannel(
            "Dim is not supported on this backend".into(),
        ))
    }

    /// Input-strip stereo link (the AN1/2 pair): `true` = linked (the
    /// default TotalMix state — gains/48V move together), `false` =
    /// split into individual AN1/AN2 buses. USB backend writes the
    /// 0x17 wIdx=0x1000 flag; ALSA/mock are no-ops.
    fn set_input_link(&mut self, linked: bool) -> Result<(), Error> {
        let _ = linked;
        Ok(())
    }

    /// Trim (T) for an input channel, dB on the master curve
    /// (-65..+6, 0 = 0x2000; cap_trim.pcap). USB backend writes the
    /// low map (trim) + the standard map (fader × trim, cap_trim2 —
    /// the combined dB); ALSA/mock no-op.
    fn set_trim(&mut self, idx: usize, db: f32) -> Result<(), Error> {
        let _ = (idx, db);
        Ok(())
    }

    /// Phase Ø invert for an input (bitwise-NOT of its crosspoints).
    fn set_phase(&mut self, idx: usize, invert: bool) -> Result<(), Error> {
        let _ = (idx, invert);
        Err(Error::InvalidChannel(
            "Phase is not supported on this backend".into(),
        ))
    }

    /// FX send level in dB (-65..0, 0 = max send).
    fn set_fx_send(&mut self, db: f32) -> Result<(), Error> {
        let _ = db;
        Err(Error::InvalidChannel(
            "FX send is not supported on this backend".into(),
        ))
    }

    /// Stereo-split a playback strip (its AN1/2 crosspoints are
    /// rewritten to 0x2000/0x0000 split-mono instead of the -6 dB pair).
    fn set_stereo_split(&mut self, pb: usize, split: bool) -> Result<(), Error> {
        let _ = (pb, split);
        Err(Error::InvalidChannel(
            "Stereo split is not supported on this backend".into(),
        ))
    }

    /// Stereo width (-1..+1, 0 = normal).
    fn set_width(&mut self, width: f32) -> Result<(), Error> {
        let _ = width;
        Err(Error::InvalidChannel(
            "Width is not supported on this backend".into(),
        ))
    }

    /// Ref level code (Instr 3/4) as the raw state word (bits 2-3).
    fn set_ref_level(&mut self, idx: usize, raw: u16) -> Result<(), Error> {
        let _ = (idx, raw);
        Err(Error::InvalidChannel(
            "Ref level is not supported on this backend".into(),
        ))
    }

    // ── Hardware DSP EQ (3-band + low cut, analog inputs only) ──────
    // The device DSP does the biquad math — these just carry the plain
    // freq/Q/gain/type parameters, see `channel::InputEq`.

    /// Enable/bypass an input's EQ strip. Errors if the input has no
    /// EQ (only the 4 analog inputs do — see [`InputChannel::eq`]).
    fn set_eq_enabled(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        let _ = (idx, on);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set one band's filter type (`band` is 0-2 for bands 1-3).
    fn set_eq_band_type(
        &mut self,
        idx: usize,
        band: usize,
        band_type: EqBandType,
    ) -> Result<(), Error> {
        let _ = (idx, band, band_type);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set one band's center/corner frequency (20-20000 Hz).
    fn set_eq_band_freq(&mut self, idx: usize, band: usize, freq_hz: u16) -> Result<(), Error> {
        let _ = (idx, band, freq_hz);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set one band's Q factor (0.05-10.0).
    fn set_eq_band_q(&mut self, idx: usize, band: usize, q: f32) -> Result<(), Error> {
        let _ = (idx, band, q);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set one band's gain (-24.0..+24.0 dB).
    fn set_eq_band_gain(&mut self, idx: usize, band: usize, gain_db: f32) -> Result<(), Error> {
        let _ = (idx, band, gain_db);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set the low-cut filter's corner frequency (20-20000 Hz).
    fn set_eq_low_cut_freq(&mut self, idx: usize, freq_hz: u16) -> Result<(), Error> {
        let _ = (idx, freq_hz);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set the low-cut filter's slope (6, 12, 18, or 24 dB/octave).
    fn set_eq_low_cut_slope(&mut self, idx: usize, slope_db_oct: u8) -> Result<(), Error> {
        let _ = (idx, slope_db_oct);
        Err(Error::InvalidChannel(
            "EQ is not supported on this backend".into(),
        ))
    }

    /// Set input sensitivity (+4dBu / -10dBV). Errors if the input
    /// type has no sensitivity switch (only Instrument inputs do).
    fn set_sensitivity(&mut self, idx: usize, sensitivity: Sensitivity) -> Result<(), Error>;

    /// Enable/disable the SPDIF output (`IEC958 Switch`).
    fn set_spdif_enabled(&mut self, enabled: bool) -> Result<(), Error>;

    /// Set SPDIF emphasis (`IEC958 Emphasis`).
    fn set_spdif_emphasis(&mut self, enabled: bool) -> Result<(), Error>;

    /// Set the SPDIF professional/consumer format flag (`IEC958 Pro Mask`).
    fn set_spdif_professional(&mut self, enabled: bool) -> Result<(), Error>;

    /// Set the sample clock source. `source` must be one of
    /// `DeviceSettings::clock_sources`; errors otherwise.
    fn set_clock_source(&mut self, source: &str) -> Result<(), Error>;

    // ── Scene / snapshot ────────────────────────────────────────

    /// Read the full hardware state into a [`Scene`].
    fn capture_scene(&self) -> Scene;

    /// Apply a previously captured [`Scene`] to the hardware.
    fn apply_scene(&mut self, scene: &Scene) -> Result<(), Error>;

    // ── Polling ─────────────────────────────────────────────────

    /// Process pending ALSA events (e.g. hardware state changes).
    /// Should be called periodically from the UI event loop.
    fn poll_events(&mut self) -> Result<(), Error>;

    // ── Meters ───────────────────────────────────────────────────

    /// Current input meter levels (0.0-1.0 of full scale, one entry per
    /// hardware input channel), or `None` if the backend provides no
    /// meters (e.g. the ALSA class-compliant path).
    ///
    /// Draining: each call returns the levels accumulated since the
    /// previous call, so the UI should poll it **once per tick** (not
    /// once per channel) — a second call in the same tick returns
    /// zeros.
    fn meters(&self) -> Option<Vec<f32>> {
        None
    }
}
