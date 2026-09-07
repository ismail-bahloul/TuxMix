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
    /// EQ for Record: whether the hardware DSP EQ is applied to the
    /// recorded signal too, not just the monitor mix. Settings
    /// keepalive bit 6 (`PROTOCOL.md`'s `cap_eqr.pcap`, hardware-
    /// verified) — an RME-specific setting, distinct from the generic
    /// `spdif_*` fields above.
    #[serde(default)]
    pub eq_for_record: bool,
    /// Optical Out format: `true` = SPDIF (2-channel), `false` = ADAT
    /// (8-channel, the device default). Settings keepalive bit 10
    /// (`PROTOCOL.md`'s `cap_opt.pcap`, hardware-verified) — an
    /// RME-specific setting for the single physical optical port,
    /// distinct from the generic `spdif_*` fields above (which target
    /// the standard ALSA S/PDIF control surface this device doesn't
    /// expose).
    #[serde(default)]
    pub optical_out_spdif: bool,
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
    /// Input-strip stereo link mirror, kept in sync by
    /// [`RmeDevice::set_input_link`] so it's readable (the trait itself
    /// only has a setter — this is where the UI reads the current state
    /// from). `true` = linked, TotalMix's default.
    #[serde(default = "default_input_link")]
    pub input_link: bool,
    /// Per-output-pair stereo link state — index = submix pair (same
    /// indexing as [`RmeDevice::output_pair_count`]/`OUT_LABELS`).
    /// Missing/short index = linked (TotalMix's default for every pair),
    /// not a plain `#[serde(default)]` empty-vec special case — see
    /// [`RmeDevice::output_linked`]. Mirrors a real TotalMix behavior
    /// confirmed against hardware captures: the stereo-link toggle itself
    /// writes nothing to the device (it's a pure UI/model state); only
    /// the *next* volume/mute/solo write on either channel of the pair is
    /// affected by whether it's linked.
    #[serde(default)]
    pub output_link: Vec<bool>,
    /// Per-input-pair stereo link state — generic display/control
    /// grouping (one fader/mute/solo for the pair vs two independent
    /// channels), same shape and default as [`Self::output_link`] — see
    /// [`RmeDevice::input_pair_linked`]. Deliberately separate from
    /// [`Self::input_link`]: that field mirrors the AN1/2-specific
    /// hardware preamp-link flag (48V/gain bits physically written
    /// together for that one pair); this one is the same pure-UI
    /// grouping concept `output_link` uses, and applies to every input
    /// pair, not just AN1/2 — TotalMix shows a stereo button on every
    /// strip, not only the mic pair.
    #[serde(default)]
    pub input_pair_link: Vec<bool>,
}

/// Serde default for [`DeviceSettings::sample_rate`].
fn default_sample_rate() -> u32 {
    48_000
}

/// Serde default for [`DeviceSettings::input_link`] (TotalMix's own
/// default is linked).
fn default_input_link() -> bool {
    true
}

/// A generic RME audio interface.
///
/// Each implementation maps to a specific hardware model and knows
/// how to discover and control its ALSA mixer elements.
///
/// The device exposes a matrix (submix) mixer: each input and playback
/// channel has its own volume and pan towards every hardware output pair.
pub trait RmeDevice {
    /// Human-readable model name (e.g. "Babyface Pro FS"), decorated per
    /// backend for display in the UI (e.g. "Babyface Pro FS (USB)",
    /// "Babyface Pro FS (mock)") — NOT what scene compatibility should be
    /// judged on, see [`Self::canonical_model`].
    fn model_name(&self) -> &str;

    /// Model identity used for [`Scene::check_compatible`] — deliberately
    /// separate from [`Self::model_name`]. Two backends that talk to the
    /// same physical device (e.g. the ALSA kernel-driver path and the
    /// direct-USB path both controlling a Babyface Pro FS) should be able
    /// to restore each other's saved scenes; only genuinely different
    /// hardware should be rejected. Defaults to [`Self::model_name`] for
    /// backends where the two don't need to differ (e.g. the mock, whose
    /// captures are never persisted to the shared auto-scene anyway).
    fn canonical_model(&self) -> &str {
        self.model_name()
    }

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

    /// Sets input `idx`'s phantom power, respecting its pair's link
    /// state — same idea as [`Self::set_input_mute`]/[`Self::
    /// set_input_volume`], but `idx` alone is enough here (no separate
    /// `pair`/`which`): a linked pair only ever exposes ONE strip (the
    /// left/even channel), so the caller never needs to disambiguate
    /// which side was clicked the way Mute/Solo/Volume's own callers
    /// do when driven from a bare pair index.
    ///
    /// Previously missing entirely — every caller called [`Self::
    /// set_phantom`] directly, so toggling 48V on a linked strip only
    /// ever touched its own (left) channel, silently leaving the right
    /// channel's phantom power wherever it last was — found 2026-09-07
    /// (the user noticed AN1/2's 48V toggle wasn't actually affecting
    /// both physical inputs).
    fn set_input_phantom(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        self.set_phantom(idx, on)?;
        let pair = idx / 2;
        if self.input_pair_linked(pair) {
            self.set_phantom(idx ^ 1, on)?;
        }
        Ok(())
    }

    /// Same idea as [`Self::set_input_phantom`], for the -20dB pad.
    fn set_input_pad(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        self.set_pad(idx, on)?;
        let pair = idx / 2;
        if self.input_pair_linked(pair) {
            self.set_pad(idx ^ 1, on)?;
        }
        Ok(())
    }

    /// Same idea as [`Self::set_input_phantom`], for preamp gain.
    fn set_input_gain(&mut self, idx: usize, gain: u32) -> Result<(), Error> {
        self.set_gain(idx, gain)?;
        let pair = idx / 2;
        if self.input_pair_linked(pair) {
            self.set_gain(idx ^ 1, gain)?;
        }
        Ok(())
    }

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

    /// EQ for Record toggle (settings keepalive bit 6).
    fn set_eq_for_record(&mut self, on: bool) -> Result<(), Error> {
        let _ = on;
        Err(Error::InvalidChannel(
            "EQ for Record is not supported on this backend".into(),
        ))
    }

    /// Optical Out format: `true` = SPDIF, `false` = ADAT (settings
    /// keepalive bit 10).
    fn set_optical_out_format(&mut self, spdif: bool) -> Result<(), Error> {
        let _ = spdif;
        Err(Error::InvalidChannel(
            "Optical Out format is not supported on this backend".into(),
        ))
    }

    /// CUE: exclusively monitors output `idx`'s own dedicated playback
    /// pair through the AN1/2 bus, muting every other playback pair
    /// there (`on = true`); `false` restores the normal monitor mix.
    /// Exclusive across every output, same as solo — engaging CUE on
    /// one output silently disengages whichever other output had it.
    fn set_cue(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        let _ = (idx, on);
        Err(Error::InvalidChannel(
            "CUE is not supported on this backend".into(),
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

    // ── Output pair stereo link (bus grouping) ──────────────────────
    // TotalMix shows each hardware output pair as ONE linked stereo bus
    // by default (one fader/mute/solo drives both channels); the
    // "stereo" button on that bus splits it into two independently
    // controllable mono channels. Confirmed against hardware captures
    // that the toggle itself never writes to the device — only
    // `set_output_volume`/`set_output_mute`/`set_output_solo` below,
    // called for a fader/mute/solo action on the strip, need to know the
    // current link state. Assumes `ChannelId::Output(pair * 2 + which)`
    // addressing (one physical channel each), so every backend's
    // `outputs()` must lay out pairs contiguously in that order.

    /// Whether output pair `pair` (submix index, 0-based) is currently a
    /// linked stereo bus. `true` (linked) if `pair` has no explicit entry
    /// in `DeviceSettings::output_link` yet — TotalMix's own default.
    fn output_linked(&self, pair: usize) -> bool {
        self.settings().output_link.get(pair).copied().unwrap_or(true)
    }

    /// Sets whether output pair `pair` is linked. Pure model state — see
    /// this section's doc comment; no backend needs to override this.
    fn set_output_linked(&mut self, pair: usize, linked: bool) -> Result<(), Error> {
        let v = &mut self.settings_mut().output_link;
        if v.len() <= pair {
            v.resize(pair + 1, true);
        }
        v[pair] = linked;
        Ok(())
    }

    /// Sets output pair `pair`'s volume — the entry point the UI should
    /// use for an output-strip fader drag instead of calling
    /// [`Self::set_volume`] directly. Linked: writes both channels of
    /// the pair to `volume`. Split: writes only `which` (0 = the pair's
    /// first/left channel, 1 = the second/right one).
    fn set_output_volume(&mut self, pair: usize, which: usize, volume: f32) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.output_linked(pair) {
            self.set_volume(ChannelId::Output(l), 0, volume)?;
            self.set_volume(ChannelId::Output(r), 0, volume)?;
        } else {
            self.set_volume(ChannelId::Output(if which == 0 { l } else { r }), 0, volume)?;
        }
        Ok(())
    }

    /// Sets output pair `pair`'s mute, respecting its link state — same
    /// shape as [`Self::set_output_volume`].
    fn set_output_mute(&mut self, pair: usize, which: usize, mute: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.output_linked(pair) {
            self.set_mute(ChannelId::Output(l), mute)?;
            self.set_mute(ChannelId::Output(r), mute)?;
        } else {
            self.set_mute(ChannelId::Output(if which == 0 { l } else { r }), mute)?;
        }
        Ok(())
    }

    /// Sets output pair `pair`'s solo, respecting its link state — same
    /// shape as [`Self::set_output_volume`].
    fn set_output_solo(&mut self, pair: usize, which: usize, solo: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.output_linked(pair) {
            self.set_solo(ChannelId::Output(l), solo)?;
            self.set_solo(ChannelId::Output(r), solo)?;
        } else {
            self.set_solo(ChannelId::Output(if which == 0 { l } else { r }), solo)?;
        }
        Ok(())
    }

    // ── Input pair stereo link (bus grouping) ───────────────────────
    // Same generic display/control grouping as the output section above
    // — TotalMix shows a stereo-link button on every strip, not only the
    // AN1/2 mic pair. Deliberately separate from `input_link`/
    // `set_input_link`, which is the AN1/2-specific hardware preamp-link
    // flag (48V/gain bits physically written together for that one
    // pair) — this is the same pure-UI grouping `output_link` uses,
    // generalized to every input pair.

    /// Whether input pair `pair` is currently a linked stereo bus —
    /// same default-linked-if-unset semantics as [`Self::output_linked`].
    fn input_pair_linked(&self, pair: usize) -> bool {
        self.settings()
            .input_pair_link
            .get(pair)
            .copied()
            .unwrap_or(true)
    }

    /// Sets whether input pair `pair` is linked. Pure model state, same
    /// as [`Self::set_output_linked`].
    fn set_input_pair_linked(&mut self, pair: usize, linked: bool) -> Result<(), Error> {
        let v = &mut self.settings_mut().input_pair_link;
        if v.len() <= pair {
            v.resize(pair + 1, true);
        }
        v[pair] = linked;
        Ok(())
    }

    /// Sets input pair `pair`'s volume (into submix `output`), respecting
    /// its link state — same shape as [`Self::set_output_volume`], with
    /// an extra `output` (submix bus) parameter since inputs route into
    /// one of several buses, unlike an output's single master.
    fn set_input_volume(
        &mut self,
        pair: usize,
        which: usize,
        output: usize,
        volume: f32,
    ) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.input_pair_linked(pair) {
            self.set_volume(ChannelId::Input(l), output, volume)?;
            self.set_volume(ChannelId::Input(r), output, volume)?;
        } else {
            self.set_volume(ChannelId::Input(if which == 0 { l } else { r }), output, volume)?;
        }
        Ok(())
    }

    /// Sets input pair `pair`'s mute, respecting its link state.
    fn set_input_mute(&mut self, pair: usize, which: usize, mute: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.input_pair_linked(pair) {
            self.set_mute(ChannelId::Input(l), mute)?;
            self.set_mute(ChannelId::Input(r), mute)?;
        } else {
            self.set_mute(ChannelId::Input(if which == 0 { l } else { r }), mute)?;
        }
        Ok(())
    }

    /// Sets input pair `pair`'s solo, respecting its link state.
    fn set_input_solo(&mut self, pair: usize, which: usize, solo: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.input_pair_linked(pair) {
            self.set_solo(ChannelId::Input(l), solo)?;
            self.set_solo(ChannelId::Input(r), solo)?;
        } else {
            self.set_solo(ChannelId::Input(if which == 0 { l } else { r }), solo)?;
        }
        Ok(())
    }

    // ── Playback pair stereo link (bus grouping) ────────────────────
    // Same concept again, but there's no new settings field: a playback
    // pair's link state IS `PlaybackChannel::split`, inverted (`split`
    // already exists, per-channel, and is the real state
    // `RmeDevice::set_stereo_split` writes — duplicating it in
    // `DeviceSettings` would just risk the two disagreeing).

    /// Whether playback pair `pair` is currently linked — reads
    /// `!playbacks[pair*2].split` (both channels of a pair always carry
    /// the same flag, see `set_stereo_split`'s own implementations).
    fn playback_linked(&self, pair: usize) -> bool {
        !self
            .playbacks()
            .get(pair * 2)
            .map(|p| p.split)
            .unwrap_or(false)
    }

    /// Sets whether playback pair `pair` is linked — the inverse of
    /// [`Self::set_stereo_split`], which already writes both channels.
    fn set_playback_linked(&mut self, pair: usize, linked: bool) -> Result<(), Error> {
        self.set_stereo_split(pair * 2, !linked)
    }

    /// Sets playback pair `pair`'s volume (into submix `output`),
    /// respecting its link state — same shape as
    /// [`Self::set_input_volume`].
    fn set_playback_volume(
        &mut self,
        pair: usize,
        which: usize,
        output: usize,
        volume: f32,
    ) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.playback_linked(pair) {
            self.set_volume(ChannelId::Playback(l), output, volume)?;
            self.set_volume(ChannelId::Playback(r), output, volume)?;
        } else {
            self.set_volume(ChannelId::Playback(if which == 0 { l } else { r }), output, volume)?;
        }
        Ok(())
    }

    /// Sets playback pair `pair`'s mute, respecting its link state.
    fn set_playback_mute(&mut self, pair: usize, which: usize, mute: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.playback_linked(pair) {
            self.set_mute(ChannelId::Playback(l), mute)?;
            self.set_mute(ChannelId::Playback(r), mute)?;
        } else {
            self.set_mute(ChannelId::Playback(if which == 0 { l } else { r }), mute)?;
        }
        Ok(())
    }

    /// Sets playback pair `pair`'s solo, respecting its link state.
    fn set_playback_solo(&mut self, pair: usize, which: usize, solo: bool) -> Result<(), Error> {
        let (l, r) = (pair * 2, pair * 2 + 1);
        if self.playback_linked(pair) {
            self.set_solo(ChannelId::Playback(l), solo)?;
            self.set_solo(ChannelId::Playback(r), solo)?;
        } else {
            self.set_solo(ChannelId::Playback(if which == 0 { l } else { r }), solo)?;
        }
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
