//! Real-time input level metering for the ALSA/kernel-driver backend,
//! by opening the card's own capture PCM directly and computing peaks
//! host-side from the actual audio — the driver exposes no meter
//! register at all (see `RmeDevice::meters`'s own doc comment /
//! `PROTOCOL.md`'s "VU meters: conclusion"), so this is the only way
//! to get a real reading, same idea as the USB backend's
//! `input_peaks()` (`tuxmix-usb`), just a different audio source (ALSA
//! capture instead of the raw isochronous stream it already owns).
//!
//! **Mapping**: the driver exposes the device's own order, one capture
//! channel per input word. It was confirmed live on 2026-10-17 by
//! playing a 1 kHz tone on PB1 and enabling the device's own loopback
//! on one output pair at a time (no external source needed), then
//! reordered to that order in the driver's v7 (2026-10-23). The capture
//! channels are the output pairs' record buses:
//!
//! | capture | input strip |
//! |---------|-------------|
//! | 0/1     | AN1/2       |
//! | 2/3     | IN3/4 (PH3/4 record bus) |
//! | 4/5     | AS1/2       |
//! | 6/7     | ADAT3/4     |
//! | 8/9     | ADAT5/6     |
//! | 10/11   | ADAT7/8     |
//!
//! AS1/2 is the first optical pair (ADAT 1/2 when the input receives
//! ADAT, S/PDIF when it receives S/PDIF), hence AS1/2 at 4/5 and the
//! rest named ADAT3-8. The driver is now an identity map: capture
//! channel i is input strip i. A driver built before 2026-10-23 puts
//! AS1/2 at 10/11 and ADAT3-8 at 4-9 instead; TuxMix expects the current
//! order.
//!
//! Each pair is a record bus, and the device's loopback taps an output
//! bus into the same words, so enabling loopback on an output makes the
//! matching input pair read that output bus too. That is a property of
//! the record path, not a mapping error. `INPUT_FOR_CAPTURE` below is
//! the table, shared with `BabyfacePro::meters`.
//!
//! The reader thread reopens the capture PCM when PipeWire reconfigures
//! the card for a profile change (or the unit re-enumerates): `readi`
//! returns an error the xrun recovery cannot fix, so the thread drops
//! the stream and retries with a backoff instead of leaving the meter
//! dead until TuxMix is restarted. `is_connected()` reports the gap so
//! the UI does not keep showing stale peaks.
//!
//! `TUXMIX_METER_DEBUG=1` prints every channel once a second for future
//! hardware checks; the normal path only uses the table.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alsa::pcm::{Access, Format, HwParams, State, PCM};
use alsa::{Direction, ValueOr};

/// Every channel the kernel driver exposes, in the device's own order:
/// AN1-4, AS1/2, ADAT3-8. All of them are drained and mapped to the
/// matching input strip.
pub(crate) const CAPTURE_CHANNELS: usize = 12;

/// Capture channel to input-strip index. The driver's capture map is the
/// identity (AN1/2, IN3/4, AS1/2, ADAT3/4, ADAT5/6, ADAT7/8), so the
/// table is too; it stays explicit so a future driver reorder touches
/// one place.
pub(crate) const INPUT_FOR_CAPTURE: [Option<usize>; CAPTURE_CHANNELS] = [
    Some(0),
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
];
/// The capture period. Not small: this thread wakes once per period, forever,
/// to feed a meter the UI draws at 30 Hz, and at 256 frames (188 wakeups a
/// second at 48 kHz, measured) it was the biggest source of idle wakeups in
/// the whole app. 1024 frames is 47 a second and 21 ms of audio per read,
/// well inside a UI frame. A blocked `readi()` still returns within a period,
/// so `Drop` stops the thread in at most ~21 ms. A larger period is also the
/// safer one against the kernel driver, which refuses an application period
/// below the one another client (PipeWire) already holds the clock with.
const PERIOD_FRAMES: alsa::pcm::Frames = 1024;

/// Opens and starts the capture PCM with the channel count and format
/// the driver exposes. Every step is tolerant: a missing card, or a
/// card currently held by PipeWire, just returns `None` and the reader
/// thread retries.
fn open_pcm(card_name: &str, rate: u32) -> Option<PCM> {
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
    Some(pcm)
}

/// Reads until `stop` is set (returns `true`) or the PCM fails in a way
/// `recover()` cannot fix (returns `false`). The caller owns the `PCM`
/// and drops it on `false` before reopening.
fn read_loop(
    pcm: PCM,
    peaks: &Mutex<[f32; CAPTURE_CHANNELS]>,
    connected: &AtomicBool,
    stop: &AtomicBool,
) -> bool {
    let Ok(io) = pcm.io_i32() else {
        connected.store(false, Ordering::Relaxed);
        return false;
    };
    let mut buf = vec![0i32; PERIOD_FRAMES as usize * CAPTURE_CHANNELS];
    connected.store(true, Ordering::Relaxed);

    loop {
        if stop.load(Ordering::Relaxed) {
            connected.store(false, Ordering::Relaxed);
            return true;
        }
        match io.readi(&mut buf) {
            Ok(n) if n > 0 => {
                // `i32::abs()` panics (in debug builds) on exactly
                // `i32::MIN` — a real, hit-in-practice bit pattern in
                // raw audio samples, not a theoretical edge case.
                // `unsigned_abs()` has no such hole: `i32::MIN`'s
                // magnitude fits a `u32` fine, unlike its `i32`
                // negation.
                let mut local = [0u32; CAPTURE_CHANNELS];
                for frame in 0..n {
                    for (ch, slot) in local.iter_mut().enumerate() {
                        let s = buf[frame * CAPTURE_CHANNELS + ch].unsigned_abs();
                        if s > *slot {
                            *slot = s;
                        }
                    }
                }
                if let Ok(mut p) = peaks.lock() {
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
                let recoverable = matches!(pcm.state(), State::XRun | State::Suspended);
                if recoverable && pcm.recover(e.errno(), true).is_ok() {
                    continue;
                }
                connected.store(false, Ordering::Relaxed);
                return false;
            }
        }
    }
}

/// Sleeps for `duration`, but wakes every 50 ms to notice `stop`.
/// Returns `false` when the thread should exit.
fn sleep_unless_stopped(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
    }
}

/// Owns the capture stream and its dedicated reader thread (`readi`
/// blocks, so this can't run on the UI thread) — `drain()` is the only
/// thing the rest of `tuxmix-core` touches.
pub struct CaptureMeter {
    peaks: Arc<Mutex<[f32; CAPTURE_CHANNELS]>>,
    connected: Arc<AtomicBool>,
    reconnects: Arc<AtomicUsize>,
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
        let pcm = open_pcm(card_name, rate)?;
        let peaks = Arc::new(Mutex::new([0.0f32; CAPTURE_CHANNELS]));
        let connected = Arc::new(AtomicBool::new(true));
        let reconnects = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let peaks2 = Arc::clone(&peaks);
        let connected2 = Arc::clone(&connected);
        let reconnects2 = Arc::clone(&reconnects);
        let stop2 = Arc::clone(&stop);
        let card = card_name.to_string();

        let handle = std::thread::spawn(move || {
            let mut next_pcm = Some(pcm);
            let mut backoff = Duration::from_millis(100);
            loop {
                if stop2.load(Ordering::Relaxed) {
                    connected2.store(false, Ordering::Relaxed);
                    return;
                }
                let current = match next_pcm.take() {
                    Some(pcm) => pcm,
                    None => match open_pcm(&card, rate) {
                        Some(pcm) => {
                            reconnects2.fetch_add(1, Ordering::Relaxed);
                            log::info!("capture meter: reopened the capture PCM");
                            backoff = Duration::from_millis(100);
                            pcm
                        }
                        None => {
                            connected2.store(false, Ordering::Relaxed);
                            if !sleep_unless_stopped(&stop2, backoff) {
                                return;
                            }
                            backoff = (backoff * 2).min(Duration::from_secs(2));
                            continue;
                        }
                    },
                };
                if read_loop(current, &peaks2, &connected2, &stop2) {
                    return;
                }
                // Fatal stream error (a profile change, a re-probe, an
                // unplug): `read_loop` dropped the PCM. Retry with a
                // backoff so a card that stays busy does not spin.
                if !sleep_unless_stopped(&stop2, backoff) {
                    return;
                }
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        });

        Some(Self {
            peaks,
            connected,
            reconnects,
            stop,
            handle: Some(handle),
        })
    }

    /// Whether the reader thread currently has the capture PCM open
    /// and running. False while it is reopening after a profile change
    /// or a re-enumeration.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// How many times the reader thread has reopened the PCM after a
    /// stream error. Diagnostic only; a profile change or a re-probe
    /// should bump this without the app being restarted.
    pub fn reconnects(&self) -> usize {
        self.reconnects.load(Ordering::Relaxed)
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
    /// moves. Index map: 0/1 = AN1/2, 2/3 = IN3/4, 4/5 = AS1/2,
    /// 6/7 = ADAT3/4, 8/9 = ADAT5/6, 10/11 = ADAT7/8.
    pub fn debug_log(peaks: &[f32; CAPTURE_CHANNELS]) {
        use std::sync::OnceLock;

        static ENABLED: OnceLock<bool> = OnceLock::new();
        static LAST: Mutex<Option<Instant>> = Mutex::new(None);

        if !*ENABLED.get_or_init(|| std::env::var_os("TUXMIX_METER_DEBUG").is_some()) {
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
    use super::{sleep_unless_stopped, INPUT_FOR_CAPTURE};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    #[test]
    fn capture_table_covers_every_input_once() {
        let mut seen = [false; 12];
        for input in INPUT_FOR_CAPTURE.iter().flatten() {
            assert!(!seen[*input], "input {input} appears twice");
            seen[*input] = true;
        }
        assert!(seen.iter().all(|seen| *seen), "not every input is covered");
    }

    #[test]
    fn stop_interrupts_the_backoff() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        assert!(!sleep_unless_stopped(&stop, Duration::from_secs(5)));
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
