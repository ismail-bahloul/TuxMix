//! Real-time input level metering for the ALSA/kernel-driver backend,
//! by opening the card's own capture PCM directly and computing peaks
//! host-side from the actual audio — the driver exposes no meter
//! register at all (see `RmeDevice::meters`'s own doc comment /
//! `PROTOCOL.md`'s "VU meters: conclusion"), so this is the only way
//! to get a real reading, same idea as the USB backend's
//! `input_peaks()` (`tuxmix-usb`), just a different audio source (ALSA
//! capture instead of the raw isochronous stream it already owns).
//!
//! **Scope: only AN1/AN2 (capture words 0/1) are metered.** That's the
//! one capture-channel mapping confirmed against real hardware
//! (`PROTOCOL.md`: "w0/1 = the AN1/2 record bus" + a live mic test).
//! Every other physical input's word mapping is either genuinely
//! contextual (IN3/4 share a word pair with the PH3/4 output bus's
//! loopback signal — not a fixed channel at all) or disputed between
//! `PROTOCOL.md` and the more recent `KERNEL-DRIVER.md`. Showing a
//! reading for those would risk attributing a level to the wrong
//! physical input, worse than the honest "N/A" this falls back to —
//! see `DeviceHandle::has_input_meter` in `tuxmix-gui`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use alsa::pcm::{Access, Format, HwParams, State, PCM};
use alsa::{Direction, ValueOr};

/// AN1, AN2 — see the module doc comment for why nothing past these
/// two is metered.
const CONFIRMED_CHANNELS: usize = 2;
/// Small period so a blocked `readi()` call returns quickly enough for
/// `Drop` to stop the thread without a noticeable shutdown delay.
const PERIOD_FRAMES: alsa::pcm::Frames = 256;

/// Owns the capture stream and its dedicated reader thread (`readi`
/// blocks, so this can't run on the UI thread) — `drain()` is the only
/// thing the rest of `tuxmix-core` touches.
pub struct CaptureMeter {
    peaks: Arc<Mutex<[f32; CONFIRMED_CHANNELS]>>,
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

        let peaks = Arc::new(Mutex::new([0.0f32; CONFIRMED_CHANNELS]));
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
                        let mut local = [0u32; CONFIRMED_CHANNELS];
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
    /// call, per confirmed channel — resets the accumulator, matching
    /// `tuxmix-usb`'s own `input_peaks()` draining convention (poll
    /// once per UI tick, not once per channel).
    pub fn drain(&self) -> [f32; CONFIRMED_CHANNELS] {
        let Ok(mut p) = self.peaks.lock() else {
            return [0.0; CONFIRMED_CHANNELS];
        };
        let out = *p;
        *p = [0.0; CONFIRMED_CHANNELS];
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
