use iced::futures::channel::mpsc;
use iced::keyboard::{self, Key};
use iced::widget::{
    button, column, container, mouse_area, opaque, pick_list, row, scrollable, stack, text,
};
use iced::{window, Element, Length, Subscription, Task};

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

#[cfg(feature = "alsa")]
use tuxmix_core::BabyfacePro;
use tuxmix_core::{
    BabyfaceProUsb, ChannelId, ChannelType, MockBabyfacePro, RmeDevice, Scene, Sensitivity,
};

use crate::matrix;
use crate::osc::{self, OscCommand, OscConfig, OscOutbound};
use crate::scenes::{list_scene_files, load_scene_file, save_scene_file};
use crate::theme;
use crate::widgets::fader;
use crate::widgets::knob::{knob, Knob};
use crate::widgets::strip;

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
    pub fn open_real() -> Option<Self> {
        // ALSA class-compliant first (historical path), then the
        // proprietary USB backend.
        #[cfg(feature = "alsa")]
        if let Ok(d) = BabyfacePro::open() {
            return Some(DeviceHandle::Real(d));
        }
        BabyfaceProUsb::open().ok().map(DeviceHandle::Usb)
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
            DeviceHandle::Real(_) => vec![0.0; n],
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
    SceneNameChanged(String),
    SceneSave,
    SceneLoad(String),
    ModifiersChanged(keyboard::Modifiers),
    TabPressed,
    EscapePressed,
    /// The OS window was resized (or just opened) — see
    /// `TuxMix::window_width`. Also the trigger for recomputing `ui_scale`
    /// (see `recompute_ui_scale`) — TotalMix-2.0-style adaptive scale
    /// replaced manual zoom entirely, so window size is the only thing
    /// that ever changes it now (that, and a strip's collapsed state
    /// changing, which affects the same fit-to-window math — see
    /// `set_collapsed`). Width-only, deliberately: an earlier version
    /// also fit to height, but that meant a wide-but-short window could
    /// shrink the scale enough to leave dead space on the sides — width
    /// alone always fills the window edge to edge; a short window scrolls
    /// vertically to reach Hardware Outputs instead, same as any normal
    /// scrollable view.
    WindowResized(f32),

    Mute(ChannelId, bool),
    Solo(ChannelId, bool),
    Phantom(usize, bool),
    Pad(usize, bool),
    /// Raw preamp gain units (0..=`InputChannel::gain_max`), not dB —
    /// the hardware's own scale isn't a fixed dB curve we can label.
    Gain(usize, u32),
    /// `true` = +4dBu, `false` = -10dBV.
    Sensitivity(usize, bool),

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
    pub scene_name: String,
    pub scene_list: Vec<String>,
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
    /// Adaptive UI scale, multiplied into every text size and widget
    /// dimension in the mixer/matrix views — recomputed by
    /// `recompute_ui_scale` on every window resize (and strip
    /// collapse/expand) so the widest strip row always fits without
    /// horizontal scrolling, not a manual zoom control. `theme::SCALE_*`
    /// constants define the default/bounds.
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

pub fn new(mock: bool, osc_config: Option<OscConfig>) -> TuxMix {
    let mut device = if mock {
        DeviceHandle::open_mock()
    } else {
        DeviceHandle::open_real().unwrap_or_else(|| {
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
        scene_name: String::new(),
        scene_list: list_scene_files(),
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
fn apply_grouped_volume(state: &mut TuxMix, cid: ChannelId, out: usize, v: f32) {
    if state.selected.len() > 1 && state.selected.contains(&cid) {
        let old = state.device.volume(cid, out).unwrap_or(v);
        let delta_db = vol_to_db(v) - vol_to_db(old);
        for sel in state.selected.clone() {
            let cur = state.device.volume(sel, out).unwrap_or(0.0);
            let new_vol = db_to_vol(vol_to_db(cur) + delta_db).clamp(0.0, 2.0);
            let _ = state.device.set_volume(sel, out, new_vol);
            notify_osc(state, OscOutbound::Volume(sel, out, new_vol));
        }
    } else {
        let _ = state.device.set_volume(cid, out, v);
        notify_osc(state, OscOutbound::Volume(cid, out, v));
    }
}

/// Same relative-delta grouping as `apply_grouped_volume`, for pan.
fn apply_grouped_pan(state: &mut TuxMix, cid: ChannelId, out: usize, pan: i8) {
    if state.selected.len() > 1 && state.selected.contains(&cid) {
        let old = i16::from(state.device.pan(cid, out).unwrap_or(pan));
        let delta = i16::from(pan) - old;
        for sel in state.selected.clone() {
            let cur = i16::from(state.device.pan(sel, out).unwrap_or(0));
            let new = (cur + delta).clamp(-100, 100) as i8;
            let _ = state.device.set_pan(sel, out, new);
            notify_osc(state, OscOutbound::Pan(sel, out, new));
        }
    } else {
        let _ = state.device.set_pan(cid, out, pan);
        notify_osc(state, OscOutbound::Pan(cid, out, pan));
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
    recompute_ui_scale(state);
}

/// `target = None` closes whatever's open; `Some((cid, kind))` opens that
/// flyout, closing any other one first (of either kind — only one is ever
/// open at a time). Instant, no animation.
fn set_flyout_open(state: &mut TuxMix, target: Option<(ChannelId, strip::FlyoutKind)>) {
    state.flyout_open = target;
}

pub fn update(state: &mut TuxMix, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
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
        Message::QuickChannelSelected(cid) => state.quick_channel = cid,
        Message::SelectOutput(i) => {
            state.sel_out = i;
            // Selecting a bus from anywhere (top bar or a strip's own route
            // flyout) dismisses whatever flyout is open — it did its job.
            set_flyout_open(state, None);
        }
        Message::SceneNameChanged(s) => state.scene_name = s,
        Message::SceneSave => {
            let n = state.scene_name.trim().to_string();
            if !n.is_empty() && save_scene_file(&n, &state.device.capture_scene()).is_ok() {
                state.scene_name.clear();
                state.scene_list = list_scene_files();
            }
        }
        Message::SceneLoad(name) => {
            if let Some(scene) = load_scene_file(&name) {
                // Was previously `let _ = ...` — silently discarded a
                // scene/device model mismatch (or any other apply
                // failure) with no way for the user to ever find out
                // why nothing happened. No toast/notification system
                // exists yet, so a log line is the minimum fix that
                // makes the failure observable at all.
                if let Err(err) = state.device.apply_scene(&scene) {
                    log::warn!("Failed to apply scene '{name}': {err}");
                }
            }
        }
        Message::ModifiersChanged(m) => state.modifiers = m,
        Message::WindowResized(width) => {
            state.window_width = width;
            recompute_ui_scale(state);
        }
        Message::EscapePressed => {
            if state.editing.is_some() {
                state.editing = None;
            }
        }
        Message::Mute(cid, m) => {
            if state.selected.len() > 1 && state.selected.contains(&cid) {
                for sel in state.selected.clone() {
                    let _ = state.device.set_mute(sel, m);
                    notify_osc(state, OscOutbound::Mute(sel, m));
                }
            } else {
                let _ = state.device.set_mute(cid, m);
                notify_osc(state, OscOutbound::Mute(cid, m));
            }
        }
        Message::Solo(cid, s) => {
            if state.selected.len() > 1 && state.selected.contains(&cid) {
                for sel in state.selected.clone() {
                    let _ = state.device.set_solo(sel, s);
                    notify_osc(state, OscOutbound::Solo(sel, s));
                }
            } else {
                let _ = state.device.set_solo(cid, s);
                notify_osc(state, OscOutbound::Solo(cid, s));
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
                    let _ = state.device.set_volume(sel, out, default_vol);
                    notify_osc(state, OscOutbound::Volume(sel, out, default_vol));
                }
            } else {
                let _ = state.device.set_volume(cid, out, default_vol);
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
                    let _ = state.device.set_volume(cid, out, v);
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
                let _ = state.device.set_mute(cid, m);
                notify_osc(state, OscOutbound::Mute(cid, m));
            }
            OscCommand::Solo(cid, s) => {
                let _ = state.device.set_solo(cid, s);
                notify_osc(state, OscOutbound::Solo(cid, s));
            }
            OscCommand::OutputVolume(id, v) => {
                let cid = ChannelId::Output(id);
                // `out` must equal the output channel index (see
                // `strip_params`); `set_volume(Output(i), i, v)` writes
                // output_for(i) and updates outputs[i].volume.
                let _ = state.device.set_volume(cid, id, v);
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
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
            Key::Named(keyboard::key::Named::Tab) => Some(Message::TabPressed),
            Key::Named(keyboard::key::Named::Escape) => Some(Message::EscapePressed),
            _ => None,
        },
        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
            Some(Message::ModifiersChanged(m))
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
    let mut col = column![top, content]
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

    container(
        column![header, clock_row, rate_row, spdif_row]
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
    let status_color = if state.device.is_mock() {
        theme::YSIM
    } else {
        theme::GCONN
    };
    let status_label = if state.device.is_mock() {
        "Simulated"
    } else {
        "Connected"
    };

    // Primary identity: brand + connected device. The one element in the
    // bar that's meant to be visually loud — everything else is a tool,
    // this is "what am I even looking at".
    let device_chip = chip(
        row![
            text("●").color(status_color).size(theme::TEXT_SM * scale),
            text(state.device.model_name())
                .color(theme::TEXT_PRIMARY)
                .size(theme::TEXT_LG * scale),
            text(status_label)
                .color(status_color)
                .size(theme::TEXT_MD * scale),
        ]
        .spacing(theme::SPACE_MD)
        .align_y(iced::Alignment::Center),
        scale,
    );

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

    // Secondary session tools: scene / submix / clock. These used to be
    // three separate chips carrying the same visual weight as the device
    // identity chip — merged into one quieter toolbar so the bar reads as
    // "one important thing, one toolbar" instead of five equal boxes.
    let scene_list = state.scene_list.clone();
    let session = chip(
        row![
            text("Scene")
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale),
            iced::widget::text_input("name", &state.scene_name)
                .on_input(Message::SceneNameChanged)
                .on_submit(Message::SceneSave)
                .style(theme::text_input)
                .width(Length::Fixed(90.0 * scale))
                .size(theme::TEXT_MD * scale),
            iced::widget::button(text("Save").size(theme::TEXT_MD * scale))
                .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
                .style(theme::plain_button)
                .on_press(Message::SceneSave),
            pick_list(scene_list, None::<String>, Message::SceneLoad)
                .placeholder("load...")
                .style(theme::pick_list)
                .menu_style(theme::menu)
                .text_size(theme::TEXT_MD * scale),
            v_divider(scale),
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
        device_chip,
        tab_toggle,
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
    bar = bar.push(session);

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
        has_48v: false,
        has_pad: false,
        phantom: false,
        pad: false,
        has_gain: false,
        gain: 0,
        gain_max: 0,
        has_sensitivity: false,
        sensitivity_plus4: false,
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
                has_48v,
                has_pad: has_48v,
                phantom: ch.phantom,
                pad: ch.pad,
                has_gain: ch.gain_max.is_some(),
                gain: ch.gain.unwrap_or(0),
                gain_max: ch.gain_max.unwrap_or(0),
                has_sensitivity: ch.sensitivity.is_some(),
                sensitivity_plus4: ch.sensitivity == Some(Sensitivity::Plus4dBu),
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
                mute: ch.mute,
                solo: ch.solo,
                default_vol: 1.0,
                ..base
            }
        }
    }
}

/// A row's `scale == 1.0` width, split into the part that scales with
/// `ui_scale` (every strip's own base width — `strip::full_width`/
/// `COLLAPSED_W`, honoring current collapse state) and the part that
/// doesn't: the `SPACE_MD` gaps `mixer_view`'s own `row![].spacing(SPACE_MD)`
/// puts between items, and (when `types` is given — Hardware Inputs only)
/// the 1px rule + its own gap inserted between channel-type groups.
/// `recompute_ui_scale` needs them kept apart to solve
/// `scaling * scale + fixed <= available_width` for `scale` — collapsing
/// them into one total and solving `available_width / (scaling + fixed)`
/// looks reasonable but answers the wrong equation (it implicitly scales
/// the gaps too, which don't actually scale), landing a few pixels wider
/// than `available_width` and popping an unwanted horizontal scrollbar.
/// Mirrors `mixer_view`'s own width bookkeeping for its input/pb/out
/// loops; kept as a separate, widget-free computation (rather than
/// factored out of those loops, which build widgets at the same time) so
/// `recompute_ui_scale` can solve for the scale that makes a row fit,
/// instead of only checking whether the *current* scale already does.
fn row_width_parts(
    state: &TuxMix,
    n: usize,
    cid_at: impl Fn(usize) -> ChannelId,
    types: Option<&[ChannelType]>,
) -> (f32, f32) {
    let mut scaling = 0.0f32;
    let mut fixed = 0.0f32;
    let mut item_count = 0usize;
    let mut prev_type: Option<ChannelType> = None;
    for i in 0..n {
        if let Some(types) = types {
            let t = types[i];
            if prev_type.is_some_and(|p| p != t) {
                fixed += 1.0;
                item_count += 1;
            }
            prev_type = Some(t);
        }
        let cid = cid_at(i);
        scaling += if state.collapsed.contains(&cid) {
            strip::COLLAPSED_W
        } else {
            strip::full_width(cid)
        };
        item_count += 1;
    }
    fixed += item_count.saturating_sub(1) as f32 * theme::SPACE_MD;
    (scaling, fixed)
}

/// The largest scale that keeps `scaling * scale + fixed` within `limit`,
/// or `None` if there's nothing to scale (`scaling <= 0`, an empty row) —
/// `recompute_ui_scale` takes the smallest of these across every row/axis
/// it cares about, since that's the one constraint that's actually
/// binding.
fn max_scale_to_fit(scaling: f32, fixed: f32, limit: f32) -> Option<f32> {
    (scaling > 0.0).then(|| (limit - fixed) / scaling)
}

/// Recomputes `ui_scale` so every strip row (Hardware Inputs, Software
/// Playback, Hardware Outputs) fits the window without horizontal
/// scrolling — TotalMix-2.0-style: adaptive scale on resize replaces
/// manual zoom entirely, so this is the *only* place `ui_scale` ever
/// changes (called from `Message::WindowResized` and from
/// `set_collapsed`, the two things that can change a row's total width).
///
/// Width-only, deliberately — an earlier version also solved against
/// `window_height` so Hardware Outputs would never need a vertical
/// scroll, but that meant a wide-but-short window could shrink the scale
/// enough to leave visibly dead space on the sides (the one axis it
/// *wasn't* solving for stops filling the window once a different axis
/// becomes the binding constraint). Width alone always fills the window
/// edge to edge, by construction; a short window scrolls vertically to
/// reach Hardware Outputs instead — the page's own vertical scrollable
/// (see `page()`) already exists for exactly this, and scrolling for
/// content that doesn't fit is normal, expected behavior, not something
/// to design around.
///
/// Solves each row independently (`max_scale_to_fit`) and takes the
/// smallest result — the one row that's actually binding — clamped to
/// `SCALE_MIN`/`SCALE_MAX`.
fn recompute_ui_scale(state: &mut TuxMix) {
    let input_types: Vec<ChannelType> = state
        .device
        .inputs()
        .iter()
        .map(|c| c.channel_type)
        .collect();
    let rows = [
        row_width_parts(
            state,
            state.device.inputs().len(),
            ChannelId::Input,
            Some(&input_types),
        ),
        row_width_parts(
            state,
            state.device.playbacks().len(),
            ChannelId::Playback,
            None,
        ),
        row_width_parts(state, state.device.outputs().len(), ChannelId::Output, None),
    ];

    // Same page padding `responsive_row`'s `available_width` uses.
    let available_width = (state.window_width - 2.0 * theme::SPACE_XL - 4.0).max(1.0);

    let scale = rows
        .into_iter()
        .filter_map(|(scaling, fixed)| max_scale_to_fit(scaling, fixed, available_width))
        .fold(f32::INFINITY, f32::min);

    state.ui_scale = if scale.is_finite() {
        scale.clamp(theme::SCALE_MIN, theme::SCALE_MAX)
    } else {
        theme::SCALE_DEFAULT
    };
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

/// The settings flyout's content — 48V/PAD, sensitivity, and gain, gated
/// exactly like the strip itself used to gate them inline (built from the
/// same `strip_params()` every strip already uses, so that has_48v/etc.
/// logic isn't duplicated here). Non-`Input` channels never have this
/// flyout open in the first place (see `header_row`'s `has_settings`
/// gate), so the early-return is just a defensive fallback, not a real
/// path.
///
/// Gain used to be excluded: a `Knob` (Canvas widget) placed inside this
/// flyout back when it was a `Stack`-based overlay broke input handling
/// for the *entire* row after the first click. Now that this flyout
/// pushes the row instead of overlaying it (no more `Stack`/`opaque`, see
/// `with_flyout`), that failure mode's precondition is gone, so Gain
/// moved in with the rest.
fn settings_popover(state: &TuxMix, cid: ChannelId, width: f32) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let ChannelId::Input(idx) = cid else {
        return container(iced::widget::Space::new()).into();
    };
    let p = strip_params(state, cid, state.sel_out);

    let mut col = column![].spacing(theme::SPACE_SM * scale);

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
        col = col.push(
            button(text(label).size(theme::TEXT_SM * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::plain_button)
                .on_press(Message::Sensitivity(idx, !p.sensitivity_plus4)),
        );
    }

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
    let mut input_strips = row![].spacing(theme::SPACE_MD);
    let mut input_width = 0.0f32;
    let mut input_item_count = 0usize;
    let mut input_open_x: Option<f32> = None;
    let mut prev_type: Option<ChannelType> = None;
    for (i, ch) in state.device.inputs().iter().enumerate() {
        if prev_type.is_some_and(|t| t != ch.channel_type) {
            // `rule::vertical` hardcodes `height: Length::Fill` with no way
            // to override it — inside this row (itself `Length::Shrink`,
            // sized to its tallest strip), that Fill child was pulling the
            // *entire row* up to whatever space the window happened to
            // have, leaving a large empty gap below Hardware Inputs on any
            // window taller than its content. Wrapping it in a
            // `Length::Shrink` container stops the Fill from escaping
            // upward — it collapses to the container's own (content-sized)
            // height instead of the whole window's.
            input_strips = input_strips
                .push(container(iced::widget::rule::vertical(1)).height(Length::Shrink));
            input_width += 1.0;
            input_item_count += 1;
        }
        prev_type = Some(ch.channel_type);
        let cid = ChannelId::Input(i);
        let strip_widget = strip::strip(strip_params(state, cid, state.sel_out));
        let mut item_width = rendered_strip_width(state, cid);
        // Settings pushes the row instead of overlaying it (see
        // `with_flyout`'s doc comment for why) — done here, by widening
        // this loop iteration's own item, rather than in `with_flyout`,
        // since ordinary `row!` layout already pushes every later sibling
        // over for free once this one item is wider.
        if state.flyout_open == Some((cid, strip::FlyoutKind::Settings)) {
            // Same width as the strip itself (see the reference design),
            // not the Route flyout's own fixed `FLYOUT_W` — a dropdown
            // list of bus names and a settings panel of knobs/buttons
            // don't need to match each other's width, just their own
            // strip's.
            let panel_w = item_width;
            input_strips = input_strips.push(
                row![strip_widget, settings_popover(state, cid, panel_w)].spacing(theme::SPACE_MD),
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
    input_width += input_item_count.saturating_sub(1) as f32 * theme::SPACE_MD;

    let mut pb_strips = row![].spacing(theme::SPACE_MD);
    let mut pb_width = 0.0f32;
    let mut pb_open_x: Option<f32> = None;
    for i in 0..state.device.playbacks().len() {
        let cid = ChannelId::Playback(i);
        pb_strips = pb_strips.push(strip::strip(strip_params(state, cid, state.sel_out)));
        pb_width += rendered_strip_width(state, cid);
        if state.flyout_open.map(|(c, _)| c) == Some(cid) {
            pb_open_x = Some(pb_width + i as f32 * theme::SPACE_MD);
        }
    }
    pb_width += state.device.playbacks().len().saturating_sub(1) as f32 * theme::SPACE_MD;

    let mut out_strips = row![].spacing(theme::SPACE_MD);
    let mut out_width = 0.0f32;
    for i in 0..state.device.outputs().len() {
        let cid = ChannelId::Output(i);
        out_strips = out_strips.push(strip::strip(strip_params(state, cid, state.sel_out)));
        out_width += rendered_strip_width(state, cid);
    }
    out_width += state.device.outputs().len().saturating_sub(1) as f32 * theme::SPACE_MD;

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
    use super::{max_scale_to_fit, new, recompute_ui_scale, row_width_parts, MeterAnim};
    use tuxmix_core::{ChannelId, RmeDevice};

    fn widest_row_width_parts(state: &super::TuxMix) -> (f32, f32) {
        let input_types: Vec<_> = state
            .device
            .inputs()
            .iter()
            .map(|c| c.channel_type)
            .collect();
        [
            row_width_parts(
                state,
                state.device.inputs().len(),
                ChannelId::Input,
                Some(&input_types),
            ),
            row_width_parts(
                state,
                state.device.playbacks().len(),
                ChannelId::Playback,
                None,
            ),
            row_width_parts(state, state.device.outputs().len(), ChannelId::Output, None),
        ]
        .into_iter()
        .max_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)))
        .unwrap()
    }

    #[test]
    fn scale_clamps_to_min_when_window_is_very_narrow() {
        let mut state = new(true, None);
        state.window_width = 50.0;
        recompute_ui_scale(&mut state);
        assert_eq!(state.ui_scale, crate::theme::SCALE_MIN);
    }

    #[test]
    fn scale_clamps_to_max_when_window_is_very_wide() {
        let mut state = new(true, None);
        state.window_width = 20_000.0;
        recompute_ui_scale(&mut state);
        assert_eq!(state.ui_scale, crate::theme::SCALE_MAX);
    }

    #[test]
    fn width_scale_exactly_fits_the_widest_row_at_the_solved_boundary() {
        let mut state = new(true, None);
        let (scaling, fixed) = widest_row_width_parts(&state);
        // Inverse of `recompute_ui_scale`'s own `available_width` math —
        // the exact window width that makes `scale == 1.0` the answer.
        state.window_width = scaling + fixed + 2.0 * crate::theme::SPACE_XL + 4.0;

        recompute_ui_scale(&mut state);
        assert!(
            (state.ui_scale - crate::theme::SCALE_DEFAULT).abs() < 0.01,
            "expected ~{}, got {}",
            crate::theme::SCALE_DEFAULT,
            state.ui_scale
        );
    }

    #[test]
    fn scale_grows_monotonically_with_window_width() {
        let mut state = new(true, None);
        state.window_width = 600.0;
        recompute_ui_scale(&mut state);
        let narrow = state.ui_scale;

        state.window_width = 1600.0;
        recompute_ui_scale(&mut state);
        let wide = state.ui_scale;

        assert!(
            wide > narrow,
            "wider window should yield a larger scale: {narrow} vs {wide}"
        );
    }

    #[test]
    fn max_scale_to_fit_is_none_for_an_empty_row() {
        assert_eq!(max_scale_to_fit(0.0, 10.0, 500.0), None);
    }

    #[test]
    fn collapsing_a_strip_shrinks_the_row_it_belongs_to() {
        let state = new(true, None);
        let (before, _) =
            row_width_parts(&state, state.device.inputs().len(), ChannelId::Input, None);

        let mut collapsed_state = new(true, None);
        collapsed_state.collapsed.insert(ChannelId::Input(0));
        let (after, _) = row_width_parts(
            &collapsed_state,
            collapsed_state.device.inputs().len(),
            ChannelId::Input,
            None,
        );

        assert!(
            after < before,
            "collapsing a strip should shrink the row's total scaling width: {before} vs {after}"
        );
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
}
