// SPDX-License-Identifier: MIT AND ISC
// Copyright (c) 2026 Ismaïl Bahloul
// Channel table derived from oscmix `device_ffufxii.c`:
// Copyright (c) 2023-2024 Michael Forney, ISC license (see
// `LICENSES/oscmix-ISC.txt`).
// Taken from the charlysound/oscmix-ufxplus-linux fork of oscmix.

//! Fireface UFX II channel topology, for the simulation only.
//!
//! No backend drives a Fireface UFX II yet: this profile lets `--mock` show the
//! unit's layout. Generated from oscmix's table for the unit (the charlysound fork; upstream oscmix has no table for this unit),
//! by `tools/profiles/gen_oscmix_profiles.py`; edit the generator, not
//! this file. Not checked on a unit.
//!
//! The refresh value is not settled in the source: `.refresh = 0x234A, //0x67CD`.

use crate::channel::ChannelType;
use crate::profile::{
    ControlProtocol, DeviceFeatures, DeviceProfile, GainRange, InputCaps, InputSpec,
    OutputPairSpec, Support,
};

pub const PROFILE: DeviceProfile = DeviceProfile {
    id: "ufx2",
    card_substring: "Fireface UFX II",
    model_name: "Fireface UFX II",
    inputs: &[
        InputSpec::new(
            "AN1",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN2",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN3",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN4",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN5",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN6",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN7",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN8",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+4dBu", "Lo Gain"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN9",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 75)),
                phantom: true,
                hi_z: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN10",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 75)),
                phantom: true,
                hi_z: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN11",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 75)),
                phantom: true,
                hi_z: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN12",
            ChannelType::Mic,
            InputCaps {
                gain: Some(GainRange::db(0, 75)),
                phantom: true,
                hi_z: true,
                autoset: true,
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AES L",
            ChannelType::AES,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AES R",
            ChannelType::AES,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT1",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT2",
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
        InputSpec::new(
            "ADAT9",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT10",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT11",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT12",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT13",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT14",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT15",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "ADAT16",
            ChannelType::ADAT,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
    ],
    outputs: &[
        OutputPairSpec::with_ref_levels(
            "AN1",
            "AN2",
            "AN1/2",
            &["-10dBV", "+4dBu", "Hi Gain", "+24dBu"],
        ),
        OutputPairSpec::with_ref_levels("AN3", "AN4", "AN3/4", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("AN5", "AN6", "AN5/6", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("AN7", "AN8", "AN7/8", &["-10dBV", "+4dBu", "Hi Gain"]),
        OutputPairSpec::with_ref_levels("PH9", "PH10", "PH9/10", &["Low", "High"]),
        OutputPairSpec::with_ref_levels("PH11", "PH12", "PH11/12", &["Low", "High"]),
        OutputPairSpec::new("AES L", "AES R", "AES"),
        OutputPairSpec::new("ADAT1", "ADAT2", "ADAT1/2"),
        OutputPairSpec::new("ADAT3", "ADAT4", "ADAT3/4"),
        OutputPairSpec::new("ADAT5", "ADAT6", "ADAT5/6"),
        OutputPairSpec::new("ADAT7", "ADAT8", "ADAT7/8"),
        OutputPairSpec::new("ADAT9", "ADAT10", "ADAT9/10"),
        OutputPairSpec::new("ADAT11", "ADAT12", "ADAT11/12"),
        OutputPairSpec::new("ADAT13", "ADAT14", "ADAT13/14"),
        OutputPairSpec::new("ADAT15", "ADAT16", "ADAT15/16"),
    ],
    // TotalMix's Main is not documented in the source table; the first
    // pair stands in for it.
    main_pair: Some(0),
    // USB identities not collected yet.
    usb: &[],
    protocol: ControlProtocol::Latter { refresh: 0x234A },
    features: DeviceFeatures {
        durec: true,
        room_eq: true,
        mix_volume_only: false,
    },
    support: Support::SimulationOnly,
    source: "oscmix device_ffufxii.c (https://github.com/charlysound/oscmix-ufxplus-linux)",
};
