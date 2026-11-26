// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Ismaïl Bahloul

//! Writes the host FX response and filter-chain config for the default
//! reverb and echo, to try them by hand:
//!
//! ```text
//! cargo run -p tuxmix-core --example host-fx-files -- DIR CAPTURE_NODE PLAYBACK_NODE
//! pipewire -c DIR/tuxmix-fx.conf
//! ```

use tuxmix_core::host_fx::{filter_chain_conf, responses, wav, HostFx};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (dir, capture, playback) = match a.as_slice() {
        [_, d, c, p] => (d, c, p),
        _ => {
            eprintln!("usage: host-fx-files DIR CAPTURE_NODE PLAYBACK_NODE");
            std::process::exit(2);
        }
    };
    let mut fx = HostFx::default();
    fx.reverb.enabled = true;
    fx.echo.enabled = true;
    let ir = format!("{dir}/tuxmix-fx.wav");
    std::fs::write(&ir, wav(&responses(&fx))).expect("write the response");
    std::fs::write(
        format!("{dir}/tuxmix-fx.conf"),
        filter_chain_conf(&ir, capture, playback),
    )
    .expect("write the config");
    println!("{ir}: {} frames", responses(&fx).len());
}
