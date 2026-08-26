//! libusb-backed device driver (feature `driver`).
//!
//! Opens the Babyface Pro FS, sends the vendor-control requests and
//! drives the audio-streaming endpoints (interface 5, interrupt
//! transfers) for VU metering.
//!
//! On Linux the kernel's `snd-usb-audio` may claim the device; the
//! driver auto-detaches it. Note that the device in proprietary mode
//! enumerates as vendor-specific (class 0xFF) — unbind or blacklist
//! `snd-usb-audio` for this device if both want to use it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusb::UsbContext;

use crate::map::{Output, Source};
use crate::protocol::{self, FlagCounter, VendorRequest};

mod iso;
mod stream;
pub use iso::{IsoStats, IsoStream}; // superseded (isochronous) — kept for reference
pub use stream::{AudioRing, EventFd, IntrStream, MeterAccum, StreamStats};

/// Babyface Pro FS USB identifiers (proprietary mode).
pub const VID: u16 = 0x2A39;
pub const PID: u16 = 0x3FC0;

/// Audio streaming interface and endpoints — confirmed against the real
/// device descriptor AND a cold-plug capture of TotalMix on Windows
/// (cap_coldplug.pcap): the audio runs on **interface 5**, endpoints
/// `0x01` OUT / `0x82` IN, as **INTERRUPT transfers** (bmAttributes
/// 0x03 = `USB_ENDPOINT_XFER_INT`; the USB spec maps 1 = isochronous,
/// 3 = interrupt — earlier RE notes had this backwards). The driver
/// selects **alt-setting 1** (`SET_INTERFACE wVal=1 wIdx=5`, 448-byte
/// packets) and then sends the [`protocol::streaming_init`] sequence
/// before starting the interrupt URBs (14336-B = 32×448-B URBs every
/// ~5.33 ms in the capture).
///
/// ⚠️ NOT interface 0 (`0x03`/`0x84`): those are ISOCHRONOUS endpoints
/// (bmAttributes 0x01, 420/396 B at alt 1) and saw zero packets in the
/// cold-start capture — streaming them does not engage the device.
pub const AUDIO_INTERFACE: u8 = 5;
/// Alt-setting the driver starts at (48 kHz): 1 = 448-byte packets.
/// `set_sample_rate` switches alt 1/2/3 via SET_INTERFACE — see
/// [`protocol::rate_to_alt`].
pub const AUDIO_ALT_SETTING: u8 = 1;
pub const AUDIO_EP_OUT: u8 = 0x01;
pub const AUDIO_EP_IN: u8 = 0x82;

/// DSP coefficient (EQ/FX) bulk endpoint — interface 1, ep 0x0A OUT
/// (mps 512), 64-byte blocks (PROTOCOL.md "Bulk OUT ep 0x0A"). The
/// EQ/FX parameter uploads are BULK OUT, not vendor control.
pub const DSP_INTERFACE: u8 = 1;
pub const DSP_EP_OUT: u8 = 0x0A;
/// `wMaxPacketSize` for [`AUDIO_EP_OUT`]/[`AUDIO_EP_IN`] at
/// [`AUDIO_ALT_SETTING`] is 448 bytes, but the audio URBs must be
/// **14336 bytes** (256 frames × 56 B, the Windows driver's URB size):
/// with 448-B URBs the DSP never starts producing audio and 48V never
/// engages (verified on hardware).

const CONTROL_TIMEOUT: Duration = Duration::from_millis(200);

/// Error type for device operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usb error: {0}")]
    Usb(#[from] rusb::Error),
    #[error("device not found (VID {VID:04x}:{PID:04x})")]
    NotFound,
    #[error("the kernel driver snd-usb-babyface-pro owns the device — unload it / unbind the interface before using the usbfs stack (the two would fight over the card)")]
    KernelDriverBound,
    #[error("audio transfer allocation failed")]
    StreamAlloc,
    #[error("audio transfer submission failed (libusb error {0})")]
    StreamSubmit(i32),
    #[error("eventfd creation failed")]
    EventFdFailed,
    #[error("unsupported sample rate {0} Hz (32/44.1/48/64/88.2/96/128/176.4/192 kHz only)")]
    UnsupportedRate(u32),
}

/// A connected Babyface Pro FS.
pub struct BabyfaceUsb {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    flag: FlagCounter,
    streams: Option<(IntrStream, IntrStream)>,
    /// Active sample rate in Hz (the audio alt-setting follows it).
    sample_rate: u32,
    /// Whether interface 1 (DSP bulk 0x0A) is claimed.
    dsp_claimed: bool,
}

impl BabyfaceUsb {
    /// True when the kernel driver owns the proprietary interface — the
    /// usbfs stack must NOT be used on top of it: libusb's
    /// USBDEVFS_DISCONNECT_CLAIM would unbind the driver and the card
    /// vanishes mid-session (seen live).  The bound interfaces appear
    /// in the driver's sysfs dir as "<bus>-<port>:<cfg>.<iface>"
    /// (e.g. 3-1:1.5) — a name with a ':' and a leading digit.
    #[cfg(target_os = "linux")]
    fn kernel_driver_bound() -> bool {
        std::fs::read_dir("/sys/bus/usb/drivers/snd-usb-babyface-pro")
            .map(|d| {
                d.filter_map(|e| e.ok()).any(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with(|c: char| c.is_ascii_digit()) && n.contains(':')
                })
            })
            .unwrap_or(false)
    }

    /// Open the first Babyface Pro FS found on the USB bus.
    pub fn open() -> Result<Self, Error> {
        #[cfg(target_os = "linux")]
        if Self::kernel_driver_bound() {
            return Err(Error::KernelDriverBound);
        }
        let handle = rusb::open_device_with_vid_pid(VID, PID).ok_or(Error::NotFound)?;
        if cfg!(target_os = "linux") {
            let _ = handle.set_auto_detach_kernel_driver(true);
        }
        Ok(Self {
            handle,
            flag: FlagCounter::default(),
            streams: None,
            sample_rate: 48_000,
            dsp_claimed: false,
        })
    }

    /// Send a single vendor-control request (OUT, device recipient).
    pub fn send(&mut self, req: &VendorRequest) -> Result<(), Error> {
        self.handle.write_control(
            0x40,
            req.b_request,
            req.w_value,
            req.w_index,
            &[],
            CONTROL_TIMEOUT,
        )?;
        Ok(())
    }

    /// Send a batch of requests (e.g. one TotalMix action).
    pub fn send_all(&mut self, reqs: &[VendorRequest]) -> Result<(), Error> {
        for r in reqs {
            self.send(r)?;
        }
        Ok(())
    }

    // ── Mixer control ──────────────────────────────────────────────

    /// Set the volume of a source into an output (raw 16-bit scale).
    pub fn set_volume(&mut self, out: Output, src: Source, volume: u16) -> Result<(), Error> {
        let reqs = protocol::set_crosspoint_volume(out, src, volume, &mut self.flag);
        self.send_all(&reqs)
    }

    /// Write the AN1/2 low-map mirror of a crosspoint.
    pub fn set_low_map_volume(&mut self, src: Source, volume: u16) -> Result<(), Error> {
        let reqs = protocol::set_low_map_volume(src, volume, &mut self.flag);
        self.send_all(&reqs)
    }

    /// Stereo balance (pan) on a crosspoint.
    pub fn set_balance(
        &mut self,
        out: Output,
        src: Source,
        balance: f32,
        fixed_volume: u16,
        fixed_is_left: bool,
    ) -> Result<(), Error> {
        let reqs = protocol::set_crosspoint_balance(
            out,
            src,
            balance,
            fixed_volume,
            fixed_is_left,
            &mut self.flag,
        );
        self.send_all(&reqs)
    }

    /// Set an output's master fader (raw 16-bit volume + 8-bit companion).
    pub fn set_output_master(
        &mut self,
        out: Output,
        volume_16: u16,
        volume_8: u8,
    ) -> Result<(), Error> {
        let reqs = protocol::set_output_master(out, volume_16, volume_8, &mut self.flag);
        self.send_all(&reqs)
    }

    /// Mute/unmute an output master (unmute restores the caller's
    /// current 16-bit/8-bit volume pair).
    pub fn set_output_master_mute(
        &mut self,
        out: Output,
        muted: bool,
        restore_16: u16,
        restore_8: u8,
    ) -> Result<(), Error> {
        let reqs = protocol::set_output_master_mute(out, muted, restore_16, restore_8);
        self.send_all(&reqs)
    }

    /// Write the full preamp state for one mic (48V, PAD, four gains).
    /// `mic` is 0-3 (AN1-AN4); see [`protocol::set_preamp`].
    pub fn set_preamp(
        &mut self,
        mic: usize,
        phantom: bool,
        pad: bool,
        gain: [u8; 4],
    ) -> Result<(), Error> {
        let reqs = protocol::set_preamp(mic, phantom, pad, gain);
        self.send_all(&reqs)
    }

    /// Upload one 64-byte DSP coefficient block (EQ/FX) on bulk OUT
    /// ep 0x0A (interface 1). Claims the interface on first use.
    /// Blocks are written per-channel (L then R) — see
    /// [`protocol::set_low_cut`] for the pairing.
    pub fn write_eq_block(&mut self, block: &[u8; protocol::EQ_BLOCK_LEN]) -> Result<(), Error> {
        if !self.dsp_claimed {
            self.handle.claim_interface(DSP_INTERFACE)?;
            self.dsp_claimed = true;
        }
        self.handle.write_bulk(DSP_EP_OUT, block, CONTROL_TIMEOUT)?;
        Ok(())
    }

    /// Set the EQ low cut (device DSP): `freq_hz = None` turns it off.
    /// `slope_db_per_oct` = 6/12/18/24. Writes the L+R block pair on
    /// bulk ep 0x0A. Band slots stay as passed (`bands`/`shared`) —
    /// pass the current band coefficients once the biquad is decoded
    /// so a low-cut change doesn't zero the bands.
    pub fn set_low_cut(
        &mut self,
        freq_hz: Option<f32>,
        slope_db_per_oct: u8,
        bands: &[[i32; 4]; 3],
        shared: i32,
    ) -> Result<(), Error> {
        let blocks = protocol::set_low_cut(freq_hz, slope_db_per_oct, bands, shared);
        self.write_eq_block(&blocks[0])?;
        self.write_eq_block(&blocks[1])
    }

    /// Set one EQ band (0 = Low, 1 = Mid, 2 = High) on the device DSP:
    /// RBJ bell/shelf → the 5 stored words (see
    /// [`protocol::eq_band_storage`]), written on bulk ep 0x0A as the
    /// L+R block pair. `current` = the other slots' stored words so a
    /// single-band change preserves them.
    pub fn set_eq_band(
        &mut self,
        slot: usize,
        eq_type: protocol::EqType,
        freq_hz: f32,
        q: f32,
        gain_db: f32,
        fs: f32,
        current: &[[i32; 4]; 3],
    ) -> Result<(), Error> {
        let blocks = protocol::set_eq_band(slot, eq_type, freq_hz, q, gain_db, fs, current);
        self.write_eq_block(&blocks[0])?;
        self.write_eq_block(&blocks[1])
    }

    /// Send the session-start sequence (`0x10 0x8000` + `0x1D` +
    /// `0x14 0xC000`) — see [`protocol::session_start`].
    pub fn session_start(&mut self) -> Result<(), Error> {
        let reqs = protocol::session_start();
        self.send_all(&reqs)
    }

    /// Send the session-stop write (`0x13 0xC000`) — see
    /// [`protocol::session_stop`].
    pub fn session_stop(&mut self) -> Result<(), Error> {
        self.send(&protocol::session_stop())
    }

    /// Send the superseded cold-start handshake (see
    /// [`protocol::cold_init`]) — kept for the `cold48v` example.
    pub fn cold_init(&mut self) -> Result<(), Error> {
        let reqs = protocol::cold_init();
        self.send_all(&reqs)
    }

    /// Send the superseded preamp-arm write (see
    /// [`protocol::preamp_arm`]) — kept for the `cold48v` example.
    pub fn preamp_arm(&mut self) -> Result<(), Error> {
        self.send(&protocol::preamp_arm())
    }

    /// Read a status register (vendor read, 4-byte reply).
    ///
    /// `0x17` is the one with a state payload: **byte 0 mirrors the
    /// preamp state register** (base 0x0C: bit0/1 = 48V AN1/2, bit4/5 =
    /// PAD AN1/2 — verified 2026-08-23: the readback tracks the
    /// front-panel P48 LEDs, e.g. 0x0C/0x0D/0x1D), **byte 2 = clock
    /// state** (0x40 = Internal, 0x80 = optical no-lock). The old
    /// "bit 1 = streaming active" note was wrong.
    pub fn read_status(&mut self, reg: u8) -> Result<[u8; 4], Error> {
        let mut buf = [0u8; 4];
        let n = self.handle.read_control(
            0xC0, // IN, vendor, device
            reg,
            0x0000,
            0x0000,
            &mut buf,
            CONTROL_TIMEOUT,
        )?;
        if n < 4 {
            buf[n..].fill(0);
        }
        Ok(buf)
    }

    // ── Audio streaming (interrupt transfers) ───────────────────────
    //
    // The device only physically engages 48V phantom power (and,
    // presumably, the rest of the analog front end) while a valid audio
    // session is streaming — confirmed on real hardware: writing the
    // preamp register alone leaves the status readback saying "on" but
    // the LED off. This opens interface 5's interrupt endpoints and
    // keeps them fed with silence/discarded audio to satisfy that
    // interlock: 14336-B URBs (the DSP only starts producing audio at
    // that size), several in flight per endpoint (see
    // [`IntrStream`]). Real audio I/O and VU metering (see PROTOCOL.md's
    // frame format) can build on top of it later.

    /// Claim the audio interface, run the captured driver init sequence
    /// and start the interrupt OUT/IN streams (silence out, discarded
    /// in) at the active [`sample_rate`][Self::sample_rate]. Call
    /// [`pump`][Self::pump] periodically afterwards to keep transfers
    /// moving, and [`stop_streaming`][Self::stop_streaming] when done.
    pub fn start_streaming(&mut self) -> Result<(), Error> {
        let ra = protocol::rate_to_alt(self.sample_rate).expect("default 48k is alt 1");
        let urb = ra.frame_bytes * 256;

        self.handle.claim_interface(AUDIO_INTERFACE)?;
        self.handle.set_alternate_setting(AUDIO_INTERFACE, ra.alt)?;

        // The init sequence the RME driver sends at session start
        // (cap_coldplug.pcap) — without it the firmware never validates
        // the stream (48V stays physically off).
        let init = protocol::streaming_init(false);
        self.send_all(&init)?;

        // The stream trigger (cap_audio.pcap frames 5807/5809): the
        // `0x10 0x0000 0x8000` + `0x1D` pair right before the URBs.
        let start = protocol::session_start();
        self.send(&start[0])?;
        self.send(&start[1])?;

        self.start_streams(urb, ra.frame_bytes, None, None)?;

        // The session-arm write `0x14 0x0000 0xC000` — in the capture it
        // follows the first URBs (frame 5829, 114 µs after URB #1).
        // NEVER send the `0x13 0xC000` stop write here: it disarms the
        // session (clock never activates, 48V never engages).
        self.send(&start[2])?;
        Ok(())
    }

    /// Like [`start_streaming`][Self::start_streaming] but wires real
    /// audio rings into the streams: `playback` feeds the OUT URBs
    /// (silence on underrun), `capture` collects the IN frames. The
    /// caller keeps both rings alive and pumps via
    /// [`pump`][Self::pump] (a dedicated thread for real-time use).
    pub fn start_streaming_audio(
        &mut self,
        playback: Arc<AudioRing>,
        capture: Arc<AudioRing>,
    ) -> Result<(), Error> {
        let ra = protocol::rate_to_alt(self.sample_rate).expect("default 48k is alt 1");
        let urb = ra.frame_bytes * 256;

        self.handle.claim_interface(AUDIO_INTERFACE)?;
        self.handle.set_alternate_setting(AUDIO_INTERFACE, ra.alt)?;

        let init = protocol::streaming_init(false);
        self.send_all(&init)?;

        let start = protocol::session_start();
        self.send(&start[0])?;
        self.send(&start[1])?;

        self.start_streams(urb, ra.frame_bytes, Some(playback), Some(capture))?;

        self.send(&start[2])?;
        Ok(())
    }

    /// Drop the current streams and recreate them sized for a URB of
    /// `urb` bytes (the rate's frame layout). Used by
    /// [`set_sample_rate`][Self::set_sample_rate] — a mid-session rate
    /// change is SET_INTERFACE only, no re-init (ratetest.c).
    /// `playback`/`capture` wire real audio rings into the streams
    /// (None = silence OUT / discard IN, the meter-only mode).
    fn start_streams(
        &mut self,
        urb: usize,
        frame_bytes: usize,
        playback: Option<Arc<AudioRing>>,
        capture: Option<Arc<AudioRing>>,
    ) -> Result<(), Error> {
        self.streams.take(); // cancels + frees the old URBs
        let ctx_raw = self.handle.context().as_raw();
        let dev_raw = self.handle.as_raw();
        // SAFETY: ctx_raw/dev_raw come from the open rusb handle this
        // `BabyfaceUsb` owns, which outlives the streams (stopped in
        // `stop_streaming` before `self.handle` can be dropped).
        let (out, in_) = unsafe {
            (
                IntrStream::start(ctx_raw, dev_raw, AUDIO_EP_OUT, urb, None, playback)?,
                IntrStream::start(
                    ctx_raw,
                    dev_raw,
                    AUDIO_EP_IN,
                    urb,
                    Some(Arc::new(Mutex::new(MeterAccum::new(frame_bytes)))),
                    capture,
                )?,
            )
        };
        self.streams = Some((out, in_));
        Ok(())
    }

    /// Change the sample rate mid-session: `SET_INTERFACE(5, alt)`
    /// only (validated 2026-08-22 on Linux, ratetest.c) + restart the
    /// URBs at the new frame layout. No-op if the rate is unchanged.
    /// Unsupported rates (e.g. 50 kHz) error out.
    pub fn set_sample_rate(&mut self, rate: u32) -> Result<(), Error> {
        self.set_sample_rate_impl(rate, None, None)
    }

    /// [`set_sample_rate`][Self::set_sample_rate] keeping the audio
    /// rings wired (the caller recreates them for the new frame layout
    /// and passes them here).
    pub fn set_sample_rate_audio(
        &mut self,
        rate: u32,
        playback: Arc<AudioRing>,
        capture: Arc<AudioRing>,
    ) -> Result<(), Error> {
        self.set_sample_rate_impl(rate, Some(playback), Some(capture))
    }

    fn set_sample_rate_impl(
        &mut self,
        rate: u32,
        playback: Option<Arc<AudioRing>>,
        capture: Option<Arc<AudioRing>>,
    ) -> Result<(), Error> {
        let ra = protocol::rate_to_alt(rate).ok_or(Error::UnsupportedRate(rate))?;
        if rate == self.sample_rate {
            return Ok(());
        }
        // Not streaming yet? Just record the target rate —
        // `start_streaming` will pick it up (the interface isn't
        // claimed, so SET_INTERFACE would fail).
        self.sample_rate = rate;
        if self.streams.is_some() {
            self.handle.set_alternate_setting(AUDIO_INTERFACE, ra.alt)?;
            self.start_streams(ra.frame_bytes * 256, ra.frame_bytes, playback, capture)?;
        }
        Ok(())
    }

    /// The active sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Set the sample clock source: `optical = true` syncs to the
    /// optical input, `false` = internal. The ~3 s keepalive doubles as
    /// this settings word (bit 2, PROTOCOL.md — verified cap_clk: the
    /// `0x17` readback byte 2 goes 0x40 → 0x80 no-lock).
    pub fn set_clock_optical(&mut self, optical: bool) -> Result<(), Error> {
        self.send(&protocol::settings_keepalive(protocol::settings_word(
            optical, false, false,
        )))
    }

    /// Pump the streams' libusb event loop so transfers keep moving.
    /// No-op if streaming isn't active.
    pub fn pump(&self, timeout: Duration) {
        if let Some((out, in_)) = &self.streams {
            out.pump(timeout);
            in_.pump(timeout);
        }
    }

    /// `(out_stats, in_stats)` with completed-ok/error counts — `None`
    /// if streaming isn't active.
    pub fn streaming_stats(&self) -> Option<(StreamStats, StreamStats)> {
        self.streams
            .as_ref()
            .map(|(out, in_)| (out.stats(), in_.stats()))
    }

    /// Per-channel input peak levels (ch0-3 = AN1-4, 0..1 of full
    /// scale) accumulated since the last call, resetting the
    /// accumulator. `None` if streaming isn't active.
    ///
    /// Poll this at ~50-200 Hz from the UI thread and shape the display
    /// (hold/decay/ballistics) presentation-side; the raw values are the
    /// max |sample| seen since the previous poll.
    pub fn input_peaks(&self) -> Option<[f32; 4]> {
        self.streams.as_ref().and_then(|(_, in_)| in_.drain_peaks())
    }

    /// Stop streaming and release the audio interface.
    pub fn stop_streaming(&mut self) -> Result<(), Error> {
        if self.streams.take().is_some() {
            self.handle.release_interface(AUDIO_INTERFACE)?;
        }
        Ok(())
    }
}

/// Convenience: open + session test (run with `--example probe`).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_the_captured_device() {
        assert_eq!(VID, 0x2A39);
        assert_eq!(PID, 0x3FC0);
    }
}
