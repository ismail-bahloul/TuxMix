//! Diagnostic: reproduce the cold-start handshake seen in
//! `cap_coldstart.pcap` (30x `0x15` + `0x17`/`0x8080`, then the first
//! preamp write followed by `0x17`/`0xF040`) and check whether that
//! alone — no ISO streaming — makes 48V physically engage on AN1.
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example cold48v
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;

    println!("cold-start handshake (30x 0x15 + 0x17/0x8080)...");
    dev.cold_init()?;

    println!("sending 48V ON...");
    dev.set_preamp(0, true, false, [0, 0, 0, 0])?;

    println!("preamp arm (0x17/0xF040)...");
    dev.preamp_arm()?;

    for i in 0..15 {
        std::thread::sleep(Duration::from_secs(1));
        let s = dev.read_status(0x17)?;
        let v48 = if s[0] & 1 == 0 { "on" } else { "off" };
        println!("t={i:02}s  reg 0x17 = {s:02X?}  48V={v48}");
    }

    println!("done, exiting...");
    Ok(())
}
