//! `tuxmix-tui` — Terminal-based RME interface controller.
//!
//! ```bash
//! cargo run -p tuxmix-tui              # with hardware
//! cargo run -p tuxmix-tui -- --mock    # simulation
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
    fn open_real() -> Option<Self> {
        // ALSA class-compliant first (historical path), then the
        // proprietary USB backend.
        #[cfg(feature = "alsa")]
        if let Ok(d) = BabyfacePro::open() {
            return Some(DeviceHandle::Real(d));
        }
        BabyfaceProUsb::open().ok().map(DeviceHandle::Usb)
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
    /// Output meters, computed host-side like TotalMix: each output's
    /// level is the power sum of every routed source (inputs + playbacks)
    /// scaled by that source's fader into the output.
    fn output_meters(&self) -> Vec<f32> {
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
    fn is_mock(&self) -> bool {
        matches!(self, DeviceHandle::Mock(_))
    }
}
const OUT_LABELS: [&str; 6] = ["AN1/2", "PH3/4", "AS1/2", "A3/A4", "A5/A6", "A7/A8"];

/// Maps the current section/channel cursor to the `ChannelId` it points
/// at — was five near-identical `match section { 0 => Input, 1 =>
/// Playback, _ => Output }` blocks inline in `run`'s key handler, one per
/// key binding.
fn selected_channel_id(section: usize, channel: usize) -> ChannelId {
    match section {
        0 => ChannelId::Input(channel),
        1 => ChannelId::Playback(channel),
        _ => ChannelId::Output(channel),
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
    let mock = std::env::args().any(|a| a == "--mock");
    let mut device: DeviceHandle = if mock {
        DeviceHandle::open_mock()
    } else {
        DeviceHandle::open_real().unwrap_or_else(|| {
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
            )
        })?;
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
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
                                0 => dev.inputs().len(),
                                1 => dev.playbacks().len(),
                                _ => dev.outputs().len(),
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
                            let cid = selected_channel_id(section, channel);
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
                                let _ = dev.set_volume(cid, out, v);
                            }
                        }
                        KeyCode::Char('-') => {
                            let cid = selected_channel_id(section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db - 1.0).max(-65.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                let _ = dev.set_volume(cid, out, v);
                            }
                        }
                        // Fine volume: +/- 0.1 dB (PgUp/PgDn — Shift+'+' is
                        // ambiguous on AZERTY where '+' already needs Shift).
                        KeyCode::PageUp => {
                            let cid = selected_channel_id(section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db + 0.1).min(6.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                let _ = dev.set_volume(cid, out, v);
                            }
                        }
                        KeyCode::PageDown => {
                            let cid = selected_channel_id(section, channel);
                            let out = if section == 2 { channel } else { sel_out };
                            if let Ok(v) = dev.volume(cid, out) {
                                let db = if v <= 0.0 { -65.0 } else { 20.0 * v.log10() };
                                let db = (db - 0.1).max(-65.0);
                                let v = if db <= -65.0 {
                                    0.0
                                } else {
                                    10f32.powf(db / 20.0)
                                };
                                let _ = dev.set_volume(cid, out, v);
                            }
                        }
                        KeyCode::Char('m') => {
                            let cid = selected_channel_id(section, channel);
                            if let Ok(m) = dev.mute(cid) {
                                let _ = dev.set_mute(cid, !m);
                            }
                        }
                        KeyCode::Char('s') => {
                            let cid = selected_channel_id(section, channel);
                            if let Ok(s) = dev.solo(cid) {
                                let _ = dev.set_solo(cid, !s);
                            }
                        }
                        // Lowercase 48V / uppercase (Shift+p) PAD — both are
                        // input-only, gated on Mic type the same way
                        // `tuxmix-gui`'s strip only shows these two buttons
                        // for mic channels (see `strip::full_strip`).
                        KeyCode::Char('p') => {
                            if section == 0 {
                                if let Some(ic) = dev.inputs().get(channel) {
                                    let new_state = !ic.phantom;
                                    let _ = dev.set_phantom(channel, new_state);
                                }
                            }
                        }
                        KeyCode::Char('P') => {
                            if section == 0 {
                                if let Some(ic) = dev.inputs().get(channel) {
                                    let new_state = !ic.pad;
                                    let _ = dev.set_pad(channel, new_state);
                                }
                            }
                        }
                        // Preamp gain in dB (0-65), stepped 1 dB at a time
                        // like TotalMix. Only Mic/Instrument inputs have a
                        // gain control. `]`/`[` (QWERTY) + `g`/`d`
                        // (AZERTY-friendly alternates).
                        KeyCode::Char(']') | KeyCode::Char('g') => {
                            if section == 0 {
                                if let Some(ic) = dev.inputs().get(channel) {
                                    if let Some(max) = ic.gain_max {
                                        let new_gain = (ic.gain.unwrap_or(0) + 1).min(max);
                                        let _ = dev.set_gain(channel, new_gain);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('[') | KeyCode::Char('d') => {
                            if section == 0 {
                                if let Some(ic) = dev.inputs().get(channel) {
                                    if ic.gain_max.is_some() {
                                        let new_gain = ic.gain.unwrap_or(0).saturating_sub(1);
                                        let _ = dev.set_gain(channel, new_gain);
                                    }
                                }
                            }
                        }
                        // Input sensitivity (+4dBu / -10dBV), Instrument
                        // inputs only.
                        KeyCode::Char('v') => {
                            if section == 0 {
                                if let Some(ic) = dev.inputs().get(channel) {
                                    if let Some(current) = ic.sensitivity {
                                        let next = match current {
                                            tuxmix_core::Sensitivity::Plus4dBu => {
                                                tuxmix_core::Sensitivity::Minus10dBV
                                            }
                                            tuxmix_core::Sensitivity::Minus10dBV => {
                                                tuxmix_core::Sensitivity::Plus4dBu
                                            }
                                        };
                                        let _ = dev.set_sensitivity(channel, next);
                                    }
                                }
                            }
                        }
                        // Pan, Input/Playback only — matches tuxmix-gui's
                        // step-less drag range in spirit but as a fixed
                        // nudge, same pattern as +/-  for volume.
                        KeyCode::Char(',') => {
                            if section == 0 || section == 1 {
                                let cid = selected_channel_id(section, channel);
                                if let Ok(p) = dev.pan(cid, sel_out) {
                                    let _ = dev.set_pan(cid, sel_out, (p - 5).max(-100));
                                }
                            }
                        }
                        KeyCode::Char('.') => {
                            if section == 0 || section == 1 {
                                let cid = selected_channel_id(section, channel);
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
    let view_tag = if show_matrix {
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
            Span::raw("  q:quit Tab:toggle o:submix y/h:pitch r:rate"),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        top[0],
    );

    let s = format!(
        "HW Inputs: {}  |  SW Playbacks: {}  |  Submix: {}  |  Rate: {} kHz  |  Clock: {}  |  Pitch: {:+0.1}%",
        dev.inputs().len(),
        dev.playbacks().len(),
        OUT_LABELS[sel_out],
        dev.settings().sample_rate / 1000,
        dev.settings().clock_source,
        dev.settings().pitch_percent
    );
    f.render_widget(
        Paragraph::new(s).block(Block::default().borders(Borders::ALL).title("Overview")),
        top[1],
    );

    if show_matrix {
        render_matrix(f, "Matrix Mixer", matrix_area, dev);
    } else {
        render_strips(
            f,
            "Hardware Inputs",
            inputs_area,
            dev.inputs().len(),
            sel_sec == 0,
            sel_chan,
            |i| {
                let ch = &dev.inputs()[i];
                let m = in_meters.get(i).copied().unwrap_or(0.0);
                let mut label = format!(
                    "{} [{:?}] {}",
                    ch.name,
                    ch.channel_type,
                    db_text(ch.volumes[sel_out])
                );
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
                (label, m)
            },
        );
        render_strips(
            f,
            "Software Playbacks",
            playbacks_area,
            dev.playbacks().len(),
            sel_sec == 1,
            sel_chan,
            |i| {
                let ch = &dev.playbacks()[i];
                let m = pb_meters.get(i).copied().unwrap_or(0.0);
                let mut label = format!("{} {}", ch.name, db_text(ch.volumes[sel_out]));
                if ch.mute {
                    label.push_str(" [M]");
                }
                if ch.solo {
                    label.push_str(" [S]");
                }
                label.push_str(&format!(" {}", pan_text(ch.pans[sel_out])));
                (label, m)
            },
        );
        render_strips(
            f,
            "Hardware Outputs",
            outputs_area,
            dev.outputs().len(),
            sel_sec == 2,
            sel_chan,
            |i| {
                let ch = &dev.outputs()[i];
                let mut label = format!("{} {}", ch.name, db_text(ch.volume));
                if ch.mute {
                    label.push_str(" [M]");
                }
                if ch.solo {
                    label.push_str(" [S]");
                }
                (label, out_meters.get(i).copied().unwrap_or(0.0))
            },
        );
    }
    let footer: String = if show_matrix {
        "Tab: return to mixer".into()
    } else {
        format!(
            "IN:{}:{}  +/-:vol  PgUp/PgDn:0.1dB  ,/.:pan  m:mute  s:solo  p:48V  P:pad  [/] or g/d:gain  v:sens  arrows:navigate  q:quit",
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
    label_fn: impl Fn(usize) -> (String, f32),
) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let cols = count.min(6) as u16;
    let rows = ((count as u16) + cols - 1) / cols;
    let row_h = (inner.height / rows.max(1)).max(3);
    for i in 0..count {
        let (label, meter) = label_fn(i);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(selected_channel_id(0, 3), ChannelId::Input(3));
        assert_eq!(selected_channel_id(1, 2), ChannelId::Playback(2));
        assert_eq!(selected_channel_id(2, 5), ChannelId::Output(5));
    }
}
