//! Diagnostic: turn 48V on, then keep the device open and poll status for
//! 15s, to check whether phantom power holds while the handle stays open
//! vs reverting once the process (and its USB handle) closes.
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example hold48v
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;
    println!("device opened, sending 48V ON...");
    dev.set_preamp(0, true, false, [0, 0, 0, 0])?;

    for i in 0..15 {
        std::thread::sleep(Duration::from_secs(1));
        let s = dev.read_status(0x17)?;
        println!(
            "t={i:02}s  reg 0x17 = {s:02X?}  (bit0: {} = {})",
            s[0] & 1,
            if s[0] & 1 == 0 { "48V on" } else { "48V off" }
        );
    }

    println!("holding done, process about to exit (handle will close)...");
    Ok(())
}
