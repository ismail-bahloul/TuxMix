// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Fireface UCX channel topology, for the simulation only.
//!
//! No backend drives a UCX yet. This profile lets the mock (and so the
//! GUI) run with another unit's layout, to find what still assumes the
//! Babyface Pro. The topology is the one documented publicly for the UCX
//! in TotalMix order: 8 analog inputs (1/2 mic, 3/4 instrument, 5-8 line),
//! S/PDIF and 8 ADAT in; AN1/2 to AN5/6, phones on 7/8, S/PDIF and ADAT
//! out. Not checked on a unit.

use crate::channel::ChannelType;
use crate::profile::{DeviceProfile, InputSpec, OutputPairSpec};

pub const PROFILE: DeviceProfile = DeviceProfile {
    card_substring: "Fireface UCX",
    model_name: "Fireface UCX",
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
            name: "AN3",
            channel_type: ChannelType::Instrument,
        },
        InputSpec {
            name: "AN4",
            channel_type: ChannelType::Instrument,
        },
        InputSpec {
            name: "AN5",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "AN6",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "AN7",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "AN8",
            channel_type: ChannelType::Line,
        },
        InputSpec {
            name: "SPDIF L",
            channel_type: ChannelType::SPDIF,
        },
        InputSpec {
            name: "SPDIF R",
            channel_type: ChannelType::SPDIF,
        },
        InputSpec {
            name: "ADAT1",
            channel_type: ChannelType::ADAT,
        },
        InputSpec {
            name: "ADAT2",
            channel_type: ChannelType::ADAT,
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
            label: "AN1/2",
        },
        OutputPairSpec {
            left: "AN3",
            right: "AN4",
            label: "AN3/4",
        },
        OutputPairSpec {
            left: "AN5",
            right: "AN6",
            label: "AN5/6",
        },
        OutputPairSpec {
            left: "PH7",
            right: "PH8",
            label: "PH7/8",
        },
        OutputPairSpec {
            left: "SPDIF L",
            right: "SPDIF R",
            label: "SPDIF",
        },
        OutputPairSpec {
            left: "AS1",
            right: "AS2",
            label: "AS1/2",
        },
        OutputPairSpec {
            left: "ADAT3",
            right: "ADAT4",
            label: "ADAT3/4",
        },
        OutputPairSpec {
            left: "ADAT5",
            right: "ADAT6",
            label: "ADAT5/6",
        },
        OutputPairSpec {
            left: "ADAT7",
            right: "ADAT8",
            label: "ADAT7/8",
        },
    ],
    // Which pair TotalMix shows as Main on a UCX is not checked: no Main
    // until it is.
    main_pair: None,
};
