//! panelprobe — dump the 0x17 readback live so we can see the REAL
//! states while pressing the front-panel buttons (diagnostic for the
//! MIX/IN/wheel mode switching).
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example panelprobe
//! ```
//!
//! For 20 s it prints every CHANGED 0x17 readback (byte0..3), plus a
//! decode: engaged / out_pos / dim / byte2 (mode nibble + counter) /
//! button flash. Drive the sequence: IN, wheel, MIX, wheel, SELECT.

use std::time::{Duration, Instant};

use tuxmix_usb::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;
    println!("opened the device. 20 s of 0x17 readbacks, printing changes.");
    println!("DO: press IN (select an input), turn the wheel, press MIX, turn the wheel.");
    let t0 = Instant::now();
    let mut last = [0u8; 4];
    let mut first = true;
    while t0.elapsed() < Duration::from_secs(20) {
        let st = dev.read_status(0x17)?;
        if first || st != last {
            let (b0, b1, b2, b3) = (st[0], st[1], st[2], st[3]);
            let engaged = b0 & 0x80 != 0;
            let out_pos = b1 & 0x07;
            let dim = b1 & 0x20 != 0;
            let mode = b2 >> 4;
            let cnt = b2 & 0x0F;
            let mode_s = match (mode, engaged) {
                (0x0, true) => "MIX(fader)",
                (0x8, _) => "OUT",
                (0x4, _) => "IN",
                (0x5, _) => "IN",
                (0x6, _) => "IN",
                (0x0, false) => "idle0",
                (m, _) => return Err(format!("unknown mode nibble {m:#x}").into()),
            };
            let btn = match b3 {
                0x40 => "idle",
                0x44 => "MIX",
                0x41 => "IN",
                0x48 => "OUT",
                0x42 => "SET",
                0x50 => "SELECT",
                0x60 => "DIM",
                x => return Err(format!("unknown flash {x:#x}").into()),
            };
            println!(
                "t={:6.2}s  {b0:02X} {b1:02X} {b2:02X} {b3:02X}  engaged={engaged} out={out_pos} dim={dim}  mode={mode_s} cnt={cnt}  flash={btn}",
                t0.elapsed().as_secs_f32()
            );
            last = st;
        }
        if first {
            first = false;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Ok(())
}
