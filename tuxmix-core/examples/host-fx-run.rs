// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Runs the host FX on the card for a while, to hear or measure it:
//!
//! ```text
//! cargo run -p tuxmix-core --example host-fx-run -- reverb|echo|both SECONDS
//! ```
//!
//! Turns the driver's Host FX Switch on, runs the filter with the default
//! settings of the chosen effect, then stops both.

use tuxmix_core::host_fx::{HostFx, Runner};
use tuxmix_core::{BabyfacePro, RmeDevice};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let which = a.get(1).map(String::as_str).unwrap_or("both");
    let secs: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut fx = HostFx::default();
    fx.reverb.enabled = which != "echo";
    fx.echo.enabled = which != "reverb";

    let mut dev = <BabyfacePro as RmeDevice>::open().expect("open the card");
    if !dev.has_host_fx() {
        eprintln!("the driver has no Host FX Switch (needs 0.2.5)");
        std::process::exit(1);
    }
    dev.set_host_fx(true).expect("Host FX Switch");
    let mut runner = Runner::new();
    match runner.apply(&fx, None) {
        Ok(()) => println!("running {which} for {secs} s"),
        Err(e) => {
            let _ = dev.set_host_fx(false);
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(secs));
    println!("filter alive at the end: {}", runner.alive());
    runner.stop();
    dev.set_host_fx(false).expect("Host FX Switch");
}
