//! route_pb — route PB1+PB2 (the ALSA plugin's playback channels) to
//! the Phones (out pair 1) so ALSA playback through the tuxmix PCM
//! plugin is audible. The mixer registers persist on the device after
//! exit (the plugin doesn't touch the mixer).
//!
//! ```bash
//! cargo run -p tuxmix-usb --features driver --example route_pb
//! ```

use tuxmix_usb::map::{Output, Playback, Source};
use tuxmix_usb::BabyfaceUsb;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = BabyfaceUsb::open()?;
    // The plugin writes the OUT frame ch0/1 (PB1) + ch2/3 (PB2).
    let xpt = 0x16A0; // 0 dB crosspoint (the calibrated fader curve)
    for pb in 1..=2 {
        dev.set_volume(Output::Ph34, Source::Playback(Playback(pb)), xpt)?;
    }
    // The Phones master at -20 dB (the exponential master curve), unmuted
    // — moderate level for headphones.
    dev.set_output_master(Output::Ph34, 0x032B, 0x00F3)?;
    println!("PB1+PB2 -> Phones @ 0 dB xpt, master -20 dB (persists).");
    Ok(())
}
