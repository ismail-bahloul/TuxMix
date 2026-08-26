//! EQ upload demo: exercise the decoded EQ transport on real hardware.
//!
//! The band-EQ biquad and the low-cut formulas are now fully decoded
//! (2026-08-24, see `tools/usbdump/eq_biquad.md`) and implemented as
//! `protocol::eq_band_storage` / `set_eq_band` / `set_low_cut`, uploaded
//! as 64-byte BULK OUT blocks on ep 0x0A (interface 1).
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example eqtest
//! ```
//!
//! What to check by ear (with a mic on AN1 routed to the headphones):
//!
//! 1. Default (no EQ): the raw mic sound.
//! 2. `+6 dB Low bell @ 200 Hz` — the low end gets audibly louder.
//! 3. `-6 dB Low bell @ 200 Hz` — the low end gets quieter (notch).
//! 4. `+10 dB Low bell @ 4 kHz` — a nasal/telephone-ish emphasis.
//! 5. Low cut 100 Hz @ 12 dB/oct — rumble/boom below 100 Hz disappears.
//! 6. Low cut off — the low end returns.
//!
//! The VU meters (ch0-3 = AN1-4) confirm the level changes.

use std::time::Duration;

use tuxmix_usb::map::{Input, Output, Source};
use tuxmix_usb::protocol::EqType;
use tuxmix_usb::BabyfaceUsb;

fn pause(secs: u64, msg: &str) {
    println!("\n>>> {msg} ({secs}s) <<<");
    std::thread::sleep(Duration::from_secs(secs));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("starting stream (interface 5, ep 0x01/0x82, alt-setting 1)...");
    dev.start_streaming()?;

    // Routing recipe: 48V + gain raw 11 (≈ 35 dB) on Mic 1, AN1 ->
    // AN1/2 + PH3/4 crosspoints at 0 dB (fader raw 0x16A0), masters at
    // -30 dB (raw 0x0100 = 0x2000·2^(-5), the user's safety level).
    dev.set_preamp(0, true, false, [11, 0, 0, 0])?;
    dev.set_volume(Output::An12, Source::Input(Input::An1), 0x16A0)?;
    dev.set_output_master(Output::An12, 0x0100, 0x00F3)?;
    dev.set_volume(Output::Ph34, Source::Input(Input::An1), 0x16A0)?;
    dev.set_output_master(Output::Ph34, 0x0100, 0x00F3)?;

    // Let the DSP settle.
    for _ in 0..20 {
        dev.pump(Duration::from_millis(50));
    }

    // No band EQ, no low cut: the baseline 64-byte block (0x38 = OFF).
    let empty = [[0i32; 4]; 3];
    println!("EQ transport ready (bulk ep 0x0A, interface 1).");

    pause(3, "1. NO EQ — baseline mic sound (48V + 35 dB gain on AN1)");

    // +6 dB Low bell @ 200 Hz, Q = 0.7 (the exact cap_eq8c state).
    dev.set_eq_band(0, EqType::Bell, 200.0, 0.7, 6.0, 48_000.0, &empty)?;
    pause(4, "2. +6 dB Low BELL @ 200 Hz (Q 0.7) — low end louder");

    // -6 dB — the notch (mirror of +6).
    dev.set_eq_band(0, EqType::Bell, 200.0, 0.7, -6.0, 48_000.0, &empty)?;
    pause(4, "3. -6 dB Low BELL @ 200 Hz — low end quieter (notch)");

    // +10 dB @ 4 kHz — a clearly audible mid emphasis.
    dev.set_eq_band(0, EqType::Bell, 4000.0, 0.7, 10.0, 48_000.0, &empty)?;
    pause(4, "4. +10 dB Low BELL @ 4 kHz — nasal/telephone emphasis");

    // Reset the band (0 dB = identity), then add a low cut 100 Hz.
    dev.set_eq_band(0, EqType::Bell, 200.0, 0.7, 0.0, 48_000.0, &empty)?;
    dev.set_low_cut(Some(100.0), 12, &empty, 1 << 27)?;
    pause(4, "5. LOW CUT 100 Hz @ 12 dB/oct — boom below 100 Hz gone");

    // Low cut off: byte1 = 0x00, 0x38 = LOW_CUT_OFF.
    dev.set_low_cut(None, 12, &empty, 1 << 27)?;
    pause(3, "6. LOW CUT OFF — the low end returns");

    dev.session_stop()?;
    println!("\nstopped.");
    Ok(())
}
