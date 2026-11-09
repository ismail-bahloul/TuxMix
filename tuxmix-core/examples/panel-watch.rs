// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Follow the kernel driver's state the way the GUI does.
//!
//! Opens the Babyface Pro through the ALSA backend, calls `poll_events`
//! every 50 ms (the GUI's tick) and prints every change it sees in the
//! model: output masters and mutes, preamp gains, 48V, the inputs'
//! crosspoints into AN1/2 and PH3/4, DIM and the front-panel selection.
//! Turn the wheel, press DIM or change a control from `alsamixer`, and
//! each change should show up here within a tick.
//!
//! Usage:
//!     cargo run --example panel-watch            # runs for 60 s
//!     cargo run --example panel-watch -- 20      # runs for 20 s

use std::time::{Duration, Instant};

use tuxmix_core::{BabyfacePro, RmeDevice};

fn snapshot(d: &BabyfacePro) -> Vec<String> {
    let mut v = Vec::new();
    for (i, o) in d.outputs().iter().enumerate().step_by(2) {
        v.push(format!("out{} vol={:.3} mute={}", i / 2, o.volume, o.mute));
    }
    for (i, c) in d.inputs().iter().take(4).enumerate() {
        v.push(format!(
            "in{} gain={:?} 48V={} ->AN1/2={:.3} ->PH3/4={:.3}",
            i, c.gain, c.phantom, c.volumes[0], c.volumes[1]
        ));
    }
    v.push(format!("dim={}", d.settings().dim));
    v.push(format!("panel={:?}", d.panel_selection()));
    v
}

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let mut dev = BabyfacePro::open().expect("open the Babyface Pro");
    let mut last = snapshot(&dev);
    for line in &last {
        println!("  {line}");
    }
    println!("watching for {secs} s...");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(50));
        if let Err(e) = dev.poll_events() {
            eprintln!("poll_events: {e}");
        }
        let now = snapshot(&dev);
        for (a, b) in last.iter().zip(&now) {
            if a != b {
                println!("{:7.2}s  {b}", start.elapsed().as_secs_f32());
            }
        }
        last = now;
    }
}
