//! Interrupt-transfer audio streaming (raw libusb async, no rusb sync API).
//!
//! The Babyface Pro FS' real audio stream runs on **interface 5**,
//! endpoints `0x01` OUT / `0x82` IN, as **INTERRUPT** transfers
//! (bmAttributes 0x03 = `USB_ENDPOINT_XFER_INT` per the USB spec — a
//! previous "settled" conclusion misread this as isochronous; the
//! kernel's own parse agrees, see `/sys/kernel/debug/usb/devices`:
//! `Atr=03(Int.)`), alt-setting 1 = 448-byte packets.
//!
//! Confirmed on real hardware (usbmon + the front-panel 48V LED):
//! - The device produces real audio (the 14×32-bit frame format
//!   PROTOCOL.md decodes) only when the URBs are **14336 bytes** (256
//!   frames — the Windows driver's URB size) and several are kept in
//!   flight per endpoint (~8+), continuously resubmitted: the audio
//!   then flows at exactly 48 kHz (14336 B × ~187/s = 2.7 MB/s) and
//!   **48V phantom power physically engages** (the front-panel LED
//!   lights) after `0x17 0x000D wIndex=0x003F` + `0x21` is written
//!   (0x000D = 48V ON — verified 2026-08-22).
//! - Smaller 448-byte URBs complete but the DSP never starts producing
//!   audio (IN completions carry 0 bytes) and 48V never engages.
//! - The device only completes URBs while **both** the OUT and IN
//!   endpoint have pending URBs, and the streams must be submitted
//!   together and resubmitted on completion — a synchronous per-URB
//!   submit-and-wait API (rusb) can't express that, hence raw libusb.

use std::collections::VecDeque;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libusb1_sys::{
    constants::LIBUSB_TRANSFER_COMPLETED, libusb_alloc_transfer, libusb_cancel_transfer,
    libusb_context, libusb_device_handle, libusb_fill_interrupt_transfer, libusb_free_transfer,
    libusb_handle_events_timeout, libusb_submit_transfer, libusb_transfer,
};

use super::Error;

/// URBs kept in flight per endpoint. The Windows driver queues ~32
/// 14336-B URBs per endpoint; 8 was verified on hardware to start the
/// DSP (48 kHz audio + 48V engagement).
pub const URBS_IN_FLIGHT: usize = 8;

/// 2^23 — 24-bit full scale (the frame format is 24-bit audio).
const FULL_SCALE: f32 = 8_388_608.0;

/// Linux `eventfd(2)` used as a capture-wakeup: the IN transfer
/// callback signals it on every pushed frame, the ALSA plugin polls it
/// so capture apps don't spin (Linux-only, like the whole driver).
///
/// The type always exists (the ring's `wake` field is platform-
/// neutral) but `new()` only succeeds on Linux — mingw's libc has no
/// `eventfd` (verified 2026-08-24: libc 0.2.186 defines it for
/// linux/hermit only), so on other targets `signal`/`drain` are no-ops.
pub struct EventFd(std::os::raw::c_int);

impl EventFd {
    /// Create a new non-blocking, close-on-exec eventfd (counter 0).
    #[cfg(target_os = "linux")]
    pub fn new() -> Result<Self, Error> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(Error::EventFdFailed);
        }
        Ok(Self(fd))
    }

    /// No eventfd off Linux (mingw libc lacks it) — the ALSA wakeup
    /// path is Linux-only anyway.
    #[cfg(not(target_os = "linux"))]
    pub fn new() -> Result<Self, Error> {
        Err(Error::EventFdFailed)
    }

    pub fn as_raw_fd(&self) -> std::os::raw::c_int {
        self.0
    }

    /// Increment the counter (non-blocking; EAGAIN on overflow is fine
    /// — the fd stays readable).
    #[cfg(target_os = "linux")]
    pub fn signal(&self) {
        let one: u64 = 1;
        unsafe {
            libc::write(self.0, &one as *const u64 as *const libc::c_void, 8);
        }
    }

    /// No-op off Linux (the fd can't exist there).
    #[cfg(not(target_os = "linux"))]
    pub fn signal(&self) {}

    /// Drain the counter (make the fd unreadable again).
    #[cfg(target_os = "linux")]
    pub fn drain(&self) {
        let mut buf = [0u8; 8];
        unsafe {
            libc::read(self.0, buf.as_mut_ptr() as *mut libc::c_void, 8);
        }
    }

    /// No-op off Linux.
    #[cfg(not(target_os = "linux"))]
    pub fn drain(&self) {}
}

impl Drop for EventFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

/// SPSC-style byte ring for real audio (device frame format, frame-
/// aligned): the OUT stream's transfer callback PULLS frames from it
/// before resubmitting (zero-padding on underrun), the IN stream's
/// callback PUSHES the received frames into it (dropping oldest on
/// overrun). The producer/consumer live on different threads
/// (app/plugin vs the libusb pump thread), so the inner state is
/// mutex-protected — low contention, correctness first.
///
/// Capacity is in FRAMES so the queue stays frame-aligned; the caller
/// only ever pushes/pulls whole frames of `frame_size` bytes.
pub struct AudioRing {
    inner: Mutex<RingInner>,
    frame_size: usize,
    /// Optional capture-wakeup: signalled on every push so a poll-based
    /// app (ALSA) wakes immediately instead of spinning.
    wake: Option<Arc<EventFd>>,
    /// Total frames pushed since creation — the monotonic hw position
    /// source for the ALSA plugin (playback hw = pushed - queued,
    /// capture hw = pushed).
    pushed: AtomicU64,
}

struct RingInner {
    /// Queued device-format bytes (always a multiple of `frame_size`).
    data: VecDeque<u8>,
    /// Max queued frames (the device-format bytes / frame_size).
    cap_frames: usize,
    /// OUT: zero frames inserted on underrun (silence sent).
    underflow: u64,
    /// IN: frames dropped on overrun.
    overflow: u64,
}

impl AudioRing {
    /// New ring for `frame_size`-byte frames, `cap_frames` frames deep.
    pub fn new(frame_size: usize, cap_frames: usize) -> Self {
        Self::with_wake(frame_size, cap_frames, None)
    }

    /// [`new`][Self::new] with a capture wakeup fd.
    pub fn with_wake(frame_size: usize, cap_frames: usize, wake: Option<Arc<EventFd>>) -> Self {
        Self {
            inner: Mutex::new(RingInner {
                data: VecDeque::with_capacity(frame_size * cap_frames),
                cap_frames,
                underflow: 0,
                overflow: 0,
            }),
            frame_size,
            wake,
            pushed: AtomicU64::new(0),
        }
    }

    /// Push one full frame (must be exactly `frame_size` bytes).
    /// Drops the OLDEST queued frame if the ring is full (capture
    /// overrun — keep the freshest audio).
    pub fn push_frame(&self, frame: &[u8]) {
        let mut g = self.inner.lock().unwrap();
        if g.data.len() / self.frame_size >= g.cap_frames {
            g.data.drain(0..self.frame_size);
            g.overflow += 1;
        }
        g.data.extend(frame.iter().copied());
        self.pushed.fetch_add(1, Ordering::Relaxed);
        if let Some(w) = &self.wake {
            w.signal();
        }
    }

    /// Playback push: only when there is room — returns false when the
    /// ring is full so the caller (the ALSA transfer) can return a
    /// short count and let the app pace itself (a full ring here means
    /// the device is behind; dropping the OLDEST would skip audio).
    pub fn push_frame_if_space(&self, frame: &[u8]) -> bool {
        let mut g = self.inner.lock().unwrap();
        if g.data.len() / self.frame_size >= g.cap_frames {
            return false;
        }
        g.data.extend(frame.iter().copied());
        self.pushed.fetch_add(1, Ordering::Relaxed);
        if let Some(w) = &self.wake {
            w.signal();
        }
        true
    }

    /// Pull up to `n` frames into `out` (device format). Returns the
    /// number of frames written; the rest of `out` is untouched (the
    /// caller zero-fills for silence). Signals the wake when it frees
    /// space (the playback poll path).
    pub fn pull_frames(&self, out: &mut [u8]) -> usize {
        let mut g = self.inner.lock().unwrap();
        let want = out.len() / self.frame_size;
        let have = g.data.len() / self.frame_size;
        let n = want.min(have);
        if n > 0 {
            let bytes = n * self.frame_size;
            for (dst, src) in out[..bytes].iter_mut().zip(g.data.drain(..bytes)) {
                *dst = src;
            }
            if let Some(w) = &self.wake {
                w.signal();
            }
        }
        n
    }

    /// OUT underruns (silence frames inserted) since creation.
    pub fn underflow_count(&self) -> u64 {
        self.inner.lock().unwrap().underflow
    }

    /// IN overruns (frames dropped) since creation.
    pub fn overflow_count(&self) -> u64 {
        self.inner.lock().unwrap().overflow
    }

    /// Frames currently queued.
    pub fn queued_frames(&self) -> usize {
        let g = self.inner.lock().unwrap();
        g.data.len() / self.frame_size
    }

    /// Total frames pushed since creation (monotonic hw position).
    pub fn pushed_frames(&self) -> u64 {
        self.pushed.load(Ordering::Relaxed)
    }

    /// Ring capacity in frames.
    pub fn capacity_frames(&self) -> usize {
        self.inner.lock().unwrap().cap_frames
    }
}

/// Per-channel input-level accumulator for the IN stream (ch0-3 =
/// AN1-4). Updated from the transfer callback (which runs on the pump
/// thread, same thread as the reader — the Mutex is uncontended
/// insurance) and drained by [`IntrStream::drain_peaks`].
pub struct MeterAccum {
    /// Bytes per audio frame at the active rate (56 B = 14 ch ≤ 64k,
    /// 40 B = 10 ch at 88.2-128k, 32 B = 8 ch at 176.4/192k — the
    /// analog pair stays at ch0-3 in all layouts).
    frame_size: usize,
    /// Max |sample| per channel since the last drain (2^23 full scale).
    peak: [u32; 4],
}

impl MeterAccum {
    /// New accumulator for a given frame layout (see [`crate::protocol::rate_to_alt`]).
    pub fn new(frame_size: usize) -> Self {
        Self {
            frame_size,
            peak: [0; 4],
        }
    }

    /// Fold one completed IN URB into the peaks: `frame_size`-byte
    /// frames, the 24-bit sample lives in bytes 1-3 little-endian
    /// (byte 0 = 0x00 / frame marker), recovered as `(word as i32) >> 8`
    /// — see PROTOCOL.md "Frame format".
    fn accumulate(&mut self, buf: &[u8]) {
        let n = buf.len() / self.frame_size;
        for f in 0..n {
            let base = f * self.frame_size;
            for c in 0..4 {
                let word = u32::from_le_bytes([
                    buf[base + c * 4],
                    buf[base + c * 4 + 1],
                    buf[base + c * 4 + 2],
                    buf[base + c * 4 + 3],
                ]);
                let mag = ((word as i32) >> 8).unsigned_abs();
                if mag > self.peak[c] {
                    self.peak[c] = mag;
                }
            }
        }
    }

    /// Peak per channel since the last drain, as 0..1 of full scale,
    /// and reset the accumulator.
    fn drain(&mut self) -> [f32; 4] {
        let out = self.peak.map(|p| p as f32 / FULL_SCALE);
        self.peak = [0; 4];
        out
    }
}

struct TransferCtx {
    keep_running: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    completed_ok: Arc<AtomicUsize>,
    completed_err: Arc<AtomicUsize>,
    /// libusb status of the most recent non-OK completion, for diagnostics.
    last_err_status: Arc<AtomicI32>,
    /// Present on IN streams: accumulate per-channel input levels.
    meters: Option<Arc<Mutex<MeterAccum>>>,
    /// Present when real audio is wired: OUT pulls the transfer buffer
    /// from this ring before resubmit, IN pushes the received frames in.
    audio: Option<Arc<AudioRing>>,
}

/// libusb transfer-completion callback: drain the IN buffer (meters /
/// capture ring), refill the OUT buffer from the playback ring, and
/// resubmit while `keep_running`; otherwise let the transfer end and
/// mark it inactive.
extern "system" fn on_complete(transfer: *mut libusb_transfer) {
    unsafe {
        let ctx = &*((*transfer).user_data as *const TransferCtx);
        let status = (*transfer).status;
        let is_in = (*transfer).endpoint & 0x80 != 0;
        if status == LIBUSB_TRANSFER_COMPLETED {
            ctx.completed_ok.fetch_add(1, Ordering::Relaxed);
            let len = ((*transfer).actual_length.min((*transfer).length)) as usize;
            let buf = std::slice::from_raw_parts((*transfer).buffer, len);
            if is_in {
                // IN: fold the peaks (if metering) and push the frames
                // into the capture ring (frame-aligned).
                if let Some(meters) = &ctx.meters {
                    if let Ok(mut acc) = meters.lock() {
                        acc.accumulate(buf);
                    }
                }
                if let Some(ring) = &ctx.audio {
                    let aligned = len / ring.frame_size * ring.frame_size;
                    for f in 0..aligned / ring.frame_size {
                        let base = f * ring.frame_size;
                        ring.push_frame(&buf[base..base + ring.frame_size]);
                    }
                }
            }
        } else {
            ctx.completed_err.fetch_add(1, Ordering::Relaxed);
            ctx.last_err_status.store(status, Ordering::Relaxed);
        }
        let resubmit = if ctx.keep_running.load(Ordering::Acquire) {
            if !is_in {
                // OUT: refill the whole transfer buffer from the
                // playback ring (silence on underrun) before resubmit.
                if let Some(ring) = &ctx.audio {
                    let n = (*transfer).length as usize;
                    let b = std::slice::from_raw_parts_mut((*transfer).buffer, n);
                    b.fill(0);
                    let pulled = ring.pull_frames(b);
                    if pulled == 0 {
                        let mut g = ring.inner.lock().unwrap();
                        g.underflow += 1;
                    }
                }
            }
            libusb_submit_transfer(transfer) == 0
        } else {
            false
        };
        if resubmit {
            return;
        }
        ctx.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Snapshot of a stream's transfer completions, for diagnostics.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamStats {
    pub completed_ok: usize,
    pub completed_err: usize,
    /// `LIBUSB_TRANSFER_*` status of the most recent non-OK completion.
    pub last_err_status: i32,
}

/// A running interrupt stream on one endpoint: [`URBS_IN_FLIGHT`] URBs
/// continuously resubmitting until [`stop`][Self::stop]ped.
pub struct IntrStream {
    ctx_raw: *mut libusb_context,
    transfers: Vec<*mut libusb_transfer>,
    buffers: Vec<(*mut u8, usize)>,
    ctxs: Vec<*mut TransferCtx>,
    keep_running: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    completed_ok: Arc<AtomicUsize>,
    completed_err: Arc<AtomicUsize>,
    last_err_status: Arc<AtomicI32>,
    /// Shared with every transfer's callback; only the IN stream has it.
    meters: Option<Arc<Mutex<MeterAccum>>>,
    /// Real-audio ring (OUT = playback pull, IN = capture push). The
    /// field is the Arc KEEPER — it guarantees the ring outlives the
    /// transfers (whose callbacks hold their own clones).
    #[allow(dead_code)]
    audio: Option<Arc<AudioRing>>,
}

// The raw pointers here are libusb handles/buffers used only through
// libusb's own thread-safe API (protected by its internal locking); no
// Rust-side aliasing occurs across threads.
unsafe impl Send for IntrStream {}

impl IntrStream {
    /// Start streaming on `endpoint` (with its direction bit, e.g. `0x01`
    /// OUT or `0x82` IN), sized for `max_packet_size` bytes per transfer
    /// (14336 = the audio URB size, see the module docs), with
    /// [`URBS_IN_FLIGHT`] URBs queued.
    ///
    /// # Safety
    ///
    /// `ctx_raw` and `dev_handle` must be valid, currently-open libusb
    /// context/device-handle pointers (e.g. from
    /// `rusb::UsbContext::as_raw`/`rusb::DeviceHandle::as_raw`) that
    /// outlive the returned `IntrStream`.
    pub unsafe fn start(
        ctx_raw: *mut libusb_context,
        dev_handle: *mut libusb_device_handle,
        endpoint: u8,
        max_packet_size: usize,
        meters: Option<Arc<Mutex<MeterAccum>>>,
        audio: Option<Arc<AudioRing>>,
    ) -> Result<Self, Error> {
        let keep_running = Arc::new(AtomicBool::new(true));
        let active = Arc::new(AtomicUsize::new(0));
        let completed_ok = Arc::new(AtomicUsize::new(0));
        let completed_err = Arc::new(AtomicUsize::new(0));
        let last_err_status = Arc::new(AtomicI32::new(0));
        let mut transfers = Vec::with_capacity(URBS_IN_FLIGHT);
        let mut buffers = Vec::with_capacity(URBS_IN_FLIGHT);
        let mut ctxs = Vec::with_capacity(URBS_IN_FLIGHT);

        for _ in 0..URBS_IN_FLIGHT {
            unsafe {
                let transfer = libusb_alloc_transfer(0);
                if transfer.is_null() {
                    free_all(&transfers, &buffers, &ctxs);
                    return Err(Error::StreamAlloc);
                }

                let buffer_ptr =
                    Box::into_raw(vec![0u8; max_packet_size].into_boxed_slice()) as *mut u8;
                let ctx = Box::into_raw(Box::new(TransferCtx {
                    keep_running: keep_running.clone(),
                    active: active.clone(),
                    completed_ok: completed_ok.clone(),
                    completed_err: completed_err.clone(),
                    last_err_status: last_err_status.clone(),
                    meters: meters.clone(),
                    audio: audio.clone(),
                }));

                libusb_fill_interrupt_transfer(
                    transfer,
                    dev_handle,
                    endpoint,
                    buffer_ptr,
                    max_packet_size as c_int,
                    on_complete,
                    ctx as *mut _,
                    0,
                );

                let rc = libusb_submit_transfer(transfer);
                if rc != 0 {
                    drop(Box::from_raw(ctx));
                    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                        buffer_ptr,
                        max_packet_size,
                    )));
                    libusb_free_transfer(transfer);
                    free_all(&transfers, &buffers, &ctxs);
                    return Err(Error::StreamSubmit(rc));
                }

                active.fetch_add(1, Ordering::AcqRel);
                transfers.push(transfer);
                buffers.push((buffer_ptr, max_packet_size));
                ctxs.push(ctx);
            }
        }

        Ok(Self {
            ctx_raw,
            transfers,
            buffers,
            ctxs,
            keep_running,
            active,
            completed_ok,
            completed_err,
            last_err_status,
            meters,
            audio,
        })
    }

    /// Drain the per-channel input peak levels (ch0-3 = AN1-4, 0..1 of
    /// full scale) accumulated since the last call, resetting the
    /// accumulator. `None` if this stream has no meter state (OUT).
    pub fn drain_peaks(&self) -> Option<[f32; 4]> {
        let meters = self.meters.as_ref()?;
        let mut acc = meters.lock().ok()?;
        Some(acc.drain())
    }

    /// Diagnostics for whether the queued transfers are actually landing
    /// on the bus.
    pub fn stats(&self) -> StreamStats {
        StreamStats {
            completed_ok: self.completed_ok.load(Ordering::Relaxed),
            completed_err: self.completed_err.load(Ordering::Relaxed),
            last_err_status: self.last_err_status.load(Ordering::Relaxed),
        }
    }

    /// Pump the libusb event loop so submitted/completed transfers make
    /// progress. Call this periodically while the stream should be
    /// running — nothing advances otherwise.
    pub fn pump(&self, timeout: Duration) {
        let tv = libc::timeval {
            tv_sec: timeout.as_secs() as _,
            tv_usec: timeout.subsec_micros() as _,
        };
        unsafe {
            libusb_handle_events_timeout(self.ctx_raw, &tv);
        }
    }
}

impl Drop for IntrStream {
    fn drop(&mut self) {
        self.keep_running.store(false, Ordering::Release);
        unsafe {
            for &t in &self.transfers {
                libusb_cancel_transfer(t);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.active.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
            self.pump(Duration::from_millis(50));
        }
        unsafe {
            free_all(&self.transfers, &self.buffers, &self.ctxs);
        }
    }
}

unsafe fn free_all(
    transfers: &[*mut libusb_transfer],
    buffers: &[(*mut u8, usize)],
    ctxs: &[*mut TransferCtx],
) {
    for &t in transfers {
        libusb_free_transfer(t);
    }
    for &(ptr, len) in buffers {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
    }
    for &c in ctxs {
        drop(Box::from_raw(c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_accum_decodes_24bit_frames() {
        // One 56-byte frame; per channel the 24-bit sample lives in
        // bytes 1-3 LE (byte 0 = frame marker), recovered as
        // `(word as i32) >> 8`.
        let mut buf = [0u8; 56];
        // ch0 = +0.5 FS (0x400000)
        buf[1] = 0x00;
        buf[2] = 0x00;
        buf[3] = 0x40;
        // ch1 = -0.5 FS (0xFFC00000 >> 8)
        buf[5] = 0x00;
        buf[6] = 0x00;
        buf[7] = 0xC0;
        // ch2 = silence
        // ch3 = +0.99999988 FS (0x7FFFFF)
        buf[13] = 0xFF;
        buf[14] = 0xFF;
        buf[15] = 0x7F;

        let mut acc = MeterAccum::new(56); // alt-1 frame (48 kHz)
        acc.accumulate(&buf);
        let peaks = acc.drain();
        assert!((peaks[0] - 0.5).abs() < 1e-6, "ch0 = {}", peaks[0]);
        assert!((peaks[1] - 0.5).abs() < 1e-6, "ch1 = {}", peaks[1]);
        assert_eq!(peaks[2], 0.0);
        assert!(
            (peaks[3] - (8388607.0 / 8388608.0)).abs() < 1e-6,
            "ch3 = {}",
            peaks[3]
        );
        // Draining resets the accumulator.
        assert_eq!(acc.drain(), [0.0; 4]);
    }
}
