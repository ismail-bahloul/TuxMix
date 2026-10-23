//! Babyface Pro (FS) implementation of the [`RmeDevice`] trait.

use alsa::mixer::{MilliBel, Selem, SelemChannelId};
use alsa::Round;
use log::info;

use crate::channel::OutputChannel;
use crate::channel::*;
use crate::curves::{fader_db_to_raw, fader_raw_to_db};
use crate::device::{DeviceSettings, RmeDevice};
use crate::error::Error;
use crate::mixer::AlsaMixer;
use crate::profile::DeviceProfile;
use crate::profiles::babyface_pro::PROFILE;
use crate::scene::Scene;

// ── Helpers ────────────────────────────────────────────────────
//
// The ALSA control names/indices below were cross-checked 2026-08-28
// against the real kernel driver (`babyface-pro-linux/tools/kernel/
// mixer.c`) and live `amixer -c FS scontents` output on real hardware
// — not guessed. The previous grammar this file used
// (`"<Type>-<Name>-<Output>"`, `" 48V"`/`" PAD"`/`" Sens."` suffixes)
// never matched anything the kernel driver actually creates, so every
// crosspoint/phantom/pad/gain read-write was a silent no-op.

/// One entry per `mixer.c`'s `bf_sources[14]` crosspoint source, in the
/// same order (source index = position in this array = `src` in the
/// kernel's `index = out*14 + src`). The first 4 (AN1-AN4) are true
/// mono sources: their control's front-left/front-right ALSA channels
/// write the source's level into the output pair's left/right side, so
/// TotalMix-style volume+pan applies (see `input_crosspoint_slot`). The
/// other 10 are hardware-linked stereo pairs (AS1/2, the ADAT pairs,
/// PB1-PB6): front-left/front-right are each physical channel's OWN
/// level into the output pair, with no cross-routing — there is no
/// ALSA-level way to route e.g. AS1 into an output's right side, so pan
/// does not apply to those.
const BF_SOURCES: [&str; 14] = [
    "AN1", "AN2", "AN3", "AN4", "AS1/2", "ADAT3/4", "ADAT5/6", "ADAT7/8", "PB1", "PB2", "PB3",
    "PB4", "PB5", "PB6",
];

/// `mixer.c`'s `out_names[6]` — the master-volume/mute ALSA control
/// name per output pair, in `self.profile.outputs` order (both use the
/// canonical AN1/2, PH3/4, ... order — hardware-verified in `mixer.c`
/// not to need reordering).
const PAIR_LABELS: [&str; 6] = ["AN1/2", "PH3/4", "AS1/2", "ADAT3/4", "ADAT5/6", "ADAT7/8"];

/// Which ALSA control grammar the attached card speaks.
///
/// The Babyface Pro FS presents two USB personalities and they expose
/// *structurally different* mixers, not just differently-named ones:
///
/// |                    | [`Proprietary`](ControlGrammar::Proprietary) | [`ClassCompliant`](ControlGrammar::ClassCompliant) |
/// |---|---|---|
/// | crosspoints        | 84 = 14 sources × 6 output **pairs**         | 288 = 24 sources × 12 destination **channels** |
/// | disambiguated by   | selem `.index` (`output * 14 + src`)         | the name itself; `.index` is always 0 |
/// | a mono source's pan| two channels of one stereo selem             | two separate mono controls |
///
/// Verified by dumping both surfaces from the same physical card
/// (`docs/reference/class-compliant-controls.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlGrammar {
    /// The from-scratch `snd-usb-babyface-pro` driver (USB `2a39:3fc0`).
    /// The full feature set: EQ, front panel, phase, trim, loopback…
    Proprietary,
    /// Stock `snd-usb-audio` on the class-compliant personality (USB
    /// `2a39:3fb0`). Routing, preamps and clock work; the DSP features
    /// that live in RME's own protocol do not exist here.
    ClassCompliant,
}

/// A crosspoint source's name in the class-compliant grammar, for
/// `BF_SOURCES[src]` and — for the ten paired sources — which side.
///
/// The pairing is the same one [`input_crosspoint_slot`] already
/// encodes for the proprietary grammar; only the spelling differs.
fn cc_source_name(src: usize, right: bool) -> Option<&'static str> {
    Some(match (src, right) {
        (0, _) => "Mic-AN1",
        (1, _) => "Mic-AN2",
        (2, _) => "Line-IN3",
        (3, _) => "Line-IN4",
        (4, false) => "Line-AS1",
        (4, true) => "Line-AS2",
        (5, false) => "Line-ADAT3",
        (5, true) => "Line-ADAT4",
        (6, false) => "Line-ADAT5",
        (6, true) => "Line-ADAT6",
        (7, false) => "Line-ADAT7",
        (7, true) => "Line-ADAT8",
        (8, false) => "PCM-AN1",
        (8, true) => "PCM-AN2",
        (9, false) => "PCM-PH3",
        (9, true) => "PCM-PH4",
        (10, false) => "PCM-AS1",
        (10, true) => "PCM-AS2",
        (11, false) => "PCM-ADAT3",
        (11, true) => "PCM-ADAT4",
        (12, false) => "PCM-ADAT5",
        (12, true) => "PCM-ADAT6",
        (13, false) => "PCM-ADAT7",
        (13, true) => "PCM-ADAT8",
        _ => return None,
    })
}

/// A destination channel's name in the class-compliant grammar, for
/// output pair `out` (same order as [`PAIR_LABELS`]) and side.
fn cc_dest_name(out: usize, right: bool) -> Option<&'static str> {
    Some(match (out, right) {
        (0, false) => "AN1",
        (0, true) => "AN2",
        (1, false) => "PH3",
        (1, true) => "PH4",
        (2, false) => "AS1",
        (2, true) => "AS2",
        (3, false) => "ADAT3",
        (3, true) => "ADAT4",
        (4, false) => "ADAT5",
        (4, true) => "ADAT6",
        (5, false) => "ADAT7",
        (5, true) => "ADAT8",
        _ => return None,
    })
}

/// Maps a `PROFILE.inputs` index to its crosspoint source: which of
/// `BF_SOURCES` it is, and — for the non-mono (linked-pair) sources —
/// which single ALSA channel (front-left/front-right) is *this*
/// input's own level, since the pair's other channel belongs to the
/// sibling input (e.g. `AS1` is `BF_SOURCES[4]` front-left, `AS2` is
/// the same source's front-right).
fn input_crosspoint_slot(idx: usize) -> Option<(usize, SelemChannelId, bool)> {
    use SelemChannelId::{FrontLeft as L, FrontRight as R};
    Some(match idx {
        0 => (0, L, true),  // AN1 (Mic 1)
        1 => (1, L, true),  // AN2 (Mic 2)
        2 => (2, L, true),  // IN3 -> hardware source "AN3"
        3 => (3, L, true),  // IN4 -> hardware source "AN4"
        4 => (4, L, false), // AS1
        5 => (4, R, false), // AS2
        6 => (5, L, false), // ADAT3
        7 => (5, R, false), // ADAT4
        8 => (6, L, false), // ADAT5
        9 => (6, R, false), // ADAT6
        10 => (7, L, false), // ADAT7
        11 => (7, R, false), // ADAT8
        _ => return None,
    })
}

/// Same idea for `PROFILE.outputs`-derived playback channels: each of
/// the 6 `PBn` sources is one playback pair (`PCM <left>`/`PCM
/// <right>`), always linked (no mono PB source exists).
fn playback_crosspoint_slot(idx: usize) -> Option<(usize, SelemChannelId)> {
    if idx >= 12 {
        return None;
    }
    let src = 8 + idx / 2;
    let ch = if idx % 2 == 0 {
        SelemChannelId::FrontLeft
    } else {
        SelemChannelId::FrontRight
    };
    Some((src, ch))
}

/// EQ strips exist only on the 4 analog inputs (`eq.c`'s `names[4] =
/// {"AN1","AN2","AN3","AN4"}`), which happen to be exactly
/// `BF_SOURCES[0..4]`.
fn eq_strip_name(idx: usize) -> Option<&'static str> {
    if idx < 4 {
        Some(BF_SOURCES[idx])
    } else {
        None
    }
}

/// `eq.c`'s `bf_eq_type_texts` enum-item order: Off/Bell/Low Shelf/High Shelf.
fn eq_band_type_to_enum(t: EqBandType) -> u32 {
    match t {
        EqBandType::Off => 0,
        EqBandType::Bell => 1,
        EqBandType::LowShelf => 2,
        EqBandType::HighShelf => 3,
    }
}

fn eq_band_type_from_enum(v: u32) -> EqBandType {
    match v {
        1 => EqBandType::Bell,
        2 => EqBandType::LowShelf,
        3 => EqBandType::HighShelf,
        _ => EqBandType::Off,
    }
}

/// `eq.c`'s `bf_eq_slope_texts` enum-item order: 6/12/18/24 dB/oct.
fn eq_slope_to_enum(slope_db_oct: u8) -> u32 {
    match slope_db_oct {
        12 => 1,
        18 => 2,
        24 => 3,
        _ => 0, // 6 dB/oct, and the fallback for any other value
    }
}

fn eq_slope_from_enum(v: u32) -> u8 {
    match v {
        1 => 12,
        2 => 18,
        3 => 24,
        _ => 6,
    }
}

/// A mono input's route into a stereo output pair is really *two*
/// independent ALSA volumes (one per crosspoint channel) — there is no
/// separate "pan" control on the hardware. TotalMix's single
/// volume+pan fader is a UI convenience over that pair, so this
/// decodes raw front-left/front-right volumes into the same
/// convention: the louder side sets volume, and how far the quieter
/// side is attenuated sets pan. `max` is the ALSA control's own
/// reported full-scale (`get_playback_volume_range().1`) — the real
/// hardware ranges differ per control (crosspoints top out at 0x2d41,
/// master volume at 0x4000), so this must never be hardcoded.
/// `encode_volume_pan` is the exact inverse, so round-tripping through
/// both is lossless (mod integer rounding).
fn decode_volume_pan(l_raw: i64, r_raw: i64, max: f32) -> (f32, i8) {
    pan_join(l_raw as f32 / max, r_raw as f32 / max)
}

/// The linear left and right levels of a mono source at `volume` and
/// `pan` (-100 = left only, +100 = right only): the louder side carries the
/// volume and the other is attenuated by the pan.
fn pan_split(volume: f32, pan: i8) -> (f32, f32) {
    let p = (pan.clamp(-100, 100) as f32) / 100.0;
    if p <= 0.0 {
        (volume, volume * (1.0 + p))
    } else {
        (volume * (1.0 - p), volume)
    }
}

/// The inverse of [`pan_split`].
fn pan_join(l: f32, r: f32) -> (f32, i8) {
    let volume = l.max(r);
    let pan = if volume > 0.0 {
        (((r - l) / volume) * 100.0).round().clamp(-100.0, 100.0) as i8
    } else {
        0
    };
    (volume, pan)
}

/// A level control's value as the model's linear volume (1.0 = 0 dB,
/// 2.0 = +6 dB), read through the control's own dB scale (its TLV) rather
/// than as a share of its raw range.  The driver's raw ranges are not all
/// amplitude scales: the output masters count the hardware's 0.5 dB steps.
/// The bottom of the range is silence.
fn tlv_volume(selem: &Selem, ch: SelemChannelId) -> Option<f32> {
    let raw = selem.get_playback_volume(ch).ok()?;
    if raw <= selem.get_playback_volume_range().0 {
        return Some(0.0);
    }
    let db = selem.ask_playback_vol_db(raw).ok()?.to_db();
    Some(10f32.powf(db / 20.0))
}

/// The raw value that holds the linear `volume`, through the control's dB
/// scale, rounded down so a level never lands louder than asked.
fn tlv_raw(selem: &Selem, volume: f32) -> alsa::Result<i64> {
    if volume <= 0.0 {
        return Ok(selem.get_playback_volume_range().0);
    }
    let mb = (2000.0 * volume.min(2.0).log10()).floor() as i64;
    selem.ask_playback_db_vol(MilliBel(mb), Round::Floor)
}

/// Sets one channel of a level control to the linear `volume`.
fn set_tlv_volume(selem: &Selem, ch: SelemChannelId, volume: f32) -> alsa::Result<()> {
    selem.set_playback_volume(ch, tlv_raw(selem, volume)?)
}

/// Whether writing the model's `volume` would leave the raw value the card
/// holds - the re-read test for a dB-converted control.  A write rounds to
/// the control's grid, so the card's level can sit up to one step below the
/// model's, and a re-read must not move a fader the user is holding there.
fn tlv_matches(selem: &Selem, ch: SelemChannelId, volume: f32) -> bool {
    match (tlv_raw(selem, volume), selem.get_playback_volume(ch)) {
        (Ok(want), Ok(have)) => want == have,
        _ => false,
    }
}

fn encode_volume_pan(volume: f32, pan: i8, max: f32) -> (i64, i64) {
    let (l, r) = pan_split(volume.clamp(0.0, 1.0), pan);
    ((l * max) as i64, (r * max) as i64)
}

// ── Following the driver ───────────────────────────────────────
//
// The kernel driver changes controls on its own: the front-panel wheel
// moves an output master, a preamp gain or a monitoring crosspoint, SET
// toggles 48V, and `alsactl restore` or another mixer can write any of
// them.  `poll_events` re-reads the model from the card whenever ALSA
// reports a change, so the GUI shows what the hardware really holds.

/// How long after the last ALSA event to read the card once more.  The
/// driver notifies the front-panel controls on every wheel click, but not
/// every control a click moves (a gain or a crosspoint), and the notify
/// can come before the write it reports - one more read after the burst
/// catches what the events themselves did not carry.
const RESYNC_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// Largest difference between the model and the card that still counts as
/// "the same value": the model keeps the float the UI asked for, the card
/// the integer it was rounded to, so a re-read must not nudge a fader the
/// user is holding.  Below one step of the finest control (a crosspoint,
/// full scale 0x2d41).
const RESYNC_EPSILON: f32 = 1.0 / 8192.0;

/// Whether a value read back from the card differs from the model by more
/// than the rounding a write introduces.
fn resync_differs(model: f32, card: f32) -> bool {
    (model - card).abs() > RESYNC_EPSILON
}

/// The driver's "Front Panel In"/"Front Panel Out" enum item (0 = not
/// known yet, 1..=3 = the three positions) as the 0-based selection the
/// USB backend reports: IN 0 = Ch 1/2, 1 = Ch 3/4, 2 = Opt; OUT 0 = Ch
/// 1/2, 1 = Phones, 2 = Opt.
fn panel_enum_to_sel(item: u32) -> Option<usize> {
    match item {
        1..=3 => Some(item as usize - 1),
        _ => None,
    }
}

/// Whether the DIM presses counted between two reads of the driver's
/// "DIM Button Press Count" leave DIM toggled: an odd number of presses
/// does, an even one cancels out.  The count wraps at 2^31.
fn dim_presses_toggle(prev: i64, now: i64) -> bool {
    let delta = (now - prev).rem_euclid(1 << 31);
    delta % 2 == 1
}

// ── Main struct ────────────────────────────────────────────────

/// Babyface Pro (FS) device controller.
pub struct BabyfacePro {
    mixer: AlsaMixer,
    /// Which control grammar this card speaks — decided once at
    /// [`RmeDevice::open`] and never re-checked, since a card cannot
    /// change personality without re-enumerating (the PC/CC toggle is
    /// a power-on button combo, see `PROTOCOL.md`).
    grammar: ControlGrammar,
    /// The kernel's enumeration number of the USB device this mixer was
    /// opened on (see `AlsaMixer::card_devnum`): a re-created card comes
    /// back on the same ALSA index with the same names, so this is what
    /// `is_connected` has to tell "still ours" apart with.
    card_devnum: Option<String>,
    profile: &'static DeviceProfile,
    inputs: Vec<InputChannel>,
    playbacks: Vec<PlaybackChannel>,
    outputs: Vec<OutputChannel>,
    settings: DeviceSettings,
    /// AN1/2 input-strip stereo link state (`"AN1/2 Link"`).
    /// Not exposed via [`DeviceSettings`] — matches the USB backend,
    /// which also keeps this outside `DeviceSettings` (no getter exists
    /// in [`RmeDevice`] for it, only `set_input_link`).
    linked: bool,
    /// Real-time input level metering for every hardware input — `None`
    /// if the capture PCM couldn't be opened (metering is a nice-to-have,
    /// not something that should keep the rest of the device from
    /// working). See `capture_meter`'s module doc comment.
    capture_meter: Option<crate::capture_meter::CaptureMeter>,
    /// The driver's "DIM Button Press Count" as last seen, `None` when the
    /// control does not exist (an older driver that applied DIM itself,
    /// or the class-compliant mode).  Since `babyface-pro-linux` 1597d28
    /// the driver only counts presses; acting on them is the mixer
    /// application's job, as it is TotalMix's on Windows.
    dim_press_count: Option<i64>,
    /// Front-panel state from the driver's "Front Panel *" controls:
    /// (MIX engaged, IN selection, OUT selection), see
    /// [`BabyfacePro::panel_selection`].
    panel: Option<(bool, usize, usize)>,
    /// When to re-read the card once more after a burst of ALSA events,
    /// see [`RESYNC_SETTLE`].
    resync_at: Option<std::time::Instant>,
}

impl BabyfacePro {
    /// Whether the capture stream behind [`RmeDevice::meters`] opened at
    /// all. Per-channel availability is still exposed separately (see
    /// `capture_meter`'s module doc comment); this only answers "could
    /// we meter anything", so a failed PCM open does not read as a bank
    /// of silent-but-live inputs.
    pub fn meters_available(&self) -> bool {
        self.capture_meter.is_some()
    }

    /// How many times the input capture meter has reopened its PCM
    /// after a stream error (a PipeWire profile change, a re-probe).
    /// Diagnostic only; the meter recovers on its own.
    pub fn meter_reconnects(&self) -> usize {
        self.capture_meter
            .as_ref()
            .map_or(0, |cm| cm.reconnects())
    }

    /// Look up a crosspoint's ALSA element: `BF_SOURCES[src]` at
    /// `.index = output*14 + src`, exactly `mixer.c`'s
    /// `babyface_create_xpoints` layout.
    fn crosspoint_selem(&self, src: usize, output: usize) -> Option<Selem<'_>> {
        self.mixer
            .find_selem(BF_SOURCES[src], (output * 14 + src) as u32)
    }

    /// Writes `volume` (0.0..=1.0) straight to a crosspoint's ALSA
    /// element, preserving `pan` for the 4 true-mono sources — the
    /// same encoding `set_volume`/`set_pan` use, but without touching
    /// the stored `self.inputs`/`self.playbacks` volume model.
    /// `set_mute`/`set_solo` use this to force a crosspoint to 0 (or
    /// back) while keeping the channel's own "real" volume intact to
    /// restore later, exactly like `usb.rs`'s already
    /// hardware-validated mute/solo. A missing element (e.g. an
    /// unsupported output count) is a silent no-op, matching
    /// `set_volume`'s own `if let Some(selem) = ...` behavior.
    fn write_crosspoint(
        &self,
        src: usize,
        ch: SelemChannelId,
        is_mono: bool,
        output: usize,
        volume: f32,
        pan: i8,
    ) -> Result<(), Error> {
        if self.grammar == ControlGrammar::ClassCompliant {
            // Same arithmetic, different addressing: each destination
            // channel is its own mono control, so a mono source's pan
            // is two writes rather than two channels of one selem.
            let right = ch == SelemChannelId::FrontRight;
            let Some(src_name) = cc_source_name(src, right) else {
                return Ok(());
            };
            let mono = SelemChannelId::mono();
            if is_mono {
                let (Some(dl), Some(dr)) = (cc_dest_name(output, false), cc_dest_name(output, true))
                else {
                    return Ok(());
                };
                let (Some(sl), Some(sr)) = (
                    self.mixer.find_selem(&format!("{src_name}-{dl}"), 0),
                    self.mixer.find_selem(&format!("{src_name}-{dr}"), 0),
                ) else {
                    return Ok(());
                };
                let max = sl.get_playback_volume_range().1 as f32;
                let (l, r) = encode_volume_pan(volume.clamp(0.0, 1.0), pan, max);
                sl.set_playback_volume(mono, l)?;
                sr.set_playback_volume(mono, r)?;
            } else {
                // A paired source feeds only its own side of the pair,
                // exactly as the proprietary path writes only `ch`.
                let Some(dest) = cc_dest_name(output, right) else {
                    return Ok(());
                };
                let Some(selem) = self.mixer.find_selem(&format!("{src_name}-{dest}"), 0) else {
                    return Ok(());
                };
                let max = selem.get_playback_volume_range().1 as f32;
                selem.set_playback_volume(mono, (volume.clamp(0.0, 1.0) * max) as i64)?;
            }
            return Ok(());
        }

        // The driver's crosspoints are in dB (TLV), with +6 dB at the top:
        // convert through that, as for the output masters.
        let Some(selem) = self.crosspoint_selem(src, output) else {
            return Ok(());
        };
        let volume = volume.clamp(0.0, 2.0);
        if is_mono {
            let (l, r) = pan_split(volume, pan);
            set_tlv_volume(&selem, SelemChannelId::FrontLeft, l)?;
            set_tlv_volume(&selem, SelemChannelId::FrontRight, r)?;
        } else {
            set_tlv_volume(&selem, ch, volume)?;
        }
        Ok(())
    }

    /// Which control grammar this card speaks. Lets a UI gate the
    /// DSP-only features (EQ, phase, trim, split, loopback, the front
    /// panel) instead of offering them and letting
    /// [`Self::require_proprietary`] reject the click.
    pub fn grammar(&self) -> ControlGrammar {
        self.grammar
    }

    /// Refuses a feature that exists only in RME's own protocol.
    ///
    /// The class-compliant surface is routing + preamps + clock and
    /// nothing else — no EQ, no phase/trim/split, no loopback, no
    /// front panel. Those setters all look them up with a tolerant
    /// `if let Some(selem)`, which would return `Ok(())` having done
    /// nothing: the exact "silently lying to the user" failure this
    /// backend spent 2026-09-12 removing. Fail loudly instead.
    fn require_proprietary(&self, feature: &str) -> Result<(), Error> {
        if self.grammar == ControlGrammar::ClassCompliant {
            return Err(Error::InvalidChannel(format!(
                "{feature} needs the snd-usb-babyface-pro driver — this card is \
                 in Class Compliant mode, which does not expose it"
            )));
        }
        Ok(())
    }

    /// The preamp base name for input `idx` in the class-compliant
    /// grammar: `Mic-AN1`/`Mic-AN2` for the two mics, `Line-IN3`/
    /// `Line-IN4` for the two instrument inputs.
    fn cc_preamp_base(idx: usize) -> Option<&'static str> {
        Some(match idx {
            0 => "Mic-AN1",
            1 => "Mic-AN2",
            2 => "Line-IN3",
            3 => "Line-IN4",
            _ => return None,
        })
    }

    /// The preamp gain element for input `idx`, in either grammar.
    ///
    /// The two differ in more than spelling: the proprietary driver
    /// exposes one `Mic 1` element per `.index`, as a *capture* volume;
    /// the class-compliant surface has a distinctly-named element per
    /// input carrying both playback and capture volume, so the callers
    /// also have to pick the right accessor.
    fn preamp_gain_selem(&self, idx: usize) -> Option<Selem<'_>> {
        if self.grammar == ControlGrammar::ClassCompliant {
            self.mixer
                .find_selem(&format!("{} Gain", Self::cc_preamp_base(idx)?), 0)
        } else {
            self.mixer.find_selem("Mic 1", idx as u32)
        }
    }

    /// The 48V element for mic `idx` (0/1), in either grammar.
    fn phantom_selem(&self, idx: usize) -> Option<Selem<'_>> {
        if self.grammar == ControlGrammar::ClassCompliant {
            self.mixer
                .find_selem(&format!("{} 48V", Self::cc_preamp_base(idx)?), 0)
        } else {
            self.mixer.find_selem("Phantom Power Mic 1", idx as u32)
        }
    }

    /// The PAD element for mic `idx` (0/1), in either grammar.
    fn pad_selem(&self, idx: usize) -> Option<Selem<'_>> {
        if self.grammar == ControlGrammar::ClassCompliant {
            self.mixer
                .find_selem(&format!("{} PAD", Self::cc_preamp_base(idx)?), 0)
        } else {
            self.mixer.find_selem("Pad Mic 1", idx as u32)
        }
    }

    /// Whether the model's crosspoint value is what the card holds - the
    /// re-read test.  The driver's crosspoints are converted through dB, so
    /// compare raw values: a write rounds to the control's grid, and a
    /// re-read must not move a fader the user is holding.  The
    /// class-compliant card keeps the share-of-range comparison.
    #[allow(clippy::too_many_arguments)]
    fn crosspoint_matches(
        &self,
        src: usize,
        ch: SelemChannelId,
        is_mono: bool,
        output: usize,
        model_volume: f32,
        model_pan: i8,
        card_volume: f32,
    ) -> bool {
        if self.grammar == ControlGrammar::ClassCompliant {
            return !resync_differs(model_volume, card_volume);
        }
        let Some(selem) = self.crosspoint_selem(src, output) else {
            return false;
        };
        let volume = model_volume.clamp(0.0, 2.0);
        if is_mono {
            let (l, r) = pan_split(volume, model_pan);
            tlv_matches(&selem, SelemChannelId::FrontLeft, l)
                && tlv_matches(&selem, SelemChannelId::FrontRight, r)
        } else {
            tlv_matches(&selem, ch, volume)
        }
    }

    /// Reads a crosspoint back as `(volume, pan)`, in whichever
    /// grammar this card speaks. `pan` is meaningful only for the four
    /// true-mono sources; paired sources report 0.
    ///
    /// The mirror of [`Self::write_crosspoint`] — kept next to it so
    /// the two addressing models stay visibly in step.
    fn read_crosspoint(
        &self,
        src: usize,
        ch: SelemChannelId,
        is_mono: bool,
        output: usize,
    ) -> Option<(f32, i8)> {
        if self.grammar == ControlGrammar::ClassCompliant {
            let right = ch == SelemChannelId::FrontRight;
            let src_name = cc_source_name(src, right)?;
            let mono = SelemChannelId::mono();
            if is_mono {
                let sl = self
                    .mixer
                    .find_selem(&format!("{src_name}-{}", cc_dest_name(output, false)?), 0)?;
                let sr = self
                    .mixer
                    .find_selem(&format!("{src_name}-{}", cc_dest_name(output, true)?), 0)?;
                let max = sl.get_playback_volume_range().1 as f32;
                let l = sl.get_playback_volume(mono).ok()?;
                let r = sr.get_playback_volume(mono).ok()?;
                Some(decode_volume_pan(l, r, max))
            } else {
                let selem = self
                    .mixer
                    .find_selem(&format!("{src_name}-{}", cc_dest_name(output, right)?), 0)?;
                let max = selem.get_playback_volume_range().1 as f32;
                Some((selem.get_playback_volume(mono).ok()? as f32 / max, 0))
            }
        } else {
            let selem = self.crosspoint_selem(src, output)?;
            if is_mono {
                let l = tlv_volume(&selem, SelemChannelId::FrontLeft)?;
                let r = tlv_volume(&selem, SelemChannelId::FrontRight)?;
                Some(pan_join(l, r))
            } else {
                Some((tlv_volume(&selem, ch)?, 0))
            }
        }
    }

    /// Match ALSA mixer elements to our channel model.
    ///
    /// Every lookup here is a deliberate `if let Some(selem)` no-op when
    /// the control is absent, because the driver grew its control set
    /// over time and an older module should still give a working mixer
    /// for whatever it *does* expose. (The 2026-08-28 version of this
    /// comment listed Sample Clock Source and per-input Sensitivity as
    /// permanently missing; both were added to `snd-usb-babyface-pro`
    /// on 2026-09-06 — as "Sample Clock Source" and "Instrument Ref
    /// Level" — and are matched below. The proprietary driver also
    /// exposes "Optical Out Format" (ADAT / S/PDIF) since 2026-10-18;
    /// the IEC958 controls remain Class Compliant only.)
    ///
    /// That tolerance is why [`RmeDevice::open`] has to check for a few
    /// sentinel controls *before* getting here: absent the guard, a card
    /// with an entirely foreign control grammar (a CC-mode Babyface on
    /// `snd-usb-audio`) matches nothing at all and still looks healthy.
    ///
    /// `resync` is set when re-reading a card already in use (see
    /// `poll_events`).  Mute and solo on the input/playback strips exist
    /// only here - they zero crosspoints on the card while the model keeps
    /// the real levels to restore - so a re-read leaves the crosspoints of
    /// a muted strip, and every strip's while one is soloed, alone.  So
    /// does an output's master in the class-compliant grammar while it is
    /// muted, which is model-level there too.
    fn attach_mixer_elements(&mut self, resync: bool) {
        let mono = SelemChannelId::mono();

        for (name, selem) in self.mixer.iter_selems() {
            if name == "Sample Clock Source" {
                if let Ok(current) = selem.get_enum_item(mono) {
                    if let Ok(item_name) = selem.get_enum_item_name(current) {
                        self.settings.clock_source = item_name;
                    }
                }
                if let Ok(count) = selem.get_enum_items() {
                    self.settings.clock_sources = (0..count)
                        .filter_map(|i| selem.get_enum_item_name(i).ok())
                        .collect();
                }
                continue;
            }
            if name == "Optical Out Format" {
                if let Ok(item) = selem.get_enum_item(mono) {
                    if let Ok(item_name) = selem.get_enum_item_name(item) {
                        self.settings.optical_out_spdif = item_name == "S/PDIF";
                    }
                }
                continue;
            }
            if name == "Instrument Ref Level" {
                // A single shared switch (see `set_sensitivity`'s own
                // doc comment) — read once, mirrored to every
                // Instrument channel below. The real control has 3
                // states (+4dBu/-10dBV/Boost) but this trait's
                // `Sensitivity` enum only has 2 — Boost (item 2) maps
                // to the closer of the two rather than being lost
                // entirely: it shares -10dBV's state bits, only the
                // one-shot commit differs (see `babyface-pro-linux`'s
                // own "Instrument Ref Level" control).
                if let Ok(item) = selem.get_enum_item(mono) {
                    let sens = match item {
                        0 => Some(Sensitivity::Plus4dBu),
                        _ => Some(Sensitivity::Minus10dBV), // 1 = -10dBV, 2 = Boost
                    };
                    for c in self.inputs.iter_mut() {
                        if c.channel_type == ChannelType::Instrument {
                            c.sensitivity = sens;
                        }
                    }
                }
                continue;
            }
            if name == "IEC958 Emphasis" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_emphasis = v != 0;
                }
                continue;
            }
            if name == "IEC958 Pro Mask" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_professional = v != 0;
                }
                continue;
            }
            if name == "IEC958" {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.settings.spdif_enabled = v != 0;
                }
                continue;
            }
        }

        // ── Phantom 48V & PAD — the two mics. Read both first, then
        // apply: the grammar-aware lookups borrow `self`, so they can't
        // run while `self.inputs` is mutably borrowed.
        for i in 0..2 {
            let phantom = self
                .phantom_selem(i)
                .and_then(|s| s.get_playback_switch(mono).ok())
                .map(|v| v != 0);
            let pad = self
                .pad_selem(i)
                .and_then(|s| s.get_playback_switch(mono).ok())
                .map(|v| v != 0);
            let Some(inp) = self.inputs.get_mut(i) else {
                continue;
            };
            if let Some(v) = phantom {
                inp.phantom = v;
            }
            if let Some(v) = pad {
                inp.pad = v;
            }
        }

        // ── Sensitivity, class-compliant grammar: a separate
        // `<base> Sens.` enum per instrument input (the proprietary
        // "Instrument Ref Level" is read in the loop above instead, and
        // is a single shared switch). Resolved by item NAME, because the
        // two controls enumerate their items in opposite orders —
        // ["-10dBV","+4dBu"] here vs ["+4dBu","-10dBV","Boost"] there.
        // Without this, sensitivity silently reported the model default
        // rather than the card: observed 2026-09-12 showing "+4dBu"
        // while the card was really on -10dBV.
        if self.grammar == ControlGrammar::ClassCompliant {
            for i in 0..4.min(self.inputs.len()) {
                let read = Self::cc_preamp_base(i)
                    .and_then(|base| self.mixer.find_selem(&format!("{base} Sens."), 0))
                    .and_then(|selem| {
                        let item = selem.get_enum_item(mono).ok()?;
                        match selem.get_enum_item_name(item).ok()?.as_str() {
                            "+4dBu" => Some(Sensitivity::Plus4dBu),
                            "-10dBV" => Some(Sensitivity::Minus10dBV),
                            _ => None,
                        }
                    });
                if let Some(sens) = read {
                    self.inputs[i].sensitivity = Some(sens);
                }
            }
        }

        // ── Phase Ø invert — AN1-4 = inputs[0..4], one distinctly-
        // named control per mic (`babyface-pro-linux`'s own "<name>
        // Phase" Switch, added 2026-09-06).
        for i in 0..4.min(self.inputs.len()) {
            if let Some(selem) = self.mixer.find_selem(&format!("{} Phase", BF_SOURCES[i]), i as u32)
            {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    self.inputs[i].phase = v != 0;
                }
            }
        }

        // ── Trim (T) — AN1-4, same per-mic naming as Phase above
        // (`babyface-pro-linux`'s own "<name> Trim Volume", added
        // 2026-09-06).
        for i in 0..4.min(self.inputs.len()) {
            if let Some(selem) = self.mixer.find_selem(&format!("{} Trim", BF_SOURCES[i]), i as u32)
            {
                if let Ok(v) = selem.get_playback_volume(mono) {
                    self.inputs[i].trim = v as f32;
                }
            }
        }

        // ── Stereo split — playback pairs PB1-PB6, one control per
        // pair (`babyface-pro-linux`'s own "PBx Stereo Split" Switch,
        // added 2026-09-06); mirrored to both channels of the pair.
        for pair in 0..(self.playbacks.len() / 2) {
            if let Some(selem) = self
                .mixer
                .find_selem(&format!("PB{} Stereo Split", pair + 1), pair as u32)
            {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    let split = v != 0;
                    if let Some(c) = self.playbacks.get_mut(pair * 2) {
                        c.split = split;
                    }
                    if let Some(c) = self.playbacks.get_mut(pair * 2 + 1) {
                        c.split = split;
                    }
                }
            }
        }

        // ── Preamp gain — inputs[0..4] (Mic1, Mic2, Instr3, Instr4) map
        // 1:1 to ALSA index 0..4 of "Mic 1" (a *capture* volume, not
        // playback — `mixer.c:1313-1326`).
        for i in 0..4 {
            let read = self.preamp_gain_selem(i).and_then(|s| {
                let v = s.get_capture_volume(mono).ok()?;
                Some((v as u32, s.get_capture_volume_range().1 as u32))
            });
            let Some(inp) = self.inputs.get_mut(i) else {
                continue;
            };
            if let Some((v, max)) = read {
                inp.gain = Some(v);
                inp.gain_max = Some(max);
            }
        }

        // ── Crosspoint volume (+ pan for the 4 true-mono sources) ──
        let any_solo = self.inputs.iter().any(|c| c.solo) || self.playbacks.iter().any(|c| c.solo);
        for i in 0..self.inputs.len() {
            let Some((src, ch, is_mono)) = input_crosspoint_slot(i) else {
                continue;
            };
            if resync && (self.inputs[i].mute || any_solo) {
                continue;
            }
            for out in 0..self.profile.output_pair_count() {
                if let Some((volume, pan)) = self.read_crosspoint(src, ch, is_mono, out) {
                    if resync
                        && self.crosspoint_matches(
                            src,
                            ch,
                            is_mono,
                            out,
                            self.inputs[i].volumes[out],
                            self.inputs[i].pans[out],
                            volume,
                        )
                    {
                        continue;
                    }
                    self.inputs[i].volumes[out] = volume;
                    // A silent crosspoint carries no pan: keep the
                    // model's, so a fader pulled down and back up does
                    // not come back centred.
                    if is_mono && (!resync || volume > 0.0) {
                        self.inputs[i].pans[out] = pan;
                    }
                }
            }
        }

        for i in 0..self.playbacks.len() {
            let Some((src, ch)) = playback_crosspoint_slot(i) else {
                continue;
            };
            if resync && (self.playbacks[i].mute || any_solo) {
                continue;
            }
            for out in 0..self.profile.output_pair_count() {
                if let Some((volume, _)) = self.read_crosspoint(src, ch, false, out) {
                    if resync
                        && self.crosspoint_matches(
                            src,
                            ch,
                            false,
                            out,
                            self.playbacks[i].volumes[out],
                            0,
                            volume,
                        )
                    {
                        continue;
                    }
                    self.playbacks[i].volumes[out] = volume;
                }
            }
        }

        // ── Output pair master volume + mute — one 2-channel ALSA
        // element per pair (`PAIR_LABELS[pair_idx]` @ index `pair_idx`),
        // front-left/front-right are the pair's two physical channels;
        // the mute switch is shared by both (`mixer.c`'s `bf_mute_get`
        // mirrors the same value into both ALSA channels), so both
        // `self.outputs` entries for the pair get the same mute state.
        // In the class-compliant grammar each physical output is its
        // own `Main-Out <ch>` control and there is NO mute switch
        // anywhere on the card (the dump has exactly seven switches:
        // 2x48V, 2xPAD, 3xIEC958), so mute stays model-level there —
        // the same conclusion the 2026-08-13 CC probe reached.
        for pair_idx in 0..PAIR_LABELS.len() {
            for (side, right) in [(0usize, false), (1usize, true)] {
                let (volume, muted) = if self.grammar == ControlGrammar::ClassCompliant {
                    let Some(dest) = cc_dest_name(pair_idx, right) else {
                        continue;
                    };
                    let Some(selem) = self.mixer.find_selem(&format!("Main-Out {dest}"), 0) else {
                        continue;
                    };
                    let max = selem.get_playback_volume_range().1 as f32;
                    let Ok(v) = selem.get_playback_volume(mono) else {
                        continue;
                    };
                    (v as f32 / max, None)
                } else {
                    let Some(selem) = self.mixer.find_selem(PAIR_LABELS[pair_idx], pair_idx as u32)
                    else {
                        continue;
                    };
                    let ch = if right {
                        SelemChannelId::FrontRight
                    } else {
                        SelemChannelId::FrontLeft
                    };
                    let Some(v) = tlv_volume(&selem, ch) else {
                        continue;
                    };
                    // When the model's value is what the card holds after
                    // rounding, keep the model's: the check below then
                    // leaves the fader alone, and the mute still syncs.
                    let model = self.outputs.get(pair_idx * 2 + side).map_or(0.0, |o| o.volume);
                    let v = if resync && tlv_matches(&selem, ch, model) { model } else { v };
                    // The mute switch is shared by both channels
                    // (`mixer.c`'s `bf_mute_get` mirrors one value).
                    let muted = selem
                        .get_playback_switch(SelemChannelId::FrontLeft)
                        .map(|v| v == 0)
                        .unwrap_or(false);
                    (v, Some(muted))
                };
                if let Some(out) = self.outputs.get_mut(pair_idx * 2 + side) {
                    // Class-compliant mute is the master driven to 0.
                    let model_level_mute = muted.is_none() && out.mute;
                    if !(resync && (model_level_mute || !resync_differs(out.volume, volume))) {
                        out.volume = volume;
                    }
                    if let Some(m) = muted {
                        out.mute = m;
                    }
                }
            }
        }

        // ── Loopback — one bool per output pair.
        for pair_idx in 0..PAIR_LABELS.len() {
            if let Some(selem) = self.mixer.find_selem("Loopback", pair_idx as u32) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    let on = v != 0;
                    if let Some(o) = self.outputs.get_mut(pair_idx * 2) {
                        o.loopback = on;
                    }
                    if let Some(o) = self.outputs.get_mut(pair_idx * 2 + 1) {
                        o.loopback = on;
                    }
                }
            }
        }

        // ── Global switches / values (AN 1>2, AN1/2 Link, MS Processor,
        // Dim, Width, FX Send, Varispeed Pitch) — all real ALSA controls
        // (mixer.c:1016-1114), previously never wired up on this backend.
        if let Some(selem) = self.mixer.find_selem("AN 1>2", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.an12 = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("AN1/2 Link", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.linked = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("MS Processor", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.ms_proc = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("Dim", 0) {
            if let Ok(v) = selem.get_playback_switch(mono) {
                self.settings.dim = v != 0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("Width", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.width = v as f32 / 100.0;
            }
        }
        if let Some(selem) = self.mixer.find_selem("FX Send", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.fx_send_db = Some(if v <= 0 { -65.0 } else { fader_raw_to_db(v as u16) });
            }
        }
        if let Some(selem) = self.mixer.find_selem("Varispeed Pitch", 0) {
            if let Ok(v) = selem.get_playback_volume(mono) {
                self.settings.pitch_percent = v as f32 / 10.0;
            }
        }

        // ── Hardware DSP EQ — analog inputs only (`eq_strip_name`).
        for i in 0..4 {
            let Some(strip) = eq_strip_name(i) else {
                continue;
            };
            let mut eq = InputEq::default();
            let mut found_any = false;
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Enable"), 0) {
                if let Ok(v) = selem.get_playback_switch(mono) {
                    eq.enabled = v != 0;
                    found_any = true;
                }
            }
            for band in 0..3 {
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Type", band + 1), 0)
                {
                    if let Ok(v) = selem.get_enum_item(mono) {
                        eq.bands[band].band_type = eq_band_type_from_enum(v);
                        found_any = true;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Freq", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].freq_hz = v.clamp(0, 20_000) as u16;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Q", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].q = v as f32 / 100.0;
                    }
                }
                if let Some(selem) = self
                    .mixer
                    .find_selem(&format!("{strip} EQ Band {} Gain", band + 1), 0)
                {
                    if let Ok(v) = selem.get_playback_volume(mono) {
                        eq.bands[band].gain_db = v as f32 / 10.0;
                    }
                }
            }
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Freq"), 0) {
                if let Ok(v) = selem.get_playback_volume(mono) {
                    eq.low_cut_freq_hz = v.clamp(0, 20_000) as u16;
                }
            }
            if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Slope"), 0) {
                if let Ok(v) = selem.get_enum_item(mono) {
                    eq.low_cut_slope_db_oct = eq_slope_from_enum(v);
                }
            }
            if found_any {
                self.inputs[i].eq = Some(eq);
            }
        }

        self.read_panel();

        if !resync {
            info!(
                "Attached {} inputs, {} playbacks, clock: {}",
                self.inputs.len(),
                self.playbacks.len(),
                self.settings.clock_source
            );
        }
    }

    /// Read the driver's front-panel controls into `self.panel`, and act
    /// on DIM presses counted since the last read.
    fn read_panel(&mut self) {
        let mono = SelemChannelId::mono();
        if self.grammar != ControlGrammar::Proprietary {
            return;
        }
        let enum_item = |name: &str| {
            self.mixer
                .find_selem(name, 0)
                .and_then(|s| s.get_enum_item(mono).ok())
        };
        let (in_item, out_item) = (enum_item("Front Panel In"), enum_item("Front Panel Out"));
        let mix = self
            .mixer
            .find_selem("Front Panel Mix", 0)
            .and_then(|s| s.get_playback_switch(mono).ok())
            .map(|v| v != 0);
        self.panel = match (mix, in_item, out_item) {
            (Some(mix), Some(i), Some(o)) => Some((
                mix,
                panel_enum_to_sel(i).unwrap_or(0),
                panel_enum_to_sel(o).unwrap_or(0),
            )),
            _ => None,
        };

        let count = self
            .mixer
            .find_selem("DIM Button Press Count", 0)
            .and_then(|s| s.get_playback_volume(mono).ok());
        if let (Some(prev), Some(now)) = (self.dim_press_count, count) {
            if dim_presses_toggle(prev, now) {
                let on = !self.settings.dim;
                if let Err(e) = self.set_dim(on) {
                    log::warn!("front-panel DIM press: setting Dim failed: {e}");
                }
            }
        }
        self.dim_press_count = count;
    }

    /// The front-panel state the driver reports, for the UI to follow:
    /// (MIX engaged, IN selection, OUT selection), in the USB backend's
    /// convention (IN 0 = Ch 1/2, 1 = Ch 3/4, 2 = Opt; OUT 0 = Ch 1/2,
    /// 1 = Phones, 2 = Opt).  `None` if the driver has no front-panel
    /// controls, or the card is in class-compliant mode.
    pub fn panel_selection(&self) -> Option<(bool, usize, usize)> {
        self.panel
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

impl RmeDevice for BabyfacePro {
    fn model_name(&self) -> &str {
        self.profile.model_name
    }

    fn output_pair_count(&self) -> usize {
        self.profile.output_pair_count()
    }

    /// The class-compliant surface is routing, preamps and clock; every
    /// feature in RME's own protocol is missing there (see
    /// `ControlGrammar`). The UIs gate on this so they stop offering
    /// controls that would fail.
    fn protocol_features(&self) -> bool {
        self.grammar != ControlGrammar::ClassCompliant
    }

    /// Card still resolvable (the unplug case ends here, silently), and
    /// still the same *enumeration*: a re-created card (replug,
    /// personality switch) comes back on the same ALSA index with the
    /// same names while our mixer keeps answering from a fd the old card
    /// no longer backs, and the USB `devnum` is the one thing that
    /// moves. A mixer iteration cannot tell: snd_mixer caches its
    /// element list, so it yields elements long after the card is gone
    /// (checked against a live personality switch, 2026-10-16: the chip
    /// kept saying "Proprietary" for a class-compliant card).
    fn is_connected(&self) -> bool {
        AlsaMixer::resolve_card(PROFILE.card_substring).is_some()
            && AlsaMixer::card_devnum(self.mixer.card_name()) == self.card_devnum
    }

    fn open() -> Result<Self, Error> {
        info!("Searching for RME Babyface Pro...");
        let profile = &PROFILE;
        let mixer = AlsaMixer::open_by_card_name(profile.card_substring)?;

        // Matching the card *name* isn't enough to know which mixer
        // this is. A Babyface Pro in Class Compliant mode on stock
        // `snd-usb-audio` announces itself as "RME Babyface Pro
        // (<serial>)", which contains `card_substring` just as ours
        // does, yet speaks a completely different control grammar.
        // Every lookup below is a tolerant `if let Some(selem)`, so
        // guessing wrong doesn't fail loudly — it comes up "working"
        // while driving nothing (observed 2026-09-12: 48V read as off
        // while the card had it on, and setters returning `Ok(())`
        // without reaching the hardware). So pick the grammar from a
        // sentinel control unique to each, and refuse only if neither
        // is there.
        let grammar = if mixer.find_selem("Mic 1", 0).is_some() {
            ControlGrammar::Proprietary
        } else if mixer.find_selem("Mic-AN1 Gain", 0).is_some() {
            ControlGrammar::ClassCompliant
        } else {
            return Err(Error::UnsupportedDeviceMode {
                card: mixer.card_name().to_string(),
                missing: "Mic 1 (proprietary), Mic-AN1 Gain (class-compliant)".into(),
            });
        };
        info!("ALSA control grammar: {grammar:?}");

        // Request all 12 capture channels. The kernel driver's capture
        // map is the identity in v7 (2026-10-23): capture channel i is the
        // i-th word, in the device's own order AN1-4, AS1/2, ADAT3-8.
        // `capture_meter::INPUT_FOR_CAPTURE` therefore maps them straight
        // onto the input strips. See `capture_meter`'s module doc comment
        // for the live mapping.
        let capture_meter =
            crate::capture_meter::CaptureMeter::start(mixer.card_name(), 48_000);
        if capture_meter.is_none() {
            log::warn!("Could not open the capture PCM for input level metering; meters will read N/A");
        }
        let mut device = Self {
            card_devnum: AlsaMixer::card_devnum(mixer.card_name()),
            mixer,
            grammar,
            capture_meter,
            profile,
            inputs: profile.build_inputs(),
            playbacks: profile.build_playbacks(),
            outputs: profile.build_outputs(),
            settings: DeviceSettings {
                clock_source: "Internal".into(),
                clock_sources: Vec::new(),
                spdif_optical: false,
                spdif_emphasis: false,
                spdif_professional: false,
                spdif_enabled: false,
                pitch_percent: 0.0,
                ms_proc: false,
                an12: false,
                eq_for_record: false,
                optical_out_spdif: false,
                dim: false,
                fx_send_db: None,
                width: 0.0,
                sample_rate: 48_000,
                input_link: true,
                output_link: Vec::new(),
                input_pair_link: Vec::new(),
            },
            linked: true,
            dim_press_count: None,
            panel: None,
            resync_at: None,
        };
        device.attach_mixer_elements(false);
        Ok(device)
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
        let vol_clamped = volume.clamp(0.0, 1.0);

        // Output channels: one 2-channel "<pair label> Playback Volume"
        // ALSA element per pair (`PAIR_LABELS`), front-left/front-right
        // are the pair's two physical channels — `idx / 2` finds the
        // pair, `idx % 2` picks which channel of it.
        if let ChannelId::Output(idx) = channel {
            let out = self
                .outputs
                .get_mut(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let label = PAIR_LABELS
                .get(idx / 2)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            let ch = if idx % 2 == 0 {
                SelemChannelId::FrontLeft
            } else {
                SelemChannelId::FrontRight
            };
            if self.grammar == ControlGrammar::ClassCompliant {
                if let Some(dest) = cc_dest_name(idx / 2, idx % 2 == 1) {
                    if let Some(selem) = self.mixer.find_selem(&format!("Main-Out {dest}"), 0) {
                        let max = selem.get_playback_volume_range().1 as f32;
                        selem
                            .set_playback_volume(SelemChannelId::mono(), (vol_clamped * max) as i64)?;
                    }
                }
                out.volume = vol_clamped;
            } else {
                // The model's +6 dB headroom is real on this driver.
                let vol = volume.clamp(0.0, 2.0);
                if let Some(selem) = self.mixer.find_selem(label, (idx / 2) as u32) {
                    set_tlv_volume(&selem, ch, vol)?;
                }
                out.volume = vol;
            }
            return Ok(());
        }

        if output >= self.profile.output_pair_count() {
            return Err(Error::InvalidChannel(format!("Output {}", output)));
        }

        let (src, ch, is_mono) = match channel {
            ChannelId::Input(idx) => input_crosspoint_slot(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?,
            ChannelId::Playback(idx) => {
                let (src, ch) = playback_crosspoint_slot(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                (src, ch, false)
            }
            ChannelId::Output(_) => unreachable!("Output handled above"),
        };

        // Preserve the channel's current pan: a mono source's "volume"
        // is really two independent ALSA channels, so writing volume
        // alone must re-encode with the cached pan rather than
        // flattening it to center.
        let pan = if let ChannelId::Input(idx) = channel { self.inputs[idx].pans[output] } else { 0 };
        // The driver's crosspoints reach +6 dB, the model's 2.0.
        let vol = if self.grammar == ControlGrammar::ClassCompliant {
            vol_clamped
        } else {
            volume.clamp(0.0, 2.0)
        };
        self.write_crosspoint(src, ch, is_mono, output, vol, pan)?;

        match channel {
            ChannelId::Input(idx) => self.inputs[idx].volumes[output] = vol,
            ChannelId::Playback(idx) => self.playbacks[idx].volumes[output] = vol,
            ChannelId::Output(_) => unreachable!("Output handled above"),
        }
        Ok(())
    }

    fn volume(&self, channel: ChannelId, output: usize) -> Result<f32, Error> {
        match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                ch.volumes
                    .get(output)
                    .copied()
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))
            }
            ChannelId::Playback(idx) => {
                let ch = self
                    .playbacks
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                ch.volumes
                    .get(output)
                    .copied()
                    .ok_or_else(|| Error::InvalidChannel(format!("Output {}", output)))
            }
            ChannelId::Output(idx) => self
                .outputs
                .get(idx)
                .map(|c| c.volume)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx))),
        }
    }

    fn set_pan(&mut self, channel: ChannelId, output: usize, pan: i8) -> Result<(), Error> {
        let pan = pan.clamp(-100, 100);

        // Only the 4 true-mono sources (AN1-AN4) have an independent
        // left/right crosspoint pair to pan across — AS1/2, the ADAT
        // pairs and every PCM playback channel are hardware-linked
        // stereo sources with one ALSA channel each, no cross-routing
        // possible (see `BF_SOURCES`/`input_crosspoint_slot`).
        let (idx, volume) = match channel {
            ChannelId::Input(idx) => {
                let ch = self
                    .inputs
                    .get(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                if output >= ch.pans.len() {
                    return Err(Error::InvalidChannel(format!("Output {}", output)));
                }
                (idx, ch.volumes[output])
            }
            ChannelId::Playback(_) => {
                return Err(Error::InvalidChannel(
                    "Playback channels are linked stereo pairs and have no independent pan"
                        .into(),
                ));
            }
            ChannelId::Output(_) => return Err(Error::InvalidChannel("Output has no pan".into())),
        };
        let (src, _ch, is_mono) =
            input_crosspoint_slot(idx).ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if !is_mono {
            return Err(Error::InvalidChannel(format!(
                "Input {} is a linked stereo source and has no independent pan",
                idx
            )));
        }

        self.write_crosspoint(src, SelemChannelId::FrontLeft, true, output, volume, pan)?;

        self.inputs[idx].pans[output] = pan;
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
        // Output mute is a real hardware switch, but it's shared by the
        // whole pair (`mixer.c`'s `bf_mute_get` mirrors one flag onto
        // both ALSA channels) — muting one physical output channel
        // mutes its sibling too, so mirror that into both `self.outputs`
        // entries rather than leaving them inconsistent with hardware.
        if let ChannelId::Output(idx) = channel {
            let pair_idx = idx / 2;
            let label = PAIR_LABELS
                .get(pair_idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Output {}", idx)))?;
            if self.grammar == ControlGrammar::ClassCompliant {
                // No mute switch exists anywhere on the CC surface, so
                // mute is "drive the master to zero and put the stored
                // volume back on unmute" — the same trick `usb.rs` uses
                // for strip mute. The real level stays in the model, so
                // nothing is lost; the cost is that mute state doesn't
                // survive a re-open (there's no hardware bit to read).
                for side in 0..2 {
                    let Some(dest) = cc_dest_name(pair_idx, side == 1) else {
                        continue;
                    };
                    let Some(selem) = self.mixer.find_selem(&format!("Main-Out {dest}"), 0) else {
                        continue;
                    };
                    let max = selem.get_playback_volume_range().1 as f32;
                    let level = if mute {
                        0.0
                    } else {
                        self.outputs
                            .get(pair_idx * 2 + side)
                            .map(|o| o.volume)
                            .unwrap_or(0.0)
                    };
                    selem.set_playback_volume(SelemChannelId::mono(), (level * max) as i64)?;
                }
            } else if let Some(selem) = self.mixer.find_selem(label, pair_idx as u32) {
                let unmuted = (!mute) as i32;
                selem.set_playback_switch(SelemChannelId::FrontLeft, unmuted)?;
                selem.set_playback_switch(SelemChannelId::FrontRight, unmuted)?;
            }
            if let Some(out) = self.outputs.get_mut(pair_idx * 2) {
                out.mute = mute;
            }
            if let Some(out) = self.outputs.get_mut(pair_idx * 2 + 1) {
                out.mute = mute;
            }
            return Ok(());
        }

        // Input/Playback have no dedicated ALSA mute switch — same as
        // TotalMix on Windows, which does this by driving the routing
        // matrix directly: `cap_mute2.pcap` (2026-08-24) shows muting a
        // strip zeroing its crosspoints into EVERY output, restored
        // from the model on unmute. Ported from `usb.rs`'s own
        // already-hardware-validated implementation of the same
        // capture, just through ALSA crosspoint controls instead of
        // raw USB writes. No solo-awareness here, matching `usb.rs`
        // exactly: unmuting a strip while some other strip is soloed
        // restores this strip's own levels regardless — a real
        // TotalMix quirk this ports faithfully rather than "fixing".
        let n_outputs = self.profile.output_pair_count();
        match channel {
            ChannelId::Input(idx) => {
                let (src, ch, is_mono) = input_crosspoint_slot(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
                for out in 0..n_outputs {
                    let level = if mute { 0.0 } else { self.inputs[idx].volumes[out] };
                    let pan = self.inputs[idx].pans[out];
                    self.write_crosspoint(src, ch, is_mono, out, level, pan)?;
                }
            }
            ChannelId::Playback(idx) => {
                let (src, ch) = playback_crosspoint_slot(idx)
                    .ok_or_else(|| Error::InvalidChannel(format!("Playback {}", idx)))?;
                for out in 0..n_outputs {
                    let level = if mute { 0.0 } else { self.playbacks[idx].volumes[out] };
                    self.write_crosspoint(src, ch, false, out, level, 0)?;
                }
            }
            ChannelId::Output(_) => unreachable!("handled above"),
        }

        let ch = self.channel_mut(channel)?;
        *ch.0 = mute;
        Ok(())
    }

    fn mute(&self, channel: ChannelId) -> Result<bool, Error> {
        let ch = self.channel(channel)?;
        Ok(*ch.0)
    }

    fn set_solo(&mut self, channel: ChannelId, solo: bool) -> Result<(), Error> {
        // Ported from `usb.rs`'s own already-hardware-validated
        // implementation of `cap_solo2.pcap` (2026-08-24): soloing a
        // strip does NOT touch its own crosspoints — it zeroes every
        // OTHER input/playback strip's send into every output instead,
        // so only the soloed strip is heard in the monitor mix.
        // Un-solo restores every other strip from the model,
        // respecting its own independent mute. Outputs have no solo in
        // TotalMix. Exclusive solo (TotalMix's default): soloing a
        // strip clears every other strip's solo flag first, so a later
        // un-solo of any of them is a no-op instead of fighting this
        // one.
        let (solo_i, solo_c) = match channel {
            ChannelId::Input(i) => (Some(i), None),
            ChannelId::Playback(c) => (None, Some(c)),
            ChannelId::Output(_) => return Ok(()),
        };
        // A linked pair is soloed one channel at a time
        // (`set_input_solo`/`set_playback_solo`), so the exclusive solo
        // below must spare the strip's stereo partner: it used to clear
        // the partner's flag and zero its sends, so a soloed linked pair
        // played one side only and its strip (which shows the left
        // channel) never lit.
        let partner_i = solo_i.filter(|i| self.input_pair_linked(i / 2)).map(|i| i ^ 1);
        let partner_c = solo_c.filter(|c| self.playback_linked(c / 2)).map(|c| c ^ 1);
        match channel {
            ChannelId::Input(i) => self.inputs[i].solo = solo,
            ChannelId::Playback(c) => self.playbacks[c].solo = solo,
            ChannelId::Output(_) => unreachable!(),
        }

        let n_outputs = self.profile.output_pair_count();
        for j in 0..self.inputs.len() {
            if solo_i == Some(j) || partner_i == Some(j) {
                continue;
            }
            if solo {
                self.inputs[j].solo = false;
            }
            let Some((src, ch, is_mono)) = input_crosspoint_slot(j) else {
                continue;
            };
            let muted = self.inputs[j].mute;
            for out in 0..n_outputs {
                let level = if solo || muted { 0.0 } else { self.inputs[j].volumes[out] };
                let pan = self.inputs[j].pans[out];
                self.write_crosspoint(src, ch, is_mono, out, level, pan)?;
            }
        }
        for j in 0..self.playbacks.len() {
            if solo_c == Some(j) || partner_c == Some(j) {
                continue;
            }
            if solo {
                self.playbacks[j].solo = false;
            }
            let Some((src, ch)) = playback_crosspoint_slot(j) else {
                continue;
            };
            let muted = self.playbacks[j].mute;
            for out in 0..n_outputs {
                let level = if solo || muted { 0.0 } else { self.playbacks[j].volumes[out] };
                self.write_crosspoint(src, ch, false, out, level, 0)?;
            }
        }
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
        // Mic 1/Mic 2 = inputs[0]/inputs[1], mapping 1:1 to ALSA `.index`
        // (both mics share the literal name "Phantom Power Mic 1").
        if let Some(selem) = self.phantom_selem(idx) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.inputs[idx].phantom = on;
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
        if let Some(selem) = self.pad_selem(idx) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.inputs[idx].pad = on;
        Ok(())
    }

    fn set_gain(&mut self, idx: usize, gain: u32) -> Result<(), Error> {
        let inp = self
            .inputs
            .get(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if !matches!(inp.channel_type, ChannelType::Mic | ChannelType::Instrument) {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no gain control",
                idx
            )));
        }
        let clamped = inp.gain_max.map_or(gain, |max| gain.min(max));
        // A *capture* volume in both grammars (the class-compliant
        // `<base> Gain` carries playback and capture volume both, and
        // the capture side is the one that matches the proprietary
        // driver's law and range).
        if let Some(selem) = self.preamp_gain_selem(idx) {
            selem.set_capture_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        self.inputs[idx].gain = Some(clamped);
        Ok(())
    }

    fn set_eq_enabled(&mut self, idx: usize, on: bool) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Enable"), 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).enabled = on;
        Ok(())
    }

    fn set_eq_band_type(
        &mut self,
        idx: usize,
        band: usize,
        band_type: EqBandType,
    ) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Type", band + 1), 0)
        {
            selem.set_enum_item(SelemChannelId::mono(), eq_band_type_to_enum(band_type))?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].band_type = band_type;
        Ok(())
    }

    fn set_eq_band_freq(&mut self, idx: usize, band: usize, freq_hz: u16) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = freq_hz.min(20_000);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Freq", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].freq_hz = clamped;
        Ok(())
    }

    fn set_eq_band_q(&mut self, idx: usize, band: usize, q: f32) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = q.clamp(0.05, 10.0);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Q", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), (clamped * 100.0).round() as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].q = clamped;
        Ok(())
    }

    fn set_eq_band_gain(&mut self, idx: usize, band: usize, gain_db: f32) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        if band >= 3 {
            return Err(Error::InvalidChannel(format!("EQ band {}", band)));
        }
        let clamped = gain_db.clamp(-24.0, 24.0);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Band {} Gain", band + 1), 0)
        {
            selem.set_playback_volume(SelemChannelId::mono(), (clamped * 10.0).round() as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).bands[band].gain_db = clamped;
        Ok(())
    }

    fn set_eq_low_cut_freq(&mut self, idx: usize, freq_hz: u16) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let clamped = freq_hz.min(20_000);
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self.mixer.find_selem(&format!("{strip} EQ Low Cut Freq"), 0) {
            selem.set_playback_volume(SelemChannelId::mono(), clamped as i64)?;
        }
        inp.eq.get_or_insert_with(InputEq::default).low_cut_freq_hz = clamped;
        Ok(())
    }

    fn set_eq_low_cut_slope(&mut self, idx: usize, slope_db_oct: u8) -> Result<(), Error> {
        self.require_proprietary("EQ")?;
        let strip = eq_strip_name(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {} has no EQ", idx)))?;
        let inp = self
            .inputs
            .get_mut(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?;
        if let Some(selem) = self
            .mixer
            .find_selem(&format!("{strip} EQ Low Cut Slope"), 0)
        {
            selem.set_enum_item(SelemChannelId::mono(), eq_slope_to_enum(slope_db_oct))?;
        }
        inp.eq.get_or_insert_with(InputEq::default).low_cut_slope_db_oct = slope_db_oct;
        Ok(())
    }

    fn set_sensitivity(&mut self, idx: usize, sensitivity: Sensitivity) -> Result<(), Error> {
        if self
            .inputs
            .get(idx)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {}", idx)))?
            .channel_type
            != ChannelType::Instrument
        {
            return Err(Error::InvalidChannel(format!(
                "Input {} has no sensitivity switch",
                idx
            )));
        }
        // A single shared switch for the Instrument pair, not
        // per-channel — the protocol has no independent bits for IN3
        // vs IN4 (see `babyface-pro-linux`'s own "Instrument Ref
        // Level" control, added 2026-09-06 specifically to close this
        // gap the comment below used to describe as possibly a
        // genuinely-missing driver feature — it wasn't, just not built
        // yet at the time). Only 2 of that control's 3 states map to
        // this trait's 2-state `Sensitivity` enum; "Boost" isn't
        // reachable here (see `RmeDevice::set_ref_level`'s own 3-state
        // `REF_*` codes for that, used by the USB backend).
        if self.grammar == ControlGrammar::ClassCompliant {
            // Two differences from the proprietary control, both of
            // which would silently invert or over-apply the setting if
            // assumed away: it is PER-CHANNEL here (`Line-IN3 Sens.` /
            // `Line-IN4 Sens.`, so no mirroring), and its item order is
            // REVERSED — the card enumerates ["-10dBV", "+4dBu"], while
            // "Instrument Ref Level" enumerates ["+4dBu", "-10dBV",
            // "Boost"]. Resolve the item by *name* rather than by index
            // so the order can never bite us again.
            let base = Self::cc_preamp_base(idx)
                .ok_or_else(|| Error::InvalidChannel(format!("Input {idx} has no Sens. control")))?;
            let selem = self
                .mixer
                .find_selem(&format!("{base} Sens."), 0)
                .ok_or_else(|| Error::InvalidChannel(format!("No {base} Sens. control")))?;
            let wanted = match sensitivity {
                Sensitivity::Plus4dBu => "+4dBu",
                Sensitivity::Minus10dBV => "-10dBV",
            };
            let count = selem.get_enum_items()?;
            let item = (0..count)
                .find(|i| selem.get_enum_item_name(*i).is_ok_and(|n| n == wanted))
                .ok_or_else(|| {
                    Error::InvalidChannel(format!("{base} Sens. has no {wanted} setting"))
                })?;
            selem.set_enum_item(SelemChannelId::mono(), item)?;
            self.inputs[idx].sensitivity = Some(sensitivity);
            return Ok(());
        }

        let item = match sensitivity {
            Sensitivity::Plus4dBu => 0,
            Sensitivity::Minus10dBV => 1,
        };
        let selem = self
            .mixer
            .find_selem("Instrument Ref Level", 0)
            .ok_or_else(|| Error::InvalidChannel("No Ref Level control".into()))?;
        selem.set_enum_item(SelemChannelId::mono(), item)?;
        // Mirror across every Instrument channel — there's only one
        // real switch, so both must agree (matches `usb.rs`'s own
        // `set_ref_level` fix, same day).
        for c in self.inputs.iter_mut() {
            if c.channel_type == ChannelType::Instrument {
                c.sensitivity = Some(sensitivity);
            }
        }
        Ok(())
    }


    fn set_pitch(&mut self, pitch_percent: f32) -> Result<(), Error> {
        self.require_proprietary("Varispeed pitch")?;
        // "Varispeed Pitch" exists over ALSA too (mixer.c:1021-1030),
        // range -50..50 in 0.1%-steps (raw = percent * 10) — despite
        // this method's old comment claiming otherwise, this is not
        // USB-only.
        let clamped = pitch_percent.clamp(-5.0, 5.0);
        if let Some(selem) = self.mixer.find_selem("Varispeed Pitch", 0) {
            let raw = (clamped * 10.0).round() as i64;
            selem.set_playback_volume(SelemChannelId::mono(), raw)?;
        }
        self.settings.pitch_percent = clamped;
        Ok(())
    }

    fn set_loopback(&mut self, out: usize, on: bool) -> Result<(), Error> {
        self.require_proprietary("Loopback")?;
        if out >= PAIR_LABELS.len() {
            return Err(Error::InvalidChannel(format!("Output pair {}", out)));
        }
        if let Some(selem) = self.mixer.find_selem("Loopback", out as u32) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        if let Some(o) = self.outputs.get_mut(out * 2) {
            o.loopback = on;
        }
        if let Some(o) = self.outputs.get_mut(out * 2 + 1) {
            o.loopback = on;
        }
        Ok(())
    }

    fn set_ms_proc(&mut self, on: bool) -> Result<(), Error> {
        self.require_proprietary("MS processing")?;
        if let Some(selem) = self.mixer.find_selem("MS Processor", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.ms_proc = on;
        Ok(())
    }

    fn set_an12(&mut self, on: bool) -> Result<(), Error> {
        self.require_proprietary("AN 1>2")?;
        if let Some(selem) = self.mixer.find_selem("AN 1>2", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.an12 = on;
        Ok(())
    }

    fn set_input_link(&mut self, linked: bool) -> Result<(), Error> {
        self.require_proprietary("Input link")?;
        if let Some(selem) = self.mixer.find_selem("AN1/2 Link", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), linked as i32)?;
        }
        self.linked = linked;
        self.settings.input_link = linked;
        Ok(())
    }

    /// Phase Ø invert, AN1-4 only — `babyface-pro-linux`'s own
    /// per-mic `"<name> Phase" Switch` controls (added 2026-09-06,
    /// named directly from its own `bf_sources[]`, matching
    /// `BF_SOURCES` here). Errors rather than silently no-opping if
    /// missing (e.g. an older module without this control), since this
    /// is a real toggle a user would notice not taking effect.
    fn set_phase(&mut self, idx: usize, invert: bool) -> Result<(), Error> {
        self.require_proprietary("Phase invert")?;
        if idx >= 4 {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no phase switch"
            )));
        }
        let selem = self
            .mixer
            .find_selem(&format!("{} Phase", BF_SOURCES[idx]), idx as u32)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {idx} has no phase switch")))?;
        selem.set_playback_switch(SelemChannelId::mono(), invert as i32)?;
        self.inputs[idx].phase = invert;
        Ok(())
    }

    /// Stereo split (a playback pair routed hard-split into the AN1/2
    /// monitor bus instead of the normal stereo pair) — `babyface-pro-
    /// linux`'s own per-pair `"PBx Stereo Split" Switch` controls
    /// (added 2026-09-06). `pb` is a raw playback CHANNEL index (0-11,
    /// matching `RmeDevice::set_stereo_split`'s own convention); the
    /// pair index (0-5) is `pb / 2`.
    fn set_stereo_split(&mut self, pb: usize, split: bool) -> Result<(), Error> {
        self.require_proprietary("Stereo split")?;
        if pb >= self.playbacks.len() {
            return Err(Error::InvalidChannel(format!("Playback {pb}")));
        }
        let pair = pb / 2;
        let selem = self
            .mixer
            .find_selem(&format!("PB{} Stereo Split", pair + 1), pair as u32)
            .ok_or_else(|| {
                Error::InvalidChannel(format!("Playback {pb} has no stereo split switch"))
            })?;
        selem.set_playback_switch(SelemChannelId::mono(), split as i32)?;
        self.playbacks[pair * 2].split = split;
        self.playbacks[pair * 2 + 1].split = split;
        Ok(())
    }

    /// The trait default is a silent `Ok(())` no-op (see its own doc
    /// comment) — real now, targeting `babyface-pro-linux`'s own
    /// "<name> Trim Volume" controls (added 2026-09-06, the last of
    /// the driver's 5 upstream follow-ups). Restricted to AN1-4
    /// (`idx < 4`) even though the GUI's own `has_trim` gate is
    /// unconditional for every hardware input — PROTOCOL.md's Trim
    /// captures (`cap_trim2/3/4.pcap`) only ever verified the 4 analog
    /// inputs; the USB backend's own `set_trim` doesn't have this
    /// guard and would silently write into an unrelated channel's
    /// crosspoint if called on e.g. AS1/2 (flagged, not fixed here —
    /// out of scope for this backend's own wiring).
    fn set_trim(&mut self, idx: usize, db: f32) -> Result<(), Error> {
        self.require_proprietary("Trim")?;
        if idx >= 4 {
            return Err(Error::InvalidChannel(format!(
                "Input {idx} has no trim control"
            )));
        }
        let db = db.clamp(-65.0, 6.0).round();
        let selem = self
            .mixer
            .find_selem(&format!("{} Trim", BF_SOURCES[idx]), idx as u32)
            .ok_or_else(|| Error::InvalidChannel(format!("Input {idx} has no trim control")))?;
        selem.set_playback_volume(SelemChannelId::mono(), db as i64)?;
        self.inputs[idx].trim = db;
        Ok(())
    }

    fn set_dim(&mut self, on: bool) -> Result<(), Error> {
        self.require_proprietary("Dim")?;
        if let Some(selem) = self.mixer.find_selem("Dim", 0) {
            selem.set_playback_switch(SelemChannelId::mono(), on as i32)?;
        }
        self.settings.dim = on;
        Ok(())
    }

    fn set_width(&mut self, width: f32) -> Result<(), Error> {
        self.require_proprietary("Width")?;
        let clamped = width.clamp(-1.0, 1.0);
        if let Some(selem) = self.mixer.find_selem("Width", 0) {
            let raw = (clamped * 100.0).round() as i64;
            selem.set_playback_volume(SelemChannelId::mono(), raw)?;
        }
        self.settings.width = clamped;
        Ok(())
    }

    fn set_fx_send(&mut self, db: f32) -> Result<(), Error> {
        self.require_proprietary("FX send")?;
        let db = db.clamp(-65.0, 0.0);
        if let Some(selem) = self.mixer.find_selem("FX Send", 0) {
            let max = selem.get_playback_volume_range().1 as u16;
            let raw = if db <= -65.0 {
                0
            } else {
                fader_db_to_raw(db).min(max)
            };
            selem.set_playback_volume(SelemChannelId::mono(), raw as i64)?;
        }
        self.settings.fx_send_db = Some(db);
        Ok(())
    }



    fn set_optical_out_format(&mut self, spdif: bool) -> Result<(), Error> {
        self.require_proprietary("optical out format")?;
        let wanted = if spdif { "S/PDIF" } else { "ADAT" };
        let selem = self
            .mixer
            .find_selem("Optical Out Format", 0)
            .ok_or_else(|| Error::InvalidChannel("No Optical Out Format control".into()))?;
        let count = selem.get_enum_items()?;
        let item = (0..count)
            .find(|i| selem.get_enum_item_name(*i).is_ok_and(|n| n == wanted))
            .ok_or_else(|| {
                Error::InvalidChannel(format!("Optical Out Format has no {wanted} setting"))
            })?;
        selem.set_enum_item(SelemChannelId::mono(), item)?;
        self.settings.optical_out_spdif = spdif;
        Ok(())
    }

    fn set_clock_source(&mut self, source: &str) -> Result<(), Error> {
        let selem = self
            .mixer
            .find_selem("Sample Clock Source", 0)
            .ok_or_else(|| Error::InvalidChannel("No clock source control".into()))?;
        let count = selem.get_enum_items().unwrap_or(0);
        for idx in 0..count {
            if selem
                .get_enum_item_name(idx)
                .map(|n| n == source)
                .unwrap_or(false)
            {
                selem.set_enum_item(SelemChannelId::mono(), idx)?;
                self.settings.clock_source = source.to_string();
                return Ok(());
            }
        }
        Err(Error::InvalidChannel(format!(
            "Unknown clock source: {}",
            source
        )))
    }

    fn capture_scene(&self) -> Scene {
        Scene {
            name: "Untitled".into(),
            model: self.canonical_model().to_string(),
            inputs: self.inputs.clone(),
            playbacks: self.playbacks.clone(),
            outputs: self.outputs.clone(),
            settings: self.settings.clone(),
        }
    }

    fn apply_scene(&mut self, scene: &Scene) -> Result<(), Error> {
        scene.check_compatible(self.canonical_model())?;
        for (i, saved) in scene.inputs.iter().enumerate() {
            for (out, &v) in saved.volumes.iter().enumerate() {
                self.set_volume(ChannelId::Input(i), out, v)?;
            }
        }
        for (i, saved) in scene.playbacks.iter().enumerate() {
            for (out, &v) in saved.volumes.iter().enumerate() {
                self.set_volume(ChannelId::Playback(i), out, v)?;
            }
        }
        for (i, saved) in scene.outputs.iter().enumerate() {
            self.set_volume(ChannelId::Output(i), 0, saved.volume)?;
        }
        self.inputs = scene.inputs.clone();
        self.playbacks = scene.playbacks.clone();
        self.outputs = scene.outputs.clone();
        self.settings = scene.settings.clone();
        // Re-issue the actual ALSA writes for mute/solo/global toggles —
        // previously this only ever updated the in-memory model above
        // (`self.inputs`/etc.), so loading a scene looked right in the
        // GUI but never touched real hardware for anything except
        // volume. `set_mute`/`set_solo` read `self.inputs`/`self.
        // playbacks`/`self.outputs` to decide what to restore, so this
        // runs after the state's been adopted, not before.
        for i in 0..self.inputs.len() {
            self.set_mute(ChannelId::Input(i), scene.inputs[i].mute)?;
            self.set_solo(ChannelId::Input(i), scene.inputs[i].solo)?;
        }
        for i in 0..self.playbacks.len() {
            self.set_mute(ChannelId::Playback(i), scene.playbacks[i].mute)?;
            self.set_solo(ChannelId::Playback(i), scene.playbacks[i].solo)?;
        }
        for pair in 0..self.profile.output_pair_count() {
            if let Some(out) = scene.outputs.get(pair * 2) {
                self.set_mute(ChannelId::Output(pair * 2), out.mute)?;
                let _ = self.set_loopback(pair, out.loopback);
            }
        }
        // Best-effort: `ms_proc`/`an12`/`width`/`fx_send`/`clock_source`
        // are all real ALSA controls on this backend, but some (clock
        // source especially — see `PROTOCOL.md`/this session's own
        // finding that it's a CC-mode-only control surface under the
        // proprietary driver) may legitimately not exist depending on
        // which kernel driver mode is loaded. A missing optional control
        // shouldn't abort the rest of the scene load.
        let _ = self.set_ms_proc(self.settings.ms_proc);
        let _ = self.set_an12(self.settings.an12);
        if self.settings.width != 0.0 {
            let _ = self.set_width(self.settings.width);
        }
        if let Some(db) = self.settings.fx_send_db {
            let _ = self.set_fx_send(db);
        }
        if !self.settings.clock_source.is_empty() {
            let _ = self.set_clock_source(&self.settings.clock_source.clone());
        }
        Ok(())
    }

    fn poll_events(&mut self) -> Result<(), Error> {
        let now = std::time::Instant::now();
        if self.mixer.handle_events()? > 0 {
            self.attach_mixer_elements(true);
            self.resync_at = Some(now + RESYNC_SETTLE);
        } else if self.resync_at.is_some_and(|t| now >= t) {
            self.resync_at = None;
            self.attach_mixer_elements(true);
        }
        Ok(())
    }

    /// Real for every hardware input (AN1-AN4, AS1/2, ADAT3-8) via
    /// `capture_meter`; the GUI gates a failed capture open back to
    /// "N/A" through `DeviceHandle::has_input_meter`. `None` when the
    /// capture PCM could not be opened at all, or while the reader thread
    /// is reopening it after a PipeWire profile change, matching the
    /// trait's own contract. See `capture_meter`'s module doc comment for
    /// the live mapping behind `INPUT_FOR_CAPTURE`.
    fn meters(&self) -> Option<Vec<f32>> {
        let cm = self.capture_meter.as_ref()?;
        if !cm.is_connected() {
            return None;
        }
        let peaks = cm.drain();
        crate::capture_meter::CaptureMeter::debug_log(&peaks);
        let mut levels = vec![0.0; self.inputs.len()];
        for (cap, input) in peaks
            .iter()
            .zip(crate::capture_meter::INPUT_FOR_CAPTURE)
        {
            if let Some(i) = input {
                if let Some(slot) = levels.get_mut(i) {
                    *slot = *cap;
                }
            }
        }
        Some(levels)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn pan_split_and_join_round_trip_in_linear_levels() {
        // The dB-converted crosspoints pan on linear levels, louder side
        // at the volume: centre, both extremes and between, up to +6 dB.
        for &volume in &[0.25f32, 1.0, 1.5, 2.0] {
            for &pan in &[-100i8, -37, 0, 1, 64, 100] {
                let (l, r) = pan_split(volume, pan);
                assert!((l.max(r) - volume).abs() < 1e-6);
                let (v, p) = pan_join(l, r);
                assert!((v - volume).abs() < 1e-6);
                assert_eq!(p, pan, "volume {volume} pan {pan}");
            }
        }
        assert_eq!(pan_join(0.0, 0.0), (0.0, 0));
    }
    use super::*;

    const TEST_MAX: f32 = 65536.0;

    #[test]
    fn dim_toggles_on_an_odd_number_of_presses_only() {
        assert!(!dim_presses_toggle(5, 5));
        assert!(dim_presses_toggle(5, 6));
        assert!(!dim_presses_toggle(5, 7));
        assert!(dim_presses_toggle(5, 8));
    }

    #[test]
    fn dim_press_count_wraps_at_two_to_the_31() {
        let top = (1i64 << 31) - 1;
        // top -> 0 is one press, top -> 1 two.
        assert!(dim_presses_toggle(top, 0));
        assert!(!dim_presses_toggle(top, 1));
    }

    #[test]
    fn panel_enum_maps_to_the_usb_backend_selection() {
        assert_eq!(panel_enum_to_sel(0), None); // not known yet
        assert_eq!(panel_enum_to_sel(1), Some(0)); // Ch 1/2
        assert_eq!(panel_enum_to_sel(2), Some(1)); // Ch 3/4 / Phones
        assert_eq!(panel_enum_to_sel(3), Some(2)); // Opt
        assert_eq!(panel_enum_to_sel(4), None);
    }

    #[test]
    fn resync_ignores_rounding_but_not_a_real_change() {
        // A crosspoint step (full scale 0x2d41) is below the threshold...
        assert!(!resync_differs(0.5, 0.5 + 1.0 / 11585.0));
        // ...a wheel click (0.5 dB, about 6 %) is well above it.
        assert!(resync_differs(0.5, 0.53));
        assert!(resync_differs(0.0, 0.01));
    }

    #[test]
    fn decode_center_pan_from_equal_raw_volumes() {
        let (volume, pan) = decode_volume_pan(32768, 32768, TEST_MAX);
        assert!((volume - 0.5).abs() < 1e-4);
        assert_eq!(pan, 0);
    }

    #[test]
    fn decode_hard_left_from_silent_right() {
        let (volume, pan) = decode_volume_pan(65536, 0, TEST_MAX);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, -100);
    }

    #[test]
    fn decode_hard_right_from_silent_left() {
        let (volume, pan) = decode_volume_pan(0, 65536, TEST_MAX);
        assert!((volume - 1.0).abs() < 1e-4);
        assert_eq!(pan, 100);
    }

    #[test]
    fn decode_silence_defaults_to_center_pan() {
        let (volume, pan) = decode_volume_pan(0, 0, TEST_MAX);
        assert_eq!(volume, 0.0);
        assert_eq!(pan, 0);
    }

    #[test]
    fn encode_then_decode_round_trips_for_a_range_of_volume_pan_pairs() {
        for vol_pct in [0, 10, 25, 50, 75, 100] {
            for pan in [-100, -50, -25, 0, 25, 50, 100] {
                let volume = vol_pct as f32 / 100.0;
                let (l, r) = encode_volume_pan(volume, pan, TEST_MAX);
                let (decoded_vol, decoded_pan) = decode_volume_pan(l, r, TEST_MAX);
                assert!(
                    (decoded_vol - volume).abs() < 0.01,
                    "volume {} pan {}: decoded volume {}",
                    volume,
                    pan,
                    decoded_vol
                );
                // At volume 0 every raw pair is (0, 0), so pan is
                // undecidable — decode_volume_pan defaults it to 0
                // regardless of what was encoded.
                if vol_pct > 0 {
                    assert!(
                        (decoded_pan - pan).abs() <= 1,
                        "volume {} pan {}: decoded pan {}",
                        volume,
                        pan,
                        decoded_pan
                    );
                }
            }
        }
    }

    /// The identity probe's failure modes: a name that cannot be a card
    /// and an index no card occupies both resolve to no `devnum`, which
    /// `is_connected` reads as "not ours".
    #[test]
    fn card_devnum_is_none_for_a_missing_card() {
        assert!(AlsaMixer::card_devnum("nonsense").is_none());
        assert!(AlsaMixer::card_devnum("hw:99").is_none());
    }

    /// The "card is gone" half of the ALSA liveness probe: a name nothing
    /// matches must resolve to nothing, which is what makes the chip show
    /// Disconnected. (The "card is there" half needs the real card and is
    /// covered live.)
    #[test]
    fn resolve_card_finds_nothing_for_a_missing_name() {
        assert!(AlsaMixer::resolve_card("TuxMix No Such Card").is_none());
    }

    #[test]
    fn encode_center_pan_gives_equal_left_and_right() {
        let (l, r) = encode_volume_pan(0.75, 0, TEST_MAX);
        assert_eq!(l, r);
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_pan_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let cid = ChannelId::Input(1); // AN2, verified silent (0%) before running this
        let orig_vol = dev.volume(cid, 0).unwrap();
        let orig_pan = dev.pan(cid, 0).unwrap();

        dev.set_volume(cid, 0, 0.5).unwrap();
        dev.set_pan(cid, 0, 50).unwrap();
        assert!((dev.volume(cid, 0).unwrap() - 0.5).abs() < 0.02);
        assert!((dev.pan(cid, 0).unwrap() - 50).abs() <= 1);

        // Re-open to force a fresh attach_mixer_elements() read from
        // hardware, proving the round trip survives a real re-read,
        // not just the in-memory field set above.
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!((dev2.volume(cid, 0).unwrap() - 0.5).abs() < 0.02);
        assert!((dev2.pan(cid, 0).unwrap() - 50).abs() <= 1);
        drop(dev2);

        dev.set_volume(cid, 0, orig_vol).unwrap();
        dev.set_pan(cid, 0, orig_pan).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_sensitivity_round_trip() {
        // Was honestly-unmapped (this test used to assert it errored) —
        // now real, since babyface-pro-linux gained an "Instrument Ref
        // Level" control (2026-09-06). Verified via a fresh second
        // handle, forcing a real re-read, not the model this call just
        // updated itself.
        let mut dev = BabyfacePro::open().expect("real device attached");
        let idx = 2; // IN3 — Instrument-type, the only kind sensitivity applies to
        let orig = dev.inputs()[idx].sensitivity;

        dev.set_sensitivity(idx, Sensitivity::Minus10dBV).unwrap();
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.inputs()[idx].sensitivity, Some(Sensitivity::Minus10dBV));
        // Mirrored across both Instrument channels — there's only one
        // real switch (IN4 = idx 3).
        assert_eq!(dev2.inputs()[3].sensitivity, Some(Sensitivity::Minus10dBV));
        drop(dev2);

        dev.set_sensitivity(idx, Sensitivity::Plus4dBu).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev3.inputs()[idx].sensitivity, Some(Sensitivity::Plus4dBu));
        drop(dev3);

        if let Some(orig) = orig {
            dev.set_sensitivity(idx, orig).unwrap();
        }
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_phase_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let idx = 1; // AN2, verified silent (0%) before running this

        dev.set_phase(idx, true).unwrap();
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(dev2.inputs()[idx].phase);
        drop(dev2);

        dev.set_phase(idx, false).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(!dev3.inputs()[idx].phase);
        drop(dev3);
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_trim_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let idx = 1; // AN2, verified silent (0%) before running this
        let orig = dev.inputs()[idx].trim;

        dev.set_trim(idx, -5.0).unwrap();
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.inputs()[idx].trim, -5.0);
        drop(dev2);

        dev.set_trim(idx, 3.0).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev3.inputs()[idx].trim, 3.0);
        drop(dev3);

        dev.set_trim(idx, orig).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_input_phantom_mirrors_across_a_linked_pair() {
        // User-caught: toggling 48V on a linked AN1/2 strip only ever
        // touched its own channel (Message::Phantom called set_phantom
        // directly, no pair-awareness at all) — verified here against
        // the real card, not just mock, since this is orchestration
        // over already-hardware-verified set_phantom/input_pair_linked.
        let mut dev = BabyfacePro::open().expect("real device attached");
        let was_linked = dev.input_pair_linked(0);
        if !was_linked {
            dev.set_input_pair_linked(0, true).unwrap();
        }
        let orig0 = dev.inputs()[0].phantom;
        let orig1 = dev.inputs()[1].phantom;

        dev.set_input_phantom(0, false).unwrap();
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(!dev2.inputs()[0].phantom);
        assert!(!dev2.inputs()[1].phantom, "48V off must mirror to AN2 when linked");
        drop(dev2);

        dev.set_input_phantom(0, true).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(dev3.inputs()[0].phantom);
        assert!(dev3.inputs()[1].phantom, "48V on must mirror to AN2 when linked");
        drop(dev3);

        dev.set_phantom(0, orig0).unwrap();
        dev.set_phantom(1, orig1).unwrap();
        if !was_linked {
            dev.set_input_pair_linked(0, false).unwrap();
        }
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_stereo_split_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let pb = 0; // PB1 (playback channels 0/1)

        dev.set_stereo_split(pb, true).unwrap();
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(dev2.playbacks()[0].split);
        assert!(dev2.playbacks()[1].split, "split mirrors across both channels of the pair");
        drop(dev2);

        dev.set_stereo_split(pb, false).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(!dev3.playbacks()[0].split);
        assert!(!dev3.playbacks()[1].split);
        drop(dev3);
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_mute_zeroes_the_crosspoint_and_unmute_restores_it() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let cid = ChannelId::Input(1); // AN2, verified silent (0%) before running this
        let orig_vol = dev.volume(cid, 0).unwrap();
        dev.set_volume(cid, 0, 0.5).unwrap();

        dev.set_mute(cid, true).unwrap();
        // The model's own "real" volume is untouched by mute — needed
        // so unmute can restore it.
        assert!((dev.volume(cid, 0).unwrap() - 0.5).abs() < 0.02);
        // But the actual ALSA crosspoint register really is zeroed —
        // proven by re-reading it fresh (a new `attach_mixer_elements`
        // pass), not from the model that deliberately wasn't touched.
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(
            dev2.volume(cid, 0).unwrap() < 0.02,
            "crosspoint should read back near 0 while muted"
        );
        drop(dev2);

        dev.set_mute(cid, false).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(
            (dev3.volume(cid, 0).unwrap() - 0.5).abs() < 0.02,
            "crosspoint should be restored after unmute"
        );
        drop(dev3);

        dev.set_volume(cid, 0, orig_vol).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_solo_zeroes_other_channels_and_unsolo_restores_them() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let soloed = ChannelId::Input(0); // AN1
        let other = ChannelId::Input(1); // AN2, verified silent (0%) before running this
        let orig_soloed_vol = dev.volume(soloed, 0).unwrap();
        let orig_other_vol = dev.volume(other, 0).unwrap();
        dev.set_volume(soloed, 0, 0.5).unwrap();
        dev.set_volume(other, 0, 0.5).unwrap();

        dev.set_solo(soloed, true).unwrap();
        // The soloed channel's own level is untouched by soloing it —
        // matching `usb.rs`'s own hardware-validated behavior.
        assert!((dev.volume(soloed, 0).unwrap() - 0.5).abs() < 0.02);
        // The other channel's model volume is preserved for restore...
        assert!((dev.volume(other, 0).unwrap() - 0.5).abs() < 0.02);
        // ...but its actual crosspoint register reads back near 0,
        // proven the same way as the mute test above.
        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(
            (dev2.volume(soloed, 0).unwrap() - 0.5).abs() < 0.02,
            "soloed channel's own level must be untouched"
        );
        assert!(
            dev2.volume(other, 0).unwrap() < 0.02,
            "non-soloed channel should read back near 0 while something else is soloed"
        );
        drop(dev2);

        dev.set_solo(soloed, false).unwrap();
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(
            (dev3.volume(other, 0).unwrap() - 0.5).abs() < 0.02,
            "un-solo should restore the other channel's crosspoint"
        );
        drop(dev3);

        dev.set_volume(soloed, 0, orig_soloed_vol).unwrap();
        dev.set_volume(other, 0, orig_other_vol).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_apply_scene_reissues_mute_and_loopback_not_just_the_model() {
        // Previously `apply_scene` only ever updated `self.inputs`/
        // `self.outputs`/etc — a loaded scene looked right in the GUI
        // but never touched real hardware for anything besides volume.
        // Proves the fix the same way every other live_hardware test in
        // this file does: re-open a second handle to force a fresh ALSA
        // read, rather than trusting the in-memory model.
        let mut dev = BabyfacePro::open().expect("real device attached");
        let baseline = dev.capture_scene();

        let mut modified = baseline.clone();
        modified.inputs[1].mute = true; // AN2, verified silent (0%) before running this
        modified.outputs[0].loopback = true; // AN1/2 pair

        dev.apply_scene(&modified).expect("apply_scene should succeed");

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!(
            dev2.outputs[0].loopback,
            "loopback should read back engaged after apply_scene, not just in the model"
        );
        assert!(
            dev2.volume(ChannelId::Input(1), 0).unwrap() < 0.02,
            "AN2 crosspoint should read back near 0 while apply_scene's mute is engaged"
        );
        drop(dev2);

        dev.apply_scene(&baseline).expect("restore baseline");
        let dev3 = BabyfacePro::open().expect("real device attached");
        assert!(
            !dev3.outputs[0].loopback,
            "loopback should be restored off after re-applying the baseline scene"
        );
        drop(dev3);
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached AND switched to \
                Class Compliant mode (snd-usb-audio, USB 2a39:3fb0); run \
                manually with --ignored"]
    fn live_hardware_cc_grammar_is_detected_and_read_faithfully() {
        // This replaces an earlier test that asserted `open()` *refused*
        // a CC card. That was right for the few hours between adding the
        // guard and teaching the backend the CC grammar; now the card is
        // supported, and what matters is that the state it reports is the
        // card's own rather than model defaults — the failure mode that
        // started all of this (48V read off while the card had it on).
        let dev = BabyfacePro::open().expect("CC-mode card should now open");
        assert_eq!(dev.grammar, ControlGrammar::ClassCompliant);
        assert!(
            !dev.protocol_features(),
            "the CC personality drives routing, preamps and clock only; every \
             RME-protocol feature is missing, and this is what the UIs gate on"
        );

        // Ranges come from the hardware and differ per preamp law, so
        // they prove these are real reads: mics 0..65, instruments 0..18.
        assert_eq!(dev.inputs()[0].gain_max, Some(65), "mic gain law");
        assert_eq!(dev.inputs()[2].gain_max, Some(18), "instrument gain law");
        for i in 2..4 {
            assert!(
                dev.inputs()[i].sensitivity.is_some(),
                "input {i} sensitivity must be read from the card, not defaulted"
            );
        }
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached AND switched to \
                Class Compliant mode (snd-usb-audio, USB 2a39:3fb0); run \
                manually with --ignored"]
    fn live_hardware_cc_grammar_round_trips_through_a_fresh_handle() {
        // Every check re-opens the device, so a value that only ever
        // reached the in-memory model would fail here.
        let mut dev = BabyfacePro::open().expect("CC-mode card");

        // A paired source writes only its own side of the output pair.
        let orig = dev.volume(ChannelId::Input(4), 1).unwrap();
        dev.set_volume(ChannelId::Input(4), 1, 0.42).unwrap();
        assert!((BabyfacePro::open().unwrap().volume(ChannelId::Input(4), 1).unwrap() - 0.42).abs() < 0.01);
        dev.set_volume(ChannelId::Input(4), 1, orig).unwrap();

        // A mono source's pan is two separate controls in this grammar.
        let (ov, op) = (
            dev.volume(ChannelId::Input(0), 0).unwrap(),
            dev.pan(ChannelId::Input(0), 0).unwrap(),
        );
        dev.set_volume(ChannelId::Input(0), 0, 0.8).unwrap();
        dev.set_pan(ChannelId::Input(0), 0, -50).unwrap();
        assert!((BabyfacePro::open().unwrap().pan(ChannelId::Input(0), 0).unwrap() + 50).abs() <= 2);
        dev.set_volume(ChannelId::Input(0), 0, ov).unwrap();
        dev.set_pan(ChannelId::Input(0), 0, op).unwrap();

        // Sensitivity is per-channel here (the proprietary control is a
        // single shared switch) and its enum items are in the opposite
        // order, so this also guards the resolve-by-name fix.
        let orig_s = dev.inputs()[2].sensitivity.unwrap();
        let other = match orig_s {
            Sensitivity::Plus4dBu => Sensitivity::Minus10dBV,
            Sensitivity::Minus10dBV => Sensitivity::Plus4dBu,
        };
        dev.set_sensitivity(2, other).unwrap();
        let f = BabyfacePro::open().unwrap();
        assert_eq!(f.inputs()[2].sensitivity, Some(other), "IN3 must change");
        assert_eq!(f.inputs()[3].sensitivity, Some(orig_s), "IN4 must NOT mirror");
        drop(f);
        dev.set_sensitivity(2, orig_s).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached AND switched to \
                Class Compliant mode (snd-usb-audio, USB 2a39:3fb0); run \
                manually with --ignored"]
    fn live_hardware_cc_grammar_refuses_proprietary_only_features() {
        // These all look their controls up with a tolerant `if let
        // Some(selem)`, so without the guard they would return Ok(())
        // having done nothing — the failure this whole day was about.
        let mut dev = BabyfacePro::open().expect("CC-mode card");
        assert!(dev.set_phase(0, true).is_err(), "phase");
        assert!(dev.set_trim(0, 3.0).is_err(), "trim");
        assert!(dev.set_loopback(0, true).is_err(), "loopback");
        assert!(dev.set_width(0.5).is_err(), "width");
        assert!(dev.set_dim(true).is_err(), "dim");
        assert!(dev.set_pitch(1.0).is_err(), "pitch");
        assert!(dev.set_eq_enabled(0, true).is_err(), "eq");
        assert!(dev.set_ms_proc(true).is_err(), "ms proc");
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached AND running in \
                Class Compliant mode (snd-usb-audio) — the from-scratch \
                snd-usb-babyface-pro driver has no Sample Clock Source \
                control yet (its README lists clock source as a future \
                follow-up, not yet implemented); run manually with \
                --ignored"]
    fn live_hardware_clock_source_round_trip() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        let orig = dev.settings().clock_source.clone();
        let sources = dev.settings().clock_sources.clone();
        let other = sources
            .iter()
            .find(|s| **s != orig)
            .expect("hardware exposes more than one clock source")
            .clone();

        dev.set_clock_source(&other).unwrap();
        assert_eq!(dev.settings().clock_source, other);

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert_eq!(dev2.settings().clock_source, other);
        drop(dev2);

        dev.set_clock_source(&orig).unwrap();
    }

    #[test]
    #[ignore = "requires the real Babyface Pro FS attached; run manually with --ignored"]
    fn live_hardware_output_volume_beyond_sixth_pair_does_not_panic_or_clobber_a_sibling() {
        let mut dev = BabyfacePro::open().expect("real device attached");
        // idx=8 (ADAT5 OUT, the 9th individual output channel) is the
        // exact index reported crashing in the wild: `self.outputs` has
        // 12 individual-channel entries, but the old code indexed
        // `self.profile.outputs` (6 *pairs*) with the same raw `idx`,
        // panicking for any idx >= 6. idx=9 (ADAT6 OUT) is idx=8's pair
        // sibling — the old code, for idx in 2..=5, also silently wrote
        // *both* channels of the wrong pair, so round-tripping idx=8
        // alone and confirming idx=9 didn't move covers both failure
        // modes in one test.
        let target = ChannelId::Output(8);
        let sibling = ChannelId::Output(9);
        let orig_target = dev.volume(target, 0).unwrap();
        let orig_sibling = dev.volume(sibling, 0).unwrap();

        dev.set_volume(target, 0, 0.5).unwrap();
        assert!((dev.volume(target, 0).unwrap() - 0.5).abs() < 0.02);
        assert!(
            (dev.volume(sibling, 0).unwrap() - orig_sibling).abs() < 0.02,
            "writing Output(8) should not move Output(9)'s volume"
        );

        let dev2 = BabyfacePro::open().expect("real device attached");
        assert!((dev2.volume(target, 0).unwrap() - 0.5).abs() < 0.02);
        drop(dev2);

        dev.set_volume(target, 0, orig_target).unwrap();
    }
}
