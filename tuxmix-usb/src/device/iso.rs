//! Raw libusb isochronous transfer support.
//!
//! ⚠️ **SUPERSEDED** — the real audio stream is INTERRUPT transfers on
//! interface 5 (`0x01`/`0x82`, bmAttributes 0x03 = `USB_ENDPOINT_XFER_INT`),
//! not isochronous. Submitting ISO URBs to those endpoints makes the
//! kernel reject them with `EINVAL`. The working path is
//! [`super::IntrStream`] (plain rusb interrupt transfers). This module
//! is kept for reference only.
//!
//! Neither `rusb` nor `nusb` (checked as of nusb 0.2.7, the latest
//! published version at the time this was written) submit isochronous
//! transfers — both stop at Bulk/Interrupt. On real hardware, the
//! Babyface Pro FS only physically engages 48V phantom power while an
//! audio session is streaming (confirmed: writing the preamp register
//! alone leaves the register readback saying "on" but the front-panel
//! LED off). Getting a real stream running therefore requires talking to
//! `libusb1-sys` directly, via the raw pointers `rusb::DeviceHandle`
//! exposes exactly for this ("advanced use in unsafe code").
//!
//! This module only pushes silence/discards received audio — it exists
//! to keep the interlock satisfied and, later, as the base for real
//! audio I/O and VU metering (see `PROTOCOL.md`'s frame format).

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libusb1_sys::{
    constants::LIBUSB_TRANSFER_COMPLETED, libusb_alloc_transfer, libusb_cancel_transfer,
    libusb_context, libusb_device_handle, libusb_fill_iso_transfer, libusb_free_transfer,
    libusb_handle_events_timeout, libusb_set_iso_packet_lengths, libusb_submit_transfer,
    libusb_transfer,
};

use super::Error;

/// Isochronous packets bundled into a single URB.
const PACKETS_PER_URB: c_int = 8;

struct TransferCtx {
    keep_running: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    completed_ok: Arc<AtomicUsize>,
    completed_err: Arc<AtomicUsize>,
    /// Status code (`LIBUSB_TRANSFER_*`) of the most recent non-OK
    /// completion, for diagnostics.
    last_err_status: Arc<AtomicI32>,
}

/// libusb transfer-completion callback: resubmit while `keep_running`,
/// otherwise let the transfer end and mark it inactive.
extern "system" fn on_complete(transfer: *mut libusb_transfer) {
    unsafe {
        let ctx = &*((*transfer).user_data as *const TransferCtx);
        let status = (*transfer).status;
        if status == LIBUSB_TRANSFER_COMPLETED {
            ctx.completed_ok.fetch_add(1, Ordering::Relaxed);
        } else {
            ctx.completed_err.fetch_add(1, Ordering::Relaxed);
            ctx.last_err_status.store(status, Ordering::Relaxed);
        }
        if ctx.keep_running.load(Ordering::Acquire) && libusb_submit_transfer(transfer) == 0 {
            return;
        }
        ctx.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Snapshot of an [`IsoStream`]'s transfer completions, for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct IsoStats {
    pub completed_ok: usize,
    pub completed_err: usize,
    /// `LIBUSB_TRANSFER_*` status of the most recent non-OK completion.
    pub last_err_status: i32,
}

/// A running isochronous stream on one endpoint, continuously
/// resubmitting its URBs until [`stop`][Self::stop]ped.
pub struct IsoStream {
    ctx_raw: *mut libusb_context,
    transfers: Vec<*mut libusb_transfer>,
    buffers: Vec<(*mut u8, usize)>,
    ctxs: Vec<*mut TransferCtx>,
    keep_running: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    completed_ok: Arc<AtomicUsize>,
    completed_err: Arc<AtomicUsize>,
    last_err_status: Arc<AtomicI32>,
}

// The raw pointers here are libusb handles/buffers used only through
// libusb's own thread-safe API (protected by its internal locking); no
// Rust-side aliasing occurs across threads.
unsafe impl Send for IsoStream {}

impl IsoStream {
    /// Start streaming on `endpoint` (with its direction bit, e.g. `0x01`
    /// OUT or `0x82` IN), sized for `max_packet_size` bytes/packet from
    /// the active alternate setting's endpoint descriptor.
    ///
    /// # Safety
    ///
    /// `ctx_raw` and `dev_handle` must be valid, currently-open libusb
    /// context/device-handle pointers (e.g. from
    /// `rusb::UsbContext::as_raw`/`rusb::DeviceHandle::as_raw`) that
    /// outlive the returned `IsoStream`.
    pub unsafe fn start(
        ctx_raw: *mut libusb_context,
        dev_handle: *mut libusb_device_handle,
        endpoint: u8,
        max_packet_size: usize,
        n_urbs: usize,
    ) -> Result<Self, Error> {
        let keep_running = Arc::new(AtomicBool::new(true));
        let active = Arc::new(AtomicUsize::new(0));
        let completed_ok = Arc::new(AtomicUsize::new(0));
        let completed_err = Arc::new(AtomicUsize::new(0));
        let last_err_status = Arc::new(AtomicI32::new(0));
        let mut transfers = Vec::with_capacity(n_urbs);
        let mut buffers = Vec::with_capacity(n_urbs);
        let mut ctxs = Vec::with_capacity(n_urbs);
        let buf_len = max_packet_size * PACKETS_PER_URB as usize;

        for _ in 0..n_urbs {
            unsafe {
                let transfer = libusb_alloc_transfer(PACKETS_PER_URB);
                if transfer.is_null() {
                    free_all(&transfers, &buffers, &ctxs);
                    return Err(Error::StreamAlloc);
                }

                let buffer_ptr = Box::into_raw(vec![0u8; buf_len].into_boxed_slice()) as *mut u8;
                let ctx = Box::into_raw(Box::new(TransferCtx {
                    keep_running: keep_running.clone(),
                    active: active.clone(),
                    completed_ok: completed_ok.clone(),
                    completed_err: completed_err.clone(),
                    last_err_status: last_err_status.clone(),
                }));

                libusb_fill_iso_transfer(
                    transfer,
                    dev_handle,
                    endpoint,
                    buffer_ptr,
                    buf_len as c_int,
                    PACKETS_PER_URB,
                    on_complete,
                    ctx as *mut _,
                    1000,
                );
                libusb_set_iso_packet_lengths(transfer, max_packet_size as u32);

                let rc = libusb_submit_transfer(transfer);
                if rc != 0 {
                    drop(Box::from_raw(ctx));
                    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                        buffer_ptr, buf_len,
                    )));
                    libusb_free_transfer(transfer);
                    free_all(&transfers, &buffers, &ctxs);
                    return Err(Error::StreamSubmit(rc));
                }

                active.fetch_add(1, Ordering::AcqRel);
                transfers.push(transfer);
                buffers.push((buffer_ptr, buf_len));
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
        })
    }

    /// Diagnostics for whether the queued transfers are actually landing
    /// on the bus.
    pub fn stats(&self) -> IsoStats {
        IsoStats {
            completed_ok: self.completed_ok.load(Ordering::Relaxed),
            completed_err: self.completed_err.load(Ordering::Relaxed),
            last_err_status: self.last_err_status.load(Ordering::Relaxed),
        }
    }

    /// Pump the libusb event loop so submitted/completed transfers make
    /// progress. Call this periodically while the stream should be
    /// running — nothing advances otherwise.
    pub fn pump(&self, timeout: Duration) {
        // `libusb1_sys` declares `libusb_handle_events_timeout` against
        // `libc::timeval`; its field types differ per platform
        // (`suseconds_t` on Unix), so let the casts infer from the
        // struct's own fields instead of naming them.
        let tv = libc::timeval {
            tv_sec: timeout.as_secs() as _,
            tv_usec: timeout.subsec_micros() as _,
        };
        unsafe {
            libusb_handle_events_timeout(self.ctx_raw, &tv);
        }
    }

    /// Stop the stream: cancel all transfers, pump events until they've
    /// wound down (or a 2s deadline passes), then free everything.
    ///
    /// Equivalent to dropping the `IsoStream` — this method exists for a
    /// clearer call site; the real cleanup lives in `Drop` so a stream
    /// dropped on an error path (e.g. its sibling OUT/IN stream failing
    /// to start) still gets torn down instead of leaking its transfers.
    pub fn stop(self) {}
}

impl Drop for IsoStream {
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
