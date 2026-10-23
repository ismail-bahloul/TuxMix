# Changelog

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
