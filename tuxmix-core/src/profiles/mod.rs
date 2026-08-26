//! Per-model device profiles — see [`crate::profile::DeviceProfile`].
//!
//! To add a new ALSA-selem RME model, see the recipe in `CONTRIBUTING.md`
//! at the repo root: copy `babyface_pro.rs` as a template for the profile
//! data, then re-verify `selem_name`/`ch_type_str` in `babyface.rs`
//! against real `amixer scontents` output for your device — that naming
//! grammar is only confirmed for the Babyface Pro so far.

pub mod babyface_pro;
