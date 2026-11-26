// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Reverb and echo for the Babyface Pro FS, computed on the host.
//!
//! The unit has no effects of its own. TotalMix FX computes its reverb and
//! echo on the computer: its knobs send nothing over USB, the FX bus (the
//! sum of every strip's FX send) arrives on capture channels 13/14, and the
//! effect goes back on playback channels 13/14, which the unit mixes into
//! each output at that output's FX Return level (usbmon against TotalMix
//! FX, 2026-11-26; kernel driver 0.2.5 exposes both, and its `Host FX
//! Switch` turns the return on).
//!
//! TuxMix runs the effect as a PipeWire filter-chain between those
//! channels: four convolvers, one per input/output side, whose impulse
//! responses this module computes from TotalMix-like settings. A
//! convolution covers a reverb, an echo with feedback and their stereo
//! width at once, so the settings never run as code in the audio thread:
//! a change writes new responses and restarts the filter.
//!
//! The reverb types are modelled here, not copied from RME: same names
//! and knobs, a sound of our own.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Sample rate the responses are computed at. The FX channels exist at 44.1
/// and 48 kHz only; PipeWire resamples the response for 44.1.
pub const RATE: u32 = 48_000;

/// Longest response written, in seconds: past this a tail is inaudible at
/// any setting we offer, and the convolvers' cost grows with the length.
pub const MAX_SECONDS: f32 = 6.0;

/// The reverb's character, TotalMix FX's "Type".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReverbType {
    SmallRoom,
    MediumRoom,
    #[default]
    LargeRoom,
    Hall,
    Plate,
    Space,
    Gated,
}

impl ReverbType {
    pub const ALL: [ReverbType; 7] = [
        ReverbType::SmallRoom,
        ReverbType::MediumRoom,
        ReverbType::LargeRoom,
        ReverbType::Hall,
        ReverbType::Plate,
        ReverbType::Space,
        ReverbType::Gated,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ReverbType::SmallRoom => "Small Room",
            ReverbType::MediumRoom => "Medium Room",
            ReverbType::LargeRoom => "Large Room",
            ReverbType::Hall => "Hall",
            ReverbType::Plate => "Plate",
            ReverbType::Space => "Space",
            ReverbType::Gated => "Gated",
        }
    }

    /// (decay time to -60 dB, span of the early reflections, how many of
    /// them), in seconds, at room scale 1.
    fn shape(self) -> (f32, f32, usize) {
        match self {
            ReverbType::SmallRoom => (0.45, 0.025, 8),
            ReverbType::MediumRoom => (0.8, 0.04, 10),
            ReverbType::LargeRoom => (1.3, 0.06, 12),
            ReverbType::Hall => (2.2, 0.08, 14),
            ReverbType::Plate => (1.6, 0.0, 0),
            ReverbType::Space => (4.0, 0.12, 16),
            // The gate's length, not a decay: see `reverb_side`.
            ReverbType::Gated => (0.25, 0.02, 6),
        }
    }
}

/// TotalMix FX's reverb panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReverbParams {
    pub enabled: bool,
    pub kind: ReverbType,
    /// Before the first reflection, ms (0..=999).
    pub pre_delay_ms: f32,
    /// Scales the room's size and decay (0.5..=3.0).
    pub room_scale: f32,
    /// High-pass on the reverb, Hz (20..=500).
    pub low_cut_hz: f32,
    /// Low-pass on the reverb, Hz (5000..=20000).
    pub high_cut_hz: f32,
    /// How gradually the late reverb builds up, 0..=1.
    pub smooth: f32,
    /// Stereo width of the reverb, -1..=1 (+1 the widest, 0 mono).
    pub width: f32,
    /// Level of the reverb, dB (-65..=6).
    pub volume_db: f32,
}

impl Default for ReverbParams {
    /// TotalMix FX's defaults, as its panel shows them.
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ReverbType::LargeRoom,
            pre_delay_ms: 35.0,
            room_scale: 1.0,
            low_cut_hz: 80.0,
            high_cut_hz: 18_000.0,
            smooth: 1.0,
            width: 0.8,
            volume_db: -3.0,
        }
    }
}

impl ReverbParams {
    /// Switches to `kind` and loads its typical settings, like a preset:
    /// pre delay, room scale, cuts, smooth and width move to suit the
    /// type. On/off and the volume stay as they are. Large Room loads the
    /// defaults.
    pub fn load_type(&mut self, kind: ReverbType) {
        let (pre_delay_ms, room_scale, low_cut_hz, high_cut_hz, smooth, width) = match kind {
            ReverbType::SmallRoom => (5.0, 1.0, 100.0, 12_000.0, 0.6, 0.6),
            ReverbType::MediumRoom => (15.0, 1.0, 90.0, 14_000.0, 0.8, 0.7),
            ReverbType::LargeRoom => (35.0, 1.0, 80.0, 18_000.0, 1.0, 0.8),
            ReverbType::Hall => (40.0, 1.2, 60.0, 16_000.0, 1.0, 1.0),
            ReverbType::Plate => (10.0, 1.0, 150.0, 20_000.0, 0.5, 0.9),
            ReverbType::Space => (80.0, 1.5, 50.0, 12_000.0, 1.0, 1.0),
            ReverbType::Gated => (0.0, 1.0, 120.0, 18_000.0, 0.2, 0.7),
        };
        *self = Self {
            kind,
            pre_delay_ms,
            room_scale,
            low_cut_hz,
            high_cut_hz,
            smooth,
            width,
            ..*self
        };
    }
}

/// How the echo's repeats move between the sides, TotalMix FX's echo
/// "Type".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EchoType {
    /// Each side repeats on itself.
    #[default]
    StereoEcho,
    /// The repeats cross from one side to the other.
    StereoCross,
    /// The sum of both sides bounces left, right, left.
    PongEcho,
}

impl EchoType {
    pub const ALL: [EchoType; 3] = [
        EchoType::StereoEcho,
        EchoType::StereoCross,
        EchoType::PongEcho,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EchoType::StereoEcho => "Stereo Echo",
            EchoType::StereoCross => "Stereo Cross",
            EchoType::PongEcho => "Pong Echo",
        }
    }
}

/// TotalMix FX's echo panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EchoParams {
    pub enabled: bool,
    pub kind: EchoType,
    /// Between repeats, seconds (0.01..=2.0). TotalMix shows it next to a
    /// BPM, see [`EchoParams::bpm`].
    pub delay_s: f32,
    /// How much of each repeat comes back, 0..=0.99.
    pub feedback: f32,
    /// Low-pass in the feedback loop, Hz; `None` is TotalMix's "off".
    pub high_cut_hz: Option<f32>,
    /// Stereo width of the echo, -1..=1.
    pub width: f32,
    /// Level of the echo, dB (-65..=6).
    pub volume_db: f32,
}

impl Default for EchoParams {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: EchoType::StereoEcho,
            delay_s: 0.6,
            feedback: 0.2,
            high_cut_hz: None,
            width: 0.6,
            volume_db: -3.0,
        }
    }
}

impl EchoParams {
    /// The delay as beats per minute, one beat per repeat.
    pub fn bpm(&self) -> f32 {
        60.0 / self.delay_s
    }

    /// Sets the delay from a tempo.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.delay_s = (60.0 / bpm.max(1.0)).clamp(0.01, 2.0);
    }
}

/// Both effects, what the FX section holds.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HostFx {
    pub reverb: ReverbParams,
    pub echo: EchoParams,
}

impl HostFx {
    /// Whether anything runs: with both off, the filter is not needed.
    pub fn active(&self) -> bool {
        self.reverb.enabled || self.echo.enabled
    }
}

/// The four responses, one per input side to output side: `ll` is left in
/// to left out, `lr` left in to right out, and so on.
#[derive(Debug, Clone, PartialEq)]
pub struct Responses {
    pub ll: Vec<f32>,
    pub lr: Vec<f32>,
    pub rl: Vec<f32>,
    pub rr: Vec<f32>,
}

impl Responses {
    fn silent(len: usize) -> Self {
        Self {
            ll: vec![0.0; len],
            lr: vec![0.0; len],
            rl: vec![0.0; len],
            rr: vec![0.0; len],
        }
    }

    pub fn len(&self) -> usize {
        self.ll.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ll.is_empty()
    }

    fn add(&mut self, other: &Responses) {
        let n = self.len().max(other.len());
        for (dst, src) in [
            (&mut self.ll, &other.ll),
            (&mut self.lr, &other.lr),
            (&mut self.rl, &other.rl),
            (&mut self.rr, &other.rr),
        ] {
            dst.resize(n, 0.0);
            for (d, s) in dst.iter_mut().zip(src) {
                *d += *s;
            }
        }
    }

    /// Trims the silent end, keeping at least one sample.
    fn trim(&mut self) {
        let last = [&self.ll, &self.lr, &self.rl, &self.rr]
            .iter()
            .filter_map(|v| v.iter().rposition(|x| x.abs() > 1e-6))
            .max()
            .unwrap_or(0);
        for v in [&mut self.ll, &mut self.lr, &mut self.rl, &mut self.rr] {
            v.truncate(last + 1);
        }
    }
}

/// The responses for `fx`: the reverb and the echo summed, or one silent
/// sample when both are off.
pub fn responses(fx: &HostFx) -> Responses {
    let mut out = Responses::silent(1);
    if fx.reverb.enabled {
        out.add(&reverb(&fx.reverb));
    }
    if fx.echo.enabled {
        out.add(&echo(&fx.echo));
    }
    out.trim();
    out
}

/// A small deterministic noise source, so the same settings always give
/// the same response (and the tests can pin it).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

fn db_to_gain(db: f32) -> f32 {
    if db <= -65.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// One-pole low-pass, in place.
fn low_pass(x: &mut [f32], cutoff_hz: f32) {
    let a = (-2.0 * std::f32::consts::PI * cutoff_hz / RATE as f32).exp();
    let mut y = 0.0;
    for s in x.iter_mut() {
        y = (1.0 - a) * *s + a * y;
        *s = y;
    }
}

/// One-pole high-pass, in place.
fn high_pass(x: &mut [f32], cutoff_hz: f32) {
    let a = (-2.0 * std::f32::consts::PI * cutoff_hz / RATE as f32).exp();
    let (mut px, mut py) = (0.0, 0.0);
    for s in x.iter_mut() {
        let y = a * (py + *s - px);
        px = *s;
        py = y;
        *s = y;
    }
}

/// One side of the reverb: early reflections, then a decaying noise tail.
fn reverb_side(p: &ReverbParams, seed: u64) -> Vec<f32> {
    let (rt60, er_span, er_count) = p.kind.shape();
    let scale = p.room_scale.clamp(0.5, 3.0);
    let rt60 = rt60 * scale;
    let er_span = er_span * scale;
    let pre = (p.pre_delay_ms.clamp(0.0, 999.0) / 1000.0 * RATE as f32) as usize;
    let tail = if p.kind == ReverbType::Gated {
        rt60
    } else {
        rt60 * 1.2
    };
    let len = (pre + (tail * RATE as f32) as usize).min((MAX_SECONDS * RATE as f32) as usize);
    let mut x = vec![0.0f32; len.max(1)];
    let mut noise = Noise(seed);

    // Early reflections: sparse taps over `er_span`, quieter as they come.
    for i in 0..er_count {
        let t = er_span * (i as f32 + 0.5 + 0.4 * noise.next()) / er_count as f32;
        let at = pre + (t * RATE as f32) as usize;
        if let Some(s) = x.get_mut(at) {
            *s += 0.6 * (1.0 - i as f32 / er_count as f32) * noise.next().signum();
        }
    }

    // Late reverb from where the reflections end: noise under a -60 dB per
    // `rt60` decay (a gate holds it flat and cuts it), faded in over a time
    // `smooth` sets.
    let start = pre + (er_span * RATE as f32) as usize;
    let fade_in = (p.smooth.clamp(0.0, 1.0) * 0.08 * scale * RATE as f32) as usize + 1;
    let decay = (6.908 / (rt60 * RATE as f32)).max(0.0); // ln(1000) per rt60
    for (k, s) in x.iter_mut().skip(start).enumerate() {
        let env = if p.kind == ReverbType::Gated {
            1.0
        } else {
            (-decay * k as f32).exp()
        };
        let rise = (k as f32 / fade_in as f32).min(1.0);
        *s += 0.25 * env * rise * noise.next();
    }

    high_pass(&mut x, p.low_cut_hz.clamp(20.0, 500.0));
    low_pass(&mut x, p.high_cut_hz.clamp(5_000.0, 20_000.0));
    x
}

/// Scales `x` to unit energy, so a type or a size changes the sound and not
/// the level.
fn normalize(x: &mut [f32]) {
    let e: f32 = x.iter().map(|s| s * s).sum();
    if e > 0.0 {
        let g = e.sqrt().recip();
        for s in x.iter_mut() {
            *s *= g;
        }
    }
}

/// Mixes two sides into mid/side and back at `width` (-1..=1): +1 keeps
/// them apart, 0 makes them one, -1 swaps them.
fn widen(l: &mut [f32], r: &mut [f32], width: f32) {
    let w = width.clamp(-1.0, 1.0);
    for (a, b) in l.iter_mut().zip(r.iter_mut()) {
        let (mid, side) = ((*a + *b) * 0.5, (*a - *b) * 0.5);
        *a = mid + w * side;
        *b = mid - w * side;
    }
}

/// The reverb's responses. The input is summed to mono, so both input
/// sides feed the same two decorrelated tails.
fn reverb(p: &ReverbParams) -> Responses {
    let mut l = reverb_side(p, 0x5EED_0001);
    let mut r = reverb_side(p, 0x5EED_0002);
    normalize(&mut l);
    normalize(&mut r);
    widen(&mut l, &mut r, p.width);
    // Half from each input side, at the panel's volume.
    let g = 0.5 * db_to_gain(p.volume_db.clamp(-65.0, 6.0));
    for s in l.iter_mut().chain(r.iter_mut()) {
        *s *= g;
    }
    Responses {
        ll: l.clone(),
        lr: r.clone(),
        rl: l,
        rr: r,
    }
}

/// The echo's responses: a feedback delay with a low-pass in the loop, run
/// on an impulse into each input side.
fn echo(p: &EchoParams) -> Responses {
    let d = ((p.delay_s.clamp(0.01, 2.0) * RATE as f32) as usize).max(1);
    let fb = p.feedback.clamp(0.0, 0.99);
    // Long enough for the repeats to fall 60 dB, within the cap.
    let repeats = if fb > 0.0 {
        (-6.908 / fb.ln()).ceil() as usize + 1
    } else {
        1
    };
    let len = (d * (repeats + 1)).min((MAX_SECONDS * RATE as f32) as usize);
    let cut = p
        .high_cut_hz
        .map(|hz| (-2.0 * std::f32::consts::PI * hz.clamp(1_000.0, 20_000.0) / RATE as f32).exp());

    // Runs the loop for an impulse on the left (`from_left`) or the right
    // input, giving (to left, to right).
    let run = |from_left: bool| -> (Vec<f32>, Vec<f32>) {
        let (mut yl, mut yr) = (vec![0.0f32; len], vec![0.0f32; len]);
        let (mut lpl, mut lpr) = (0.0f32, 0.0f32);
        for n in 0..len {
            let x = if n == d { 1.0 } else { 0.0 };
            let (fl, fr) = if n >= d {
                (yl[n - d], yr[n - d])
            } else {
                (0.0, 0.0)
            };
            // The repeats through the loop's low-pass, if any.
            let (fl, fr) = match cut {
                Some(a) => {
                    lpl = (1.0 - a) * fl + a * lpl;
                    lpr = (1.0 - a) * fr + a * lpr;
                    (lpl, lpr)
                }
                None => (fl, fr),
            };
            match p.kind {
                EchoType::StereoEcho => {
                    yl[n] = if from_left { x } else { 0.0 } + fb * fl;
                    yr[n] = if from_left { 0.0 } else { x } + fb * fr;
                }
                EchoType::StereoCross => {
                    yl[n] = if from_left { x } else { 0.0 } + fb * fr;
                    yr[n] = if from_left { 0.0 } else { x } + fb * fl;
                }
                EchoType::PongEcho => {
                    // Both sides start on the left, half each.
                    yl[n] = 0.5 * x + fb * fr;
                    yr[n] = fb * fl;
                }
            }
        }
        (yl, yr)
    };
    let (mut ll, mut lr) = run(true);
    let (mut rl, mut rr) = run(false);
    widen(&mut ll, &mut lr, p.width);
    widen(&mut rl, &mut rr, p.width);
    let g = db_to_gain(p.volume_db.clamp(-65.0, 6.0));
    for v in [&mut ll, &mut lr, &mut rl, &mut rr] {
        for s in v.iter_mut() {
            *s *= g;
        }
    }
    Responses { ll, lr, rl, rr }
}

/// The responses as a 4-channel 32-bit float WAV (LL, LR, RL, RR), the
/// file the convolvers read.
pub fn wav(r: &Responses) -> Vec<u8> {
    let frames = r.len() as u32;
    let channels = 4u16;
    let data_len = frames * channels as u32 * 4;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * channels as u32 * 4).to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..r.len() {
        for v in [&r.ll, &r.lr, &r.rl, &r.rr] {
            out.extend_from_slice(&v[i].to_le_bytes());
        }
    }
    out
}

/// The PipeWire config that runs the effect: a filter-chain from the
/// capture node's channels 13/14 (AUX12/AUX13) to the playback node's,
/// through the four convolvers of `ir_path` (see [`wav`]).
///
/// The capture side must not be passive: nothing else may be reading the
/// card's input, and a passive stream does not wake a suspended node, so
/// the filter would read silence.
pub fn filter_chain_conf(ir_path: &str, capture_node: &str, playback_node: &str) -> String {
    let mut c = String::new();
    let conv = |name: &str, ch: u32| {
        format!(
            "{{ type = builtin name = {name} label = convolver config = {{ filename = \"{ir_path}\" channel = {ch} }} }}"
        )
    };
    let _ = writeln!(c, "# Written by TuxMix: the host FX of a Babyface Pro FS.");
    let _ = writeln!(c, "context.properties = {{ log.level = 0 }}");
    let _ = writeln!(c, "context.spa-libs = {{");
    let _ = writeln!(c, "    audio.convert.* = audioconvert/libspa-audioconvert");
    let _ = writeln!(c, "    support.*       = support/libspa-support");
    let _ = writeln!(c, "}}");
    let _ = writeln!(c, "context.modules = [");
    let _ = writeln!(
        c,
        "    {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}"
    );
    let _ = writeln!(c, "    {{ name = libpipewire-module-protocol-native }}");
    let _ = writeln!(c, "    {{ name = libpipewire-module-client-node }}");
    let _ = writeln!(c, "    {{ name = libpipewire-module-adapter }}");
    let _ = writeln!(c, "    {{ name = libpipewire-module-filter-chain");
    let _ = writeln!(c, "        args = {{");
    let _ = writeln!(c, "            node.description = \"TuxMix FX\"");
    let _ = writeln!(c, "            media.name = \"TuxMix FX\"");
    let _ = writeln!(c, "            filter.graph = {{");
    let _ = writeln!(c, "                nodes = [");
    let _ = writeln!(
        c,
        "                    {{ type = builtin name = inl label = copy }}"
    );
    let _ = writeln!(
        c,
        "                    {{ type = builtin name = inr label = copy }}"
    );
    for (name, ch) in [("ll", 0), ("lr", 1), ("rl", 2), ("rr", 3)] {
        let _ = writeln!(c, "                    {}", conv(name, ch));
    }
    let _ = writeln!(
        c,
        "                    {{ type = builtin name = outl label = mixer }}"
    );
    let _ = writeln!(
        c,
        "                    {{ type = builtin name = outr label = mixer }}"
    );
    let _ = writeln!(c, "                ]");
    let _ = writeln!(c, "                links = [");
    for (from, to) in [
        ("inl:Out", "ll:In"),
        ("inl:Out", "lr:In"),
        ("inr:Out", "rl:In"),
        ("inr:Out", "rr:In"),
        ("ll:Out", "outl:In 1"),
        ("rl:Out", "outl:In 2"),
        ("lr:Out", "outr:In 1"),
        ("rr:Out", "outr:In 2"),
    ] {
        let _ = writeln!(
            c,
            "                    {{ output = \"{from}\" input = \"{to}\" }}"
        );
    }
    let _ = writeln!(c, "                ]");
    let _ = writeln!(c, "                inputs = [ \"inl:In\" \"inr:In\" ]");
    let _ = writeln!(c, "                outputs = [ \"outl:Out\" \"outr:Out\" ]");
    let _ = writeln!(c, "            }}");
    let _ = writeln!(c, "            capture.props = {{");
    let _ = writeln!(c, "                node.name = \"tuxmix_fx_in\"");
    let _ = writeln!(c, "                target.object = \"{capture_node}\"");
    let _ = writeln!(c, "                audio.position = [ AUX12 AUX13 ]");
    let _ = writeln!(c, "                stream.dont-remix = true");
    let _ = writeln!(c, "            }}");
    let _ = writeln!(c, "            playback.props = {{");
    let _ = writeln!(c, "                node.name = \"tuxmix_fx_out\"");
    let _ = writeln!(c, "                target.object = \"{playback_node}\"");
    let _ = writeln!(c, "                audio.position = [ AUX12 AUX13 ]");
    let _ = writeln!(c, "                stream.dont-remix = true");
    let _ = writeln!(c, "                node.passive = true");
    let _ = writeln!(c, "            }}");
    let _ = writeln!(c, "        }}");
    let _ = writeln!(c, "    }}");
    let _ = writeln!(c, "]");
    c
}

/// Runs the filter-chain for the current settings: writes the response and
/// the config, and starts `pipewire -c` on them as a child process. A
/// convolver reads its response once, so a change of settings restarts it,
/// with a short gap in the effect.
///
/// The effect goes back through the card's PipeWire sink, whose volume
/// scales it: with the sink at 80 %, the effect came out 5.8 dB low on the
/// unit, even with channels 13/14 at 100 %. At 100 % the levels are
/// TotalMix's (an echo at -3 dB came back 3 dB under the FX bus).
///
/// The child's pid is kept in the runtime directory: a TuxMix that crashed
/// leaves its filter running, and the next one stops it before starting its
/// own, so two filters never add their effects.
pub struct Runner {
    dir: std::path::PathBuf,
    child: Option<std::process::Child>,
    running: Option<HostFx>,
    generation: u64,
}

/// Why the filter could not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The card's pro-audio nodes are missing or have 12 channels: the
    /// effect needs kernel driver 0.2.5 and PipeWire's Pro Audio profile.
    NoFxChannels,
    /// Writing the files or starting `pipewire` failed.
    Io(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::NoFxChannels => write!(
                f,
                "No FX channels: reverb and echo need kernel driver 0.2.5 and the Pro Audio profile"
            ),
            RunError::Io(e) => write!(f, "Host FX: {e}"),
        }
    }
}

const PID_FILE: &str = "fx.pid";
const CONF_STEM: &str = "tuxmix-fx";

impl Runner {
    /// A runner writing to `$XDG_RUNTIME_DIR/tuxmix`, after stopping a
    /// filter an earlier TuxMix left there.
    pub fn new() -> Self {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Self::in_dir(base.join("tuxmix"))
    }

    /// A runner writing to `dir`.
    pub fn in_dir(dir: std::path::PathBuf) -> Self {
        let runner = Self {
            dir,
            child: None,
            running: None,
            generation: 0,
        };
        runner.stop_stale();
        runner
    }

    /// The settings the filter runs now, if it runs.
    pub fn running(&self) -> Option<&HostFx> {
        self.running.as_ref()
    }

    /// Runs `fx`, or stops the filter when nothing is on. Does nothing when
    /// these settings already run. `nodes` are the capture and playback
    /// node names (see [`crate::pw_meter::fx_nodes`]); `None` asks
    /// PipeWire.
    pub fn apply(&mut self, fx: &HostFx, nodes: Option<(String, String)>) -> Result<(), RunError> {
        if !fx.active() {
            self.stop();
            return Ok(());
        }
        if self.running.as_ref() == Some(fx) && self.alive() {
            return Ok(());
        }
        let (capture, playback) = nodes
            .or_else(crate::pw_meter::fx_nodes)
            .ok_or(RunError::NoFxChannels)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| RunError::Io(e.to_string()))?;
        // A new response file each time: the old filter may still read its
        // own while this one starts.
        self.generation += 1;
        let ir = self
            .dir
            .join(format!("{CONF_STEM}-{}.wav", self.generation));
        let conf = self
            .dir
            .join(format!("{CONF_STEM}-{}.conf", self.generation));
        std::fs::write(&ir, wav(&responses(fx))).map_err(|e| RunError::Io(e.to_string()))?;
        let text = filter_chain_conf(&ir.to_string_lossy(), &capture, &playback);
        std::fs::write(&conf, text).map_err(|e| RunError::Io(e.to_string()))?;
        self.stop();
        let mut cmd = std::process::Command::new("pipewire");
        cmd.arg("-c").arg(&conf);
        // The filter dies with TuxMix, even when TuxMix is killed: an
        // effect nothing can turn off any more is worse than none.
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: prctl(2) is async-signal-safe, which is all pre_exec
            // asks of the closure.
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        let child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| RunError::Io(format!("cannot start pipewire: {e}")))?;
        let _ = std::fs::write(self.dir.join(PID_FILE), child.id().to_string());
        self.child = Some(child);
        self.running = Some(*fx);
        self.clean_files();
        Ok(())
    }

    /// Stops the filter, if it runs.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.running = None;
        let _ = std::fs::remove_file(self.dir.join(PID_FILE));
    }

    /// Whether the child still runs: `pipewire` exits when PipeWire goes
    /// away, and the next [`Self::apply`] starts it again.
    pub fn alive(&mut self) -> bool {
        match self.child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                false
            }
            None => false,
        }
    }

    /// Stops a filter whose pid an earlier TuxMix left, if that process
    /// still runs one of our configs.
    fn stop_stale(&self) {
        let Ok(text) = std::fs::read_to_string(self.dir.join(PID_FILE)) else {
            return;
        };
        let Ok(pid) = text.trim().parse::<i32>() else {
            return;
        };
        let ours = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|c| String::from_utf8_lossy(&c).contains(CONF_STEM))
            .unwrap_or(false);
        if ours && pid > 0 {
            // SAFETY: plain kill(2) on a pid we checked runs our config.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        let _ = std::fs::remove_file(self.dir.join(PID_FILE));
    }

    /// Removes the files of earlier generations.
    fn clean_files(&self) {
        let keep = format!("{CONF_STEM}-{}.", self.generation);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(CONF_STEM) && !name.starts_with(&keep) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.stop();
        self.clean_files();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_at(x: &[f32]) -> usize {
        x.iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tuxmix-fx-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn nothing_on_runs_nothing() {
        let dir = scratch("off");
        let mut r = Runner::in_dir(dir.clone());
        assert_eq!(r.apply(&HostFx::default(), None), Ok(()));
        assert!(r.running().is_none());
        assert!(!r.alive());
        assert!(!dir.join(PID_FILE).exists());
    }

    #[test]
    fn a_stale_pid_that_is_not_ours_is_left_alone() {
        let dir = scratch("stale");
        std::fs::create_dir_all(&dir).unwrap();
        // Our own pid runs the test binary, not a filter: never killed.
        std::fs::write(dir.join(PID_FILE), std::process::id().to_string()).unwrap();
        let _r = Runner::in_dir(dir.clone());
        assert!(!dir.join(PID_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_on_is_one_silent_sample() {
        let r = responses(&HostFx::default());
        assert_eq!(r.len(), 1);
        assert!([&r.ll, &r.lr, &r.rl, &r.rr].iter().all(|v| v[0] == 0.0));
        assert!(!HostFx::default().active());
    }

    #[test]
    fn the_same_settings_give_the_same_response() {
        let mut fx = HostFx::default();
        fx.reverb.enabled = true;
        assert_eq!(responses(&fx), responses(&fx));
    }

    #[test]
    fn the_reverb_starts_after_its_pre_delay_and_decays() {
        let mut p = ReverbParams {
            enabled: true,
            pre_delay_ms: 100.0,
            ..Default::default()
        };
        let r = reverb(&p);
        let first = r.ll.iter().position(|s| s.abs() > 1e-4).unwrap();
        // Nothing before the pre delay, and the first reflection within
        // the room's reflection span after it (60 ms for a large room).
        assert!((4800..4800 + 2880).contains(&first), "{first}");
        // The second half holds far less energy than the first.
        let e = |x: &[f32]| x.iter().map(|s| s * s).sum::<f32>();
        let half = r.ll.len() / 2;
        assert!(e(&r.ll[half..]) < 0.05 * e(&r.ll[..half]));
        // A bigger room rings longer.
        let short = reverb(&p).ll.len();
        p.room_scale = 2.0;
        assert!(reverb(&p).ll.len() > short);
    }

    #[test]
    fn the_reverb_level_follows_the_volume_not_the_room() {
        let e = |r: &Responses| r.ll.iter().map(|s| s * s).sum::<f32>();
        let mut p = ReverbParams {
            enabled: true,
            ..Default::default()
        };
        let base = e(&reverb(&p));
        // Another type or size: about the same energy.
        p.kind = ReverbType::Hall;
        p.room_scale = 2.0;
        let hall = e(&reverb(&p));
        assert!((hall / base - 1.0).abs() < 0.3, "{}", hall / base);
        // 6 dB down is a quarter of the energy.
        p.volume_db -= 6.0;
        let ratio = e(&reverb(&p)) / hall;
        assert!((ratio - 0.251).abs() < 0.002, "{ratio}");
    }

    #[test]
    fn width_zero_makes_the_reverb_mono_and_minus_one_swaps_it() {
        let mut p = ReverbParams {
            enabled: true,
            width: 0.0,
            ..Default::default()
        };
        let r = reverb(&p);
        assert_eq!(r.ll, r.lr);
        p.width = 1.0;
        let wide = reverb(&p);
        p.width = -1.0;
        let swapped = reverb(&p);
        assert_eq!(wide.ll, swapped.lr);
        assert_eq!(wide.lr, swapped.ll);
    }

    #[test]
    fn a_stereo_echo_repeats_on_its_own_side_and_fades() {
        let p = EchoParams {
            enabled: true,
            delay_s: 0.25,
            feedback: 0.5,
            width: 1.0,
            volume_db: 0.0,
            ..Default::default()
        };
        let r = echo(&p);
        let d = 12_000;
        assert_eq!(r.ll[d], 1.0);
        assert_eq!(r.ll[2 * d], 0.5);
        assert_eq!(r.ll[3 * d], 0.25);
        assert!(r.lr.iter().all(|s| *s == 0.0), "nothing crosses");
        assert_eq!(r.rr, r.ll);
    }

    #[test]
    fn a_cross_echo_alternates_sides_and_a_pong_starts_left() {
        let mut p = EchoParams {
            enabled: true,
            kind: EchoType::StereoCross,
            delay_s: 0.25,
            feedback: 0.5,
            width: 1.0,
            volume_db: 0.0,
            ..Default::default()
        };
        let d = 12_000;
        let r = echo(&p);
        assert_eq!((r.ll[d], r.lr[d]), (1.0, 0.0));
        assert_eq!((r.ll[2 * d], r.lr[2 * d]), (0.0, 0.5));
        assert_eq!((r.ll[3 * d], r.lr[3 * d]), (0.25, 0.0));
        p.kind = EchoType::PongEcho;
        let r = echo(&p);
        // Either input side bounces the same way, starting left.
        assert_eq!((r.ll[d], r.lr[d]), (0.5, 0.0));
        assert_eq!((r.ll[2 * d], r.lr[2 * d]), (0.0, 0.25));
        assert_eq!(r.rl, r.ll);
        assert_eq!(r.rr, r.lr);
    }

    #[test]
    fn the_echo_high_cut_softens_each_repeat_more() {
        let p = EchoParams {
            enabled: true,
            delay_s: 0.1,
            feedback: 0.9,
            high_cut_hz: Some(2_000.0),
            width: 1.0,
            volume_db: 0.0,
            ..Default::default()
        };
        let r = echo(&p);
        let d = 4_800;
        // The first repeat is the dry impulse; later ones spread out, so
        // their peak drops faster than the feedback alone.
        assert_eq!(peak_at(&r.ll[..d + 10]), d);
        let peak = |k: usize| {
            r.ll[k * d..(k + 1) * d]
                .iter()
                .fold(0f32, |m, s| m.max(s.abs()))
        };
        assert!(peak(3) < 0.9 * 0.9 * peak(1) * 0.9);
    }

    #[test]
    fn bpm_and_delay_go_together() {
        let mut p = EchoParams::default();
        assert!((p.bpm() - 100.0).abs() < 0.01);
        p.set_bpm(120.0);
        assert!((p.delay_s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn responses_stay_within_the_cap() {
        let fx = HostFx {
            reverb: ReverbParams {
                enabled: true,
                kind: ReverbType::Space,
                room_scale: 3.0,
                pre_delay_ms: 999.0,
                ..Default::default()
            },
            echo: EchoParams {
                enabled: true,
                delay_s: 2.0,
                feedback: 0.99,
                ..Default::default()
            },
        };
        assert!(responses(&fx).len() <= (MAX_SECONDS * RATE as f32) as usize);
    }

    #[test]
    fn the_wav_is_four_float_channels() {
        let mut fx = HostFx::default();
        fx.echo.enabled = true;
        let r = responses(&fx);
        let w = wav(&r);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([w[20], w[21]]), 3);
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 4);
        assert_eq!(w.len(), 44 + r.len() * 16);
        let first = f32::from_le_bytes(w[44..48].try_into().unwrap());
        assert_eq!(first, r.ll[0]);
    }

    #[test]
    fn the_conf_wires_channels_13_and_14_through_four_convolvers() {
        let c = filter_chain_conf("/tmp/ir.wav", "in-node", "out-node");
        assert_eq!(c.matches("label = convolver").count(), 4);
        assert!(c.contains("channel = 3"));
        assert!(c.contains("target.object = \"in-node\""));
        assert!(c.contains("target.object = \"out-node\""));
        assert_eq!(c.matches("audio.position = [ AUX12 AUX13 ]").count(), 2);
        // Only the playback side is passive (see the doc comment).
        assert_eq!(c.matches("node.passive").count(), 1);
        let i = c.find("playback.props").unwrap();
        assert!(c[i..].contains("node.passive = true"));
    }
    #[test]
    fn a_reverb_type_loads_its_settings() {
        let mut r = ReverbParams {
            enabled: true,
            volume_db: -10.0,
            ..ReverbParams::default()
        };
        r.load_type(ReverbType::Space);
        assert_eq!(
            (r.kind, r.pre_delay_ms, r.room_scale),
            (ReverbType::Space, 80.0, 1.5)
        );
        assert!(r.enabled);
        assert_eq!(r.volume_db, -10.0);
        r.load_type(ReverbType::LargeRoom);
        assert_eq!(
            r,
            ReverbParams {
                enabled: true,
                volume_db: -10.0,
                ..ReverbParams::default()
            }
        );
    }
}
