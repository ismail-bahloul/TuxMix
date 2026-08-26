//! High-level real-audio API on top of the interrupt streams
//! (interface 5, ep 0x01/0x82 — see `device::stream`).
//!
//! `BabyfaceAudio` owns the device, the two audio rings (playback OUT
//! pull / capture IN push, wired through `start_streaming_audio`) and
//! the frame-format conversion between the device's proprietary
//! 14×32-bit frames and plain interleaved S24_LE app audio:
//!
//! - **device frame** (48 kHz class): 14 channels × 4 bytes; byte 0 of
//!   every audio word is 0x00 and the 24-bit sample lives in bytes 1-3
//!   LE (recover with `(word as i32) >> 8`). IN ch4/5 carry a fixed
//!   0x20 marker, ch6-13 = ADAT/SPDIF. OUT ch0-13 = the playback
//!   channels (PB1+PB2 = ch0-3), no marker. See PROTOCOL.md "Frame
//!   format".
//! - **app format**: interleaved S24_LE (3 bytes/sample). The device's
//!   bytes 1-3 ARE the S24_LE sample, so the conversion is a byte
//!   shuffle: `[0x00, s0, s1, s2]` <-> `[s0, s1, s2]`.
//!
//! The libusb event loop is pumped by every IO call (ALSA plugins call
//! readi/writei in a tight loop, so the transfers keep moving; no
//! dedicated thread — the callbacks run synchronously on the calling
//! thread, so the ring mutexes are never contended).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::device::{AudioRing, BabyfaceUsb, Error, EventFd};
use crate::protocol::rate_to_alt;

/// App capture channels exposed: AN1-4 = device ch0-3. (The IN frame
/// markers ch4/5 and ADAT/SPDIF ch6-13 are not exposed yet.)
pub const CAPTURE_CHANNELS: usize = 4;
/// App playback channels exposed: PB1 = ch0/1, PB2 = ch2/3.
pub const PLAYBACK_CHANNELS: usize = 4;
/// Ring depth: 1 s of audio at the active rate (the ALSA app can dump
/// a whole burst in; the transfer returns short counts when full so
/// the app paces itself).
const RING_MILLIS: usize = 1000;

/// How many device-frame bytes `channels` interleaved S24_LE app frames
/// occupy (per frame).
pub const S24_LE_BYTES: usize = 3;

pub struct BabyfaceAudio {
    /// Pump thread — declared FIRST so it's dropped (joined) before
    /// `dev` (Rust drops fields in declaration order).
    pump: Option<Pump>,
    dev: Arc<Mutex<BabyfaceUsb>>,
    playback: Arc<AudioRing>,
    capture: Arc<AudioRing>,
    /// Capture wakeup fd (the ALSA plugin polls it).
    capture_wake: Arc<EventFd>,
    /// Device frame bytes at the active rate (56/40/32).
    frame_bytes: usize,
}

impl Drop for BabyfaceAudio {
    fn drop(&mut self) {
        if let Some(p) = self.pump.take() {
            p.stop.store(true, Ordering::Release);
            let _ = p.thread.join();
        }
    }
}

struct Pump {
    thread: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

impl BabyfaceAudio {
    /// Open the device (no streaming yet).
    pub fn open() -> Result<Self, Error> {
        let dev = Arc::new(Mutex::new(BabyfaceUsb::open()?));
        let frame_bytes = rate_to_alt(48_000).unwrap().frame_bytes;
        let cap = RING_MILLIS * 48_000 / 1000;
        let wake = Arc::new(EventFd::new()?);
        let capture = Arc::new(AudioRing::with_wake(frame_bytes, cap, Some(wake.clone())));
        // The playback ring shares the wake fd: pull_frames signals it
        // when space is freed (the ALSA playback poll path).
        let playback = Arc::new(AudioRing::with_wake(frame_bytes, cap, Some(wake.clone())));
        Ok(Self {
            pump: None,
            dev,
            playback,
            capture,
            capture_wake: wake,
            frame_bytes,
        })
    }

    /// Start the streaming session with the audio rings wired + the
    /// libusb pump thread (2 ms cadence). Idempotent: a second start
    /// (the PipeWire sink + source share one session) is a no-op.
    pub fn start(&mut self) -> Result<(), Error> {
        if self.pump.is_some() {
            return Ok(());
        }
        self.dev
            .lock()
            .unwrap()
            .start_streaming_audio(self.playback.clone(), self.capture.clone())?;
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let dev = self.dev.clone();
        let thread = thread::spawn(move || {
            // SAFETY: BabyfaceUsb is Send (the libusb handle is
            // thread-safe; `IntrStream` carries its own `unsafe impl
            // Send`), and the thread is joined in `drop` before the
            // device is released.
            while !s.load(Ordering::Acquire) {
                if let Ok(d) = dev.lock() {
                    d.pump(Duration::from_millis(2));
                }
            }
        });
        self.pump = Some(Pump { thread, stop });
        Ok(())
    }

    /// Stop the pump thread + streaming + release the interface.
    pub fn stop(&mut self) -> Result<(), Error> {
        if let Some(p) = self.pump.take() {
            p.stop.store(true, Ordering::Release);
            let _ = p.thread.join();
        }
        self.dev.lock().unwrap().stop_streaming()
    }

    /// The active sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.dev.lock().unwrap().sample_rate()
    }

    /// Change the sample rate: recreate the rings (the frame layout
    /// changes with the rate class) and restart the streams.
    pub fn set_rate(&mut self, rate: u32) -> Result<(), Error> {
        let ra = rate_to_alt(rate).ok_or(Error::UnsupportedRate(rate))?;
        if rate == self.sample_rate() {
            return Ok(());
        }
        let cap = RING_MILLIS * rate as usize / 1000;
        let playback = Arc::new(AudioRing::with_wake(
            ra.frame_bytes,
            cap,
            Some(self.capture_wake.clone()),
        ));
        let capture = Arc::new(AudioRing::with_wake(
            ra.frame_bytes,
            cap,
            Some(self.capture_wake.clone()),
        ));
        self.dev
            .lock()
            .unwrap()
            .set_sample_rate_audio(rate, playback.clone(), capture.clone())?;
        self.playback = playback;
        self.capture = capture;
        self.frame_bytes = ra.frame_bytes;
        Ok(())
    }

    /// Push `frames` of interleaved S24_LE app audio (`channels` 2 or
    /// 4) into the playback ring (device format: ch0/1 = PB1, ch2/3 =
    /// PB2; a 2-channel app maps to PB1 and leaves PB2 silent). Returns
    /// the number of app frames ACCEPTED — when the ring is full the
    /// rest are dropped (the caller returns the short count so the app
    /// paces).
    pub fn write_playback(&mut self, app: &[u8], frames: usize, channels: usize) -> usize {
        debug_assert!(channels == 2 || channels == PLAYBACK_CHANNELS);
        let mut dev_frame = vec![0u8; self.frame_bytes];
        let mut accepted = 0;
        for f in 0..frames {
            let base = f * channels * S24_LE_BYTES;
            for c in 0..channels {
                let s = &app[base + c * S24_LE_BYTES..base + (c + 1) * S24_LE_BYTES];
                let w = c * 4;
                dev_frame[w] = 0; // byte 0 of every audio word
                dev_frame[w + 1..w + 4].copy_from_slice(s);
            }
            // PB2 (ch2/3) stays zero for a 2-channel app (already zero).
            if self.playback.push_frame_if_space(&dev_frame) {
                accepted += 1;
            } else {
                break;
            }
        }
        accepted
    }

    /// Read up to `frames` of interleaved S24_LE (`channels` 2 or 4)
    /// into `app` from the capture ring (device ch0/1 = AN1/2, ch2/3 =
    /// AN3/4). Returns the frames actually read (may be < requested
    /// when the ring is drained; the caller zero-pads).
    pub fn read_capture(&mut self, app: &mut [u8], frames: usize, channels: usize) -> usize {
        debug_assert!(channels == 2 || channels == CAPTURE_CHANNELS);
        let mut dev_frame = vec![0u8; self.frame_bytes];
        let mut n = 0;
        while n < frames {
            if self.capture.pull_frames(&mut dev_frame) == 0 {
                break;
            }
            let base = n * channels * S24_LE_BYTES;
            for c in 0..channels {
                let w = c * 4;
                app[base + c * S24_LE_BYTES..base + (c + 1) * S24_LE_BYTES]
                    .copy_from_slice(&dev_frame[w + 1..w + 4]);
            }
            n += 1;
        }
        // Reset the capture wakeup so poll() blocks again until new
        // frames actually arrive.
        self.capture_wake.drain();
        n
    }

    /// The capture wakeup fd (readable when capture frames are queued).
    pub fn capture_fd(&self) -> std::os::raw::c_int {
        self.capture_wake.as_raw_fd()
    }

    /// Playback frames currently queued (for `snd_pcm_delay`).
    pub fn playback_queued(&self) -> usize {
        self.playback.queued_frames()
    }

    /// Playback ring capacity in frames.
    pub fn playback_capacity(&self) -> usize {
        self.playback.capacity_frames()
    }

    /// Playback frames pushed since start (monotonic hw position).
    pub fn playback_pushed(&self) -> u64 {
        self.playback.pushed_frames()
    }

    /// Capture frames currently queued.
    pub fn capture_queued(&self) -> usize {
        self.capture.queued_frames()
    }

    /// Capture frames produced by the device since start (monotonic
    /// hw position).
    pub fn capture_pushed(&self) -> u64 {
        self.capture.pushed_frames()
    }

    /// Diagnostics: playback underruns / capture overruns since start.
    pub fn ring_stats(&self) -> (u64, u64) {
        (
            self.playback.underflow_count(),
            self.capture.overflow_count(),
        )
    }
}
