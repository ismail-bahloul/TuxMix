//! Diagnostic: the real audio stream is INTERRUPT transfers on interface
//! 5, ep `0x01` OUT / `0x82` IN (bmAttributes 0x03 = interrupt; a
//! tshark analysis of cap_audio.pcap shows 14336-byte URBs every
//! ~5.33 ms, and of cap_coldplug.pcap the device's own descriptor), NOT
//! isochronous. This starts the session (init burst + trigger) and the
//! paired OUT/IN interrupt URBs, then reports transfer stats — the
//! device only services the endpoints while both have a pending URB.
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example interrupt_stream_48v
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("starting session + interrupt streams (ep 0x01/0x82)...");
    dev.start_streaming()?;

    for i in 0..10 {
        for _ in 0..20 {
            dev.pump(Duration::from_millis(50));
        }
        let (out, in_) = dev.streaming_stats().unwrap();
        println!("t={i:02}s  out={out:?} in={in_:?}");
    }

    dev.stop_streaming()?;
    println!("done, exiting...");
    Ok(())
}
