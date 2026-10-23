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
    widgets::{Block, Borders, Paragraph, Widget},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::{Duration, Instant};
#[cfg(feature = "alsa")]
use tuxmix_core::BabyfacePro;
use tuxmix_core::channel::{ChannelType, EqBandType};
use tuxmix_core::pw_meter::PlaybackTap;
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
            DeviceHandle::Real(d) => d.meters().unwrap_or_else(|| vec![0.0; n]),
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
    /// Per-channel availability for the input strips: the ALSA backend
    /// has real levels for all twelve inputs in the device's own order
    /// with the v7 driver (AS1/2 is at capture 4/5). See `tuxmix-gui`'s
    /// identical method and `tuxmix-core`'s `capture_meter` module for
    /// the live mapping.
    fn has_input_meter(&self, idx: usize) -> bool {
        match self {
            DeviceHandle::Mock(_) => true,
            DeviceHandle::Usb(_) => idx < 4,
            #[cfg(feature = "alsa")]
            DeviceHandle::Real(d) => idx < d.inputs().len() && d.meters_available(),
        }
    }
    /// Whether `playback_meters()` is real — true only for Mock; see
    /// `tuxmix-gui`'s identical method (the USB backend's ISO OUT stream
    /// runs in meter-only/silence mode, and only one process can hold the
    /// device's single streaming session at a time).
    fn has_playback_meters(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
    /// Output meters, computed host-side like TotalMix from levels the
    /// caller already drained this draw. Passing them in matters:
    /// `input_meters()`/`playback_meters()` drain on the USB and ALSA
    /// backends, and the old `output_meters()` called them a second time,
    /// so the sum used a near-empty second drain. See
    /// `power_sum_output_meters`'s own doc comment for the math.
    fn output_meters_from(&self, input_meters: &[f32], playback_meters: &[f32]) -> Vec<f32> {
        power_sum_output_meters(
            &self.inputs().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            input_meters,
            &self.playbacks().iter().map(|c| c.volumes.clone()).collect::<Vec<_>>(),
            playback_meters,
            self.outputs().len(),
        )
    }
    fn is_mock(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
}

/// Whether an output's host-computed meter is complete. The TUI has no
/// PipeWire playback tap, so any nonzero playback crosspoint keeps the
/// output N/A; inputs are real per `has_input_meter`. See `tuxmix-gui`'s
/// `output_meter_available` for the GUI version, which does have
/// playback meters.
/// Whether playback channel `idx` has a real level. Mock is always
/// real; a real backend follows the PipeWire tap's channel count (2 =
/// PB1 only, 12 = all six pairs).
fn playback_meter_available(dev: &DeviceHandle, tap_channels: usize, idx: usize) -> bool {
    if dev.is_mock() {
        dev.has_playback_meters()
    } else {
        idx < tap_channels
    }
}

fn output_meter_available(dev: &DeviceHandle, tap_channels: usize, out_idx: usize) -> bool {
    let one_per_pair = dev.outputs().len() == dev.output_pair_count();
    let pair = if one_per_pair { out_idx } else { out_idx / 2 };
    let inputs_ok = dev.inputs().iter().enumerate().all(|(i, ch)| {
        ch.volumes.get(pair).copied().unwrap_or(0.0) == 0.0 || dev.has_input_meter(i)
    });
    let playbacks_ok = dev.playbacks().iter().enumerate().all(|(i, ch)| {
        ch.volumes.get(pair).copied().unwrap_or(0.0) == 0.0
            || playback_meter_available(dev, tap_channels, i)
    });
    inputs_ok && playbacks_ok
}

/// The actual math behind `DeviceHandle::output_meters_from` — see
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
    // Horizontal scroll offset (in individual output *channels*) for
    // the Matrix view — there are up to 12 output columns, more than
    // fit most terminal widths at once. Left/Right move it while the
    // Matrix view is showing (see the `show_matrix` guard below);
    // clamped properly at render time (`render_matrix`) once the
    // actual visible column count is known, so an increment here can't
    // scroll arbitrarily far past the real end.
    let mut matrix_col: usize = 0;
    // `e` on one of the 4 analog inputs opens the EQ editor for it —
    // `(input_idx, selected_row)`, see `render_eq`/`adjust_eq_field`.
    let mut eq_view: Option<(usize, usize)> = None;
    // Shared-state persistence (same `auto.json` as the GUI): debounced
    // auto-save in the loop + a final save on quit.
    let mut last_auto_save = Instant::now();
    let mut last_saved_json: Option<String> = None;
    // Mock has its own generated playback levels; a real backend gets
    // them from PipeWire's sink monitor (the same tap as the GUI).
    let playback_tap = if dev.is_mock() {
        PlaybackTap::disabled()
    } else {
        PlaybackTap::start()
    };
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
        let (tap_levels, tap_channels) = playback_tap.drain();
        let pb_meters = if dev.is_mock() {
            dev.playback_meters()
        } else {
            tap_levels.to_vec()
        };
        let out_meters = dev.output_meters_from(&in_meters, &pb_meters);
        term.draw(|f| {
            ui(
                f,
                dev,
                &in_meters,
                &pb_meters,
                tap_channels,
                &out_meters,
                show_matrix,
                section,
                channel,
                sel_out,
                matrix_col,
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
                        // Scroll the Matrix view's output columns
                        // instead of moving the (currently invisible,
                        // since Matrix hides the per-section views
                        // entirely) channel cursor — these guarded arms
                        // only match while Matrix is showing, so the
                        // plain `Left`/`Right` arms right below still
                        // handle every other view exactly as before.
                        KeyCode::Left if show_matrix => {
                            matrix_col = matrix_col.saturating_sub(1);
                        }
                        KeyCode::Right if show_matrix => {
                            matrix_col += 1;
                        }
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
                        KeyCode::Char('s') if section != 2 => {
                            // No solo on outputs: neither hardware backend
                            // has one (TotalMix neither), so it would only
                            // toggle a flag in the simulation.
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
                                    let _ = dev.set_input_phantom(idx, new_state);
                                }
                            }
                        }
                        KeyCode::Char('P') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if let Some(ic) = dev.inputs().get(idx) {
                                    let new_state = !ic.pad;
                                    let _ = dev.set_input_pad(idx, new_state);
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
                                        let _ = dev.set_input_gain(idx, new_gain);
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
                                        let _ = dev.set_input_gain(idx, new_gain);
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
                            if dev.protocol_features() {
                                let _ = dev.set_pitch((p + 0.1).min(5.0));
                            }
                        }
                        KeyCode::Char('h') => {
                            let p = dev.settings().pitch_percent;
                            if dev.protocol_features() {
                                let _ = dev.set_pitch((p - 0.1).max(-5.0));
                            }
                        }
                        KeyCode::Char('Y') => {
                            let p = dev.settings().pitch_percent;
                            if dev.protocol_features() {
                                let _ = dev.set_pitch((p + 1.0).min(5.0));
                            }
                        }
                        KeyCode::Char('H') => {
                            let p = dev.settings().pitch_percent;
                            if dev.protocol_features() {
                                let _ = dev.set_pitch((p - 1.0).max(-5.0));
                            }
                        }
                        // Stereo width (global, -1..+1). `u`/`j` (vertically
                        // adjacent on QWERTY, same up/down convention as
                        // pitch's `y`/`h`) — one step size is enough given
                        // the much narrower range than pitch's.
                        KeyCode::Char('u') => {
                            let w = dev.settings().width;
                            if dev.protocol_features() {
                                let _ = dev.set_width((w + 0.05).min(1.0));
                            }
                        }
                        KeyCode::Char('j') => {
                            let w = dev.settings().width;
                            if dev.protocol_features() {
                                let _ = dev.set_width((w - 0.05).max(-1.0));
                            }
                        }
                        // Global device toggles — mirrors tuxmix-gui's
                        // device_panel (MS Proc / AN 1>2 / Input Link).
                        KeyCode::Char('x') => {
                            let on = dev.settings().ms_proc;
                            if dev.protocol_features() {
                                let _ = dev.set_ms_proc(!on);
                            }
                        }
                        KeyCode::Char('a') => {
                            let on = dev.settings().an12;
                            if dev.protocol_features() {
                                let _ = dev.set_an12(!on);
                            }
                        }
                        KeyCode::Char('k') => {
                            let on = dev.settings().input_link;
                            if dev.protocol_features() {
                                let _ = dev.set_input_link(!on);
                            }
                        }
                        KeyCode::Char('f') => {
                            let on = dev.settings().eq_for_record;
                            if dev.protocol_features() {
                                let _ = dev.set_eq_for_record(!on);
                            }
                        }
                        KeyCode::Char('w') => {
                            let on = dev.settings().optical_out_spdif;
                            let _ = dev.set_optical_out_format(!on);
                        }
                        // EQ editor — analog inputs (AN1-4) only, mirrors
                        // tuxmix-gui's EQ flyout. See `render_eq`/
                        // `adjust_eq_field`.
                        KeyCode::Char('e') => {
                            if section == 0 && dev.protocol_features() {
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
                                        if dev.protocol_features() {
                                            let _ = dev.set_loopback(pair, new_state);
                                        }
                                    }
                                }
                            }
                        }
                        // CUE — Output section only, mirrors tuxmix-gui's
                        // per-output-strip "C" button (exclusively monitor
                        // this output's own playback feed through AN1/2).
                        KeyCode::Char('c') => {
                            if section == 2 {
                                if let Some(item) = output_items(dev).get(channel).copied() {
                                    let (pair, ch) = match item {
                                        PairItem::Linked(p) => (p, p * 2),
                                        PairItem::Single(c) => (c / 2, c),
                                    };
                                    if let Some(oc) = dev.outputs().get(ch) {
                                        let new_state = !oc.cue;
                                        let _ = dev.set_cue(pair, new_state);
                                    }
                                }
                            }
                        }
                        // Phase Ø invert — Input section, the 4 analog
                        // inputs only, mirrors tuxmix-gui's "Ø" icon-col
                        // button.
                        KeyCode::Char('i') => {
                            if section == 0 {
                                let idx = selected_input_idx(dev, channel);
                                if idx < 4 {
                                    let on = dev.inputs()[idx].phase;
                                    if dev.protocol_features() {
                                        let _ = dev.set_phase(idx, !on);
                                    }
                                }
                            }
                        }
                        // Stereo split — Playback section only, mirrors
                        // tuxmix-gui's "SP" icon-col button. Distinct from
                        // the stereo LINK toggle ('t') below — this is a
                        // real hardware register (hard-pan into the AN1/2
                        // monitor bus), not a "one strip or two" UI concept.
                        KeyCode::Char('n') => {
                            if section == 1 {
                                if let Some(item) = playback_items(dev).get(channel).copied() {
                                    let ch = match item {
                                        PairItem::Linked(p) => p * 2,
                                        PairItem::Single(c) => c,
                                    };
                                    if let Some(pc) = dev.playbacks().get(ch) {
                                        let on = pc.split;
                                        if dev.protocol_features() {
                                            let _ = dev.set_stereo_split(ch, !on);
                                        }
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

/// The top bar's key list. On the class-compliant personality the keys
/// that drive RME's own protocol are dead (their setters refuse, see
/// `RmeDevice::protocol_features`), so the bar stops advertising them,
/// the same rule the bottom footer already follows. `w` (optical out
/// format) and `c` (cue) leave with them: the ALSA backend implements
/// neither, and a class-compliant card is only ever reached over ALSA.
fn top_hints(protocol_features: bool) -> &'static str {
    if protocol_features {
        "  o:submix  r:rate  y/h:pitch  u/j:width  x:MS  a:AN1>2  k:input link  f:EQ rec  w:SPDIF out"
    } else {
        "  o:submix  r:rate"
    }
}

fn ui(
    f: &mut Frame,
    dev: &DeviceHandle,
    in_meters: &[f32],
    pb_meters: &[f32],
    pb_channels: usize,
    out_meters: &[f32],
    show_matrix: bool,
    sel_sec: usize,
    sel_chan: usize,
    sel_out: usize,
    matrix_col: usize,
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
            Span::raw(top_hints(dev.protocol_features())),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        top[0],
    );

    let s = format!(
        "HW Inputs: {}  |  SW Playbacks: {}  |  Submix: {}  |  Rate: {} kHz  |  Clock: {}  |  Pitch: {:+0.1}%  |  Width: {:+0.2}  |  MS:{}  AN1>2:{}  Link:{}  EQRec:{}  SPDIFOut:{}",
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
        if dev.settings().eq_for_record { "on" } else { "off" },
        if dev.settings().optical_out_spdif { "on" } else { "off" },
    );
    f.render_widget(
        Paragraph::new(s).block(Block::default().borders(Borders::ALL).title("Overview")),
        top[1],
    );

    if let Some((idx, eq_row)) = eq_view {
        render_eq(f, content, dev, idx, eq_row);
    } else if show_matrix {
        render_matrix(f, "Matrix Mixer", matrix_area, dev, matrix_col);
    } else {
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
                let mut badges = Vec::new();
                if ch.mute {
                    badges.push(("M".to_string(), MUTE_COLOR));
                }
                if ch.solo {
                    badges.push(("S".to_string(), SOLO_COLOR));
                }
                if ch.phantom {
                    badges.push(("48V".to_string(), BADGE_COLOR));
                }
                if ch.pad {
                    badges.push(("PAD".to_string(), BADGE_COLOR));
                }
                if ch.phase {
                    badges.push(("\u{00D8}".to_string(), BADGE_COLOR));
                }
                if let Some(gain) = ch.gain {
                    // Gain is tracked in dB (0-65) like TotalMix.
                    badges.push((format!("{gain:.0}dB"), Color::Gray));
                }
                if let Some(sens) = ch.sensitivity {
                    badges.push((
                        match sens {
                            tuxmix_core::Sensitivity::Plus4dBu => "+4dBu",
                            tuxmix_core::Sensitivity::Minus10dBV => "-10dBV",
                        }
                        .to_string(),
                        Color::Gray,
                    ));
                }
                StripView {
                    name,
                    tag: Some(format!("{:?}", ch.channel_type)),
                    vol: ch.volumes[sel_out],
                    level: db_text(ch.volumes[sel_out]),
                    pan: Some(pan_text(ch.pans[sel_out])),
                    badges,
                    meter: in_meters.get(ch_idx).copied().unwrap_or(0.0),
                    meter_available: dev.has_input_meter(ch_idx),
                }
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
                let mut badges = Vec::new();
                if ch.mute {
                    badges.push(("M".to_string(), MUTE_COLOR));
                }
                if ch.solo {
                    badges.push(("S".to_string(), SOLO_COLOR));
                }
                if ch.split {
                    badges.push(("SPLIT".to_string(), BADGE_COLOR));
                }
                StripView {
                    name,
                    tag: Some("PB".to_string()),
                    vol: ch.volumes[sel_out],
                    level: db_text(ch.volumes[sel_out]),
                    pan: Some(pan_text(ch.pans[sel_out])),
                    badges,
                    meter: pb_meters.get(ch_idx).copied().unwrap_or(0.0),
                    meter_available: playback_meter_available(dev, pb_channels, ch_idx),
                }
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
                // No solo badge: neither hardware backend has an output
                // solo (see the GUI notes), so one would only ever light in
                // the simulation.
                let mut badges = Vec::new();
                if ch.mute {
                    badges.push(("M".to_string(), MUTE_COLOR));
                }
                if ch.loopback {
                    badges.push(("LOOP".to_string(), BADGE_COLOR));
                }
                if ch.cue {
                    badges.push(("CUE".to_string(), BADGE_COLOR));
                }
                StripView {
                    name,
                    tag: Some("OUT".to_string()),
                    vol: ch.volume,
                    level: db_text(ch.volume),
                    pan: None,
                    badges,
                    meter: out_meters.get(ch_idx).copied().unwrap_or(0.0),
                    meter_available: output_meter_available(dev, pb_channels, ch_idx),
                }
            },
        );
    }
    let footer: String = if eq_view.is_some() {
        "arrows:navigate/adjust  PgUp/PgDn:coarse  Esc:close".into()
    } else if show_matrix {
        "Tab: return to mixer".into()
    } else {
        // Only the keys of the focused section, and of what the connection
        // can do: the keys that drive RME's own protocol are listed only
        // when they work (the class-compliant personality says what is
        // missing instead of advertising it).
        let proto = dev.protocol_features();
        let (section_name, keys) = match sel_sec {
            0 => (
                "Input",
                if proto {
                    "m:mute s:solo ,/.:pan p:48V P:pad [/]:gain v:sens i:phase e:eq t:stereo"
                } else {
                    "m:mute s:solo ,/.:pan p:48V P:pad [/]:gain v:sens t:stereo"
                },
            ),
            1 => (
                "Playback",
                if proto {
                    "m:mute s:solo ,/.:pan n:split t:stereo"
                } else {
                    "m:mute s:solo ,/.:pan t:stereo"
                },
            ),
            _ => (
                "Output",
                if proto {
                    "m:mute l:loop c:cue t:stereo"
                } else {
                    "m:mute c:cue t:stereo"
                },
            ),
        };
        format!(
            "{section_name} {sel_chan}  \u{2502}  +/-:level PgUp/PgDn:0.1dB  {keys}  \u{2502}  arrows:move Tab:matrix q:quit{}",
            if proto {
                ""
            } else {
                "  \u{2502}  class compliant: no RME-protocol features"
            },
        )
    };
    f.render_widget(
        Paragraph::new(footer).block(Block::default().borders(Borders::TOP)),
        footer_area,
    );
}

/// What one strip cell shows. Built per channel by the section's closure,
/// drawn by `render_strips`.
struct StripView {
    name: String,
    /// Channel-type tag next to the name ("Mic", "ADAT"...), if any.
    tag: Option<String>,
    /// Fader position as linear volume (0..2), drawn on the vertical fader.
    vol: f32,
    level: String,
    pan: Option<String>,
    /// Badges under the level: (label, color) when lit.
    badges: Vec<(String, Color)>,
    meter: f32,
    meter_available: bool,
}

const MUTE_COLOR: Color = Color::Rgb(0x3d, 0x9d, 0xf6);
const SOLO_COLOR: Color = Color::Rgb(0xf0, 0xb4, 0x29);
const BADGE_COLOR: Color = Color::Rgb(0xf2, 0xf3, 0xf5);
const SEL_BG: Color = Color::Rgb(0x26, 0x27, 0x2b);

/// A linear amplitude (1.0 = 0 dB) as a 0..1 position on the console's
/// tapered scale, the same law the GUI's faders and meters use (-60..+6 dB,
/// exponent 0.6), so a level sits at the same height in both interfaces.
fn taper(amp: f32) -> f32 {
    if amp <= 0.0 {
        return 0.0;
    }
    let db = (20.0 * amp.log10()).clamp(-60.0, 6.0);
    let x = ((db - 6.0) / (-60.0 - 6.0)).clamp(0.0, 1.0);
    (1.0 - x.powf(0.6)).clamp(0.0, 1.0)
}

/// Cuts `text` to `width` columns, ending with an ellipsis when it was cut.
fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('\u{2026}');
    out
}

fn render_strips(
    f: &mut Frame,
    title: &str,
    area: Rect,
    count: usize,
    is_focused: bool,
    selected: usize,
    view_fn: impl Fn(usize) -> StripView,
) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if count == 0 || inner.width == 0 || inner.height < 3 {
        return;
    }
    let cols = count.min(6) as u16;
    let rows = ((count as u16) + cols - 1) / cols;
    let row_h = (inner.height / rows.max(1)).max(3);
    for i in 0..count {
        let v = view_fn(i);
        let col = i as u16 % cols;
        let row = i as u16 / cols;
        let w = inner.width / cols;
        let cell = Rect::new(
            inner.left() + col * w,
            inner.top() + row * row_h,
            w.saturating_sub(1),
            row_h.saturating_sub(1),
        );
        if cell.width < 8 || cell.height < 2 {
            continue;
        }
        let is_sel = is_focused && i == selected;
        let base = if is_sel {
            Style::default().bg(SEL_BG)
        } else {
            Style::default()
        };
        f.render_widget(Block::default().style(base), cell);

        // Name line: bold, the type tag dimmed, cut to the cell.
        let tag_w = v.tag.as_ref().map_or(0, |t| t.chars().count() + 1);
        let name = fit(&v.name, (cell.width as usize).saturating_sub(tag_w + 2));
        let mut spans = vec![
            Span::styled(
                if is_sel { "\u{25B8} " } else { "  " },
                base.fg(BADGE_COLOR),
            ),
            Span::styled(name, base.add_modifier(Modifier::BOLD)),
        ];
        if let Some(tag) = &v.tag {
            spans.push(Span::styled(
                format!(" {tag}"),
                base.fg(Color::DarkGray),
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(cell.x, cell.y, cell.width, 1),
        );

        // Vertical fader and meter side by side, left of the readouts.
        let bar_top = cell.y + 1;
        let bar_h = cell.height.saturating_sub(1);
        if bar_h == 0 {
            continue;
        }
        let fader_x = cell.x + 2;
        let meter_x = cell.x + 4;
        let fader_fill = (taper(v.vol) * bar_h as f32).round() as u16;
        let meter_fill = (taper(v.meter) * bar_h as f32).round() as u16;
        let buf = f.buffer_mut();
        for k in 0..bar_h {
            let y = bar_top + bar_h - 1 - k; // k = 0 is the bottom cell
            // Fader: a thin track, filled up to the cap.
            let on = k < fader_fill;
            buf.set_string(
                fader_x,
                y,
                if k + 1 == fader_fill { "\u{2501}" } else if on { "\u{2503}" } else { "\u{2502}" },
                base.fg(if on { BADGE_COLOR } else { Color::DarkGray }),
            );
            // Meter: green below -12 dB, amber to -3, red above; dashed
            // and dim when the backend has no reading at all.
            if !v.meter_available {
                buf.set_string(meter_x, y, if k % 2 == 0 { "\u{2504}" } else { " " }, base.fg(Color::DarkGray));
                continue;
            }
            let frac = (k as f32 + 0.5) / bar_h as f32;
            let color = if frac < taper(0.25) {
                Color::Green
            } else if frac < taper(0.7) {
                Color::Yellow
            } else {
                Color::Red
            };
            if k < meter_fill {
                buf.set_string(meter_x, y, "\u{2588}", base.fg(color));
            } else {
                buf.set_string(meter_x, y, "\u{2591}", base.fg(Color::Rgb(0x30, 0x31, 0x36)));
            }
        }

        // Readouts to the right of the bars.
        let text_x = cell.x + 7;
        if text_x >= cell.right() {
            continue;
        }
        let text_w = (cell.right() - text_x) as usize;
        let mut line_y = bar_top;
        let put = |buf: &mut ratatui::buffer::Buffer, y: u16, spans: Vec<Span>| {
            if y < cell.bottom() {
                Paragraph::new(Line::from(spans)).render(Rect::new(text_x, y, text_w as u16, 1), buf);
            }
        };
        put(
            f.buffer_mut(),
            line_y,
            vec![Span::styled(fit(&v.level, text_w), base)],
        );
        line_y += 1;
        if let Some(pan) = &v.pan {
            put(
                f.buffer_mut(),
                line_y,
                vec![Span::styled(format!("pan {pan}"), base.fg(Color::Gray))],
            );
            line_y += 1;
        }
        // Badges wrap onto as many lines as the width needs.
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0usize;
        for (label, color) in &v.badges {
            let w = label.chars().count() + 1;
            if used + w > text_w && !spans.is_empty() {
                put(f.buffer_mut(), line_y, std::mem::take(&mut spans));
                line_y += 1;
                used = 0;
            }
            spans.push(Span::styled(
                format!("{label} "),
                base.fg(*color).add_modifier(Modifier::BOLD),
            ));
            used += w;
        }
        if !spans.is_empty() {
            put(f.buffer_mut(), line_y, spans);
        }
    }
}

/// Row-group boundaries for the input section — mirrors
/// `tuxmix-gui::matrix::input_row_groups` (a separate crate, its own
/// copy of the same grouping rule): the two Mic-type inputs (AN1, AN2)
/// are each their own physical jack, shown solo; every other input
/// type is an inherent hardware pair, shown as one 2-row group.
/// Returns `(label, first_channel_idx, count)`.
fn matrix_input_groups(dev: &DeviceHandle) -> Vec<(String, usize, usize)> {
    let inputs = dev.inputs();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < inputs.len() {
        if inputs[i].channel_type == ChannelType::Mic {
            groups.push((inputs[i].name.clone(), i, 1));
            i += 1;
        } else if let Some(next) = inputs.get(i + 1) {
            groups.push((matrix_pair_label(&inputs[i].name, &next.name), i, 2));
            i += 2;
        } else {
            groups.push((inputs[i].name.clone(), i, 1));
            i += 1;
        }
    }
    groups
}

/// Row-group boundaries for the playback section: always one pair per
/// hardware output bus, labeled with this app's own `OUT_LABELS`.
fn matrix_playback_groups(dev: &DeviceHandle) -> Vec<(String, usize, usize)> {
    let n = dev.playbacks().len();
    (0..n / 2)
        .map(|pair| {
            (
                OUT_LABELS.get(pair).copied().unwrap_or_default().to_string(),
                pair * 2,
                2,
            )
        })
        .collect()
}

/// Combines a linked pair's two channel names ("AN1"/"AN2", "ADAT7"/
/// "ADAT8") into one label ("AN1/2", "ADAT7/8") — mirrors
/// `tuxmix-gui::app::pair_bus_label` (a separate crate, its own copy).
fn matrix_pair_label(left: &str, right: &str) -> String {
    let right_num: String = right.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    if right_num.is_empty() {
        format!("{left}/{right}")
    } else {
        format!("{left}/{right_num}")
    }
}

/// Bare "-9.8"-style cell text, blank for an unrouted crosspoint —
/// mirrors `tuxmix-gui::matrix`'s identical cells.
fn matrix_cell_text(vol: f32) -> String {
    if vol > f32::EPSILON {
        format!("{:>5.1}", 20.0 * vol.log10())
    } else {
        "    .".to_string()
    }
}

const MATRIX_LABEL_W: usize = 11;
const MATRIX_COL_W: usize = 6;

/// Appends one section's rows (inputs or playbacks) to `lines`. Only
/// the column matching this channel's natural side (`idx % 2`, the
/// same convention `set_channel_volume` uses everywhere) shows a
/// value — simplified vs. the GUI, which additionally decodes the 4
/// true-mono AN1-4 inputs' pan into independent L/R values; not worth
/// the terminal-width cost of a second numeric column pair here.
fn matrix_render_rows(
    lines: &mut Vec<String>,
    cols: &[usize],
    groups: &[(String, usize, usize)],
    row_prefix: &str,
    volumes_of: impl Fn(usize) -> Vec<f32>,
) {
    for (label, first, count) in groups {
        for local in 0..*count {
            let idx = first + local;
            let name = if local == 0 { label.as_str() } else { "" };
            let mut line = format!(
                "{:<name_w$}{:>idx_w$}",
                name,
                format!("{row_prefix}{}", idx + 1),
                name_w = MATRIX_LABEL_W - 4,
                idx_w = 4,
            );
            let vols = volumes_of(idx);
            let natural_side = idx % 2;
            for &o in cols {
                let pair = o / 2;
                let text = if o % 2 == natural_side {
                    matrix_cell_text(vols.get(pair).copied().unwrap_or(0.0))
                } else {
                    "    .".to_string()
                };
                line.push_str(&format!("{:>col_w$}", text, col_w = MATRIX_COL_W));
            }
            lines.push(line);
        }
    }
}

/// Matrix (submix) view: TotalMix's own crosspoint grid, rebuilt
/// 2026-09-06 to match `tuxmix-gui::matrix`'s own rebuild — individual
/// output *channels* as columns, individual input/playback *channels*
/// as rows. The previous version here had rows as output *pairs* and
/// columns as input/playback channels: the same backwards axes the GUI
/// had before its own fix, found by checking whether this crate's
/// independent copy of the same view carried the same mistake (it
/// did). Horizontally scrollable (Left/Right while this view is
/// showing, see `matrix_col` in `run()`) since a terminal is rarely
/// wide enough for all 12 output columns at once, unlike the GUI's own
/// scrollable canvas.
fn render_matrix(f: &mut Frame, title: &str, area: Rect, dev: &DeviceHandle, col_offset: usize) {
    let n_out = dev.outputs().len();
    let visible_cols =
        ((area.width as usize).saturating_sub(2 + MATRIX_LABEL_W) / MATRIX_COL_W).max(1);
    let max_offset = n_out.saturating_sub(visible_cols);
    let offset = col_offset.min(max_offset);
    let cols: Vec<usize> = (offset..(offset + visible_cols).min(n_out)).collect();

    let block = Block::default().borders(Borders::ALL).title(format!(
        "{title} — cols {}-{} of {} (\u{2190}/\u{2192} scroll)",
        cols.first().map(|c| c + 1).unwrap_or(0),
        cols.last().map(|c| c + 1).unwrap_or(0),
        n_out
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();

    let mut header = format!("{:<label_w$}", "", label_w = MATRIX_LABEL_W);
    for &o in &cols {
        header.push_str(&format!("{:>col_w$}", format!("Out{}", o + 1), col_w = MATRIX_COL_W));
    }
    lines.push(header);

    matrix_render_rows(&mut lines, &cols, &matrix_input_groups(dev), "I", |idx| {
        dev.inputs()[idx].volumes.clone()
    });
    matrix_render_rows(&mut lines, &cols, &matrix_playback_groups(dev), "P", |idx| {
        dev.playbacks()[idx].volumes.clone()
    });

    // Master "Outputs" row + pair-group footer.
    lines.push(String::new());
    let mut out_row = format!("{:<label_w$}", "Out", label_w = MATRIX_LABEL_W);
    for &o in &cols {
        let vol = dev.outputs().get(o).map(|c| c.volume).unwrap_or(0.0);
        out_row.push_str(&format!("{:>col_w$}", matrix_cell_text(vol), col_w = MATRIX_COL_W));
    }
    lines.push(out_row);

    let mut footer = format!("{:<label_w$}", "", label_w = MATRIX_LABEL_W);
    for &o in &cols {
        let pair = o / 2;
        let label = OUT_LABELS.get(pair).copied().unwrap_or_default();
        footer.push_str(&format!("{:>col_w$}", label, col_w = MATRIX_COL_W));
    }
    lines.push(footer);

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
/// Steps a frequency value (band freq / low-cut freq, both 20-20,000 Hz)
/// multiplicatively rather than by a fixed Hz amount — mirrors
/// `tuxmix-gui::widgets::knob::Knob::log_scale`'s identical reasoning:
/// a fixed additive step over a 3-decade range is either too coarse to
/// dial in a bass frequency precisely, or (the bug this replaces) so
/// fine that reaching the top of the range takes an impractical number
/// of presses — the old 10 Hz fine-step needed ~1800 presses to get
/// from 20 Hz to 20 kHz. A ~5%-per-press fine step (~25% coarse)
/// covers the same 3 decades in roughly 140/30 presses instead, and —
/// same as a real parametric EQ's frequency control — feels like a
/// constant amount of adjustment at any point in the range rather than
/// speeding up disproportionately as the value grows.
fn step_freq_hz(current: u16, dir: i32, coarse: bool) -> u16 {
    let factor: f32 = if coarse { 1.25 } else { 1.05 };
    let new = current as f32 * factor.powi(dir);
    new.round().clamp(20.0, 20_000.0) as u16
}

fn adjust_eq_field(dev: &mut DeviceHandle, idx: usize, row: usize, dir: i32, coarse: bool) {
    let eq = dev
        .inputs()
        .get(idx)
        .and_then(|c| c.eq)
        .unwrap_or_default();
    match row {
        0 => {
            if dev.protocol_features() {
                let _ = dev.set_eq_enabled(idx, !eq.enabled);
            }
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
            let new = step_freq_hz(eq.bands[band].freq_hz, dir, coarse);
            let _ = dev.set_eq_band_freq(idx, band, new);
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
            let new = step_freq_hz(eq.low_cut_freq_hz, dir, coarse);
            let _ = dev.set_eq_low_cut_freq(idx, new);
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
    fn top_hints_drop_the_protocol_keys_on_a_class_compliant_card() {
        // `e`, `l`, `c`, `n`, `i` live in the footer now, per section; this
        // bar keeps the device-wide keys.
        let protocol_keys = [
            "y/h:pitch",
            "u/j:width",
            "x:MS",
            "a:AN1>2",
            "k:input link",
            "f:EQ rec",
            "w:SPDIF out",
        ];
        let full = super::top_hints(true);
        for key in protocol_keys {
            assert!(full.contains(key), "full hint bar should list {key}");
        }
        let cc = super::top_hints(false);
        for key in protocol_keys {
            assert!(
                !cc.contains(key),
                "class-compliant hint bar must not advertise {key}"
            );
        }
        for key in ["o:submix", "r:rate"] {
            assert!(cc.contains(key), "{key} still works on a class-compliant card");
        }
    }

    #[test]
    fn matrix_input_groups_splits_mic_solo_and_pairs_the_rest() {
        let dev = DeviceHandle::Mock(MockBabyfacePro::open().unwrap());
        let groups = matrix_input_groups(&dev);
        // AN1, AN2 (Mic) each solo; everything else in pairs of 2.
        assert_eq!(groups[0], ("AN1".to_string(), 0, 1));
        assert_eq!(groups[1], ("AN2".to_string(), 1, 1));
        assert_eq!(groups[2].1, 2);
        assert_eq!(groups[2].2, 2);
        // Every group after the first two should be a pair (count 2).
        for g in &groups[2..] {
            assert_eq!(g.2, 2, "{:?} should be a 2-channel group", g);
        }
    }

    #[test]
    fn matrix_pair_label_combines_names() {
        assert_eq!(matrix_pair_label("AN1", "AN2"), "AN1/2");
        assert_eq!(matrix_pair_label("ADAT7", "ADAT8"), "ADAT7/8");
    }

    #[test]
    fn matrix_cell_text_is_blank_for_silent_and_numeric_for_routed() {
        assert_eq!(matrix_cell_text(0.0), "    .");
        assert_eq!(matrix_cell_text(1.0), "  0.0");
    }

    #[test]
    fn step_freq_hz_moves_a_roughly_constant_percentage_at_any_point_in_range() {
        // A fixed additive step (the bug this replaces) would move the
        // same absolute Hz amount at 100 Hz as at 10,000 Hz — this must
        // move a roughly constant *ratio* instead.
        let low = step_freq_hz(100, 1, false) as f32 / 100.0;
        let high = step_freq_hz(10_000, 1, false) as f32 / 10_000.0;
        assert!(
            (low - high).abs() < 0.02,
            "expected a similar ratio at both ends, got {low} vs {high}"
        );
        assert!(low > 1.0, "a positive step should increase the frequency");
    }

    #[test]
    fn step_freq_hz_reaches_20khz_from_20hz_in_well_under_a_hundred_coarse_presses() {
        // The old additive coarse step (100 Hz) needed 180 presses for
        // this; the old fine step (10 Hz) needed ~1800. Multiplicative
        // stepping should need on the order of tens of presses.
        let mut hz = 20u16;
        let mut presses = 0;
        while hz < 20_000 && presses < 100 {
            hz = step_freq_hz(hz, 1, true);
            presses += 1;
        }
        assert_eq!(hz, 20_000, "should reach the top of the range");
        assert!(presses < 50, "took {presses} coarse presses, expected well under 50");
    }

    #[test]
    fn step_freq_hz_stays_in_bounds() {
        assert_eq!(step_freq_hz(20, -1, true), 20);
        assert_eq!(step_freq_hz(20_000, 1, true), 20_000);
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
    fn mock_output_meters_are_available_when_every_source_is_metered() {
        let dev = DeviceHandle::open_mock();
        // The mock has real input and playback meters, so even a fully
        // routed output can be summed without guessing.
        assert!(output_meter_available(&dev, 0, 0));
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

    #[test]
    fn the_fader_taper_matches_the_gui_scale() {
        // 0 dB sits well above half height, +6 dB is the top, silence the
        // bottom: the same law the GUI's faders use.
        assert_eq!(taper(0.0), 0.0);
        assert!((taper(2.0) - 1.0).abs() < 1e-4, "+6 dB is the top");
        let unity = taper(1.0);
        assert!(unity > 0.6 && unity < 0.8, "0 dB reads at {unity}");
        assert!(taper(0.5) < unity && taper(1.5) > unity);
    }

    #[test]
    fn long_names_are_cut_with_an_ellipsis_not_run_into_the_next_cell() {
        assert_eq!(fit("AN1/2", 10), "AN1/2");
        assert_eq!(fit("A very long channel name", 8), "A very \u{2026}");
        assert_eq!(fit("abc", 0), "\u{2026}".chars().take(0).collect::<String>() + "\u{2026}");
    }
}
