// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Read-only probe of a Fireface UCX through `snd-fireface-ucx`'s hwdep
//! node (realbogart/fireface-ucx-linux): status, register table, meters.
//! It never writes to the unit. For owners who want to check TuxMix's
//! reading of their unit:
//!
//!     cargo run -p tuxmix-core --example ucx-probe [/dev/snd/hwCxDy]

use tuxmix_core::latter::meters::Bank;
use tuxmix_core::ucx_hwdep::UcxHwdep;

fn main() {
    let dev = match std::env::args().nth(1) {
        Some(path) => UcxHwdep::open(std::path::Path::new(&path)),
        None => UcxHwdep::find(),
    };
    let mut dev = match dev {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    match dev.status() {
        Ok(s) => println!(
            "status {:#010x}  firmware {:#010x}  clock {}  rate code {}",
            s.status,
            s.version,
            s.clock(),
            s.rate_code().map_or("none".into(), |r| r.to_string())
        ),
        Err(e) => println!("status: {e}"),
    }

    match dev.registers() {
        Ok(regs) => {
            let mix = regs.keys().filter(|r| **r & 0x4000 != 0).count();
            println!(
                "registers: {} known ({} mixer gains written since the driver loaded)",
                regs.len(),
                mix
            );
            // Per-channel parameters of the first input and the first
            // output, to compare with TotalMix on the same unit.
            for ch in [0u16, 18] {
                let row: Vec<String> = regs
                    .range(ch << 8..(ch << 8) + 0x40)
                    .map(|(r, v)| format!("{:02x}={v:#06x}", r & 0xff))
                    .collect();
                println!("  channel {ch:2}: {}", row.join(" "));
            }
        }
        Err(e) => println!("registers: {e}"),
    }

    match dev.meters() {
        Ok(banks) => {
            for b in banks {
                let name = match b.bank {
                    Bank::InputsPost => "inputs (post FX)",
                    Bank::InputsPre => "inputs (pre FX)",
                    Bank::Playback => "playback",
                    Bank::OutputsPre => "outputs (pre volume)",
                    Bank::OutputsPost => "outputs (post volume)",
                };
                let peaks: Vec<String> = b
                    .peak_db
                    .iter()
                    .map(|p| p.map_or("  -".into(), |db| format!("{db:5.1}")))
                    .collect();
                println!("{name:22} {}", peaks.join(" "));
            }
        }
        Err(e) => println!("meters: {e}"),
    }
}
