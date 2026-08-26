//! C ABI for the TuxMix Babyface Pro FS proprietary-mode audio driver.
//!
//! The ALSA plugin (`tools/alsa/pcm_tuxmix.c`) links against this
//! cdylib. The handle wraps the `BabyfaceAudio` device in a mutex (the
//! plugin may call from any thread). All buffers are interleaved
//! **S24_LE** (3 bytes/sample):
//!
//! - playback: `PLAYBACK_CHANNELS` (4) channels per frame — device
//!   ch0-3 = PB1 (ch0/1) + PB2 (ch2/3) into the TotalMix mixer.
//! - capture: `CAPTURE_CHANNELS` (4) channels per frame — AN1-4.
//!
//! Frame rates: the device frame layout is 56/40/32 bytes at the
//! 32-88.2k / 96-128k / 176.4-192k classes (see PROTOCOL.md "Frame
//! format"); the conversions live in `tuxmix_usb::audio`.

use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tuxmix_usb::BabyfaceAudio;

/// Opaque handle: the audio device + rings + pump thread.
pub struct AudioHandle {
    audio: Mutex<BabyfaceAudio>,
}

/// One shared device session per PROCESS (the ALSA plugin is loaded
/// once; PipeWire opens the sink AND the source in the same process —
/// they must share a single streaming session, since the device has
/// only one). The handle pointer outlives the calls: the global keeps
/// the Arc alive until the refcount drops to zero (`tuxmix_audio_close`).
static DEV: Mutex<Option<Arc<AudioHandle>>> = Mutex::new(None);
static REFS: AtomicUsize = AtomicUsize::new(0);

/// Returns a handle or NULL on failure (device not found, eventfd, …).
/// The handle is process-shared: playback and capture PCMs opened by
/// the same process (PipeWire) get the SAME session.
#[no_mangle]
pub extern "C" fn tuxmix_audio_open() -> *mut c_void {
    let mut g = DEV.lock().unwrap();
    if g.is_none() {
        match BabyfaceAudio::open() {
            Ok(a) => {
                *g = Some(Arc::new(AudioHandle {
                    audio: Mutex::new(a),
                }))
            }
            Err(e) => {
                eprintln!("tuxmix_audio_open: {e}");
                return std::ptr::null_mut();
            }
        }
    }
    REFS.fetch_add(1, Ordering::AcqRel);
    Arc::as_ptr(g.as_ref().unwrap()) as *mut c_void
}

/// Start the streaming session (DSP audio + the pump thread). 0 = ok.
#[no_mangle]
pub extern "C" fn tuxmix_audio_start(h: *mut c_void) -> c_int {
    match handle_mut(h).audio.lock() {
        Ok(mut a) => a.start().map(|_| 0).unwrap_or_else(|e| {
            eprintln!("tuxmix_audio_start: {e}");
            1
        }),
        Err(_) => 1,
    }
}

/// Stop streaming + release the interface. 0 = ok.
#[no_mangle]
pub extern "C" fn tuxmix_audio_stop(h: *mut c_void) -> c_int {
    match handle_mut(h).audio.lock() {
        Ok(mut a) => a.stop().map(|_| 0).unwrap_or_else(|e| {
            eprintln!("tuxmix_audio_stop: {e}");
            1
        }),
        Err(_) => 1,
    }
}

/// Close the device and free the handle (the LAST close of the
/// process stops the session + releases the device so other clients —
/// the TuxMix GUI — can use it).
#[no_mangle]
pub extern "C" fn tuxmix_audio_close(_h: *mut c_void) {
    if REFS.fetch_sub(1, Ordering::AcqRel) == 1 {
        if let Some(dev) = DEV.lock().unwrap().take() {
            if let Ok(mut a) = dev.audio.lock() {
                let _ = a.stop();
            }
        }
    }
}

/// Set the sample rate in Hz. 0 = ok (no-op if unchanged).
#[no_mangle]
pub extern "C" fn tuxmix_audio_set_rate(h: *mut c_void, rate: u32) -> c_int {
    match handle_mut(h).audio.lock() {
        Ok(mut a) => a.set_rate(rate).map(|_| 0).unwrap_or_else(|e| {
            eprintln!("tuxmix_audio_set_rate({rate}): {e}");
            1
        }),
        Err(_) => 1,
    }
}

/// The active sample rate in Hz.
#[no_mangle]
pub extern "C" fn tuxmix_audio_rate(h: *mut c_void) -> u32 {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.sample_rate())
        .unwrap_or(0)
}

/// Push `frames` of interleaved S24_LE (`channels` 2 or 4) playback
/// audio. Returns the frames accepted (== frames unless the ring is
/// full — then the caller returns the short count so the app paces).
#[no_mangle]
pub extern "C" fn tuxmix_audio_write_playback(
    h: *mut c_void,
    buf: *const u8,
    frames: usize,
    channels: usize,
) -> usize {
    let channels = channels.clamp(2, 4);
    let bytes = frames * channels * 3;
    if buf.is_null() || bytes == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(buf, bytes) };
    handle_mut(h)
        .audio
        .lock()
        .map(|mut a| a.write_playback(slice, frames, channels))
        .unwrap_or(0)
}

/// Read up to `frames` of interleaved S24_LE (`channels` 2 or 4) into
/// `buf`. Returns the frames actually read (< requested = drained).
#[no_mangle]
pub extern "C" fn tuxmix_audio_read_capture(
    h: *mut c_void,
    buf: *mut u8,
    frames: usize,
    channels: usize,
) -> usize {
    let channels = channels.clamp(2, 4);
    let bytes = frames * channels * 3;
    if buf.is_null() || bytes == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, bytes) };
    handle_mut(h)
        .audio
        .lock()
        .map(|mut a| a.read_capture(slice, frames, channels))
        .unwrap_or(0)
}

/// The capture wakeup fd: readable when capture frames are queued
/// (the ALSA plugin's poll descriptor). -1 if unavailable.
#[no_mangle]
pub extern "C" fn tuxmix_audio_capture_fd(h: *mut c_void) -> c_int {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.capture_fd())
        .unwrap_or(-1)
}

/// Capture frames currently queued.
#[no_mangle]
pub extern "C" fn tuxmix_audio_capture_queued(h: *mut c_void) -> usize {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.capture_queued())
        .unwrap_or(0)
}

/// Playback frames currently queued (for snd_pcm_delay).
#[no_mangle]
pub extern "C" fn tuxmix_audio_playback_queued(h: *mut c_void) -> usize {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.playback_queued())
        .unwrap_or(0)
}

/// Playback ring capacity in frames (the poll path's "has space"
/// condition).
#[no_mangle]
pub extern "C" fn tuxmix_audio_playback_capacity(h: *mut c_void) -> usize {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.playback_capacity())
        .unwrap_or(0)
}

/// Playback frames pushed since start (monotonic hw position).
#[no_mangle]
pub extern "C" fn tuxmix_audio_playback_pushed(h: *mut c_void) -> u64 {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.playback_pushed())
        .unwrap_or(0)
}

/// Capture frames produced by the device since start (monotonic hw
/// position).
#[no_mangle]
pub extern "C" fn tuxmix_audio_capture_pushed(h: *mut c_void) -> u64 {
    handle_mut(h)
        .audio
        .lock()
        .map(|a| a.capture_pushed())
        .unwrap_or(0)
}

fn handle_mut<'a>(h: *mut c_void) -> &'a mut AudioHandle {
    assert!(!h.is_null(), "tuxmix-sys: null handle");
    unsafe { &mut *(h as *mut AudioHandle) }
}
