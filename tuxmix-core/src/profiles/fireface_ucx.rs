// SPDX-License-Identifier: MIT AND ISC
// Copyright (c) 2026 Ismaïl Bahloul
// Channel table derived from oscmix `device_ffucx.c`:
// Copyright (c) 2023-2024 Michael Forney, ISC license (see
// `LICENSES/oscmix-ISC.txt`).
// Taken from the charlysound/oscmix-ufxplus-linux fork of oscmix.

//! Fireface UCX channel topology, for the simulation only.
//!
//! No backend drives a Fireface UCX yet: this profile lets `--mock` show the
//! unit's layout. Generated from oscmix's table for the unit (the charlysound fork; upstream oscmix has no table for this unit),
//! by `tools/profiles/gen_oscmix_profiles.py`; edit the generator, not
//! this file. Not checked on a unit.

use crate::channel::ChannelType;
use crate::profile::{
    ControlProtocol, DeviceFeatures, DeviceProfile, GainRange, InputCaps, InputSpec,
    OutputPairSpec, Support, UsbIdentity,
};

pub const PROFILE: DeviceProfile = DeviceProfile {
    id: "ucx",
    card_substring: "Fireface UCX",
    model_name: "Fireface UCX",
    inputs: &[
        InputSpec::new(
            "AN1",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 65)),
                phantom: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN2",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 65)),
                phantom: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN3",
            ChannelType::Instrument,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                hi_z: true,
                autoset: true,
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN4",
            ChannelType::Instrument,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                hi_z: true,
                autoset: true,
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN5",
            ChannelType::Line,
            InputCaps {
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN6",
            ChannelType::Line,
            InputCaps {
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN7",
            ChannelType::Line,
            InputCaps {
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN8",
            ChannelType::Line,
            InputCaps {
                ref_levels: &["-10dBV", "+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "SPDIF L",
            ChannelType::SPDIF,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "SPDIF R",
            ChannelType::SPDIF,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AS1",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AS2",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT3",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT4",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT5",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT6",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT7",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT8",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
    ],
    outputs: &[
        OutputPairSpec::with_ref_levels("AN1", "AN2", "AN1/2", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("AN3", "AN4", "AN3/4", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("AN5", "AN6", "AN5/6", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("PH7", "PH8", "PH7/8", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::new("SPDIF L", "SPDIF R", "SPDIF"),
        OutputPairSpec::new("AS1", "AS2", "AS1/2"),
        OutputPairSpec::new("ADAT3", "ADAT4", "ADAT3/4"),
        OutputPairSpec::new("ADAT5", "ADAT6", "ADAT5/6"),
        OutputPairSpec::new("ADAT7", "ADAT8", "ADAT7/8"),
    ],
    // TotalMix's Main is not documented in the source table; the first
    // pair stands in for it.
    main_pair: Some(0),
    usb: &[
        UsbIdentity::vendor_mode(0x2a39, 0x3fc9),
        UsbIdentity::vendor_mode(0x0424, 0x3fc9),
        UsbIdentity::class_compliant(0x2a39, 0x3fb9),
    ],
    protocol: ControlProtocol::Latter { refresh: 0x0812 },
    features: DeviceFeatures {
        durec: false,
        room_eq: false,
        mix_volume_only: false,
    },
    support: Support::SimulationOnly,
    source: "oscmix device_ffucx.c (https://github.com/charlysound/oscmix-ufxplus-linux)",
};
