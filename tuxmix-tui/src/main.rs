//! `tuxmix-tui` — Terminal-based RME interface controller.
//!
//! ```bash
//! cargo run -p tuxmix-tui              # with hardware
//! cargo run -p tuxmix-tui -- --mock    # simulation
//! cargo run -p tuxmix-tui -- --backend usb  # force libusb over the kernel driver
//! ```
//!
//! The one control surface no other RME mixer — official or third-party —
//! offers: a real terminal UI, so the machine actually running the audio
//! interface never needs a display attached to it.
//!
//! - **Broadcast / install machine room**: adjust levels over SSH from the
//!   control booth, no X forwarding, no VNC, no GPU required.
//! - **Fixed installations** (conference rooms, houses of worship,
//!   theaters): headless box, full control from any terminal on the network.
//! - **Live sound**: quick adjustments from a FOH laptop's terminal without
//!   waiting on a GUI to launch.
//! - **Scripting / CI**: `tmux send-keys`-drivable, so scene changes or
//!   level tweaks can be scripted the same way this app's own headless
//!   tests drive the GUI.

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::{Duration, Instant};
#[cfg(feature = "alsa")]
use tuxmix_core::BabyfacePro;
use tuxmix_core::channel::EqBandType;
use tuxmix_core::{BabyfaceProUsb, ChannelId, MockBabyfacePro, RmeDevice};

enum DeviceHandle {
    #[cfg(feature = "alsa")]
    Real(BabyfacePro),
    Mock(MockBabyfacePro),
    /// The proprietary USB backend (the TotalMix protocol) — the path
    /// that actually works with the device in proprietary mode on Linux.
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
    fn capture_scene(&self) -> tuxmix_core::Scene {
        delegate!(self, capture_scene)
    }
    fn apply_scene(&mut self, s: &tuxmix_core::Scene) -> Result<(), tuxmix_core::Error> {
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
    fn open_real(backend: Option<&str>) -> Option<Self> {
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
    fn open_mock() -> Self {
        DeviceHandle::Mock(MockBabyfacePro::open().expect("mock opens"))
    }
    /// All input meter levels in one call (the USB backend's
    /// `meters()` is draining — call once per draw, not per channel).
    fn input_meters(&self) -> Vec<f32> {
        let n = self.inputs().len();
        match self {
            DeviceHandle::Mock(d) => (0..n).map(|i| d.input_meter(i)).collect(),
            DeviceHandle::Usb(d) => d.meters().unwrap_or_else(|| vec![0.0; n]),
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => vec![0.0; n],
        }
    }
    fn playback_meters(&self) -> Vec<f32> {
        let n = self.playbacks().len();
        match self {
            DeviceHandle::Mock(d) => (0..n).map(|i| d.playback_meter(i)).collect(),
            // Playback meters come from the OUT stream — not wired yet.
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => vec![0.0; n],
            DeviceHandle::Usb(_) => vec![0.0; n],
        }
    }
    fn input_meter(&self, idx: usize) -> f32 {
        self.input_meters().get(idx).copied().unwrap_or(0.0)
    }
    fn playback_meter(&self, idx: usize) -> f32 {
        self.playback_meters().get(idx).copied().unwrap_or(0.0)
    }
    /// Whether `input_meters()` is a real per-session reading — see
    /// `tuxmix-gui`'s identical method for the full reasoning (shared
    /// `PROTOCOL.md` conclusion: the device has no meter registers, only
    /// host-side computation from the ISO streams, which only the USB
    /// backend captures).
    fn has_input_meters(&self) -> bool {
        match self {
            DeviceHandle::Mock(_) | DeviceHandle::Usb(_) => true,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(_) => false,
        }
    }
    /// Whether `playback_meters()` is real — true only for Mock; see
    /// `tuxmix-gui`'s identical method (the USB backend's ISO OUT stream
    /// runs in meter-only/silence mode, and only one process can hold the
    /// device's single streaming session at a time).
    fn has_playback_meters(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
    /// Output meters, computed host-side like TotalMix: each output's
    /// level is the power sum of every routed source (inputs + playbacks)
    /// scaled by that source's fader into the output. See
    /// `power_sum_output_meters`'s own doc comment for the actual math —
    /// pulled out as a free function so it's testable with deterministic
    /// inputs (`tuxmix-gui`'s identical fix, ported here: this file has
    /// its own independent copy of the same `DeviceHandle`/meter logic,
    /// not a shared one, so the same bug needed fixing twice).
    fn output_meters(&self) -> Vec<f32> {
        power_sum_output_meters(
            &self.inputs().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            &self.input_meters(),
            &self.playbacks().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            &self.playback_meters(),
            self.outputs().len(),
        )
    }
    fn is_mock(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
    /// See `tuxmix-gui`'s identical method: `true` when outputs are laid
    /// out one channel per submix pair (the proprietary USB path) rather
    /// than two per pair (the ALSA/profile `build_outputs` layout) —
    /// needed to map an output-strip channel index back to the submix
    /// pair index `set_loopback` expects.
    fn outputs_one_per_pair(&self) -> bool {
        self.outputs().len() == self.output_pair_count()
    }
}

/// The actual math behind `DeviceHandle::output_meters` — see
/// `tuxmix-gui`'s identical function for the full reasoning (both
/// crates had their own independent copy of the same buggy logic:
/// `input_volumes`/`playback_volumes` are indexed by *output pair*
/// while the individual output *channel* count is 2 per pair, and the
/// original code conflated the two, silently reading the wrong pair
/// for odd channels and a permanent zero for every pair past the
/// volumes arrays' own length).
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

const OUT_LABELS: [&str; 6] = ["AN1/2", "PH3/4", "AS1/2", "A3/A4", "A5/A6", "A7/A8"];

/// One entry in a pair-grouped section's display list (Hardware Inputs,
/// Software Playbacks, Hardware Outputs) — a linked pair (shown as one
/// bus, TotalMix's default) or a single channel of a pair that's been
/// split (`RmeDevice::{input_pair_linked,playback_linked,output_linked}`).
/// Rendering and each section's navigation/key-handling iterate this
/// instead of raw channel indices, so the cursor and what's on screen
/// always agree regardless of how many pairs are currently split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairItem {
    Linked(usize),
    Single(usize),
}

/// Builds a section's display list from its pair count and link state —
/// shared by `input_items`/`playback_items`/`output_items` below.
fn pair_items(pair_count: usize, is_linked: impl Fn(usize) -> bool) -> Vec<PairItem> {
    let mut items = Vec::new();
    for pair in 0..pair_count {
        if is_linked(pair) {
            items.push(PairItem::Linked(pair));
        } else {
            items.push(PairItem::Single(pair * 2));
            items.push(PairItem::Single(pair * 2 + 1));
        }
    }
    items
}

fn input_items(dev: &DeviceHandle) -> Vec<PairItem> {
    pair_items(dev.inputs().len() / 2, |p| dev.input_pair_linked(p))
}

fn playback_items(dev: &DeviceHandle) -> Vec<PairItem> {
    pair_items(dev.playbacks().len() / 2, |p| dev.playback_linked(p))
}

fn output_items(dev: &DeviceHandle) -> Vec<PairItem> {
    pair_items(dev.output_pair_count(), |p| dev.output_linked(p))
}

/// The pair index an item belongs to, regardless of whether it's shown
/// linked or as one half of a split.
fn item_pair(item: PairItem) -> usize {
    match item {
        PairItem::Linked(p) => p,
        PairItem::Single(c) => c / 2,
    }
}

/// See `tuxmix-gui`'s identical helper — combines a linked pair's two
/// per-channel names ("AN1"/"AN2") into the historical combined bus
/// label ("AN1/2").
fn pair_bus_label(left: &str, right: &str) -> String {
    let right_num: String = right.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    if right_num.is_empty() {
        format!("{left}/{right}")
    } else {
        format!("{left}/{right_num}")
    }
}

/// See `tuxmix-gui`'s identical helpers — routes a write through the
/// relevant pair-link logic (`RmeDevice::set_input_volume`/
/// `set_playback_volume`/`set_output_volume`, and their `_mute`/`_solo`
/// counterparts) so a linked pair moves both channels together and a
/// split pair moves only the addressed side.
fn set_channel_volume(dev: &mut DeviceHandle, cid: ChannelId, out: usize, v: f32) {
    let _ = match cid {
        ChannelId::Input(ch) => dev.set_input_volume(ch / 2, ch % 2, out, v),
        ChannelId::Playback(ch) => dev.set_playback_volume(ch / 2, ch % 2, out, v),
        ChannelId::Output(ch) => dev.set_output_volume(ch / 2, ch % 2, v),
    };
}

fn set_channel_mute(dev: &mut DeviceHandle, cid: ChannelId, m: bool) {
    let _ = match cid {
        ChannelId::Input(ch) => dev.set_input_mute(ch / 2, ch % 2, m),
        ChannelId::Playback(ch) => dev.set_playback_mute(ch / 2, ch % 2, m),
        ChannelId::Output(ch) => dev.set_output_mute(ch / 2, ch % 2, m),
    };
}

fn set_channel_solo(dev: &mut DeviceHandle, cid: ChannelId, s: bool) {
    let _ = match cid {
        ChannelId::Input(ch) => dev.set_input_solo(ch / 2, ch % 2, s),
        ChannelId::Playback(ch) => dev.set_playback_solo(ch / 2, ch % 2, s),
        ChannelId::Output(ch) => dev.set_output_solo(ch / 2, ch % 2, s),
    };
}

/// Resolves an item-list cursor position to the `ChannelId` it points
/// at — a linked pair's `ChannelId` is its first/left channel (every
/// pair-aware `set_*` method above writes both sides regardless of
/// which one is named).
fn item_channel_id(items: &[PairItem], channel: usize, make: impl Fn(usize) -> ChannelId) -> ChannelId {
    match items.get(channel) {
        Some(PairItem::Linked(pair)) => make(pair * 2),
        Some(PairItem::Single(ch)) => make(*ch),
        None => make(channel),
    }
}

/// Maps the current section/channel cursor to the `ChannelId` it points
/// at — was five near-identical `match section { 0 => Input, 1 =>
/// Playback, _ => Output }` blocks inline in `run`'s key handler, one per
/// key binding. Every section resolves through its own pair-grouped item
/// list (`input_items`/`playback_items`/`output_items`), so the cursor
/// always agrees with what's on screen regardless of which pairs are
/// currently split.
fn selected_channel_id(dev: &DeviceHandle, section: usize, channel: usize) -> ChannelId {
    match section {
        0 => item_channel_id(&input_items(dev), channel, ChannelId::Input),
        1 => item_channel_id(&playback_items(dev), channel, ChannelId::Playback),
        _ => item_channel_id(&output_items(dev), channel, ChannelId::Output),
    }
}

/// The raw input-channel index the cursor currently points at — for the
/// controls that are inherently per-physical-channel (48V/PAD/gain/
/// sensitivity/EQ) rather than per-submix-output like volume/pan. When
/// the pair is linked this is the representative left channel, same as
/// `tuxmix-gui`'s identical convention (see `mixer_view`'s Input loop).
fn selected_input_idx(dev: &DeviceHandle, channel: usize) -> usize {
    match selected_channel_id(dev, 0, channel) {
        ChannelId::Input(i) => i,
        _ => channel,
    }
}

/// Matches `tuxmix-gui`'s `app::db_text` formatting so the same level
/// reads the same way in both interfaces.
fn db_text(vol: f32) -> String {
    if vol <= 0.0 {
        return "-infdB".into();
    }
    // Round to 0.1 dB and collapse ±0.0 so a fader at 0 dB reads
    // "0.0dB", not "-0.00dB" (f32 20·log10(v) can land slightly below 0).
    let db = (20.0 * vol.log10() * 10.0).round() / 10.0;
    if db == 0.0 {
        "0.0dB".into()
    } else {
        format!("{:.1}dB", db)
    }
}

/// Matches `tuxmix-gui`'s `strip::full_strip` pan readout formatting.
fn pan_text(pan: i8) -> String {
    match pan.cmp(&0) {
        std::cmp::Ordering::Less => format!("L{}", -pan),
        std::cmp::Ordering::Greater => format!("R{}", pan),
        std::cmp::Ordering::Equal => "C".to_string(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let mock = args.iter().any(|a| a == "--mock");

    // `None` = auto-detect (ALSA/kernel-driver first, USB fallback — see
    // `DeviceHandle::open_real`). An unrecognized value falls back to
    // auto rather than silently opening nothing.
    let backend = args
        .iter()
        .position(|a| a == "--backend")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| match v.as_str() {
            "alsa" | "usb" => Some(v.as_str()),
            other => {
                eprintln!("Unknown --backend value {other:?} (use alsa|usb), auto-detecting");
                None
            }
        });

    let mut device: DeviceHandle = if mock {
        DeviceHandle::open_mock()
    } else {
        DeviceHandle::open_real(backend).unwrap_or_else(|| {
            eprintln!("No device found. Use --mock.");
            DeviceHandle::open_mock()
        })
    };
    // Restore the shared auto-saved state (the same `auto.json` the GUI
    // uses) so the two UIs stay in sync — the device has no gain/volume
    // readback. Skip in mock mode (no hardware to write to).
    if !mock {
        if let Some(scene) = tuxmix_core::scene::load_auto_scene() {
            if let Err(e) = device.apply_scene(&scene) {
                eprintln!("auto scene load failed: {e:?}");
            }
        }
    }
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = run(&mut terminal, &mut device);
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    if let Err(e) = res {
        eprintln!("Error: {}", e);
    }
    Ok(())
}

fn run(term: &mut Terminal<CrosstermBackend<Stdout>>, dev: &mut DeviceHandle) -> io::Result<()> {
    let mut show_matrix = false;
    let mut section: usize = 0; // 0=inputs, 1=playbacks, 2=outputs // 0=inputs, 1=playbacks
    let mut channel: usize = 0;
    // The active SUBMIX output (TotalMix: clicking a hardware output
    // shows every input/playback's fader INTO that output). `o`/`O`
    // cycle it; the strip rows read `volumes[sel_out]`.
    let mut sel_out: usize = 0;
    // `e` on one of the 4 analog inputs opens the EQ editor for it —
    // `(input_idx, selected_row)`, see `render_eq`/`adjust_eq_field`.
    let mut eq_view: Option<(usize, usize)> = None;
    // Shared-state persistence (same `auto.json` as the GUI): debounced
    // auto-save in the loop + a final save on quit.
    let mut last_auto_save = Instant::now();
    let mut last_saved_json: Option<String> = None;
    loop {
        let _ = dev.poll_events();
        if last_auto_save.elapsed() >= Duration::from_secs(3) {
            last_auto_save = Instant::now();
            // The mock has no hardware and its state would pollute the
            // SHARED auto.json with a "(mock)" model (making the real
            // device reject it on load) — never persist it.
            if dev.is_mock() {
                last_saved_json = None;
                continue;
            }
            // GUI/TUI sync: if the OTHER UI wrote auto.json since our
            // last save, re-apply its state first so we don't clobber it
            // with our own (possibly stale) copy — then save ours.
            if let Some(their) =
                tuxmix_core::scene::auto_scene_written_by_other(last_saved_json.as_deref())
            {
                let _ = dev.apply_scene(&their);
            }
            let scene = dev.capture_scene();
            if let Ok(json) = scene.to_json() {
                if last_saved_json.as_deref() != Some(json.as_str()) {
                    if tuxmix_core::scene::save_auto_scene(&scene).is_ok() {
                        last_saved_json = Some(json);
                    }
                }
            }
        }
        // Meters are draining on the USB backend — fetch once per draw.
        let in_meters = dev.input_meters();
        let pb_meters = dev.playback_meters();
        let out_meters = dev.output_meters();
        term.draw(|f| {
            ui(
                f,
                dev,
                &in_meters,
                &pb_meters,
                &out_meters,
                show_matrix,
                section,
                channel,
                sel_out,
                eq_view,
            )
        })?;
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    if let Some((eq_idx, row_val)) = eq_view {
                        match k.code {
                            KeyCode::Esc => eq_view = None,
                            KeyCode::Up => {
                                eq_view = Some((eq_idx, row_val.saturating_sub(1)));
                            }
                            KeyCode::Down => {
                                eq_view = Some((eq_idx, (row_val + 1).min(EQ_ROW_COUNT - 1)));
                            }
                            KeyCode::Left => adjust_eq_field(dev, eq_idx, row_val, -1, false),
                            KeyCode::Right => adjust_eq_field(dev, eq_idx, row_val, 1, false),
                            KeyCode::PageDown => adjust_eq_field(dev, eq_idx, row_val, -1, true),
                            KeyCode::PageUp => adjust_eq_field(dev, eq_idx, row_val, 1, true),
                            _ => {}
                        }
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            // Final state save — the next UI (GUI or TUI)
                            // restores this from the shared auto.json.
                            // Sync first (see the 3 s auto-save above).
                            if let Some(their) = tuxmix_core::scene::auto_scene_written_by_other(
                                last_saved_json.as_deref(),
                            ) {
                                let _ = dev.apply_scene(&their);
                            }
                            let scene = dev.capture_scene();
                            let _ = tuxmix_core::scene::save_auto_scene(&scene);
                            break;
                        }
                        KeyCode::Tab => show_matrix = !show_matrix,
                        KeyCode::Left => {
                            if channel > 0 {
                                channel -= 1;
                            }
                        }
                        KeyCode::Right => {
                            let max = match section {
                                0 => input_items(dev).len(),
                                1 => playback_items(dev).len(),
                                _ => output_items(dev).len(),
                            };
                            if channel + 1 < max {
                                channel += 1;
                            }
                        }
                        KeyCode::Up => {
                            if section > 0 {
                                section -= 1;
                                channel = 0;
                            }
                        }
                        KeyCode::Down => {
                            let max_sec = 2;
                            if section < max_sec {
                                section += 1;
                                channel = 0;
                            }
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            let cid = selected_channel_id(dev, section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                // Step the fader by +1 dB (TotalMix-style)
                                // from -inf (0.0) to +6 dB (2.0), instead of
                                // a fixed linear 0.05 (huge at the bottom,
                                // tiny at the top).
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db + 1.0).min(6.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                set_channel_volume(dev, cid, out, v);
                            }
                        }
                        KeyCode::Char('-') => {
                            let cid = selected_channel_id(dev, section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db - 1.0).max(-65.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                set_channel_volume(dev, cid, out, v);
                            }
                        }
                        // Fine volume: +/- 0.1 dB (PgUp/PgDn — Shift+'+' is
                        // ambiguous on AZERTY where '+' already needs Shift).
                        KeyCode::PageUp => {
                            let cid = selected_channel_id(dev, section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db + 0.1).min(6.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                set_channel_volume(dev, cid, out, v);
                            }
                        }
                        KeyCode::PageDown => {
                            let cid = selected_channel_id(dev, section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db - 0.1).max(-65.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                set_channel_volume(dev, cid, out, v);
                            }
                        }
                        KeyCode::Char('m') => {
                            let cid = selected_channel_id(dev, section, channel);
                            if let Ok(m) = dev.mute(cid) {
                                set_channel_mute(dev, cid, !m);
                            }
                        }
                        KeyCode::Char('s') => {
                            let cid = selected_channel_id(dev, section, channel);
                            if let Ok(s) = dev.solo(cid) {
                                set_channel_solo(dev, cid, !s);
                            }
                        }
                        // Lowercase 48V / uppercase (Shift+p) PAD — both are
                        // input-only, gated on Mic type the same way
                        // `tuxmix-gui`'s strip only shows these two buttons
                        // for mic channels (see `strip::full_strip`).
                        KeyCode::Char('p') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    let new_state = !ic.phantom;
                                    let _ = dev.set_phantom(idx, new_state);
                                }
                            }
                        }
                        KeyCode::Char('P') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    let new_state = !ic.pad;
                                    let _ = dev.set_pad(idx, new_state);
                                }
                            }
                        }
                        // Preamp gain in dB (0-65), stepped 1 dB at a time
                        // like TotalMix. Only Mic/Instrument inputs have a
                        // gain control. `]`/`[` (QWERTY) + `g`/`d`
                        // (AZERTY-friendly alternates).
                        KeyCode::Char(']') | KeyCode::Char('g') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    if let Some(max) = ic.gain_max {
                                        let new_gain = (ic.gain.unwrap_or(0) + 1).min(max);
                                        let _ = dev.set_gain(idx, new_gain);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('[') | KeyCode::Char('d') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    if ic.gain_max.is_some() {
                                        let new_gain = ic.gain.unwrap_or(0).saturating_sub(1);
                                        let _ = dev.set_gain(idx, new_gain);
                                    }
                                }
                            }
                        }
                        // Input sensitivity (+4dBu / -10dBV), Instrument
                        // inputs only.
                        KeyCode::Char('v') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    if let Some(current) = ic.sensitivity {
                                        let next = match current {
                                            tuxmix_core::Sensitivity::Plus4dBu => {
                                                tuxmix_core::Sensitivity::Minus10dBV
                                            }
                                            tuxmix_core::Sensitivity::Minus10dBV => {
                                                tuxmix_core::Sensitivity::Plus4dBu
                                            }
                                        };
                                        let _ = dev.set_sensitivity(idx, next);
                                    }
                                }
                            }
                        }
                        // Pan, Input/Playback only — matches tuxmix-gui's
                        // step-less drag range in spirit but as a fixed
                        // nudge, same pattern as +/-  for volume.
                        KeyCode::Char(',') => {
                            if section == 0 || section == 1 {
                                let cid = selected_channel_id(dev, section, channel);
                                if let Ok(p) = dev.pan(cid, sel_out) {
                                    let _ = dev.set_pan(cid, sel_out, (p - 5).max(-100));
                                }
                            }
                        }
                        KeyCode::Char('.') => {
                            if section == 0 || section == 1 {
                                let cid = selected_channel_id(dev, section, channel);
                                if let Ok(p) = dev.pan(cid, sel_out) {
                                    let _ = dev.set_pan(cid, sel_out, (p + 5).min(100));
                                }
                            }
                        }
                        // Active SUBMIX output cycle (TotalMix: each hardware
                        // output has its own matrix — the input/playback
                        // rows show the faders INTO this output).
                        KeyCode::Char('o') => {
                            sel_out = (sel_out + 1) % dev.output_pair_count();
                        }
                        KeyCode::Char('O') => {
                            sel_out =
                                (sel_out + dev.output_pair_count() - 1) % dev.output_pair_count();
                        }
                        // Pitch/varispeed (global clock, -5..+5%). `y`/`h`
                        // (vertically aligned on AZERTY too) = 0.1% steps
                        // like Fireface USB Settings, `Y`/`H` = 1% steps.
                        // Not persisted: the next stream re-init starts at 0%.
                        KeyCode::Char('y') => {
                            let p = dev.settings().pitch_percent;
                            let _ = dev.set_pitch((p + 0.1).min(5.0));
                        }
                        KeyCode::Char('h') => {
                            let p = dev.settings().pitch_percent;
                            let _ = dev.set_pitch((p - 0.1).max(-5.0));
                        }
                        KeyCode::Char('Y') => {
                            let p = dev.settings().pitch_percent;
                            let _ = dev.set_pitch((p + 1.0).min(5.0));
                        }
                        KeyCode::Char('H') => {
                            let p = dev.settings().pitch_percent;
                            let _ = dev.set_pitch((p - 1.0).max(-5.0));
                        }
                        // Stereo width (global, -1..+1). `u`/`j` (vertically
                        // adjacent on QWERTY, same up/down convention as
                        // pitch's `y`/`h`) — one step size is enough given
                        // the much narrower range than pitch's.
                        KeyCode::Char('u') => {
                            let w = dev.settings().width;
                            let _ = dev.set_width((w + 0.05).min(1.0));
                        }
                        KeyCode::Char('j') => {
                            let w = dev.settings().width;
                            let _ = dev.set_width((w - 0.05).max(-1.0));
                        }
                        // Global device toggles — mirrors tuxmix-gui's
                        // device_panel (MS Proc / AN 1>2 / Input Link).
                        KeyCode::Char('x') => {
                            let on = dev.settings().ms_proc;
                            let _ = dev.set_ms_proc(!on);
                        }
                        KeyCode::Char('a') => {
                            let on = dev.settings().an12;
                            let _ = dev.set_an12(!on);
                        }
                        KeyCode::Char('k') => {
                            let on = dev.settings().input_link;
                            let _ = dev.set_input_link(!on);
                        }
                        // EQ editor — analog inputs (AN1-4) only, mirrors
                        // tuxmix-gui's EQ flyout. See `render_eq`/
                        // `adjust_eq_field`.
                        KeyCode::Char('e') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if idx < 4 {
                                    eq_view = Some((idx, 0));
                                }
                            }
                        }
                        // Loopback — Output section only, mirrors
                        // tuxmix-gui's per-output-strip LOOP button.
                        KeyCode::Char('l') => {
                            if section == 2 {
                                if let Some(item) = output_items(dev).get(channel).copied() {
                                    let (pair, ch) = match item {
                                        PairItem::Linked(p) => (p, p * 2),
                                        PairItem::Single(c) => (c / 2, c),
                                    };
                                    if let Some(oc) = dev.outputs().get(ch) {
                                        let new_state = !oc.loopback;
                                        let _ = dev.set_loopback(pair, new_state);
                                    }
                                }
                            }
                        }
                        // Stereo link toggle — every section, mirrors
                        // tuxmix-gui's per-pair STEREO button (moved into
                        // the pair's Settings flyout there; the TUI has no
                        // such panel, so it's just a direct key here).
                        KeyCode::Char('t') => {
                            let items = match section {
                                0 => input_items(dev),
                                1 => playback_items(dev),
                                _ => output_items(dev),
                            };
                            if let Some(item) = items.get(channel).copied() {
                                let pair = item_pair(item);
                                let linked = match section {
                                    0 => dev.input_pair_linked(pair),
                                    1 => dev.playback_linked(pair),
                                    _ => dev.output_linked(pair),
                                };
                                let _ = match section {
                                    0 => dev.set_input_pair_linked(pair, !linked),
                                    1 => dev.set_playback_linked(pair, !linked),
                                    _ => dev.set_output_linked(pair, !linked),
                                };
                                // The item list just changed shape — keep
                                // the cursor in bounds (splitting adds an
                                // item, re-linking removes one).
                                let max = match section {
                                    0 => input_items(dev).len(),
                                    1 => playback_items(dev).len(),
                                    _ => output_items(dev).len(),
                                };
                                if channel >= max {
                                    channel = max.saturating_sub(1);
                                }
                            }
                        }
                        // Sample rate cycle: r = next in 44.1/48/96/192 kHz
                        // (the supported rate classes — alt 1/2/3). Only the
                        // proprietary backend actually switches (SET_INTERFACE
                        // + stream restart); other backends no-op.
                        KeyCode::Char('r') => {
                            let cur = dev.settings().sample_rate;
                            let next = match cur {
                                44100 => 48000,
                                48000 => 96000,
                                96000 => 192000,
                                _ => 44100,
                            };
                            let _ = dev.set_sample_rate(next);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn ui(
    f: &mut Frame,
    dev: &DeviceHandle,
    in_meters: &[f32],
    pb_meters: &[f32],
    out_meters: &[f32],
    show_matrix: bool,
    sel_sec: usize,
    sel_chan: usize,
    sel_out: usize,
    // `Some((input_idx, selected_row))` when the EQ editor is open (`e` on
    // one of the 4 analog inputs) — takes over the whole content area,
    // same as the Matrix view does.
    eq_view: Option<(usize, usize)>,
) {
    let area = f.area();
    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(area);
    let bottom = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(Rect::new(
            area.left(),
            top[1].bottom(),
            area.width,
            area.bottom() - top[1].bottom(),
        ));
    let content = bottom[0];
    let footer_area = bottom[1];
    let (inputs_area, playbacks_area, outputs_area, matrix_area) = if show_matrix {
        (Rect::default(), Rect::default(), Rect::default(), content)
    } else {
        let c = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Min(1), Constraint::Min(1)])
            .split(content);
        (c[0], c[1], c[2], Rect::default())
    };
    let view_tag = if eq_view.is_some() {
        " [EQ]".yellow().bold().to_string()
    } else if show_matrix {
        " [Matrix]".yellow().bold().to_string()
    } else {
        String::new()
    };
    let mode = if dev.is_mock() {
        " [SIMULATED]".yellow().bold()
    } else {
        "".into()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("TuxMix", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" - {}  ", dev.model_name())),
            mode,
            Span::raw(format!("{}", view_tag)),
            Span::raw(
                "  q:quit Tab:toggle e:EQ(AN1-4) o:submix y/h:pitch r:rate u/j:width x:ms a:an1>2 k:link",
            ),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        top[0],
    );

    let s = format!(
        "HW Inputs: {}  |  SW Playbacks: {}  |  Submix: {}  |  Rate: {} kHz  |  Clock: {}  |  Pitch: {:+0.1}%  |  Width: {:+0.2}  |  MS:{}  AN1>2:{}  Link:{}",
        dev.inputs().len(),
        dev.playbacks().len(),
        OUT_LABELS[sel_out],
        dev.settings().sample_rate / 1000,
        dev.settings().clock_source,
        dev.settings().pitch_percent,
        dev.settings().width,
        if dev.settings().ms_proc { "on" } else { "off" },
        if dev.settings().an12 { "on" } else { "off" },
        if dev.settings().input_link { "on" } else { "off" },
    );
    f.render_widget(
        Paragraph::new(s).block(Block::default().borders(Borders::ALL).title("Overview")),
        top[1],
    );

    if let Some((idx, eq_row)) = eq_view {
        render_eq(f, content, dev, idx, eq_row);
    } else if show_matrix {
        render_matrix(f, "Matrix Mixer", matrix_area, dev);
    } else {
        let has_in_meters = dev.has_input_meters();
        let has_pb_meters = dev.has_playback_meters();
        // All three sections grouped by pair, TotalMix-style: a linked
        // pair (the default) shows as ONE entry driving both channels; a
        // split pair shows its two channels separately — see
        // `RmeDevice::{input_pair_linked,playback_linked,output_linked}`.
        let in_items = input_items(dev);
        render_strips(
            f,
            "Hardware Inputs",
            inputs_area,
            in_items.len(),
            sel_sec == 0,
            sel_chan,
            |i| {
                let (ch_idx, name) = match in_items[i] {
                    PairItem::Linked(p) => {
                        let l = &dev.inputs()[p * 2];
                        let name = dev
                            .inputs()
                            .get(p * 2 + 1)
                            .map(|r| pair_bus_label(&l.name, &r.name))
                            .unwrap_or_else(|| l.name.clone());
                        (p * 2, name)
                    }
                    PairItem::Single(c) => (c, dev.inputs()[c].name.clone()),
                };
                let ch = &dev.inputs()[ch_idx];
                let m = in_meters.get(ch_idx).copied().unwrap_or(0.0);
                let mut label =
                    format!("{} [{:?}] {}", name, ch.channel_type, db_text(ch.volumes[sel_out]));
                if ch.mute {
                    label.push_str(" [M]");
                }
                if ch.solo {
                    label.push_str(" [S]");
                }
                if ch.phantom {
                    label.push_str(" 48V");
                }
                if ch.pad {
                    label.push_str(" PAD");
                }
                if let Some(gain) = ch.gain {
                    // Gain is tracked in dB (0-65) like TotalMix.
                    label.push_str(&format!(" {:.0}dB", gain));
                }
                if let Some(sens) = ch.sensitivity {
                    label.push_str(match sens {
                        tuxmix_core::Sensitivity::Plus4dBu => " +4dBu",
                        tuxmix_core::Sensitivity::Minus10dBV => " -10dBV",
                    });
                }
                label.push_str(&format!(" {}", pan_text(ch.pans[sel_out])));
                (label, m, has_in_meters)
            },
        );
        let pb_items = playback_items(dev);
        render_strips(
            f,
            "Software Playbacks",
            playbacks_area,
            pb_items.len(),
            sel_sec == 1,
            sel_chan,
            |i| {
                let (ch_idx, name) = match pb_items[i] {
                    PairItem::Linked(p) => {
                        let l = &dev.playbacks()[p * 2];
                        let name = dev
                            .playbacks()
                            .get(p * 2 + 1)
                            .map(|r| pair_bus_label(&l.name, &r.name))
                            .unwrap_or_else(|| l.name.clone());
                        (p * 2, name)
                    }
                    PairItem::Single(c) => (c, dev.playbacks()[c].name.clone()),
                };
                let ch = &dev.playbacks()[ch_idx];
                let m = pb_meters.get(ch_idx).copied().unwrap_or(0.0);
                let mut label = format!("{} {}", name, db_text(ch.volumes[sel_out]));
                if ch.mute {
                    label.push_str(" [M]");
                }
                if ch.solo {
                    label.push_str(" [S]");
                }
                label.push_str(&format!(" {}", pan_text(ch.pans[sel_out])));
                (label, m, has_pb_meters)
            },
        );
        let out_items = output_items(dev);
        render_strips(
            f,
            "Hardware Outputs",
            outputs_area,
            out_items.len(),
            sel_sec == 2,
            sel_chan,
            |i| {
                let (ch_idx, name) = match out_items[i] {
                    PairItem::Linked(p) => {
                        let l = &dev.outputs()[p * 2];
                        let name = dev
                            .outputs()
                            .get(p * 2 + 1)
                            .map(|r| pair_bus_label(&l.name, &r.name))
                            .unwrap_or_else(|| l.name.clone());
                        (p * 2, name)
                    }
                    PairItem::Single(c) => (c, dev.outputs()[c].name.clone()),
                };
                let ch = &dev.outputs()[ch_idx];
                let mut label = format!("{} {}", name, db_text(ch.volume));
                if ch.mute {
                    label.push_str(" [M]");
                }
                if ch.solo {
                    label.push_str(" [S]");
                }
                if ch.loopback {
                    label.push_str(" [LOOP]");
                }
                (
                    label,
                    out_meters.get(ch_idx).copied().unwrap_or(0.0),
                    has_in_meters || has_pb_meters,
                )
            },
        );
    }
    let footer: String = if eq_view.is_some() {
        "arrows:navigate/adjust  PgUp/PgDn:coarse  Esc:close".into()
    } else if show_matrix {
        "Tab: return to mixer".into()
    } else {
        format!(
            "IN:{}:{}  +/-:vol  PgUp/PgDn:0.1dB  ,/.:pan  m:mute  s:solo  p:48V  P:pad  [/] or g/d:gain  v:sens  l:loop(OUT)  t:stereo  arrows:navigate  q:quit",
            match sel_sec {
                0 => "IN",
                1 => "PB",
                _ => "OUT",
            },
            sel_chan
        )
    };
    f.render_widget(
        Paragraph::new(footer).block(Block::default().borders(Borders::TOP)),
        footer_area,
    );
}

fn render_strips(
    f: &mut Frame,
    title: &str,
    area: Rect,
    count: usize,
    is_focused: bool,
    selected: usize,
    label_fn: impl Fn(usize) -> (String, f32, bool),
) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let cols = count.min(6) as u16;
    let rows = ((count as u16) + cols - 1) / cols;
    let row_h = (inner.height / rows.max(1)).max(3);
    for i in 0..count {
        let (label, meter, meter_available) = label_fn(i);
        let col = i as u16 % cols;
        let row = i as u16 / cols;
        let w = inner.width / cols;
        let ch_area = Rect::new(
            inner.left() + col * w,
            inner.top() + row * row_h,
            w,
            row_h - 1,
        );
        let is_sel = is_focused && i == selected;
        let mut style = Style::default();
        if is_sel {
            style = style
                .bg(Color::Rgb(0x2a, 0x6a, 0x88))
                .add_modifier(Modifier::BOLD);
        }
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                if is_sel {
                    format!("> {} <", label)
                } else {
                    label
                },
                style,
            )])),
            ch_area,
        );
        // Always draw the meter track (TotalMix-style); label only when
        // there is signal, so empty strips still show their meter.
        let ma = Rect::new(
            ch_area.left(),
            ch_area.bottom().saturating_sub(2),
            ch_area.width.min(20),
            1,
        );
        // `meter_available` is a per-session backend capability, not a
        // per-frame reading (see `DeviceHandle::has_input_meters`/
        // `has_playback_meters`) — a flat 0% gauge would claim "silence"
        // where the real state is "not measured on this backend", so an
        // unavailable meter renders as a dim dashed track instead.
        let gauge = if meter_available {
            let c = if meter < 0.6 {
                Color::Green
            } else if meter < 0.85 {
                Color::Yellow
            } else {
                Color::Red
            };
            let mut gauge = Gauge::default()
                .gauge_style(Style::default().fg(c))
                .percent((meter * 100.0) as u16);
            if meter > 0.0 {
                gauge = gauge.label(format!("{:.0}%", meter * 100.0));
            }
            gauge
        } else {
            Gauge::default()
                .gauge_style(Style::default().fg(Color::DarkGray))
                .percent(0)
                .label("n/a")
        };
        f.render_widget(gauge, ma);
    }
}

fn render_matrix(f: &mut Frame, title: &str, area: Rect, dev: &DeviceHandle) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let ni = dev.inputs().len();
    let np = dev.playbacks().len();
    let total = ni + np;
    let mut lines = Vec::new();
    let mut header = "  ".to_string();
    for col in 0..total.min(8) {
        let name = if col < ni {
            &dev.inputs()[col].name
        } else {
            &dev.playbacks()[col - ni].name
        };
        header.push_str(&format!(" {:>6}", &name[..name.len().min(6)]));
    }
    lines.push(header);
    for row in 0..6 {
        let mut line = format!("  {:>8}", OUT_LABELS[row]);
        for col in 0..total.min(8) {
            let v = if col < ni {
                dev.inputs()[col].volumes[row]
            } else {
                dev.playbacks()[col - ni].volumes[row]
            };
            line.push_str(&format!(" {:>5.0}%", v * 100.0));
        }
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines.join("\n")), inner);
}

/// Number of selectable rows in the EQ editor: Enabled (1) + 3 bands ×
/// (Type/Freq/Q/Gain, 4 each) + Low Cut (Freq/Slope, 2) = 15. Row layout,
/// see `render_eq`/`adjust_eq_field`:
/// `0`=Enabled, `1..=4`=Band 1 (Type/Freq/Q/Gain), `5..=8`=Band 2,
/// `9..=12`=Band 3, `13`=Low Cut Freq, `14`=Low Cut Slope.
const EQ_ROW_COUNT: usize = 15;

fn eq_band_type_label(t: EqBandType) -> &'static str {
    match t {
        EqBandType::Off => "Off",
        EqBandType::Bell => "Bell",
        EqBandType::LowShelf => "Low Shelf",
        EqBandType::HighShelf => "High Shelf",
    }
}

/// The EQ editor for one of the 4 analog inputs (AN1-4) — takes over the
/// whole content area, same as the Matrix view. Mirrors `tuxmix-gui`'s
/// `eq_popover`: enable toggle, 3 parametric bands, low-cut filter. A
/// flat, single-column list of 15 rows (see `EQ_ROW_COUNT`) rather than a
/// 2D grid — simpler to navigate with just Up/Down than inventing a
/// row×field cursor model for a first implementation.
fn render_eq(f: &mut Frame, area: Rect, dev: &DeviceHandle, idx: usize, eq_row: usize) {
    let name = dev
        .inputs()
        .get(idx)
        .map(|c| c.name.as_str())
        .unwrap_or("?");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("EQ — {name}"));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let eq = dev
        .inputs()
        .get(idx)
        .and_then(|c| c.eq)
        .unwrap_or_default();

    let mut rows = vec![format!(
        "Enabled          : {}",
        if eq.enabled { "On" } else { "Off" }
    )];
    for (i, b) in eq.bands.iter().enumerate() {
        rows.push(format!(
            "Band {} Type      : {}",
            i + 1,
            eq_band_type_label(b.band_type)
        ));
        rows.push(format!("Band {} Freq      : {} Hz", i + 1, b.freq_hz));
        rows.push(format!("Band {} Q         : {:.2}", i + 1, b.q));
        rows.push(format!("Band {} Gain      : {:+.1} dB", i + 1, b.gain_db));
    }
    rows.push(format!("Low Cut Freq     : {} Hz", eq.low_cut_freq_hz));
    rows.push(format!(
        "Low Cut Slope    : {} dB/oct",
        eq.low_cut_slope_db_oct
    ));

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if i == eq_row {
                Line::from(Span::styled(
                    format!("> {r}"),
                    Style::default()
                        .bg(Color::Rgb(0x2a, 0x6a, 0x88))
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::raw(format!("  {r}")))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// Applies one Left/Right (`dir` = -1/+1, `coarse` = false) or
/// PageUp/PageDown (`coarse` = true) step to whichever EQ parameter
/// `row` (see `EQ_ROW_COUNT`'s doc comment) currently points at.
/// Band-type and low-cut-slope rows cycle through their fixed option
/// list instead of stepping a continuous range.
fn adjust_eq_field(dev: &mut DeviceHandle, idx: usize, row: usize, dir: i32, coarse: bool) {
    let eq = dev
        .inputs()
        .get(idx)
        .and_then(|c| c.eq)
        .unwrap_or_default();
    match row {
        0 => {
            let _ = dev.set_eq_enabled(idx, !eq.enabled);
        }
        1 | 5 | 9 => {
            let band = (row - 1) / 4;
            const ORDER: [EqBandType; 4] = [
                EqBandType::Off,
                EqBandType::Bell,
                EqBandType::LowShelf,
                EqBandType::HighShelf,
            ];
            let cur = ORDER
                .iter()
                .position(|t| *t == eq.bands[band].band_type)
                .unwrap_or(0) as i32;
            let next = (cur + dir).rem_euclid(ORDER.len() as i32) as usize;
            let _ = dev.set_eq_band_type(idx, band, ORDER[next]);
        }
        2 | 6 | 10 => {
            let band = (row - 2) / 4;
            let step = if coarse { 100 } else { 10 };
            let new = (eq.bands[band].freq_hz as i32 + dir * step).clamp(20, 20_000);
            let _ = dev.set_eq_band_freq(idx, band, new as u16);
        }
        3 | 7 | 11 => {
            let band = (row - 3) / 4;
            let step = if coarse { 0.5 } else { 0.05 };
            let new = (eq.bands[band].q + dir as f32 * step).clamp(0.05, 10.0);
            let _ = dev.set_eq_band_q(idx, band, new);
        }
        4 | 8 | 12 => {
            let band = (row - 4) / 4;
            let step = if coarse { 3.0 } else { 0.5 };
            let new = (eq.bands[band].gain_db + dir as f32 * step).clamp(-24.0, 24.0);
            let _ = dev.set_eq_band_gain(idx, band, new);
        }
        13 => {
            let step = if coarse { 100 } else { 10 };
            let new = (eq.low_cut_freq_hz as i32 + dir * step).clamp(20, 20_000);
            let _ = dev.set_eq_low_cut_freq(idx, new as u16);
        }
        14 => {
            const ORDER: [u8; 4] = [6, 12, 18, 24];
            let cur = ORDER
                .iter()
                .position(|s| *s == eq.low_cut_slope_db_oct)
                .unwrap_or(0) as i32;
            let next = (cur + dir).rem_euclid(ORDER.len() as i32) as usize;
            let _ = dev.set_eq_low_cut_slope(idx, ORDER[next]);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_meters_reads_the_right_pair_for_every_individual_channel() {
        // Same scenario as tuxmix-gui's identical test: 3 output pairs
        // (6 individual channels), one input routed at full volume into
        // pair 2 only, reading a fixed 1.0 meter level.
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
    fn db_text_matches_gui_formatting_at_key_points() {
        assert_eq!(db_text(0.0), "-infdB");
        assert_eq!(db_text(1.0), "0.0dB");
        // 0.5 linear ≈ -6.0 dB — same `20.0 * log10(v)` formula as
        // `tuxmix-gui::app::db_text`, just without the space/∞ glyph.
        assert_eq!(db_text(0.5), "-6.0dB");
    }

    #[test]
    fn selected_channel_id_maps_sections_correctly() {
        let dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        // All pairs linked by default in every section — item index 3
        // is the pair starting at physical channel 6.
        assert_eq!(selected_channel_id(&dev, 0, 3), ChannelId::Input(6));
        assert_eq!(selected_channel_id(&dev, 1, 2), ChannelId::Playback(4));
        // Item 5 (0-indexed) is the pair starting at channel 10 (ADAT7/8).
        assert_eq!(selected_channel_id(&dev, 2, 5), ChannelId::Output(10));
    }

    #[test]
    fn output_items_all_linked_by_default() {
        let dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        let items = output_items(&dev);
        assert_eq!(items.len(), dev.output_pair_count());
        assert_eq!(items[0], PairItem::Linked(0));
    }

    #[test]
    fn output_items_reflect_a_split_pair() {
        let mut dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        dev.set_output_linked(0, false).unwrap();
        let items = output_items(&dev);
        // One extra item: the split pair contributes 2 entries instead
        // of 1, everything else stays linked.
        assert_eq!(items.len(), dev.output_pair_count() + 1);
        assert_eq!(items[0], PairItem::Single(0));
        assert_eq!(items[1], PairItem::Single(1));
        assert_eq!(items[2], PairItem::Linked(1));
        // The cursor now resolves to the specific split channel, not
        // always the pair's left channel.
        assert_eq!(selected_channel_id(&dev, 2, 1), ChannelId::Output(1));
    }

    #[test]
    fn input_items_all_linked_by_default() {
        let dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        let items = input_items(&dev);
        assert_eq!(items.len(), dev.inputs().len() / 2);
        assert_eq!(items[0], PairItem::Linked(0));
    }

    #[test]
    fn input_items_reflect_a_split_pair_and_isolate_writes() {
        let mut dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        dev.set_input_pair_linked(0, false).unwrap();
        let items = input_items(&dev);
        assert_eq!(items.len(), dev.inputs().len() / 2 + 1);
        assert_eq!(items[0], PairItem::Single(0));
        assert_eq!(items[1], PairItem::Single(1));
        // Moving the second entry (physical channel 1) must not move
        // channel 0 — the whole point of splitting the pair.
        let cid = selected_channel_id(&dev, 0, 1);
        assert_eq!(cid, ChannelId::Input(1));
        let before = dev.volume(ChannelId::Input(0), 0).unwrap();
        set_channel_volume(&mut dev, cid, 0, 0.5);
        assert_eq!(dev.volume(ChannelId::Input(0), 0).unwrap(), before);
        assert_eq!(dev.volume(ChannelId::Input(1), 0).unwrap(), 0.5);
    }

    #[test]
    fn playback_items_all_linked_by_default() {
        let dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        let items = playback_items(&dev);
        assert_eq!(items.len(), dev.playbacks().len() / 2);
        assert_eq!(items[0], PairItem::Linked(0));
    }

    #[test]
    fn playback_items_reflect_a_split_pair_and_isolate_mute() {
        let mut dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        dev.set_playback_linked(0, false).unwrap();
        let items = playback_items(&dev);
        assert_eq!(items.len(), dev.playbacks().len() / 2 + 1);
        assert_eq!(items[0], PairItem::Single(0));
        assert_eq!(items[1], PairItem::Single(1));
        let cid = selected_channel_id(&dev, 1, 0);
        assert_eq!(cid, ChannelId::Playback(0));
        set_channel_mute(&mut dev, cid, true);
        assert!(dev.mute(ChannelId::Playback(0)).unwrap());
        assert!(!dev.mute(ChannelId::Playback(1)).unwrap());
    }

    #[test]
    fn pair_bus_label_combines_channel_names() {
        assert_eq!(pair_bus_label("AN1", "AN2"), "AN1/2");
        assert_eq!(pair_bus_label("ADAT7", "ADAT8"), "ADAT7/8");
    }
}
