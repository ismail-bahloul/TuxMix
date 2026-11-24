// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Per-model device profiles — see [`crate::profile::DeviceProfile`].
//!
//! `babyface_pro` is the one model a backend drives. The `fireface_*`
//! profiles are generated from oscmix's channel tables by
//! `tools/profiles/gen_oscmix_profiles.py` and serve the simulation only
//! (`TUXMIX_MOCK_MODEL=<id> tuxmix --mock`), until a backend for RME's
//! "latter" protocol exists (see `crate::latter`).
//!
//! To add an ALSA-selem RME model, see the recipe in `CONTRIBUTING.md`
//! at the repo root: copy `babyface_pro.rs` as a template for the profile
//! data, then re-verify `selem_name`/`ch_type_str` in `babyface.rs`
//! against real `amixer scontents` output for your device — that naming
//! grammar is only confirmed for the Babyface Pro so far.

use crate::profile::DeviceProfile;

pub mod babyface_pro;
pub mod fireface_ff802;
pub mod fireface_ucx;
pub mod fireface_ucx2;
pub mod fireface_ufx;
pub mod fireface_ufx2;
pub mod fireface_ufx3;
pub mod fireface_ufxplus;

/// Every profile, the driven one first.
pub const ALL: &[&DeviceProfile] = &[
    &babyface_pro::PROFILE,
    &fireface_ucx::PROFILE,
    &fireface_ucx2::PROFILE,
    &fireface_ff802::PROFILE,
    &fireface_ufx::PROFILE,
    &fireface_ufx2::PROFILE,
    &fireface_ufx3::PROFILE,
    &fireface_ufxplus::PROFILE,
];

/// The profile with this [`DeviceProfile::id`], ignoring case.
pub fn by_id(id: &str) -> Option<&'static DeviceProfile> {
    ALL.iter().copied().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// The profile of the unit that enumerates as `vendor:product`, in any
/// mode.
pub fn by_usb(vendor: u16, product: u16) -> Option<&'static DeviceProfile> {
    ALL.iter()
        .copied()
        .find(|p| p.usb_mode(vendor, product).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelType;
    use crate::profile::{ControlProtocol, Support};
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_found() {
        let mut seen = HashSet::new();
        for p in ALL {
            assert!(seen.insert(p.id), "duplicate id {}", p.id);
            assert_eq!(by_id(&p.id.to_uppercase()).unwrap().id, p.id);
        }
        assert!(by_id("nope").is_none());
    }

    #[test]
    fn only_the_babyface_pro_is_driven() {
        for p in ALL {
            let driven = p.support == Support::Driven;
            assert_eq!(driven, p.id == "babyface_pro", "{}", p.id);
        }
    }

    /// The channel counts oscmix asserts with `_Static_assert` in each
    /// `device_ff*.c`, and the refresh values of the review's table.
    #[test]
    fn family_b_tables_have_oscmix_channel_counts() {
        let expect = [
            ("ucx", 18, 0x0812),
            ("ucx2", 20, 0x67cd),
            ("ff802", 30, 0x0812),
            ("ufx", 30, 0x0812),
            ("ufx2", 30, 0x234a),
            ("ufx3", 94, 0x67cd),
            ("ufxplus", 94, 0x67cd),
        ];
        for (id, n, refresh) in expect {
            let p = by_id(id).unwrap();
            assert_eq!(p.input_count(), n, "{id} inputs");
            assert_eq!(p.output_pair_count() * 2, n, "{id} outputs");
            assert_eq!(p.protocol, ControlProtocol::Latter { refresh }, "{id}");
        }
    }

    #[test]
    fn every_profile_is_consistent() {
        for p in ALL {
            assert!(
                p.main_pair.is_none_or(|m| m < p.output_pair_count()),
                "{}",
                p.id
            );
            for s in p.inputs {
                if let Some(g) = s.caps.gain {
                    assert!(g.min_db < g.max_db && g.max_db <= 75, "{} {}", p.id, s.name);
                }
                if s.caps.phantom {
                    assert_eq!(s.channel_type, ChannelType::Mic, "{} {}", p.id, s.name);
                }
            }
            // Pair labels are what the matrix and submix pickers show:
            // two pairs with one name would be ambiguous.
            let labels: HashSet<_> = p.pair_labels().into_iter().collect();
            assert_eq!(labels.len(), p.output_pair_count(), "{}", p.id);
        }
    }

    #[test]
    fn usb_identities_lead_to_their_profile() {
        assert_eq!(by_usb(0x2a39, 0x3fc0).unwrap().id, "babyface_pro");
        assert_eq!(by_usb(0x2a39, 0x3fb0).unwrap().id, "babyface_pro");
        assert_eq!(by_usb(0x0424, 0x3fc9).unwrap().id, "ucx");
        assert_eq!(by_usb(0x2a39, 0x3f82).unwrap().id, "ucx2");
        assert!(by_usb(0x046d, 0xc52b).is_none());
    }

    #[test]
    fn simulated_inputs_carry_the_declared_controls() {
        let p = by_id("ucx2").unwrap();
        let inputs = p.build_simulated_inputs();
        assert_eq!(inputs[0].gain_max, Some(75));
        assert_eq!(inputs[0].gain, Some(0));
        assert_eq!(inputs[8].gain_max, None, "SPDIF L has no preamp");
        // Only the backend's answers fill these on the real path.
        assert!(p.build_inputs().iter().all(|c| c.gain_max.is_none()));
    }

    #[test]
    fn babyface_pro_capabilities_match_the_driver() {
        let p = &babyface_pro::PROFILE;
        let an1 = p.input_caps(0);
        assert!(an1.phantom && an1.pad && an1.eq);
        assert_eq!(an1.gain.unwrap().max_db, 65);
        let in3 = p.input_caps(2);
        assert!(!in3.phantom && in3.eq && in3.has_sensitivity_switch());
        assert_eq!(in3.gain.unwrap().max_db, 9);
        assert_eq!(p.input_caps(4), crate::profile::InputCaps::NONE);
        assert_eq!(p.input_caps(99), crate::profile::InputCaps::NONE);
    }
}
