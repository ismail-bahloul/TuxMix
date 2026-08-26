# tuxmix-usb

Proprietary USB protocol backend for the **RME Babyface Pro FS**
(VID `2A39`, PID `3FC0`) — the TotalMix-class control protocol,
reverse-engineered from USB captures of TotalMix FX (see
[`tools/usbdump/PROTOCOL.md`](../tools/usbdump/PROTOCOL.md)).

## What it provides

- `map` — the register address map: crosspoints (`0x0034 + 0x0034·out +
  idx`), the AN1/2 low-map mirror, output master faders
  (`0x03E0 + 2·out` / `0x0004 + 2·out`), preamp gains, source and output
  indices. Unit-tested against the captured addresses.
- `protocol` — vendor-request encoding (`bReq 0x12`/`0x1A`/`0x17`/`0x21`
  families, transaction counters, keepalive). Unit-tested against the
  captured write sequences.
- `driver` (feature) — libusb device driver (planned; opens the device,
  sends the requests, reads the isochronous audio for VU metering).

## Building / testing

The protocol layer is **dependency-free** and builds anywhere:

```bash
cargo test -p tuxmix-usb
```

On Windows without the MSVC build tools, use the GNU toolchain:

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup component add rust-mingw --toolchain stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu test -p tuxmix-usb
```

## Status

The mixer control surface (faders, pan, mute, solo, output masters, 48V,
PAD, gain) is fully mapped and implemented. Remaining:

- dB calibration of the fader scale (partial: -40 dB = 0x0317).
- EQ/FX coefficient format (transport found: bulk ep 0x0A 64-byte
  blocks; the coefficient encoding is not yet decoded — see
  PROTOCOL.md).
- The libusb driver (feature `driver`).
