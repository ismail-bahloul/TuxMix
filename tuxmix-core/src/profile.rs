// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Declarative description of one RME model: its channels, what each
//! channel can do, and the device-wide facts the UI needs (see
//! `crate::profiles` for the per-model instances of [`DeviceProfile`]).
//!
//! A profile is data only. It says what the hardware has (a preamp with a
//! 0-65 dB range, a 48V switch, three reference levels), never how a
//! backend reaches it: register maps and wire encodings stay in the
//! backends. That split is what lets one UI serve several models: the
//! strips and the simulation read the shape of a unit from here instead
//! of testing `channel_type` or the model name.
//!
//! RME's USB units speak one of two control protocols, and a profile names
//! the one its unit uses ([`ControlProtocol`]). Only the Babyface Pro FS is
//! driven by a backend today. Every other profile is
//! [`Support::SimulationOnly`]: `--mock` can show it, nothing writes to
//! such a unit.

use crate::channel::{
    ChannelType, InputChannel, InputEq, OutputChannel, PlaybackChannel, Sensitivity,
};

/// One RME model's channel topology and capabilities. Each model is one
/// `const DeviceProfile` (see `crate::profiles::babyface_pro` for the
/// reference instance, and `crate::profiles::ALL` for the list).
///
/// This intentionally does *not* parameterize ALSA control-naming
/// patterns (`selem_name`/`ch_type_str` in `babyface.rs`): those belong to
/// the Babyface Pro backend, the only one that reads ALSA controls.
#[derive(Debug, Clone, Copy)]
pub struct DeviceProfile {
    /// Stable short identifier (`"babyface_pro"`, `"ucx"`, ...), used to
    /// pick a profile for the simulation (`TUXMIX_MOCK_MODEL`).
    pub id: &'static str,
    /// Substring matched against `/proc/asound/cards` entries.
    pub card_substring: &'static str,
    pub model_name: &'static str,
    /// One entry per physical hardware input, in channel-id order.
    pub inputs: &'static [InputSpec],
    /// One entry per physical stereo output pair.
    pub outputs: &'static [OutputPairSpec],
    /// The output pair TotalMix shows as Main in its Control Room, if the
    /// unit has one; `None` hides the Control Room's Main strip.
    pub main_pair: Option<usize>,
    /// The USB identities the unit enumerates with, one per mode.
    pub usb: &'static [UsbIdentity],
    /// How the unit's mixer is controlled in its vendor mode.
    pub protocol: ControlProtocol,
    /// Device-wide features beyond the per-channel ones.
    pub features: DeviceFeatures,
    /// Whether a TuxMix backend drives this unit.
    pub support: Support,
    /// Where the profile's data comes from, for whoever has to check or
    /// extend it.
    pub source: &'static str,
}

/// A USB vendor/product pair and the mode it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbIdentity {
    pub vendor: u16,
    pub product: u16,
    pub mode: UsbMode,
}

impl UsbIdentity {
    pub const fn vendor_mode(vendor: u16, product: u16) -> Self {
        Self {
            vendor,
            product,
            mode: UsbMode::Vendor,
        }
    }

    pub const fn class_compliant(vendor: u16, product: u16) -> Self {
        Self {
            vendor,
            product,
            mode: UsbMode::ClassCompliant,
        }
    }
}

/// The modes an RME USB unit can enumerate in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbMode {
    /// RME's own driver mode (TotalMix on Windows, the vendor protocol).
    Vendor,
    /// USB class compliant: `snd-usb-audio` plays audio; the mixer is
    /// reached, on the units that allow it, over MIDI SysEx.
    ClassCompliant,
}

/// The control protocol family a unit speaks in vendor mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlProtocol {
    /// Vendor requests on endpoint 0 with 16/18-bit values and an address
    /// map per model (Babyface Pro, Babyface classic, Fireface UC).
    VendorRequests,
    /// 32-bit DSP words with an odd-parity bit, on a bulk endpoint pair:
    /// RME's "latter" protocol, shared with the FireWire Firefaces
    /// (Fireface UCX, 802, the UFX family, UCX II); see
    /// `crate::latter`. `refresh` is the value written to
    /// [`crate::latter::REG_REFRESH`] to ask the unit for every register.
    Latter { refresh: u16 },
}

/// Device-wide features, as TotalMix exposes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceFeatures {
    /// Direct USB recording to a stick (DURec).
    pub durec: bool,
    /// Room EQ on the outputs (TotalMix FX 2 generation).
    pub room_eq: bool,
    /// The submix crosspoints carry a volume only, no pan.
    pub mix_volume_only: bool,
}

/// Whether a backend drives the unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// A backend reads and writes the unit; checked on hardware.
    Driven,
    /// Topology only, for `--mock`: nothing in TuxMix talks to this unit.
    SimulationOnly,
}

/// A preamp gain range, in whole dB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainRange {
    pub min_db: u32,
    pub max_db: u32,
}

impl GainRange {
    pub const fn db(min_db: u32, max_db: u32) -> Self {
        Self { min_db, max_db }
    }
}

/// What one physical input can do, beyond the volume, pan, mute, solo,
/// phase and trim every input has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputCaps {
    /// Preamp gain, `None` when the input has none.
    pub gain: Option<GainRange>,
    /// 48V phantom power switch.
    pub phantom: bool,
    /// Pad switch.
    pub pad: bool,
    /// Instrument (Hi-Z) switch.
    pub hi_z: bool,
    /// AutoSet (automatic gain reduction against clipping).
    pub autoset: bool,
    /// Reference levels the input can be switched between, in the unit's
    /// own order; empty when there is no switch.
    pub ref_levels: &'static [&'static str],
    /// A hardware EQ strip.
    pub eq: bool,
}

impl InputCaps {
    /// No switch, no preamp: a digital input, or a fixed line input.
    pub const NONE: Self = Self {
        gain: None,
        phantom: false,
        pad: false,
        hi_z: false,
        autoset: false,
        ref_levels: &[],
        eq: false,
    };

    /// Whether the reference-level switch includes the +4 dBu / -10 dBV
    /// pair that [`Sensitivity`] models.
    pub fn has_sensitivity_switch(&self) -> bool {
        self.ref_levels.contains(&"+4dBu") && self.ref_levels.contains(&"-10dBV")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputSpec {
    pub name: &'static str,
    pub channel_type: ChannelType,
    pub caps: InputCaps,
}

impl InputSpec {
    /// An input with the given capabilities.
    pub const fn new(name: &'static str, channel_type: ChannelType, caps: InputCaps) -> Self {
        Self {
            name,
            channel_type,
            caps,
        }
    }

    /// An input with nothing to switch: digital, or a fixed line input.
    pub const fn plain(name: &'static str, channel_type: ChannelType) -> Self {
        Self::new(name, channel_type, InputCaps::NONE)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OutputPairSpec {
    pub left: &'static str,
    pub right: &'static str,
    /// The pair's name as a bus, in submix pickers and matrix headers
    /// ("AN1/2").
    pub label: &'static str,
    /// Reference levels the pair can be switched between, in the unit's
    /// own order; empty when there is no switch.
    pub ref_levels: &'static [&'static str],
}

impl OutputPairSpec {
    /// An output pair with no reference-level switch.
    pub const fn new(left: &'static str, right: &'static str, label: &'static str) -> Self {
        Self::with_ref_levels(left, right, label, &[])
    }

    /// An output pair with a reference-level switch.
    pub const fn with_ref_levels(
        left: &'static str,
        right: &'static str,
        label: &'static str,
        ref_levels: &'static [&'static str],
    ) -> Self {
        Self {
            left,
            right,
            label,
            ref_levels,
        }
    }
}

impl DeviceProfile {
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    pub fn output_pair_count(&self) -> usize {
        self.outputs.len()
    }

    /// The bus name of output pair `pair`, or `""` past the last pair.
    pub fn pair_label(&self, pair: usize) -> &'static str {
        self.outputs.get(pair).map_or("", |o| o.label)
    }

    /// Every output pair's bus name, in pair order.
    pub fn pair_labels(&self) -> Vec<&'static str> {
        self.outputs.iter().map(|o| o.label).collect()
    }

    /// The capabilities of input `idx`, [`InputCaps::NONE`] past the end.
    pub fn input_caps(&self, idx: usize) -> InputCaps {
        self.inputs.get(idx).map_or(InputCaps::NONE, |s| s.caps)
    }

    /// The mode the unit is in when it enumerates as `vendor:product`, or
    /// `None` if that is not this unit.
    pub fn usb_mode(&self, vendor: u16, product: u16) -> Option<UsbMode> {
        self.usb
            .iter()
            .find(|u| u.vendor == vendor && u.product == product)
            .map(|u| u.mode)
    }

    /// The inputs' names and types, with the sensitivity switch (where
    /// there is one) at +4 dBu. Preamp gains and EQ strips stay unset:
    /// a backend fills them from what the unit reports, so an input
    /// shows a control only when the unit has answered for it. See
    /// [`Self::build_simulated_inputs`] for the simulation's version.
    pub fn build_inputs(&self) -> Vec<InputChannel> {
        self.inputs
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let mut ch =
                    InputChannel::new(i, spec.name, spec.channel_type, self.output_pair_count());
                ch.sensitivity = spec
                    .caps
                    .has_sensitivity_switch()
                    .then_some(Sensitivity::Plus4dBu);
                ch
            })
            .collect()
    }

    /// [`Self::build_inputs`], plus every control the profile declares:
    /// each preamp at the bottom of its range, each EQ strip present and
    /// off. What the simulation shows in place of a unit's answers.
    pub fn build_simulated_inputs(&self) -> Vec<InputChannel> {
        let mut inputs = self.build_inputs();
        for (ch, spec) in inputs.iter_mut().zip(self.inputs) {
            ch.gain_max = spec.caps.gain.map(|g| g.max_db);
            ch.gain = spec.caps.gain.map(|g| g.min_db);
            ch.eq = spec.caps.eq.then(InputEq::default);
        }
        inputs
    }

    pub fn build_playbacks(&self) -> Vec<PlaybackChannel> {
        let mut pbs = Vec::new();
        for (i, pair) in self.outputs.iter().enumerate() {
            pbs.push(PlaybackChannel::new(
                i * 2,
                &format!("PCM {}", pair.left),
                self.output_pair_count(),
            ));
            pbs.push(PlaybackChannel::new(
                i * 2 + 1,
                &format!("PCM {}", pair.right),
                self.output_pair_count(),
            ));
        }
        pbs
    }

    pub fn build_outputs(&self) -> Vec<OutputChannel> {
        let mut outs = Vec::new();
        for (i, pair) in self.outputs.iter().enumerate() {
            outs.push(OutputChannel::new(i * 2, &format!("OUT {}", pair.left)));
            outs.push(OutputChannel::new(
                i * 2 + 1,
                &format!("OUT {}", pair.right),
            ));
        }
        outs
    }
}
