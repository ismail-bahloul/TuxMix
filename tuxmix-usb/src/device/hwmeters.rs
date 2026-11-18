// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 Ismaïl Bahloul

//! A reader for the unit's hardware meters (feature `driver`).
//!
//! The unit streams a 480-byte meter packet on bulk IN `0x85` of interface
//! 1 about every millisecond, unasked. Nothing the audio driver uses lives
//! on that interface, so this opens the device, claims it, and keeps the
//! loudest raw peak seen per slot since the last [`HwMeters::drain`], while
//! `snd-usb-babyface-pro` keeps running on the other interfaces.
//!
//! The reader only transports raw words; `tuxmix_core::babyface_meters`
//! knows what the slots mean and how to scale them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Interface and endpoint of the meter stream.
pub const INTERFACE: u8 = 1;
pub const ENDPOINT: u8 = 0x85;
/// Bytes in a packet, and slots in it.
pub const PACKET_LEN: usize = 480;
pub const SLOTS: usize = 40;
const PEAKS_AT: usize = SLOTS * 8;

/// A packet silent this long counts as a dead stream.
const STALE_AFTER: Duration = Duration::from_millis(500);

struct Shared {
    peaks: Mutex<[u32; SLOTS]>,
    last_packet: Mutex<Option<Instant>>,
}

/// The raw peak words of a packet (the `u32` half), or `None` unless it is
/// exactly [`PACKET_LEN`] bytes.
pub fn raw_peaks(buf: &[u8]) -> Option<[u32; SLOTS]> {
    if buf.len() != PACKET_LEN {
        return None;
    }
    let mut out = [0u32; SLOTS];
    for (slot, v) in out.iter_mut().enumerate() {
        let at = PEAKS_AT + slot * 4;
        *v = u32::from_le_bytes(buf[at..at + 4].try_into().ok()?);
    }
    Some(out)
}

/// Background reader of the hardware meters.
pub struct HwMeters {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl HwMeters {
    /// Open the unit and start reading. `None` when it cannot be opened or
    /// the interface cannot be claimed (another process holds it).
    pub fn start() -> Option<Self> {
        let dev = rusb::open_device_with_vid_pid(super::VID, super::PID)?;
        dev.claim_interface(INTERFACE).ok()?;
        let shared = Arc::new(Shared {
            peaks: Mutex::new([0; SLOTS]),
            last_packet: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name("tuxmix-hwmeters".into())
            .spawn(move || read_loop(dev, s, st))
            .ok()?;
        Some(Self {
            shared,
            stop,
            handle: Some(handle),
        })
    }

    /// Whether packets are arriving.
    pub fn is_live(&self) -> bool {
        self.shared
            .last_packet
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_some_and(|t| t.elapsed() < STALE_AFTER)
    }

    /// The loudest raw peak per slot since the last call.
    pub fn drain(&self) -> [u32; SLOTS] {
        match self.shared.peaks.lock() {
            Ok(mut p) => std::mem::replace(&mut *p, [0; SLOTS]),
            Err(_) => [0; SLOTS],
        }
    }
}

fn read_loop(
    dev: rusb::DeviceHandle<rusb::GlobalContext>,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
) {
    let mut buf = [0u8; PACKET_LEN];
    while !stop.load(Ordering::Relaxed) {
        match dev.read_bulk(ENDPOINT, &mut buf, Duration::from_millis(100)) {
            Ok(n) => {
                if let Some(raw) = raw_peaks(&buf[..n]) {
                    if let Ok(mut p) = shared.peaks.lock() {
                        for (acc, v) in p.iter_mut().zip(raw) {
                            // The low four bits are flags, not level.
                            *acc = (*acc).max(v & !0xf);
                        }
                    }
                    if let Ok(mut t) = shared.last_packet.lock() {
                        *t = Some(Instant::now());
                    }
                }
            }
            Err(rusb::Error::Timeout) => {}
            Err(rusb::Error::NoDevice) | Err(rusb::Error::Io) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = dev.release_interface(INTERFACE);
}

impl Drop for HwMeters {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_whole_packet_decodes() {
        assert!(raw_peaks(&[0; PACKET_LEN - 1]).is_none());
        assert!(raw_peaks(&[0; PACKET_LEN + 1]).is_none());
        let mut buf = [0u8; PACKET_LEN];
        buf[PEAKS_AT + 4 * 3..PEAKS_AT + 4 * 3 + 4].copy_from_slice(&0x0012_3450u32.to_le_bytes());
        let p = raw_peaks(&buf).unwrap();
        assert_eq!(p[3], 0x0012_3450);
        assert_eq!(p[0], 0);
    }
}
