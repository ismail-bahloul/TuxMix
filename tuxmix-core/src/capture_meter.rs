//! Real-time input level metering for the ALSA/kernel-driver backend,
//! by opening the card's own capture PCM directly and computing peaks
//! host-side from the actual audio — the driver exposes no meter
//! register at all (see `RmeDevice::meters`'s own doc comment /
//! `PROTOCOL.md`'s "VU meters: conclusion"), so this is the only way
//! to get a real reading, same idea as the USB backend's
//! `input_peaks()` (`tuxmix-usb`), just a different audio source (ALSA
//! capture instead of the raw isochronous stream it already owns).
//!
//! **Mapping, confirmed live on 2026-10-17** by playing a 1 kHz tone on
//! PB1 and enabling the device's own loopback on one output pair at a
//! time (no external source needed). The capture channels are the
//! output pairs' record buses:
//!
//! | capture | input strip |
//! |---------|-------------|
//! | 0/1     | AN1/2       |
//! | 2/3     | IN3/4 (PH3/4 record bus) |
//! | 4/5     | ADAT3/4     |
//! | 6/7     | ADAT5/6     |
//! | 8/9     | ADAT7/8     |
//! | 10/11   | AS1/2       |
//!
//! AS1/2 is capture 10/11 in the driver's 2026-10-17 map (device words
//! 4/5, confirmed with the same loopback test). That map keeps channels
//! 0-9 untouched and replaces the old playback echo tap (words 12/13)
//! with AS1/2, so existing 12-channel recordings keep their order. A
//! driver built before that date still exposes the playback tap there;
//! this table assumes the updated driver.
//!
//! Each pair is a record bus, and the device's loopback taps an output
//! bus into the same words, so enabling loopback on an output makes the
//! matching input pair read that output bus too. That is a property of
//! the record path, not a mapping error. `INPUT_FOR_CAPTURE` below is
//! the table, shared with `BabyfacePro::meters`.
//!
//! `TUXMIX_METER_DEBUG=1` prints every channel once a second for future
//! hardware checks; the normal path only uses the table.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use alsa::pcm::{Access, Format, HwParams, State, PCM};
use alsa::{Direction, ValueOr};

/// Every channel the kernel driver exposes: 0-3 = AN1-AN4, 4-9 = the
/// digital inputs, 10/11 = AS1/2. All of them are drained;
/// `INPUT_FOR_CAPTURE` maps them to input strips.
pub(crate) const CAPTURE_CHANNELS: usize = 12;

/// Capture channel to input-strip index. Confirmed live on 2026-10-17
/// (see the module doc comment): AN1/2, IN3/4, ADAT3/4, ADAT5/6,
/// ADAT7/8, then AS1/2 at 10/11.
pub(crate) const INPUT_FOR_CAPTURE: [Option<usize>; CAPTURE_CHANNELS] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    Some(6),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
    Some(4),
    Some(5),
];
/// Small period so a blocked `readi()` call returns quickly enough for
/// `Drop` to stop the thread without a noticeable shutdown delay.
const PERIOD_FRAMES: alsa::pcm::Frames = 256;

/// Owns the capture stream and its dedicated reader thread (`readi`
/// blocks, so this can't run on the UI thread) — `drain()` is the only
/// thing the rest of `tuxmix-core` touches.
pub struct CaptureMeter {
    peaks: Arc<Mutex<[f32; CAPTURE_CHANNELS]>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl CaptureMeter {
    /// Opens `card_name`'s own capture PCM (the same ALSA card the
    /// mixer already opened, e.g. `"hw:0"`) and starts the metering
    /// thread. `None` if the capture device can't be opened or
    /// configured — metering is a nice-to-have, not something that
    /// should keep the rest of the device from working.
    pub fn start(card_name: &str, rate: u32) -> Option<Self> {
        let pcm = PCM::new(card_name, Direction::Capture, false).ok()?;
        {
            let hwp = HwParams::any(&pcm).ok()?;
            hwp.set_access(Access::RWInterleaved).ok()?;
            hwp.set_format(Format::S32LE).ok()?;
            hwp.set_channels(CAPTURE_CHANNELS as u32).ok()?;
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

        let peaks = Arc::new(Mutex::new([0.0f32; CAPTURE_CHANNELS]));
        let stop = Arc::new(AtomicBool::new(false));
        let peaks2 = Arc::clone(&peaks);
        let stop2 = Arc::clone(&stop);
        let channels = CAPTURE_CHANNELS;

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
                        let mut local = [0u32; CAPTURE_CHANNELS];
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
    /// call, per capture channel: resets the accumulator, matching
    /// `tuxmix-usb`'s own `input_peaks()` draining convention (poll
    /// once per UI tick, not once per channel).
    pub fn drain(&self) -> [f32; CAPTURE_CHANNELS] {
        let Ok(mut p) = self.peaks.lock() else {
            return [0.0; CAPTURE_CHANNELS];
        };
        let out = *p;
        *p = [0.0; CAPTURE_CHANNELS];
        out
    }

    /// Print every capture channel once a second when
    /// `TUXMIX_METER_DEBUG` is set, in dBFS (`-120.0` for silence).
    /// Exists for hardware checks like the 2026-10-17 loopback mapping:
    /// play a known signal into one bus at a time and watch which index
    /// moves. Index map: 0/1 = AN1/2, 2/3 = IN3/4, 4/5 = ADAT3/4,
    /// 6/7 = ADAT5/6, 8/9 = ADAT7/8, 10/11 = AS1/2.
    pub fn debug_log(peaks: &[f32; CAPTURE_CHANNELS]) {
        use std::sync::OnceLock;
        use std::time::Instant;

        static ENABLED: OnceLock<bool> = OnceLock::new();
        static LAST: Mutex<Option<Instant>> = Mutex::new(None);

        if !*ENABLED
            .get_or_init(|| std::env::var_os("TUXMIX_METER_DEBUG").is_some())
        {
            return;
        }
        let now = Instant::now();
        let Ok(mut last) = LAST.lock() else {
            return;
        };
        if last.is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
            return;
        }
        *last = Some(now);

        let db = |v: f32| {
            if v > 0.0 {
                20.0 * v.log10()
            } else {
                -120.0
            }
        };
        let mut line = String::from("meters dBFS:");
        for (i, v) in peaks.iter().enumerate() {
            line.push_str(&format!(" [{i}]={:.1}", db(*v)));
        }
        eprintln!("{line}");
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

#[cfg(test)]
mod tests {
    use super::INPUT_FOR_CAPTURE;

    #[test]
    fn capture_table_covers_every_input_once() {
        let mut seen = [false; 12];
        for input in INPUT_FOR_CAPTURE.iter().flatten() {
            assert!(!seen[*input], "input {input} appears twice");
            seen[*input] = true;
        }
        assert!(seen.iter().all(|seen| *seen), "not every input is covered");
    }
}
