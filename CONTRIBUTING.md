# Contributing to TuxMix

TuxMix is pre-alpha and the architecture is still settling, but the most
useful thing outside contributors can do right now — adding support for a
second real RME device — has a concrete recipe below. If you're not sure
where something fits, open an issue first rather than guessing.

## Development setup

```bash
git clone https://github.com/iswad-lab/TuxMix
cd TuxMix
cargo build --release --workspace
cargo test --workspace

# Run the GUI/TUI against a simulated device — no hardware required:
cargo run -p tuxmix-gui -- --mock
cargo run -p tuxmix-tui -- --mock
```

Dependencies: `rustc`, `cargo`, `alsa-lib` (dev headers).

## Adding a new ALSA-selem device profile

This is the highest-value contribution right now if you own an RME
interface other than the Babyface Pro FS. It only covers **ALSA
simple-mixer-element (selem) devices** — USB class-compliant interfaces
like the Babyface family. RME's newer MIDI-SysEx-based models (Fireface
UCX II, UFX+/III) need a different I/O backend entirely and aren't in
scope for this recipe — see [Architecture](README.md#architecture) in the
README if you want to help with that instead.

1. **Get real `amixer scontents` output from your device.** Plug it in
   and run `amixer -c <card name> scontents > scontents.txt`. You'll need
   this to confirm every step below — don't guess at control names.

2. **Copy `tuxmix-core/src/profiles/babyface_pro.rs`** to a new file
   (e.g. `profiles/digiface.rs`) and adjust its `const PROFILE:
   DeviceProfile` to your device's real channel layout: `card_substring`
   (a substring of your card's name as it appears in
   `/proc/asound/cards`), `model_name`, one `InputSpec` per physical
   input in channel-id order, one `OutputPairSpec` per stereo output
   pair. Register the new module in `profiles/mod.rs`.

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

5. **Add tests.** `mock.rs`'s existing suite (channel counts, output pair
   count, phantom-power defaults, etc.) is the pattern to follow — it
   should read straight off your new profile's data, the same way the
   Babyface Pro FS ones do.

6. **Open a PR** with your profile, the adjusted naming helpers if
   needed, and a note on which real device you tested against.

## Other ways to help

- **UI / UX**: improve the iced GUI or the ratatui TUI.
- **Babyface Pro FS ALSA verification**: the existing mapping in
  `babyface.rs` hasn't been checked against real hardware yet (see
  [Release status](README.md#release-status)) — if you own one, comparing
  it against real `amixer scontents` output is directly useful.
- **MIDI SysEx backend**: help design the I/O layer for UCX II/UFX+/
  III-class devices — a bigger, separate effort from the recipe above.
- **Testing**: run `cargo test --workspace` and report anything that
  breaks, especially on hardware.

See the [open issues](https://github.com/iswad-lab/tuxmix/issues) for
starter tasks.
