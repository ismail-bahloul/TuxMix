//! Input VU-meter demo (Rust port of `tools/usbdump/vudemo.c`): open the
//! device, start the interrupt stream, route AN1 → AN1/2, enable 48V and
//! gain on Mic 1, then compute per-channel levels from the IN stream
//! (ch0 = AN1, ch1 = AN2, ch2 = AN3, ch3 = AN4) and draw terminal bars
//! with peak hold + decay.
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example vu
//! ```
//!
//! Speak into the mic on AN1 while it runs (12 s). The `=#` bar spans
//! 0 dB (left) to -50 dB (right); `#` is the decaying peak hold.

use std::time::{Duration, Instant};

use tuxmix_usb::map::{Input, Output, Source};
use tuxmix_usb::BabyfaceUsb;

const NCH: usize = 4; // AN1-4
const SEG: usize = 50; // bar segments (0 dB .. -50 dB)
/// Peak-hold decay per 200 ms tick (vudemo.c's 0.86).
const HOLD_DECAY: f32 = 0.86;

fn draw(tag: &str, db: f32, db_hold: f32) {
    let d = db.clamp(-50.0, 0.0);
    let dh = db_hold.clamp(-50.0, 0.0);
    let n = ((d + 50.0) * SEG as f32 / 50.0) as usize;
    let h = ((dh + 50.0) * SEG as f32 / 50.0) as usize;
    print!("{tag:<4} |");
    for i in 0..SEG {
        if i == h {
            print!("#");
        } else if i < n {
            print!("=");
        } else {
            print!(" ");
        }
    }
    println!("| {db:6.1} dB  (peak {db_hold:6.1})");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("starting stream (interface 5, ep 0x01/0x82, alt-setting 1)...");
    dev.start_streaming()?;

    // Same routing recipe as vudemo.c: 48V + gain 17 (35 dB) on Mic 1,
    // AN1 -> AN1/2 crosspoints at 0x4000, master unmuted at 0x4000.
    dev.set_preamp(0, true, false, [17, 0, 0, 0])?;
    dev.set_volume(Output::An12, Source::Input(Input::An1), 0x4000)?;
    dev.set_output_master(Output::An12, 0x4000, 0x00F3)?;

    // Let the DSP settle so the first draw shows real levels.
    for _ in 0..20 {
        dev.pump(Duration::from_millis(50));
    }

    let names = ["AN1", "AN2", "AN3", "AN4"];
    let mut peak = [0f32; NCH]; // live window (reset every 200 ms)
    let mut hold = [0f32; NCH]; // decaying hold
    let t0 = Instant::now();
    let mut last_hold = t0;
    let mut last_draw = t0;

    println!(">>> INPUT VU METERS — SPEAK INTO THE MIC (AN1), 12s <<<");
    while t0.elapsed() < Duration::from_secs(12) {
        dev.pump(Duration::from_millis(20));

        // Fold the stream's per-channel peaks into the live window.
        if let Some(live) = dev.input_peaks() {
            for c in 0..NCH {
                if live[c] > peak[c] {
                    peak[c] = live[c];
                }
            }
        }

        // Every 200 ms: promote the window to the hold, decay the hold.
        if last_hold.elapsed() >= Duration::from_millis(200) {
            for c in 0..NCH {
                if peak[c] > hold[c] {
                    hold[c] = peak[c];
                }
                hold[c] *= HOLD_DECAY;
                peak[c] = 0.0;
            }
            last_hold = Instant::now();
        }

        // Every 500 ms: draw.
        if last_draw.elapsed() >= Duration::from_millis(500) {
            last_draw = Instant::now();
            println!("\n--- t={:.1}s ---", t0.elapsed().as_secs_f32());
            for c in 0..NCH {
                let db = if peak[c] > 0.0 {
                    20.0 * peak[c].log10()
                } else {
                    -120.0
                };
                let dbh = if hold[c] > 0.0 {
                    20.0 * hold[c].log10()
                } else {
                    -120.0
                };
                draw(names[c], db.max(-60.0), dbh.max(-60.0));
            }
        }
    }

    dev.session_stop()?;
    println!("stopped.");
    Ok(())
}
