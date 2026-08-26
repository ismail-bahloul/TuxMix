//! Declarative description of an RME device controllable via ALSA
//! simple-mixer elements (selem) — see `crate::profiles` for the
//! per-model instances of [`DeviceProfile`].
//!
//! This covers the "ALSA-selem" family of RME devices only (the
//! Babyface Pro and similar USB class-compliant interfaces). Newer
//! "TotalMix FX 2"-generation devices (Fireface UCX II, UFX+/III)
//! don't expose ALSA mixer elements at all — they use a proprietary
//! MIDI SysEx protocol and would need an entirely different backend,
//! not a `DeviceProfile`.

use crate::channel::{ChannelType, InputChannel, OutputChannel, PlaybackChannel, Sensitivity};

/// One RME model's channel topology: card detection substring, model
/// name, and per-input/per-output-pair naming and typing. Each
/// supported model is one `const DeviceProfile` (see
/// `crate::profiles::babyface_pro` for the reference instance).
///
/// This intentionally does *not* parameterize ALSA control-naming
/// patterns (`selem_name`/`ch_type_str` in `babyface.rs`) — with only
/// one real device implemented, we don't yet know whether that naming
/// grammar is shared across other RME models or Babyface-specific.
/// Adding a naming-strategy field here is deferred until a second real
/// device confirms which it is.
#[derive(Debug, Clone, Copy)]
pub struct DeviceProfile {
    /// Substring matched against `/proc/asound/cards` entries.
    pub card_substring: &'static str,
    pub model_name: &'static str,
    /// One entry per physical hardware input, in channel-id order.
    pub inputs: &'static [InputSpec],
    /// One entry per physical stereo output pair.
    pub outputs: &'static [OutputPairSpec],
}

#[derive(Debug, Clone, Copy)]
pub struct InputSpec {
    pub name: &'static str,
    pub channel_type: ChannelType,
}

#[derive(Debug, Clone, Copy)]
pub struct OutputPairSpec {
    pub left: &'static str,
    pub right: &'static str,
}

impl DeviceProfile {
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    pub fn output_pair_count(&self) -> usize {
        self.outputs.len()
    }

    /// Instrument inputs default to +4dBu sensitivity; every other
    /// input type has no sensitivity switch at all.
    pub fn build_inputs(&self) -> Vec<InputChannel> {
        self.inputs
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let mut ch =
                    InputChannel::new(i, spec.name, spec.channel_type, self.output_pair_count());
                ch.sensitivity = if spec.channel_type == ChannelType::Instrument {
                    Some(Sensitivity::Plus4dBu)
                } else {
                    None
                };
                ch
            })
            .collect()
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
