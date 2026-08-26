# TuxMix — a TotalMix-class mixer for RME interfaces

TuxMix is an open-source control application for RME audio interfaces,
starting with the **Babyface Pro FS**.  It gives you the TotalMix
experience — routing, gains, monitoring, EQ, FX sends, scene
management — on Linux, without the RME Windows software.

## What's inside

- **`tuxmix-usb`** — the low-level USB backend: the full vendor
  protocol of the Babyface Pro FS proprietary mode (crosspoints,
  masters, gains, preamp, EQ, pitch, loopback, width, FX, the
  front-panel emulation), reverse-engineered from Windows captures
  and validated on hardware.  It refuses to open the device while the
  kernel driver (`snd-usb-babyface-pro`) owns it.
- **`tuxmix-core`** — the device model + control API (`RmeDevice`
  trait) with three backends: USB (libusb), ALSA (the kernel driver's
  controls), and mock (tests/scenes).
- **`tuxmix-gui` / `tuxmix-tui`** — the interfaces.  **Work in
  progress**: they are being rewritten on the final protocol
  knowledge (the first versions were built before the RE was
  complete and carried wrong scales/logic).
- **`tuxmix-sys`** — FFI bindings.

## The Linux audio stack

Two ways to get sound from the Babyface Pro FS:

1. **Kernel driver** (sibling repo `babyface-pro-linux`) — the
   production path: a real ALSA card (PipeWire sink/source, low
   latency), mixer = ALSA controls.  TuxMix can drive it through the
   ALSA backend.
2. **This stack's USB backend** (libusb) — the reference/development
   path: full control + streaming without the kernel driver.

`tools/alsa/` has the ALSA plugin and PipeWire configuration glue.

## Status

- The protocol layer (`tuxmix-usb` + `tuxmix-core`) is complete and
  hardware-validated (the laws are calibrated, not guessed).
- The GUI/TUI are being rewritten.
- See the sibling driver repo for the kernel-side status.

## Build

```sh
cargo build -p tuxmix-core --no-default-features --features usb
# or the whole workspace:
cargo build
cargo test          # unit tests (protocol laws, panel state machine, …)
```

## License

MIT (userspace application).
