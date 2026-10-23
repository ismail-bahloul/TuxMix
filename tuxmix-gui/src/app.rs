use iced::futures::channel::mpsc;
use iced::keyboard::{self, Key};
use iced::widget::{
    button, column, container, mouse_area, opaque, pick_list, responsive, row, scrollable, stack,
    text,
};
use iced::{window, Color, Element, Length, Subscription, Task};

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use tuxmix_core::channel::EqBandType;
#[cfg(feature = "alsa")]
use tuxmix_core::BabyfacePro;
use tuxmix_core::{
    BabyfaceProUsb, ChannelId, ChannelType, MockBabyfacePro, RmeDevice, Scene, Sensitivity,
};

use crate::about;
use crate::matrix;
use crate::osc::{self, OscCommand, OscConfig, OscOutbound};
use tuxmix_core::pw_clock::{self, CardStream, PwClock};
use tuxmix_core::pw_meter;
use crate::scenes::{load_scene_file, save_scene_file};
use crate::sidebar::{self, Group};
use crate::theme;
use crate::widgets::eq_plot::{eq_plot, EqPlotActions};
use crate::widgets::fader;
use crate::widgets::knob::{knob, Knob};
use crate::widgets::strip::{self, hint};

pub const OUT_LABELS: [&str; 6] = ["AN1/2", "PH3/4", "AS1/2", "A3/A4", "A5/A6", "A7/A8"];

/// Strips redundant prefixes baked into a channel's stored name before it's
/// shown in a strip header — "PCM " for playback channels, "OUT " for
/// output bus pairs (`profile.rs`'s `format!("OUT {}", pair.left)`). Both
/// are already conveyed by the strip's own type tag badge right next to
/// the name, so keeping them in the text just wastes width for no extra
/// information — on an output like "OUT ADAT3" that was enough to push the
/// collapse button (`header_row`) almost entirely off the card.
pub fn short_label(name: &str) -> &str {
    name.strip_prefix("PCM ")
        .or_else(|| name.strip_prefix("OUT "))
        .unwrap_or(name)
}

/// Combines a linked output pair's two per-channel names ("AN1"/"AN2",
/// "PH3"/"PH4", "ADAT7"/"ADAT8") into the historical combined bus label
/// ("AN1/2", "PH3/4", "ADAT7/8") — matches what `tuxmix-usb`'s output
/// list used before it became per-channel (see `usb.rs`'s `open()`),
/// and what TotalMix itself shows for a linked stereo bus. Falls back to
/// `"{left}/{right}"` unmodified if `right` doesn't end in digits (not
/// expected for this device's own channel names, but harmless).
pub(crate) fn pair_bus_label(left: &str, right: &str) -> String {
    let right_num: String = right.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    if right_num.is_empty() {
        format!("{left}/{right}")
    } else {
        format!("{left}/{right_num}")
    }
}

pub fn type_tag(t: ChannelType) -> (&'static str, iced::Color) {
    // The muted type family: quiet enough to live in a graphite UI
    // without fighting the white "engaged" states for attention, but
    // still separable hues for scanning a long strip row. Mic, line and
    // instrument keep their console associations (blue, neutral, amber);
    // the two digital ports share a violet.
    match t {
        ChannelType::Mic => ("MIC", iced::Color::from_rgb8(0x7f, 0xb3, 0xd5)),
        ChannelType::Instrument => ("INST", iced::Color::from_rgb8(0xd9, 0xa8, 0x5c)),
        ChannelType::Line => ("LINE", iced::Color::from_rgb8(0x9d, 0xa3, 0xac)),
        ChannelType::SPDIF => ("SPDIF", iced::Color::from_rgb8(0xa2, 0x92, 0xc6)),
        ChannelType::ADAT => ("ADAT", iced::Color::from_rgb8(0xa2, 0x92, 0xc6)),
    }
}

/// Tag colors for the bus rows (not real `ChannelType`s, so not part of
/// `type_tag`) — kept distinct from the input-type palette above so PB/OUT
/// don't just blend into the secondary-text gray everything else uses.
pub const PB_TAG: iced::Color = iced::Color::from_rgb8(0x5f, 0xb3, 0xa8);
pub const OUT_TAG: iced::Color = iced::Color::from_rgb8(0x7f, 0xbf, 0x8a);

pub fn parse_db_input(s: &str) -> Option<f32> {
    let raw = s.trim().to_lowercase();
    if raw.is_empty() || raw == "-inf" || raw == "-\u{221e}" {
        return Some(0.0);
    }
    raw.replace(',', ".")
        .parse::<f32>()
        .ok()
        .map(|db| (10f32.powf(db / 20.0)).clamp(0.0, 2.0))
}

pub fn db_text(vol: f32) -> String {
    if vol > 0.0 {
        format!("{:.1} dB", 20.0 * vol.log10())
    } else {
        "-\u{221e} dB".into()
    }
}

// ── Device enum ──────────────────────────────────────────────────

pub enum DeviceHandle {
    #[cfg(feature = "alsa")]
    Real(BabyfacePro),
    Mock(MockBabyfacePro),
    /// The proprietary USB backend (the TotalMix protocol). On Linux
    /// with the device in proprietary mode this is the path that
    /// actually works.
    Usb(BabyfaceProUsb),
}

impl DeviceHandle {
    /// Point the *mock* at the class-compliant personality's feature set.
    /// Only tests ever need this: every real backend answers for itself,
    /// and `TUXMIX_MOCK_CC` covers looking at the gated UI by hand.
    #[cfg(test)]
    fn set_protocol_features(&mut self, on: bool) {
        if let DeviceHandle::Mock(dev) = self {
            dev.set_protocol_features(on);
        }
    }

    /// Point the *mock* at "unplugged", or back. Tests only; every real
    /// backend answers for itself, and `TUXMIX_MOCK_DISCONNECT` covers
    /// looking at the disconnected chip by hand.
    #[cfg(test)]
    fn set_connected(&mut self, on: bool) {
        if let DeviceHandle::Mock(dev) = self {
            dev.set_connected(on);
        }
    }
}

macro_rules! delegate {
    ($self:expr, $method:ident($($arg:expr),*)) => { match $self {
        DeviceHandle::Mock(d) => d.$method($($arg),*),
        DeviceHandle::Usb(d) => d.$method($($arg),*),
        #[cfg(feature = "alsa")]
        DeviceHandle::Real(d) => d.$method($($arg),*),
    } };
    ($self:expr, $method:ident) => { match $self {
        DeviceHandle::Mock(d) => d.$method(),
        DeviceHandle::Usb(d) => d.$method(),
        #[cfg(feature = "alsa")]
        DeviceHandle::Real(d) => d.$method(),
    } };
}

impl RmeDevice for DeviceHandle {
    fn model_name(&self) -> &str {
        delegate!(self, model_name)
    }
    fn protocol_features(&self) -> bool {
        delegate!(self, protocol_features)
    }
    fn is_connected(&self) -> bool {
        delegate!(self, is_connected)
    }
    fn output_pair_count(&self) -> usize {
        delegate!(self, output_pair_count)
    }
    fn open() -> Result<Self, tuxmix_core::Error> {
        unreachable!()
    }
    fn inputs(&self) -> &[tuxmix_core::InputChannel] {
        delegate!(self, inputs)
    }
    fn inputs_mut(&mut self) -> &mut [tuxmix_core::InputChannel] {
        delegate!(self, inputs_mut)
    }
    fn playbacks(&self) -> &[tuxmix_core::PlaybackChannel] {
        delegate!(self, playbacks)
    }
    fn playbacks_mut(&mut self) -> &mut [tuxmix_core::PlaybackChannel] {
        delegate!(self, playbacks_mut)
    }
    fn outputs(&self) -> &[tuxmix_core::OutputChannel] {
        delegate!(self, outputs)
    }
    fn outputs_mut(&mut self) -> &mut [tuxmix_core::OutputChannel] {
        delegate!(self, outputs_mut)
    }
    fn settings(&self) -> &tuxmix_core::DeviceSettings {
        delegate!(self, settings)
    }
    fn settings_mut(&mut self) -> &mut tuxmix_core::DeviceSettings {
        delegate!(self, settings_mut)
    }
    fn set_volume(&mut self, ch: ChannelId, out: usize, v: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_volume(ch, out, v))
    }
    fn volume(&self, ch: ChannelId, out: usize) -> Result<f32, tuxmix_core::Error> {
        delegate!(self, volume(ch, out))
    }
    fn set_pan(&mut self, ch: ChannelId, out: usize, p: i8) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_pan(ch, out, p))
    }
    fn pan(&self, ch: ChannelId, out: usize) -> Result<i8, tuxmix_core::Error> {
        delegate!(self, pan(ch, out))
    }
    fn set_mute(&mut self, ch: ChannelId, m: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_mute(ch, m))
    }
    fn mute(&self, ch: ChannelId) -> Result<bool, tuxmix_core::Error> {
        delegate!(self, mute(ch))
    }
    fn set_solo(&mut self, ch: ChannelId, s: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_solo(ch, s))
    }
    fn solo(&self, ch: ChannelId) -> Result<bool, tuxmix_core::Error> {
        delegate!(self, solo(ch))
    }
    fn set_phantom(&mut self, idx: usize, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_phantom(idx, on))
    }
    fn set_pad(&mut self, idx: usize, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_pad(idx, on))
    }
    fn set_gain(&mut self, idx: usize, gain: u32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_gain(idx, gain))
    }
    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_pitch(pitch_percent))
    }
    fn set_sensitivity(
        &mut self,
        idx: usize,
        sensitivity: tuxmix_core::Sensitivity,
    ) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_sensitivity(idx, sensitivity))
    }
    fn set_clock_source(&mut self, source: &str) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_clock_source(source))
    }
    fn set_sample_rate(&mut self, rate: u32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_sample_rate(rate))
    }
    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_loopback(out, on))
    }
    fn set_ms_proc(&mut self, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_ms_proc(on))
    }
    fn set_an12(&mut self, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_an12(on))
    }
    fn set_dim(&mut self, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_dim(on))
    }
    fn set_eq_for_record(&mut self, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_for_record(on))
    }
    fn set_optical_out_format(&mut self, spdif: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_optical_out_format(spdif))
    }
    fn set_cue(&mut self, idx: usize, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_cue(idx, on))
    }
    fn set_input_link(&mut self, linked: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_input_link(linked))
    }
    fn set_trim(&mut self, idx: usize, db: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_trim(idx, db))
    }
    fn set_phase(&mut self, idx: usize, invert: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_phase(idx, invert))
    }
    fn set_fx_send(&mut self, db: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_fx_send(db))
    }
    fn set_stereo_split(&mut self, pb: usize, split: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_stereo_split(pb, split))
    }
    fn set_width(&mut self, width: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_width(width))
    }
    fn set_ref_level(&mut self, idx: usize, raw: u16) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_ref_level(idx, raw))
    }
    fn set_eq_enabled(&mut self, idx: usize, on: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_enabled(idx, on))
    }
    fn set_eq_band_type(
        &mut self,
        idx: usize,
        band: usize,
        band_type: tuxmix_core::channel::EqBandType,
    ) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_band_type(idx, band, band_type))
    }
    fn set_eq_band_freq(
        &mut self,
        idx: usize,
        band: usize,
        freq_hz: u16,
    ) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_band_freq(idx, band, freq_hz))
    }
    fn set_eq_band_q(&mut self, idx: usize, band: usize, q: f32) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_band_q(idx, band, q))
    }
    fn set_eq_band_gain(
        &mut self,
        idx: usize,
        band: usize,
        gain_db: f32,
    ) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_band_gain(idx, band, gain_db))
    }
    fn set_eq_low_cut_freq(&mut self, idx: usize, freq_hz: u16) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_low_cut_freq(idx, freq_hz))
    }
    fn set_eq_low_cut_slope(
        &mut self,
        idx: usize,
        slope_db_oct: u8,
    ) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_eq_low_cut_slope(idx, slope_db_oct))
    }
    fn capture_scene(&self) -> Scene {
        delegate!(self, capture_scene)
    }
    fn apply_scene(&mut self, s: &Scene) -> Result<(), tuxmix_core::Error> {
        delegate!(self, apply_scene(s))
    }
    fn poll_events(&mut self) -> Result<(), tuxmix_core::Error> {
        delegate!(self, poll_events)
    }
}

impl DeviceHandle {
    /// Opens the real hardware. `backend` forces a specific path
    /// (`"alsa"` or `"usb"`) — anything else (including `None`) auto-
    /// detects: ALSA (the kernel driver) first, falling back to the
    /// direct USB backend if the kernel driver isn't loaded. The USB
    /// backend refuses to open while the kernel driver owns the device
    /// (see `tuxmix-usb`), so the two paths are mutually exclusive —
    /// auto-detect never double-opens, it just wasn't ever *said* which
    /// one won. Every path now logs that at `info` level so a failure
    /// to find the device isn't a silent "which one did it even try?".
    pub fn open_real(backend: Option<&str>) -> Option<Self> {
        #[cfg_attr(not(feature = "alsa"), allow(unused_variables))]
        let want_alsa = !matches!(backend, Some("usb"));
        let want_usb = !matches!(backend, Some("alsa"));

        #[cfg(feature = "alsa")]
        if want_alsa {
            match BabyfacePro::open() {
                Ok(d) => {
                    log::info!("Device backend: ALSA (kernel driver)");
                    return Some(DeviceHandle::Real(d));
                }
                Err(e) if backend == Some("alsa") => {
                    log::warn!("--backend alsa requested but ALSA open failed: {e:?}");
                    return None;
                }
                // The card was found, it's just in a mode this backend
                // can't drive — user-actionable, so say it on stderr
                // instead of falling through to the generic (and here
                // simply wrong) "No device found".
                Err(e @ tuxmix_core::Error::UnsupportedDeviceMode { .. }) => {
                    eprintln!("{e}");
                }
                Err(_) => {}
            }
        }
        #[cfg(not(feature = "alsa"))]
        if backend == Some("alsa") {
            log::warn!("--backend alsa requested but this build has no `alsa` feature");
            return None;
        }

        if want_usb {
            match BabyfaceProUsb::open() {
                Ok(d) => {
                    log::info!("Device backend: USB (libusb)");
                    return Some(DeviceHandle::Usb(d));
                }
                Err(e) if backend == Some("usb") => {
                    log::warn!("--backend usb requested but USB open failed: {e:?}");
                }
                Err(_) => {}
            }
        }
        None
    }
    pub fn open_mock() -> Self {
        DeviceHandle::Mock(MockBabyfacePro::open().expect("mock opens"))
    }
    /// All input meter levels in one call (the USB backend's
    /// `meters()` is draining — call once per tick, not per channel).
    pub fn input_meters(&self) -> Vec<f32> {
        let n = self.inputs().len();
        match self {
            DeviceHandle::Mock(d) => (0..n).map(|i| d.input_meter(i)).collect(),
            DeviceHandle::Usb(d) => d.meters().unwrap_or_else(|| vec![0.0; n]),
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(d) => d.meters().unwrap_or_else(|| vec![0.0; n]),
        }
    }
    pub fn playback_meters(&self) -> Vec<f32> {
        let n = self.playbacks().len();
        match self {
            DeviceHandle::Mock(d) => (0..n).map(|i| d.playback_meter(i)).collect(),
            // Playback meters come from the OUT stream — not wired yet.
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => vec![0.0; n],
            DeviceHandle::Usb(_) => vec![0.0; n],
        }
    }
    /// Per-channel availability for the input strips. Mock is real
    /// everywhere; the USB backend's ISO IN stream meters AN1-4 only
    /// (`tuxmix-usb`'s `input_peaks()` returns four channels); the
    /// ALSA/kernel-driver backend meters all twelve inputs in the
    /// device's own order (AN1-4, AS1/2, ADAT3-8) with the v7 driver,
    /// see `capture_meter`'s module doc comment), provided the PCM
    /// opened (`BabyfacePro::meters_available`). An older driver puts
    /// AS1/2 at 10/11 instead, so TuxMix expects the updated driver.
    /// Each capture pair is also the matching
    /// output pair's record bus, so enabling that output's loopback
    /// makes the input meter read the output bus too.
    pub fn has_input_meter(&self, idx: usize) -> bool {
        match self {
            DeviceHandle::Mock(_) => true,
            DeviceHandle::Usb(_) => idx < 4,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(d) => idx < d.inputs().len() && d.meters_available(),
        }
    }
    /// Whether `playback_meters()` is real — true only for Mock. The USB
    /// backend runs its ISO OUT stream in meter-only (silence) mode, so it
    /// never sees real playback audio to compute a level from even though
    /// it technically owns the stream — and only one process can hold that
    /// stream at a time (see `tools/alsa/README.md`'s "Known limits"), so
    /// real playback audio from another app never reaches it either.
    pub fn has_playback_meters(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
    /// Output meters, computed host-side like TotalMix from levels the
    /// caller already drained this tick: each output's level is the
    /// power sum of every routed source (inputs + playbacks) scaled by
    /// that source's fader into the output. Passing the levels in
    /// matters: the previous `output_meters()` called `input_meters()`/
    /// `playback_meters()` itself, and those drain, so on USB and ALSA
    /// the sum was computed from the near-empty second drain. See
    /// `power_sum_output_meters`'s own doc comment for the math.
    pub fn output_meters_from(&self, input_meters: &[f32], playback_meters: &[f32]) -> Vec<f32> {
        power_sum_output_meters(
            &self.inputs().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            input_meters,
            &self.playbacks().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            playback_meters,
            self.outputs().len(),
        )
    }
    pub fn is_mock(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
    /// Whether the sound server, not this app, sets the card's sample rate
    /// and buffer size: true for the kernel driver (PipeWire opens the
    /// card), false for the USB backend, which streams the device itself.
    pub fn rate_follows_sound_server(&self) -> bool {
        #[cfg(feature = "alsa")]
        if matches!(self, DeviceHandle::Real(_)) {
            return true;
        }
        false
    }
    /// Whether this backend actually implements `set_sensitivity` —
    /// real for Mock and the USB/libusb backend (wired to the same
    /// hardware-verified ref-level mechanism `set_ref_level` already
    /// used for Instr 3/4's +4dBu/-10dBV/Boost switch). **2026-09-06**:
    /// now real for the ALSA/kernel-driver backend too, once
    /// `babyface-pro-linux` gained its own "Instrument Ref Level"
    /// control — previously this backend had no such control at all.
    pub fn has_sensitivity_control(&self) -> bool {
        match self {
            DeviceHandle::Mock(_) | DeviceHandle::Usb(_) => true,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => true,
        }
    }
    /// True when the backend lays out outputs as ONE channel per submix
    /// pair (the proprietary USB path: `outputs().len() ==
    /// output_pair_count()`) rather than two channels per pair (the
    /// ALSA/profile `build_outputs` layout). Callers that map an output
    /// strip click back to the submix pair index need this.
    pub fn outputs_one_per_pair(&self) -> bool {
        self.outputs().len() == self.output_pair_count()
    }
    /// The current front-panel state (MIX engaged, IN sel, OUT sel) —
    /// only the proprietary USB backend has a panel.
    pub fn panel_selection(&self) -> Option<(bool, usize, usize)> {
        match self {
            DeviceHandle::Mock(_) => None,
            DeviceHandle::Usb(d) => Some(d.panel_selection()),
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(d) => d.panel_selection(),
        }
    }
}

/// The actual math behind `DeviceHandle::output_meters_from` — each output
/// *channel*'s level is the power sum of every input/playback source
/// routed into it, scaled by that source's live meter reading. Pulled
/// out as a free, pure function (rather than left inline) so it's
/// testable with deterministic meter values — the mock backend's own
/// `input_meter`/`playback_meter` are randomized, so a test going
/// through `DeviceHandle` itself could never assert an exact number.
///
/// `input_volumes`/`playback_volumes` are indexed by *output pair*
/// (`output_pair_count()` entries — one crosspoint per pair, not per
/// individual physical channel) while `n_out` is the individual output
/// *channel* count (2 per pair) — the pair a channel `o` belongs to is
/// `o / 2`, the same convention `set_channel_volume` and friends use
/// everywhere else. Indexing the volumes arrays with the raw channel
/// index instead of `o / 2` used to silently read the *wrong* pair's
/// crosspoint for every odd channel, and always read nothing at all
/// for every channel whose pair index was past the volumes arrays'
/// own (pair-counted) length — every output pair past the first three
/// read a permanent, wrong zero level regardless of actual audio.
fn power_sum_output_meters(
    input_volumes: &[Vec<f32>],
    input_meters: &[f32],
    playback_volumes: &[Vec<f32>],
    playback_meters: &[f32],
    n_out: usize,
) -> Vec<f32> {
    let mut out = vec![0.0f32; n_out];
    for (o, slot) in out.iter_mut().enumerate() {
        let pair = o / 2;
        let mut p = 0.0f32;
        for (i, vols) in input_volumes.iter().enumerate() {
            let v = vols.get(pair).copied().unwrap_or(0.0);
            let m = input_meters.get(i).copied().unwrap_or(0.0);
            p += (m * v) * (m * v);
        }
        for (c, vols) in playback_volumes.iter().enumerate() {
            let v = vols.get(pair).copied().unwrap_or(0.0);
            let m = playback_meters.get(c).copied().unwrap_or(0.0);
            p += (m * v) * (m * v);
        }
        *slot = p.sqrt().min(1.0);
    }
    out
}

/// Which top-level page is showing. `Quick` is the default — a focused
/// "pick a source, adjust it, pick a destination, adjust it" pair of big
/// strips, for the podcaster/streamer/remote-work case that never touches
/// the full matrix. `Mixer`/`Matrix` are the existing dense views for
/// users who need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum View {
    Quick,
    Mixer,
    Matrix,
}

// ── Messages ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    SetView(View),
    /// The source channel picked in the Quick Control view — always an
    /// `Input`/`Playback` id, never `Output` (see `quick_channel_options`,
    /// the only place that builds the pick-list this feeds).
    QuickChannelSelected(ChannelId),
    SelectOutput(usize),
    ModifiersChanged(keyboard::Modifiers),
    /// F11: toggle the window between windowed and fullscreen. The id
    /// comes from the global event handler, the only place that has it.
    ToggleFullscreen(window::Id),
    TabPressed,
    EscapePressed,
    /// The OS window was resized (or just opened) — tracks the current
    /// width in `TuxMix::window_width` (coalesced, `RESIZE_THROTTLE`) so
    /// `responsive_row` can decide whether a strip row fits or scrolls.
    /// This no longer rescales anything: strips keep their current
    /// `ui_scale` (changed only by the manual `Zoom*` / `WheelZoom`
    /// messages), so a window drag is pure scroll/clip.
    WindowResized(f32),
    /// The window's full size, kept for the preferences (`window_size`).
    WindowSize(f32, f32),

    /// Ctrl+wheel — zooms the UI in/out (see `zoom`). Ignored unless Ctrl
    /// is held (`TuxMix::modifiers`); plain wheel scrolls as normal.
    WheelZoom(f32),
    /// Discrete manual zoom (Ctrl+= / Ctrl+- / Ctrl+0).
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// The monitor's logical size, asked once at the first launch to pick
    /// a sensible starting zoom (`initial_zoom_for`).
    MonitorSize(Option<iced::Size>),
    /// The window's close button (or a close request from the system):
    /// hides the window when the app runs in the tray, quits otherwise.
    CloseRequested,
    /// A window went away without our asking (the compositor closed it).
    WindowClosed(window::Id),
    /// The notification-area icon, or a second launch (`tray`).
    Tray(crate::tray::TrayEvent),
    /// "Keep running in the tray when I close the window".
    SetKeepInTray(bool),
    /// "Start with the session" (an autostart entry, hidden in the tray).
    SetAutostart(bool),

    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    /// The sidebar's global "M" button (`sidebar::msf_row`), TotalMix's
    /// Mute Master: if any channel is muted, unmutes exactly those and
    /// remembers them (`TuxMix::last_cleared_mutes`); if none is, mutes
    /// the remembered set again. The same shape as `GlobalSoloToggle`.
    /// RME's manual: "As soon as any Mute button is pressed, the Mute
    /// Master button lights up ... With this button all selected mutes can
    /// be switched off and on again." It used to mute *every* channel
    /// instead, which is a panic button TotalMix does not have, and left
    /// the button unlit while a single channel was muted.
    GlobalMuteToggle,
    /// The sidebar's global "S" button — a real toggle, symmetric with
    /// `GlobalMuteToggle`: if anything is soloed, clears every solo and
    /// remembers which channels those were (`TuxMix::last_cleared_solos`);
    /// if nothing is soloed and there's a remembered set, restores solo
    /// on exactly those channels instead. There's no sensible "solo
    /// everything" (unlike mute), so restoring the prior set is the
    /// toggle's other direction rather than that.
    GlobalSoloToggle,
    Phantom(usize, bool),
    Pad(usize, bool),
    /// Raw preamp gain units (0..=`InputChannel::gain_max`), not dB —
    /// the hardware's own scale isn't a fixed dB curve we can label.
    Gain(usize, u32),
    /// `true` = +4dBu, `false` = -10dBV.
    Sensitivity(usize, bool),
    /// Trim in dB (-65..+6, see `RmeDevice::set_trim`) — `usize` is the
    /// input index, same scale as every other Hardware Input, unlike
    /// `Gain` which is raw per-model hardware units.
    TrimChanged(usize, f32),
    TrimReset(usize),

    /// Hardware 3-band + low-cut EQ (analog inputs only) — `usize` is
    /// always the input index; the `Eq*Band*` variants carry the band
    /// index (0-2) as their second field. See `tuxmix_core::channel::InputEq`.
    EqEnabled(usize, bool),
    EqBandType(usize, usize, EqBandType),
    EqBandFreq(usize, usize, u16),
    EqBandQ(usize, usize, f32),
    EqBandGain(usize, usize, f32),
    /// A drag in the EQ plot: frequency and gain move together, so the
    /// device gets one update instead of two.
    EqBandPoint(usize, usize, u16, f32),
    /// Double-click on a band handle: back to 1 kHz, Q 0.7, 0 dB.
    EqBandReset(usize, usize),
    EqLowCutFreq(usize, u16),
    EqLowCutSlope(usize, u8),

    /// Output-channel index (not the submix pair — converted in the
    /// handler via `DeviceHandle::outputs_one_per_pair`).
    LoopbackChanged(usize, bool),
    /// Toggles the stereo link/split state of `cid`'s pair — dispatches
    /// to `set_input_pair_linked`/`set_playback_linked`/
    /// `set_output_linked` depending on which kind `cid` is.
    StereoLinkChanged(ChannelId, bool),
    /// Global device settings, all in the `device_panel` drawer.
    PitchChanged(f32),
    WidthChanged(f32),
    MsProcChanged(bool),
    An12Changed(bool),
    InputLinkChanged(bool),
    EqForRecordChanged(bool),
    OpticalOutFormatChanged(bool),
    /// The CUE button on an Output strip — exclusive across every
    /// output (see `RmeDevice::set_cue`'s own doc comment), so this
    /// always carries the *target* state, not a relative toggle.
    CueChanged(ChannelId, bool),
    /// Phase Ø invert — Hardware Input strips, the 4 analog inputs only.
    PhaseChanged(ChannelId, bool),

    VolumeChanged(ChannelId, usize, f32),
    FaderPressed(ChannelId, usize, f32, Option<(f32, f32)>),
    RangeCleared(ChannelId),
    Reset(ChannelId, usize, f32),

    PanChanged(ChannelId, usize, i8),
    PanReset(ChannelId, usize),
    ToggleCollapse(ChannelId),
    /// Opens/closes one strip's flyout (route bus picker or settings —
    /// see `strip::FlyoutKind`). Opening one closes whichever other was
    /// open, of either kind, since at most one is ever shown at a time —
    /// a display preference, deliberately not propagated across a multi-
    /// selection the way `ToggleCollapse` is.
    ToggleFlyout(ChannelId, strip::FlyoutKind),
    /// Closes the open flyout — the click-outside catcher.
    CloseFlyout,
    /// Fires only while `collapse_anim` is non-empty (see `subscription`) —
    /// exists purely to trigger a redraw at a much higher rate than the
    /// normal 50ms `Tick` so the width tween looks smooth, and to prune
    /// entries once they've settled so that faster timer can shut off.
    CollapseTick,
    /// Window close requested — save the mixer state NOW so the next UI
    /// (GUI or TUI, shared `auto.json`) doesn't restore a stale snapshot.
    SaveNow,

    EditStart(ChannelId, String),
    EditChanged(String),
    EditCommit,

    /// Ctrl/Shift+click on a strip's non-control area toggles its
    /// selection membership; a plain click there is a no-op (so
    /// double-click-to-collapse on a selected strip isn't disrupted by an
    /// intervening deselect on the first press).
    StripClicked(ChannelId),
    /// Plain click on genuinely empty page background clears the
    /// selection — see `page()`.
    ClearSelection,
    /// The cursor just entered (`Some`) or left (`None`) a strip's card —
    /// drives the hover border/glow. See `TuxMix::hovered_strip`.
    StripHovered(Option<ChannelId>),

    /// The OSC worker (see `osc.rs`) just started — carries the sender
    /// used to push outgoing feedback packets back out over UDP. Only
    /// fires if `--osc` was passed; `state.osc_tx` stays `None` otherwise.
    OscReady(mpsc::Sender<OscOutbound>),
    /// A command decoded from an incoming OSC UDP packet — applied the
    /// same way a GUI-originated change would be, then echoed back out so
    /// a connected controller and the GUI stay in sync regardless of
    /// which one actually moved a fader.
    OscCommand(OscCommand),
    /// One formatted line for the OSC debug log panel — sent for every
    /// message crossing the bridge in either direction, regardless of
    /// whether the panel is currently open (see `osc::worker`).
    OscLog(String),
    /// Toggles the OSC debug log panel (top-bar button, only shown when
    /// `--osc` is active).
    ToggleOscLog,
    ClearOscLog,
    /// Toggles the global device settings drawer (clock source, SPDIF,
    /// sample rate).
    ToggleDevicePanel,
    /// Toggles the About modal (version, icon, links; see `about`).
    ToggleAbout,
    /// The TuxMix settings popover (the app's own, not the device's).
    ToggleSettings,
    /// Opens the repository in the desktop's browser, from the About
    /// drawer's own link.
    OpenRepoUrl,
    ClockSourceSelected(String),
    SampleRateSelected(u32),

    // ── Right sidebar ("control strip") ─────────────────────────────
    /// Pops `undo_stack`, pushes the current state onto `redo_stack`,
    /// applies the popped one. No-op if `undo_stack` is empty.
    Undo,
    /// Mirror of `Undo` against `redo_stack`.
    Redo,
    /// Expands/collapses one of the 4 sidebar panels (each has its own
    /// "−"/"+" header, like the reference).
    ToggleSidebarPanel(sidebar::Panel),
    /// Expands/collapses the *whole* right sidebar — distinct from
    /// `ToggleSidebarPanel`, see `TuxMix::sidebar_open`'s own doc comment.
    ToggleSidebar,
    /// One horizontal drag step on the sidebar's own resize strip, in
    /// logical pixels (negative = the pointer moved left, which *widens*
    /// the sidebar) — see `sidebar::resize_strip`. Clamped to
    /// `sidebar::MIN_WIDTH`/`MAX_WIDTH` in `update`. UI-only: never
    /// pushed on `undo_stack`, never persisted.
    SidebarResize(f32),
    /// The Options panel's mixer layout (TotalMix's "2 Row"), see
    /// `RowMode`. UI-only.
    SetRowMode(RowMode),
    /// Flips the hardware Dim (`RmeDevice::set_dim`), the same switch the
    /// front-panel DIM button toggles.
    DimToggle,
    /// Recalls "Mix `u8`" (1-8) and marks it as the slot `SnapshotStore`
    /// will save into.
    SnapshotClicked(u8),
    SnapshotStore,
    /// Enters/exits group-assignment mode — while active, clicking a
    /// group's mute/solo/fader cell assigns the current multi-selection
    /// (`state.selected`) as that group's membership.
    GroupEditToggle,
    GroupLinkToggle(usize, sidebar::GroupLink),
    /// Clears the group currently being edited (or all four, if none
    /// is — see the handler).
    GroupClear,
    /// Recalls "Layout `u8`" (1-6) and marks it as the slot
    /// `LayoutStore` will save into.
    LayoutClicked(u8),
    LayoutStore,
    /// Double-click on a strip's name: start typing a new one for `cid`.
    RenameStart(ChannelId),
    RenameChanged(String),
    /// Enter: keep the typed name (an empty one restores the original).
    RenameCommit,
    /// The Options panel's "names" row, TotalMix's "Show Names": show the
    /// original channel names even where the user renamed one.
    SetShowOriginalNames(bool),
    /// The pointer entered (`true`) or left a Matrix cell: the source row,
    /// and the physical output column. Drives the crosshair labels and
    /// the view's readout.
    MatrixHover(ChannelId, usize, bool),
    /// Any left press in the window (from the global event listener).
    /// While a text field is open, it asks the field whether it still has
    /// focus, see `Message::FieldFocus`.
    PointerPressed,
    /// The answer to that question: `false` means the click landed outside
    /// the field (iced unfocuses a text input then), so its text is
    /// accepted, the way a click away commits an edit in any desktop app.
    /// Carries which field and which strip were open when asked, so a late
    /// answer cannot commit a different, newer edit.
    FieldFocus(EditField, ChannelId, bool),
    /// Force PipeWire's graph rate (Hz), `0` = back to its default. See
    /// `tuxmix_core::pw_clock`: system-wide, until PipeWire restarts.
    EngineRate(u32),
    /// Force PipeWire's quantum (buffer size in samples), `0` = default.
    EngineQuantum(u32),
    /// Periodic re-read of the PipeWire clock (another app may change it).
    EnginePoll,
    /// The graph clock, and the card's own open stream (`None` = idle).
    EngineRead(Option<PwClock>, Option<CardStream>),
}

/// The ALSA card id the kernel driver registers, for reading the card's
/// own stream parameters (`pw_clock::card_stream`).
const CARD_ID: &str = "BabyfacePro";

/// The two inline text fields a strip can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Rename,
    Db,
}

/// How the Mixer lays its three sections out. `Two` is TotalMix's "2 Row":
/// Hardware Inputs and Software Playback side by side, outputs underneath.
/// `Auto` (the default) takes two rows when the two blocks fit side by
/// side and three otherwise: on a 1920 px window that shows the whole
/// mixer without scrolling, where three rows used half the width and
/// pushed the outputs below the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RowMode {
    #[default]
    Auto,
    Three,
    Two,
}

/// The strip-rename field's widget id, so `RenameStart` can focus it.
pub(crate) const RENAME_INPUT_ID: &str = "strip-rename";
/// The typed-dB field's widget id, for the same reason.
pub(crate) const DB_INPUT_ID: &str = "strip-db";

// ── App state ────────────────────────────────────────────────────

pub struct TuxMix {
    pub device: DeviceHandle,
    pub sel_out: usize,
    /// Last front-panel OUT selection seen by `Message::Tick` — when it
    /// changes, `sel_out` follows (TotalMix highlights the panel's
    /// current submix). `None` = never seen (no panel / first tick).
    pub last_panel_out: Option<usize>,
    pub view: View,
    /// The channel shown in the Quick Control view's source block — an
    /// `Input`/`Playback` id, defaulting to the first input. Independent of
    /// `selected` (multi-select in the Mixer/Matrix views) — Quick Control
    /// always focuses exactly one channel.
    pub quick_channel: ChannelId,
    pub editing: Option<ChannelId>,
    pub edit_buf: String,
    pub drag_range: Option<(ChannelId, f32, f32)>,
    pub modifiers: keyboard::Modifiers,
    /// Ballistics-smoothed meter values shown in the UI — the raw values
    /// from `device.input_meter`/`playback_meter` jump straight to their new
    /// reading every tick, which reads as flickery rather than a real meter
    /// needle. Smoothed here instead of at the device layer so it applies
    /// uniformly regardless of data source (mock or real hardware).
    pub input_meters: Vec<MeterAnim>,
    pub playback_meters: Vec<MeterAnim>,
    /// Host-side playback tap from PipeWire's sink monitor. Disabled in
    /// mock mode, where the mock's own levels are authoritative.
    pub playback_tap: pw_meter::PlaybackTap,
    /// Monitor channel count the tap currently has (0 = not running):
    /// 2 in the stereo profile (PB1 only), 12 in Pro Audio (all pairs).
    pub playback_tap_channels: usize,
    /// Host-computed output levels (power sum of routed sources ×
    /// faders) — same ballistics as the input/playback meters.
    pub output_meters: Vec<MeterAnim>,
    /// Strips the user has collapsed to save horizontal space — presence in
    /// the set means collapsed. This is the *target* state; the strip may
    /// still be mid-transition, tracked separately in `collapse_anim`.
    pub collapsed: HashSet<ChannelId>,
    /// In-flight collapse/expand width animations, keyed by strip — only
    /// holds an entry while a strip is actually transitioning; pruned once
    /// settled (see `Message::CollapseTick`), which also lets the extra
    /// high-frequency redraw timer in `subscription()` shut itself off.
    pub collapse_anim: HashMap<ChannelId, strip::CollapseAnim>,
    /// Every strip+flyout currently open — the route bus picker or the
    /// 48V/PAD/Sensitivity settings panel, EQ, or Trim (see
    /// `strip::FlyoutKind`, `route_popover`, `settings_popover`,
    /// `Message::ToggleFlyout`). Any number can be open at once, on
    /// different strips or different kinds on the same strip — matches
    /// real TotalMix, which doesn't close one panel just because another
    /// opened (user-caught: TuxMix used to force a single global
    /// `Option`, closing whatever was open the moment a new gear icon
    /// was clicked). **Exception**: `Route` stays exclusive with itself
    /// (opening a new one still closes any other Route flyout) — its
    /// Stack-based overlay (`with_flyout`) only ever positions/renders
    /// one popover at a time, a real rendering constraint, not a design
    /// choice; Settings/Eq/Trim have no such limit, they push the row's
    /// own layout instead of overlaying it. Opens/closes instantly, no
    /// width tween — unlike `collapse_anim`, this isn't animated.
    pub flyout_open: HashSet<(ChannelId, strip::FlyoutKind)>,
    /// Manual UI zoom for the mixer/matrix views — every text size and
    /// widget dimension there is multiplied by this. Changed only by
    /// `ZoomIn`/`ZoomOut`/`ZoomReset`/Ctrl+molette (`zoom`); it is NOT tied
    /// to window size, so resizing never rescales anything — strips keep
    /// fixed geometry and rows/pages scroll. `SCALE_DEFAULT` is the default.
    pub ui_scale: f32,
    /// F11 state: the window is currently fullscreen. Kept in sync with
    /// `Message::ToggleFullscreen`; the WM can still change the mode on
    /// its own, in which case this only affects the next F11 toggle.
    pub fullscreen: bool,
    /// Current window width in logical pixels, kept in sync via
    /// `Message::WindowResized` — lets `mixer_view` decide whether a
    /// strip row fits without scrolling (see `responsive_row`).
    /// `Length::Fill` alone can't answer that inside a horizontal
    /// `Scrollable`: iced compresses Fill back down to the content's
    /// natural size on the scroll axis, so centering has to be done by
    /// skipping the scrollable entirely when content already fits —
    /// which needs the real window width tracked in state.
    pub window_width: f32,
    /// The window's size, saved at close so the next launch opens the
    /// same (`prefs::Prefs::window_size`). Not written on every resize
    /// step: a drag would rewrite the file 20 times a second.
    pub window_size: Option<(f32, f32)>,
    /// The last time anything on screen changed or the user acted: while
    /// it is recent the UI ticks at `TICK_ACTIVE`, afterwards at
    /// `TICK_IDLE` (see `tick_interval`). An idle mixer used to redraw 20
    /// times a second for nothing: ~12 ms a frame, a quarter of a core.
    pub last_activity: Instant,
    /// The notification-area icon is up (`tray::start` found a service):
    /// only then may the window be hidden, there is a way back.
    pub tray_active: bool,
    /// Closing the window hides it instead of quitting (needs `tray_active`).
    pub keep_in_tray: bool,
    /// The window is hidden in the tray right now.
    pub window_hidden: bool,
    /// The last failure worth telling the user about (a device write that
    /// was refused), shown for `NOTICE_FOR`.
    pub notice: Option<(String, Instant)>,
    /// The open window, if any: none while hidden in the tray.
    pub window_id: Option<window::Id>,
    /// An autostart entry exists (`tray::autostart_enabled`).
    pub autostart: bool,
    /// The first launch has no saved zoom: pick one from the monitor.
    pub initial_zoom_pending: bool,
    /// The sidebar was folded by a narrow window (not by the user), so it
    /// reopens when the window widens again.
    pub auto_folded: bool,
    /// When the last resize was actually applied to `window_width`/
    /// `ui_scale` (`None` = never) — the resize coalescing throttle (see
    /// `RESIZE_THROTTLE`) compares against this so a stream of `Resized`
    /// events doesn't rebuild the whole view on every single one.
    last_resize_applied: Option<Instant>,
    /// The most recent resize width still waiting to be applied (it arrived
    /// inside `RESIZE_THROTTLE` of the last applied one) — flushed by the
    /// next due resize or the next `Message::Tick`. See `apply_pending_resize`.
    pending_resize_width: Option<f32>,
    /// Multi-selected strips — Ctrl+click toggles just the clicked strip,
    /// Shift+click selects the whole range from `select_anchor` (standard
    /// file-manager convention), click empty background to clear.
    /// Mute/solo/collapse/volume/pan applied to any selected strip apply
    /// to the whole selection at once.
    pub selected: HashSet<ChannelId>,
    /// The strip a future Shift+click's range is measured from — the
    /// most recent strip explicitly Ctrl- or Shift-clicked. Not moved by
    /// Shift+click itself, so repeated Shift+clicks pivot around the same
    /// point (letting you grow/shrink a range interactively) the way
    /// Explorer/Finder do.
    pub select_anchor: Option<ChannelId>,
    /// Which strip the cursor is currently over, if any — drives the hover
    /// border/glow in `theme::strip_panel`. Plain `container::Style`
    /// closures don't get a hover `Status` the way `button`'s do, so this
    /// has to be tracked explicitly via `mouse_area::on_enter`/`on_exit`
    /// (see `widgets/strip.rs`) rather than read off the widget itself.
    pub hovered_strip: Option<ChannelId>,
    /// Port config for the OSC control surface — `None` unless `--osc` was
    /// passed, in which case `subscription()` starts the worker.
    pub osc_config: Option<OscConfig>,
    /// Sender for outgoing OSC feedback packets, handed back by the worker
    /// via `Message::OscReady` once it's actually bound and listening.
    /// `None` until then (or always, if OSC isn't enabled) — `notify_osc`
    /// is a no-op in that case.
    pub osc_tx: Option<mpsc::Sender<OscOutbound>>,
    /// Ring buffer of formatted OSC traffic lines (see `Message::OscLog`),
    /// newest first — capped at `OSC_LOG_MAX` so a busy controller can't
    /// grow this unboundedly. Populated regardless of `show_osc_log`, so
    /// opening the panel after a burst of activity isn't a blank page.
    pub osc_log: VecDeque<String>,
    pub show_osc_log: bool,
    /// Docked drawer for global device settings (clock source, SPDIF
    /// flags) — same pattern as `show_osc_log`, see `device_panel`.
    pub show_device_panel: bool,
    /// Whether the About modal is open: the version, the icon and the
    /// links, layered above the whole window (see `about::about_modal`).
    pub show_about: bool,
    pub show_settings: bool,
    /// Whether the device this handle was opened on is still there, as of
    /// the last `Message::Tick` probe (`RmeDevice::is_connected`): the
    /// handle keeps answering for a card that is long gone, so the chip
    /// and About read this, not the handle's mere existence.
    pub connected: bool,
    /// How the device was opened (`None` = auto-detect), kept so a
    /// reconnect can use the same path (see `DeviceHandle::open_real`).
    backend: Option<String>,
    /// `--mock` was passed: never auto-reconnect, there is no hardware to
    /// come back and the mock was asked for.
    explicit_mock: bool,
    /// When the connection was last probed. The throttle for the Tick
    /// probe; `None` = never, so the first tick probes.
    last_conn_check: Option<Instant>,
    /// Debounce timer for the auto-saved mixer state (see
    /// `Message::Tick` — the device has no readback, so this is how the
    /// UI restores the gains/volumes/48V on the next open).
    pub last_auto_save: Instant,
    /// Last JSON written to the "auto" scene, to skip redundant writes.
    pub last_saved_json: Option<String>,

    // ── Right sidebar ────────────────────────────────────────────────
    /// Coarse, `Scene`-snapshot-based undo — see `Message::Undo`'s own
    /// doc comment for the granularity trade-off. Capped at
    /// `UNDO_STACK_CAP`.
    pub undo_stack: Vec<Scene>,
    pub redo_stack: Vec<Scene>,
    pub groups: [Group; 4],
    /// Whether the "edit" button is engaged — while `true`, clicking a
    /// group's mute/solo/fader cell assigns `selected` as that group's
    /// membership (see `Message::GroupLinkToggle`); while `false`, the
    /// same click just flips that link type on/off for the group's
    /// existing membership.
    pub group_editing: bool,
    /// `None` = that slot has never been stored to.
    pub layouts: [Option<HashSet<ChannelId>>; 6],
    pub active_layout: Option<usize>,
    pub active_snapshot: Option<usize>,
    /// Which "Mix n" slots have a stored scene on disk, so the sidebar can
    /// tell an empty slot from a stored one. Read once at startup, kept in
    /// step by `SnapshotStore`.
    pub stored_snapshots: [bool; 8],
    /// The simulated device's snapshots. A mock session never touches the
    /// real `Mix n` files (as its auto-save never does): storing there
    /// would overwrite the user's real snapshots with simulated data, and
    /// recalling a real one would push hardware state into the simulation.
    pub mock_snapshots: [Option<Scene>; 8],
    /// Which channels `Message::GlobalSoloToggle` most recently cleared
    /// — restored on the next press if nothing's been soloed again in
    /// the meantime. Empty when there's nothing to restore.
    pub last_cleared_solos: Vec<ChannelId>,
    /// Which channels `Message::GlobalMuteToggle` most recently
    /// unmuted, muted again on the next press if nothing has been muted in
    /// the meantime. Empty when there's nothing to restore.
    pub last_cleared_mutes: Vec<ChannelId>,
    /// Whether the mix has changed since `active_snapshot` was recalled
    /// or stored: TotalMix flashes the slot ("As soon as the mixer state
    /// is changed the button starts flashing"), the sidebar marks it.
    pub snapshot_dirty: bool,
    /// See `RowMode`.
    pub row_mode: RowMode,
    pub sidebar_panels_open: sidebar::PanelsOpen,
    /// Whether the whole right sidebar is expanded — distinct from
    /// `sidebar_panels_open`, which collapses individual panels *within*
    /// an expanded sidebar. `false` reclaims its width for the mixer,
    /// leaving only a thin rail with the button to bring it back.
    pub sidebar_open: bool,
    /// Draggable width of the whole sidebar, in logical pixels — set by
    /// `Message::SidebarResize`, starting at `sidebar::DEFAULT_WIDTH` and
    /// clamped to `sidebar::MIN_WIDTH`/`MAX_WIDTH`. Like `sidebar_open`
    /// and `sidebar_panels_open`, session state: nothing persists it.
    pub sidebar_width: f32,
    /// User channel names, by the strip they were typed on (see
    /// `Message::RenameStart`). A GUI label only: the device model, OSC
    /// addresses and scenes keep the hardware names.
    pub labels: HashMap<ChannelId, String>,
    /// See `Message::SetShowOriginalNames`.
    pub show_original_names: bool,
    /// The strip whose name is being edited, and the text typed so far.
    pub renaming: Option<ChannelId>,
    pub rename_buf: String,
    /// Where `prefs::Prefs` is written. Set by `main` only, so tests (which
    /// build state through `new`) never touch the user's config.
    pub prefs_path: Option<std::path::PathBuf>,
    /// A preference changed since the last write; flushed on `Tick` and
    /// on window close (`SaveNow`).
    pub prefs_dirty: bool,
    /// The Matrix cell under the pointer (see `Message::MatrixHover`).
    pub matrix_hover: Option<(ChannelId, usize)>,
    /// The continuous edit the last undo step was opened for, and when it
    /// last moved (see `undo_gesture`).
    pub undo_gesture: Option<(UndoGesture, Instant)>,
    /// Set when a text field opens. The press that opened it (the second
    /// click of the double-click) also reaches `PointerPressed`, after the
    /// widget's own message; this makes that one press not count as a
    /// click away.
    pub field_opening: bool,
    /// PipeWire's graph clock (rate, buffer), when the sound server owns
    /// it (`rate_follows_sound_server`); simulated for the mock, which
    /// never touches the real PipeWire; `None` for the USB backend.
    pub engine: Option<PwClock>,
    /// The rate / quantum the user forced here (`0` = PipeWire's own),
    /// kept in the preferences and applied again at launch.
    pub engine_force_rate: u32,
    pub engine_force_quantum: u32,
    /// What the card itself runs at while audio plays (kernel driver only):
    /// its rate and hardware period, which the sound server may pin
    /// regardless of the quantum. `None` while the card is idle.
    pub card_stream: Option<CardStream>,
    /// The kernel driver's period floor (`urb_frames_min`), read at
    /// launch: which buffer sizes the card can actually run. `None` when
    /// unknown (no kernel driver), taken as the default 32.
    pub driver_floor: Option<u32>,
}

/// The mock's stand-in for PipeWire's clock: what the real card runs with
/// nothing forced, 48 kHz and the 256-sample default the WirePlumber rule
/// requests (`node.latency`).
const SIMULATED_ENGINE: PwClock = PwClock {
    rate: 48_000,
    quantum: 256,
    force_rate: 0,
    force_quantum: 0,
};

/// What a continuous control is editing. A drag, a wheel spin or a knob
/// turn sends many messages; all of them on the same control, without a
/// pause longer than `UNDO_COALESCE`, are one undo step instead of one
/// per message (a knob turn used to need dozens of undos, and fader wheel
/// spins and Matrix edits, sent as `VolumeChanged`, were never undoable at
/// all).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UndoGesture {
    Volume(ChannelId, usize),
    Pan(ChannelId, usize),
    Gain(usize),
    Trim(usize),
    EqBand(usize, usize),
    LowCut(usize),
    Pitch,
    Width,
}

/// See `UndoGesture`. Long enough to bridge the gaps inside one wheel spin
/// or knob turn, short enough that two deliberate edits stay two steps.
const UNDO_COALESCE: Duration = Duration::from_millis(700);

fn undo_gesture(message: &Message) -> Option<UndoGesture> {
    Some(match message {
        Message::VolumeChanged(cid, out, _) | Message::FaderPressed(cid, out, ..) => {
            UndoGesture::Volume(*cid, *out)
        }
        Message::PanChanged(cid, out, _) => UndoGesture::Pan(*cid, *out),
        Message::Gain(idx, _) => UndoGesture::Gain(*idx),
        Message::TrimChanged(idx, _) => UndoGesture::Trim(*idx),
        Message::EqBandFreq(idx, band, _)
        | Message::EqBandQ(idx, band, _)
        | Message::EqBandGain(idx, band, _)
        | Message::EqBandPoint(idx, band, ..) => UndoGesture::EqBand(*idx, *band),
        Message::EqLowCutFreq(idx, _) => UndoGesture::LowCut(*idx),
        Message::PitchChanged(_) => UndoGesture::Pitch,
        Message::WidthChanged(_) => UndoGesture::Width,
        _ => return None,
    })
}

/// Matches the "Max lines" default in oscmix's own OSC debug log — enough
/// history to catch a burst of activity without the panel scrolling
/// forever.
const OSC_LOG_MAX: usize = 500;

/// Matches the `Tick` subscription interval below — the release curve is
/// timed in real milliseconds rather than "per tick" so it stays correct if
/// that interval ever changes.
const METER_TICK_MS: f32 = 50.0;
/// Fast rise — a meter should jump to a new peak almost instantly so
/// transients don't feel muted.
const METER_ATTACK: f32 = 0.7;
/// Release rate right after a peak: falls quickly at first...
const METER_RELEASE_START: f32 = 0.22;
/// ...decelerating to a gentle final approach as it settles, instead of
/// falling at one constant rate the whole way down. This ease-out shape
/// (fast-then-gentle) is the same curve easyeffects animates its meters
/// with (a 300ms cubic ease-out) — it's what reads as a real analog needle
/// settling rather than a value sliding down at a fixed speed.
const METER_RELEASE_END: f32 = 0.04;
/// Time to go from `METER_RELEASE_START` to `METER_RELEASE_END` after a peak.
const METER_RELEASE_MS: f32 = 300.0;

/// Per-channel VU ballistics state.
#[derive(Clone, Copy, Debug)]
pub struct MeterAnim {
    /// Value as of the *previous* `step` — the start of the current
    /// keyframe transition `MeterFrame` interpolates from.
    prev_value: f32,
    value: f32,
    /// When `value` was last computed — the display layer (`MeterFrame`)
    /// uses this to interpolate a smooth in-between value at full display
    /// refresh rate instead of jumping once per `Tick`.
    last_step_at: Instant,
    /// Time since the level last rose (i.e. since the last peak) — drives
    /// the release ease-out curve. Clamped at `METER_RELEASE_MS`, meaning
    /// "fully settled into the tail rate".
    release_elapsed_ms: f32,
}

impl MeterAnim {
    fn new() -> Self {
        Self {
            prev_value: 0.0,
            value: 0.0,
            last_step_at: Instant::now(),
            release_elapsed_ms: METER_RELEASE_MS,
        }
    }

    pub fn frame(&self) -> fader::MeterFrame {
        fader::MeterFrame {
            prev: self.prev_value,
            value: self.value,
            since: self.last_step_at,
        }
    }

    /// One nominal tick (`METER_TICK_MS`): what the ballistics were tuned
    /// with, and what the tests step by.
    #[cfg(test)]
    fn step(&mut self, target: f32) {
        self.step_for(target, METER_TICK_MS);
    }

    /// Steps by the time really elapsed since the last step. The attack
    /// and release factors are per nominal tick; a tick that is faster or
    /// slower (the UI ticks at 30 Hz with a signal, 2 Hz at rest) must not
    /// make a meter rise or fall faster or slower, so the factor is raised
    /// to the elapsed/nominal ratio.
    fn step_elapsed(&mut self, target: f32) {
        let dt = self.last_step_at.elapsed().as_secs_f32() * 1000.0;
        self.step_for(target, dt.clamp(1.0, 250.0));
    }

    fn step_for(&mut self, target: f32, dt_ms: f32) {
        self.prev_value = self.value;
        let ratio = dt_ms / METER_TICK_MS;
        let eff = |alpha: f32| 1.0 - (1.0 - alpha).powf(ratio);
        if target >= self.value {
            self.value += (target - self.value) * eff(METER_ATTACK);
            self.release_elapsed_ms = 0.0;
        } else {
            self.release_elapsed_ms = (self.release_elapsed_ms + dt_ms).min(METER_RELEASE_MS);
            let t = self.release_elapsed_ms / METER_RELEASE_MS;
            let alpha = METER_RELEASE_END
                + (METER_RELEASE_START - METER_RELEASE_END) * (1.0 - t) * (1.0 - t);
            self.value += (target - self.value) * eff(alpha);
        }
        self.last_step_at = Instant::now();
    }
}

pub fn new(mock: bool, osc_config: Option<OscConfig>, backend: Option<String>) -> TuxMix {
    let mut device = if mock {
        DeviceHandle::open_mock()
    } else {
        DeviceHandle::open_real(backend.as_deref()).unwrap_or_else(|| {
            eprintln!("No device found. Use --mock for simulation.");
            DeviceHandle::open_mock()
        })
    };
    // Restore the last mixer state so the UI starts in sync with the
    // hardware — the USB backend has NO gain/volume readback (only the
    // 48V/PAD byte is readable), so like TotalMix we re-apply our own
    // saved state (auto-saved in `Message::Tick`). The file is SHARED
    // with the TUI, so whichever UI ran last wins.
    if !mock {
        if let Some(scene) = tuxmix_core::scene::load_auto_scene() {
            if let Err(e) = device.apply_scene(&scene) {
                eprintln!("auto scene load failed: {e:?}");
            }
        }
    }
    let n_inputs = device.inputs().len();
    let n_playbacks = device.playbacks().len();
    let n_outputs = device.outputs().len();
    // Probed before the literal: `device` moves into the struct, so it
    // cannot be asked afterwards.
    let connected = device.is_connected();
    let simulated = device.is_mock();
    // Mock generates its own playback levels; a real backend gets them
    // from PipeWire's sink monitor (see `pw_meter`).
    let playback_tap = if device.is_mock() {
        pw_meter::PlaybackTap::disabled()
    } else {
        pw_meter::PlaybackTap::start()
    };

    TuxMix {
        device,
        connected,
        sel_out: 0,
        last_panel_out: None,
        view: View::Quick,
        quick_channel: ChannelId::Input(0),
        editing: None,
        edit_buf: String::new(),
        drag_range: None,
        modifiers: keyboard::Modifiers::default(),
        input_meters: vec![MeterAnim::new(); n_inputs],
        playback_meters: vec![MeterAnim::new(); n_playbacks],
        playback_tap,
        playback_tap_channels: 0,
        output_meters: vec![MeterAnim::new(); n_outputs],
        collapsed: HashSet::new(),
        collapse_anim: HashMap::new(),
        flyout_open: HashSet::new(),
        ui_scale: theme::SCALE_DEFAULT,
        fullscreen: false,
        // Matches `window::Settings::size` in main.rs — updated for real
        // as soon as the first `Opened`/`Resized` event arrives.
        window_size: None,
        last_activity: Instant::now(),
        tray_active: false,
        keep_in_tray: true,
        window_hidden: false,
        notice: None,
        window_id: None,
        autostart: crate::tray::autostart_enabled(),
        initial_zoom_pending: false,
        auto_folded: false,
        window_width: 1280.0,
        last_resize_applied: None,
        pending_resize_width: None,
        selected: HashSet::new(),
        select_anchor: None,
        hovered_strip: None,
        osc_config,
        osc_tx: None,
        osc_log: VecDeque::new(),
        show_osc_log: false,
        show_device_panel: false,
        show_about: false,
        show_settings: false,
        backend,
        explicit_mock: mock,
        last_conn_check: None,
        // State persistence (the device has no gain/volume readback —
        // TotalMix re-applies its saved state; this is our equivalent).
        last_auto_save: Instant::now(),
        last_saved_json: None,
        undo_stack: Vec::new(),
        redo_stack: Vec::new(),
        groups: Default::default(),
        group_editing: false,
        layouts: std::array::from_fn(|i| crate::layouts::load_layout_file((i + 1) as u8)),
        active_layout: None,
        active_snapshot: None,
        stored_snapshots: std::array::from_fn(|i| {
            !simulated
                && crate::scenes::scenes_dir()
                    .join(format!("Mix {}.json", i + 1))
                    .exists()
        }),
        mock_snapshots: Default::default(),
        last_cleared_solos: Vec::new(),
        last_cleared_mutes: Vec::new(),
        snapshot_dirty: false,
        row_mode: RowMode::Auto,
        sidebar_panels_open: sidebar::PanelsOpen::default(),
        sidebar_open: true,
        sidebar_width: sidebar::DEFAULT_WIDTH,
        labels: HashMap::new(),
        show_original_names: false,
        renaming: None,
        rename_buf: String::new(),
        prefs_path: None,
        prefs_dirty: false,
        matrix_hover: None,
        undo_gesture: None,
        field_opening: false,
        engine: simulated.then_some(SIMULATED_ENGINE),
        engine_force_rate: 0,
        engine_force_quantum: 0,
        card_stream: None,
        // The mock stands for the stock driver.
        driver_floor: simulated.then_some(32),
    }
}

/// The persisted subset of `state` (see `prefs.rs`).
pub fn prefs_of(state: &TuxMix) -> crate::prefs::Prefs {
    let p = &state.sidebar_panels_open;
    let mut labels: Vec<(ChannelId, String)> =
        state.labels.iter().map(|(c, l)| (*c, l.clone())).collect();
    // Stable order, so an unchanged state writes an unchanged file.
    labels.sort_by_key(|(c, _)| format!("{c:?}"));
    crate::prefs::Prefs {
        view: Some(state.view),
        sidebar_open: Some(state.sidebar_open),
        sidebar_width: Some(state.sidebar_width),
        panels_open: Some([p.options, p.snapshots, p.groups, p.layout]),
        two_row: None,
        row_mode: Some(state.row_mode),
        ui_scale: Some(state.ui_scale),
        labels,
        show_original_names: state.show_original_names,
        window_size: state.window_size,
        // Only a choice to leave the default is worth writing down.
        keep_in_tray: (!state.keep_in_tray).then_some(false),
        engine_force_rate: Some(state.engine_force_rate),
        engine_force_quantum: Some(state.engine_force_quantum),
    }
}

/// Restores what `prefs_of` saved. Values are clamped to the same bounds
/// the live controls use, so a hand-edited file cannot open the app in a
/// state the UI could never reach.
pub fn apply_prefs(state: &mut TuxMix, prefs: crate::prefs::Prefs) {
    if let Some(v) = prefs.view {
        state.view = v;
    }
    if let Some(open) = prefs.sidebar_open {
        state.sidebar_open = open;
    }
    if let Some(w) = prefs.sidebar_width {
        state.sidebar_width = w.clamp(sidebar::MIN_WIDTH, sidebar::MAX_WIDTH);
    }
    if let Some([options, snapshots, groups, layout]) = prefs.panels_open {
        state.sidebar_panels_open = sidebar::PanelsOpen {
            options,
            snapshots,
            groups,
            layout,
        };
    }
    // `two_row` is the older boolean key: a saved `true` still means two
    // rows; `false` was merely the old default, so it maps to `Auto`.
    state.row_mode = match (prefs.row_mode, prefs.two_row) {
        (Some(mode), _) => mode,
        (None, Some(true)) => RowMode::Two,
        _ => RowMode::Auto,
    };
    if let Some(z) = prefs.ui_scale {
        state.ui_scale = z.clamp(theme::SCALE_MIN, theme::SCALE_MAX);
    } else {
        // Never saved: the first launch, see `Message::MonitorSize`.
        state.initial_zoom_pending = true;
    }
    state.labels = prefs
        .labels
        .into_iter()
        .filter(|(_, l)| !l.trim().is_empty())
        .collect();
    state.show_original_names = prefs.show_original_names;
    state.window_size = prefs.window_size;
    state.keep_in_tray = prefs.keep_in_tray.unwrap_or(true);
    state.engine_force_rate = prefs.engine_force_rate.unwrap_or(0);
    state.engine_force_quantum = prefs.engine_force_quantum.unwrap_or(0);
}

/// Brings the audio engine in line with the preferences at launch: the
/// rate / buffer forced here last time are forced again (PipeWire forgets
/// them when it restarts), then the clock is read. A choice of "Auto"
/// (`0`) is not re-sent, so launching TuxMix never lifts an override some
/// other tool set. The mock applies them to its simulated clock only.
pub fn restore_engine(state: &mut TuxMix) {
    if state.device.is_mock() {
        // The simulated clock starts at its defaults: the remembered
        // choices are the real PipeWire's.
        return;
    }
    if !state.device.rate_follows_sound_server() {
        return;
    }
    if state.engine_force_rate > 0 {
        if let Err(e) = pw_clock::force_rate(state.engine_force_rate) {
            log::warn!("Restoring the PipeWire rate failed: {e}");
        }
    }
    if state.engine_force_quantum > 0 {
        if let Err(e) = pw_clock::force_quantum(state.engine_force_quantum) {
            log::warn!("Restoring the PipeWire buffer size failed: {e}");
        }
    }
    state.engine = pw_clock::read();
    state.card_stream = pw_clock::card_stream(CARD_ID);
    state.driver_floor = pw_clock::driver_period_floor();
}

/// Writes the preferences if something changed (and a path is set).
fn flush_prefs(state: &mut TuxMix) {
    if !state.prefs_dirty {
        return;
    }
    state.prefs_dirty = false;
    if let Some(path) = state.prefs_path.clone() {
        if let Err(e) = crate::prefs::save(&path, &prefs_of(state)) {
            log::warn!("Failed to save GUI preferences: {e}");
        }
    }
}

/// Whether `message` changes something `prefs_of` persists.
fn changes_prefs(message: &Message) -> bool {
    matches!(
        message,
        Message::SetView(_)
            | Message::TabPressed
            | Message::ToggleSidebar
            | Message::SidebarResize(_)
            | Message::ToggleSidebarPanel(_)
            | Message::SetRowMode(_)
            | Message::WheelZoom(_)
            | Message::ZoomIn
            | Message::ZoomOut
            | Message::ZoomReset
            | Message::SetKeepInTray(_)
            | Message::MonitorSize(_)
            | Message::RenameCommit
            | Message::SetShowOriginalNames(_)
            | Message::EngineRate(_)
            | Message::EngineQuantum(_)
    )
}

/// The user's name for `cid`'s strip, unless "show original names" is on.
/// Keyed by the strip's own channel (a linked pair's left channel), which
/// is also what the Quick picker and the Matrix row groups look up.
pub(crate) fn user_label(state: &TuxMix, cid: ChannelId) -> Option<&str> {
    if state.show_original_names {
        return None;
    }
    state.labels.get(&cid).map(String::as_str)
}

/// The name a strip shows: the user's label unless "show original names"
/// is on, else the hardware-derived one the caller built. Also hands the
/// strip its rename-field state. Every `strip::strip` call goes through
/// this, after any pair-label joining, so a label replaces the whole
/// strip name ("AN1/2" -> "Vocals"), not one half of it.
fn finish_params<'a>(state: &'a TuxMix, mut p: strip::StripParams<'a>) -> strip::StripParams<'a> {
    if let Some(label) = user_label(state, p.cid) {
        p.name = label.to_string();
    }
    p.renaming = state.renaming == Some(p.cid);
    p.mute_armed = mute_armed(state, p.cid);
    p.solo_armed = solo_armed(state, p.cid);
    p.rename_buf = &state.rename_buf;
    p
}

/// The window title: which device this window drives, and whether it is
/// there. That is what the taskbar and the window switcher show, where
/// "TuxMix - RME Mixer" said the same thing for every state.
pub fn title(state: &TuxMix, _window: window::Id) -> String {
    let model = state
        .device
        .model_name()
        .strip_suffix(" (mock)")
        .unwrap_or_else(|| state.device.model_name());
    if !state.connected {
        format!("TuxMix \u{2014} {model} (disconnected)")
    } else if state.device.is_mock() {
        format!("TuxMix \u{2014} {model} (simulated)")
    } else {
        format!("TuxMix \u{2014} {model}")
    }
}

/// Floor used when converting silence (linear 0.0) to dB for group-delta
/// math — an actual `f32::NEG_INFINITY` would turn one dragged-to-zero
/// channel into an infinite delta that snaps every other selected channel
/// to 0.0 or 1.0 depending on direction. A large-but-finite floor keeps
/// the swing dramatic (as it should be) without the infinity/NaN edge
/// case.
const GROUP_SILENCE_DB: f32 = -100.0;

fn vol_to_db(v: f32) -> f32 {
    if v <= 0.0 {
        GROUP_SILENCE_DB
    } else {
        (20.0 * v.log10()).max(GROUP_SILENCE_DB)
    }
}

fn db_to_vol(db: f32) -> f32 {
    if db <= GROUP_SILENCE_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Every selectable channel, in the same order they're laid out on
/// screen (Hardware Inputs, then Software Playback, then Hardware
/// Outputs, each in index order) — the linear ordering Shift+click range
/// selection is measured against, so a range visually spans exactly the
/// strips between two clicks the way it would in a file manager.
fn channel_order(state: &TuxMix) -> Vec<ChannelId> {
    let d = &state.device;
    (0..d.inputs().len())
        .map(ChannelId::Input)
        .chain((0..d.playbacks().len()).map(ChannelId::Playback))
        .chain((0..d.outputs().len()).map(ChannelId::Output))
        .collect()
}

/// Sets `cid`'s volume to `v`. If `cid` is part of an active multi-selection,
/// every other selected channel moves by the same *relative* amount instead
/// of jumping to the same absolute level — preserving the balance between
/// them, the way dragging one fader in a DAW's multi-track selection moves
/// the whole group together rather than flattening it to one value.
///
/// The delta is computed in dB, not raw linear amplitude — the fader's own
/// travel is dB-tapered, so an equal *linear* delta applied to channels
/// sitting at different points on that curve produces wildly different dB
/// swings (a channel near the bottom barely moves while one near unity
/// swings hard). dB delta is what actually reads as "moving together."
/// Routes a volume write through the output-pair link logic
/// (`RmeDevice::set_output_volume`) when `cid` is an Output channel —
/// so a linked pair always moves both channels together regardless of
/// which path triggered the write (fader drag, reset, typed dB, OSC,
/// grouped multi-select) — otherwise a plain `set_volume`. The single
/// choke point every output-channel volume write should go through.
fn set_channel_volume(state: &mut TuxMix, cid: ChannelId, out: usize, v: f32) {
    let _ = match cid {
        ChannelId::Input(ch) => state.device.set_input_volume(ch / 2, ch % 2, out, v),
        ChannelId::Playback(ch) => state.device.set_playback_volume(ch / 2, ch % 2, out, v),
        ChannelId::Output(ch) => state.device.set_output_volume(ch / 2, ch % 2, v),
    };
}

/// Same idea as `set_channel_volume`, for mute.
fn set_channel_mute(state: &mut TuxMix, cid: ChannelId, m: bool) {
    let _ = match cid {
        ChannelId::Input(ch) => state.device.set_input_mute(ch / 2, ch % 2, m),
        ChannelId::Playback(ch) => state.device.set_playback_mute(ch / 2, ch % 2, m),
        ChannelId::Output(ch) => state.device.set_output_mute(ch / 2, ch % 2, m),
    };
}

/// Same idea as `set_channel_volume`, for solo.
fn set_channel_solo(state: &mut TuxMix, cid: ChannelId, s: bool) {
    let _ = match cid {
        ChannelId::Input(ch) => state.device.set_input_solo(ch / 2, ch % 2, s),
        ChannelId::Playback(ch) => state.device.set_playback_solo(ch / 2, ch % 2, s),
        ChannelId::Output(ch) => state.device.set_output_solo(ch / 2, ch % 2, s),
    };
}

fn channel_is_muted(state: &TuxMix, cid: ChannelId) -> bool {
    match cid {
        ChannelId::Input(ch) => state.device.inputs()[ch].mute,
        ChannelId::Playback(ch) => state.device.playbacks()[ch].mute,
        ChannelId::Output(ch) => state.device.outputs()[ch].mute,
    }
}

fn channel_is_soloed(state: &TuxMix, cid: ChannelId) -> bool {
    match cid {
        ChannelId::Input(ch) => state.device.inputs()[ch].solo,
        ChannelId::Playback(ch) => state.device.playbacks()[ch].solo,
        ChannelId::Output(ch) => state.device.outputs()[ch].solo,
    }
}

/// Every channel the device currently exposes, across all three types
/// — what the sidebar's global "M"/"S" buttons
/// (`Message::GlobalMuteToggle`/`GlobalSoloToggle`) act on.
fn all_channel_ids(state: &TuxMix) -> Vec<ChannelId> {
    (0..state.device.inputs().len())
        .map(ChannelId::Input)
        .chain((0..state.device.playbacks().len()).map(ChannelId::Playback))
        .chain((0..state.device.outputs().len()).map(ChannelId::Output))
        .collect()
}

/// Whether at least one channel is currently muted — drives the global
/// "M" button's toggle direction and its lit state (`sidebar::msf_row`
/// reads this too, hence `pub(crate)`).
pub(crate) fn any_channel_muted(state: &TuxMix) -> bool {
    all_channel_ids(state).into_iter().any(|cid| channel_is_muted(state, cid))
}

/// Whether `cid`'s mute is held off by the Mute master: the master's last
/// press cleared it, and nothing has been muted since (a new mute makes the
/// master's next press clear that one instead, so the old set is no longer
/// what it would bring back).
pub(crate) fn mute_armed(state: &TuxMix, cid: ChannelId) -> bool {
    state.last_cleared_mutes.contains(&cid)
        && !channel_is_muted(state, cid)
        && !any_channel_muted(state)
}

/// The same for the Solo master and `cid`'s solo.
pub(crate) fn solo_armed(state: &TuxMix, cid: ChannelId) -> bool {
    state.last_cleared_solos.contains(&cid)
        && !channel_is_soloed(state, cid)
        && !any_channel_soloed(state)
}

/// Whether at least one channel is currently soloed — drives the
/// global "S" button's lit state (there's something for it to clear).
pub(crate) fn any_channel_soloed(state: &TuxMix) -> bool {
    all_channel_ids(state).into_iter().any(|cid| channel_is_soloed(state, cid))
}

/// Every *other* channel that should move/mute/solo alongside `cid`
/// right now: the active multi-selection (if `cid` is part of one, same
/// as before the sidebar's Groups panel existed) unioned with the
/// membership of any of `state.groups` whose `link` is engaged and that
/// `cid` belongs to. `cid` itself is never included, so callers can
/// `is_empty()`-check this directly instead of re-deriving "is anything
/// else supposed to move." A `HashSet` (not `Vec`) since a channel can
/// be both multi-selected *and* in a linked group — the union must
/// de-duplicate, not propagate to the same channel twice.
fn propagation_set(
    state: &TuxMix,
    cid: ChannelId,
    link: sidebar::GroupLink,
) -> HashSet<ChannelId> {
    let mut set = HashSet::new();
    if state.selected.len() > 1 && state.selected.contains(&cid) {
        set.extend(state.selected.iter().copied());
    }
    for g in &state.groups {
        let linked = match link {
            sidebar::GroupLink::Mute => g.mute_linked,
            sidebar::GroupLink::Solo => g.solo_linked,
            sidebar::GroupLink::Fader => g.fader_linked,
        };
        if linked && g.members.contains(&cid) {
            set.extend(g.members.iter().copied());
        }
    }
    set.remove(&cid);
    set
}

fn apply_grouped_volume(state: &mut TuxMix, cid: ChannelId, out: usize, v: f32) {
    let others = propagation_set(state, cid, sidebar::GroupLink::Fader);
    if others.is_empty() {
        set_channel_volume(state, cid, out, v);
        notify_osc(state, OscOutbound::Volume(cid, out, v));
        return;
    }
    let old = state.device.volume(cid, out).unwrap_or(v);
    let delta_db = vol_to_db(v) - vol_to_db(old);
    set_channel_volume(state, cid, out, v);
    notify_osc(state, OscOutbound::Volume(cid, out, v));
    for sel in others {
        let cur = state.device.volume(sel, out).unwrap_or(0.0);
        let new_vol = db_to_vol(vol_to_db(cur) + delta_db).clamp(0.0, 2.0);
        set_channel_volume(state, sel, out, new_vol);
        notify_osc(state, OscOutbound::Volume(sel, out, new_vol));
    }
}

/// Same relative-delta grouping as `apply_grouped_volume`, for pan.
fn apply_grouped_pan(state: &mut TuxMix, cid: ChannelId, out: usize, pan: i8) {
    let others = propagation_set(state, cid, sidebar::GroupLink::Fader);
    if others.is_empty() {
        if let Err(e) = state.device.set_pan(cid, out, pan) {
            fail(state, "set_pan", &e);
        }
        notify_osc(state, OscOutbound::Pan(cid, out, pan));
        return;
    }
    let old = i16::from(state.device.pan(cid, out).unwrap_or(pan));
    let delta = i16::from(pan) - old;
    if let Err(e) = state.device.set_pan(cid, out, pan) {
        fail(state, "set_pan", &e);
    }
    notify_osc(state, OscOutbound::Pan(cid, out, pan));
    for sel in others {
        let cur = i16::from(state.device.pan(sel, out).unwrap_or(0));
        let new = (cur + delta).clamp(-100, 100) as i8;
        if let Err(e) = state.device.set_pan(sel, out, new) {
            fail(state, "set_pan", &e);
        }
        notify_osc(state, OscOutbound::Pan(sel, out, new));
    }
}

/// Pushes a state change out to any connected OSC client — a no-op if
/// `--osc` wasn't passed (`osc_tx` is `None`), and silently dropped rather
/// than blocking the update loop if the outgoing channel is momentarily
/// full (a lagging/absent UDP consumer should never stall the GUI).
fn notify_osc(state: &mut TuxMix, msg: OscOutbound) {
    if let Some(tx) = state.osc_tx.as_mut() {
        let _ = tx.try_send(msg);
    }
}

/// Flips `cid`'s collapsed/expanded target and kicks off (or redirects, if
/// one was already in flight — e.g. double-clicking again before it
/// settles) the width animation that plays it out. A no-op if `cid` is
/// already at `target`, so re-applying the same target to a whole
/// selection doesn't restart every strip's animation from scratch.
fn set_collapsed(state: &mut TuxMix, cid: ChannelId, target: bool) {
    if state.collapsed.contains(&cid) == target {
        return;
    }
    let now = Instant::now();
    let current_w = state.collapse_anim.get(&cid).map(|a| a.at(now)).unwrap_or(
        if state.collapsed.contains(&cid) {
            strip::COLLAPSED_W
        } else {
            strip::full_width(cid)
        },
    );
    let target_w = if target {
        strip::COLLAPSED_W
    } else {
        strip::full_width(cid)
    };
    state.collapse_anim.insert(
        cid,
        strip::CollapseAnim {
            prev: current_w,
            value: target_w,
            since: now,
        },
    );
    if target {
        state.collapsed.insert(cid);
    } else {
        state.collapsed.remove(&cid);
    }
}

/// Opens `(cid, kind)` — adds it to the open set without disturbing any
/// OTHER strip's flyout, matching real TotalMix (see `TuxMix::
/// flyout_open`'s own doc comment). Two exceptions, both real rendering
/// constraints rather than a design choice:
/// - `Route` closes any other open `Route` first (its Stack overlay only
///   positions one popover at a time).
/// - Settings/Eq/Trim close any OTHER Settings/Eq/Trim already open on
///   the SAME `cid` first — the inline-push render loop only handles one
///   panel per strip (an `if`/`else if` chain), a second entry for the
///   same strip would just sit in the set unrendered.
fn open_flyout(state: &mut TuxMix, cid: ChannelId, kind: strip::FlyoutKind) {
    if kind == strip::FlyoutKind::Route {
        state.flyout_open.retain(|&(_, k)| k != strip::FlyoutKind::Route);
    } else {
        state
            .flyout_open
            .retain(|&(c, k)| !(c == cid && k != strip::FlyoutKind::Route));
    }
    state.flyout_open.insert((cid, kind));
}

/// Closes every currently-open `Route` flyout — the only kind
/// `Message::CloseFlyout` is ever sent for (the click-outside catcher in
/// `with_flyout`, `Route`'s own Stack overlay); Settings/Eq/Trim have no
/// such catcher, they're plain inline layout with their own explicit
/// close (re-pressing the same gear/EQ/T icon).
fn close_route_flyout(state: &mut TuxMix) {
    state.flyout_open.retain(|&(_, k)| k != strip::FlyoutKind::Route);
}

/// Cap on `TuxMix::undo_stack`/`redo_stack` — unbounded growth over a
/// long session would otherwise hold one full `Scene` (every channel's
/// full state) per mutating action forever.
const UNDO_STACK_CAP: usize = 50;

/// Whether `message` should push a pre-action snapshot onto
/// `undo_stack` before being handled — a blacklist of messages that
/// either don't touch device state at all (view/selection/animation/
/// text-in-progress UI) or would flood the stack with one entry per
/// event if included (`VolumeChanged` fires continuously during a fader
/// drag; the drag's *start*, `FaderPressed`, is what actually captures
/// the "before" state — deliberately not blacklisted). Defaults to
/// "capture" for anything not explicitly listed, so a future `Message`
/// variant that *does* mutate device state is undoable by default
/// rather than silently missed.
///
/// Known, accepted gap: knob-driven values (`PanChanged`, `Gain`,
/// `TrimChanged`, the EQ band params) have no distinct "drag start"
/// message the way faders do, so a knob drag pushes one snapshot per
/// tick rather than one per gesture — coarser undo than fader moves,
/// not worth a `Knob` widget API change in this pass (see the sidebar
/// implementation plan).
///
/// Also known: `undo_stack`/`redo_stack` hold `Scene`s, which only
/// capture *device* state (`RmeDevice::capture_scene`) — sidebar-only
/// concepts (group membership, which layout/snapshot slot is selected,
/// panel collapse state) aren't part of a `Scene` at all, so those
/// messages are excluded below not just to avoid noise but because
/// capturing around them would be a no-op anyway.
fn is_undoable(message: &Message) -> bool {
    !matches!(
        message,
        Message::Tick
            | Message::SetView(_)
            | Message::QuickChannelSelected(_)
            | Message::SelectOutput(_)
            | Message::ModifiersChanged(_)
            | Message::TabPressed
            | Message::EscapePressed
            | Message::WindowResized(_)
            | Message::WindowSize(..)
            | Message::WheelZoom(_)
            | Message::ZoomIn
            | Message::ZoomOut
            | Message::ZoomReset
            | Message::MonitorSize(_)
            | Message::CloseRequested
            | Message::WindowClosed(_)
            | Message::Tray(_)
            | Message::SetKeepInTray(_)
            | Message::SetAutostart(_)
            | Message::RangeCleared(_)
            | Message::ToggleCollapse(_)
            | Message::ToggleFlyout(..)
            | Message::CloseFlyout
            | Message::CollapseTick
            | Message::SaveNow
            | Message::EditStart(..)
            | Message::EditChanged(_)
            | Message::StripClicked(_)
            | Message::ClearSelection
            | Message::StripHovered(_)
            | Message::OscReady(_)
            | Message::OscCommand(_)
            | Message::OscLog(_)
            | Message::ToggleOscLog
            | Message::ClearOscLog
            | Message::ToggleDevicePanel
            | Message::ToggleAbout
            | Message::ToggleSettings
            | Message::OpenRepoUrl
            | Message::Undo
            | Message::Redo
            | Message::ToggleSidebarPanel(_)
            | Message::ToggleSidebar
            | Message::SidebarResize(_)
            | Message::SetRowMode(_)
            | Message::DimToggle
            | Message::MatrixHover(..)
            | Message::PointerPressed
            | Message::FieldFocus(..)
            | Message::EngineRate(_)
            | Message::EngineQuantum(_)
            | Message::EnginePoll
            | Message::EngineRead(..)
            | Message::RenameStart(_)
            | Message::RenameChanged(_)
            | Message::RenameCommit
            | Message::SetShowOriginalNames(_)
            | Message::SnapshotStore
            | Message::GroupEditToggle
            | Message::GroupLinkToggle(..)
            | Message::GroupClear
            | Message::LayoutClicked(_)
            | Message::LayoutStore
    )
}

/// Whether `message` changes the mix away from the recalled snapshot:
/// everything that would push an undo step, plus the steps of a fader
/// drag (`VolumeChanged`, kept off the undo stack, see `is_undoable`) and
/// undo/redo themselves. Recalling a snapshot is the one undoable message
/// that does the opposite, so it is left out; its handler clears the flag.
fn changes_mix(message: &Message) -> bool {
    match message {
        Message::SnapshotClicked(_) => false,
        Message::VolumeChanged(..) | Message::Undo | Message::Redo => true,
        other => is_undoable(other),
    }
}

/// Whether `message` can only do something on a connection that speaks
/// RME's own protocol: the setters behind `require_proprietary` in the ALSA
/// backend (`babyface.rs`), plus `set_eq_for_record`, which that backend
/// does not implement at all. Everything else in the app (routing, gains,
/// preamps, clock, meters, scenes, the sidebar's own state) works in either
/// personality, so this is deliberately not a general "may the UI do this"
/// list. The renderers hide the controls these carry, and `update` drops the
/// messages, so a stale key binding or an OSC message cannot slip past a
/// hidden button.
fn needs_protocol_features(message: &Message) -> bool {
    matches!(
        message,
        Message::TrimChanged(..)
            | Message::TrimReset(_)
            | Message::PhaseChanged(..)
            | Message::LoopbackChanged(..)
            | Message::PitchChanged(_)
            | Message::WidthChanged(_)
            | Message::MsProcChanged(_)
            | Message::An12Changed(_)
            | Message::InputLinkChanged(_)
            | Message::EqForRecordChanged(_)
            | Message::DimToggle
            | Message::EqEnabled(..)
            | Message::EqBandType(..)
            | Message::EqBandFreq(..)
            | Message::EqBandQ(..)
            | Message::EqBandGain(..)
            | Message::EqBandPoint(..)
            | Message::EqBandReset(..)
            | Message::EqLowCutFreq(..)
            | Message::EqLowCutSlope(..)
    )
}

pub fn update(state: &mut TuxMix, message: Message) -> Task<Message> {
    // Anything but the background polls is the user doing something, or
    // something the user must see: keep the tick fast for a while.
    if !matches!(
        message,
        Message::Tick | Message::EnginePoll | Message::EngineRead(..)
    ) {
        state.last_activity = Instant::now();
    }
    // The class-compliant personality drives routing, preamps and clock and
    // nothing else (see `RmeDevice::protocol_features`). The UI already
    // hides the controls; this is the other half of that, so nothing built
    // by hand reaches a setter that can only fail.
    if needs_protocol_features(&message) && !state.device.protocol_features() {
        log::debug!("ignored {message:?}: this connection has no RME-protocol features");
        return Task::none();
    }
    if changes_mix(&message) {
        state.snapshot_dirty = true;
    }
    if changes_prefs(&message) {
        state.prefs_dirty = true;
    }
    if is_undoable(&message) {
        // A continuous edit opens one undo step for the whole gesture.
        let now = Instant::now();
        let gesture = undo_gesture(&message);
        let continues = matches!(
            (gesture, state.undo_gesture),
            (Some(g), Some((last, at))) if g == last && now.duration_since(at) < UNDO_COALESCE
        );
        state.undo_gesture = gesture.map(|g| (g, now));
        if !continues {
            let scene = state.device.capture_scene();
            state.undo_stack.push(scene);
            if state.undo_stack.len() > UNDO_STACK_CAP {
                state.undo_stack.remove(0);
            }
            state.redo_stack.clear();
        }
    }
    match message {
        Message::ToggleFullscreen(id) => {
            state.fullscreen = !state.fullscreen;
            let mode = if state.fullscreen {
                window::Mode::Fullscreen
            } else {
                window::Mode::Windowed
            };
            return iced::window::set_mode(id, mode);
        }
        Message::Tick => {
            if state.notice.as_ref().is_some_and(|(_, at)| at.elapsed() > NOTICE_FOR) {
                state.notice = None;
            }
            if state.tray_active {
                crate::tray::set_status(crate::tray::Status {
                    window_visible: !state.window_hidden,
                    dim: state.device.settings().dim,
                });
            }
            flush_prefs(state);
            // Resize coalescing: if the last `Resized` of a drag landed
            // inside the throttle window, flush it here so the width always
            // settles (even if no further `Resized` arrives). 50 ms is well
            // past `RESIZE_THROTTLE`, so this only ever catches the tail end
            // of a drag.
            apply_pending_resize(state);
            // Connection probe, throttled: the handle keeps answering
            // after the card is unplugged (and after a personality switch
            // re-enumerates it), so this is the only thing that can tell
            // the chip the truth. When the card is back: reopen through
            // the same path the app started with, and re-apply the
            // auto-saved scene exactly like `new` does, so the UI and the
            // hardware agree again.
            let conn_due = state
                .last_conn_check
                .map_or(true, |t| t.elapsed() >= Duration::from_millis(CONN_CHECK_MS));
            if conn_due {
                state.last_conn_check = Some(Instant::now());
                state.connected = state.device.is_connected();
                if !state.connected && !state.explicit_mock {
                    if let Some(fresh) = DeviceHandle::open_real(state.backend.as_deref()) {
                        log::info!("Device reconnected");
                        state.device = fresh;
                        state.connected = true;
                        if let Some(scene) = tuxmix_core::scene::load_auto_scene() {
                            if let Err(e) = state.device.apply_scene(&scene) {
                                fail(state, "apply_scene", &e);
                            }
                        }
                    }
                }
            }
            let before = panel_fingerprint(state);
            let _ = state.device.poll_events();
            if panel_fingerprint(state) != before {
                // The front panel moved a level, a mute or the dim: show
                // it at the fast rate while it keeps moving.
                state.last_activity = Instant::now();
            }
            // Follow the front panel's OUT selection (TotalMix
            // highlights the panel's current submix) — only when it
            // CHANGES on the panel, so a GUI click on the submix picker
            // keeps winning until the user touches OUT. Panel OUT
            // selection (0=Ch1/2, 1=Phones, 2=Opt) → submix pair index
            // (OUT_LABELS: AN1/2=0, PH3/4=1, … A7/A8=5).
            if let Some((_, _, out_sel)) = state.device.panel_selection() {
                let out = match out_sel {
                    1 => 1,
                    2 => 5, // Opt = the optical output (A7/A8)
                    _ => 0,
                };
                if state.last_panel_out != Some(out_sel) {
                    state.last_panel_out = Some(out_sel);
                    state.sel_out = out;
                }
            }
            let in_levels = state.device.input_meters();
            for (i, m) in state.input_meters.iter_mut().enumerate() {
                m.step_elapsed(in_levels.get(i).copied().unwrap_or(0.0));
            }
            // Mock owns its playback levels; every real backend reads
            // the host mixer's sink monitor through `pw_meter`.
            let (tap_levels, tap_channels) = state.playback_tap.drain();
            state.playback_tap_channels = tap_channels;
            let pb_levels = if state.device.is_mock() {
                state.device.playback_meters()
            } else {
                tap_levels.to_vec()
            };
            for (i, m) in state.playback_meters.iter_mut().enumerate() {
                m.step_elapsed(pb_levels.get(i).copied().unwrap_or(0.0));
            }
            let out_levels = state.device.output_meters_from(&in_levels, &pb_levels);
            for (i, m) in state.output_meters.iter_mut().enumerate() {
                m.step_elapsed(out_levels.get(i).copied().unwrap_or(0.0));
            }
            // A live signal keeps the UI on the fast tick (and the fade
            // after it, `ACTIVE_FOR`); silence lets it go idle.
            const LIVE: f32 = 0.002;
            if in_levels
                .iter()
                .chain(pb_levels.iter())
                .chain(out_levels.iter())
                .any(|l| *l > LIVE)
            {
                state.last_activity = Instant::now();
            }
            // Persist the mixer state (debounced 3 s) — no hardware
            // readback for gains/volumes, so this is the restore source
            // for the next open (loaded in `new()`). Shared with the TUI.
            if state.last_auto_save.elapsed() >= Duration::from_secs(3) {
                state.last_auto_save = Instant::now();
                // The mock has no hardware and its state would pollute
                // the SHARED auto.json with a "(mock)" model (making the
                // real device reject it on load) — never persist it.
                if state.device.is_mock() {
                    state.last_saved_json = None;
                    return Task::none();
                }
                // GUI/TUI sync: if the OTHER UI wrote auto.json since our
                // last save, re-apply its state first so we don't clobber
                // it with our own (possibly stale) copy — then save ours.
                if let Some(their) = tuxmix_core::scene::auto_scene_written_by_other(
                    state.last_saved_json.as_deref(),
                ) {
                    if let Err(e) = state.device.apply_scene(&their) {
                        fail(state, "apply_scene", &e);
                    }
                }
                let scene = state.device.capture_scene();
                if let Ok(json) = scene.to_json() {
                    if state.last_saved_json.as_deref() != Some(json.as_str()) {
                        if tuxmix_core::scene::save_auto_scene(&scene).is_ok() {
                            state.last_saved_json = Some(json);
                        }
                    }
                }
            }
        }
        Message::WindowClosed(id) => {
            // Our own hide already forgot the id; this is a close we did not
            // ask for. With a tray the app lives on hidden, without one a
            // daemon with no window would run unseen: quit.
            if state.window_id == Some(id) {
                state.window_id = None;
                if state.tray_active && state.keep_in_tray {
                    state.window_hidden = true;
                } else {
                    return Task::batch([update(state, Message::SaveNow), iced::exit()]);
                }
            }
        }
        Message::CloseRequested => {
            let saved = update(state, Message::SaveNow);
            if state.tray_active && state.keep_in_tray {
                // Keep running: the DIM button and the saved mix's restore
                // after a replug are this app's job (see `tray`).
                return Task::batch([saved, hide_window(state)]);
            }
            return Task::batch([saved, iced::exit()]);
        }
        Message::Tray(event) => match event {
            crate::tray::TrayEvent::Show => return show_window(state),
            crate::tray::TrayEvent::Toggle => {
                return if state.window_hidden {
                    show_window(state)
                } else {
                    hide_window(state)
                };
            }
            crate::tray::TrayEvent::Settings => {
                state.show_settings = true;
                state.show_device_panel = false;
                return show_window(state);
            }
            crate::tray::TrayEvent::Dim => return update(state, Message::DimToggle),
            crate::tray::TrayEvent::Snapshot(n) => {
                return update(state, Message::SnapshotClicked(n))
            }
            crate::tray::TrayEvent::Quit => {
                let saved = update(state, Message::SaveNow);
                return Task::batch([saved, iced::exit()]);
            }
        },
        Message::SetKeepInTray(on) => state.keep_in_tray = on,
        Message::SetAutostart(on) => match crate::tray::set_autostart(on) {
            Ok(()) => state.autostart = crate::tray::autostart_enabled(),
            Err(e) => log::warn!("Could not change the autostart entry: {e}"),
        },
        // Immediate save (window close) — the debounce above would lose
        // the last few seconds of changes. Sync with the other UI first
        // (see the 3 s auto-save in `Message::Tick`). The mock never
        // persists (same reasoning as the auto-save).
        Message::SaveNow => {
            // Closing writes the window size, which no resize step does.
            state.prefs_dirty = true;
            flush_prefs(state);
            if !state.device.is_mock() {
                if let Some(their) = tuxmix_core::scene::auto_scene_written_by_other(
                    state.last_saved_json.as_deref(),
                ) {
                    if let Err(e) = state.device.apply_scene(&their) {
                        fail(state, "apply_scene", &e);
                    }
                }
                let scene = state.device.capture_scene();
                if let Ok(json) = scene.to_json() {
                    if tuxmix_core::scene::save_auto_scene(&scene).is_ok() {
                        state.last_saved_json = Some(json);
                    }
                }
            }
        }
        Message::TabPressed => {
            // Cycles all three views — previously skipped Quick (Matrix and
            // Mixer only), which read as broken since the tab bar itself
            // shows three tabs, not two.
            state.view = match state.view {
                View::Quick => View::Mixer,
                View::Mixer => View::Matrix,
                View::Matrix => View::Quick,
            };
        }
        Message::SetView(v) => {
            state.view = v;
            state.matrix_hover = None;
        }
        Message::WheelZoom(y) => {
            // Only zoom on Ctrl+wheel; plain wheel keeps scrolling the
            // hovered scrollable (which iced handles on its own).
            if state.modifiers.contains(keyboard::Modifiers::CTRL) {
                zoom(state, if y > 0.0 { ZOOM_STEP } else { -ZOOM_STEP });
            }
        }
        Message::ZoomIn => zoom(state, ZOOM_STEP),
        Message::ZoomOut => zoom(state, -ZOOM_STEP),
        Message::ZoomReset => state.ui_scale = theme::SCALE_DEFAULT,
        Message::MonitorSize(size) => {
            if std::mem::take(&mut state.initial_zoom_pending) {
                if let Some(size) = size {
                    state.ui_scale = initial_zoom_for(size.height);
                    state.prefs_dirty = true;
                }
            }
        }
        Message::QuickChannelSelected(cid) => state.quick_channel = cid,
        Message::SelectOutput(i) => {
            state.sel_out = i;
            // Selecting a bus dismisses the Route flyout — it did its
            // job. Doesn't touch any other (Settings/Eq/Trim) flyout
            // that happens to be open elsewhere.
            close_route_flyout(state);
        }
        Message::ModifiersChanged(m) => state.modifiers = m,
        Message::WindowSize(w, h) => {
            // A narrow window folds the sidebar, and a wider one brings it
            // back (only if the window folded it: a user's own fold stays).
            let was_narrow = state
                .window_size
                .is_some_and(|(pw, _)| sidebar_does_not_fit(state, pw));
            let narrow = sidebar_does_not_fit(state, w);
            if narrow && !was_narrow && state.sidebar_open {
                state.sidebar_open = false;
                state.auto_folded = true;
            } else if !narrow && was_narrow && state.auto_folded {
                state.sidebar_open = true;
                state.auto_folded = false;
            }
            state.window_size = Some((w, h));
            return update(state, Message::WindowResized(w));
        }
        Message::WindowResized(width) => {
            // Coalesce: a live window drag fires `Resized` faster than the
            // display can present. Stash the width and only rebuild/recompute
            // when `RESIZE_THROTTLE` has elapsed since the last applied one,
            // so the UI doesn't do N full rebuilds for N intermediate sizes
            // that were never even rendered. The throttle is flushed by the
            // next due resize or `Message::Tick` (above).
            state.pending_resize_width = Some(width);
            apply_pending_resize(state);
        }
        Message::EscapePressed => {
            // Topmost thing first: the modal outranks an in-place edit.
            if state.show_about {
                state.show_about = false;
            } else if state.show_device_panel {
                state.show_device_panel = false;
            } else if state.show_settings {
                state.show_settings = false;
            } else if state.renaming.is_some() {
                state.renaming = None;
                state.rename_buf.clear();
            } else if state.editing.is_some() {
                state.editing = None;
            }
        }
        Message::Mute(cid, m) => {
            // `propagation_set` covers both the pre-existing multi-select
            // case and (new) any mute-linked group `cid` belongs to.
            set_channel_mute(state, cid, m);
            notify_osc(state, OscOutbound::Mute(cid, m));
            for other in propagation_set(state, cid, sidebar::GroupLink::Mute) {
                set_channel_mute(state, other, m);
                notify_osc(state, OscOutbound::Mute(other, m));
            }
        }
        Message::Solo(cid, s) => {
            set_channel_solo(state, cid, s);
            notify_osc(state, OscOutbound::Solo(cid, s));
            for other in propagation_set(state, cid, sidebar::GroupLink::Solo) {
                set_channel_solo(state, other, s);
                notify_osc(state, OscOutbound::Solo(other, s));
            }
        }
        Message::GlobalMuteToggle => {
            let muted: Vec<ChannelId> =
                all_channel_ids(state).into_iter().filter(|&cid| channel_is_muted(state, cid)).collect();
            if muted.is_empty() {
                for cid in std::mem::take(&mut state.last_cleared_mutes) {
                    set_channel_mute(state, cid, true);
                    notify_osc(state, OscOutbound::Mute(cid, true));
                }
            } else {
                for &cid in &muted {
                    set_channel_mute(state, cid, false);
                    notify_osc(state, OscOutbound::Mute(cid, false));
                }
                state.last_cleared_mutes = muted;
            }
        }
        Message::GlobalSoloToggle => {
            let soloed: Vec<ChannelId> =
                all_channel_ids(state).into_iter().filter(|&cid| channel_is_soloed(state, cid)).collect();
            if soloed.is_empty() {
                for cid in std::mem::take(&mut state.last_cleared_solos) {
                    set_channel_solo(state, cid, true);
                    notify_osc(state, OscOutbound::Solo(cid, true));
                }
            } else {
                for &cid in &soloed {
                    set_channel_solo(state, cid, false);
                    notify_osc(state, OscOutbound::Solo(cid, false));
                }
                state.last_cleared_solos = soloed;
            }
        }
        Message::Phantom(idx, p) => {
            if let Err(e) = state.device.set_input_phantom(idx, p) {
                fail(state, "set_input_phantom", &e);
            }
        }
        Message::Pad(idx, p) => {
            if let Err(e) = state.device.set_input_pad(idx, p) {
                fail(state, "set_input_pad", &e);
            }
        }
        Message::Gain(idx, g) => {
            if let Err(e) = state.device.set_input_gain(idx, g) {
                fail(state, "set_input_gain", &e);
            }
        }
        Message::Sensitivity(idx, plus4) => {
            let s = if plus4 {
                tuxmix_core::Sensitivity::Plus4dBu
            } else {
                tuxmix_core::Sensitivity::Minus10dBV
            };
            if let Err(e) = state.device.set_sensitivity(idx, s) {
                fail(state, "set_sensitivity", &e);
            }
        }
        Message::TrimChanged(idx, db) => {
            if let Err(e) = state.device.set_trim(idx, db) {
                fail(state, "set_trim", &e);
            }
        }
        Message::TrimReset(idx) => {
            if let Err(e) = state.device.set_trim(idx, 0.0) {
                fail(state, "set_trim", &e);
            }
        }
        Message::EqEnabled(idx, on) => {
            if let Err(e) = state.device.set_eq_enabled(idx, on) {
                fail(state, "set_eq_enabled", &e);
            }
        }
        Message::EqBandType(idx, band, t) => {
            if let Err(e) = state.device.set_eq_band_type(idx, band, t) {
                fail(state, "set_eq_band_type", &e);
            }
        }
        Message::EqBandFreq(idx, band, freq) => {
            if let Err(e) = state.device.set_eq_band_freq(idx, band, freq) {
                fail(state, "set_eq_band_freq", &e);
            }
        }
        Message::EqBandQ(idx, band, q) => {
            if let Err(e) = state.device.set_eq_band_q(idx, band, q) {
                fail(state, "set_eq_band_q", &e);
            }
        }
        Message::EqBandGain(idx, band, gain) => {
            if let Err(e) = state.device.set_eq_band_gain(idx, band, gain) {
                fail(state, "set_eq_band_gain", &e);
            }
        }
        Message::EqBandPoint(idx, band, freq, gain) => {
            if let Err(e) = state.device.set_eq_band_freq(idx, band, freq) {
                fail(state, "set_eq_band_freq", &e);
            }
            if let Err(e) = state.device.set_eq_band_gain(idx, band, gain) {
                fail(state, "set_eq_band_gain", &e);
            }
        }
        Message::EqBandReset(idx, band) => {
            if let Err(e) = state.device.set_eq_band_freq(idx, band, 1000) {
                fail(state, "set_eq_band_freq", &e);
            }
            if let Err(e) = state.device.set_eq_band_q(idx, band, 0.7) {
                fail(state, "set_eq_band_q", &e);
            }
            if let Err(e) = state.device.set_eq_band_gain(idx, band, 0.0) {
                fail(state, "set_eq_band_gain", &e);
            }
        }
        Message::EqLowCutFreq(idx, freq) => {
            if let Err(e) = state.device.set_eq_low_cut_freq(idx, freq) {
                fail(state, "set_eq_low_cut_freq", &e);
            }
        }
        Message::EqLowCutSlope(idx, slope) => {
            if let Err(e) = state.device.set_eq_low_cut_slope(idx, slope) {
                fail(state, "set_eq_low_cut_slope", &e);
            }
        }
        Message::LoopbackChanged(i, on) => {
            let pair = if state.device.outputs_one_per_pair() {
                i
            } else {
                i / 2
            };
            if let Err(e) = state.device.set_loopback(pair, on) {
                fail(state, "set_loopback", &e);
            }
        }
        Message::StereoLinkChanged(cid, linked) => {
            let _ = match cid {
                ChannelId::Input(i) => state.device.set_input_pair_linked(i / 2, linked),
                ChannelId::Playback(i) => state.device.set_playback_linked(i / 2, linked),
                ChannelId::Output(i) => state.device.set_output_linked(i / 2, linked),
            };
        }
        Message::PitchChanged(v) => {
            if let Err(e) = state.device.set_pitch(v) {
                fail(state, "set_pitch", &e);
            }
        }
        Message::WidthChanged(v) => {
            if let Err(e) = state.device.set_width(v) {
                fail(state, "set_width", &e);
            }
        }
        Message::MsProcChanged(on) => {
            if let Err(e) = state.device.set_ms_proc(on) {
                fail(state, "set_ms_proc", &e);
            }
        }
        Message::An12Changed(on) => {
            if let Err(e) = state.device.set_an12(on) {
                fail(state, "set_an12", &e);
            }
        }
        Message::InputLinkChanged(on) => {
            if let Err(e) = state.device.set_input_link(on) {
                fail(state, "set_input_link", &e);
            }
        }
        Message::EqForRecordChanged(on) => {
            if let Err(e) = state.device.set_eq_for_record(on) {
                fail(state, "set_eq_for_record", &e);
            }
        }
        Message::OpticalOutFormatChanged(spdif) => {
            if let Err(e) = state.device.set_optical_out_format(spdif) {
                fail(state, "set_optical_out_format", &e);
            }
        }
        Message::CueChanged(cid, on) => {
            if let ChannelId::Output(idx) = cid {
                if let Err(e) = state.device.set_cue(idx / 2, on) {
                    fail(state, "set_cue", &e);
                }
            }
        }
        Message::PhaseChanged(cid, invert) => {
            if let ChannelId::Input(idx) = cid {
                if let Err(e) = state.device.set_phase(idx, invert) {
                    fail(state, "set_phase", &e);
                }
            }
        }
        Message::VolumeChanged(cid, out, v) => {
            apply_grouped_volume(state, cid, out, v);
        }
        Message::FaderPressed(cid, out, v, range) => {
            if let Some((lo, hi)) = range {
                state.drag_range = Some((cid, lo, hi));
            }
            apply_grouped_volume(state, cid, out, v);
        }
        Message::RangeCleared(cid) => {
            // Letting go ends the gesture: the next drag is its own step.
            state.undo_gesture = None;
            if state.drag_range.is_some_and(|(dc, _, _)| dc == cid) {
                state.drag_range = None;
            }
        }
        Message::Reset(cid, out, default_vol) => {
            if state.selected.len() > 1 && state.selected.contains(&cid) {
                // Reset means "back to default" for the whole group, not a
                // relative move — every selected fader snaps to the same
                // absolute value, unlike a drag which preserves balance.
                for sel in state.selected.clone() {
                    set_channel_volume(state, sel, out, default_vol);
                    notify_osc(state, OscOutbound::Volume(sel, out, default_vol));
                }
            } else {
                set_channel_volume(state, cid, out, default_vol);
                notify_osc(state, OscOutbound::Volume(cid, out, default_vol));
            }
            if state.drag_range.is_some_and(|(dc, _, _)| dc == cid) {
                state.drag_range = None;
            }
        }
        Message::PanChanged(cid, out, pan) => {
            apply_grouped_pan(state, cid, out, pan);
        }
        Message::PanReset(cid, out) => {
            if state.selected.len() > 1 && state.selected.contains(&cid) {
                for sel in state.selected.clone() {
                    if let Err(e) = state.device.set_pan(sel, out, 0) {
                        fail(state, "set_pan", &e);
                    }
                    notify_osc(state, OscOutbound::Pan(sel, out, 0));
                }
            } else {
                if let Err(e) = state.device.set_pan(cid, out, 0) {
                    fail(state, "set_pan", &e);
                }
                notify_osc(state, OscOutbound::Pan(cid, out, 0));
            }
        }
        Message::ToggleCollapse(cid) => {
            let target = !state.collapsed.contains(&cid);
            if state.selected.len() > 1 && state.selected.contains(&cid) {
                // Uniform target for the whole group — the opposite of
                // what the clicked strip currently is — rather than each
                // toggling its own state independently, which would leave
                // them out of sync with each other.
                for sel in state.selected.clone() {
                    set_collapsed(state, sel, target);
                }
            } else {
                set_collapsed(state, cid, target);
            }
        }
        Message::ToggleFlyout(cid, kind) => {
            if state.flyout_open.contains(&(cid, kind)) {
                state.flyout_open.remove(&(cid, kind));
            } else {
                open_flyout(state, cid, kind);
            }
        }
        Message::CloseFlyout => {
            close_route_flyout(state);
        }
        Message::CollapseTick => {
            let now = Instant::now();
            state.collapse_anim.retain(|_, a| a.is_settling(now));
        }
        Message::StripClicked(cid) => {
            if state.modifiers.control() {
                // Toggle just this one strip, leaving the rest of the
                // selection untouched — the standard Ctrl+click
                // convention. Becomes the pivot for the next Shift+click.
                if !state.selected.remove(&cid) {
                    state.selected.insert(cid);
                }
                state.select_anchor = Some(cid);
            } else if state.modifiers.shift() {
                // Select the whole visual range from the anchor through
                // cid, replacing the current selection — the standard
                // Shift+click convention. Doesn't move the anchor, so a
                // second Shift+click elsewhere re-measures from the same
                // start rather than the last endpoint.
                let order = channel_order(state);
                let anchor = state.select_anchor.unwrap_or(cid);
                if let (Some(from), Some(to)) = (
                    order.iter().position(|&c| c == anchor),
                    order.iter().position(|&c| c == cid),
                ) {
                    let (lo, hi) = (from.min(to), from.max(to));
                    state.selected = order[lo..=hi].iter().copied().collect();
                }
                if state.select_anchor.is_none() {
                    state.select_anchor = Some(cid);
                }
            } else if let ChannelId::Output(i) = cid {
                // Clicking a hardware OUTPUT strip selects that output's
                // submix (TotalMix behavior) — the input/playback strips
                // then show their faders INTO this output. The output
                // layout depends on the backend: the USB path is ONE
                // channel per submix pair (index == pair), while the
                // ALSA/profile path is two channels per pair (`2*i` /
                // `2*i+1` in `build_outputs`).
                state.sel_out = if state.device.outputs_one_per_pair() {
                    i
                } else {
                    i / 2
                };
            }
        }
        Message::ClearSelection => {
            state.selected.clear();
            state.select_anchor = None;
        }
        Message::StripHovered(h) => state.hovered_strip = h,
        Message::EditStart(cid, buf) => {
            state.editing = Some(cid);
            state.edit_buf = buf;
            state.field_opening = true;
            // Ready to type over, like the rename field: it used to open
            // unfocused, so typing did nothing until it was clicked again.
            return Task::batch([
                iced::widget::operation::focus(DB_INPUT_ID),
                iced::widget::operation::select_all(DB_INPUT_ID),
            ]);
        }
        Message::EditChanged(s) => state.edit_buf = s,
        Message::EditCommit => {
            if let Some(cid) = state.editing {
                if let Some(v) = parse_db_input(&state.edit_buf) {
                    // An output strip's typed value is its master — use
                    // the channel index, not the selected submix.
                    let out = match cid {
                        ChannelId::Output(i) => i,
                        _ => state.sel_out,
                    };
                    set_channel_volume(state, cid, out, v);
                }
                state.editing = None;
            }
        }
        Message::OscReady(tx) => {
            state.osc_tx = Some(tx);
            // A controller connecting after startup shouldn't stay blind
            // until the next manual change — snapshot everything once.
            let n_out = state.device.output_pair_count();
            for ch in state.device.inputs().to_vec() {
                let cid = ChannelId::Input(ch.id);
                for out in 0..n_out {
                    notify_osc(state, OscOutbound::Volume(cid, out, ch.volumes[out]));
                    notify_osc(state, OscOutbound::Pan(cid, out, ch.pans[out]));
                }
                notify_osc(state, OscOutbound::Mute(cid, ch.mute));
                notify_osc(state, OscOutbound::Solo(cid, ch.solo));
            }
            for ch in state.device.playbacks().to_vec() {
                let cid = ChannelId::Playback(ch.id);
                for out in 0..n_out {
                    notify_osc(state, OscOutbound::Volume(cid, out, ch.volumes[out]));
                    notify_osc(state, OscOutbound::Pan(cid, out, ch.pans[out]));
                }
                notify_osc(state, OscOutbound::Mute(cid, ch.mute));
                notify_osc(state, OscOutbound::Solo(cid, ch.solo));
            }
            for ch in state.device.outputs().to_vec() {
                notify_osc(state, OscOutbound::OutputVolume(ch.id, ch.volume));
                notify_osc(state, OscOutbound::Mute(ChannelId::Output(ch.id), ch.mute));
                notify_osc(state, OscOutbound::Solo(ChannelId::Output(ch.id), ch.solo));
            }
        }
        Message::OscCommand(cmd) => match cmd {
            OscCommand::Volume(cid, out, v) => apply_grouped_volume(state, cid, out, v),
            OscCommand::Pan(cid, out, p) => apply_grouped_pan(state, cid, out, p),
            OscCommand::Mute(cid, m) => {
                set_channel_mute(state, cid, m);
                notify_osc(state, OscOutbound::Mute(cid, m));
            }
            OscCommand::Solo(cid, s) => {
                set_channel_solo(state, cid, s);
                notify_osc(state, OscOutbound::Solo(cid, s));
            }
            OscCommand::OutputVolume(id, v) => {
                let cid = ChannelId::Output(id);
                // `out` must equal the output channel index (see
                // `strip_params`); `set_channel_volume` routes this
                // through the output-pair link logic.
                set_channel_volume(state, cid, id, v);
                notify_osc(state, OscOutbound::OutputVolume(id, v));
            }
        },
        Message::OscLog(line) => {
            state.osc_log.push_front(line);
            state.osc_log.truncate(OSC_LOG_MAX);
        }
        Message::ToggleOscLog => state.show_osc_log = !state.show_osc_log,
        Message::ClearOscLog => state.osc_log.clear(),
        Message::ToggleDevicePanel => {
            state.show_device_panel = !state.show_device_panel;
            state.show_settings = false;
        }
        Message::ToggleSettings => {
            state.show_settings = !state.show_settings;
            state.show_device_panel = false;
        }
        Message::ToggleAbout => state.show_about = !state.show_about,
        Message::OpenRepoUrl => {
            // Fire and forget: the browser is not our child process, and a
            // minimal session may have no `xdg-open` to hand it to at all.
            if let Err(e) = std::process::Command::new("xdg-open")
                .arg(about::REPO_URL)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                log::warn!("Could not open {}: {e}", about::REPO_URL);
            }
        }
        Message::ClockSourceSelected(source) => {
            if let Err(e) = state.device.set_clock_source(&source) {
                fail(state, "set_clock_source", &e);
            }
        }
        Message::SampleRateSelected(rate) => {
            if let Err(e) = state.device.set_sample_rate(rate) {
                fail(state, "set_sample_rate", &e);
            }
        }

        // ── Right sidebar ────────────────────────────────────────────
        Message::Undo => {
            state.undo_gesture = None;
            if let Some(scene) = state.undo_stack.pop() {
                state.redo_stack.push(state.device.capture_scene());
                if let Err(e) = state.device.apply_scene(&scene) {
                    fail(state, "undo", &e);
                }
            }
        }
        Message::Redo => {
            state.undo_gesture = None;
            if let Some(scene) = state.redo_stack.pop() {
                state.undo_stack.push(state.device.capture_scene());
                if let Err(e) = state.device.apply_scene(&scene) {
                    fail(state, "redo", &e);
                }
            }
        }
        Message::ToggleSidebarPanel(panel) => state.sidebar_panels_open.toggle(panel),
        Message::ToggleSidebar => {
            state.sidebar_open = !state.sidebar_open;
            state.auto_folded = false;
        }
        Message::SidebarResize(dx) => {
            state.sidebar_width =
                (state.sidebar_width - dx).clamp(sidebar::MIN_WIDTH, sidebar::MAX_WIDTH);
        }
        Message::SetRowMode(mode) => state.row_mode = mode,
        Message::RenameStart(cid) => {
            // Opening another strip's name accepts the one being typed.
            if state.renaming.is_some_and(|open| open != cid) {
                let _ = update(state, Message::RenameCommit);
            }
            state.rename_buf = state.labels.get(&cid).cloned().unwrap_or_default();
            state.renaming = Some(cid);
            state.field_opening = true;
            return Task::batch([
                iced::widget::operation::focus(RENAME_INPUT_ID),
                iced::widget::operation::select_all(RENAME_INPUT_ID),
            ]);
        }
        Message::RenameChanged(s) => state.rename_buf = s,
        Message::RenameCommit => {
            if let Some(cid) = state.renaming.take() {
                let label = state.rename_buf.trim().to_string();
                if label.is_empty() {
                    state.labels.remove(&cid);
                } else {
                    state.labels.insert(cid, label);
                }
                state.rename_buf.clear();
            }
        }
        Message::SetShowOriginalNames(on) => state.show_original_names = on,
        Message::EngineRate(rate) => {
            // A simulated session changes its simulated clock only, and
            // never the remembered choice: that is applied to the real
            // PipeWire at the next real launch.
            if !state.device.is_mock() {
                state.engine_force_rate = rate;
            }
            if state.device.is_mock() {
                if let Some(e) = state.engine.as_mut() {
                    e.force_rate = rate;
                }
            } else if state.device.rate_follows_sound_server() {
                if let Err(e) = pw_clock::force_rate(rate) {
                    log::warn!("Setting the PipeWire rate failed: {e}");
                }
                state.engine = pw_clock::read();
            }
        }
        Message::EngineQuantum(quantum) => {
            if !state.device.is_mock() {
                state.engine_force_quantum = quantum;
            }
            if state.device.is_mock() {
                if let Some(e) = state.engine.as_mut() {
                    e.force_quantum = quantum;
                }
            } else if state.device.rate_follows_sound_server() {
                if let Err(e) = pw_clock::force_quantum(quantum) {
                    log::warn!("Setting the PipeWire buffer size failed: {e}");
                }
                state.engine = pw_clock::read();
                state.card_stream = pw_clock::card_stream(CARD_ID);
            }
        }
        Message::EnginePoll => {
            // Off the UI thread: a process spawn is ~10 ms.
            return Task::perform(
                async { (pw_clock::read(), pw_clock::card_stream(CARD_ID)) },
                |(clock, stream)| Message::EngineRead(clock, stream),
            );
        }
        Message::EngineRead(clock, stream) => {
            if state.device.rate_follows_sound_server() {
                state.engine = clock;
                state.card_stream = stream;
            }
        }
        Message::PointerPressed => {
            if std::mem::take(&mut state.field_opening) {
                return Task::none();
            }
            let mut checks = Vec::new();
            if let Some(cid) = state.renaming {
                checks.push(
                    iced::widget::operation::is_focused(RENAME_INPUT_ID)
                        .map(move |f| Message::FieldFocus(EditField::Rename, cid, f)),
                );
            }
            if let Some(cid) = state.editing {
                checks.push(
                    iced::widget::operation::is_focused(DB_INPUT_ID)
                        .map(move |f| Message::FieldFocus(EditField::Db, cid, f)),
                );
            }
            return Task::batch(checks);
        }
        Message::FieldFocus(field, cid, focused) => {
            if !focused {
                match field {
                    EditField::Rename if state.renaming == Some(cid) => {
                        return update(state, Message::RenameCommit);
                    }
                    EditField::Db if state.editing == Some(cid) => {
                        return update(state, Message::EditCommit);
                    }
                    _ => {}
                }
            }
        }
        Message::MatrixHover(cid, col, over) => {
            if over {
                state.matrix_hover = Some((cid, col));
            } else if state.matrix_hover == Some((cid, col)) {
                // Only the cell that is hovered may clear it: moving from
                // one cell to the next can deliver the new enter first.
                state.matrix_hover = None;
            }
        }
        Message::DimToggle => {
            let on = !state.device.settings().dim;
            if let Err(e) = state.device.set_dim(on) {
                fail(state, "dim", &e);
            }
        }
        Message::SnapshotClicked(n) => {
            state.active_snapshot = Some(n as usize);
            state.snapshot_dirty = false;
            let scene = if state.device.is_mock() {
                state.mock_snapshots.get(n as usize - 1).cloned().flatten()
            } else {
                load_scene_file(&format!("Mix {n}"))
            };
            if let Some(scene) = scene {
                if let Err(e) = state.device.apply_scene(&scene) {
                    fail(state, "snapshot", &e);
                }
            }
        }
        Message::SnapshotStore => {
            if let Some(n) = state.active_snapshot {
                let scene = state.device.capture_scene();
                let saved = if state.device.is_mock() {
                    if let Some(slot) = state.mock_snapshots.get_mut(n - 1) {
                        *slot = Some(scene);
                    }
                    Ok(())
                } else {
                    save_scene_file(&format!("Mix {n}"), &scene)
                };
                if let Err(e) = saved {
                    log::warn!("Failed to store snapshot 'Mix {n}': {e}");
                } else {
                    state.snapshot_dirty = false;
                    if let Some(slot) = state.stored_snapshots.get_mut(n - 1) {
                        *slot = true;
                    }
                }
            }
        }
        Message::GroupEditToggle => state.group_editing = !state.group_editing,
        Message::GroupLinkToggle(idx, link) => {
            if let Some(g) = state.groups.get_mut(idx) {
                if state.group_editing {
                    // Assign the current multi-selection as this
                    // group's membership and engage this link type —
                    // the existing Ctrl/Shift-click selection is the
                    // whole "picker UI" here, deliberately, rather than
                    // a bespoke channel-assignment dialog.
                    g.members = state.selected.iter().copied().collect();
                    match link {
                        sidebar::GroupLink::Mute => g.mute_linked = true,
                        sidebar::GroupLink::Solo => g.solo_linked = true,
                        sidebar::GroupLink::Fader => g.fader_linked = true,
                    }
                } else {
                    match link {
                        sidebar::GroupLink::Mute => g.mute_linked = !g.mute_linked,
                        sidebar::GroupLink::Solo => g.solo_linked = !g.solo_linked,
                        sidebar::GroupLink::Fader => g.fader_linked = !g.fader_linked,
                    }
                }
            }
        }
        Message::GroupClear => {
            state.groups = Default::default();
            state.group_editing = false;
        }
        Message::LayoutClicked(n) => {
            state.active_layout = Some(n as usize);
            if let Some(set) = state.layouts.get((n - 1) as usize).cloned().flatten() {
                state.collapsed = set;
                // Bulk recall snaps instantly (matching Scene recall,
                // which doesn't animate faders either) — clear any
                // in-flight collapse animations so a stale one can't
                // fight the new target state.
                state.collapse_anim.clear();
            }
        }
        Message::LayoutStore => {
            if let Some(n) = state.active_layout {
                state.layouts[(n - 1) as usize] = Some(state.collapsed.clone());
                if let Err(e) = crate::layouts::save_layout_file(n as u8, &state.collapsed) {
                    log::warn!("Failed to store layout {n}: {e}");
                }
            }
        }
    }
    Task::none()
}

/// How long a failure stays on screen.
const NOTICE_FOR: Duration = Duration::from_secs(5);

/// Reports a refused device write: logged, and shown briefly so a fader that
/// moves without effect is not silent. A drag repeats the same failure many
/// times a second; the same text only refreshes the timer.
fn fail(state: &mut TuxMix, what: &str, error: &dyn std::fmt::Display) {
    let what = what.trim_start_matches("set_").replace('_', " ");
    let text = format!("Could not apply {what}: {error}");
    log::warn!("{text}");
    state.notice = Some((text, Instant::now()));
}

/// Hides the window in the tray by closing it: the process, and with it the
/// ticks, the front panel's DIM and the reconnect logic, keeps running.
fn hide_window(state: &mut TuxMix) -> Task<Message> {
    state.window_hidden = true;
    match state.window_id.take() {
        Some(id) => window::close(id),
        None => Task::none(),
    }
}

/// Brings the window back: opens it again if it was hidden, else raises it.
fn show_window(state: &mut TuxMix) -> Task<Message> {
    state.window_hidden = false;
    state.last_activity = Instant::now();
    if let Some(id) = state.window_id {
        return Task::batch([window::minimize(id, false), window::gain_focus(id)]);
    }
    let size = state
        .window_size
        .map(|(w, h)| iced::Size::new(w.max(MIN_WINDOW.0), h.max(MIN_WINDOW.1)))
        .unwrap_or(iced::Size::new(1280.0, 800.0));
    let (id, open) = window::open(crate::window_settings(size, state.fullscreen));
    state.window_id = Some(id);
    Task::batch([open.discard(), window::gain_focus(id)])
}

/// A cheap summary of what the front panel can change behind the app's back
/// (output levels, mutes, the dim): when it differs after a poll, the panel
/// was touched. Not a hash of the whole mix: just enough to notice.
fn panel_fingerprint(state: &TuxMix) -> f64 {
    state
        .device
        .outputs()
        .iter()
        .enumerate()
        .map(|(i, o)| {
            (o.volume as f64 + if o.mute { 1000.0 } else { 0.0 }) * (1.0 + i as f64 * 0.01)
        })
        .sum::<f64>()
        + if state.device.settings().dim { 7777.0 } else { 0.0 }
}

/// The tick while something is moving (meters, a drag, the front panel):
/// 30 Hz. Meters redraw on it and no longer in between (see
/// `widgets::fader::INTERPOLATE_METERS`), so it is also their frame rate.
const TICK_ACTIVE: Duration = Duration::from_millis(33);
/// The tick at rest. Every tick rebuilds and redraws the whole view, so at
/// rest it only has to keep the slow chores alive (connection probe,
/// auto-save, the front panel); anything that changes the screen switches
/// back to `TICK_ACTIVE` at once.
const TICK_IDLE: Duration = Duration::from_millis(500);
/// How long after the last change the tick stays fast: longer than the
/// meters' release, so a fading peak is drawn to the end.
const ACTIVE_FOR: Duration = Duration::from_millis(1500);

/// Whether the screen has anything moving, i.e. the tick should be fast.
fn is_active(state: &TuxMix) -> bool {
    state.last_activity.elapsed() < ACTIVE_FOR
        || state.pending_resize_width.is_some()
        || !state.collapse_anim.is_empty()
        || state.drag_range.is_some()
        || state.notice.is_some()
}

pub fn tick_interval(state: &TuxMix) -> Duration {
    if is_active(state) {
        TICK_ACTIVE
    } else {
        TICK_IDLE
    }
}

pub fn subscription(state: &TuxMix) -> Subscription<Message> {
    let mut subs = vec![
        iced::time::every(tick_interval(state)).map(|_| Message::Tick),
        iced::event::listen_with(handle_global_event),
    ];
    // Another app (or `pw-metadata` by hand) can change the graph clock;
    // re-read it now and then so the device card never shows a stale rate.
    if state.device.rate_follows_sound_server() {
        subs.push(iced::time::every(Duration::from_secs(3)).map(|_| Message::EnginePoll));
    }
    // Only running while a collapse/expand transition is actually in
    // flight — plain `column`/`container` widgets can't self-request a
    // redraw the way the canvas-based fader/meter animations do, so a
    // much faster timer stands in for that during the ~160ms transition,
    // then switches itself back off once `collapse_anim` empties out.
    if !state.collapse_anim.is_empty() {
        subs.push(iced::time::every(Duration::from_millis(8)).map(|_| Message::CollapseTick));
    }
    // The notification-area icon and second launches, see `tray`.
    subs.push(crate::tray::subscription().map(Message::Tray));
    // Only running when `--osc` was passed — see `osc.rs`.
    if let Some(config) = &state.osc_config {
        subs.push(Subscription::run_with(*config, osc::worker));
    }
    Subscription::batch(subs)
}

fn handle_global_event(
    event: iced::Event,
    status: iced::event::Status,
    id: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            // Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y. Left to a focused text field
            // (a rename or a typed dB value), which has its own undo.
            if modifiers.command() && status == iced::event::Status::Ignored {
                if let Key::Character(c) = &key {
                    match c.to_lowercase().as_str() {
                        "z" if modifiers.shift() => return Some(Message::Redo),
                        "z" => return Some(Message::Undo),
                        "y" => return Some(Message::Redo),
                        _ => {}
                    }
                }
            }
            // Ctrl+= / Ctrl+- / Ctrl+0 — manual zoom. Accept "=" and "+"
            // (shifted/unshifted) and "0"; requires Ctrl.
            if modifiers.contains(keyboard::Modifiers::CTRL) {
                if let Key::Character(c) = &key {
                    let zoom = match c.as_str() {
                        "=" | "+" => Some(Message::ZoomIn),
                        "-" | "_" => Some(Message::ZoomOut),
                        "0" => Some(Message::ZoomReset),
                        "," => Some(Message::ToggleSettings),
                        _ => None,
                    };
                    if zoom.is_some() {
                        return zoom;
                    }
                }
            }
            match key {
                Key::Named(keyboard::key::Named::F11) => {
                    Some(Message::ToggleFullscreen(id))
                }
                Key::Named(keyboard::key::Named::Tab) => Some(Message::TabPressed),
                Key::Named(keyboard::key::Named::Escape) => Some(Message::EscapePressed),
                _ => None,
            }
        }
        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::ModifiersChanged(m))
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
            Some(Message::PointerPressed)
        }
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta, .. }) => {
            // Reported every wheel tick; `update` decides whether it's a
            // zoom (Ctrl held) or leaves it alone (plain scroll handled by
            // the widget tree).
            let y = match delta {
                iced::mouse::ScrollDelta::Lines { y, .. } => y,
                iced::mouse::ScrollDelta::Pixels { y, .. } => y,
            };
            Some(Message::WheelZoom(y))
        }
        iced::Event::Window(window::Event::Resized(size)) => {
            Some(Message::WindowSize(size.width, size.height))
        }
        iced::Event::Window(window::Event::CloseRequested) => Some(Message::CloseRequested),
        iced::Event::Window(window::Event::Closed) => Some(Message::WindowClosed(id)),
        iced::Event::Window(window::Event::Opened { size, .. }) => {
            Some(Message::WindowSize(size.width, size.height))
        }
        _ => None,
    }
}

// ── View ─────────────────────────────────────────────────────────

pub fn view(state: &TuxMix, _window: window::Id) -> Element<'_, Message> {
    let top = top_bar(state);
    let content = match state.view {
        View::Quick => quick_view(state),
        View::Mixer => mixer_view(state),
        View::Matrix => matrix_view(state),
    };

    // Explicit Fill — a Shrink parent doesn't actually grant a Fill-sized
    // child the real window height for layout/hit-testing, even though
    // the raw window clear color visually fills the gap identically to
    // our own background (same near-black), making a real empty area
    // indistinguishable on screen from a genuinely non-interactive one.
    // That's what made `page()`'s click-to-clear-selection silently miss
    // every click below the shortest section's natural content height.
    // The sidebar runs the window's full height; the top bar, banner,
    // view and OSC drawer stack in the column beside it. (The bar used to
    // span the whole window above both, which left its right-hand tools
    // floating over the sidebar's own title, aligned with neither.)
    let mut main = column![top];
    if !state.connected {
        main = main.push(disconnected_banner(state));
    }
    main = main.push(content);
    if state.show_osc_log {
        main = main.push(osc_log_panel(state));
    }
    let main = main.width(Length::Fill).height(Length::Fill);
    let mut base: Element<'_, Message> = row![main, sidebar::sidebar(state)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    if state.show_device_panel {
        base = stack(vec![base, device_panel(state)])
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    if let Some((text, _)) = &state.notice {
        base = stack(vec![base, notice_toast(text, state.ui_scale)])
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    if state.show_settings {
        base = stack(vec![base, settings_panel(state)])
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    if state.show_about {
        // The About dialog rides above the whole window, drawers included:
        // it is a modal now, not one more docked panel (see
        // `about::about_modal`).
        stack(vec![base, about::about_modal(state)])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        base
    }
}

/// Shown under the top bar while the device is gone. The sidebar chip
/// already says "Disconnected", but that is a small word at the window's
/// edge, while the mixer underneath keeps looking live; a full-width line
/// says it where the eye is, and says what happens next.
fn disconnected_banner(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let model = state
        .device
        .model_name()
        .strip_suffix(" (mock)")
        .unwrap_or_else(|| state.device.model_name())
        .to_string();
    container(
        row![
            text("\u{25CF}").size(theme::TEXT_XS * scale).color(theme::MRED),
            text(format!(
                "{model} is disconnected. TuxMix reconnects on its own and restores the mix when it is back."
            ))
            .size(theme::TEXT_SM * scale)
            .color(theme::TEXT_PRIMARY),
        ]
        .spacing(theme::SPACE_MD * scale)
        .align_y(iced::Alignment::Center),
    )
    .padding([theme::SPACE_SM * scale, theme::SPACE_XXL * scale])
    .width(Length::Fill)
    .style(|_theme: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(theme::blend(
            theme::SURFACE,
            theme::MRED,
            0.22,
        ))),
        ..container::Style::default()
    })
    .into()
}

/// A fixed-height drawer docked under the main view, listing raw OSC
/// traffic in both directions — the same idea as oscmix's own OSC debug
/// log window, just docked into the single-window layout instead of a
/// separate floating one (`tuxmix-gui` doesn't use multi-window at all
/// elsewhere, so this stays consistent rather than being the one exception).
/// Newest line first, so the most recent activity is always visible at the
/// top without needing scroll-follow logic.
fn osc_log_panel(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    let header = row![
        text("OSC DEBUG LOG")
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_SM * scale),
        text(format!("{} lines", state.osc_log.len()))
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        iced::widget::Space::new().width(Length::Fill),
        iced::widget::button(text("Clear").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .style(theme::plain_button)
            .on_press(Message::ClearOscLog),
        iced::widget::button(text("Close").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .style(theme::plain_button)
            .on_press(Message::ToggleOscLog),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let body: Element<'_, Message> = if state.osc_log.is_empty() {
        text("No OSC traffic yet.")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale)
            .into()
    } else {
        let mut lines = column![].spacing(1.0);
        for line in &state.osc_log {
            lines = lines.push(
                text(line.clone())
                    .font(iced::Font::MONOSPACE)
                    .color(theme::TEXT_SEC)
                    .size(theme::TEXT_XS * scale),
            );
        }
        scrollable(lines)
            .direction(scrollable::Direction::Vertical(theme::thin_scrollbar()))
            .height(Length::Fill)
            .style(theme::scrollable)
            .into()
    };

    container(
        column![header, body]
            .spacing(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .style(theme::top_bar)
    .padding(theme::SPACE_MD * scale)
    .width(Length::Fill)
    .height(Length::Fixed(180.0 * scale))
    .into()
}

/// Settings that have no per-channel strip to live on (clock, sample
/// rate, pitch/width, the global protocol flags), as a popover card
/// beside the sidebar's device card, which opens it (or the folded rail's
/// gear). It was opened from a top-bar field captioned "Clock" first,
/// where nobody would look for the sample rate.
///
/// It used to be a full-width drawer docked under the main view: a strip
/// of loose rows pinned to the window's bottom-left corner, as far as the
/// window allows from the field that opened it. A card under that field,
/// closed by a click anywhere outside it (or by the field / Close), is
/// what the field promises, and sections give the flags a structure.
fn device_panel(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let settings = state.device.settings();
    let modifiers = state.modifiers;

    // Caption column shared by every labelled row, so the controls line up.
    const LABEL_W: f32 = 92.0;
    let labelled = |label: &'static str, control: Element<'static, Message>| -> Element<'static, Message> {
        row![
            text(label)
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale)
                .width(Length::Fixed(LABEL_W * scale)),
            control,
        ]
        .spacing(theme::SPACE_MD * scale)
        .align_y(iced::Alignment::Center)
        .into()
    };
    let section = |label: &'static str| -> Element<'static, Message> {
        row![
            text(label)
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_MICRO * scale),
            iced::widget::rule::horizontal(1),
        ]
        .spacing(theme::SPACE_MD * scale)
        .align_y(iced::Alignment::Center)
        .into()
    };

    let header = row![
        text("Device settings")
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_MD * scale),
        iced::widget::Space::new().width(Length::Fill),
        iced::widget::button(text("Close").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .style(theme::ghost_button)
            .on_press(Message::ToggleDevicePanel),
    ]
    .align_y(iced::Alignment::Center);

    // The device's supported sample-rate classes (alt 1/2/3 — see
    // PROTOCOL.md "Sample rate"): 32/44.1/48/64/88.2, 96/128, 176.4/192.
    // A custom entry is not editable, so the full supported set is listed.
    let rates: Vec<u32> = vec![
        32000, 44100, 48000, 64000, 88200, 96000, 128000, 176400, 192000,
    ];
    let clock = column![
        section("CLOCK"),
        labelled(
            "Clock source",
            pick_list(
                settings.clock_sources.clone(),
                Some(settings.clock_source.clone()),
                Message::ClockSourceSelected,
            )
            .style(theme::pick_list)
            .menu_style(theme::menu)
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .text_size(theme::TEXT_MD * scale)
            .width(Length::Fill)
            .into(),
        ),
    ]
    .spacing(theme::SPACE_MD * scale);
    // The USB backend streams the device itself, so the rate is the
    // device's to set, here. With the kernel driver the sound server owns
    // it: see the audio engine section below (this list used to show there
    // too, and silently did nothing).
    let clock = if state.device.rate_follows_sound_server() || state.device.is_mock() {
        clock
    } else {
        clock.push(labelled(
            "Sample rate",
            pick_list(rates, Some(settings.sample_rate), Message::SampleRateSelected)
                .style(theme::pick_list)
                .menu_style(theme::menu)
                .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
                .text_size(theme::TEXT_MD * scale)
                .width(Length::Fill)
                .into(),
        ))
    };

    let mut body = column![header, clock]
        .spacing(theme::SPACE_XL * scale)
        .width(Length::Fill);

    if state.device.rate_follows_sound_server() || state.device.is_mock() {
        body = body.push(engine_section(state, LABEL_W));
    }

    // NOTE: there used to be a "SPDIF: Enabled / Emphasis / Professional"
    // row here. Removed 2026-09-12: those were never Babyface Pro
    // features. They are generic IEC958 controls that `snd-usb-audio`
    // synthesizes for any class-compliant device with an S/PDIF
    // endpoint — RME's own Fireface USB Settings dialog for this card
    // offers no such options (its SPDIF settings are "Optical Out
    // ADAT/SPDIF" and "SPDIF In TMS"; see PROTOCOL.md's captured UI
    // reference). They reached TuxMix through the CC-mode ALSA surface,
    // and once `open()` started refusing CC-mode cards they were three
    // buttons that could not do anything on any backend. The real
    // optical-format control lives in the Global grid below.

    if state.device.protocol_features() {
        let knob_cell = |label: &'static str, k: Element<'static, Message>| -> Element<'static, Message> {
            column![
                k,
                text(label)
                    .color(theme::TEXT_SEC)
                    .size(theme::TEXT_XS * scale),
            ]
            .spacing(theme::SPACE_SM * scale)
            .align_x(iced::Alignment::Center)
            .width(Length::Fill)
            .into()
        };
        let output = column![
            section("OUTPUT"),
            row![
                knob_cell(
                    "Pitch",
                    knob(Knob {
                        value: settings.pitch_percent,
                        range: (-5.0, 5.0),
                        label: format!("{:+.1}%", settings.pitch_percent),
                        arc_from_center: true,
                        interactive: true,
                        log_scale: false,
                        modifiers,
                        scale,
                        on_change: Box::new(|v| Message::PitchChanged(v.clamp(-5.0, 5.0))),
                        on_reset: Box::new(|| Message::PitchChanged(0.0)),
                    }),
                ),
                knob_cell(
                    "Width",
                    knob(Knob {
                        value: settings.width,
                        range: (-1.0, 1.0),
                        label: format!("{:+.2}", settings.width),
                        arc_from_center: true,
                        interactive: true,
                        log_scale: false,
                        modifiers,
                        scale,
                        on_change: Box::new(|v| Message::WidthChanged(v.clamp(-1.0, 1.0))),
                        on_reset: Box::new(|| Message::WidthChanged(0.0)),
                    }),
                ),
            ],
        ]
        .spacing(theme::SPACE_MD * scale);

        // The global flags as a two-column grid of equal toggles, the same
        // family as the sidebar's group cells.
        let flags: [(&'static str, bool, fn(bool) -> Message); 5] = [
            ("MS Proc", settings.ms_proc, Message::MsProcChanged),
            ("AN 1>2", settings.an12, Message::An12Changed),
            ("Input Link", settings.input_link, Message::InputLinkChanged),
            ("EQ for Record", settings.eq_for_record, Message::EqForRecordChanged),
            ("Opt Out: SPDIF", settings.optical_out_spdif, Message::OpticalOutFormatChanged),
        ];
        let mut grid = column![section("GLOBAL")].spacing(theme::SPACE_SM * scale);
        for pair in flags.chunks(2) {
            let mut line = row![].spacing(theme::SPACE_SM * scale);
            for &(label, active, on_toggle) in pair {
                line = line.push(
                    iced::widget::button(text(label).size(theme::TEXT_XS * scale).center())
                        .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
                        .width(Length::Fill)
                        .style(theme::toggle_button(active, theme::ACCENT))
                        .on_press(on_toggle(!active)),
                );
            }
            if pair.len() == 1 {
                line = line.push(iced::widget::Space::new().width(Length::Fill));
            }
            grid = grid.push(line);
        }
        body = body.push(output).push(grid);
    } else {
        // Say why the card is shorter rather than leaving two knobs and
        // five toggles silently missing: on the class-compliant personality
        // everything below the sample rate lives in RME's own protocol.
        body = body.push(
            text(
                "Class Compliant mode: routing, preamps, gains and clock only. \
                 Pitch, Width, MS Proc, AN 1>2, Input Link, EQ for Record and \
                 the optical-out format are RME-protocol features, and the \
                 strips drop EQ, Trim, Phase \u{00d8}, Split and Loopback for the \
                 same reason.",
            )
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        );
    }

    popover(state, body.into(), Message::ToggleDevicePanel)
}

/// The card of a popover docked beside the sidebar, closing on `close` when
/// a click lands outside it.
fn popover<'a>(state: &'a TuxMix, body: Element<'a, Message>, close: Message) -> Element<'a, Message> {
    let scale = state.ui_scale;
    const CARD_W: f32 = 320.0;
    let card = opaque(
        container(body)
            .style(theme::dialog)
            .padding(theme::SPACE_XL * scale)
            .width(Length::Fixed(CARD_W * scale)),
    );

    // One layer, not a catcher stacked under the card: in a `stack`, the
    // top layer's full-window container reports a mouse interaction, which
    // makes iced levitate the cursor for every layer below it, so a
    // catcher underneath never saw a click. Here the card (`opaque`)
    // captures its own clicks first, and `mouse_area` only acts on the
    // uncaptured ones: a click anywhere outside the card closes it, like a
    // menu. No dimming: it is a popover, not a modal.
    opaque(
        mouse_area(
            container(card)
                .align_x(iced::Alignment::End)
                .padding(iced::Padding {
                    top: POPOVER_TOP * scale,
                    // Beside the device card that opens it, just off the
                    // sidebar's edge.
                    right: theme::SPACE_MD * scale + sidebar_px(state),
                    bottom: 0.0,
                    left: 0.0,
                })
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(close),
    )
}

/// A refused device write, as a small card at the foot of the window. It
/// takes no clicks (a `Space`-sized layer would levitate the cursor for the
/// mixer below): the card alone is as big as its text.
fn notice_toast(message: &str, scale: f32) -> Element<'_, Message> {
    container(
        container(
            text(message.to_string())
                .size(theme::TEXT_SM * scale)
                .color(theme::TEXT_PRIMARY),
        )
        .padding([theme::SPACE_SM * scale, theme::SPACE_LG * scale])
        .style(theme::dialog),
    )
    .padding(theme::SPACE_XL * scale)
    .center_x(Length::Fill)
    .align_bottom(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The app's own settings (as opposed to the device's): how TuxMix behaves
/// around its window, not what the card does.
fn settings_panel(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let header = row![
        text("TuxMix settings")
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_MD * scale),
        iced::widget::Space::new().width(Length::Fill),
        iced::widget::button(text("Close").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .style(theme::ghost_button)
            .on_press(Message::ToggleSettings),
    ]
    .align_y(iced::Alignment::Center);
    let section = row![
        text("BACKGROUND")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_MICRO * scale),
        iced::widget::rule::horizontal(1),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);
    let toggle = |label: &'static str, active: bool, message: Message| {
        iced::widget::button(text(label).size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(active, theme::ACCENT))
            .on_press(message)
    };
    let mut background = column![section].spacing(theme::SPACE_SM * scale);
    if state.tray_active {
        background = background.push(toggle(
            "Keep running in the tray when closed",
            state.keep_in_tray,
            Message::SetKeepInTray(!state.keep_in_tray),
        ));
    } else {
        background = background.push(
            text("No system tray found: closing the window quits.")
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale),
        );
    }
    background = background.push(toggle(
        "Start with the session, hidden in the tray",
        state.autostart,
        Message::SetAutostart(!state.autostart),
    ));
    let body = column![header, background]
        .spacing(theme::SPACE_XL * scale)
        .width(Length::Fill);
    popover(state, body.into(), Message::ToggleSettings)
}

/// A sample rate as people say it: "48 kHz", "44.1 kHz", "176.4 kHz".
pub(crate) fn rate_label(hz: u32) -> String {
    if hz % 1000 == 0 {
        format!("{} kHz", hz / 1000)
    } else {
        format!("{:.1} kHz", hz as f32 / 1000.0)
    }
}

/// The sidebar's current width on screen, open or folded.
pub(crate) fn sidebar_px(state: &TuxMix) -> f32 {
    let logical = if state.sidebar_open {
        state.sidebar_width
    } else {
        sidebar::COLLAPSED_WIDTH
    };
    logical * sidebar_scale(state)
}

/// Sample rate and buffer size, as PipeWire runs them (see
/// `tuxmix_core::pw_clock`). These are the settings a Windows user finds in
/// RME's driver dialog; on Linux they belong to the sound server and are
/// system-wide, which the section says rather than hides.
fn engine_section(state: &TuxMix, label_w: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let caption = |label: &'static str| {
        text(label)
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale)
            .width(Length::Fixed(label_w * scale))
    };
    let heading = row![
        text(if state.device.is_mock() {
            "AUDIO ENGINE \u{00b7} PIPEWIRE (SIMULATED)"
        } else {
            "AUDIO ENGINE \u{00b7} PIPEWIRE"
        })
        .color(theme::TEXT_SEC)
        .size(theme::TEXT_MICRO * scale),
        iced::widget::rule::horizontal(1),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let Some(clock) = state.engine else {
        return column![
            heading,
            text(
                "PipeWire is not running: the sample rate and buffer size are \
                 set by whatever opens the card.",
            )
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        ]
        .spacing(theme::SPACE_MD * scale)
        .width(Length::Fill)
        .into();
    };

    // "Auto" is PipeWire's own value (it may differ per graph); naming it
    // keeps the default visible next to the overrides.
    const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
    // 48 and 96 are 1 and 2 ms at 48 kHz, whole USB packets; PipeWire takes
    // quanta that are not powers of two. 16 and 24 only when the driver can
    // run them: its period floor is one URB, `urb_frames_min` (32 unless
    // the module is loaded lower). Measured 2026-10-21 (the driver's
    // KERNEL-DRIVER.md): 10-minute real-time soaks at 16 and 24 ran clean
    // once loaded with `urb_frames_min=8`; 8 still loses capture buffers,
    // so it is never offered.
    const ALL_QUANTA: [u32; 11] = [16, 24, 32, 48, 64, 96, 128, 256, 512, 1024, 2048];
    let floor = state.driver_floor.unwrap_or(32);
    let quanta: Vec<u32> = ALL_QUANTA.iter().copied().filter(|q| *q >= floor).collect();
    let rate_opts: Vec<String> = std::iter::once(format!("Auto ({})", rate_label(clock.rate)))
        .chain(RATES.iter().map(|r| rate_label(*r)))
        .collect();
    let rate_sel = if clock.force_rate == 0 {
        rate_opts[0].clone()
    } else {
        rate_label(clock.force_rate)
    };
    // "Auto" without a number: with nothing forced, the card runs what
    // its active clients ask for, 256 by default from the WirePlumber rule
    // (`node.latency`), not PipeWire's global `clock.quantum`, which this
    // label used to show. The latency line below shows what really runs.
    let quantum_opts: Vec<String> = std::iter::once("Auto".to_string())
        .chain(quanta.iter().map(|q| format!("{q} samples")))
        .collect();
    let quantum_sel = if clock.force_quantum == 0 {
        quantum_opts[0].clone()
    } else {
        format!("{} samples", clock.force_quantum)
    };
    let field = |p: iced::widget::PickList<'static, String, Vec<String>, String, Message>| {
        p.style(theme::pick_list)
            .menu_style(theme::menu)
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .text_size(theme::TEXT_MD * scale)
            .width(Length::Fill)
    };
    let rate_list = rate_opts.clone();
    let rate_pick = field(pick_list(rate_opts, Some(rate_sel), move |label| {
        let idx = rate_list.iter().position(|l| *l == label).unwrap_or(0);
        Message::EngineRate(if idx == 0 { 0 } else { RATES[idx - 1] })
    }));
    let quantum_list = quantum_opts.clone();
    let quanta_for_pick = quanta.clone();
    let quantum_pick = field(pick_list(quantum_opts, Some(quantum_sel), move |label| {
        let idx = quantum_list.iter().position(|l| *l == label).unwrap_or(0);
        Message::EngineQuantum(if idx == 0 { 0 } else { quanta_for_pick[idx - 1] })
    }));

    column![
        heading,
        row![caption("Sample rate"), rate_pick]
            .spacing(theme::SPACE_MD * scale)
            .align_y(iced::Alignment::Center),
        row![caption("Buffer size"), quantum_pick]
            .spacing(theme::SPACE_MD * scale)
            .align_y(iced::Alignment::Center),
        row![caption("Latency"), latency_readout(state, clock)]
            .spacing(theme::SPACE_MD * scale)
            .align_y(iced::Alignment::Center),
    ]
    .spacing(theme::SPACE_MD * scale)
    .width(Length::Fill)
    .into()
}

/// The latency line, from the card's own stream when it is playing: the
/// quantum is only the graph's, and the sound server may open the card with
/// a fixed hardware period of its own (WirePlumber's `api.alsa.period-size`
/// in IRQ mode; `tools/alsa/wireplumber-rme.conf` pinned 256 until
/// 2026-10-21 and no longer does, but a user rule still can). Then
/// the buffer size above does not reach the card, and this says so instead
/// of showing the quantum's latency as the card's.
fn latency_readout(state: &TuxMix, clock: PwClock) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let line = |s: String, c: Color| text(s).color(c).size(theme::TEXT_XS * scale);
    match state.card_stream {
        Some(card) => {
            let mut col = column![line(
                format!(
                    "{:.1} ms \u{00b7} {} samples at {}",
                    card.period_ms(),
                    card.period,
                    rate_label(card.rate)
                ),
                theme::TEXT_PRIMARY,
            )]
            .spacing(theme::SPACE_TIGHT * scale);
            // Only when the user asked for a size and the card ignores it
            // (a fixed period in a WirePlumber rule); "Auto" follows clients.
            if clock.force_quantum != 0 && card.period != clock.effective_quantum() {
                col = col.push(line(
                    "The card keeps its own fixed period.".into(),
                    theme::YSIM,
                ));
            }
            col.into()
        }
        None if state.device.is_mock() => line(
            format!(
                "{:.1} ms per buffer at {}",
                clock.latency_ms(),
                rate_label(clock.effective_rate())
            ),
            theme::TEXT_PRIMARY,
        )
        .into(),
        None => line(
            format!(
                "{:.1} ms per buffer at {}",
                clock.latency_ms(),
                rate_label(clock.effective_rate())
            ),
            theme::TEXT_PRIMARY,
        )
        .into(),
    }
}

/// Where the device popover's card starts, from the window top at scale 1:
/// level with the sidebar's device card, which opens it.
const POPOVER_TOP: f32 = theme::TOP_BAR_H;

/// A section label (HARDWARE INPUTS, SOFTWARE PLAYBACK, ...) with an accent
/// tick and a rule trailing off to the right, instead of bare gray text that
/// blends into the background.
fn section_header(label: &str, scale: f32) -> Element<'_, Message> {
    row![
        container(
            iced::widget::Space::new()
                .width(3.0 * scale)
                .height(12.0 * scale)
        )
        .style(theme::accent_bar),
        text(label)
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_MD * scale),
        iced::widget::rule::horizontal(1),
    ]
    .spacing(theme::SPACE_LG)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Wraps a view's body in the root background, filling the window, with a
/// vertical scrollbar for when the stacked sections (or the matrix grid)
/// don't fit the window height.
fn page<'a>(body: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    mouse_area(
        container(
            scrollable(body)
                .direction(scrollable::Direction::Vertical(theme::thin_scrollbar()))
                .width(Length::Fill)
                .style(theme::scrollable),
        )
        .style(theme::root)
        .padding([theme::SPACE_LG, theme::SPACE_XL])
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .on_press(Message::ClearSelection)
    .into()
}

fn top_bar(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    // The device identity chip (name + connected/simulated status) used
    // to live here — removed as a straight duplicate now that the
    // sidebar's own device chip shows the same thing (see
    // `sidebar::device_chip`, which absorbed the status dot/label this
    // one used to carry). Everything else in this bar (view tabs, the
    // Scene/Submix/Clock session tools) has no sidebar equivalent, so it
    // stays — TotalMix's own control strip doesn't replace *this* bar's
    // job, and neither does ours.

    // View switch: a plain segmented toggle, not a chip — it's navigation,
    // not a status readout, so it shouldn't carry the same visual weight
    // as the identity chip. Both labels are always visible and clickable
    // (previously only the active view's name showed, with no click
    // target — Tab-key was the only way to switch).
    let tab_toggle = row![
        iced::widget::button(text("QUICK").size(theme::TEXT_MD * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_XL * scale])
            .style(theme::tab_toggle(state.view == View::Quick))
            .on_press(Message::SetView(View::Quick)),
        iced::widget::button(text("MIXER").size(theme::TEXT_MD * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_XL * scale])
            .style(theme::tab_toggle(state.view == View::Mixer))
            .on_press(Message::SetView(View::Mixer)),
        iced::widget::button(text("MATRIX").size(theme::TEXT_MD * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_XL * scale])
            .style(theme::tab_toggle(state.view == View::Matrix))
            .on_press(Message::SetView(View::Matrix)),
    ]
    .spacing(theme::SPACE_TIGHT);

    // Secondary session tools: submix / clock. Scene save/load used to
    // live here too — moved into the sidebar's Snapshots panel (see
    // `sidebar::snapshots_panel`'s own comment), since it's the exact
    // same `save_scene_file`/`load_scene_file` mechanism as the Mix 1-8
    // slots there, just with free-form names instead of fixed ones —
    // no reason to keep two separate homes for one feature.
    // Flat fields, like everything else in the bar: this was the bar's
    // only boxed element (a `chip`), and read as dropped into the corner.
    let session = row![
            text("Submix")
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale),
            pick_list(
                OUT_LABELS.to_vec(),
                Some(OUT_LABELS[state.sel_out]),
                |label| {
                    let idx = OUT_LABELS.iter().position(|l| *l == label).unwrap_or(0);
                    Message::SelectOutput(idx)
                },
            )
            .style(theme::pick_list)
            .menu_style(theme::menu)
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .text_size(theme::TEXT_MD * scale),

    ]
    .spacing(theme::SPACE_SM * scale)
    .align_y(iced::Alignment::Center);

    // The mark and the wordmark as one lockup, the same artwork the window
    // icon wears (see `icon::mark`), flat white like every other engaged
    // state and sized off the wordmark's own height. A missing asset just
    // falls back to the plain wordmark.
    let brand_row: Element<'_, Message> = match crate::icon::mark() {
        Some((handle, ratio)) => {
            let mark_h = 20.0 * scale;
            row![
                iced::widget::image(handle)
                    .width(mark_h * ratio)
                    .height(mark_h)
                    .filter_method(iced::widget::image::FilterMethod::Linear),
                text("TuxMix")
                    .color(theme::ACCENT)
                    .size(theme::TEXT_XL * scale),
            ]
            .spacing(theme::SPACE_SM * scale)
            .align_y(iced::Alignment::Center)
            .into()
        }
        None => text("TuxMix")
            .color(theme::ACCENT)
            .size(theme::TEXT_XL * scale)
            .into(),
    };

    // The lockup is the way into About: the product's face opens the
    // product's own dialog, which is also why the separate "About" button
    // that used to sit at the far right of this bar is gone. A ghost
    // button keeps it looking like the wordmark at rest and gives it a
    // hover target and a pointer cursor.
    let brand = iced::widget::button(brand_row)
        .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
        .style(theme::ghost_button)
        .on_press(Message::ToggleAbout);

    let mut bar = row![
        brand,
        tab_toggle,
        // Everything right of here is the tool side of the bar: the
        // flexible space sits in the middle, so the session chip stays
        // against the right edge instead of floating at whatever width it
        // used to claim for itself.
        iced::widget::Space::new().width(Length::Fill),
    ]
    .spacing(theme::SPACE_XXL)
    .align_y(iced::Alignment::Center);

    // Only when `--osc` is actually running — otherwise there's nothing
    // for the log to show, and the button would just be dead weight.
    if state.osc_config.is_some() {
        bar = bar.push(
            iced::widget::button(text("OSC LOG").size(theme::TEXT_MD * scale))
                .padding([theme::SPACE_SM * scale, theme::SPACE_XL * scale])
                .style(theme::tab_toggle(state.show_osc_log))
                .on_press(Message::ToggleOscLog),
        );
    }
    // The session chip hugs its own content (Submix + Clock) at the right
    // edge: it used to be stretched across `FillPortion(20)` inside a
    // scrollable, which painted a mostly-empty recessed pill across the
    // middle of the window. Content-sized, it reads as the bar's
    // right-hand tool cluster. The scrollable went with the stretch: at
    // the chip's real width there is nothing left to overflow.
    bar = bar.push(session);

    // A fixed height (`theme::TOP_BAR_H`) the sidebar's title row is
    // aligned to: the bar spans the mixer's width only, and the sidebar
    // runs the window's full height beside it, its title on the same line.
    container(bar)
        .style(theme::top_bar)
        .padding([0.0, theme::SPACE_XXL * scale])
        .height(Length::Fixed(theme::TOP_BAR_H * scale))
        .align_y(iced::Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// Whether playback channel `idx` has a real level. Mock has its own
/// generated meters; real backends get them from the PipeWire monitor,
/// whose channel count depends on the card profile (2 = PB1 only, 12 =
/// all six pairs). See `pw_meter`.
fn playback_meter_available(state: &TuxMix, idx: usize) -> bool {
    if state.device.is_mock() {
        state.device.has_playback_meters()
    } else {
        idx < state.playback_tap_channels
    }
}

/// Whether an output's host-computed meter is complete: every source
/// routed into that output pair must itself have a real meter. Inputs
/// are all real with the updated ALSA driver; playbacks depend on the
/// PipeWire tap's channel count (2 = PB1 only, 12 = all six pairs). An
/// output with an unmetered nonzero crosspoint stays N/A instead of
/// showing a partial sum.
fn output_meter_available(state: &TuxMix, out_idx: usize) -> bool {
    let pair = if state.device.outputs_one_per_pair() {
        out_idx
    } else {
        out_idx / 2
    };
    let inputs_ok = state.device.inputs().iter().enumerate().all(|(i, ch)| {
        ch.volumes.get(pair).copied().unwrap_or(0.0) == 0.0
            || state.device.has_input_meter(i)
    });
    let playbacks_ok = state
        .device
        .playbacks()
        .iter()
        .enumerate()
        .all(|(i, ch)| {
            ch.volumes.get(pair).copied().unwrap_or(0.0) == 0.0
                || playback_meter_available(state, i)
        });
    inputs_ok && playbacks_ok
}

/// Builds the full `StripParams` for any channel — the field-by-field
/// logic (which meter buffer, which overrides, which default level, pan
/// vs. no-pan for outputs) used to be duplicated once per channel kind
/// inline in `mixer_view`'s three loops; factored out so `quick_view` can
/// render the exact same strip (just at a bigger `scale`) without a fourth
/// copy of it.
fn strip_params<'a>(
    state: &'a TuxMix,
    cid: ChannelId,
    output_idx: usize,
) -> strip::StripParams<'a> {
    let drag_range = state
        .drag_range
        .and_then(|(dc, lo, hi)| (dc == cid).then_some((lo, hi)));
    // An OUTPUT strip's fader IS that output's master — the strip's
    // `out` (used in the VolumeChanged/Reset messages) must be the
    // output channel index, NOT the selected submix `output_idx`
    // (which is only meaningful for input/playback crosspoints).
    // `set_volume(Output(i), out, v)` writes `output_for(out)` — so
    // out must equal i or the wrong master moves.
    let output_idx = match cid {
        ChannelId::Output(i) => i,
        _ => output_idx,
    };
    let base = strip::StripParams {
        cid,
        output_idx,
        // The three per-kind literals below spread this one, so the
        // capability is read once. See `RmeDevice::protocol_features`.
        protocol_features: state.device.protocol_features(),
        name: String::new(),
        type_tag: None,
        vol: 0.0,
        pan: 0,
        meter: fader::MeterFrame::still(0.0),
        meter_available: false,
        meter2: None,
        meter2_available: false,
        has_48v: false,
        has_pad: false,
        phantom: false,
        pad: false,
        has_gain: false,
        gain: 0,
        gain_max: 0,
        has_sensitivity: false,
        sensitivity_plus4: false,
        has_eq: false,
        eq_enabled: false,
        has_trim: false,
        trim: 0.0,
        has_phase: false,
        phase: false,
        loopback: false,
        stereo_linked: false,
        open_flyout: state
            .flyout_open
            .iter()
            .find(|(c, _)| *c == cid)
            .map(|(_, k)| *k),
        mute: false,
        solo: false,
        mute_armed: false,
        solo_armed: false,
        has_cue: false,
        cue: false,
        default_vol: 1.0,
        editing: state.editing == Some(cid),
        edit_buf: &state.edit_buf,
        renaming: false,
        rename_buf: "",
        drag_range,
        modifiers: state.modifiers,
        collapsed: state.collapsed.contains(&cid),
        collapse_anim: state.collapse_anim.get(&cid).copied(),
        scale: state.ui_scale,
        selected: state.selected.contains(&cid),
        hovered: state.hovered_strip == Some(cid),
    };

    match cid {
        ChannelId::Input(i) => {
            let ch = &state.device.inputs()[i];
            let has_48v = ch.channel_type == ChannelType::Mic;
            strip::StripParams {
                name: ch.name.clone(),
                type_tag: Some(type_tag(ch.channel_type)),
                vol: ch.volumes[output_idx],
                pan: ch.pans[output_idx],
                meter: state
                    .input_meters
                    .get(i)
                    .map(MeterAnim::frame)
                    .unwrap_or_else(|| fader::MeterFrame::still(0.0)),
                meter_available: state.device.has_input_meter(i),
                has_48v,
                has_pad: has_48v,
                phantom: ch.phantom,
                pad: ch.pad,
                has_gain: ch.gain_max.is_some(),
                gain: ch.gain.unwrap_or(0),
                gain_max: ch.gain_max.unwrap_or(0),
                has_sensitivity: ch.sensitivity.is_some(),
                sensitivity_plus4: ch.sensitivity == Some(Sensitivity::Plus4dBu),
                has_eq: ch.eq.is_some(),
                eq_enabled: ch.eq.is_some_and(|e| e.enabled),
                has_trim: true,
                trim: ch.trim,
                has_phase: ch.eq.is_some(),
                phase: ch.phase,
                stereo_linked: state.device.input_pair_linked(i / 2),
                mute: ch.mute,
                solo: ch.solo,
                default_vol: 1.0,
                ..base
            }
        }
        ChannelId::Playback(i) => {
            let ch = &state.device.playbacks()[i];
            strip::StripParams {
                name: ch.name.clone(),
                type_tag: Some(("PB", PB_TAG)),
                vol: ch.volumes[output_idx],
                pan: ch.pans[output_idx],
                meter: state
                    .playback_meters
                    .get(i)
                    .map(MeterAnim::frame)
                    .unwrap_or_else(|| fader::MeterFrame::still(0.0)),
                meter_available: playback_meter_available(state, i),
                stereo_linked: state.device.playback_linked(i / 2),
                mute: ch.mute,
                solo: ch.solo,
                default_vol: 1.0,
                ..base
            }
        }
        ChannelId::Output(i) => {
            let ch = &state.device.outputs()[i];
            strip::StripParams {
                name: ch.name.clone(),
                type_tag: Some(("OUT", OUT_TAG)),
                vol: ch.volume,
                pan: 0,
                meter: state
                    .output_meters
                    .get(i)
                    .map(MeterAnim::frame)
                    .unwrap_or_else(|| fader::MeterFrame::still(0.0)),
                // Host-computed from routed sources: available only
                // when every nonzero crosspoint feeding this pair has
                // a real meter (see `output_meter_available`).
                meter_available: output_meter_available(state, i),
                loopback: ch.loopback,
                stereo_linked: state.device.output_linked(i / 2),
                mute: ch.mute,
                solo: ch.solo,
                has_cue: true,
                cue: ch.cue,
                default_vol: 1.0,
                ..base
            }
        }
    }
}

/// Minimum interval between two applied window-resize updates. A live window
/// drag fires `Resized` events at pointer/mouse rate — often several between
/// two display frames — and each one that mutates state forces a full view
/// rebuild. Throttling to ~one per frame stops the UI from doing N rebuilds
/// for N intermediate widths that most of them never even got shown; the
/// latest width is stashed and applied when the window elapses (see
/// `apply_pending_resize`). ~16 ms keeps it at or just under display-refresh
/// cadence without feeling laggy.
const RESIZE_THROTTLE: Duration = Duration::from_millis(16);

/// How often `Message::Tick` probes whether the device is still there
/// (`RmeDevice::is_connected`): the tick itself runs at 20 Hz, and a card
/// does not vanish between two frames. Doubles as the reconnect retry
/// cadence while disconnected.
const CONN_CHECK_MS: u64 = 1000;

/// Apply the stashed resize width to `window_width`/`ui_scale`, but at most
/// once per `RESIZE_THROTTLE` — see `Message::WindowResized` and the const
/// above for why. No-op when there's nothing pending or the throttle window
/// hasn't elapsed yet.
fn apply_pending_resize(state: &mut TuxMix) {
    let Some(width) = state.pending_resize_width else {
        return;
    };
    let due = match state.last_resize_applied {
        Some(t) => t.elapsed() >= RESIZE_THROTTLE,
        None => true,
    };
    if !due {
        return;
    }
    state.pending_resize_width = None;
    state.last_resize_applied = Some(Instant::now());
    state.window_width = width;
}

// ── Scale: initial zoom, fit, sidebar ────────────────────────────

/// The smallest window the app opens at (logical px). Width: a strip row at
/// the minimum zoom plus the folded rail and the top bar's tools; height: one
/// row of strips under the top bar (the rest scrolls). It was 960x600, a
/// round number rather than a measured one, and it kept a window that fits
/// everything at 80 % from being any smaller.
pub(crate) const MIN_WINDOW: (f32, f32) = (640.0, 420.0);

/// Whether, in a window `w` wide, the sidebar would leave the widest strip
/// row less room than it needs at the current zoom (the sidebar then folds
/// itself, see `Message::WindowSize`). Measured against the real row widths
/// rather than a fixed threshold: at 80 % everything fits beside the
/// sidebar in 970 px, at 130 % it needs more than 1300.
fn sidebar_does_not_fit(state: &TuxMix, w: f32) -> bool {
    let (inputs, playbacks, outputs) = unit_row_widths(state);
    let row = inputs.max(playbacks).max(outputs) * state.ui_scale;
    let sidebar = state.sidebar_width * sidebar_scale(state);
    // Slack so the fold is not on the very edge of "just fits".
    w < row + sidebar + 2.0 * theme::SPACE_XL + 4.0 + 24.0
}
/// The sidebar scales with the zoom only up to this: it is a column of
/// small controls, and at the strips' 175 % it was as wide as half a
/// small window for no gain.
const SIDEBAR_SCALE_MAX: f32 = 1.25;
/// A starting zoom for a monitor of `height` logical pixels (the OS scale
/// factor is already in it): 100 % at 1080p and below, more on bigger
/// screens, where the 10-13 px text is small. Only used at the first
/// launch; the user's choice is remembered after that.
pub(crate) fn initial_zoom_for(height: f32) -> f32 {
    if height >= 2000.0 {
        1.5
    } else if height >= 1300.0 {
        1.25
    } else if height >= 1100.0 {
        1.1
    } else {
        theme::SCALE_DEFAULT
    }
}

/// The scale the sidebar draws at: the zoom, capped (`SIDEBAR_SCALE_MAX`).
pub(crate) fn sidebar_scale(state: &TuxMix) -> f32 {
    state.ui_scale.min(SIDEBAR_SCALE_MAX)
}

/// One strip's width at scale 1 (collapsed or full).
fn unit_strip_width(state: &TuxMix, cid: ChannelId) -> f32 {
    if state.collapsed.contains(&cid) {
        strip::COLLAPSED_W
    } else {
        strip::full_width(cid)
    }
}

/// The widths, at scale 1, of the mixer's three rows (inputs, playbacks,
/// outputs): a linked pair is one strip, a split one two.
fn unit_row_widths(state: &TuxMix) -> (f32, f32, f32) {
    let row = |n_pairs: usize, linked: &dyn Fn(usize) -> bool, id: &dyn Fn(usize) -> ChannelId| {
        let mut w = 0.0;
        let mut n = 0usize;
        for pair in 0..n_pairs {
            let l = pair * 2;
            let shown: &[usize] = if linked(pair) { &[0] } else { &[0, 1] };
            for k in shown {
                w += unit_strip_width(state, id(l + k));
                n += 1;
            }
        }
        w + n.saturating_sub(1) as f32 * theme::SPACE_MD
    };
    (
        row(
            state.device.inputs().len() / 2,
            &|p| state.device.input_pair_linked(p),
            &ChannelId::Input,
        ),
        row(
            state.device.playbacks().len() / 2,
            &|p| state.device.playback_linked(p),
            &ChannelId::Playback,
        ),
        row(
            state.device.output_pair_count(),
            &|p| state.device.output_linked(p),
            &ChannelId::Output,
        ),
    )
}

/// How much one zoom step changes `ui_scale` (a Ctrl+molette notch, or one
/// Ctrl+= / Ctrl+- press). The scale is decoupled from window size — fixed
/// geometry, rows and pages scroll — so this is the only thing that moves it.
const ZOOM_STEP: f32 = 0.1;

/// Moves `ui_scale` by `delta`, clamped to the manual-zoom range
/// (`SCALE_MIN`..`SCALE_MAX`). `ZoomReset` sets `SCALE_DEFAULT` directly.
fn zoom(state: &mut TuxMix, delta: f32) {
    state.ui_scale = (state.ui_scale + delta).clamp(theme::SCALE_MIN, theme::SCALE_MAX);
}

/// A strip's on-screen width at the current zoom, matching whatever
/// `strip::strip()` actually renders (collapsed vs. full) — used to
/// decide if a row fits without scrolling, see `responsive_row`.
fn rendered_strip_width(state: &TuxMix, cid: ChannelId) -> f32 {
    let w = if state.collapsed.contains(&cid) {
        strip::COLLAPSED_W
    } else {
        strip::full_width(cid)
    };
    w * state.ui_scale
}

/// Wraps a strip row: always left-aligned (stuck to the window's left
/// edge), like a real mixer's channel strips, whether or not it fits —
/// falls back to the horizontal scrollable only once it doesn't.
fn responsive_row<'a>(
    content: Element<'a, Message>,
    content_width: f32,
    available_width: f32,
) -> Element<'a, Message> {
    if content_width <= available_width {
        content
    } else {
        scrollable(content)
            .direction(scrollable::Direction::Horizontal(theme::thin_scrollbar()))
            .style(theme::scrollable)
            .into()
    }
}

/// The route flyout's content — one button per output bus, current
/// selection highlighted. `width` is the caller's animated tween value;
/// `.clip(true)` keeps the list from spilling out past it mid-animation.
fn route_popover(state: &TuxMix, width: f32, scale: f32) -> Element<'_, Message> {
    let mut list = column![].spacing(theme::SPACE_TIGHT);
    for (idx, label) in OUT_LABELS.iter().enumerate() {
        list = list.push(
            button(text(*label).size(theme::TEXT_SM * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                // The bus a strip routes to is a position, not an "on"
                // state: the graphite thumb, not the white fill.
                .style(theme::segment_button(idx == state.sel_out))
                .on_press(Message::SelectOutput(idx)),
        );
    }
    // Floats over the next strips, so it is a raised card with a shadow,
    // like the device popover; it used to be the top bar's flat fill and
    // read as painted onto the strips under it.
    container(list)
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(width))
        .style(theme::dialog)
        .into()
}

/// The card a strip's settings / trim / EQ panel sits in: the strip's own
/// card style, so the panel reads as the strip opened up rather than as
/// loose controls on the page. These used to take the top bar's flat fill
/// (no border, a shade off the page), and only the controls showed.
fn flyout_card<'a>(
    content: impl Into<Element<'a, Message>>,
    width: f32,
    scale: f32,
) -> Element<'a, Message> {
    container(content)
        .padding(theme::SPACE_MD * scale)
        .width(Length::Fixed(width))
        // As tall as the strip beside it, like the EQ panel.
        .height(Length::Fill)
        .style(theme::panel)
        .clip(true)
        .into()
}

/// The settings flyout's content — every channel kind gets the STEREO
/// link/split toggle; Input additionally gets 48V/PAD/sensitivity/gain,
/// gated exactly like the strip itself used to gate them inline (built
/// from the same `strip_params()` every strip already uses, so that
/// has_48v/etc. logic isn't duplicated here).
///
/// Gain used to be excluded: a `Knob` (Canvas widget) placed inside this
/// flyout back when it was a `Stack`-based overlay broke input handling
/// for the *entire* row after the first click. Now that this flyout
/// pushes the row instead of overlaying it (no more `Stack`/`opaque`, see
/// `with_flyout`), that failure mode's precondition is gone, so Gain
/// moved in with the rest.
fn settings_popover(state: &TuxMix, cid: ChannelId, width: f32, scale: f32) -> Element<'_, Message> {
    let p = strip_params(state, cid, state.sel_out);

    let mut col = column![].spacing(theme::SPACE_SM * scale);

    // Stereo link/split — every channel kind has this (TotalMix shows
    // the button on every strip, not just AN1/2 or hardware outputs; see
    // `RmeDevice::{input_pair,playback,output}_linked`'s doc comments for
    // why this is a pure display/control-grouping concept, not tied to
    // a specific hardware register).
    // A playback pair's link is its hardware stereo split (the pair goes
    // split-mono into AN1/2), an RME-protocol register: in Class Compliant
    // mode the switch is shown but inert, saying why.
    let can_toggle = !matches!(cid, ChannelId::Playback(_)) || p.protocol_features;
    let tip = match (cid, can_toggle) {
        (ChannelId::Playback(_), false) => {
            "Splitting a playback pair needs the RME protocol (not in Class Compliant mode)"
        }
        (ChannelId::Playback(_), true) => {
            "Stereo: one strip for the pair. Off: two mono strips, split L/R into AN1/2"
        }
        _ => "Stereo: one strip for the pair. Off: the two channels as separate strips",
    };
    col = col.push(hint(
        button(text("STEREO").size(theme::TEXT_SM * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(p.stereo_linked, theme::ACCENT))
            .on_press_maybe(
                can_toggle.then_some(Message::StereoLinkChanged(cid, !p.stereo_linked)),
            ),
        tip,
        scale,
    ));

    let ChannelId::Input(idx) = cid else {
        return flyout_card(col, width, scale);
    };

    if p.has_gain {
        let gain_max = p.gain_max;
        col = col.push(
            row![
                text("Gain")
                    .size(theme::TEXT_SM * scale)
                    .color(theme::TEXT_SEC),
                container(knob(Knob {
                    value: p.gain as f32,
                    range: (0.0, gain_max as f32),
                    // Gain is tracked in dB (0-65 Mic / 0-18 Instr),
                    // 1 dB steps, like TotalMix.
                    label: p.gain.to_string(),
                    arc_from_center: false,
                    interactive: true,
                    log_scale: false,
                    modifiers: p.modifiers,
                    scale,
                    on_change: Box::new(move |v| Message::Gain(
                        idx,
                        (v.round() as u32).min(gain_max)
                    )),
                    on_reset: Box::new(move || Message::Gain(idx, 0)),
                }))
                .width(Length::Fill)
                .align_x(iced::Alignment::End),
            ]
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
        );
    }

    if p.has_48v || p.has_pad {
        let mut tg_row = row![].spacing(theme::SPACE_TIGHT).width(Length::Fill);
        if p.has_48v {
            tg_row = tg_row.push(
                button(text("48V").size(theme::TEXT_SM * scale))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                    .width(Length::Fill)
                    .style(theme::toggle_button(p.phantom, theme::PHANTOM))
                    .on_press(Message::Phantom(idx, !p.phantom)),
            );
        }
        if p.has_pad {
            tg_row = tg_row.push(
                button(text("PAD").size(theme::TEXT_SM * scale))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                    .width(Length::Fill)
                    .style(theme::toggle_button(p.pad, theme::ACCENT))
                    .on_press(Message::Pad(idx, !p.pad)),
            );
        }
        col = col.push(tg_row);
    }

    if p.has_sensitivity {
        let label = if p.sensitivity_plus4 {
            "+4dBu"
        } else {
            "-10dBV"
        };
        // The USB/libusb backend genuinely implements this (wired to
        // the same hardware-verified ref-level mechanism `set_ref_level`
        // already used for Instr 3/4's ref-level switch) — only the
        // ALSA/kernel-driver backend still has no such control at all,
        // dimmed and unpressable there, same treatment as this
        // session's other confirmed-N/A controls (Output balance,
        // `meters: post fx/RMS`).
        if state.device.has_sensitivity_control() {
            col = col.push(
                button(text(label).size(theme::TEXT_SM * scale))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                    .width(Length::Fill)
                    .style(theme::plain_button)
                    .on_press(Message::Sensitivity(idx, !p.sensitivity_plus4)),
            );
        } else {
            let dim = Color { a: 0.4, ..theme::TEXT_SEC };
            col = col.push(
                button(text(label).size(theme::TEXT_SM * scale).color(dim))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                    .width(Length::Fill)
                    .style(theme::plain_button),
            );
        }
    }

    flyout_card(col, width, scale)
}

/// The Trim flyout's content: a single knob, -65..+6 dB on the fader's
/// own master curve (see `RmeDevice::set_trim`). Every hardware input
/// gets this trigger (`StripParams::has_trim`, unconditional — unlike
/// Gain, which is Mic/Instrument-only), so unlike `eq_popover` there's
/// no `Option`-shaped hardware gate to check here, just the channel kind.
/// Same "pushes the row" shape as `settings_popover`/`eq_popover`.
fn trim_popover(state: &TuxMix, cid: ChannelId, width: f32, scale: f32) -> Element<'_, Message> {
    let ChannelId::Input(idx) = cid else {
        return container(iced::widget::Space::new()).into();
    };
    let modifiers = state.modifiers;
    let trim = state.device.inputs()[idx].trim;

    let col = column![
        text("Trim").size(theme::TEXT_SM * scale).color(theme::TEXT_SEC),
        container(knob(Knob {
            value: trim,
            range: (-65.0, 6.0),
            label: format!("{trim:+.1}"),
            arc_from_center: false,
            interactive: true,
            log_scale: false,
            modifiers,
            scale,
            on_change: Box::new(move |v| Message::TrimChanged(idx, v)),
            on_reset: Box::new(move || Message::TrimReset(idx)),
        }))
        .width(Length::Fill)
        .center_x(Length::Fill),
    ]
    .spacing(theme::SPACE_SM * scale);

    flyout_card(col, width, scale)
}

/// The EQ flyout's content: the interactive response plot, then a
/// three-column band matrix (one column per band) with type, gain,
/// frequency and Q rows, and the low-cut slope/frequency row below.
/// Laid out like TotalMix's own EQ panel, with TuxMix's knobs and
/// palette. Analog inputs only (`ch.eq: Some(_)`).
///
/// The panel is height-constrained to the strip so opening it does not
/// stretch the row; its controls scroll when the window is too short to
/// show them all. The frequency knobs are log-scaled, matching the plot's
/// axis.
fn eq_popover(state: &TuxMix, cid: ChannelId, width: f32, scale: f32) -> Element<'_, Message> {
    let ChannelId::Input(idx) = cid else {
        return container(iced::widget::Space::new()).into();
    };
    let Some(eq) = state.device.inputs()[idx].eq else {
        return container(iced::widget::Space::new()).into();
    };
    let modifiers = state.modifiers;
    let label_w = 42.0 * scale;
    let gap = theme::SPACE_TIGHT;
    // The EQ matrix has to fit inside one strip's height. Smaller knobs
    // here (the strip's own gain/pan knobs keep their full size) are what
    // makes the three control rows and the low cut fit without scrolling.
    let eq_scale = scale * 0.9;

    let mut col = column![].spacing(gap);

    col = col.push(
        row![
            button(text("EQ").size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .style(theme::toggle_button(eq.enabled, theme::ACCENT))
                .on_press(Message::EqEnabled(idx, !eq.enabled)),
            text(if eq.enabled { "enabled" } else { "bypassed" })
                .size(theme::TEXT_XS * scale)
                .color(theme::TEXT_SEC),
        ]
        .spacing(theme::SPACE_SM * scale)
        .align_y(iced::Alignment::Center),
    );

    col = col.push(eq_plot(
        eq,
        state.device.settings().sample_rate,
        scale,
        modifiers,
        EqPlotActions {
            on_band: Box::new(move |band, freq, gain| {
                Message::EqBandPoint(idx, band, freq, gain)
            }),
            on_band_reset: Box::new(move |band| Message::EqBandReset(idx, band)),
            on_band_q: Box::new(move |band, q| Message::EqBandQ(idx, band, q)),
            on_low_cut: Box::new(move |freq| Message::EqLowCutFreq(idx, freq)),
        },
    ));

    let label = |content: &'static str| {
        container(
            text(content)
                .size(theme::TEXT_XS * scale)
                .color(theme::TEXT_SEC),
        )
        .width(Length::Fixed(label_w))
    };

    let mut type_row = row![label("type")]
        .spacing(gap)
        .align_y(iced::Alignment::Center);
    for band in 0..3 {
        let b = eq.bands[band];
        let (short, full, next_type) = match b.band_type {
            EqBandType::Off => ("Off", "Off (click for Bell)", EqBandType::Bell),
            EqBandType::Bell => ("Bell", "Bell (click for Low Shelf)", EqBandType::LowShelf),
            EqBandType::LowShelf => (
                "Low",
                "Low Shelf (click for High Shelf)",
                EqBandType::HighShelf,
            ),
            EqBandType::HighShelf => ("High", "High Shelf (click for Off)", EqBandType::Off),
        };
        let number_color = if b.band_type == EqBandType::Off {
            theme::EQ_BAND[band]
        } else {
            theme::ON_ACTIVE
        };
        type_row = type_row.push(
            container(hint(
                button(
                    row![
                        text(format!("{}", band + 1))
                            .size(theme::TEXT_XS * scale)
                            .color(number_color),
                        text(short).size(theme::TEXT_XS * scale),
                    ]
                    .spacing(theme::SPACE_TIGHT)
                    .align_y(iced::Alignment::Center),
                )
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_TIGHT * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(
                    b.band_type != EqBandType::Off,
                    theme::EQ_BAND[band],
                ))
                .on_press(Message::EqBandType(idx, band, next_type)),
                full,
                scale,
            ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }
    col = col.push(type_row);

    let mut gain_row = row![label("gain dB")]
        .spacing(gap)
        .align_y(iced::Alignment::Center);
    for band in 0..3 {
        let b = eq.bands[band];
        gain_row = gain_row.push(
            container(hint(
                knob(Knob {
                    value: b.gain_db,
                    range: (-24.0, 24.0),
                    label: format!("{:+.1}", b.gain_db),
                    arc_from_center: true,
                    interactive: true,
                    log_scale: false,
                    modifiers,
                    scale: eq_scale,
                    on_change: Box::new(move |v| {
                        Message::EqBandGain(idx, band, v.clamp(-24.0, 24.0))
                    }),
                    on_reset: Box::new(move || Message::EqBandGain(idx, band, 0.0)),
                }),
                "Band gain",
                scale,
            ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }
    col = col.push(gain_row);

    let mut freq_row = row![label("freq Hz")]
        .spacing(gap)
        .align_y(iced::Alignment::Center);
    for band in 0..3 {
        let b = eq.bands[band];
        freq_row = freq_row.push(
            container(hint(
                knob(Knob {
                    value: b.freq_hz as f32,
                    range: (20.0, 20_000.0),
                    label: format!("{}Hz", b.freq_hz),
                    arc_from_center: false,
                    interactive: true,
                    log_scale: true,
                    modifiers,
                    scale: eq_scale,
                    on_change: Box::new(move |v| {
                        Message::EqBandFreq(idx, band, v.round().clamp(20.0, 20_000.0) as u16)
                    }),
                    on_reset: Box::new(move || Message::EqBandFreq(idx, band, 1000)),
                }),
                "Band frequency",
                scale,
            ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }
    col = col.push(freq_row);

    let mut q_row = row![label("Q")]
        .spacing(gap)
        .align_y(iced::Alignment::Center);
    for band in 0..3 {
        let b = eq.bands[band];
        q_row = q_row.push(
            container(hint(
                knob(Knob {
                    value: b.q,
                    range: (0.05, 10.0),
                    label: format!("{:.2}", b.q),
                    arc_from_center: false,
                    interactive: true,
                    log_scale: false,
                    modifiers,
                    scale: eq_scale,
                    on_change: Box::new(move |v| {
                        Message::EqBandQ(idx, band, v.clamp(0.05, 10.0))
                    }),
                    on_reset: Box::new(move || Message::EqBandQ(idx, band, 0.7)),
                }),
                "Band Q",
                scale,
            ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }
    col = col.push(q_row);

    let mut slope_row = row![].spacing(gap).width(Length::Fill);
    for slope in [6u8, 12, 18, 24] {
        slope_row = slope_row.push(
            button(text(format!("{slope}")).size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_TIGHT * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(
                    eq.low_cut_slope_db_oct == slope,
                    theme::ACCENT,
                ))
                .on_press(Message::EqLowCutSlope(idx, slope)),
        );
    }
    let slope_col = column![
        slope_row,
        text("dB/oct")
            .size(theme::TEXT_MICRO * scale)
            .color(theme::TEXT_SEC),
    ]
    .spacing(theme::SPACE_TIGHT)
    .width(Length::Fill)
    .align_x(iced::Alignment::Center);

    let freq_col = container(hint(
        knob(Knob {
            value: eq.low_cut_freq_hz as f32,
            range: (20.0, 20_000.0),
            label: format!("{}Hz", eq.low_cut_freq_hz),
            arc_from_center: false,
            interactive: true,
            log_scale: true,
            modifiers,
            scale: eq_scale,
            on_change: Box::new(move |v| {
                Message::EqLowCutFreq(idx, v.round().clamp(20.0, 20_000.0) as u16)
            }),
            on_reset: Box::new(move || Message::EqLowCutFreq(idx, 20)),
        }),
        "Low-cut frequency",
        scale,
    ))
    .width(Length::Fill)
    .center_x(Length::Fill);

    col = col.push(
        row![label("low cut"), slope_col, freq_col]
            .spacing(gap)
            .align_y(iced::Alignment::Center),
    );

    container(
        scrollable(col)
            .direction(scrollable::Direction::Vertical(theme::thin_scrollbar()))
            .style(theme::scrollable)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .padding(theme::SPACE_MD * scale)
    .width(Length::Fixed(width))
    .height(Length::Fill)
    .style(theme::panel)
    .clip(true)
    .into()
}

/// Layers the open route flyout on top of a strip row, positioned by
/// left-padding computed from `open_x` (the open strip's right edge within
/// the row — see `mixer_view`). `None` (nothing open, or the open flyout
/// is `Settings` — see below) skips the `Stack` entirely — the common
/// case, so most frames don't pay for it.
///
/// Settings doesn't come through here at all: unlike Route, which
/// deliberately slides out *over* the strip's right neighbor, the
/// reference design (`Bus+settings_opened.png`) shows the gear panel
/// pushing the row — growing the strip's own footprint rather than
/// floating above whatever's next to it. `mixer_view`'s input loop
/// handles that directly, by widening the row item itself when a strip's
/// Settings flyout is open, which naturally pushes every strip after it
/// over — ordinary layout, no `Stack`/`opaque` overlay machinery needed.
///
/// Positioning assumes the row isn't horizontally scrolled — a known v1
/// gap (no per-row scroll-offset tracking yet); the panel can land at a
/// slightly wrong x if it is.
fn with_flyout<'a>(
    state: &'a TuxMix,
    row_element: Element<'a, Message>,
    open_x: Option<f32>,
) -> Element<'a, Message> {
    let Some(x) = open_x else {
        return row_element;
    };
    if !state
        .flyout_open
        .iter()
        .any(|(_, k)| *k == strip::FlyoutKind::Route)
    {
        return row_element;
    }
    let content = route_popover(state, strip::FLYOUT_W, state.ui_scale);
    // `opaque` captures clicks across its *own* widget's bounds — those
    // have to be just the small popover itself (its natural, tightly-fit
    // size), not the Length::Fill positioning container around it. Get
    // that backwards (opaque wrapping the Fill container) and it silently
    // swallows every click anywhere in the row, popover visible there or
    // not, and the click-outside-closes catcher below never sees a thing.
    // So: outer Fill container (plain, not opaque — a click landing on
    // its empty padding is free to fall through to the catcher) positions
    // an inner `opaque(...)` (small, only as big as the panel actually
    // is) via padding + bottom alignment.
    stack(vec![
        row_element,
        mouse_area(
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::CloseFlyout)
        .into(),
        container(opaque(content))
            .padding(iced::Padding {
                left: x,
                ..iced::Padding::ZERO
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .align_bottom(Length::Fill)
            .into(),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn mixer_view(state: &TuxMix) -> Element<'_, Message> {
    // Grouped by pair, TotalMix-style: a linked pair (the default) shows
    // as ONE strip driving both channels; a split pair shows its two
    // channels independently — see `RmeDevice::input_pair_linked`. Pairs
    // never straddle a channel-type boundary (the device profile always
    // groups same-type channels together), so iterating by pair instead
    // of by individual channel doesn't change where the type dividers
    // land.
    // No channel-type dividers — used to insert a 1px `rule::vertical`
    // between channel-type groups (Mic/Instrument/Line/ADAT), which read
    // as a wider gap there (`SPACE_MD` + the rule + `SPACE_MD`, ~13px)
    // than between two strips of the same type (`SPACE_MD` alone, ~6px).
    // The user caught the inconsistency directly, comparing against
    // Software Playback/Hardware Outputs (see their own comment below —
    // neither ever had dividers, so they were already uniform): removed
    // so every gap on every row is the same `SPACE_MD`, everywhere.
    let mut input_strips = row![].spacing(theme::SPACE_MD);
    let mut input_width = 0.0f32;
    let mut input_item_count = 0usize;
    let mut input_open_x: Option<f32> = None;
    let n_input_pairs = state.device.inputs().len() / 2;
    for pair in 0..n_input_pairs {
        let l = pair * 2;
        let linked = state.device.input_pair_linked(pair);
        let pair_channels = [l, l + 1];
        let shown: &[usize] = if linked {
            &pair_channels[..1]
        } else {
            &pair_channels[..]
        };
        for &ch_idx in shown {
            let cid = ChannelId::Input(ch_idx);
            let mut params = strip_params(state, cid, state.sel_out);
            if linked {
                // Combined "AN1/2"-style label for the linked bus. EQ
                // (idx < 4 only) still only reaches this representative
                // (left) channel while shown combined — split the pair
                // to reach the right channel's own EQ.
                if let Some(right) = state.device.inputs().get(l + 1) {
                    params.name = pair_bus_label(&params.name, &right.name);
                }
                // Real TotalMix splits a linked pair's meter into two
                // thin bars (this channel + its sibling), not one bar
                // showing only the left/even channel — previously the
                // right channel's own real level was captured but never
                // displayed at all while linked (found 2026-09-07).
                params.meter2 = state.input_meters.get(l + 1).map(MeterAnim::frame);
                params.meter2_available = state.device.has_input_meter(l + 1);
            }
            let strip_widget = strip::strip(finish_params(state, params));
            let mut item_width = rendered_strip_width(state, cid);
            // Settings/EQ push the row instead of overlaying it (see
            // `with_flyout`'s doc comment for why) — done here, by
            // widening this loop iteration's own item, rather than in
            // `with_flyout`, since ordinary `row!` layout already pushes
            // every later sibling over for free once this one item is
            // wider.
            if state.flyout_open.contains(&(cid, strip::FlyoutKind::Settings)) {
                // Same width as the strip itself (see the reference
                // design), not the Route flyout's own fixed `FLYOUT_W` —
                // a dropdown list of bus names and a settings panel of
                // knobs/buttons don't need to match each other's width,
                // just their own strip's.
                let panel_w = item_width;
                input_strips = input_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w, state.ui_scale)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else if state.flyout_open.contains(&(cid, strip::FlyoutKind::Eq)) {
                // Unlike Settings, sized to fit the three-column band
                // matrix rather than matching the (much narrower) strip
                // width. `EQ_FLYOUT_W` is the width at scale 1.0, so the
                // manual zoom scales the panel with its contents.
                let panel_w = strip::EQ_FLYOUT_W * state.ui_scale;
                input_strips = input_strips.push(
                    row![strip_widget, eq_popover(state, cid, panel_w, state.ui_scale)].spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else if state.flyout_open.contains(&(cid, strip::FlyoutKind::Trim)) {
                // One knob — reuses the Route flyout's own fixed width
                // rather than a third bespoke constant; Settings' "match
                // the strip's own width" doesn't apply here since there's
                // no strip-width-scaling content (no button rows) to fit.
                let panel_w = strip::FLYOUT_W;
                input_strips = input_strips.push(
                    row![strip_widget, trim_popover(state, cid, panel_w, state.ui_scale)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else {
                input_strips = input_strips.push(strip_widget);
            }
            input_width += item_width;
            input_item_count += 1;
            if state.flyout_open.contains(&(cid, strip::FlyoutKind::Route)) {
                // Gaps placed so far (`item_count - 1`, spacing is between
                // items) plus the content accumulated up to and including this
                // strip is exactly its right edge on screen.
                input_open_x = Some(input_width + (input_item_count - 1) as f32 * theme::SPACE_MD);
            }
        }
    }
    input_width += input_item_count.saturating_sub(1) as f32 * theme::SPACE_MD;

    // Same pair-grouping idea, no channel-type dividers to worry about.
    let mut pb_strips = row![].spacing(theme::SPACE_MD);
    let mut pb_width = 0.0f32;
    let mut pb_item_count = 0usize;
    let mut pb_open_x: Option<f32> = None;
    let n_pb_pairs = state.device.playbacks().len() / 2;
    for pair in 0..n_pb_pairs {
        let l = pair * 2;
        let linked = state.device.playback_linked(pair);
        let pair_channels = [l, l + 1];
        let shown: &[usize] = if linked {
            &pair_channels[..1]
        } else {
            &pair_channels[..]
        };
        for &ch_idx in shown {
            let cid = ChannelId::Playback(ch_idx);
            let mut params = strip_params(state, cid, state.sel_out);
            if linked {
                if let Some(right) = state.device.playbacks().get(l + 1) {
                    params.name = pair_bus_label(&params.name, &right.name);
                }
                params.meter2 = state.playback_meters.get(l + 1).map(MeterAnim::frame);
                params.meter2_available = playback_meter_available(state, l + 1);
            }
            let strip_widget = strip::strip(finish_params(state, params));
            let mut item_width = rendered_strip_width(state, cid);
            if state.flyout_open.contains(&(cid, strip::FlyoutKind::Settings)) {
                let panel_w = item_width;
                pb_strips = pb_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w, state.ui_scale)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else {
                pb_strips = pb_strips.push(strip_widget);
            }
            pb_width += item_width;
            pb_item_count += 1;
            if state.flyout_open.contains(&(cid, strip::FlyoutKind::Route)) {
                pb_open_x = Some(pb_width + (pb_item_count - 1) as f32 * theme::SPACE_MD);
            }
        }
    }
    pb_width += pb_item_count.saturating_sub(1) as f32 * theme::SPACE_MD;

    // Grouped by pair, TotalMix-style: a linked pair (the default) shows
    // as ONE bus strip driving both channels; a split pair shows its two
    // channels as independent strips — see `RmeDevice::output_linked`.
    // Outputs have no Route flyout (a single master, not a per-submix
    // crosspoint), only Settings (the STEREO toggle) — pushed the same
    // way Input/Playback do, no `with_flyout`/open_x tracking needed.
    let mut out_strips = row![].spacing(theme::SPACE_MD);
    let mut out_width = 0.0f32;
    let mut out_item_count = 0usize;
    for pair in 0..state.device.output_pair_count() {
        let l = pair * 2;
        let linked = state.device.output_linked(pair);
        let pair_channels = [l, l + 1];
        let shown: &[usize] = if linked {
            &pair_channels[..1]
        } else {
            &pair_channels[..]
        };
        for &ch_idx in shown {
            let cid = ChannelId::Output(ch_idx);
            let mut params = strip_params(state, cid, state.sel_out);
            if linked {
                if let Some(right) = state.device.outputs().get(l + 1) {
                    params.name = pair_bus_label(&params.name, &right.name);
                }
                params.meter2 = state.output_meters.get(l + 1).map(MeterAnim::frame);
                params.meter2_available = output_meter_available(state, l + 1);
            }
            let strip_widget = strip::strip(finish_params(state, params));
            let mut item_width = rendered_strip_width(state, cid);
            if state.flyout_open.contains(&(cid, strip::FlyoutKind::Settings)) {
                let panel_w = item_width;
                out_strips = out_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w, state.ui_scale)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else {
                out_strips = out_strips.push(strip_widget);
            }
            out_width += item_width;
            out_item_count += 1;
        }
    }
    out_width += out_item_count.saturating_sub(1) as f32 * theme::SPACE_MD;

    // `page()`'s own horizontal padding, plus a small safety margin so a
    // borderline-fitting row biases toward scrolling instead of clipping.
    // The sidebar's width comes off too: the mixer only gets what is left
    // of the window beside it (this used to measure against the whole
    // window, so a row could overflow under the sidebar without ever
    // switching to its scrolling form).
    let sidebar_w = sidebar_px(state);
    let available_width =
        (state.window_width - sidebar_w - 2.0 * theme::SPACE_XL - 4.0).max(0.0);

    let scale = state.ui_scale;
    let submix_hint = || {
        text(format!(
            "Submix: {} \u{00b7} Tab for Matrix",
            OUT_LABELS[state.sel_out]
        ))
        .color(theme::TEXT_SEC)
        .size(theme::TEXT_XS * scale)
    };
    // TotalMix's "2 Row" (Options panel): inputs and playbacks side by
    // side, outputs underneath. Always honored once chosen: when the two
    // blocks are wider than the window, the top row scrolls sideways like
    // any other over-wide strip row. (It used to fall back to the stacked
    // layout instead, so on a narrower window the switch lit up and
    // nothing changed.)
    let side_gap = theme::SPACE_XXL * scale;
    let side_by_side_width = input_width + side_gap + pb_width;
    let two_rows = match state.row_mode {
        RowMode::Two => true,
        RowMode::Three => false,
        RowMode::Auto => side_by_side_width <= available_width,
    };
    let sources: Element<'_, Message> = if two_rows {
        let side_by_side = row![
            column![
                section_header("HARDWARE INPUTS", scale),
                submix_hint(),
                with_flyout(state, input_strips.into(), input_open_x),
            ]
            .spacing(theme::SPACE_LG)
            .width(Length::Fixed(input_width)),
            column![
                section_header("SOFTWARE PLAYBACK", scale),
                // Keeps the playback strips level with the input strips,
                // which have the submix hint above them.
                text(" ").size(theme::TEXT_XS * scale),
                with_flyout(state, pb_strips.into(), pb_open_x),
            ]
            .spacing(theme::SPACE_LG)
            .width(Length::Fixed(pb_width)),
        ]
        .spacing(side_gap);
        responsive_row(side_by_side.into(), side_by_side_width, available_width)
    } else {
        column![
            section_header("HARDWARE INPUTS", scale),
            submix_hint(),
            with_flyout(
                state,
                responsive_row(input_strips.into(), input_width, available_width),
                input_open_x,
            ),
            section_header("SOFTWARE PLAYBACK", scale),
            with_flyout(
                state,
                responsive_row(pb_strips.into(), pb_width, available_width),
                pb_open_x,
            ),
        ]
        .spacing(theme::SPACE_LG)
        .into()
    };
    let body = column![
        sources,
        section_header("HARDWARE OUTPUTS", scale),
        responsive_row(out_strips.into(), out_width, available_width),
    ]
    .spacing(theme::SPACE_LG)
    .width(Length::Fill);

    page(body)
}

fn matrix_view(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    // The caption reads out the cell under the pointer, and is the how-to
    // the rest of the time (the gestures are not discoverable otherwise).
    let caption = match matrix::hover_readout(state) {
        Some(readout) => text(readout).color(theme::TEXT_PRIMARY),
        None => text(
            "Drag a cell up or down to set its gain (Shift: fine) \u{00b7} \
             Ctrl+click: 0 dB / off \u{00b7} double-click: reset \u{00b7} Tab for Quick",
        )
        .color(theme::TEXT_SEC),
    }
    .size(theme::TEXT_XS * scale);
    let body = column![
        section_header("MATRIX MIXER", scale),
        caption,
        matrix::view(state),
    ]
    .spacing(theme::SPACE_LG)
    .width(Length::Fill)
    .height(Length::Fill);

    // Not `page()`: the grid scrolls inside itself, under a fixed header
    // (see `matrix::view`), which a page-wide scrollable would defeat.
    mouse_area(
        container(body)
            .style(theme::root)
            .padding([theme::SPACE_LG, theme::SPACE_XL])
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::ClearSelection)
    .into()
}

/// Every channel selectable as a Quick Control source, in display order —
/// inputs then playbacks, matching `channel_order`. Outputs are excluded:
/// the Quick view's destination block is always `Output(state.sel_out)`,
/// picked via the existing Submix selector in the top bar rather than a
/// second picker here.
fn quick_channel_options(state: &TuxMix) -> Vec<(ChannelId, String)> {
    let mut opts = Vec::new();
    for (i, ch) in state.device.inputs().iter().enumerate() {
        let cid = ChannelId::Input(i);
        let name = user_label(state, cid).unwrap_or(short_label(&ch.name));
        opts.push((cid, format!("{name} · IN")));
    }
    for (i, ch) in state.device.playbacks().iter().enumerate() {
        let cid = ChannelId::Playback(i);
        let name = user_label(state, cid).unwrap_or(short_label(&ch.name));
        opts.push((cid, format!("{name} · PB")));
    }
    opts
}

/// How much bigger than a normal strip the two Quick Control blocks are:
/// the point of this view is that the one source and one destination that
/// matter are large, easy targets, not a dense grid. A plain multiple of
/// `scale`, like every other dimension, so Ctrl+wheel zoom grows the strips
/// too, and the view scrolls once they no longer fit. Sizing them to the
/// window instead would cancel the zoom out: the fit would hand back exactly
/// the room the zoom had just taken away.
///
/// 1.8 rather than 2 so the whole group still fits, centered, in the default
/// 1280x800 window at 100%. That is now the floor: a taller window grows
/// the strips up to `QUICK_SCALE_MAX` (see `quick_fit`), where at 1080p a
/// fixed 1.8 left the bottom half of the view empty.
const QUICK_SCALE_MULT: f32 = 1.8;
/// The largest the Quick strips grow on a tall window.
const QUICK_SCALE_MAX: f32 = 3.0;

/// The strip multiple that fills `height` (the view's height at zoom 1),
/// between `QUICK_SCALE_MULT` and `QUICK_SCALE_MAX`. Measured at zoom 1
/// and then multiplied by the zoom, so the zoom keeps growing the strips
/// (and the view scrolls once they no longer fit) instead of being
/// cancelled by a fit that hands back the room it took.
fn quick_fit(height: f32) -> f32 {
    ((height - QUICK_CHROME) / QUICK_STRIP_UNIT).clamp(QUICK_SCALE_MULT, QUICK_SCALE_MAX)
}

/// The Quick content's height at `scale` 1, split between the part that does
/// not grow with the strips (the header, the picker, the captions) and the
/// part the two strips take per unit of strip scale. Only used to choose
/// between the centered form and the scrolling one, never to size anything,
/// and deliberately a little high: erring high means scrolling a touch
/// early, while erring low would pick the centered form for a group that
/// does not fit and clip it off at both ends.
const QUICK_CHROME: f32 = 110.0;
const QUICK_STRIP_UNIT: f32 = 330.0;

/// A Quick block: the picker over the strip, with the panel the strip has
/// open (Settings, EQ, Trim or the route list) to the strip's right. The
/// panel is drawn at the strips' own scale and fills the strip's height, so
/// it stays proportional to them; the picker keeps the strip's width.
fn quick_block<'a>(
    state: &'a TuxMix,
    cid: ChannelId,
    heading: Element<'a, Message>,
    picker: Element<'a, Message>,
    strip_widget: Element<'a, Message>,
    strip_w: f32,
    big_scale: f32,
) -> Element<'a, Message> {
    use strip::FlyoutKind::{Eq, Route, Settings, Trim};
    let open = |k| state.flyout_open.contains(&(cid, k));
    let panel = if open(Settings) {
        Some((settings_popover(state, cid, strip_w, big_scale), strip_w))
    } else if open(Eq) {
        let w = strip::EQ_FLYOUT_W * big_scale;
        Some((eq_popover(state, cid, w, big_scale), w))
    } else if open(Trim) {
        let w = strip::FLYOUT_W * big_scale;
        Some((trim_popover(state, cid, w, big_scale), w))
    } else if open(Route) {
        let w = strip::FLYOUT_W * big_scale;
        Some((route_popover(state, w, big_scale), w))
    } else {
        None
    };
    let gap = theme::SPACE_MD * state.ui_scale;
    let head = container(column![heading, picker].spacing(gap).align_x(iced::Alignment::Center))
        .width(Length::Fixed(strip_w))
        .center_x(Length::Fixed(strip_w));
    let body: Element<'a, Message> = match panel {
        // A stack, not a row: the strip alone sets the height, and the
        // panel's layer fills it (a `Fill` panel in a row would take all
        // the room the page offers).
        Some((panel, panel_w)) => iced::widget::stack![
            row![strip_widget, iced::widget::Space::new().width(panel_w + gap)],
            row![iced::widget::Space::new().width(strip_w + gap), panel]
                .height(Length::Fill),
        ]
        .into(),
        None => strip_widget,
    };
    column![head, body].spacing(gap).into()
}

fn quick_view(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let options = quick_channel_options(state);
    let labels: Vec<String> = options.iter().map(|(_, l)| l.clone()).collect();
    let cids: Vec<ChannelId> = options.iter().map(|(c, _)| *c).collect();
    let current = options
        .iter()
        .find(|(cid, _)| *cid == state.quick_channel)
        .map(|(_, l)| l.clone());

    let quick_channel = state.quick_channel;
    let sel_out = state.sel_out;
    let dest_cid = ChannelId::Output(sel_out * 2);

    // Rebuilt on every layout pass: the content sits behind a `responsive`
    // (the centered form needs the available size, which a `Length::Fill`
    // cannot provide inside the page's scrollable), so it only captures
    // owned or copyable data and clones what it needs per call.
    // The destination picker lists the submix buses by the same names the
    // rest of the UI uses for them (a user label on an output strip
    // included), and selecting one is exactly the top bar's Submix field.
    let dest_labels: Vec<String> = (0..OUT_LABELS.len())
        .map(|i| {
            user_label(state, ChannelId::Output(i * 2))
                .unwrap_or(OUT_LABELS[i])
                .to_string()
        })
        .collect();

    let build_content = move |big_scale: f32| -> Element<'_, Message> {
        let labels_for_pick = labels.clone();
        let cids = cids.clone();
        let strip_w = strip::STRIP_W * big_scale;
        fn styled<'p>(
            p: iced::widget::PickList<'p, String, Vec<String>, String, Message>,
            scale: f32,
            width: f32,
        ) -> iced::widget::PickList<'p, String, Vec<String>, String, Message> {
            p.style(theme::pick_list)
                .menu_style(theme::menu)
                .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
                .text_size(theme::TEXT_MD * scale)
                .width(Length::Fixed(width))
        }
        let picker_style = |p| styled(p, scale, strip_w);
        let source_picker = picker_style(
            pick_list(labels.clone(), current.clone(), move |label| {
                let idx = labels_for_pick
                    .iter()
                    .position(|l| *l == label)
                    .unwrap_or(0);
                Message::QuickChannelSelected(cids[idx])
            })
            .placeholder("select..."),
        );
        let dest_for_pick = dest_labels.clone();
        let dest_picker = picker_style(pick_list(
            dest_labels.clone(),
            dest_labels.get(sel_out).cloned(),
            move |label| {
                let idx = dest_for_pick.iter().position(|l| *l == label).unwrap_or(0);
                Message::SelectOutput(idx)
            },
        ));

        // `sel_out` indexes the 6 *submix pairs* (`OUT_LABELS`), but
        // `device.outputs()` is a flat list of individual physical
        // channels, two per pair (`build_outputs`: index `2*i`/`2*i+1` =
        // left/right of pair `i`) — so the pair's representative channel
        // is `sel_out * 2`, not `sel_out` itself.
        let mut source_params = strip_params(state, quick_channel, sel_out);
        source_params.scale = big_scale;
        let mut dest_params = strip_params(state, dest_cid, sel_out);
        dest_params.scale = big_scale;
        // Named and metered as the Mixer's output strip is: a linked pair
        // is one "AN1/2" bus with both channels' meters, not its left half.
        if state.device.output_linked(sel_out) {
            let l = sel_out * 2;
            if let Some(right) = state.device.outputs().get(l + 1) {
                dest_params.name = pair_bus_label(&dest_params.name, &right.name);
            }
            dest_params.meter2 = state.output_meters.get(l + 1).map(MeterAnim::frame);
            dest_params.meter2_available = output_meter_available(state, l + 1);
        }

        // Each block is caption, picker, strip: the picker sits on the
        // strip it chooses, so the two columns read as one sentence (this
        // source, into this destination) and the destination no longer
        // has to be changed from the top bar.
        let caption = |label: &'static str| {
            text(label)
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale)
        };
        let source_block = quick_block(
            state,
            quick_channel,
            caption("SOURCE").into(),
            source_picker.into(),
            strip::strip(finish_params(state, source_params)),
            strip_w,
            big_scale,
        );
        let dest_block = quick_block(
            state,
            dest_cid,
            caption("DESTINATION").into(),
            dest_picker.into(),
            strip::strip(finish_params(state, dest_params)),
            strip_w,
            big_scale,
        );

        let connector = text("\u{2192}")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XL * scale);

        let blocks = row![source_block, connector, dest_block]
            .spacing(theme::SPACE_XL * big_scale)
            .align_y(iced::Alignment::Center);

        column![
            section_header("QUICK CONTROL", scale),
            text("Choose a source and a destination, then set how loud the source is there. Tab switches to the Mixer.")
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale),
            blocks,
        ]
        .spacing(theme::SPACE_LG)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill)
        .into()
    };

    // One centered group when it fits, as the view is meant to be read as a
    // single sentence instead of a page of controls; the scrolling page the
    // other views use is the fallback for small windows and a zoomed-in UI.
    // Centered *horizontally* only: centering it vertically too left a dead
    // band between the top bar and the QUICK CONTROL header that read as a
    // layout bug (asked to be removed, 2026-10-16). The height is per unit
    // of `scale`, so it grows with the zoom the same way the content it
    // measures does.
    let content = responsive(move |size| {
        let fit = quick_fit(size.height);
        let needed = (QUICK_CHROME + QUICK_STRIP_UNIT * fit) * scale;
        if size.height >= needed {
            container(build_content(scale * fit))
                .center_x(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            scrollable(build_content(scale * fit))
                .direction(scrollable::Direction::Vertical(theme::thin_scrollbar()))
                .width(Length::Fill)
                .style(theme::scrollable)
                .into()
        }
    });

    mouse_area(
        container(content)
            .style(theme::root)
            .padding([theme::SPACE_LG, theme::SPACE_XL])
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::ClearSelection)
    .into()
}

#[cfg(test)]
mod tests {
    use super::{
        any_channel_muted, any_channel_soloed, apply_pending_resize,
        apply_prefs, channel_is_muted, channel_is_soloed, is_undoable, new, prefs_of,
        mute_armed, power_sum_output_meters, update, zoom, ChannelId, EditField, MeterAnim, Message, RowMode, View, ZOOM_STEP,
    };
    use crate::sidebar;
    use crate::theme;
    use crate::widgets::strip;
    use std::collections::HashSet;
    use std::time::{Duration, Instant};
    use tuxmix_core::RmeDevice;

    #[test]
    fn output_meters_reads_the_right_pair_for_every_individual_channel() {
        // 3 output pairs (6 individual channels): pair 0 = ch0/1, pair 1
        // = ch2/3, pair 2 = ch4/5. One input, routed at full volume into
        // pair 2 only (and silent into pairs 0/1), reading a fixed 1.0
        // meter level — deterministic, unlike the mock backend's own
        // randomized meters.
        let input_volumes = vec![vec![0.0, 0.0, 1.0]];
        let input_meters = vec![1.0];
        let out = power_sum_output_meters(&input_volumes, &input_meters, &[], &[], 6);

        assert_eq!(out.len(), 6);
        assert_eq!(out[0], 0.0, "pair 0 (ch0) has no signal routed to it");
        assert_eq!(out[1], 0.0, "pair 0 (ch1) has no signal routed to it");
        assert_eq!(out[2], 0.0, "pair 1 (ch2) has no signal routed to it");
        assert_eq!(out[3], 0.0, "pair 1 (ch3) has no signal routed to it");
        assert_eq!(out[4], 1.0, "pair 2 (ch4) should read the full routed signal");
        assert_eq!(out[5], 1.0, "pair 2 (ch5) should read the full routed signal");
    }

    #[test]
    fn output_meters_never_goes_out_of_bounds_for_pairs_past_the_volumes_array() {
        // A device with more output *pairs* than any single source's
        // `volumes` array happens to have entries for (shouldn't happen
        // in practice, but the old bug's failure mode was exactly an
        // out-of-bounds pair read silently going to 0.0 instead of
        // panicking) — must stay silent-zero, not panic.
        let input_volumes = vec![vec![1.0]]; // only 1 pair's worth of data
        let input_meters = vec![1.0];
        let out = power_sum_output_meters(&input_volumes, &input_meters, &[], &[], 12);
        assert_eq!(out.len(), 12);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], 1.0);
        for level in &out[2..] {
            assert_eq!(*level, 0.0);
        }
    }

    #[test]
    fn a_refused_write_is_shown_and_expires() {
        let mut state = new(true, None, None);
        super::fail(&mut state, "set_input_phantom", &"busy");
        let (text, _) = state.notice.clone().expect("a notice");
        assert_eq!(text, "Could not apply input phantom: busy");
        // The same failure repeated (a drag) just refreshes it.
        super::fail(&mut state, "set_input_phantom", &"busy");
        assert!(state.notice.is_some());
        state.notice = Some((text, Instant::now() - Duration::from_secs(6)));
        let _ = update(&mut state, Message::Tick);
        assert!(state.notice.is_none());
    }

    #[test]
    fn a_window_closed_behind_our_back_is_forgotten_or_ends_the_app() {
        let mut state = new(true, None, None);
        let id = iced::window::Id::unique();
        state.window_id = Some(id);
        state.tray_active = true;
        let _ = update(&mut state, Message::WindowClosed(id));
        assert_eq!(state.window_id, None);
        assert!(state.window_hidden, "with a tray the app lives on, hidden");
        // Another window's close is none of our business.
        state.window_id = Some(id);
        let _ = update(&mut state, Message::WindowClosed(iced::window::Id::unique()));
        assert_eq!(state.window_id, Some(id));
    }

    #[test]
    fn closing_hides_in_the_tray_only_when_there_is_one() {
        // Without a tray (the mock never has one) there would be no way back
        // to a hidden window: closing must quit, not hide.
        let mut state = new(true, None, None);
        assert!(!state.tray_active && state.keep_in_tray);
        let _ = update(&mut state, Message::CloseRequested);
        assert!(!state.window_hidden);
    }

    #[test]
    fn keep_in_tray_defaults_on_and_only_an_opt_out_is_saved() {
        let mut state = new(true, None, None);
        assert_eq!(prefs_of(&state).keep_in_tray, None);
        let _ = update(&mut state, Message::SetKeepInTray(false));
        let prefs = prefs_of(&state);
        assert_eq!(prefs.keep_in_tray, Some(false));
        apply_prefs(&mut state, prefs);
        assert!(!state.keep_in_tray);
        assert!(!is_undoable(&Message::SetKeepInTray(true)));
    }

    #[test]
    fn zoom_in_steps_up_from_default() {
        let mut state = new(true, None, None);
        assert_eq!(state.ui_scale, crate::theme::SCALE_DEFAULT);
        zoom(&mut state, ZOOM_STEP);
        assert!(
            (state.ui_scale - (crate::theme::SCALE_DEFAULT + ZOOM_STEP)).abs() < 1e-6,
            "one zoom-in should add exactly ZOOM_STEP: {}",
            state.ui_scale
        );
    }

    #[test]
    fn zoom_clamps_to_min_and_max() {
        let mut state = new(true, None, None);
        for _ in 0..200 {
            zoom(&mut state, -ZOOM_STEP);
        }
        assert_eq!(state.ui_scale, crate::theme::SCALE_MIN);
        for _ in 0..400 {
            zoom(&mut state, ZOOM_STEP);
        }
        assert_eq!(state.ui_scale, crate::theme::SCALE_MAX);
    }

    #[test]
    fn window_resize_does_not_change_the_zoom() {
        // The whole point of fixed geometry: a resize must NOT rescale.
        // `zoom` is the only thing that moves `ui_scale`; a resize just
        // updates `window_width` (see `apply_pending_resize`).
        let mut state = new(true, None, None);
        let before = state.ui_scale;
        state.pending_resize_width = Some(700.0);
        apply_pending_resize(&mut state);
        assert_eq!(state.ui_scale, before);
        assert_eq!(state.window_width, 700.0);
    }

    #[test]
    fn attack_rises_fast() {
        let mut m = MeterAnim::new();
        m.step(1.0);
        assert!(
            m.frame().value > 0.5,
            "one attack tick should jump most of the way: {}",
            m.frame().value
        );
    }

    #[test]
    fn release_decelerates_over_time() {
        let mut m = MeterAnim::new();
        m.step(1.0); // reach a peak first
        let peak = m.frame().value;

        m.step(0.0);
        let drop_1 = peak - m.frame().value;

        for _ in 0..10 {
            m.step(0.0);
        }
        let before_late = m.frame().value;
        m.step(0.0);
        let drop_late = before_late - m.frame().value;

        assert!(
            drop_1 > drop_late,
            "first release tick should fall faster than a tick late into the release: {drop_1} vs {drop_late}"
        );
    }

    #[test]
    fn rising_mid_release_cancels_it_and_resets_the_curve() {
        let mut m = MeterAnim::new();
        m.step(1.0);
        m.step(0.0);
        m.step(0.0);
        m.step(1.0); // new peak — release curve should restart from here
        assert_eq!(m.release_elapsed_ms, 0.0);
    }

    // ── Right sidebar: Undo/Redo, Groups, Layout ──────────────────────
    // Interactive click-testing (`--mock` + synthetic clicks) turned out
    // to be unreliable in this sandbox for this pass — see the
    // `project_bus_redesign_2026_09` memory. These exercise the same
    // `update()` codepath a real click would, directly, which needs no
    // window/focus/click-coordinate cooperation from the environment.

    #[test]
    fn undo_reverts_a_mute() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::Mute(cid, true));
        assert!(state.device.inputs()[0].mute);
        let _ = update(&mut state, Message::Undo);
        assert!(!state.device.inputs()[0].mute, "undo should revert the mute");
    }

    #[test]
    fn redo_reapplies_after_undo() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::Mute(cid, true));
        let _ = update(&mut state, Message::Undo);
        assert!(!state.device.inputs()[0].mute);
        let _ = update(&mut state, Message::Redo);
        assert!(state.device.inputs()[0].mute, "redo should reapply the mute");
    }

    #[test]
    fn a_new_mutating_action_clears_the_redo_stack() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::Mute(cid, true));
        let _ = update(&mut state, Message::Undo);
        assert!(!state.redo_stack.is_empty(), "sanity: undo should have populated redo");
        let _ = update(&mut state, Message::Mute(cid, true));
        assert!(
            state.redo_stack.is_empty(),
            "a fresh mutating action should invalidate the old redo history"
        );
    }

    #[test]
    fn ui_only_messages_do_not_grow_the_undo_stack() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::Tick);
        let _ = update(&mut state, Message::StripHovered(Some(ChannelId::Input(0))));
        assert!(
            state.undo_stack.is_empty(),
            "Tick/hover are UI-only and shouldn't be undoable steps"
        );
    }

    // `Input(0)`/`Input(4)` deliberately — channels 0/1 are a *stereo
    // link* pair by default in mock (`input_pair_link`, see
    // `mock.rs::test_input_pair_linked_by_default_and_moves_both_channels`),
    // which propagates independently of anything Groups-related and
    // would confound these tests (caught by an earlier failing run of
    // this exact test using `Input(0)`/`Input(1)` — the stereo-pair
    // mechanism was moving "b" too, for a reason that had nothing to do
    // with the group). Channels 0 and 4 are in different pairs.

    #[test]
    fn group_mute_link_propagates_to_members() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(4);
        state.selected.insert(a);
        state.selected.insert(b);
        let _ = update(&mut state, Message::GroupEditToggle);
        let _ = update(&mut state, Message::GroupLinkToggle(0, sidebar::GroupLink::Mute));
        assert_eq!(state.groups[0].members.len(), 2);
        assert!(state.groups[0].mute_linked);
        // Clear the selection used to assign membership — isolates
        // group-driven propagation from the pre-existing multi-select
        // one, which would also propagate here and mask a group-only bug.
        state.selected.clear();

        let _ = update(&mut state, Message::Mute(a, true));
        assert!(state.device.inputs()[0].mute);
        assert!(
            state.device.inputs()[4].mute,
            "muting one mute-linked group member should mute the other"
        );
    }

    #[test]
    fn group_solo_link_does_not_propagate_mute() {
        // A solo-linked-only group shouldn't cross-propagate mute, and
        // vice versa — the three link types are independent.
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(4);
        state.selected.insert(a);
        state.selected.insert(b);
        let _ = update(&mut state, Message::GroupEditToggle);
        let _ = update(&mut state, Message::GroupLinkToggle(0, sidebar::GroupLink::Solo));
        state.selected.clear();

        let _ = update(&mut state, Message::Mute(a, true));
        assert!(state.device.inputs()[0].mute);
        assert!(
            !state.device.inputs()[4].mute,
            "solo-linked (not mute-linked) group shouldn't propagate mute"
        );
    }

    #[test]
    fn group_fader_link_moves_members_by_the_same_relative_delta() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(4);
        state.selected.insert(a);
        state.selected.insert(b);
        let _ = update(&mut state, Message::GroupEditToggle);
        let _ = update(&mut state, Message::GroupLinkToggle(0, sidebar::GroupLink::Fader));
        state.selected.clear();

        // Both start at the default unity volume (1.0) — an equal
        // relative move should land them at the same place.
        let _ = update(&mut state, Message::VolumeChanged(a, 0, 0.5));
        let vol_a = state.device.inputs()[0].volumes[0];
        let vol_b = state.device.inputs()[4].volumes[0];
        assert!((vol_a - 0.5).abs() < 1e-4);
        assert!(
            (vol_b - 0.5).abs() < 1e-4,
            "fader-linked member starting at the same volume should track exactly: {vol_b}"
        );
    }

    #[test]
    fn group_clear_removes_all_membership() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        state.selected.insert(a);
        let _ = update(&mut state, Message::GroupEditToggle);
        let _ = update(&mut state, Message::GroupLinkToggle(0, sidebar::GroupLink::Mute));
        assert!(!state.groups[0].members.is_empty());

        let _ = update(&mut state, Message::GroupClear);
        assert!(state.groups.iter().all(|g| g.members.is_empty()));
        assert!(!state.group_editing);
    }

    /// The chip's word maps the connection state in priority order: a
    /// disconnect outranks everything, then the mock's own "Simulated"
    /// (it is the loudest fact about the connection, even when the mock
    /// imitates a class-compliant card). The two real words only exist
    /// on real hardware, where they were verified live.
    #[test]
    fn device_state_reports_the_personality_and_a_disconnect() {
        let mut state = new(true, None, None);
        assert_eq!(crate::sidebar::device_state(&state).1, "Simulated");

        // The mock keeps its own word even when it imitates the
        // class-compliant feature set (`TUXMIX_MOCK_CC=1 --mock`).
        state.device.set_protocol_features(false);
        assert_eq!(crate::sidebar::device_state(&state).1, "Simulated");

        state.device.set_connected(false);
        let _ = update(&mut state, Message::Tick);
        assert!(!state.connected, "the tick probe must see the disconnect");
        assert_eq!(crate::sidebar::device_state(&state).1, "Disconnected");
        assert_eq!(
            crate::sidebar::device_state(&state).0,
            crate::theme::MRED,
            "a disconnect is the red state, not the amber ones"
        );

        // `--mock` was explicit here, so the probe must not try to
        // "reconnect" to hardware that was never asked for: force the
        // throttle open and tick again.
        state.last_conn_check = None;
        let _ = update(&mut state, Message::Tick);
        assert!(!state.connected, "explicit --mock never reconnects");
    }

    #[test]
    fn class_compliant_mode_drops_protocol_messages() {
        let mut state = new(true, None, None);

        // The mock drives the full set by default, so the message the TRIM
        // control sends really does reach the device.
        let _ = update(&mut state, Message::TrimChanged(0, 6.0));
        assert_eq!(state.device.inputs()[0].trim, 6.0);

        // On the class-compliant personality those setters can only fail,
        // so the renderers hide the controls and `update` drops what a stale
        // key binding or an OSC message would still send.
        let pushed = state.undo_stack.len();
        state.device.set_protocol_features(false);
        let _ = update(&mut state, Message::TrimChanged(0, -3.0));
        assert_eq!(state.device.inputs()[0].trim, 6.0, "trim must not move");
        let _ = update(&mut state, Message::EqEnabled(0, true));
        assert!(
            !state.device.inputs()[0].eq.as_ref().unwrap().enabled,
            "the EQ must not engage behind a hidden button"
        );

        // A dropped message leaves no scene behind: the guard has to run
        // before the undo capture, or a trim drag in this mode would fill
        // the stack with no-ops.
        assert_eq!(
            state.undo_stack.len(),
            pushed,
            "a dropped message must not have pushed a scene"
        );

        // Messages that are not protocol-bound still land, and still push.
        let _ = update(&mut state, Message::Mute(ChannelId::Input(0), true));
        assert!(state.device.inputs()[0].mute);
        assert_eq!(state.undo_stack.len(), pushed + 1, "a real action still pushes a scene");
    }

    #[test]
    fn toggle_sidebar_flips_and_is_not_undoable() {
        let mut state = new(true, None, None);
        assert!(state.sidebar_open, "sidebar starts expanded");
        let _ = update(&mut state, Message::ToggleSidebar);
        assert!(!state.sidebar_open);
        let _ = update(&mut state, Message::ToggleSidebar);
        assert!(state.sidebar_open);
        assert!(
            state.undo_stack.is_empty(),
            "collapsing/expanding the sidebar is UI chrome, not a device change"
        );
    }

    #[test]
    fn sidebar_resize_clamps_and_is_not_undoable() {
        let mut state = new(true, None, None);
        assert_eq!(state.sidebar_width, sidebar::DEFAULT_WIDTH);

        // Dragging the strip left (negative dx) widens the column.
        let _ = update(&mut state, Message::SidebarResize(-40.0));
        assert_eq!(state.sidebar_width, sidebar::DEFAULT_WIDTH + 40.0);

        // Clamped both ends, so a fast drag past the window edge can't
        // produce a zero or negative width to lay the mixer out against.
        let _ = update(&mut state, Message::SidebarResize(-10_000.0));
        assert_eq!(state.sidebar_width, sidebar::MAX_WIDTH);
        let _ = update(&mut state, Message::SidebarResize(10_000.0));
        assert_eq!(state.sidebar_width, sidebar::MIN_WIDTH);

        // One message per drag step, so this is the assertion that
        // matters: a resize must not push a `Scene` per mouse move.
        assert!(
            state.undo_stack.is_empty(),
            "resizing the sidebar is UI chrome, not a device change"
        );
    }

    #[test]
    fn eq_plot_drag_updates_frequency_and_gain_together() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::EqBandPoint(0, 0, 2_500, 6.5));
        let eq = state.device.inputs()[0].eq.unwrap();
        assert_eq!(eq.bands[0].freq_hz, 2_500);
        assert!((eq.bands[0].gain_db - 6.5).abs() < 0.01);
    }

    #[test]
    fn eq_plot_double_click_resets_the_band() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::EqBandPoint(0, 0, 2_500, 6.5));
        let _ = update(&mut state, Message::EqBandQ(0, 0, 3.0));
        let _ = update(&mut state, Message::EqBandReset(0, 0));
        let eq = state.device.inputs()[0].eq.unwrap();
        assert_eq!(eq.bands[0].freq_hz, 1_000);
        assert!(eq.bands[0].gain_db.abs() < 0.01);
        assert!((eq.bands[0].q - 0.7).abs() < 0.01);
    }

    #[test]
    fn global_mute_master_clears_the_active_mutes_then_restores_them() {
        let mut state = new(true, None, None);
        assert!(!any_channel_muted(&state), "mock starts fully unmuted");
        let a = ChannelId::Input(0);
        let b = ChannelId::Playback(2);
        let _ = update(&mut state, Message::Mute(a, true));
        let _ = update(&mut state, Message::Mute(b, true));
        assert!(any_channel_muted(&state), "the master lights with any mute");

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(!any_channel_muted(&state), "first press clears every mute");

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(channel_is_muted(&state, a));
        assert!(channel_is_muted(&state, b));
        assert!(state.last_cleared_mutes.is_empty(), "the remembered set was used up");
    }

    #[test]
    fn cue_is_exclusive_across_outputs() {
        let mut state = new(true, None, None);
        let a = ChannelId::Output(0); // pair 0
        let b = ChannelId::Output(2); // pair 1

        let _ = update(&mut state, Message::CueChanged(a, true));
        assert!(state.device.outputs()[0].cue);
        assert!(state.device.outputs()[1].cue, "cue mirrors across a pair");
        assert!(!state.device.outputs()[2].cue);

        // Engaging CUE on a different output silently disengages `a`
        // (there's only one AN1/2 monitor bus to share) rather than
        // both reading as cued at once.
        let _ = update(&mut state, Message::CueChanged(b, true));
        assert!(!state.device.outputs()[0].cue, "a should no longer be cued");
        assert!(!state.device.outputs()[1].cue);
        assert!(state.device.outputs()[2].cue);
        assert!(state.device.outputs()[3].cue);

        let _ = update(&mut state, Message::CueChanged(b, false));
        assert!(!state.device.outputs()[2].cue, "explicit off clears it");
        assert!(!state.device.outputs()[3].cue);
    }

    #[test]
    fn eq_for_record_and_optical_out_format_toggles_reach_device_settings() {
        let mut state = new(true, None, None);
        assert!(!state.device.settings().eq_for_record);
        assert!(!state.device.settings().optical_out_spdif);

        let _ = update(&mut state, Message::EqForRecordChanged(true));
        assert!(state.device.settings().eq_for_record);
        assert!(!state.device.settings().optical_out_spdif);

        let _ = update(&mut state, Message::OpticalOutFormatChanged(true));
        assert!(state.device.settings().optical_out_spdif);

        let _ = update(&mut state, Message::EqForRecordChanged(false));
        assert!(!state.device.settings().eq_for_record);
        assert!(
            state.device.settings().optical_out_spdif,
            "toggling EQ-for-Record must not stomp the SPDIF flag (shared settings-word register)"
        );
    }

    #[test]
    fn phase_changed_reaches_the_input_model() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(1); // AN2

        let _ = update(&mut state, Message::PhaseChanged(cid, true));
        assert!(state.device.inputs()[1].phase);

        let _ = update(&mut state, Message::PhaseChanged(cid, false));
        assert!(!state.device.inputs()[1].phase);
    }

    #[test]
    fn phantom_changed_mirrors_across_a_linked_input_pair() {
        // User-caught: toggling 48V on a linked AN1/2 strip only ever
        // touched its own (left) channel — Message::Phantom called
        // set_phantom directly, bypassing the new pair-aware
        // set_input_phantom entirely.
        let mut state = new(true, None, None);
        assert!(state.device.input_pair_linked(0));
        let _ = update(&mut state, Message::Phantom(0, false));
        let _ = update(&mut state, Message::Phantom(1, false));

        let _ = update(&mut state, Message::Phantom(0, true));
        assert!(state.device.inputs()[0].phantom);
        assert!(
            state.device.inputs()[1].phantom,
            "48V must mirror to AN2 when the pair is linked"
        );
    }

    #[test]
    fn unlinking_a_playback_pair_splits_both_channels() {
        // A playback pair's link is its hardware stereo split: the STEREO
        // switch is the one control for it (the strip's "SP" button was the
        // same state a second time, and is gone).
        let mut state = new(true, None, None);
        let cid = ChannelId::Playback(0); // PB1 (channels 0/1)

        let _ = update(&mut state, Message::StereoLinkChanged(cid, false));
        assert!(state.device.playbacks()[0].split);
        assert!(state.device.playbacks()[1].split, "split mirrors across both channels of the pair");

        let _ = update(&mut state, Message::StereoLinkChanged(cid, true));
        assert!(!state.device.playbacks()[0].split);
        assert!(!state.device.playbacks()[1].split);
    }

    #[test]
    fn opening_a_flyout_on_one_strip_does_not_close_another_strips_flyout() {
        // User-caught regression: TuxMix used to force a single global
        // `Option<(ChannelId, FlyoutKind)>`, so opening Settings on a
        // second strip silently closed whatever was open on the first —
        // real TotalMix allows several channel-settings panels open at
        // once.
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(2);

        let _ = update(&mut state, Message::ToggleFlyout(a, strip::FlyoutKind::Settings));
        let _ = update(&mut state, Message::ToggleFlyout(b, strip::FlyoutKind::Settings));
        assert!(state.flyout_open.contains(&(a, strip::FlyoutKind::Settings)));
        assert!(state.flyout_open.contains(&(b, strip::FlyoutKind::Settings)));

        // Re-pressing the same strip's own trigger still closes just it.
        let _ = update(&mut state, Message::ToggleFlyout(a, strip::FlyoutKind::Settings));
        assert!(!state.flyout_open.contains(&(a, strip::FlyoutKind::Settings)));
        assert!(state.flyout_open.contains(&(b, strip::FlyoutKind::Settings)));
    }

    #[test]
    fn a_second_kind_on_the_same_strip_replaces_the_first_but_route_stays_exclusive_globally() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(2);

        // Settings then EQ on the SAME strip: only one inline panel can
        // render per strip (the render loop is an if/else-if chain), so
        // the second replaces the first rather than both lingering
        // unrendered in the set.
        let _ = update(&mut state, Message::ToggleFlyout(a, strip::FlyoutKind::Settings));
        let _ = update(&mut state, Message::ToggleFlyout(a, strip::FlyoutKind::Eq));
        assert!(!state.flyout_open.contains(&(a, strip::FlyoutKind::Settings)));
        assert!(state.flyout_open.contains(&(a, strip::FlyoutKind::Eq)));

        // Route stays exclusive with itself across DIFFERENT strips too
        // (the Stack overlay only positions one popover) — but doesn't
        // touch `a`'s still-open EQ.
        let _ = update(&mut state, Message::ToggleFlyout(a, strip::FlyoutKind::Route));
        let _ = update(&mut state, Message::ToggleFlyout(b, strip::FlyoutKind::Route));
        assert!(
            !state.flyout_open.contains(&(a, strip::FlyoutKind::Route)),
            "opening b's Route flyout must close a's"
        );
        assert!(state.flyout_open.contains(&(b, strip::FlyoutKind::Route)));
        assert!(
            state.flyout_open.contains(&(a, strip::FlyoutKind::Eq)),
            "a's EQ flyout must survive b's Route opening"
        );
    }

    #[test]
    fn channels_held_by_the_mute_master_show_as_armed() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let other = ChannelId::Playback(0);
        let _ = update(&mut state, Message::Mute(a, true));
        assert!(!mute_armed(&state, a), "muted, not armed");
        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(mute_armed(&state, a), "held off by the master");
        assert!(!mute_armed(&state, other), "never muted, never armed");
        // A fresh mute makes the master's next press clear that one, so the
        // old set is no longer shown as coming back.
        let _ = update(&mut state, Message::Mute(other, true));
        assert!(!mute_armed(&state, a));
    }

    #[test]
    fn global_mute_master_does_nothing_with_nothing_to_restore() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(!any_channel_muted(&state), "it never mutes everything");
    }

    #[test]
    fn global_solo_toggle_clears_active_solos_without_touching_mute() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(4);
        let _ = update(&mut state, Message::Solo(a, true));
        let _ = update(&mut state, Message::Solo(b, true));
        let _ = update(&mut state, Message::Mute(a, true));
        assert!(any_channel_soloed(&state));

        let _ = update(&mut state, Message::GlobalSoloToggle);
        assert!(!any_channel_soloed(&state));
        assert!(channel_is_muted(&state, a), "solo-clear must not touch mute state");
    }

    #[test]
    fn global_solo_toggle_restores_the_previously_cleared_solos() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let b = ChannelId::Input(4);
        let _ = update(&mut state, Message::Solo(a, true));
        let _ = update(&mut state, Message::Solo(b, true));

        let _ = update(&mut state, Message::GlobalSoloToggle);
        assert!(!any_channel_soloed(&state), "first press clears");

        let _ = update(&mut state, Message::GlobalSoloToggle);
        assert!(channel_is_soloed(&state, a), "second press restores a");
        assert!(channel_is_soloed(&state, b), "second press restores b");
    }

    #[test]
    fn global_solo_toggle_soloing_a_new_channel_after_a_clear_replaces_the_restore_set() {
        let mut state = new(true, None, None);
        // Input(0) and Input(8) are each half of a hardware-linked pair
        // by default (see `mock.rs::test_input_pair_linked_by_default`),
        // so soloing either one mirrors onto its sibling too — accounted
        // for below rather than fought.
        let a = ChannelId::Input(0);
        let c = ChannelId::Input(8);
        let _ = update(&mut state, Message::Solo(a, true));
        let _ = update(&mut state, Message::GlobalSoloToggle);
        assert!(!any_channel_soloed(&state));

        // Solo a different channel manually before pressing the global
        // toggle again — the toggle should clear *that* one (and its
        // linked sibling), not try to restore the stale `a`/`Input(1)`
        // pair from before.
        let _ = update(&mut state, Message::Solo(c, true));
        let _ = update(&mut state, Message::GlobalSoloToggle);
        assert!(!any_channel_soloed(&state));
        assert!(state.last_cleared_solos.contains(&c));
        assert!(!state.last_cleared_solos.contains(&a));
        assert!(!state.last_cleared_solos.contains(&ChannelId::Input(1)));
    }

    #[test]
    fn global_mute_toggle_is_undoable() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::Mute(ChannelId::Input(0), true));
        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(!any_channel_muted(&state));
        let _ = update(&mut state, Message::Undo);
        assert!(channel_is_muted(&state, ChannelId::Input(0)));
    }

    #[test]
    fn a_mix_change_marks_the_recalled_snapshot_modified() {
        let mut state = new(true, None, None);
        state.active_snapshot = Some(1);
        assert!(!state.snapshot_dirty);
        let _ = update(&mut state, Message::VolumeChanged(ChannelId::Input(0), 0, 0.5));
        assert!(state.snapshot_dirty, "a fader drag step is a mix change");
        let _ = update(&mut state, Message::SnapshotClicked(1));
        assert!(!state.snapshot_dirty, "recalling the slot clears it");
        let _ = update(&mut state, Message::ToggleSidebar);
        assert!(!state.snapshot_dirty, "UI-only messages do not count");
    }

    #[test]
    fn renaming_a_strip_sets_then_clears_its_label() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::RenameStart(cid));
        assert_eq!(state.renaming, Some(cid));
        let _ = update(&mut state, Message::RenameChanged("  Vocal  ".into()));
        let _ = update(&mut state, Message::RenameCommit);
        assert_eq!(state.renaming, None);
        assert_eq!(state.labels.get(&cid).map(String::as_str), Some("Vocal"));
        assert!(state.undo_stack.is_empty(), "a label is not part of the mix");

        // Editing starts from the current label; an empty commit restores
        // the hardware name.
        let _ = update(&mut state, Message::RenameStart(cid));
        assert_eq!(state.rename_buf, "Vocal");
        let _ = update(&mut state, Message::RenameChanged(String::new()));
        let _ = update(&mut state, Message::RenameCommit);
        assert!(state.labels.get(&cid).is_none());
    }

    #[test]
    fn a_click_away_accepts_the_typed_name() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::RenameStart(cid));
        // The press that opened the field does not count as a click away.
        let _ = update(&mut state, Message::PointerPressed);
        assert_eq!(state.renaming, Some(cid));
        let _ = update(&mut state, Message::RenameChanged("Kick".into()));
        // A later click: the field reports it lost focus.
        let _ = update(&mut state, Message::FieldFocus(EditField::Rename, cid, false));
        assert_eq!(state.renaming, None);
        assert_eq!(state.labels.get(&cid).map(String::as_str), Some("Kick"));
    }

    #[test]
    fn a_click_inside_the_field_keeps_it_open() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        let _ = update(&mut state, Message::RenameStart(cid));
        let _ = update(&mut state, Message::FieldFocus(EditField::Rename, cid, true));
        assert_eq!(state.renaming, Some(cid));
    }

    #[test]
    fn a_stale_focus_answer_does_not_commit_a_newer_edit() {
        let mut state = new(true, None, None);
        let (a, b) = (ChannelId::Input(0), ChannelId::Input(2));
        let _ = update(&mut state, Message::RenameStart(a));
        let _ = update(&mut state, Message::RenameChanged("Snare".into()));
        // Opening another name accepts the first one.
        let _ = update(&mut state, Message::RenameStart(b));
        assert_eq!(state.labels.get(&a).map(String::as_str), Some("Snare"));
        assert_eq!(state.renaming, Some(b));
        let _ = update(&mut state, Message::FieldFocus(EditField::Rename, a, false));
        assert_eq!(state.renaming, Some(b), "the answer about `a` is ignored");
    }

    #[test]
    fn a_click_away_applies_a_typed_db_value() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(4);
        let _ = update(&mut state, Message::EditStart(cid, "0.0".into()));
        let _ = update(&mut state, Message::EditChanged("-12".into()));
        let _ = update(&mut state, Message::FieldFocus(EditField::Db, cid, false));
        assert_eq!(state.editing, None);
        let db = 20.0 * state.device.inputs()[4].volumes[state.sel_out].log10();
        assert!((db + 12.0).abs() < 0.1, "{db}");
    }

    #[test]
    fn escape_cancels_a_rename_without_touching_the_label() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Playback(0);
        state.labels.insert(cid, "DAW".into());
        let _ = update(&mut state, Message::RenameStart(cid));
        let _ = update(&mut state, Message::RenameChanged("oops".into()));
        let _ = update(&mut state, Message::EscapePressed);
        assert_eq!(state.renaming, None);
        assert_eq!(state.labels.get(&cid).map(String::as_str), Some("DAW"));
    }

    #[test]
    fn prefs_round_trip_through_the_state() {
        let mut state = new(true, None, None);
        state.view = View::Matrix;
        state.sidebar_open = false;
        state.sidebar_width = 300.0;
        state.row_mode = RowMode::Two;
        state.ui_scale = 1.25;
        state.sidebar_panels_open.groups = false;
        state.show_original_names = true;
        state.labels.insert(ChannelId::Output(2), "Monitors".into());
        let saved = prefs_of(&state);

        let mut fresh = new(true, None, None);
        apply_prefs(&mut fresh, saved.clone());
        assert_eq!(prefs_of(&fresh), saved);
    }

    #[test]
    fn an_older_two_row_choice_survives_the_new_layout_setting() {
        let mut state = new(true, None, None);
        let old: crate::prefs::Prefs = serde_json::from_str(r#"{ "two_row": true }"#).unwrap();
        apply_prefs(&mut state, old);
        assert_eq!(state.row_mode, RowMode::Two);
        let old: crate::prefs::Prefs = serde_json::from_str(r#"{ "two_row": false }"#).unwrap();
        apply_prefs(&mut state, old);
        assert_eq!(state.row_mode, RowMode::Auto, "false was only the old default");
        assert!(!serde_json::to_string(&prefs_of(&state)).unwrap().contains("two_row"));
    }

    #[test]
    fn the_window_size_is_kept_and_written_at_close() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::WindowSize(1500.0, 900.0));
        assert_eq!(state.window_width, 1500.0, "the layout width follows");
        assert!(!state.prefs_dirty, "a resize step does not rewrite the file");
        assert_eq!(prefs_of(&state).window_size, Some((1500.0, 900.0)));
        let _ = update(&mut state, Message::SaveNow);
        assert!(!state.prefs_dirty, "flushed at close");
    }

    #[test]
    fn hand_edited_prefs_are_clamped_to_reachable_values() {
        let mut state = new(true, None, None);
        apply_prefs(
            &mut state,
            crate::prefs::Prefs {
                sidebar_width: Some(10_000.0),
                ui_scale: Some(9.0),
                labels: vec![(ChannelId::Input(0), "   ".into())],
                ..Default::default()
            },
        );
        assert_eq!(state.sidebar_width, sidebar::MAX_WIDTH);
        assert_eq!(state.ui_scale, theme::SCALE_MAX);
        assert!(state.labels.is_empty(), "a blank label is no label");
    }

    #[test]
    fn layout_changes_mark_prefs_dirty_and_mix_changes_do_not() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::Mute(ChannelId::Input(0), true));
        assert!(!state.prefs_dirty);
        let _ = update(&mut state, Message::SetRowMode(RowMode::Two));
        assert!(state.prefs_dirty);
        // No `prefs_path` in tests, so the flush writes nothing.
        let _ = update(&mut state, Message::Tick);
        assert!(!state.prefs_dirty);
    }

    #[test]
    fn a_wheel_spin_or_matrix_drag_is_one_undo_step() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(4);
        let before = state.device.inputs()[4].volumes[0];
        // A burst of `VolumeChanged` with no press: a wheel spin, or a
        // Matrix cell drag.
        for v in [0.9, 0.8, 0.7, 0.6] {
            let _ = update(&mut state, Message::VolumeChanged(cid, 0, v));
        }
        assert_eq!(state.undo_stack.len(), 1, "one step for the whole gesture");
        let _ = update(&mut state, Message::Undo);
        assert!((state.device.inputs()[4].volumes[0] - before).abs() < 1e-6);
    }

    #[test]
    fn matrix_drag_then_toggle_undoes_step_by_step() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(4);
        for v in [0.9, 0.7, 0.5] {
            let _ = update(&mut state, Message::VolumeChanged(cid, 1, v));
        }
        let _ = update(&mut state, Message::RangeCleared(cid));
        let _ = update(&mut state, Message::VolumeChanged(cid, 1, 0.0));
        let _ = update(&mut state, Message::RangeCleared(cid));
        let _ = update(&mut state, Message::Undo);
        assert!((state.device.inputs()[4].volumes[1] - 0.5).abs() < 1e-6);
        let _ = update(&mut state, Message::Undo);
        assert!((state.device.inputs()[4].volumes[1] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn hovering_the_matrix_is_not_an_edit() {
        // It once was: every cell the pointer crossed pushed an empty undo
        // step, so Undo "undid" hovers and seemed to do nothing.
        let mut state = new(true, None, None);
        state.active_snapshot = Some(1);
        for col in 0..4 {
            let _ = update(&mut state, Message::MatrixHover(ChannelId::Input(0), col, true));
            let _ = update(&mut state, Message::MatrixHover(ChannelId::Input(0), col, false));
        }
        assert!(state.undo_stack.is_empty());
        assert!(!state.snapshot_dirty);
        assert!(!state.prefs_dirty);
    }

    #[test]
    fn a_late_exit_does_not_clear_the_next_cells_hover() {
        let mut state = new(true, None, None);
        let a = ChannelId::Input(0);
        let _ = update(&mut state, Message::MatrixHover(a, 0, true));
        let _ = update(&mut state, Message::MatrixHover(a, 1, true));
        let _ = update(&mut state, Message::MatrixHover(a, 0, false));
        assert_eq!(state.matrix_hover, Some((a, 1)));
    }

    #[test]
    fn a_knob_turn_is_one_undo_step() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Playback(0);
        for p in [10, 20, 30, 40, 50] {
            let _ = update(&mut state, Message::PanChanged(cid, 0, p));
        }
        assert_eq!(state.undo_stack.len(), 1);
    }

    #[test]
    fn letting_go_ends_the_gesture() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(4);
        let _ = update(&mut state, Message::VolumeChanged(cid, 0, 0.5));
        let _ = update(&mut state, Message::RangeCleared(cid));
        let _ = update(&mut state, Message::VolumeChanged(cid, 0, 0.4));
        assert_eq!(state.undo_stack.len(), 2, "two drags, two steps");
    }

    #[test]
    fn a_different_control_starts_a_new_step() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::VolumeChanged(ChannelId::Input(4), 0, 0.5));
        let _ = update(&mut state, Message::VolumeChanged(ChannelId::Input(5), 0, 0.5));
        let _ = update(&mut state, Message::Mute(ChannelId::Input(0), true));
        assert_eq!(state.undo_stack.len(), 3);
    }

    #[test]
    fn a_simulated_session_keeps_its_snapshots_in_memory() {
        let mut state = new(true, None, None);
        assert_eq!(state.stored_snapshots, [false; 8], "never reads the real slots");
        let cid = ChannelId::Input(4);
        let _ = update(&mut state, Message::SnapshotClicked(3));
        let _ = update(&mut state, Message::VolumeChanged(cid, 0, 0.5));
        let _ = update(&mut state, Message::SnapshotStore);
        assert!(state.stored_snapshots[2]);
        assert!(state.mock_snapshots[2].is_some(), "stored in memory, not on disk");
        let _ = update(&mut state, Message::VolumeChanged(cid, 0, 0.1));
        let _ = update(&mut state, Message::SnapshotClicked(3));
        assert!((state.device.inputs()[4].volumes[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sample_rates_read_as_people_say_them() {
        assert_eq!(super::rate_label(48_000), "48 kHz");
        assert_eq!(super::rate_label(44_100), "44.1 kHz");
        assert_eq!(super::rate_label(176_400), "176.4 kHz");
        assert_eq!(super::rate_label(192_000), "192 kHz");
    }

    #[test]
    fn the_mock_simulates_the_audio_engine() {
        let mut state = new(true, None, None);
        let e = state.engine.expect("the mock has a simulated engine");
        assert_eq!((e.effective_rate(), e.effective_quantum()), (48_000, 256));
        let _ = update(&mut state, Message::EngineRate(96_000));
        let _ = update(&mut state, Message::EngineQuantum(128));
        let e = state.engine.unwrap();
        assert_eq!((e.effective_rate(), e.effective_quantum()), (96_000, 128));
        assert!(state.prefs_dirty, "the choice is remembered");
        assert!(state.undo_stack.is_empty(), "not part of the mix");
        // Back to PipeWire's own values.
        let _ = update(&mut state, Message::EngineQuantum(0));
        assert_eq!(state.engine.unwrap().effective_quantum(), 256);
    }

    #[test]
    fn a_simulated_session_never_changes_the_remembered_engine_choice() {
        // A choice made with `--mock` once landed in the preferences and
        // was forced on the real PipeWire at the next real launch.
        let mut state = new(true, None, None);
        apply_prefs(
            &mut state,
            crate::prefs::Prefs {
                engine_force_quantum: Some(0),
                ..Default::default()
            },
        );
        super::restore_engine(&mut state);
        let _ = update(&mut state, Message::EngineQuantum(128));
        assert_eq!(state.engine.unwrap().effective_quantum(), 128, "simulated");
        assert_eq!(state.engine_force_quantum, 0, "remembered choice untouched");
        assert_eq!(prefs_of(&state).engine_force_quantum, Some(0));
    }

    #[test]
    fn quick_strips_fill_a_tall_view_within_bounds() {
        use super::{quick_fit, QUICK_SCALE_MAX, QUICK_SCALE_MULT};
        assert_eq!(quick_fit(500.0), QUICK_SCALE_MULT, "a short view keeps the floor");
        assert_eq!(quick_fit(5000.0), QUICK_SCALE_MAX, "a huge one stops at the cap");
        let f = quick_fit(1000.0);
        assert!(f > QUICK_SCALE_MULT && f < QUICK_SCALE_MAX, "1080p grows: {f}");
    }

    #[test]
    fn the_initial_zoom_follows_the_monitor_height() {
        use super::initial_zoom_for;
        assert_eq!(initial_zoom_for(1080.0), 1.0);
        assert_eq!(initial_zoom_for(768.0), 1.0);
        assert_eq!(initial_zoom_for(1200.0), 1.1);
        assert_eq!(initial_zoom_for(1440.0), 1.25);
        assert_eq!(initial_zoom_for(2160.0), 1.5);
    }

    #[test]
    fn the_first_launch_picks_a_zoom_from_the_monitor_and_later_ones_keep_the_users() {
        let mut state = new(true, None, None);
        apply_prefs(&mut state, crate::prefs::Prefs::default());
        assert!(state.initial_zoom_pending, "nothing saved: ask the monitor");
        let _ = update(&mut state, Message::MonitorSize(Some(iced::Size::new(2560.0, 1440.0))));
        assert_eq!(state.ui_scale, 1.25);
        assert!(!state.initial_zoom_pending && state.prefs_dirty);
        let _ = update(&mut state, Message::MonitorSize(Some(iced::Size::new(3840.0, 2160.0))));
        assert_eq!(state.ui_scale, 1.25, "asked once only");

        let mut saved = new(true, None, None);
        apply_prefs(&mut saved, crate::prefs::Prefs { ui_scale: Some(1.0), ..Default::default() });
        assert!(!saved.initial_zoom_pending, "a saved zoom is never overridden");
    }

    #[test]
    fn a_narrow_window_folds_the_sidebar_and_a_wide_one_brings_it_back() {
        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::WindowSize(1400.0, 900.0));
        assert!(state.sidebar_open);
        let _ = update(&mut state, Message::WindowSize(800.0, 900.0));
        assert!(!state.sidebar_open && state.auto_folded);
        let _ = update(&mut state, Message::WindowSize(1400.0, 900.0));
        assert!(state.sidebar_open && !state.auto_folded);
        // A fold the user made stays folded.
        let _ = update(&mut state, Message::ToggleSidebar);
        let _ = update(&mut state, Message::WindowSize(800.0, 900.0));
        let _ = update(&mut state, Message::WindowSize(1400.0, 900.0));
        assert!(!state.sidebar_open, "the user's own fold is respected");
    }

    #[test]
    fn the_fold_threshold_follows_the_zoom() {
        // Everything fits beside the sidebar at 80 % in ~970 px, not at 130 %.
        let mut state = new(true, None, None);
        state.ui_scale = 0.8;
        assert!(!super::sidebar_does_not_fit(&state, 971.0));
        state.ui_scale = 1.3;
        assert!(super::sidebar_does_not_fit(&state, 971.0));
    }

    #[test]
    fn the_sidebar_stops_growing_with_the_zoom() {
        let mut state = new(true, None, None);
        state.ui_scale = 1.75;
        assert_eq!(super::sidebar_scale(&state), 1.25);
        state.ui_scale = 1.0;
        assert_eq!(super::sidebar_scale(&state), 1.0);
    }

    #[test]
    fn the_tick_slows_down_at_rest_and_speeds_up_on_any_activity() {
        use super::{tick_interval, TICK_ACTIVE, TICK_IDLE};
        let mut state = new(true, None, None);
        state.last_activity = Instant::now() - Duration::from_secs(10);
        assert_eq!(tick_interval(&state), TICK_IDLE, "nothing happened for a while");

        // The user doing anything wakes it, even a message that changes
        // nothing visible (it costs one fast second).
        let _ = update(&mut state, Message::ToggleSidebarPanel(sidebar::Panel::Groups));
        assert_eq!(tick_interval(&state), TICK_ACTIVE);

        // Background polls do not count as activity.
        state.last_activity = Instant::now() - Duration::from_secs(10);
        let _ = update(&mut state, Message::EnginePoll);
        let _ = update(&mut state, Message::EngineRead(None, None));
        assert_eq!(tick_interval(&state), TICK_IDLE);

        // A pending window resize and a running collapse animation keep it fast.
        state.pending_resize_width = Some(1200.0);
        assert_eq!(tick_interval(&state), TICK_ACTIVE);
    }

    #[test]
    fn a_live_signal_keeps_the_tick_fast_and_silence_releases_it() {
        // The mock generates levels, so a Tick sees a live signal.
        let mut state = new(true, None, None);
        state.last_activity = Instant::now() - Duration::from_secs(10);
        let _ = update(&mut state, Message::Tick);
        assert!(
            state.last_activity.elapsed() < Duration::from_secs(1),
            "meters moving count as activity"
        );
    }

    #[test]
    fn meter_ballistics_do_not_depend_on_the_tick_rate() {
        // One second of a held signal then a second of silence, stepped at
        // three different tick rates: the meter ends in the same place.
        let run = |dt_ms: f32| {
            let mut m = MeterAnim::new();
            let n = (1000.0 / dt_ms) as usize;
            for _ in 0..n {
                m.step_for(1.0, dt_ms);
            }
            let held = m.value;
            for _ in 0..n {
                m.step_for(0.0, dt_ms);
            }
            (held, m.value)
        };
        let (h50, r50) = run(50.0);
        // The rates that exist: 30 Hz with a signal, 20 Hz as it was, and 10 Hz
        // for a slow machine; not 4 Hz, where the changing release curve is
        // sampled too coarsely to compare (and meters are not decaying then).
        for dt in [33.0, 100.0] {
            let (h, r) = run(dt);
            assert!((h - h50).abs() < 0.02, "attack at {dt} ms: {h} vs {h50}");
            assert!((r - r50).abs() < 0.03, "release at {dt} ms: {r} vs {r50}");
        }
    }

    #[test]
    fn the_card_line_shows_the_period_only_while_the_card_plays() {
        use tuxmix_core::pw_clock::CardStream;
        let mut state = new(true, None, None);
        // Idle: the rate and the clock, never a period (the graph's quantum
        // is not what the card runs, and the capture stream may be our own
        // meter's).
        assert_eq!(crate::sidebar::clock_line(&state, "Internal"), "48 kHz \u{00b7} Internal");
        state.card_stream = Some(CardStream { rate: 48_000, period: 256, buffer: 768 });
        assert_eq!(
            crate::sidebar::clock_line(&state, "Internal"),
            "48 kHz \u{00b7} 256 smp \u{00b7} Internal"
        );
    }

    #[test]
    fn dim_toggle_flips_the_device_dim() {
        let mut state = new(true, None, None);
        assert!(!state.device.settings().dim);
        let _ = update(&mut state, Message::DimToggle);
        assert!(state.device.settings().dim);
        let _ = update(&mut state, Message::DimToggle);
        assert!(!state.device.settings().dim);
        assert!(state.undo_stack.is_empty(), "Dim is not part of a scene");
    }

    #[test]
    fn about_modal_messages_are_not_undoable() {
        // Both are pure UI, so neither may push an undo step. Checked through
        // the predicate rather than by dispatching, because dispatching
        // `OpenRepoUrl` would launch the machine's browser.
        assert!(!is_undoable(&Message::ToggleAbout));
        assert!(!is_undoable(&Message::OpenRepoUrl));

        let mut state = new(true, None, None);
        let _ = update(&mut state, Message::ToggleAbout);
        assert!(state.show_about);
        assert!(state.undo_stack.is_empty());
    }

    #[test]
    fn escape_closes_the_about_modal_before_an_edit() {
        // The modal is the topmost thing on screen, so Escape must dismiss
        // it without also abandoning the inline edit underneath it.
        let mut state = new(true, None, None);
        state.show_about = true;
        state.editing = Some(ChannelId::Input(0));
        let _ = update(&mut state, Message::EscapePressed);
        assert!(!state.show_about);
        assert_eq!(state.editing, Some(ChannelId::Input(0)));

        let _ = update(&mut state, Message::EscapePressed);
        assert_eq!(state.editing, None);
    }

    #[test]
    fn escape_closes_the_device_popover_after_the_modal() {
        let mut state = new(true, None, None);
        state.show_about = true;
        state.show_device_panel = true;
        let _ = update(&mut state, Message::EscapePressed);
        assert!(!state.show_about);
        assert!(state.show_device_panel);
        let _ = update(&mut state, Message::EscapePressed);
        assert!(!state.show_device_panel);
    }

    #[test]
    fn layout_clicked_recalls_the_stored_collapsed_set() {
        let mut state = new(true, None, None);
        let cid = ChannelId::Input(0);
        state.layouts[0] = Some(HashSet::from([cid]));
        let _ = update(&mut state, Message::LayoutClicked(1));
        assert_eq!(state.active_layout, Some(1));
        assert!(state.collapsed.contains(&cid));
    }
}
