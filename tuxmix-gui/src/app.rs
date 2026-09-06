use iced::futures::channel::mpsc;
use iced::keyboard::{self, Key};
use iced::widget::{
    button, column, container, mouse_area, opaque, pick_list, row, scrollable, stack, text,
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

use crate::matrix;
use crate::osc::{self, OscCommand, OscConfig, OscOutbound};
use crate::scenes::{load_scene_file, save_scene_file};
use crate::sidebar::{self, Group};
use crate::theme;
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
    match t {
        ChannelType::Mic => ("MIC", theme::MUTE_COLOR),
        ChannelType::Instrument => ("INST", iced::Color::from_rgb8(0xff, 0xb7, 0x4d)),
        ChannelType::Line => ("LINE", theme::ACCENT),
        ChannelType::SPDIF => ("SPDIF", iced::Color::from_rgb8(0xba, 0x68, 0xc8)),
        ChannelType::ADAT => ("ADAT", iced::Color::from_rgb8(0xba, 0x68, 0xc8)),
    }
}

/// Tag colors for the bus rows (not real `ChannelType`s, so not part of
/// `type_tag`) — kept distinct from the input-type palette above so PB/OUT
/// don't just blend into the secondary-text gray everything else uses.
pub const PB_TAG: iced::Color = iced::Color::from_rgb8(0x4d, 0xb6, 0xac);
pub const OUT_TAG: iced::Color = iced::Color::from_rgb8(0x81, 0xc7, 0x84);

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
    fn set_spdif_enabled(&mut self, enabled: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_spdif_enabled(enabled))
    }
    fn set_spdif_emphasis(&mut self, enabled: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_spdif_emphasis(enabled))
    }
    fn set_spdif_professional(&mut self, enabled: bool) -> Result<(), tuxmix_core::Error> {
        delegate!(self, set_spdif_professional(enabled))
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
    pub fn input_meter(&self, idx: usize) -> f32 {
        self.input_meters().get(idx).copied().unwrap_or(0.0)
    }
    pub fn playback_meter(&self, idx: usize) -> f32 {
        self.playback_meters().get(idx).copied().unwrap_or(0.0)
    }
    /// Whether `input_meters()` is a real per-session reading rather than
    /// a hardcoded zero vector — see `draw_meter`'s doc comment in
    /// `widgets/fader.rs`. Mock always has it; the USB backend reads it
    /// from the device (`meters()`); the ALSA/kernel-driver backend has
    /// no meter readback at all yet (see `PROTOCOL.md`'s "VU meters:
    /// conclusion" — the device has none, only host-side computation from
    /// the ISO streams, which this backend doesn't capture).
    pub fn has_input_meters(&self) -> bool {
        match self {
            DeviceHandle::Mock(_) | DeviceHandle::Usb(_) => true,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => false,
        }
    }
    /// Per-channel version of `has_input_meters` — real across the
    /// board for Mock/USB; on the ALSA/kernel-driver backend, only
    /// AN1/AN2 (idx 0/1) have a capture-channel mapping confirmed
    /// against real hardware (`PROTOCOL.md`'s "w0/1 = the AN1/2 record
    /// bus" + a live mic test). Every other input's capture-word
    /// mapping is either genuinely contextual (IN3/4 share a word pair
    /// with the PH3/4 output bus's loopback signal) or disputed between
    /// `PROTOCOL.md` and the more recent `KERNEL-DRIVER.md` — showing a
    /// reading for those would risk attributing a level to the wrong
    /// physical input, worse than the honest "N/A" dashes this falls
    /// back to.
    pub fn has_input_meter(&self, idx: usize) -> bool {
        match self {
            DeviceHandle::Mock(_) | DeviceHandle::Usb(_) => true,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => idx < 2,
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
    /// Output meters, computed host-side like TotalMix: each output's
    /// level is the power sum of every routed source (inputs + playbacks)
    /// scaled by that source's fader into the output.
    pub fn output_meters(&self) -> Vec<f32> {
        let ins = self.input_meters();
        let pbs = self.playback_meters();
        let n_out = self.outputs().len();
        let mut out = vec![0.0f32; n_out];
        for o in 0..n_out {
            let mut p = 0.0f32;
            for i in 0..self.inputs().len() {
                let v = self.inputs()[i].volumes.get(o).copied().unwrap_or(0.0);
                let m = ins.get(i).copied().unwrap_or(0.0);
                p += (m * v) * (m * v);
            }
            for c in 0..self.playbacks().len() {
                let v = self.playbacks()[c].volumes.get(o).copied().unwrap_or(0.0);
                let m = pbs.get(c).copied().unwrap_or(0.0);
                p += (m * v) * (m * v);
            }
            out[o] = p.sqrt().min(1.0);
        }
        out
    }
    pub fn is_mock(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
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
            DeviceHandle::Real(_) => None,
        }
    }
}

/// Which top-level page is showing. `Quick` is the default — a focused
/// "pick a source, adjust it, pick a destination, adjust it" pair of big
/// strips, for the podcaster/streamer/remote-work case that never touches
/// the full matrix. `Mixer`/`Matrix` are the existing dense views for
/// users who need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    TabPressed,
    EscapePressed,
    /// The OS window was resized (or just opened) — tracks the current
    /// width in `TuxMix::window_width` (coalesced, `RESIZE_THROTTLE`) so
    /// `responsive_row` can decide whether a strip row fits or scrolls.
    /// This no longer rescales anything: strips keep their current
    /// `ui_scale` (changed only by the manual `Zoom*` / `WheelZoom`
    /// messages), so a window drag is pure scroll/clip.
    WindowResized(f32),

    /// Ctrl+wheel — zooms the UI in/out (see `zoom`). Ignored unless Ctrl
    /// is held (`TuxMix::modifiers`); plain wheel scrolls as normal.
    WheelZoom(f32),
    /// Discrete manual zoom (Ctrl+= / Ctrl+- / Ctrl+0).
    ZoomIn,
    ZoomOut,
    ZoomReset,

    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    /// The sidebar's global "M" button (`sidebar::msf_row`) — a real
    /// toggle, same shape as `GlobalSoloToggle`: if any channel isn't
    /// muted yet, mutes exactly those and remembers which ones it
    /// touched (`TuxMix::last_globally_muted`); if every channel is
    /// already muted, restores mute *off* on exactly that remembered
    /// set instead of blindly unmuting everything. That distinction
    /// matters when a channel was already muted independently before
    /// the global press — a second press won't un-mute it, since this
    /// action never muted it in the first place.
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
    ClockSourceSelected(String),
    SampleRateSelected(u32),
    SpdifEnabledChanged(bool),
    SpdifEmphasisChanged(bool),
    SpdifProfessionalChanged(bool),

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
    /// Skeleton toggles — visually real 2-way switches (see
    /// `sidebar::SkeletonPair`'s own doc comment), no behavior behind
    /// them. Distinct from the M/S/F row, which isn't wired to
    /// `on_press` at all (see `sidebar::msf_row`).
    ToggleSkeletonPair(sidebar::SkeletonPair),
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
}

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
    /// The one strip+flyout (at most) currently open — either the route
    /// bus picker or the 48V/PAD/Sensitivity settings panel (see
    /// `strip::FlyoutKind`, `route_popover`, `settings_popover`,
    /// `Message::ToggleFlyout`). Only one flyout is ever open at a time,
    /// of either kind. Opens/closes instantly, no width tween — unlike
    /// `collapse_anim`, this isn't animated.
    pub flyout_open: Option<(ChannelId, strip::FlyoutKind)>,
    /// Manual UI zoom for the mixer/matrix views — every text size and
    /// widget dimension there is multiplied by this. Changed only by
    /// `ZoomIn`/`ZoomOut`/`ZoomReset`/Ctrl+molette (`zoom`); it is NOT tied
    /// to window size, so resizing never rescales anything — strips keep
    /// fixed geometry and rows/pages scroll. `SCALE_DEFAULT` is the default.
    pub ui_scale: f32,
    /// Current window width in logical pixels, kept in sync via
    /// `Message::WindowResized` — lets `mixer_view` decide whether a
    /// strip row fits without scrolling (see `responsive_row`).
    /// `Length::Fill` alone can't answer that inside a horizontal
    /// `Scrollable`: iced compresses Fill back down to the content's
    /// natural size on the scroll axis, so centering has to be done by
    /// skipping the scrollable entirely when content already fits —
    /// which needs the real window width tracked in state.
    pub window_width: f32,
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
    /// Which channels `Message::GlobalSoloToggle` most recently cleared
    /// — restored on the next press if nothing's been soloed again in
    /// the meantime. Empty when there's nothing to restore.
    pub last_cleared_solos: Vec<ChannelId>,
    /// Which channels `Message::GlobalMuteToggle` most recently muted
    /// (i.e. weren't already muted before that press) — un-muted again
    /// on the next press, leaving any channel that was independently
    /// muted beforehand untouched. Empty when there's nothing to
    /// restore.
    pub last_globally_muted: Vec<ChannelId>,
    pub sidebar_panels_open: sidebar::PanelsOpen,
    pub skeleton_pairs: sidebar::SkeletonPairs,
    /// Whether the whole right sidebar is expanded — distinct from
    /// `sidebar_panels_open`, which collapses individual panels *within*
    /// an expanded sidebar. `false` reclaims its width for the mixer,
    /// leaving only a thin rail with the button to bring it back.
    pub sidebar_open: bool,
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

    fn step(&mut self, target: f32) {
        self.prev_value = self.value;
        if target >= self.value {
            self.value += (target - self.value) * METER_ATTACK;
            self.release_elapsed_ms = 0.0;
        } else {
            self.release_elapsed_ms =
                (self.release_elapsed_ms + METER_TICK_MS).min(METER_RELEASE_MS);
            let t = self.release_elapsed_ms / METER_RELEASE_MS;
            let alpha = METER_RELEASE_END
                + (METER_RELEASE_START - METER_RELEASE_END) * (1.0 - t) * (1.0 - t);
            self.value += (target - self.value) * alpha;
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
    TuxMix {
        device,
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
        output_meters: vec![MeterAnim::new(); n_outputs],
        collapsed: HashSet::new(),
        collapse_anim: HashMap::new(),
        flyout_open: None,
        ui_scale: theme::SCALE_DEFAULT,
        // Matches `window::Settings::size` in main.rs — updated for real
        // as soon as the first `Opened`/`Resized` event arrives.
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
        last_cleared_solos: Vec::new(),
        last_globally_muted: Vec::new(),
        sidebar_panels_open: sidebar::PanelsOpen::default(),
        skeleton_pairs: sidebar::SkeletonPairs::default(),
        sidebar_open: true,
    }
}

pub fn title(state: &TuxMix) -> String {
    let _ = state;
    "TuxMix - RME Mixer".into()
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
/// (`Message::GlobalMuteToggle`/`GlobalSoloClear`) act on.
fn all_channel_ids(state: &TuxMix) -> Vec<ChannelId> {
    (0..state.device.inputs().len())
        .map(ChannelId::Input)
        .chain((0..state.device.playbacks().len()).map(ChannelId::Playback))
        .chain((0..state.device.outputs().len()).map(ChannelId::Output))
        .collect()
}

/// Whether every channel is currently muted — drives both the global
/// "M" button's toggle direction and its lit state (`sidebar::msf_row`
/// reads this too, hence `pub(crate)`).
pub(crate) fn all_channels_muted(state: &TuxMix) -> bool {
    all_channel_ids(state).into_iter().all(|cid| channel_is_muted(state, cid))
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
        let _ = state.device.set_pan(cid, out, pan);
        notify_osc(state, OscOutbound::Pan(cid, out, pan));
        return;
    }
    let old = i16::from(state.device.pan(cid, out).unwrap_or(pan));
    let delta = i16::from(pan) - old;
    let _ = state.device.set_pan(cid, out, pan);
    notify_osc(state, OscOutbound::Pan(cid, out, pan));
    for sel in others {
        let cur = i16::from(state.device.pan(sel, out).unwrap_or(0));
        let new = (cur + delta).clamp(-100, 100) as i8;
        let _ = state.device.set_pan(sel, out, new);
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

/// `target = None` closes whatever's open; `Some((cid, kind))` opens that
/// flyout, closing any other one first (of either kind — only one is ever
/// open at a time). Instant, no animation.
fn set_flyout_open(state: &mut TuxMix, target: Option<(ChannelId, strip::FlyoutKind)>) {
    state.flyout_open = target;
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
            | Message::WheelZoom(_)
            | Message::ZoomIn
            | Message::ZoomOut
            | Message::ZoomReset
            | Message::VolumeChanged(..)
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
            | Message::Undo
            | Message::Redo
            | Message::ToggleSidebarPanel(_)
            | Message::ToggleSidebar
            | Message::ToggleSkeletonPair(_)
            | Message::SnapshotStore
            | Message::GroupEditToggle
            | Message::GroupLinkToggle(..)
            | Message::GroupClear
            | Message::LayoutClicked(_)
            | Message::LayoutStore
    )
}

pub fn update(state: &mut TuxMix, message: Message) -> Task<Message> {
    if is_undoable(&message) {
        let scene = state.device.capture_scene();
        state.undo_stack.push(scene);
        if state.undo_stack.len() > UNDO_STACK_CAP {
            state.undo_stack.remove(0);
        }
        state.redo_stack.clear();
    }
    match message {
        Message::Tick => {
            // Resize coalescing: if the last `Resized` of a drag landed
            // inside the throttle window, flush it here so the width always
            // settles (even if no further `Resized` arrives). 50 ms is well
            // past `RESIZE_THROTTLE`, so this only ever catches the tail end
            // of a drag.
            apply_pending_resize(state);
            let _ = state.device.poll_events();
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
                m.step(in_levels.get(i).copied().unwrap_or(0.0));
            }
            let pb_levels = state.device.playback_meters();
            for (i, m) in state.playback_meters.iter_mut().enumerate() {
                m.step(pb_levels.get(i).copied().unwrap_or(0.0));
            }
            let out_levels = state.device.output_meters();
            for (i, m) in state.output_meters.iter_mut().enumerate() {
                m.step(out_levels.get(i).copied().unwrap_or(0.0));
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
                    let _ = state.device.apply_scene(&their);
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
        // Immediate save (window close) — the debounce above would lose
        // the last few seconds of changes. Sync with the other UI first
        // (see the 3 s auto-save in `Message::Tick`). The mock never
        // persists (same reasoning as the auto-save).
        Message::SaveNow => {
            if !state.device.is_mock() {
                if let Some(their) = tuxmix_core::scene::auto_scene_written_by_other(
                    state.last_saved_json.as_deref(),
                ) {
                    let _ = state.device.apply_scene(&their);
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
        Message::SetView(v) => state.view = v,
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
        Message::QuickChannelSelected(cid) => state.quick_channel = cid,
        Message::SelectOutput(i) => {
            state.sel_out = i;
            // Selecting a bus from anywhere (top bar or a strip's own route
            // flyout) dismisses whatever flyout is open — it did its job.
            set_flyout_open(state, None);
        }
        Message::ModifiersChanged(m) => state.modifiers = m,
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
            if state.editing.is_some() {
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
            let to_mute: Vec<ChannelId> =
                all_channel_ids(state).into_iter().filter(|&cid| !channel_is_muted(state, cid)).collect();
            if to_mute.is_empty() {
                for cid in std::mem::take(&mut state.last_globally_muted) {
                    set_channel_mute(state, cid, false);
                    notify_osc(state, OscOutbound::Mute(cid, false));
                }
            } else {
                for &cid in &to_mute {
                    set_channel_mute(state, cid, true);
                    notify_osc(state, OscOutbound::Mute(cid, true));
                }
                state.last_globally_muted = to_mute;
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
            let _ = state.device.set_phantom(idx, p);
        }
        Message::Pad(idx, p) => {
            let _ = state.device.set_pad(idx, p);
        }
        Message::Gain(idx, g) => {
            let _ = state.device.set_gain(idx, g);
        }
        Message::Sensitivity(idx, plus4) => {
            let s = if plus4 {
                tuxmix_core::Sensitivity::Plus4dBu
            } else {
                tuxmix_core::Sensitivity::Minus10dBV
            };
            let _ = state.device.set_sensitivity(idx, s);
        }
        Message::TrimChanged(idx, db) => {
            let _ = state.device.set_trim(idx, db);
        }
        Message::TrimReset(idx) => {
            let _ = state.device.set_trim(idx, 0.0);
        }
        Message::EqEnabled(idx, on) => {
            let _ = state.device.set_eq_enabled(idx, on);
        }
        Message::EqBandType(idx, band, t) => {
            let _ = state.device.set_eq_band_type(idx, band, t);
        }
        Message::EqBandFreq(idx, band, freq) => {
            let _ = state.device.set_eq_band_freq(idx, band, freq);
        }
        Message::EqBandQ(idx, band, q) => {
            let _ = state.device.set_eq_band_q(idx, band, q);
        }
        Message::EqBandGain(idx, band, gain) => {
            let _ = state.device.set_eq_band_gain(idx, band, gain);
        }
        Message::EqLowCutFreq(idx, freq) => {
            let _ = state.device.set_eq_low_cut_freq(idx, freq);
        }
        Message::EqLowCutSlope(idx, slope) => {
            let _ = state.device.set_eq_low_cut_slope(idx, slope);
        }
        Message::LoopbackChanged(i, on) => {
            let pair = if state.device.outputs_one_per_pair() {
                i
            } else {
                i / 2
            };
            let _ = state.device.set_loopback(pair, on);
        }
        Message::StereoLinkChanged(cid, linked) => {
            let _ = match cid {
                ChannelId::Input(i) => state.device.set_input_pair_linked(i / 2, linked),
                ChannelId::Playback(i) => state.device.set_playback_linked(i / 2, linked),
                ChannelId::Output(i) => state.device.set_output_linked(i / 2, linked),
            };
        }
        Message::PitchChanged(v) => {
            let _ = state.device.set_pitch(v);
        }
        Message::WidthChanged(v) => {
            let _ = state.device.set_width(v);
        }
        Message::MsProcChanged(on) => {
            let _ = state.device.set_ms_proc(on);
        }
        Message::An12Changed(on) => {
            let _ = state.device.set_an12(on);
        }
        Message::InputLinkChanged(on) => {
            let _ = state.device.set_input_link(on);
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
                    let _ = state.device.set_pan(sel, out, 0);
                    notify_osc(state, OscOutbound::Pan(sel, out, 0));
                }
            } else {
                let _ = state.device.set_pan(cid, out, 0);
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
            let target = if state.flyout_open == Some((cid, kind)) {
                None
            } else {
                Some((cid, kind))
            };
            set_flyout_open(state, target);
        }
        Message::CloseFlyout => {
            set_flyout_open(state, None);
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
        Message::ToggleDevicePanel => state.show_device_panel = !state.show_device_panel,
        Message::ClockSourceSelected(source) => {
            let _ = state.device.set_clock_source(&source);
        }
        Message::SampleRateSelected(rate) => {
            let _ = state.device.set_sample_rate(rate);
        }
        Message::SpdifEnabledChanged(v) => {
            let _ = state.device.set_spdif_enabled(v);
        }
        Message::SpdifEmphasisChanged(v) => {
            let _ = state.device.set_spdif_emphasis(v);
        }
        Message::SpdifProfessionalChanged(v) => {
            let _ = state.device.set_spdif_professional(v);
        }

        // ── Right sidebar ────────────────────────────────────────────
        Message::Undo => {
            if let Some(scene) = state.undo_stack.pop() {
                state.redo_stack.push(state.device.capture_scene());
                if let Err(e) = state.device.apply_scene(&scene) {
                    log::warn!("Undo failed to apply scene: {e}");
                }
            }
        }
        Message::Redo => {
            if let Some(scene) = state.redo_stack.pop() {
                state.undo_stack.push(state.device.capture_scene());
                if let Err(e) = state.device.apply_scene(&scene) {
                    log::warn!("Redo failed to apply scene: {e}");
                }
            }
        }
        Message::ToggleSidebarPanel(panel) => state.sidebar_panels_open.toggle(panel),
        Message::ToggleSidebar => state.sidebar_open = !state.sidebar_open,
        Message::ToggleSkeletonPair(pair) => state.skeleton_pairs.toggle(pair),
        Message::SnapshotClicked(n) => {
            state.active_snapshot = Some(n as usize);
            if let Some(scene) = load_scene_file(&format!("Mix {n}")) {
                if let Err(e) = state.device.apply_scene(&scene) {
                    log::warn!("Failed to apply snapshot 'Mix {n}': {e}");
                }
            }
        }
        Message::SnapshotStore => {
            if let Some(n) = state.active_snapshot {
                if let Err(e) =
                    save_scene_file(&format!("Mix {n}"), &state.device.capture_scene())
                {
                    log::warn!("Failed to store snapshot 'Mix {n}': {e}");
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

pub fn subscription(state: &TuxMix) -> Subscription<Message> {
    let mut subs = vec![
        iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick),
        iced::event::listen_with(handle_global_event),
    ];
    // Only running while a collapse/expand transition is actually in
    // flight — plain `column`/`container` widgets can't self-request a
    // redraw the way the canvas-based fader/meter animations do, so a
    // much faster timer stands in for that during the ~160ms transition,
    // then switches itself back off once `collapse_anim` empties out.
    if !state.collapse_anim.is_empty() {
        subs.push(iced::time::every(Duration::from_millis(8)).map(|_| Message::CollapseTick));
    }
    // Only running when `--osc` was passed — see `osc.rs`.
    if let Some(config) = &state.osc_config {
        subs.push(Subscription::run_with(*config, osc::worker));
    }
    Subscription::batch(subs)
}

fn handle_global_event(
    event: iced::Event,
    _status: iced::event::Status,
    _id: window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            // Ctrl+= / Ctrl+- / Ctrl+0 — manual zoom. Accept "=" and "+"
            // (shifted/unshifted) and "0"; requires Ctrl.
            if modifiers.contains(keyboard::Modifiers::CTRL) {
                if let Key::Character(c) = &key {
                    let zoom = match c.as_str() {
                        "=" | "+" => Some(Message::ZoomIn),
                        "-" | "_" => Some(Message::ZoomOut),
                        "0" => Some(Message::ZoomReset),
                        _ => None,
                    };
                    if zoom.is_some() {
                        return zoom;
                    }
                }
            }
            match key {
                Key::Named(keyboard::key::Named::Tab) => Some(Message::TabPressed),
                Key::Named(keyboard::key::Named::Escape) => Some(Message::EscapePressed),
                _ => None,
            }
        }
        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::ModifiersChanged(m))
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
            Some(Message::WindowResized(size.width))
        }
        iced::Event::Window(window::Event::CloseRequested) => Some(Message::SaveNow),
        iced::Event::Window(window::Event::Opened { size, .. }) => {
            Some(Message::WindowResized(size.width))
        }
        _ => None,
    }
}

// ── View ─────────────────────────────────────────────────────────

pub fn view(state: &TuxMix) -> Element<'_, Message> {
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
    let body = row![content, sidebar::sidebar(state)]
        .width(Length::Fill)
        .height(Length::Fill);
    let mut col = column![top, body]
        .width(Length::Fill)
        .height(Length::Fill);
    if state.show_osc_log {
        col = col.push(osc_log_panel(state));
    }
    if state.show_device_panel {
        col = col.push(device_panel(state));
    }
    col.into()
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

/// A docked drawer for settings that have no per-channel strip to live
/// on — clock source and SPDIF format flags. Same pattern as
/// `osc_log_panel`: fixed height, docked under the main view, toggled
/// from the top bar (here, the "<clock> ▾" button next to Submix).
fn device_panel(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let settings = state.device.settings();

    let header = row![
        text("DEVICE SETTINGS")
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_SM * scale),
        iced::widget::Space::new().width(Length::Fill),
        iced::widget::button(text("Close").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .style(theme::plain_button)
            .on_press(Message::ToggleDevicePanel),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let clock_row = row![
        text("Clock Source")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        pick_list(
            settings.clock_sources.clone(),
            Some(settings.clock_source.clone()),
            Message::ClockSourceSelected,
        )
        .style(theme::pick_list)
        .menu_style(theme::menu)
        .text_size(theme::TEXT_MD * scale),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    // The device's supported sample-rate classes (alt 1/2/3 — see
    // PROTOCOL.md "Sample rate"): 32/44.1/48/64/88.2, 96/128, 176.4/192.
    // The pick list shows the common ones; a custom entry is not
    // editable, so the full supported set is listed.
    let rates: Vec<u32> = vec![
        32000, 44100, 48000, 64000, 88200, 96000, 128000, 176400, 192000,
    ];
    let rate_row = row![
        text("Sample Rate")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        pick_list(
            rates,
            Some(settings.sample_rate),
            Message::SampleRateSelected,
        )
        .style(theme::pick_list)
        .menu_style(theme::menu)
        .text_size(theme::TEXT_MD * scale),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let spdif_toggle = |label: &'static str, active: bool, on_toggle: fn(bool) -> Message| {
        iced::widget::button(text(label).size(theme::TEXT_SM * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_LG * scale])
            .style(theme::toggle_button(active, theme::ACCENT))
            .on_press(on_toggle(!active))
    };

    let spdif_row = row![
        text("SPDIF")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        spdif_toggle(
            "Enabled",
            settings.spdif_enabled,
            Message::SpdifEnabledChanged
        ),
        spdif_toggle(
            "Emphasis",
            settings.spdif_emphasis,
            Message::SpdifEmphasisChanged
        ),
        spdif_toggle(
            "Professional",
            settings.spdif_professional,
            Message::SpdifProfessionalChanged
        ),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let modifiers = state.modifiers;
    let pitch_row = row![
        text("Pitch")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
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
        text("Width")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
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
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    let toggle_row = row![
        text("Global")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        spdif_toggle("MS Proc", settings.ms_proc, Message::MsProcChanged),
        spdif_toggle("AN 1>2", settings.an12, Message::An12Changed),
        spdif_toggle("Input Link", settings.input_link, Message::InputLinkChanged),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center);

    container(
        column![header, clock_row, rate_row, spdif_row, pitch_row, toggle_row]
            .spacing(theme::SPACE_LG * scale)
            .width(Length::Fill),
    )
    .style(theme::top_bar)
    .padding(theme::SPACE_MD * scale)
    .width(Length::Fill)
    .into()
}

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

/// Wraps a cluster of related controls in a recessed "chip" so the top bar
/// reads as grouped sections instead of one long undifferentiated row.
fn chip<'a>(content: impl Into<Element<'a, Message>>, scale: f32) -> Element<'a, Message> {
    container(content)
        .style(theme::chip)
        .padding([theme::SPACE_SM * scale, theme::SPACE_XL * scale])
        .into()
}

/// A thin vertical separator between sub-groups inside a merged chip —
/// lighter-weight than another chip boundary, just enough to break up
/// dense runs of controls (Scene tools / Submix / Clock) without adding a
/// third level of boxing.
fn v_divider<'a>(scale: f32) -> Element<'a, Message> {
    container(iced::widget::Space::new().width(1).height(16.0 * scale))
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::BORDER)),
            ..container::Style::default()
        })
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
    let session = chip(
        row![
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
            .text_size(theme::TEXT_MD * scale),
            v_divider(scale),
            iced::widget::button(
                text(format!("{} \u{25BE}", state.device.settings().clock_source))
                    .size(theme::TEXT_XS * scale),
            )
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .style(theme::plain_button)
            .on_press(Message::ToggleDevicePanel),
        ]
        .spacing(theme::SPACE_LG)
        .align_y(iced::Alignment::Center),
        scale,
    );

    let mut bar = row![
        text("TuxMix")
            .color(theme::ACCENT)
            .size(theme::TEXT_XL * scale),
        tab_toggle,
        // A small flexible pusher rather than the whole remaining width —
        // `session` below claims the bulk of it (`FillPortion(20)`), so
        // this just keeps it from being flush against `tab_toggle` on a
        // wide window without competing with it for space on a narrow one.
        iced::widget::Space::new().width(Length::FillPortion(1)),
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
    // `session` (Scene/Save/load/Submix/Clock) has no natural upper bound
    // on its own width — at the default 1280px window it used to overflow
    // past the window's right edge entirely, making the Clock Source
    // button (which opens `device_panel`) unreachable without resizing
    // wider first. A horizontal scrollable clips it to whatever room
    // `FillPortion(20)` actually leaves it and offers a scrollbar instead
    // of silently running off-screen — same overflow idiom `responsive_row`
    // already uses for strip rows, applied here too.
    bar = bar.push(
        scrollable(session)
            .direction(scrollable::Direction::Horizontal(theme::thin_scrollbar()))
            .style(theme::scrollable)
            .width(Length::FillPortion(20)),
    );

    container(bar)
        .style(theme::top_bar)
        .padding([theme::SPACE_LG * scale, theme::SPACE_XXL * scale])
        .width(Length::Fill)
        .into()
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
        name: String::new(),
        type_tag: None,
        vol: 0.0,
        pan: 0,
        meter: fader::MeterFrame::still(0.0),
        meter_available: false,
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
        loopback: false,
        stereo_linked: false,
        open_flyout: state.flyout_open.and_then(|(c, k)| (c == cid).then_some(k)),
        mute: false,
        solo: false,
        default_vol: 1.0,
        editing: state.editing == Some(cid),
        edit_buf: &state.edit_buf,
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
                meter_available: state.device.has_playback_meters(),
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
                // Host-computed from routed sources — only as real as
                // whichever of input/playback meters feed it (see
                // `DeviceHandle::output_meters`).
                meter_available: state.device.has_input_meters()
                    || state.device.has_playback_meters(),
                loopback: ch.loopback,
                stereo_linked: state.device.output_linked(i / 2),
                mute: ch.mute,
                solo: ch.solo,
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
fn route_popover(state: &TuxMix, width: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let mut list = column![].spacing(theme::SPACE_TIGHT);
    for (idx, label) in OUT_LABELS.iter().enumerate() {
        list = list.push(
            button(text(*label).size(theme::TEXT_SM * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(idx == state.sel_out, theme::ACCENT))
                .on_press(Message::SelectOutput(idx)),
        );
    }
    container(list)
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(width))
        .style(theme::top_bar)
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
fn settings_popover(state: &TuxMix, cid: ChannelId, width: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let p = strip_params(state, cid, state.sel_out);

    let mut col = column![].spacing(theme::SPACE_SM * scale);

    // Stereo link/split — every channel kind has this (TotalMix shows
    // the button on every strip, not just AN1/2 or hardware outputs; see
    // `RmeDevice::{input_pair,playback,output}_linked`'s doc comments for
    // why this is a pure display/control-grouping concept, not tied to
    // a specific hardware register).
    col = col.push(
        button(text("STEREO").size(theme::TEXT_SM * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(p.stereo_linked, theme::ACCENT))
            .on_press(Message::StereoLinkChanged(cid, !p.stereo_linked)),
    );

    let ChannelId::Input(idx) = cid else {
        return container(col)
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fixed(width))
            .style(theme::top_bar)
            .clip(true)
            .into();
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
        // Neither real backend actually has this control yet
        // (`RmeDevice::set_sensitivity` errors on both ALSA and USB —
        // see `babyface.rs`/`usb.rs`'s own doc comments) — dimmed and
        // unpressable on real hardware, same treatment as this
        // session's other confirmed-N/A controls (Output balance,
        // `meters: post fx/RMS`). Mock keeps it live since it's the
        // only backend that actually implements the switch, so the
        // wiring stays exercisable without real hardware.
        if state.device.is_mock() {
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

    container(col)
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(width))
        .style(theme::top_bar)
        .clip(true)
        .into()
}

/// The Trim flyout's content: a single knob, -65..+6 dB on the fader's
/// own master curve (see `RmeDevice::set_trim`). Every hardware input
/// gets this trigger (`StripParams::has_trim`, unconditional — unlike
/// Gain, which is Mic/Instrument-only), so unlike `eq_popover` there's
/// no `Option`-shaped hardware gate to check here, just the channel kind.
/// Same "pushes the row" shape as `settings_popover`/`eq_popover`.
fn trim_popover(state: &TuxMix, cid: ChannelId, width: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
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

    container(col)
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(width))
        .style(theme::top_bar)
        .clip(true)
        .into()
}

/// The EQ flyout's content: enable toggle, 3 parametric bands (type/freq/
/// Q/gain), and the low-cut filter (freq/slope) — analog inputs only
/// (`ch.eq: Some(_)`, see `InputChannel::eq`). Same "pushes the row"
/// shape as `settings_popover`, just wider (`strip::EQ_FLYOUT_W`) to fit
/// 3 knobs per band row.
///
/// Frequency knobs are linear over 20-20000 Hz for now, not log-scaled —
/// a real audio frequency control wants log, but that's a `Knob` widget
/// enhancement (visual polish), not part of this pass, which is about
/// making the control reachable at all.
fn eq_popover(state: &TuxMix, cid: ChannelId, width: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let ChannelId::Input(idx) = cid else {
        return container(iced::widget::Space::new()).into();
    };
    let Some(eq) = state.device.inputs()[idx].eq else {
        return container(iced::widget::Space::new()).into();
    };
    let modifiers = state.modifiers;

    let mut col = column![].spacing(theme::SPACE_SM * scale);

    col = col.push(
        button(text("EQ Enabled").size(theme::TEXT_SM * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(eq.enabled, theme::ACCENT))
            .on_press(Message::EqEnabled(idx, !eq.enabled)),
    );

    for band in 0..3 {
        let b = eq.bands[band];
        let type_label = match b.band_type {
            EqBandType::Off => "Off",
            EqBandType::Bell => "Bell",
            EqBandType::LowShelf => "Low Shelf",
            EqBandType::HighShelf => "High Shelf",
        };
        let next_type = match b.band_type {
            EqBandType::Off => EqBandType::Bell,
            EqBandType::Bell => EqBandType::LowShelf,
            EqBandType::LowShelf => EqBandType::HighShelf,
            EqBandType::HighShelf => EqBandType::Off,
        };
        col = col.push(
            row![
                text(format!("Band {}", band + 1))
                    .size(theme::TEXT_SM * scale)
                    .color(theme::TEXT_SEC),
                button(text(type_label).size(theme::TEXT_XS * scale))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                    .style(theme::plain_button)
                    .on_press(Message::EqBandType(idx, band, next_type)),
            ]
            .spacing(theme::SPACE_TIGHT)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
        );
        col = col.push(
            row![
                container(hint(
                    knob(Knob {
                        value: b.freq_hz as f32,
                        range: (20.0, 20_000.0),
                        label: format!("{}Hz", b.freq_hz),
                        arc_from_center: false,
                        interactive: true,
                        log_scale: true,
                        modifiers,
                        scale,
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
                container(hint(
                    knob(Knob {
                        value: b.q,
                        range: (0.05, 10.0),
                        label: format!("{:.2}", b.q),
                        arc_from_center: false,
                        interactive: true,
                        log_scale: false,
                        modifiers,
                        scale,
                        on_change: Box::new(move |v| Message::EqBandQ(
                            idx,
                            band,
                            v.clamp(0.05, 10.0)
                        )),
                        on_reset: Box::new(move || Message::EqBandQ(idx, band, 0.7)),
                    }),
                    "Band Q",
                    scale,
                ))
                .width(Length::Fill)
                .center_x(Length::Fill),
                container(hint(
                    knob(Knob {
                        value: b.gain_db,
                        range: (-24.0, 24.0),
                        label: format!("{:+.1}", b.gain_db),
                        arc_from_center: true,
                        interactive: true,
                        log_scale: false,
                        modifiers,
                        scale,
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
            ]
            .spacing(theme::SPACE_TIGHT)
            .width(Length::Fill),
        );
    }

    col = col.push(
        text("Low Cut")
            .size(theme::TEXT_SM * scale)
            .color(theme::TEXT_SEC),
    );
    col = col.push(
        container(hint(
            knob(Knob {
                value: eq.low_cut_freq_hz as f32,
                range: (20.0, 20_000.0),
                label: format!("{}Hz", eq.low_cut_freq_hz),
                arc_from_center: false,
                interactive: true,
                log_scale: true,
                modifiers,
                scale,
                on_change: Box::new(move |v| {
                    Message::EqLowCutFreq(idx, v.round().clamp(20.0, 20_000.0) as u16)
                }),
                on_reset: Box::new(move || Message::EqLowCutFreq(idx, 20)),
            }),
            "Low-cut frequency",
            scale,
        ))
        .width(Length::Fill)
        .center_x(Length::Fill),
    );
    let mut slope_row = row![].spacing(theme::SPACE_TIGHT).width(Length::Fill);
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
    col = col.push(slope_row);

    container(col)
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(width))
        .style(theme::top_bar)
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
    let (Some(x), Some((_cid, strip::FlyoutKind::Route))) = (open_x, state.flyout_open) else {
        return row_element;
    };
    let content = route_popover(state, strip::FLYOUT_W);
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
            }
            let strip_widget = strip::strip(params);
            let mut item_width = rendered_strip_width(state, cid);
            // Settings/EQ push the row instead of overlaying it (see
            // `with_flyout`'s doc comment for why) — done here, by
            // widening this loop iteration's own item, rather than in
            // `with_flyout`, since ordinary `row!` layout already pushes
            // every later sibling over for free once this one item is
            // wider.
            if state.flyout_open == Some((cid, strip::FlyoutKind::Settings)) {
                // Same width as the strip itself (see the reference
                // design), not the Route flyout's own fixed `FLYOUT_W` —
                // a dropdown list of bus names and a settings panel of
                // knobs/buttons don't need to match each other's width,
                // just their own strip's.
                let panel_w = item_width;
                input_strips = input_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else if state.flyout_open == Some((cid, strip::FlyoutKind::Eq)) {
                // Unlike Settings, sized to fit 3 knobs per band row rather
                // than matching the (much narrower) strip width — see
                // `strip::EQ_FLYOUT_W`'s doc comment.
                let panel_w = strip::EQ_FLYOUT_W;
                input_strips = input_strips.push(
                    row![strip_widget, eq_popover(state, cid, panel_w)].spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else if state.flyout_open == Some((cid, strip::FlyoutKind::Trim)) {
                // One knob — reuses the Route flyout's own fixed width
                // rather than a third bespoke constant; Settings' "match
                // the strip's own width" doesn't apply here since there's
                // no strip-width-scaling content (no button rows) to fit.
                let panel_w = strip::FLYOUT_W;
                input_strips = input_strips.push(
                    row![strip_widget, trim_popover(state, cid, panel_w)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else {
                input_strips = input_strips.push(strip_widget);
            }
            input_width += item_width;
            input_item_count += 1;
            if state.flyout_open.map(|(c, _)| c) == Some(cid) {
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
            }
            let strip_widget = strip::strip(params);
            let mut item_width = rendered_strip_width(state, cid);
            if state.flyout_open == Some((cid, strip::FlyoutKind::Settings)) {
                let panel_w = item_width;
                pb_strips = pb_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w)]
                        .spacing(theme::SPACE_MD),
                );
                item_width += panel_w + theme::SPACE_MD;
            } else {
                pb_strips = pb_strips.push(strip_widget);
            }
            pb_width += item_width;
            pb_item_count += 1;
            if state.flyout_open.map(|(c, _)| c) == Some(cid) {
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
            }
            let strip_widget = strip::strip(params);
            let mut item_width = rendered_strip_width(state, cid);
            if state.flyout_open == Some((cid, strip::FlyoutKind::Settings)) {
                let panel_w = item_width;
                out_strips = out_strips.push(
                    row![strip_widget, settings_popover(state, cid, panel_w)]
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
    let available_width = (state.window_width - 2.0 * theme::SPACE_XL - 4.0).max(0.0);

    let scale = state.ui_scale;
    let body = column![
        section_header("HARDWARE INPUTS", scale),
        text(format!(
            "Submix: {} - Tab for Matrix",
            OUT_LABELS[state.sel_out]
        ))
        .color(theme::TEXT_SEC)
        .size(theme::TEXT_XS * scale),
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
        section_header("HARDWARE OUTPUTS", scale),
        responsive_row(out_strips.into(), out_width, available_width),
    ]
    .spacing(theme::SPACE_LG)
    .width(Length::Fill);

    page(body)
}

fn matrix_view(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let body = column![
        section_header("MATRIX MIXER", scale),
        text("Volume per input per output - Tab for Quick")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        matrix::view(state),
    ]
    .spacing(theme::SPACE_LG)
    .width(Length::Fill);

    page(body)
}

/// Every channel selectable as a Quick Control source, in display order —
/// inputs then playbacks, matching `channel_order`. Outputs are excluded:
/// the Quick view's destination block is always `Output(state.sel_out)`,
/// picked via the existing Submix selector in the top bar rather than a
/// second picker here.
fn quick_channel_options(state: &TuxMix) -> Vec<(ChannelId, String)> {
    let mut opts = Vec::new();
    for (i, ch) in state.device.inputs().iter().enumerate() {
        opts.push((
            ChannelId::Input(i),
            format!("{} · IN", short_label(&ch.name)),
        ));
    }
    for (i, ch) in state.device.playbacks().iter().enumerate() {
        opts.push((
            ChannelId::Playback(i),
            format!("{} · PB", short_label(&ch.name)),
        ));
    }
    opts
}

/// How much bigger than a normal strip the two Quick Control blocks are —
/// the point of this view is that the one source and one destination that
/// matter are large, easy targets, not a dense grid.
const QUICK_SCALE_MULT: f32 = 2.0;

fn quick_view(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let options = quick_channel_options(state);
    let labels: Vec<String> = options.iter().map(|(_, l)| l.clone()).collect();
    let cids: Vec<ChannelId> = options.iter().map(|(c, _)| *c).collect();
    let current = options
        .iter()
        .find(|(cid, _)| *cid == state.quick_channel)
        .map(|(_, l)| l.clone());

    let labels_for_pick = labels.clone();
    let source_picker = row![
        text("Source")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        pick_list(labels, current, move |label| {
            let idx = labels_for_pick
                .iter()
                .position(|l| *l == label)
                .unwrap_or(0);
            Message::QuickChannelSelected(cids[idx])
        })
        .placeholder("select...")
        .style(theme::pick_list)
        .menu_style(theme::menu)
        .text_size(theme::TEXT_MD * scale),
    ]
    .spacing(theme::SPACE_MD)
    .align_y(iced::Alignment::Center);

    let big_scale = scale * QUICK_SCALE_MULT;
    let mut source_params = strip_params(state, state.quick_channel, state.sel_out);
    source_params.scale = big_scale;
    // `state.sel_out` indexes the 6 *submix pairs* (`OUT_LABELS`), but
    // `device.outputs()` is a flat list of individual physical channels,
    // two per pair (`build_outputs`: index `2*i`/`2*i+1` = left/right of
    // pair `i`) — so the pair's representative channel is `sel_out * 2`,
    // not `sel_out` itself.
    let dest_cid = ChannelId::Output(state.sel_out * 2);
    let mut dest_params = strip_params(state, dest_cid, state.sel_out);
    dest_params.scale = big_scale;

    let blocks = row![strip::strip(source_params), strip::strip(dest_params)]
        .spacing(theme::SPACE_XL * scale * QUICK_SCALE_MULT)
        .align_y(iced::Alignment::Start);

    let body = column![
        section_header("QUICK CONTROL", scale),
        text("Pick a source, adjust it, adjust the destination - no routing, no matrix. Tab for Mixer")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        source_picker,
        blocks,
    ]
    .spacing(theme::SPACE_LG)
    .width(Length::Fill);

    page(body)
}

#[cfg(test)]
mod tests {
    use super::{
        all_channel_ids, all_channels_muted, any_channel_soloed, apply_pending_resize,
        channel_is_muted, channel_is_soloed, new, update, zoom, ChannelId, MeterAnim, Message,
        ZOOM_STEP,
    };
    use crate::sidebar;
    use std::collections::HashSet;
    use tuxmix_core::RmeDevice;

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
    fn global_mute_toggle_mutes_everything_then_unmutes_on_second_press() {
        let mut state = new(true, None, None);
        assert!(!all_channels_muted(&state), "mock starts fully unmuted");

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(all_channels_muted(&state));
        for cid in all_channel_ids(&state) {
            assert!(channel_is_muted(&state, cid), "{cid:?} should be muted");
        }

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(!all_channels_muted(&state));
        for cid in all_channel_ids(&state) {
            assert!(!channel_is_muted(&state, cid), "{cid:?} should be unmuted");
        }
    }

    #[test]
    fn global_mute_toggle_leaves_a_pre_muted_channel_muted_after_restoring() {
        let mut state = new(true, None, None);
        // Input(8) is half of a hardware-linked pair by default, so
        // muting it mirrors onto Input(9) too — both are the
        // "independently muted before the global press" channels here.
        let pre_muted = ChannelId::Input(8);
        let its_pair_sibling = ChannelId::Input(9);
        let _ = update(&mut state, Message::Mute(pre_muted, true));

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(all_channels_muted(&state));

        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(
            channel_is_muted(&state, pre_muted),
            "a channel muted before the global press must stay muted after restoring"
        );
        assert!(channel_is_muted(&state, its_pair_sibling));
        // Everything the global press itself muted should be back off.
        for cid in all_channel_ids(&state) {
            if cid != pre_muted && cid != its_pair_sibling {
                assert!(!channel_is_muted(&state, cid), "{cid:?} should be unmuted");
            }
        }
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
        let _ = update(&mut state, Message::GlobalMuteToggle);
        assert!(!state.undo_stack.is_empty());
        let _ = update(&mut state, Message::Undo);
        assert!(!all_channels_muted(&state));
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
