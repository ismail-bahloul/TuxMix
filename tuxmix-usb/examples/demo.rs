//! Demo: print the vendor requests that set up a basic Babyface mix.
//!
//! No hardware needed — this shows the exact USB control transfers that
//! a TuxMix backend would send, using the addresses and values decoded
//! from the RE captures. Run with:
//!
//! ```bash
//! cargo run -p tuxmix-usb --example demo
//! ```

use tuxmix_usb::map::{Input, Output, Playback, Source};
use tuxmix_usb::protocol::{self, FlagCounter, VendorRequest};

fn main() {
    let mut flag = FlagCounter::default();

    println!("== Basic mix setup for the Babyface Pro FS ==\n");

    println!("1) AN1 fader at 0x0317 (~= -40 dB) into AN1/2 + PH3/4:");
    for req in
        protocol::set_crosspoint_volume(Output::An12, Source::Input(Input::An1), 0x0317, &mut flag)
    {
        print_req(&req);
    }
    for req in
        protocol::set_crosspoint_volume(Output::Ph34, Source::Input(Input::An1), 0x0317, &mut flag)
    {
        print_req(&req);
    }

    println!("\n2) Playback 1 at 0x139E (~= -20 dB) into AN1/2 (both maps in sync):");
    for req in protocol::set_crosspoint_volume(
        Output::An12,
        Source::Playback(Playback(1)),
        0x139E,
        &mut flag,
    ) {
        print_req(&req);
    }
    for req in protocol::set_low_map_volume(Source::Playback(Playback(1)), 0x139E, &mut flag) {
        print_req(&req);
    }

    println!("\n3) AN1/2 output master at 0x2000 / 8-bit 0xF3:");
    for req in protocol::set_output_master(Output::An12, 0x2000, 0xF3, &mut flag) {
        print_req(&req);
    }

    println!("\n4) Mute AN1/2 output:");
    for req in protocol::set_output_master_mute(Output::An12, true, 0, 0) {
        print_req(&req);
    }

    println!("\n5) Solo PB1 (mute-the-others, cap_solo2.pcap): mute AS1/2 + PB1's low maps...");
    // NOTE: solo is NOT a flag write — it mutes every OTHER strip's
    // crosspoints (see PROTOCOL.md). The core driver implements it with
    // set_crosspoint_volume + set_low_map_volume; there is no dedicated
    // protocol helper.

    println!("\n6) Preamp: 48V ON + PAD ON, gains [10, 0, 0, 0]:");
    for req in protocol::set_preamp(0, true, true, [10, 0, 0, 0]) {
        print_req(&req);
    }

    println!("\n7) Session start (0x10 0x8000 + 0x1D + 0x14 0xC000):");
    for req in protocol::session_start() {
        print_req(&req);
    }
    println!("\n8) Session stop (0x13 0xC000):");
    print_req(&protocol::session_stop());
}

fn print_req(req: &VendorRequest) {
    println!(
        "  bReq=0x{:02X} wValue=0x{:04X} wIndex=0x{:04X}",
        req.b_request, req.w_value, req.w_index
    );
}
