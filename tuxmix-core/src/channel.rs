use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Identifies a specific channel on an RME device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
pub enum ChannelId {
    Input(usize),
    Playback(usize),
    Output(usize),
}

/// The type of an input channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
pub enum ChannelType {
    Mic,
    Instrument,
    Line,
    SPDIF,
    ADAT,
}

/// A single physical hardware input channel.
///
/// Each channel can be routed to every hardware output pair with
/// its own volume and pan — this is the submix (matrix) model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputChannel {
    pub id: usize,
    pub name: String,
    pub channel_type: ChannelType,
    /// Volume per output pair (0.0 – 1.0). Length = number of output pairs.
    pub volumes: Vec<f32>,
    /// Pan per output pair (-100 .. 100). Length = number of output pairs.
    pub pans: Vec<i8>,
    pub phantom: bool, // 48V
    pub pad: bool,
    pub sensitivity: Option<Sensitivity>,
    /// Preamp gain in dB (device-specific range — see `gain_max`).
    /// `None` for channel types with no gain control
    /// (Line/SPDIF/ADAT).
    pub gain: Option<u32>,
    /// Hardware-reported maximum for `gain`, read from the ALSA
    /// control's own range at attach time. `None` alongside `gain`.
    pub gain_max: Option<u32>,
    /// Phase Ø invert flag (bitwise-NOT of the crosspoint coefficient).
    /// `#[serde(default)]` so old scene JSON still loads.
    #[serde(default)]
    pub phase: bool,
    /// Ref-level code (Instr 3/4: +4dBu / -10dBV / Boost) as the raw
    /// Ref-level code (Instr 3/4): 0 = unset, 1 = +4dBu, 2 = -10dBV,
    /// 3 = Boost (see `usb::REF_*`; labeled cap_reflevel2.pcap).
    #[serde(default)]
    pub ref_level: u16,
    /// Hardware 3-band + low-cut DSP EQ. `Some` only for the 4 analog
    /// inputs (AN1-AN4 on the Babyface Pro FS — the only channels with
    /// an EQ strip); `None` for every other input type.
    /// `#[serde(default)]` so old scene JSON without EQ still loads.
    #[serde(default)]
    pub eq: Option<InputEq>,
    pub mute: bool,
    pub solo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
pub enum Sensitivity {
    Minus10dBV,
    Plus4dBu,
}

/// One EQ band's type (`mixer.c`'s `bf_eq_type_texts`: "Off"/"Bell"/
/// "Low Shelf"/"High Shelf" — enum items 0-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, Default)]
pub enum EqBandType {
    #[default]
    Off,
    Bell,
    LowShelf,
    HighShelf,
}

/// One of the 3 parametric bands in an [`InputEq`] strip.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    pub band_type: EqBandType,
    /// 20 – 20000 Hz.
    pub freq_hz: u16,
    /// Q factor, 0.05 – 10.0 (ALSA raw is Q x 100).
    pub q: f32,
    /// -24.0 – +24.0 dB (ALSA raw is dB x 10).
    pub gain_db: f32,
}

impl Default for EqBand {
    fn default() -> Self {
        Self {
            band_type: EqBandType::Off,
            freq_hz: 1000,
            q: 0.7,
            gain_db: 0.0,
        }
    }
}

/// Hardware 3-band parametric EQ + low cut on one analog input strip
/// (`eq.c`: 3 bands, a shared enable, and a low-cut filter).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct InputEq {
    pub enabled: bool,
    pub bands: [EqBand; 3],
    /// 20 – 20000 Hz.
    pub low_cut_freq_hz: u16,
    /// 6, 12, 18, or 24 dB/octave.
    pub low_cut_slope_db_oct: u8,
}

impl Default for InputEq {
    fn default() -> Self {
        Self {
            enabled: false,
            bands: [EqBand::default(); 3],
            low_cut_freq_hz: 20,
            low_cut_slope_db_oct: 6,
        }
    }
}

/// A single software playback channel (from the computer to the device).
///
/// Same submix model: one volume + pan per hardware output pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackChannel {
    pub id: usize,
    pub name: String,
    /// Volume per output pair (0.0 – 1.0).
    pub volumes: Vec<f32>,
    /// Pan per output pair (-100 .. 100).
    pub pans: Vec<i8>,
    /// Stereo-split flag: the strip's L/R crosspoints are rewritten to
    /// 0x2000/0x0000 (split mono) instead of 0x1000/0x1000 (-6 dB pair).
    #[serde(default)]
    pub split: bool,
    pub mute: bool,
    pub solo: bool,
}

/// A physical hardware output (stereo pair) on the device.
///
/// Each output has a master volume, mute, and solo — the "bottom row"
/// in TotalMix. Inputs and playbacks route INTO these outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputChannel {
    pub id: usize,
    pub name: String,
    /// Master volume for this output (0.0 – 1.0).
    pub volume: f32,
    /// Loopback flag (the input signal is fed back into the playback
    /// path) — bReq 0x15, one mono channel per side (AN1/2 = ch 0/1).
    #[serde(default)]
    pub loopback: bool,
    pub mute: bool,
    pub solo: bool,
}

impl InputChannel {
    pub fn new(id: usize, name: &str, channel_type: ChannelType, outputs: usize) -> Self {
        Self {
            id,
            name: name.to_string(),
            channel_type,
            volumes: vec![1.0; outputs],
            pans: vec![0; outputs],
            phantom: false,
            pad: false,
            sensitivity: None,
            gain: None,
            gain_max: None,
            phase: false,
            ref_level: 0,
            eq: None,
            mute: false,
            solo: false,
        }
    }
}

impl PlaybackChannel {
    pub fn new(id: usize, name: &str, outputs: usize) -> Self {
        Self {
            id,
            name: name.to_string(),
            volumes: vec![1.0; outputs],
            pans: vec![0; outputs],
            split: false,
            mute: false,
            solo: false,
        }
    }
}

impl OutputChannel {
    pub fn new(id: usize, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            volume: 1.0,
            loopback: false,
            mute: false,
            solo: false,
        }
    }
}
