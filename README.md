# TuxMix: a TotalMix-class mixer for RME interfaces

TuxMix is an open-source control application for RME audio interfaces,
starting with the **Babyface Pro FS**.  It brings the TotalMix
experience to Linux, without the RME Windows software: routing, gains,
monitoring, EQ, FX sends, scene management.

## What's inside

- **`tuxmix-usb`** is the low-level USB backend: the full vendor
  protocol of the Babyface Pro FS proprietary mode (crosspoints,
  masters, gains, preamp, EQ, pitch, loopback, width, FX, the
  front-panel emulation), reverse-engineered from Windows captures
  and validated on hardware.  It refuses to open the device while the
  kernel driver (`snd-usb-babyface-pro`) owns it.
- **`tuxmix-core`** is the device model + control API (`RmeDevice`
  trait) with three backends: USB (libusb), ALSA (the kernel driver's
  controls), and mock (tests/scenes).
- **`tuxmix-gui`** (iced) and **`tuxmix-tui`** (ratatui) are the
  interfaces, both built on the final protocol knowledge.
  **Functionally complete** — custom canvas faders, VU meters with
  real ballistics, multi-select, live routing matrix. What's left is
  visual polish, not re-architecture; see `GUI-NOTES.md` for the
  prioritized list.
- **`tuxmix-sys`** provides FFI bindings.

## The Linux audio stack

Two ways to get sound from the Babyface Pro FS:

1. **Kernel driver** (sibling repo `babyface-pro-linux`): the
   production path.  It exposes a real ALSA card (PipeWire
   sink/source, low latency) whose mixer is the ALSA control set, and
   TuxMix can drive it through the ALSA backend.
2. **This stack's USB backend** (libusb): the reference/development
   path for full control + streaming without the kernel driver.

`tools/alsa/` has the ALSA plugin and PipeWire configuration glue.

## Status

- The protocol layer (`tuxmix-usb` + `tuxmix-core`) is complete and
  hardware-validated (the laws are calibrated, not guessed).
- The GUI/TUI are functionally complete; remaining work is visual
  polish (see `GUI-NOTES.md`).
- The ALSA backend (`tuxmix-core`'s `alsa` feature, driving the kernel
  driver's controls) is hardware-validated as of 2026-08-28.
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
