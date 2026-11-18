# Contributing to TuxMix

TuxMix supports the Babyface Pro FS today, validated on real hardware. The
most useful thing outside contributors can do is help bring a second RME
device: it needs someone who owns one, because nothing here can be verified
without the hardware. If you're not sure where something fits, open an issue
first rather than guessing.

The first recipe below covers devices the kernel exposes through ALSA
mixer controls, the way the Babyface Pro FS driver does. A unit whose mixer
is not exposed that way (the Fireface UC and UCX, for example) needs a
backend of its own: see [Adding a unit with no ALSA mixer
controls](#adding-a-unit-with-no-alsa-mixer-controls).

## Development setup

```bash
git clone https://github.com/ismail-bahloul/TuxMix
cd TuxMix
cargo build --release --workspace
cargo test --workspace

# Run the GUI/TUI against a simulated device — no hardware required:
cargo run -p tuxmix-gui -- --mock
cargo run -p tuxmix-tui -- --mock

# The same simulation with another unit's channels and capabilities
# (ucx, ucx2, ff802, ufx, ufx2, ufx3, ufxplus: see profiles::ALL):
TUXMIX_MOCK_MODEL=ucx2 cargo run -p tuxmix-gui -- --mock
```

Dependencies: `rustc`, `cargo`, `alsa-lib` (dev headers).

## Adding a new ALSA-selem device profile

This is the highest-value contribution right now if you own an RME
interface other than the Babyface Pro FS. It only covers **ALSA
simple-mixer-element (selem) devices** — USB class-compliant interfaces
like the Babyface family. The Fireface UCX, 802, UFX family and UCX II
speak RME's "latter" protocol in vendor mode and need another backend:
see [Adding a unit with no ALSA mixer controls](#adding-a-unit-with-no-alsa-mixer-controls).

1. **Get real `amixer scontents` output from your device.** Plug it in
   and run `amixer -c <card name> scontents > scontents.txt`. You'll need
   this to confirm every step below — don't guess at control names.

2. **Copy `tuxmix-core/src/profiles/babyface_pro.rs`** to a new file
   (e.g. `profiles/digiface.rs`) and adjust its `const PROFILE:
   DeviceProfile` to your device's real channel layout: `card_substring`
   (a substring of your card's name as it appears in
   `/proc/asound/cards`), `model_name`, one `InputSpec` per physical
   input in channel-id order with its `InputCaps` (preamp gain range,
   48V, pad, Hi-Z, AutoSet, reference levels, EQ), one `OutputPairSpec`
   per stereo output pair with the bus `label` the UIs show ("AN1/2"),
   and `main_pair`, the pair TotalMix shows as Main in its Control Room
   (`None` if you don't know: the Control Room is then hidden). Fill
   `usb` (the IDs `lsusb` shows in each mode), `protocol`, `features`,
   `support` (`SimulationOnly` until a backend drives the unit) and
   `source` (where the data comes from). The UIs read what a strip can do
   from `InputCaps`, never from the model name. Register the new module
   in `profiles/mod.rs` and add it to `profiles::ALL`.

3. **Re-verify the ALSA control-naming grammar.** `babyface.rs`'s
   `selem_name`/`ch_type_str` helpers encode a specific naming pattern
   (`"<Type>-<Name>-<Output>"`, `" 48V"`/`" PAD"`/`" Sens."` suffixes,
   `"Clock Mode"`) observed on the Babyface Pro FS specifically — **this
   is not confirmed to be shared by any other RME model.** Compare it
   against your `scontents.txt` line by line. If your device's controls
   are named differently, you'll need to adjust those helpers (or, if
   you're the second real device to need different naming, that's the
   signal to promote naming into `DeviceProfile` itself as a proper
   per-model strategy — see the comment on `DeviceProfile` in
   `profile.rs` for why that wasn't built speculatively ahead of time).

4. **Wire it up for testing.** Right now `tuxmix-gui`'s `DeviceHandle`
   enum (`app.rs`) only opens one real device type — `BabyfacePro`,
   pointed at the Babyface Pro FS profile. There's no auto-detection
   across multiple profiles yet (nothing to detect between, with only one
   real device implemented). To test your new profile against real
   hardware end-to-end today, temporarily point `BabyfacePro::open()` at
   your profile instead of `profiles::babyface_pro::PROFILE`, or add a
   `DeviceHandle` variant and extend `open_real()` to try each profile's
   `card_substring` in turn. A real multi-device-detection pass is
   planned but not built — happy to help design it once a second device
   is actually working.

5. **Look at it in the simulation.** Run the GUI with
   `TUXMIX_MOCK_MODEL=<your profile id> ... --mock`. Anything that still
   assumes the Babyface Pro's layout shows up there before you need the
   hardware.

6. **Add tests.** The suites in `profiles/mod.rs` and `mock.rs` already
   run over every profile in `profiles::ALL` (channel counts, unique pair
   labels, switches that answer only where `InputCaps` declares them);
   add what is specific to your unit.

7. **Open a PR** with your profile, the adjusted naming helpers if
   needed, and a note on which real device you tested against.

## Adding a unit with no ALSA mixer controls

The Fireface UC and UCX run in RME's vendor mode with out-of-tree drivers
that keep the mixer matrix out of ALSA: the audio is an ALSA card (some
also expose a few levels and the clock as controls), but routing, preamps
and EQ go through a hwdep node or straight over USB. TuxMix needs a backend
for that family, the way `babyface.rs` (ALSA controls) and `usb.rs` (the
Babyface Pro over libusb) are backends for the Babyface Pro. Such a backend
implements `RmeDevice` (`device.rs`) and is added to `DeviceHandle` in
`tuxmix-gui` and `tuxmix-tui`.

The UI side is ready for another layout: give the unit a `DeviceProfile`
(step 2 above) and check it with `TUXMIX_MOCK_MODEL`. The Fireface UCX,
UCX II, 802, UFX, UFX II, UFX III and UFX+ already have one
(`profiles/fireface_*.rs`), generated from oscmix's channel tables by
`tools/profiles/gen_oscmix_profiles.py`; none has been checked on a unit.

For the latter units, `tuxmix-core/src/latter.rs` holds the
transport-independent half of a backend: the 32-bit word with its parity,
the mixer coefficient, the meter packet, and the rules a writer must
follow (never the flash registers or requests, paced writes). It is tested
against the values other projects measured on a UCX, and sends nothing.

`tuxmix-core/src/ucx_hwdep.rs` is the transport for a Fireface UCX run by
[snd-fireface-ucx](https://github.com/realbogart/fireface-ucx-linux): that
driver keeps the register table and paces the writes, and exposes them on
its hwdep node. If you own a UCX with that driver, this reads your unit
without writing anything, and its output is the most useful thing to
attach to an issue:

```sh
cargo run -p tuxmix-core --example ucx-probe
```

What only an owner can provide, in an issue:

- `lsusb -v -d <vendor>:<product>` for the unit, in each mode it has.
- Which Linux driver you run it with, and what it exposes: `aplay -l`,
  `amixer -c <card> scontents`, and `ls -l /dev/snd/hwC*D*`.
- USB captures of TotalMix FX on Windows (USBPcap/Wireshark) while you
  move one control at a time, each named in the file name. One control per
  capture makes the protocol readable.
- What you can test, and whether we may write to the unit: a first
  backend should read before it writes, and nothing should reach the
  analog outputs before someone with the unit has checked the levels.

When TuxMix finds an RME unit it cannot drive yet, its waiting screen names
it from `tuxmix-core/src/usb_ids.rs`. Adding your unit's USB ID there is a
small first PR.

## Licensing of contributions

A contribution is accepted under the license of the crate it touches: MIT for
`tuxmix-core`, the tools and the docs, GPL-3.0 for the applications and the
USB backend. There is no CLA. Keep the SPDX header of the file you edit, and
add one to a new file that has none.

## Other ways to help

- **UI / UX**: improve the iced GUI or the ratatui TUI — see
  [Status](docs/guide.md#status) and `GUI-NOTES.md` for what's open.
- **Babyface Pro FS ALSA verification**: `babyface.rs`'s mapping
  (crosspoints, master volume/mute, phantom/pad/gain, loopback,
  AN 1>2/link/MS/dim/width/FX send/pitch, EQ) is hardware-validated as
  of 2026-08-28. Not yet wired: Sensitivity/Clock Source/SPDIF (no
  matching ALSA control exists in the kernel driver at all) and
  per-input EQ interaction edge cases — if you own the hardware,
  poking at those is directly useful.
- **MIDI SysEx backend**: help design the I/O layer for UCX II/UFX+/
  III-class devices — a bigger, separate effort from the recipes above.
- **Testing**: run `cargo test --workspace` and report anything that
  breaks, especially on hardware.

See the [open issues](https://github.com/ismail-bahloul/TuxMix/issues) for
starter tasks.
