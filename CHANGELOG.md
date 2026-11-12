# Changelog

## Unreleased

### Added
- A Presets menu in the EQ panel of an analog input, with TotalMix FX's
  fourteen factory presets (Modern Bass Treble, Telephone, Live Vocal Mic,
  Notch and the rest) and its Reset (flat bands at 50 Hz, 1 kHz and 10 kHz,
  12 dB/oct low cut). A preset sets the three bands and the low cut's
  frequency and slope, and leaves the EQ and low-cut switches as they were,
  as TotalMix does. The band types were read off TotalMix FX on a Babyface
  Pro.
- A mono button in the Control Room card, as in TotalMix FX: it sums the
  stereo sources into the Main pair. TotalMix writes the Main crosspoints of
  a playback pair at half level on both sides, which the kernel driver's new
  `Main Mono Switch` does; it needs a driver that has it. The 0.3.7 notes
  said Mono writes nothing over USB, which was wrong.
- A Dim knob in the device panel's Output section sets how much Dim takes
  off, from -65 to 0 dB (-20 by default), like "Main Out Dim" in TotalMix
  FX's Mixer Settings; double-click resets it. It drives the kernel driver's
  new `Dim Level`, so it needs a driver that has it.
- A "Send FX" knob in the settings panel of every input and playback strip
  and a "Return FX" knob in that of every output pair, as in TotalMix FX
  (-65 to 0 dB; double-click turns it off). They drive the kernel driver's
  new `<source> FX Send` and `<output> FX Return` controls, so they only
  show with a driver that has them. TotalMix's scales were measured with
  usbmon: a send has 0 dB at 0x1000, a return at 0x4000.
- With no supported device, the waiting screen names an RME unit it finds on
  the USB bus (Fireface UC, UCX or any other under RME's vendor ID) and says
  it is not supported yet, instead of only asking to check the cable. A
  Babyface Pro in class-compliant mode is told how to switch back.

### Fixed
- EQ for Record was inverted on the libusb backend: the toggle set bit 6 of
  the settings word when ticked, and the unit takes that bit as "EQ for
  Record off". It is now on by default, as in RME's settings dialog, and
  clearing it writes bit 6 (`0x0041`), checked against RME's driver and
  TotalMix FX. Scenes saved before this keep the unit's default instead of
  reading their old value back, since it meant the opposite.

### Changed
- An input strip whose output pair has Loop on is tagged LOOP instead of its
  type: its meter shows that output, not the input (#23).

### Fixed
- A mono input's pan follows TotalMix FX's constant-power law instead of a
  linear one: centred, each side sits 3 dB down (0x16a0 at 0 dB, measured on
  the real TotalMix), and a hard pan gives the whole level to one side. A
  centred source used to come out 3 dB louder than in TotalMix. The Matrix
  view and the output meters use the same split.
- With the proprietary USB backend, the pan of the analog inputs (AN1, AN2,
  Instr 3/4) now reaches the unit: it used to be ignored. Both crosspoints of
  the pair are written on TotalMix FX's constant-power law (centred 0x16a0 per
  side at 0 dB), and moving the fader or the phase switch keeps the pan.

## 0.3.7 (2026-11-09)

### Added
- A Control Room section against the mixer's right edge, by the sidebar, as
  in TotalMix FX on this unit: PH3/4 becomes Main, beside a card of Dim and
  Recall, with Store apart at the bottom. Recall is host-side, as in
  TotalMix, and lasts until the app closes. Mono, Ext. Input, Mute FX and
  Talkback are left out (Mono does write over USB: see Unreleased).

### Changed
- Every source file now carries an `SPDX-License-Identifier` and the
  copyright line, so attribution survives a fork or a vendored copy.
- `tools/alsa` is GPL-3.0-or-later rather than MIT: the plugin it builds links
  `tuxmix-sys` (GPL-3.0), so the binary it installs was already a GPL-3.0
  work. The rest of the tools stay MIT.
- [`TRADEMARK.md`](TRADEMARK.md) states the naming policy that applies to
  forks.
- The Mixer's faders grow to fill the window's height, as in TotalMix FX,
  instead of leaving the bottom of a tall window empty. They stop at twice
  their base length, and keep their base length (the page scrolls) when the
  rows do not fit.
- The T, Ø, settings and EQ buttons beside each fader are larger
  (18 x 20 px at 100 %, from 15 x 14), with larger labels.

## 0.3.6 (2026-11-05)

### Added
- Reset Mix, a menu beside the view tabs with the entries of TotalMix's
  Options > Reset Mix that this device can honour: straight playback, clear
  all submixes, clear channel effects (EQ and low cut off, the band settings
  kept), set output volumes (all at 0 dB), reset channel names, set all
  channels mono or stereo, set inputs mono and outputs stereo, and total
  reset. Each entry is one undo step.

### Fixed
- Undoing an EQ change or recalling a snapshot on the ALSA backend updated
  the EQ on screen but not on the card.
- An EQ band that had never been set read 0 Hz from the driver, so switching
  it on left the curve flat. It now starts at 1 kHz, Q 0.7, on the card too.
- The Submix field slid under the sidebar in a narrow window. Below about
  1000 px at 100 % zoom, the bar now drops the wordmark and the Submix label.
- `--version` started the GUI; both apps now print their version.

## 0.3.5 (2026-10-29)

### Added
- Release packages: each GitHub release gets a prebuilt binary tarball
  (the new `tuxmix-bin` AUR package), a `.deb` and an `.rpm`.
- A udev rule (`tools/udev/70-tuxmix.rules`), installed by every package,
  that lets the logged-in user open the card for the USB backend.

### Changed
- `tuxmix-gui` and `tuxmix-tui` are now GPL-3.0-only, what their binaries
  already were as a whole since they link `tuxmix-usb` and `tuxmix-sys`.
  `COPYING` holds the GPL-3.0 text and every package installs it;
  `tuxmix-core`, the tools and the docs stay MIT.

## 0.3.4 (2026-10-29)

### Changed
- Without an interface, the GUI no longer opens the simulation: it shows
  what is missing and what to check, waits for the card and opens it as
  soon as it is plugged in. The simulation is still there as a demo from
  that screen, under a banner, and `--mock` is unchanged. The TUI prints
  the same message and waits.

### Fixed
- Launching the GUI before plugging the card in left it in the simulation
  until a restart.
- A linked input pair showed as one strip but sent its two channels as
  mono into every submix, and its pan moved only the left channel. A
  linked pair now puts its left channel hard left and its right channel
  hard right, and its pan moves both together. The AN1/2 microphones are
  no longer linked by default, so a single microphone stays centred
  (#23).
- The ALSA backend now re-reads the card every two seconds, and once a
  second after opening. The driver does not notify host-side writes, so
  udev's `alsactl restore` could land after TuxMix's initial read and
  leave the faders showing the factory defaults while the unit held the
  restored levels, so a monitor fader read as off while the unit still
  passed signal (#23).
- The USB backend's crosspoint map had AN1/2 and PH3/4 swapped and every
  pair after it shifted, so a mix into one output landed in the next
  pair's submix. The map is now `0x34 * pair + 0x1A * side + source`,
  matching the kernel driver's fix and the captures in #31.
- The output meters showed a left-only source on the right output too:
  they summed each source's level into the whole output pair. They now
  follow its side, from the pan for the analog inputs and from the
  channel for the stereo sources and the playback channels (#23).

## 0.3.3 (2026-10-26)

### Added
- `--osc-dialect rme` makes the OSC bridge speak RME's documented
  TotalMix address space: 0-based indices, dB faders, pan in -1.0..+1.0,
  `/mix/{in|pb}/<source>/<submix>/{fader,faderlin,balpan,solo}`,
  per-channel `mute`, `/output/<n>/volume`, `/status/*`, and the `/send*`
  dump requests. `oscmix` is kept as an alias, since oscmix speaks the
  same dialect. The default dialect is unchanged.
- Meter peaks go out as `/level/{in|pb|out}/<n>` in dB, only when they
  move, with `/set/level/peak|rms|fx` selecting what each message carries.
- The RME dialect covers the per-channel preamp controls: `/input/<n>/gain`,
  `/48v`, `/pad` and `/phase`, plus `/output/<n>/loopback`, in both
  directions and in the full snapshot.

### Fixed
- A full-state snapshot no longer loses its tail: the outbound queue held
  100 messages and `try_send` silently dropped the rest, which hid every
  output channel on refresh.
- A preamp gain changed on the unit now reaches the UI: the ALSA backend
  re-reads the four gains on a 250 ms poll, not only on an ALSA event.
- Opening TuxMix no longer re-applies the auto-saved snapshot over the ALSA
  backend: the unit is the source of truth for levels and gains, so only
  the channel names are restored. A crosspoint at the 50% position now
  reads as 0 dB instead of being rewritten to -6 dB.
- Input meters no longer open the card's capture PCM directly: they read
  PipeWire's own source when it is running, so the capture node stays in
  the graph and the card no longer reports the meter's 1024-frame period.
  A system without PipeWire keeps the direct capture as a fallback.

## 0.3.2 (2026-10-23)

### Fixed
- `tuxmix` and `tuxmix-tui` print their usage for `-h`/`--help` instead
  of trying to open a device.
- With no device, the simulation fallback no longer applies the real auto
  scene (which failed with `SceneModelMismatch`), and the message now says
  that the simulation is starting.

## 0.3.1 (2026-10-23)

### Fixed
- Input meters follow the kernel driver 0.2.0's capture order (AN1-4,
  AS1/2, ADAT3-8; AS1/2 is now on capture 4/5).

## 0.3.0 (2026-10-23)

### Added
- Background mode: tray icon (StatusNotifierItem) with Hide/Show, Dim,
  Recall mix 1-8, Settings and Quit; closing the window keeps the app
  running when a tray exists; one instance per session; `--minimized`;
  autostart toggle; Ctrl+, opens TuxMix settings.
- Device settings: PipeWire sample rate and buffer size (16 to 2048
  samples, the low ones when the driver allows), with a one-line latency
  readout from the card's own stream.
- Quick view opens the strips' settings, EQ, trim and route panels, drawn at
  the strips' scale.
- Refused device writes are shown for a few seconds instead of swallowed.
- Zoom control, adaptive window scaling, resizable and double-click
  collapsible sidebar, undo/redo, snapshots and layouts, custom channel
  names.

### Changed
- The GUI runs as an iced daemon so that hiding in the tray closes the window
  (hiding is a no-op on Wayland).
- Dev builds optimize dependencies, release builds use thin LTO: clicks were
  laggy under `cargo run`.
- Idle CPU: adaptive tick, cached canvas layers, 1024-frame input metering.

### Fixed
- Strip panels could not be closed again in stacked layouts (canvas widgets
  answered `Idle` instead of `None` to the pointer).
- Linked-pair solo cleared its partner on the hardware backends.
- Undo recorded empty steps on Matrix hover.
