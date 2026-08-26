//! Diagnostic: turn 48V on, start a real audio stream — INTERRUPT
//! transfers on interface 5 (ep `0x01` OUT / `0x82` IN, alt-setting 1,
//! 448-B packets; bmAttributes 0x03 = interrupt, NOT isochronous) — and
//! hold it for 15s while polling status — to check whether phantom power
//! physically engages once the stream is running.
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example hold48v_iso
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("claiming audio interface, starting ISO streams...");
    dev.start_streaming()?;
    println!("streaming started (ep 0x01 OUT / 0x82 IN, alt-setting 1)");

    println!("letting the stream settle for 2s before touching 48V...");
    for _ in 0..40 {
        dev.pump(Duration::from_millis(50));
    }

    println!("sending 48V ON...");
    dev.set_preamp(0, true, false, [0, 0, 0, 0])?;

    for i in 0..15 {
        // Keep the interrupt transfers cycling; TotalMix only runs the
        // status polls while streaming (no session writes — sending the
        // `0x13` stop would disarm the session).
        for _ in 0..20 {
            dev.pump(Duration::from_millis(50));
        }
        let s = dev.read_status(0x17)?;
        let v48 = if s[0] & 1 != 0 { "on" } else { "off" };
        let streaming = if s[0] & 2 != 0 { "active" } else { "inactive" };
        let (out_stats, in_stats) = dev.streaming_stats().unwrap();
        println!(
            "t={i:02}s  reg 0x17 = {s:02X?}  48V={v48} streaming={streaming}  out={out_stats:?} in={in_stats:?}"
        );
    }

    println!("stopping stream...");
    dev.stop_streaming()?;
    println!("done, exiting...");
    Ok(())
}
