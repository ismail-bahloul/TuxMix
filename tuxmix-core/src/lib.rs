//! `tuxmix-core` — Abstraction layer for RME audio interfaces on Linux.
//!
//! This crate provides a hardware-agnostic interface for controlling
//! RME audio interfaces through ALSA mixer controls.
//!
//! # Design
//!
//! The main entrypoint is the [`RmeDevice`] trait, implemented by each
//! supported RME interface (e.g. [`BabyfacePro`]).
//!
//! A device exposes:
//! - [`InputChannel`]s (physical hardware inputs)
//! - [`PlaybackChannel`]s (software playback streams)
//! - Global [`DeviceSettings`] (clock, SPDIF, etc.)

#[cfg(feature = "alsa")]
pub mod babyface;
pub mod channel;
pub(crate) mod curves;
pub mod device;
pub mod error;
#[cfg(feature = "alsa")]
pub mod mixer;
pub mod mock;
pub mod panel;
pub mod profile;
pub mod profiles;
pub mod scene;
#[cfg(feature = "usb")]
pub mod usb;

#[cfg(feature = "alsa")]
pub use babyface::BabyfacePro;
pub use channel::{
    ChannelId, ChannelType, InputChannel, OutputChannel, PlaybackChannel, Sensitivity,
};
pub use device::{DeviceSettings, RmeDevice};
pub use error::Error;
#[cfg(feature = "alsa")]
pub use mixer::AlsaMixer;
pub use mock::MockBabyfacePro;
pub use scene::Scene;
#[cfg(feature = "usb")]
pub use usb::BabyfaceProUsb;
