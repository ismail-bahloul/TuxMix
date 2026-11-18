// SPDX-License-Identifier: MIT AND ISC
// Copyright (c) 2026 Ismaïl Bahloul
// Channel table derived from oscmix `device_ffufxiii.c`:
// Copyright (c) 2023-2024 Michael Forney, ISC license (see
// `LICENSES/oscmix-ISC.txt`).
// Taken from the charlysound/oscmix-ufxplus-linux fork of oscmix.

//! Fireface UFX III channel topology, for the simulation only.
//!
//! No backend drives a Fireface UFX III yet: this profile lets `--mock` show the
//! unit's layout. Generated from oscmix's table for the unit (the charlysound fork; upstream oscmix has no table for this unit),
//! by `tools/profiles/gen_oscmix_profiles.py`; edit the generator, not
//! this file. Not checked on a unit.

use crate::channel::ChannelType;
use crate::profile::{
    ControlProtocol, DeviceFeatures, DeviceProfile, GainRange, InputCaps, InputSpec,
    OutputPairSpec, Support,
};

pub const PROFILE: DeviceProfile = DeviceProfile {
    id: "ufx3",
    card_substring: "Fireface UFX III",
    model_name: "Fireface UFX III",
    inputs: &[
        InputSpec::new(
            "AN1",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN2",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN3",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN4",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN5",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN6",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN7",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "AN8",
            ChannelType::Line,
            InputCaps {
                gain: Some(GainRange::db(0, 12)),
                ref_levels: &["+13dBu", "+19dBu"],
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
        InputSpec::new(
            "MADI1",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI2",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI3",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI4",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI5",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI6",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI7",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI8",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI9",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI10",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI11",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI12",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI13",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI14",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI15",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI16",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI17",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI18",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI19",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI20",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI21",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI22",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI23",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI24",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI25",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI26",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI27",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI28",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI29",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI30",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI31",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI32",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI33",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI34",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI35",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI36",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI37",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI38",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI39",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI40",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI41",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI42",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI43",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI44",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI45",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI46",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI47",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI48",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI49",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI50",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI51",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI52",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI53",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI54",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI55",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI56",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI57",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI58",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI59",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI60",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI61",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI62",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI63",
            ChannelType::MADI,
            InputCaps {
                eq: true,
                ..InputCaps::NONE
            },
        ),
        InputSpec::new(
            "MADI64",
            ChannelType::MADI,
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
            &["+4dBu", "+13dBu", "+19dBu", "+24dBu"],
        ),
        OutputPairSpec::with_ref_levels("AN3", "AN4", "AN3/4", &["+4dBu", "+13dBu", "+19dBu"]),
        OutputPairSpec::with_ref_levels("AN5", "AN6", "AN5/6", &["+4dBu", "+13dBu", "+19dBu"]),
        OutputPairSpec::with_ref_levels("AN7", "AN8", "AN7/8", &["+4dBu", "+13dBu", "+19dBu"]),
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
        OutputPairSpec::new("MADI1", "MADI2", "MADI1/2"),
        OutputPairSpec::new("MADI3", "MADI4", "MADI3/4"),
        OutputPairSpec::new("MADI5", "MADI6", "MADI5/6"),
        OutputPairSpec::new("MADI7", "MADI8", "MADI7/8"),
        OutputPairSpec::new("MADI9", "MADI10", "MADI9/10"),
        OutputPairSpec::new("MADI11", "MADI12", "MADI11/12"),
        OutputPairSpec::new("MADI13", "MADI14", "MADI13/14"),
        OutputPairSpec::new("MADI15", "MADI16", "MADI15/16"),
        OutputPairSpec::new("MADI17", "MADI18", "MADI17/18"),
        OutputPairSpec::new("MADI19", "MADI20", "MADI19/20"),
        OutputPairSpec::new("MADI21", "MADI22", "MADI21/22"),
        OutputPairSpec::new("MADI23", "MADI24", "MADI23/24"),
        OutputPairSpec::new("MADI25", "MADI26", "MADI25/26"),
        OutputPairSpec::new("MADI27", "MADI28", "MADI27/28"),
        OutputPairSpec::new("MADI29", "MADI30", "MADI29/30"),
        OutputPairSpec::new("MADI31", "MADI32", "MADI31/32"),
        OutputPairSpec::new("MADI33", "MADI34", "MADI33/34"),
        OutputPairSpec::new("MADI35", "MADI36", "MADI35/36"),
        OutputPairSpec::new("MADI37", "MADI38", "MADI37/38"),
        OutputPairSpec::new("MADI39", "MADI40", "MADI39/40"),
        OutputPairSpec::new("MADI41", "MADI42", "MADI41/42"),
        OutputPairSpec::new("MADI43", "MADI44", "MADI43/44"),
        OutputPairSpec::new("MADI45", "MADI46", "MADI45/46"),
        OutputPairSpec::new("MADI47", "MADI48", "MADI47/48"),
        OutputPairSpec::new("MADI49", "MADI50", "MADI49/50"),
        OutputPairSpec::new("MADI51", "MADI52", "MADI51/52"),
        OutputPairSpec::new("MADI53", "MADI54", "MADI53/54"),
        OutputPairSpec::new("MADI55", "MADI56", "MADI55/56"),
        OutputPairSpec::new("MADI57", "MADI58", "MADI57/58"),
        OutputPairSpec::new("MADI59", "MADI60", "MADI59/60"),
        OutputPairSpec::new("MADI61", "MADI62", "MADI61/62"),
        OutputPairSpec::new("MADI63", "MADI64", "MADI63/64"),
    ],
    // TotalMix's Main is not documented in the source table; the first
    // pair stands in for it.
    main_pair: Some(0),
    // USB identities not collected yet.
    usb: &[],
    protocol: ControlProtocol::Latter { refresh: 0x67CD },
    features: DeviceFeatures {
        durec: true,
        room_eq: true,
        mix_volume_only: false,
    },
    support: Support::SimulationOnly,
    source: "oscmix device_ffufxiii.c (https://github.com/charlysound/oscmix-ufxplus-linux)",
};
