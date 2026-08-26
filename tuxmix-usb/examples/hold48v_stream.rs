//! Control-transfer-only 48V test (no streaming): arm the session and
//! watch the 0x17 readback — without the audio URBs the session never
//! activates and 48V cannot physically engage (kept as a control test).
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example hold48v_stream
//! ```

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;
    println!("device opened, sending 48V ON...");
    dev.set_preamp(0, true, false, [0, 0, 0, 0])?;

    println!("arming session (no stream)...");
    dev.session_start()?;

    for i in 0..15 {
        std::thread::sleep(Duration::from_secs(1));
        let s = dev.read_status(0x17)?;
        let v48 = if s[0] & 1 == 0 { "on" } else { "off" };
        let streaming = if s[0] & 2 != 0 { "active" } else { "inactive" };
        println!("t={i:02}s  reg 0x17 = {s:02X?}  48V={v48} streaming={streaming}");
    }
    println!("done, exiting...");
    Ok(())
}
