//! Babyface Pro FS device profile — channel topology and card
//! detection, consumed by both [`crate::BabyfacePro`] (real hardware)
//! and [`crate::MockBabyfacePro`] (simulation), so they can't drift
//! apart the way the old hand-duplicated const tables did.

use crate::channel::ChannelType;
use crate::profile::{DeviceProfile, InputSpec, OutputPairSpec};

pub const PROFILE: DeviceProfile = DeviceProfile {
    card_substring: "Babyface Pro",
    model_name: "Babyface Pro FS",
    inputs: &[
        InputSpec {
            name: "AN1",
            channel_type: ChannelType::Mic,
        },
        InputSpec {
            name: "AN2",
            channel_type: ChannelType::Mic,
        },
        InputSpec {
            name: "IN3",
            channel_type: ChannelType::Instrument,
        },
        InputSpec {
            name: "IN4",
            channel_type: ChannelType::Instrument,
        },
        InputSpec {
            name: "AS1",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "AS2",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "ADAT3",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT4",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT5",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT6",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT7",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT8",
            channel_type: ChannelType::ADAT,
        },
    ],
    outputs: &[
        OutputPairSpec {
            left: "AN1",
            right: "AN2",
        },
        OutputPairSpec {
            left: "PH3",
            right: "PH4",
        },
        OutputPairSpec {
            left: "AS1",
            right: "AS2",
        },
        OutputPairSpec {
            left: "ADAT3",
            right: "ADAT4",
        },
        OutputPairSpec {
            left: "ADAT5",
            right: "ADAT6",
        },
        OutputPairSpec {
            left: "ADAT7",
            right: "ADAT8",
        },
    ],
};
