<div align="center">

<img src="assets/tuxmix.svg" width="112" alt="TuxMix icon">

# TuxMix

**The TotalMix experience for RME interfaces, native on Linux.**

[![AUR](https://img.shields.io/aur/version/tuxmix?style=flat-square&logo=archlinux&logoColor=white&label=AUR&color=1793d1)](https://aur.archlinux.org/packages/tuxmix) ![Rust](https://img.shields.io/badge/Rust-iced-b7410e?style=flat-square&logo=rust&logoColor=white) ![Platform](https://img.shields.io/badge/Linux-ALSA%20%C2%B7%20PipeWire-333?style=flat-square&logo=linux&logoColor=white) [![License](https://img.shields.io/badge/license-MIT%20%2F%20GPL--3.0-555?style=flat-square)](docs/guide.md#license)

<br>

<img src="docs/screenshots/mixer.png" width="760" alt="TuxMix Mixer view: hardware inputs, software playback and hardware outputs">

</div>

<br>

Routing, gains, monitoring, EQ, FX sends and scenes for RME interfaces,
without the Windows software. Reverse-engineered from the vendor protocol and
validated on real hardware.

> **Supported today:** the Babyface Pro FS. More RME models are the goal,
> see [Contributing](CONTRIBUTING.md) to help bring yours.

- **Three views**: Quick, Mixer and Matrix
- **The whole DSP mixer**: preamp, trim, EQ, width, loopback, mute/solo groups
- **Snapshots and layouts**, undo/redo, custom channel names
- **Lives in the tray**: the front-panel DIM button and your mix survive a replug
- **GUI and TUI**, over the [kernel driver](https://github.com/ismail-bahloul/babyface-pro-linux) or plain libusb

## Install

```sh
paru -S tuxmix        # Arch, CachyOS
```

Other distributions: `cargo build --release`, see the
[guide](docs/guide.md#building-from-source).
Try it without hardware: `tuxmix --mock`.

## Learn more

<div align="center">

[Guide](docs/guide.md) · [Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md) · [Kernel driver](https://github.com/ismail-bahloul/babyface-pro-linux)

</div>
