//! `tuxmix-usb` — proprietary USB protocol backend for the RME Babyface Pro FS.
//!
//! This crate implements the vendor-control protocol that TotalMix FX
//! uses to drive the Babyface Pro FS in its proprietary mode
//! (VID `2A39`, PID `3FC0`). The protocol was reverse-engineered from
//! USB captures — see `tools/usbdump/PROTOCOL.md` for the full report.
//!
//! # Layout
//!
//! - [`map`] — the register address map (crosspoints, masters, gains,
//!   source/output indices), pure and unit-tested against the captured
//!   addresses.
//! - [`protocol`] — the vendor-request encoding (one request per
//!   control transfer, no data phase).
//! - `device` (feature `driver`) — libusb-backed device driver that
//!   sends the requests and reads the isochronous audio endpoints for
//!   VU metering.
//!
//! The protocol layer is dependency-free and can be tested without
//! hardware; the `driver` feature pulls in `rusb`.

pub mod map;
pub mod protocol;

mod audio;
pub use audio::{BabyfaceAudio, CAPTURE_CHANNELS, PLAYBACK_CHANNELS, S24_LE_BYTES};

pub mod device;

pub use map::{Input, Output, Playback, Source};
pub use protocol::{FlagCounter, VendorRequest};

#[cfg(feature = "driver")]
pub use device::{BabyfaceUsb, Error as DeviceError, PID, VID};
