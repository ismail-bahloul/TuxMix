# Changelog

## Unreleased

### Changed
- Without an interface, the GUI no longer opens the simulation: it shows
  what is missing and what to check, waits for the card and opens it as
  soon as it is plugged in. The simulation is still there as a demo from
  that screen, under a banner, and `--mock` is unchanged. The TUI prints
  the same message and waits.

### Fixed
- Launching the GUI before plugging the card in left it in the simulation
  until a restart.
- The ALSA backend now re-reads the card every two seconds, and once a
  second after opening. The driver does not notify host-side writes, so
  udev's `alsactl restore` could land after TuxMix's initial read and
  leave the faders showing the factory defaults while the unit held the
  restored levels — a monitor fader read as off while the unit still
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
