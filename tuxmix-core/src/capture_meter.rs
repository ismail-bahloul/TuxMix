//! Real-time input level metering for the ALSA/kernel-driver backend,
//! by opening the card's own capture PCM directly and computing peaks
//! host-side from the actual audio — the driver exposes no meter
//! register at all (see `RmeDevice::meters`'s own doc comment /
//! `PROTOCOL.md`'s "VU meters: conclusion"), so this is the only way
//! to get a real reading, same idea as the USB backend's
//! `input_peaks()` (`tuxmix-usb`), just a different audio source (ALSA
//! capture instead of the raw isochronous stream it already owns).
//!
//! **Scope: AN1-AN4 (capture channels 0-3).** The kernel driver maps
//! app capture channel `i` to device word `i` for `i < 4`
//! (`babyface_capture_copy`'s own map), and words 0-3 are the analog
//! inputs' record buses. IN3/IN4 (channels 2/3) are the PH3/4 record
//! bus: it carries the instrument inputs at idle, but the PH3/4 output
//! bus once that output's loopback is on, since the loopback taps the
//! output bus into the same record pair. So these are record-bus
//! levels, not strictly raw input levels, when loopback is in use.
//!
//! Everything past channel 3 is left out for now. Channels 4-9 are the
//! digital inputs (device words 6-11), but how those words line up
//! with the AS1/2 and ADAT3-8 strips is not confirmed (and words 4/5,
//! where a separate SPDIF pair could sit, read as a fixed marker in the
//! record stream); channels 10/11 are the FX send (the driver's comment
//! also calls them a fixed-gain playback tap). Showing a reading there
//! would risk attributing a level to the wrong physical input, worse
//! than the honest "N/A" this falls back to (see
//! `DeviceHandle::has_input_meter` in `tuxmix-gui`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use alsa::pcm::{Access, Format, HwParams, State, PCM};
use alsa::{Direction, ValueOr};

/// AN1-AN4 — see the module doc comment for why nothing past these
/// four is metered.
const METERED_CHANNELS: usize = 4;
/// Small period so a blocked `readi()` call returns quickly enough for
/// `Drop` to stop the thread without a noticeable shutdown delay.
const PERIOD_FRAMES: alsa::pcm::Frames = 256;

/// Owns the capture stream and its dedicated reader thread (`readi`
/// blocks, so this can't run on the UI thread) — `drain()` is the only
/// thing the rest of `tuxmix-core` touches.
pub struct CaptureMeter {
    peaks: Arc<Mutex<[f32; METERED_CHANNELS]>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CaptureMeter {
    /// Opens `card_name`'s own capture PCM (the same ALSA card the
    /// mixer already opened, e.g. `"hw:0"`) and starts the metering
    /// thread. `None` if the capture device can't be opened or
    /// configured — metering is a nice-to-have, not something that
    /// should keep the rest of the device from working.
    pub fn start(card_name: &str, channels: u32, rate: u32) -> Option<Self> {
        let pcm = PCM::new(card_name, Direction::Capture, false).ok()?;
        {
            let hwp = HwParams::any(&pcm).ok()?;
            hwp.set_access(Access::RWInterleaved).ok()?;
            hwp.set_format(Format::S32LE).ok()?;
            hwp.set_channels(channels).ok()?;
            hwp.set_rate_near(rate, ValueOr::Nearest).ok()?;
            hwp.set_period_size_near(PERIOD_FRAMES, ValueOr::Nearest)
                .ok()?;
            pcm.hw_params(&hwp).ok()?;
        }
        pcm.prepare().ok()?;
        // Capture doesn't auto-start the way a playback stream does on
        // its first write — without this, `readi` below never blocks
        // waiting for real audio; it immediately errors (empty ring
        // buffer never actually collecting samples), `recover()`
        // "succeeds" (it's a legitimate no-op from ALSA's point of
        // view), and the read loop spins as fast as the CPU allows
        // instead of sleeping between periods. Caught by the app
        // pegging a full core within seconds of launch.
        pcm.start().ok()?;

        let peaks = Arc::new(Mutex::new([0.0f32; METERED_CHANNELS]));
        let stop = Arc::new(AtomicBool::new(false));
        let peaks2 = Arc::clone(&peaks);
        let stop2 = Arc::clone(&stop);
        let channels = channels as usize;

        let handle = std::thread::spawn(move || {
            let Ok(io) = pcm.io_i32() else { return };
            let mut buf = vec![0i32; PERIOD_FRAMES as usize * channels];
            loop {
                if stop2.load(Ordering::Relaxed) {
                    return;
                }
                match io.readi(&mut buf) {
                    Ok(n) if n > 0 => {
                        // `i32::abs()` panics (in debug builds) on
                        // exactly `i32::MIN` — a real, hit-in-practice
                        // bit pattern in raw audio samples, not a
                        // theoretical edge case. `unsigned_abs()` has
                        // no such hole: `i32::MIN`'s magnitude fits a
                        // `u32` fine, unlike its `i32` negation.
                        let mut local = [0u32; METERED_CHANNELS];
                        for frame in 0..n {
                            for (ch, slot) in local.iter_mut().enumerate() {
                                let s = buf[frame * channels + ch].unsigned_abs();
                                if s > *slot {
                                    *slot = s;
                                }
                            }
                        }
                        if let Ok(mut p) = peaks2.lock() {
                            for (ch, slot) in p.iter_mut().enumerate() {
                                let v = local[ch] as f32 / i32::MAX as f32;
                                if v > *slot {
                                    *slot = v;
                                }
                            }
                        }
                    }
                    // `n == 0`: nothing new yet, loop and check `stop` again.
                    Ok(_) => {}
                    Err(e) => {
                        let recoverable =
                            matches!(pcm.state(), State::XRun | State::Suspended);
                        if !recoverable || pcm.recover(e.errno(), true).is_err() {
                            // Not a plain xrun, or recovery itself
                            // failed (e.g. the device was unplugged) —
                            // back off instead of spinning.
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
            }
        });

        Some(Self {
            peaks,
            stop,
            handle: Some(handle),
        })
    }

    /// Peak level (0.0-1.0 of full scale) accumulated since the last
    /// call, per metered channel: resets the accumulator, matching
    /// `tuxmix-usb`'s own `input_peaks()` draining convention (poll
    /// once per UI tick, not once per channel).
    pub fn drain(&self) -> [f32; METERED_CHANNELS] {
        let Ok(mut p) = self.peaks.lock() else {
            return [0.0; METERED_CHANNELS];
        };
        let out = *p;
        *p = [0.0; METERED_CHANNELS];
        out
    }
}

impl Drop for CaptureMeter {
    fn drop(&mut self) {
        // `readi` on the reader thread blocks for at most `PERIOD_FRAMES`
        // worth of audio (a few ms) before it next checks `stop`, so
        // this join doesn't stall the caller noticeably.
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
