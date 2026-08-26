//! Probe the interrupt stream pair: start the session and check that
//! the OUT/IN URBs complete (the device requires both endpoints to have
//! a pending URB simultaneously — a lone URB never completes).
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example probe_intr
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("starting session + interrupt streams...");
    dev.start_streaming()?;
    for _ in 0..10 {
        dev.pump(Duration::from_millis(100));
    }
    let (out, in_) = dev.streaming_stats().unwrap();
    println!("after 1s: out={out:?} in={in_:?}");
    dev.stop_streaming()?;

    println!("done, exiting...");
    Ok(())
}
