//! Probe: open the real Babyface Pro FS and drive it.
//!
//! Build with the driver feature:
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example probe -- <48v|gain> [value]
//! ```
//!
//! Subcommands:
//! - `48v on|off`  — toggle 48V phantom on Mic 1 (AN1)
//! - `gain <db>`    — set the Mic 1 gain in dB (placeholder dB/2 scale)
//! - `master <hex>` — write the AN1/2 output master (16-bit)
//! - `status`       — read the status registers (0x11/0x17/0x1C/0x1E/0x1F)
//!
//! Watch TotalMix to see the effect (the device is currently bound to
//! the RME Windows driver — whether libusb can open it is the point of
//! this probe).

use std::time::Duration;

use tuxmix_usb::device::BabyfaceUsb;
use tuxmix_usb::map::{Output, Source};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: probe <48v on|off> | <gain N> | <master N>");
        std::process::exit(2);
    }

    let mut dev = BabyfaceUsb::open()?;
    println!("device opened ✓");

    match args[1].as_str() {
        "48v" => {
            let on = args.get(2).map(|s| s == "on").unwrap_or(true);
            let mic = args
                .get(3)
                .map(|s| s.parse::<usize>().unwrap_or(1).saturating_sub(1))
                .unwrap_or(0);
            // 48V write + commit (the full preamp block pattern); the
            // state byte bit = 1 << mic (AN1 = bit0, AN2 = bit1).
            let reqs = tuxmix_usb::protocol::set_preamp(mic, on, false, [0, 0, 0, 0]);
            dev.send_all(&reqs)?;
            println!("48V Mic{} -> {}", mic + 1, if on { "ON" } else { "OFF" });
        }
        "pad" => {
            let on = args.get(2).map(|s| s == "on").unwrap_or(true);
            let mic = args
                .get(3)
                .map(|s| s.parse::<usize>().unwrap_or(1).saturating_sub(1))
                .unwrap_or(0);
            // Preserve the current 48V state (the preamp byte is a FULL
            // state — a bare PAD write would clear 48V).
            let st = dev.read_status(0x17)?;
            let phantom = st[0] & (1 << mic) != 0;
            let reqs = tuxmix_usb::protocol::set_preamp(mic, phantom, on, [0, 0, 0, 0]);
            dev.send_all(&reqs)?;
            println!("PAD Mic{} -> {}", mic + 1, if on { "ON" } else { "OFF" });
        }
        "gain" => {
            // Convert dB -> raw with the calibrated linear fit
            // (CALIBRATION.md: dB = raw * 3.25, raw 0-20 = 0-65 dB).
            let db = args
                .get(2)
                .map(|s| s.parse::<f32>().unwrap_or(0.0))
                .unwrap_or(0.0);
            let raw = (db / 3.25).round().clamp(0.0, 20.0) as u8;
            let mut cycle = 0u8;
            let reqs = tuxmix_usb::protocol::set_gain(0, raw, &mut cycle);
            dev.send_all(&reqs)?;
            println!("gain Mic1 <- {db:.0} dB (raw {raw})");
        }
        "master" => {
            let v = args
                .get(2)
                .map(|s| u16::from_str_radix(s, 16).unwrap_or(0x2000))
                .unwrap_or(0x2000);
            dev.set_output_master(Output::An12, v, 0xF3)?;
            println!("master AN1/2 -> 0x{:04X}", v);
        }
        "vol" => {
            // AN1 into AN1/2 at the given 16-bit volume.
            let v = args
                .get(2)
                .map(|s| u16::from_str_radix(s, 16).unwrap_or(0x0317))
                .unwrap_or(0x0317);
            dev.set_volume(Output::An12, Source::Input(tuxmix_usb::map::Input::An1), v)?;
            println!("AN1 -> AN1/2 vol 0x{:04X}", v);
        }
        "status" => {
            for reg in [0x11u8, 0x17, 0x1C, 0x1E, 0x1F] {
                let r = dev.read_status(reg)?;
                println!(
                    "reg 0x{:02X} = {:02X} {:02X} {:02X} {:02X}{}",
                    reg,
                    r[0],
                    r[1],
                    r[2],
                    r[3],
                    if reg == 0x17 {
                        // byte 0: bit0 = Mic1 (AN1) 48V, bit1 = Mic2
                        // (AN2) 48V, bit4 = PAD — verified on hardware.
                        let mic1 = if r[0] & 0x01 != 0 { "on" } else { "off" };
                        let mic2 = if r[0] & 0x02 != 0 { "on" } else { "off" };
                        let pad1 = if r[0] & 0x10 != 0 { "on" } else { "off" };
                        let pad2 = if r[0] & 0x20 != 0 { "on" } else { "off" };
                        format!(
                            "  (48V M1={} M2={}, pad M1={} M2={})",
                            mic1, mic2, pad1, pad2
                        )
                    } else {
                        String::new()
                    }
                );
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            std::process::exit(2);
        }
    }

    // Arm the session (start sequence without the stop write).
    dev.session_start()?;
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}
