// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 Ismaïl Bahloul

//! Print the loudest hardware peak per slot, four times a second.

use std::time::Duration;
use tuxmix_usb::HwMeters;

fn main() {
    let Some(m) = HwMeters::start() else {
        eprintln!("cannot open the unit's meter interface");
        return;
    };
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        let raw = m.drain();
        let db = |v: u32| {
            if v == 0 {
                "  -inf".to_string()
            } else {
                format!("{:6.1}", 20.0 * (v as f32 / (1u32 << 27) as f32).log10())
            }
        };
        println!(
            "live={} in1-4: {} {} {} {}  pb1/2: {} {}  out1/2: {} {}",
            m.is_live(),
            db(raw[0]), db(raw[1]), db(raw[2]), db(raw[3]),
            db(raw[12]), db(raw[13]), db(raw[26]), db(raw[27]),
        );
    }
}
