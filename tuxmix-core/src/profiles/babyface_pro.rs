// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Babyface Pro FS device profile — channel topology, capabilities and
//! card detection, consumed by both [`crate::BabyfacePro`] (real hardware)
//! and [`crate::MockBabyfacePro`] (simulation), so they can't drift
//! apart the way the old hand-duplicated const tables did.

use crate::channel::ChannelType;
use crate::profile::{
    ControlProtocol, DeviceFeatures, DeviceProfile, GainRange, InputCaps, InputSpec,
    OutputPairSpec, Support, UsbIdentity,
};

/// AN1/2: the XLR mic preamps, 0-65 dB with 48V and a pad (the kernel
/// driver's controls, 2026-08-26), and a hardware EQ strip.
const MIC: InputCaps = InputCaps {
    gain: Some(GainRange::db(0, 65)),
    phantom: true,
    pad: true,
    eq: true,
    ..InputCaps::NONE
};

/// IN3/4: the instrument/line jacks, 0-9 dB (raw dB x 2 = 0-18,
/// cap_gain34.pcap), with the three reference levels in `usb::REF_*`
/// order and a hardware EQ strip.
const INSTRUMENT: InputCaps = InputCaps {
    gain: Some(GainRange::db(0, 9)),
    ref_levels: &["+4dBu", "-10dBV", "Boost"],
    eq: true,
    ..InputCaps::NONE
};

pub const PROFILE: DeviceProfile = DeviceProfile {
    id: "babyface_pro",
    card_substring: "Babyface Pro",
    model_name: "Babyface Pro FS",
    inputs: &[
        InputSpec::new("AN1", ChannelType::Mic, MIC),
        InputSpec::new("AN2", ChannelType::Mic, MIC),
        InputSpec::new("IN3", ChannelType::Instrument, INSTRUMENT),
        InputSpec::new("IN4", ChannelType::Instrument, INSTRUMENT),
        InputSpec::plain("AS1", ChannelType::Line),
        InputSpec::plain("AS2", ChannelType::Line),
        InputSpec::plain("ADAT3", ChannelType::ADAT),
        InputSpec::plain("ADAT4", ChannelType::ADAT),
        InputSpec::plain("ADAT5", ChannelType::ADAT),
        InputSpec::plain("ADAT6", ChannelType::ADAT),
        InputSpec::plain("ADAT7", ChannelType::ADAT),
        InputSpec::plain("ADAT8", ChannelType::ADAT),
    ],
    outputs: &[
        OutputPairSpec::new("AN1", "AN2", "AN1/2"),
        OutputPairSpec::new("PH3", "PH4", "PH3/4"),
        OutputPairSpec::new("AS1", "AS2", "AS1/2"),
        OutputPairSpec::new("ADAT3", "ADAT4", "ADAT3/4"),
        OutputPairSpec::new("ADAT5", "ADAT6", "ADAT5/6"),
        OutputPairSpec::new("ADAT7", "ADAT8", "ADAT7/8"),
    ],
    // PH3/4: what TotalMix FX shows as Main on this unit, and the pair
    // DIM and the Control Room's Recall act on.
    main_pair: Some(1),
    usb: &[
        UsbIdentity::vendor_mode(0x2a39, 0x3fc0),
        UsbIdentity::class_compliant(0x2a39, 0x3fb0),
    ],
    protocol: ControlProtocol::VendorRequests,
    features: DeviceFeatures {
        durec: false,
        room_eq: false,
        mix_volume_only: false,
    },
    support: Support::Driven,
    source: "USB captures of TotalMix on Windows, checked on a Babyface Pro FS \
             (babyface-pro-linux, snd-usb-babyface-pro)",
};
