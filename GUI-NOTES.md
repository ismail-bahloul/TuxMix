# GUI Improvement Notes

Observations and roadmap for polishing the TuxMix GUI, beyond the current
pre-alpha functionality-first baseline.

## Current state

The GUI is **functionally complete and well-architected**:

- Custom canvas fader with dB tapering, snap-to-default, fine-drag (Shift),
  scroll-wheel nudge, double-click reset — all the ergonomics of a real
  console fader.
- VU meters with proper ballistics (fast attack, tapered release),
  interpolated between keyframes at display refresh rate (144 Hz smooth).
- Collapse/expand animation for channel strips.
- Live UI zoom (Ctrl+=/Ctrl+-/Ctrl+0) scales every widget proportionally.
- Multi-select (Ctrl/Shift+click) with grouped mute/solo/collapse/volume/pan.
- Dark theme, consistent type scale, spacing constants, corner radii.

**What it needs is visual polish, not re-architecture.** The codebase is
healthy; the skin is what's rough.

**2026-09-06 update**: most of the polish this section called for has
since happened (§9/§10) — palette, fader/meter/knob redesign, the right
sidebar, and a from-scratch Matrix view in both the GUI and TUI. A
hardware-audit pass on top of that found and fixed several real
correctness bugs (not just cosmetic ones) that had nothing to do with
visual polish: ALSA-backend Mute/Solo were silent no-ops, Hardware
Output VU meters read the wrong pair (or a permanent zero) in both
UIs, and the EQ frequency controls (a knob in the GUI, keyboard
stepping in the TUI) mapped a 3-decade range linearly instead of
logarithmically. See §7's updated roadmap table and the hardware-audit
entries scattered through the rest of this file for the specifics —
this top section is kept as the original framing, not rewritten, since
it was accurate for what it was assessing at the time.

---

## 1. Visual polish — high impact, low effort

### 1.1 Icons instead of text labels — mostly already done on inspection

Re-checked against the actual code (`widgets/strip.rs`, `theme.rs`)
before implementing — three of the four items here were stale:

- **48V / PAD** — already styled as colored badges via
  `theme::toggle_button` (filled background + glow shadow when active,
  press-darken, rounded corners). Nothing to do.
- **M / S** — kept as plain text, deliberately, not a gap. These are
  the universal mixing-console abbreviation (every DAW, every hardware
  strip, TotalMix itself) — real icons (speaker/headphones) would cost
  clarity for the target audience for no real gain. `🔇`/headphone
  glyphs are also emoji codepoints Inter doesn't cover (confirmed via
  the font's cmap), so they'd render as tofu on top of being a UX
  downgrade. Not pursuing this.
- **Collapse/expand** — done. Swapped `-`/`+` for `▼`/`▶` in
  `header_row` (`widgets/strip.rs`). Confirmed via the font's cmap
  that Inter covers U+25B6/U+25BC (it doesn't cover emoji, which is
  why those stay off the table above), so the old "no chevron, risk of
  tofu boxes" comment in the code was stale — updated it in place.
  Verified visually via headless screenshot: ▼ expanded, ▶ collapsed.

### 1.2 Visual depth and hierarchy

| Element | Current | Proposed | Status |
|---|---|---|---|
| Background | `#0d0d0d` flat | Subtle dark gradient or very faint grid | **Done** — `theme::root` now a top-to-bottom `iced::gradient::Linear`, ~3.5% white blended in at the top fading to pure `BG_DEEP` at the bottom. First attempt used `Radians(FRAC_PI_2)` and rendered as a barely-visible *horizontal* gradient — `Radians::to_distance` subtracts its own `FRAC_PI_2` before building the direction vector (`0` = "to top", CSS-angle convention), so `PI` is what's actually top-to-bottom. Found by reading `iced_core`'s source, not guessing twice. Confirmed via pixel sampling down the left margin: 21 → 20 → 18 → 15 → 13, matching the exact blended value at each stop. |
| Strip panel | flat | Subtle border glow on hover/selection | **Done** — selection already had a border; hover now gets its own (lighter border + soft `ACCENT`-tinted shadow), losing to selection's border when both apply. Needed real state since `container::Style` closures don't get a hover `Status` the way `button` does: `TuxMix::hovered_strip`, driven by `mouse_area::on_enter`/`on_exit` in `widgets/strip.rs`, threaded through `strip_params()` like every other per-strip flag. Confirmed via headless screenshot — visibly lighter border + halo on the hovered card vs. its unhovered neighbors. |
| Top bar | Same shade as strips (`SURFACE`) | Slightly different shade to delineate | **Done** — now `blend(SURFACE, BG_DEEP, 0.45)`, confirmed via pixel sample (`rgb(19,19,20)`, exactly the expected blend) |
| Buttons (M/S/48V) | Flat color fill | Subtle inner shadow when pressed, slight border | Already effectively covered by existing `toggle_button` (press-darken + border) — not a true inset shadow, but reads the same; not revisiting |

### 1.3 Fader cap styling

- Rounded cap — already there (`cap_radius`).
- Different shade when dragging — already there (near-white on drag).
- **Thin center line as a position indicator — done.** The cap already
  drew 3 grip ridges; the middle one (`i == 2`) sits exactly at `cap_y`
  (the value position), so instead of adding a 4th line on top of it,
  it's now drawn brighter/white instead of the dark grip color —
  serves as both grip texture and position marker. Not confirmed
  visually (headless canvas-layering limitation, see `run-gui-headless`
  skill notes) but it's a deterministic color swap on existing,
  already-correct geometry, and the build/test suite is green.
- Subtle glow on the cap — not done, same "needs a screenshot to judge"
  caveat as the background gradient above.

### 1.4 Pan indicator — already done on inspection

Re-checked `widgets/fader.rs::PanIndicator` before touching anything:
this is already a real canvas widget — a dot/puck on a groove line,
color shifts on active drag (`ACCENT` → near-white while dragging),
with the numeric text ("L25"/"C"/"R50") kept below it as a readout,
not instead of it. This note was describing the target state of a
widget that had since been built; nothing left to do here. (No hover
tooltip on the numeric value specifically, but that's a minor addition,
not the missing "visual indicator" this item was originally about.)

---

## 2. Layout improvements — medium effort

### 2.1 Strip width flexibility

`STRIP_W = 104px` is still fixed (no compact/wide mode) but the third
item here is **done**: when a row's content is narrower than the
window, it's now centered instead of left-aligned with dead space to
the right (`app.rs::responsive_row`, tracking real window width via
`Message::WindowResized`). Falls back to the existing horizontal
scrollable once content overflows. Verified via headless screenshots
at 2400px (centered, equal margins) and 1000px (scrolls, no clipping).

Still open:
- **Compact mode** for dense layouts (16+ channels): narrower strips,
  smaller fader, hide labels
- **Wide mode** for precision: full width with larger fader and extra spacing

### 2.2 Channel groups / separators

Visual separator lines between channel types already exist (AN → IN → AS →
ADAT), but they're thin rules. Consider:
- Color-coded section backgrounds (blue tint for Mic, green for Line, etc.)
- Collapsible channel groups (collapse all ADAT at once)
- Drag-to-reorder strips within a group

### 2.3 Output section layout — done

Implemented in `widgets/strip.rs`: three multipliers (`OUTPUT_WIDTH_MULT`
1.25x, `OUTPUT_FADER_MULT` 1.15x, `OUTPUT_BUTTON_MULT` 1.2x), applied only
for `ChannelId::Output`, computed once via `matches!(cid, ChannelId::Output(_))`
inside `full_strip`. `full_width(cid)` (the new home for what used to be
the bare `STRIP_W` constant) is `pub(crate)` and also used by
`app.rs::set_collapsed`, so the collapse/expand width animation targets
the correct wider size for output strips instead of snapping.

"Show output assignment / label at the top" wasn't actually missing —
output strips already show their own name (`AN1 OUT`, etc.) via the same
header every strip uses; no separate work needed there.

Verified: `cargo test -p tuxmix-gui` 20/20, and a headless screenshot with
pixel-edge measurement confirming the output row's cards render measurably
wider (~130px vs. ~95-104px for input/playback strips, matching the 1.25x
multiplier) with visibly taller faders and M/S buttons.

---

## 3. New features — medium to high effort

### 3.1 Drag & drop strip reordering

Allow dragging strips horizontally to reorder them. The `RmeDevice` trait
uses positional indices, so reordering would be a UI-only view transform
(id → position mapping) rather than changing hardware channel IDs.

### 3.2 Color tagging

Let users assign a color to any channel. The color should appear as:
- A small dot or bar at the top of the strip
- A tinted background tint when the strip is not selected
- Persisted in Scene JSON

### 3.3 Search / filter bar

A search box in the top bar that filters visible strips by name substring.
Useful when working with 20+ channels (ADAT-heavy configurations).

### 3.4 Fullscreen mode

A simple toggle (F11 or View menu) — most studio setups run the mixer on a
dedicated screen in fullscreen.

### 3.5 Web UI

An HTML/JS web frontend communicating with the existing OSC bridge would
allow tablet control from the studio couch — a common workflow pattern.
The OSC bridge already supports the full address space; a web UI would
just be a client.

### 3.6 Undo / Redo

Push state snapshots onto a stack. Low priority for v0.1 but important for
production use — especially with destructive scene operations.

### 3.7 Quick Control mode — done

Implemented in `app.rs` (`View` enum, `quick_view`, `quick_channel_options`,
the `strip_params` helper factored out of `mixer_view` so both views build
strips from the same logic). QUICK is now the default tab, left of
MIXER/MATRIX in the top bar.

Scoped down from the original mockup to what `tuxmix-core`'s `RmeDevice`
trait actually exposes today — no DIM, no SENS switch, neither exists in
the data model independent of this feature:

```
Source [ AN1 · IN  v]

┌─── AN1 MIC ─┐    ┌─── AN1 OUT ──┐
│  M  │  S    │    │  M  │  S     │
│ 48V │ PAD   │    │              │
│   ██▌       │    │   ██▌        │
│  -2.5 dB    │    │   0.0 dB     │
└─────────────┘    └──────────────┘
```

- **Source** picker lists every input/playback channel; the destination
  block is always `Output(sel_out * 2)` — the existing Submix picker
  already in the top bar doubles as "select destination", so there's no
  second picker to build or keep in sync.
- Both blocks are the *exact same* `strip::strip()` widget used in the
  Mixer view, just rendered at 2x scale (`QUICK_SCALE_MULT`) — not a
  bespoke widget. Gets fader drag/scroll/reset, VU ballistics, mute/solo,
  48V/PAD, tooltips, and OSC feedback for free, with zero new interaction
  code to maintain.
- Tab key still cycles Mixer/Matrix as before; from Quick it lands on
  Mixer. Leaving Quick is otherwise via the tab buttons, as the original
  note intended.
- Verified: builds, `cargo test -p tuxmix-gui` 20/20, and a full headless
  visual pass — default-open-on-Quick, the source dropdown listing all
  channels correctly, switching source (IN3, no 48V/PAD as expected for
  an INST channel), and Mixer/Matrix still rendering correctly after the
  `strip_params` refactor.

**Why this is a good idea:**

- Neither TotalMix nor oscmix has a simplified mode — this is a unique
  differentiator
- Very low development effort (backend unchanged, just a new view in the
  UI)
- Lowers the entry barrier for non-technical users (podcasters,
  streamers, remote workers)
- Works equally well in GUI and TUI — the simplified TUI stays readable
  even at 80×24
- Perfect if a future Web UI needs to run on tablet/phone

**Placement in the UI:**

```
Top bar: [Quick] [Mixer] [Matrix]
             ↑ new default tab — simplest possible view
```

The app opens on Quick by default. New users never see the matrix unless
they need it.

---

## 6. Killer features — strategic differentiators

These features go beyond polish. They fundamentally change what the
project is capable of and who it serves — things no competitor
(TotalMix, oscmix, bbfpromix) can match.

### 6.1 TUI (terminal / SSH) — already done, under-marketed

Already implemented in `tuxmix-tui` (ratatui + crossterm). This is
the single biggest differentiator vs TotalMix and every other RME
controller — **none of them have a terminal interface.**

**Killer use cases:**
- Broadcast server in machine room → adjust levels via SSH from the
  control booth, no X forwarding, no VNC
- Fixed installations (conference rooms, houses of worship, theaters)
  → headless machine, full control over SSH
- Live sound → quick adjustments from FOH terminal
- CI / automation → scripts that tweak mixer state programmatically

**Polish pass — done:**
- Fixed a real functional gap, not just cosmetics: strips showed *no*
  level readout at all — only mute/solo/48V/PAD flags and a VU meter
  that's only ever nonzero in `--mock` (`input_meter`/`playback_meter`
  return `0.0` on real hardware). A real-hardware TUI user had zero way
  to see what level a channel was actually at. Every strip now shows a
  dB readout (`db_text`, formatted to match `tuxmix-gui`'s own), sourced
  from `ch.volumes[0]`/`ch.volume` — always accurate regardless of
  device backend.
- Added the missing **PAD** key (`P`, mirroring `p` for 48V) — was
  gated the same way as `tuxmix-gui`'s strip (Mic-type inputs only).
  Footer help text updated.
- Deduplicated the five copies of the `section → ChannelId` match
  arm into `selected_channel_id()`.
- Module doc comment now documents the actual use cases (SSH control
  room, headless fixed installs, live sound, scripting) instead of just
  the two launch commands.
- Added `tests` module (`db_text`, `selected_channel_id`) — this crate
  had zero tests before.
- Verified: `cargo test -p tuxmix-tui` 2/2, and an actual pty capture
  (`python3`'s `pty.fork`, no `tmux` installed in this sandbox) of a
  live `--mock` run — confirmed the dB readout, the 48V flag, and the
  updated footer all render correctly with no panics.

**Deferred, not done:** a dedicated Quick Control view for the TUI
(mirroring `tuxmix-gui`'s — see §3.7). That's a new view, not polish;
scoping it in here would have blown well past a "polish" pass. Also
still missing, same as the GUI's own submix picker: the TUI only ever
reads/writes submix bus 0 (AN1/2) — `+`/`-` hardcode `dev.volume(cid, 0)`,
pre-existing, not touched by this pass.

---

### 6.2 Web UI (tablet / phone control)

An HTML/JS web frontend communicating with the existing OSC bridge
would allow **tablet control from anywhere in the studio** — no
proprietary app, no OS restrictions.

**Why it's a killer:**
- TotalMix has NO web UI. oscmix has one but UCX II only.
- Put a tablet on the piano → musician adjusts own headphone mix
- Give a limited view to a guest artist ("here, tweak your monitor")
- Works on iPad, Android, any laptop
- Nothing to install on the client device — just a browser

The OSC bridge already supports the full address space with 20+ test
cases. The web UI is just a client waiting to be written.

**Effort:** weeks (depends on polish level). A minimal version can be
done in days using the existing OSC bridge.

### 6.2.1 OSC debug log panel — done

Not originally in this document — added after comparing against oscmix's
own Qt UI screenshot (`doc/qt-preview.png` in their repo), which has a
dedicated debug log window (incoming/outgoing OSC traffic, filters,
max-lines cap). Genuinely useful and cheap to add given the bridge already
existed: every message crossing `osc::worker` in either direction now
formats a `dir addr args` line (`osc.rs::log_line`) and sends it as
`Message::OscLog`, independent of whether the panel is open — so opening
it after a burst of activity isn't a blank page. Stored in a capped
(`OSC_LOG_MAX = 500`, matching oscmix's own default) `VecDeque`, newest
first, docked as a fixed-height drawer under the main view rather than a
separate OS window — `tuxmix-gui` doesn't use multi-window anywhere else,
so a floating window would've been the one inconsistent part of the UI.
Toggled via an "OSC LOG" button in the top bar, only rendered when
`--osc` is active.

Verified end-to-end: `cargo test` unaffected (39/39 across the workspace),
and a full headless run with `--mock --osc` — sent real UDP OSC messages
via the existing Python test client, confirmed both the fader moving *and*
the corresponding `IN`/`OUT` lines appearing in the panel with correct
timestamps/addresses/args, including the ~100-line startup snapshot burst.

---

### 6.3 MIDI-triggered scene crossfade

Recall any saved scene via a MIDI trigger (footswitch, controller
keyboard), but instead of snapping instantly, **transition smoothly**
over a configurable duration (0.5s / 2s / 5s).

**Correction:** a plain MIDI-trigger → scene recall is *not* unique —
TotalMix FX already does this (snapshots 1-8 mapped to MIDI notes via
Options > MIDI Control, see
[RME's MIDI Remote docs](https://docs.rme-audio.com/aoxd/850-1c_midi_remote_tmfx/)).
The actual differentiator is the **crossfade**: TotalMix always snaps
instantly, with no transition. Combining the two — a footswitch that
triggers a *smooth* scene change — is what neither TotalMix nor oscmix
offers.

Transition every parameter linearly: volumes crossfade, mute states
change at defined points, pan positions move smoothly.

**Why it's a killer:**
- Live musicians: change entire routing between songs with one foot
  tap, no click or pop
- Radio / podcast: fade from "guest mic only" to "all hosts live"
  without a jump
- Live: crossfade between two completely different monitor mixes
  during a set change — no interruption
- Stream: transition between scenes without viewers hearing a jump

**Why it's feasible:** the `RmeDevice` trait already has `capture_scene`
and `apply_scene`. Morphing is just running `apply_scene` in small
increments over time, interpolating the vector of floats between the
current state and the target scene. MIDI triggering needs a new
worker — no MIDI infra exists in the project yet — following the same
pattern as `osc.rs` (never touch `state.device` off the update-loop
thread).

**Effort:** 1-2 weeks (interpolation engine in core, UI duration
control, MIDI listener worker).

---

### 6.4 Headless mode + HTTP / REST API

Run TuxMix without any UI at all, controlled entirely over HTTP:

```bash
rmixd --headless --listen :8080
curl -X POST localhost:8080/scene/load -d '{"name":"Studio"}'
curl localhost:8080/channels/input/0/level
```

**Why it's a killer:**
- Integration with broadcast automation (OCTOPUS, Dalet, WideOrbit)
- Theaters → QLab / sound control systems talk HTTP
- Conference rooms → Crestron / Extron touch panels
- Home automation → Home Assistant / OpenHAB
- Scripting for load tests, monitoring dashboards, cron jobs

Every established audio control protocol (OSC, MIDI) requires specific
hardware or knowledge. HTTP is universal. A REST API makes TuxMix
integrable into **any** modern system.

**Effort:** 1 week (HTTP layer on top of existing `RmeDevice` trait).

---

### 6.5 Scene morphing + MIDI recall + Quick Control = The Trifecta

Individually, each of these features is useful. Together, they create
a **workflow that doesn't exist anywhere else in the RME ecosystem**:

```
Podcaster setup:
  1. Create scenes: "Solo" / "Interview" / "Panel"
  2. Assign each to a MIDI footswitch button
  3. Quick Control view shows only what matters: mic volume + headphones
  4. Step on switch #1 → gradual crossfade to "Solo" over 2 seconds
  5. Open Web UI on tablet to adjust levels from across the room
```

This is not a "better TotalMix." This is a **whole new category** of
RME controller that TotalMix — tied to its desktop GUI and snapshot
model — cannot be.

---

## 4. Comparison with oscmix UI

See [README.md](README.md) for the feature comparison. On UI specifically:

| Aspect | TuxMix (iced) | oscmix-gtk | oscmix-web |
|---|---|---|---|
| **Fader** | Custom canvas with dB taper, snap, fine-drag | GTK scale widget | HTML range slider |
| **VU meter** | Animated, ballistics, color gradient | GTK levelbar (colored blocks) | HTML meter element |
| **Visual polish** | Dark theme, consistent, basic | GTK native (varies by theme) | Functional, minimal |
| **Channel strip** | Compact, collapsible | Fixed width | Details-based |
| **Layout** | Horizontal scroll per section | Vertical paned sections | Scrollable flex |
| **EQ plot** | None | Custom canvas widget in GTK | SVG canvas in browser |
| **Effects** | None | Reverb + Echo panels | Same via sidebar |
| **Matrix view** | Dedicated matrix tab | Routing mode (submix/free) | Routing mode selector |

**oscmix's strengths:**
- Full EQ + FX control (reverb, echo, dynamics) — much deeper hardware control
- Working EQ plot widget (custom GtkWidget with frequency curve)
- Web UI runs in any browser, no install needed
- Control Room + DURec integration

**TuxMix's strengths:**
- Higher-quality fader (dB tapered, snap, fine-drag, scroll — all absent in oscmix)
- Smoother VU meters with proper ballistics
- Dedicated Matrix view (oscmix does routing but no grid-style matrix)
- Scene save/restore (oscmix has no preset system)
- TUI mode (oscmix has no terminal interface)
- Collapsible strips for dense layouts

**Bottom line:** oscmix has *deeper hardware control* (EQ, FX, DURec, Control
Room) because its target device (UCX II) exposes those features. TuxMix has
*better core mixer UX* — the fader, meters, and matrix view are already more
polished and usable than oscmix's equivalent controls.

---

## 7. Priority roadmap

**This table predates the 2026-09 redesign (§9/§10) and the hardware-
audit pass that followed it — updated 2026-09-06 rather than left to
mislead a future read as "Undo/Redo: not for v0.1" when it's been
shipped for days.** For what actually changed and why, §9/§10 and the
hardware-audit entries scattered through this file are the real record;
this table is just the priority-tier summary kept in sync with them.

```
Priority │ Feature                       │ Effort   │ Why
─────────┼───────────────────────────────┼──────────┼──────────────────────────────
   Done  │ Icons for M/S/48V/PAD         │ —        │ Chevron done; badges/pan were already there
   Done  │ Pan visual indicator          │ —        │ Was already a real widget, not text
   Done  │ Quick Control mode            │ —        │ Unique differentiator, big audience
   Done  │ TUI polish + doc              │ —        │ dB readout, PAD key, dedup, doc, tests
   Done  │ Output strip distinction      │ —        │ Wider/taller/bigger-buttons, measured via pixel edges
   Done  │ Strip visual polish           │ —        │ Bg gradient + hover glow, both pixel-verified
   Done  │ Undo/Redo                     │ —        │ §10: Vec<Scene> stacks, is_undoable() gate
   Done  │ Right sidebar (control strip) │ —        │ §10: device chip, M/S, Snapshots, Groups, Layout
   Done  │ Matrix view rebuild (GUI+TUI) │ —        │ Real TotalMix axes, both UIs; hidden-fader cells (GUI)
   Done  │ Real ALSA mute/solo/metering  │ —        │ Hardware-audit pass: silent no-ops fixed, AN1/AN2 VU
   P2    │ MIDI-triggered scene crossfade│ 1-2 wks  │ Crossfade is unique; MIDI trigger alone isn't
   P2    │ Drag-to-reorder strips        │ 5-8 hrs  │ Usability for large sessions
   P2    │ Color tagging                 │ 3-4 hrs  │ Studio workflow standard
   P2    │ Search/filter bar             │ 2-3 hrs  │ Useful with 20+ channels
   P3    │ Fullscreen mode               │ < 1 hr   │ Common studio setup
   P3    │ Web UI (tablet)               │ weeks    │ Tablet control, big differentiator
   P3    │ Headless + HTTP API           │ 1 wk     │ Broadcast/theatre integration
   P3    │ Non-exclusive solo/PFL mode   │ medium   │ Needs usb.rs's exclusive-solo logic touched — see §10
```

**P0/P1** = all done. **P2** = usability improvements for power users —
the next open tier, untouched by the 2026-09 work above (still real
gaps, not stale entries). **P3** = ambitious, unlocks new audiences, or
(the last row) scoped out of every pass so far for a specific,
documented reason rather than just not gotten to yet.

The Done items alone already make TuxMix the most versatile RME
controller on any platform — P2/P3 are about going further, not catching up.

---

## 8. Resize & scale — decisions (2026-09-02)

Recording the conclusion of the resize/scale thread so it doesn't get
re-litigated blind:

- **Model: fixed geometry + scroll, manual zoom only.** After trying every
  resize-driven scaling (unbounded = huge strips; a cap = "rescale plateau";
  shrink-to-fit = tiny controls), the chosen behaviour is the pro standard:
  strips keep a fixed, legible size; a window too narrow for a row scrolls
  horizontally, too short scrolls vertically; and zoom is an explicit,
  sticky user action — Ctrl+molette, Ctrl+=/Ctrl+- and Ctrl+0
  (`ZoomIn`/`ZoomOut`/`ZoomReset` → `zoom`, bounded by `SCALE_MIN`..`SCALE_MAX`).
  A window drag never touches `ui_scale`, so it's cheap and fluid by design.
- **`RESIZE_THROTTLE` (16 ms) kept**: it still coalesces `Resized` events in
  `apply_pending_resize` (a drag fires them faster than the display can
  present) so `window_width`/scroll decisions update ~once per frame instead
  of rebuilding on every single event.
- **Real fix for the empty space on the right** (kept from the adaptive era):
  strip-width bookkeeping counts *rendered* strips per pair (linked → 1,
  split → 2), not every channel — over-counting linked rows ~2× used to
  shrink everything and leave the gap.
- **Open (needs eyes + hardware):** whether narrow windows should shrink to
  `SCALE_MIN` before scrolling (current) or stop at a readable floor and
  give each row its own draggable horizontal scrollbar (TotalMix v2 shows a
  per-row scrollbar once a row overflows). Both are cheap to switch; pick
  after looking at a live `--mock` window drag on a real screen.

---

## 9. Bus/channel-strip visual redesign (2026-09-02) — palette + fader/meter/knob pass

First concrete pass at the "make it look like TotalMix" redesign, using
real sampled colors from the live 2026-08-30 screenshots (`Totalmix UI
Screenshots/`, now copied into the repo checkout itself — no NTFS mount
needed) rather than guessing. Verified live via the `run` skill (KDE
XWayland + `xdotool` + `spectacle`, same technique as the 2026-08-29
session), not just build+test.

**Palette (`theme.rs`)** — every color sampled by pixel-picking the real
screenshots (`python3`/PIL `getcolors()` on cropped regions, not eyeballed):
`BG_DEEP`→`#14171a`, `SURFACE`→`#384147` (a cool blue-gray strip card —
previously near-black, almost the same shade as the background, so cards
barely read as separate from the page), `BORDER`→`#4a545c`, `ACCENT`→`#d48354`
(a muted terracotta orange — TotalMix's own dominant "engaged" color,
used almost everywhere something is lit: stereo link, EQ on, route/
settings open, the submix picker, 48V/PAD). `MUTE_COLOR`→`#17bfcf` (cyan)
and `SOLO_COLOR`→ the same orange as `ACCENT` — this cyan/orange pairing
is TotalMix's own specific signature (confirmed via a clean zoomed crop
of its sidebar M/S/F buttons), previously red/amber here. `PHANTOM`
(48V) also moved to the same orange (real TotalMix's `+48V` pill is
orange, not red) — dropped the separate "danger red" in favor of
matching the reference. Left `MGREEN`/`MRED` (meter hot-zone gradient)
untouched — TotalMix's own ruler ticks use the same red-top/green-lower
convention, so these were already directionally right.

**Pan knob arc (`widgets/knob.rs`)** — added `Knob::arc_from_center: bool`,
a filled orange arc from 12 o'clock (the range's center/default position)
round to the current value, matching a confirmed live-screenshot detail
(the "L57" pan knob showing a gold arc swept toward the panned side).
Enabled for every *bipolar/centered* knob (Pan, Pitch, Width, EQ band
gain — all default to the middle of a symmetric range) and left off for
one-sided ranges with no meaningful center (Gain, EQ freq/Q, low-cut
freq) — inferred consistently rather than confirmed pixel-by-pixel for
every one of those sites, flagged here in case a future screenshot shows
otherwise for Width/Pitch specifically.

**Fader dB ticks (`widgets/fader.rs`)** — floor changed from -65 to -60
and the ticks now exactly match the real ruler's set: `0, -6, -10, -20,
-40, -60` (dropped the unlabeled +6 top tick to match what the reference
actually shows).

**Fader/meter/ruler reorder + a real layout bug fix (`widgets/fader.rs`,
`widgets/strip.rs`)** — the biggest structural change. TotalMix's real
layout is `[fader][ruler][meter]` left-to-right; ours had `[meter+ruler]
[fader]`. Reordering (`Fader::layout_x`, replacing the old `track_x`)
incidentally **fixed a real, live bug**, not just a cosmetic mismatch:
the old code centered the *track alone* on the canvas width and tucked
the meter/ruler to its left assuming there'd be room, but at the strip's
actual on-screen width (`STRIP_W=80`) that pushed the meter/ruler column
to a *negative* x — almost entirely clipped off the card. Confirmed via
live screenshot: the ruler's tick labels were rendering as stray
parenthesis-shaped fragments (only the rightmost curve of each digit
survived the clip). Centering the *whole three-part group* on the
canvas width instead fixes this unconditionally, not just for the
default strip width.

Reordering also exposed a second instance of the same class of bug: the
settings-gear/EQ/collapse buttons sit in a side column *next to* the
fader (`row![fader_widget, icon_col]`), eating into the same width
budget — with the meter now on the *right*, its two-digit ticks
("10"/"20"/"40"/"60") started rendering underneath those buttons instead
of off-canvas. **First fix attempt (wrong, corrected by the user):**
moved gear/EQ/collapse into a row *below* the fader instead, reasoning
(incorrectly) that a narrower crop of the reference showed them there.
The user caught this directly — a fuller zoom on an actual strip shows
the gear/EQ column sitting on the *right flank*, beside the fader/meter,
spanning roughly the fader's lower half, not underneath it. Reverted:
gear/EQ/collapse are back in `row![fader_widget, icon_col]` beside the
fader. The *actual* fix for the width collision was widening `STRIP_W`
80→96 (see its own doc comment) — the real shortfall was that 80px never
had room for `[track+ruler+meter]` *and* a side icon column together at
these widget sizes, regardless of which side anything was on; moving
things to a new row had just been treating the symptom.

**Verified live**: QUICK and MIXER views both screenshotted after each
change (`--mock`, KDE XWayland via `env -u WAYLAND_DISPLAY DISPLAY=:0`).
Confirmed: strip cards now read as visibly separate blue-gray panels
against the near-black page; the fader's dB ruler shows full two-digit
labels with no clipping/occlusion; pan knobs render (arc is zero-length
and invisible at the default centered value, as expected — not yet
visually confirmed with a knob actually turned off-center); gear/EQ/
collapse column sits cleanly beside the fader on every strip, at the
wider `STRIP_W`, with no clipping. `cargo test -p tuxmix-gui`: 27/27
throughout, including after the revert.

**Explicitly deferred, not attempted this pass** (scope was "get the
fader/meter/knob structure and palette right first," per the user's own
framing of Bus design as the starting point):
- The real TotalMix meter is a **monochrome amber peak-hold line + small
  decaying marker**, not a filled green→red bar — kept our own gradient
  fill since it's arguably more informative and the peak-hold mechanic is
  a real chunk of new interaction/animation state, not a palette tweak.
- **Mute-by-solo vs. explicit-mute** visual distinction (real TotalMix
  shows an implicitly-muted-via-another-channel's-solo button as an
  *outlined* cyan badge, vs. a *solid-filled* badge for an explicit press)
  — noticed in the same live screenshot the M/S colors came from, not
  implemented; would need a new "why is this muted" concept in the data
  model, not just a color swap.
- Didn't re-verify Mute/Solo *active* colors with an actual click in this
  session — interactive `xdotool` clicks on the running mock instance hit
  coordinate drift (clicks landed on the wrong strip/control repeatedly,
  possibly a HiDPI scaling mismatch between the screenshot pixel grid and
  the window's own coordinate space) and cost more time than the check
  was worth; the colors themselves are exercised by nothing but the
  `theme.rs` constant values, so this is a "should look right" not a
  "confirmed via screenshot" for the active states specifically (unlike
  the palette/knob-arc/layout fixes above, all of which *were* screenshot-
  confirmed).
- Matrix view and Quick view weren't given their own pass beyond
  inheriting the shared `strip()`/`fader()`/`knob()` widget changes.

**Collapsed-strip redesign, same session, right after the above.** User
flagged that the collapsed/narrow strip state should also match TotalMix
("regarde les Bus collapsé pour comprendre"). Pixel-measured a live
collapsed strip in the reference (`Screenshot 2026-08-30 124715.png`):
narrow strips there are ~29px against ~75px full strips (a ~0.39 ratio)
— ours were 60/80=0.75, roughly double the real proportion, and the name
was plain horizontal text (fits fine at that width, but not at a
TotalMix-accurate narrower one). Two changes in `widgets/strip.rs`:

- `COLLAPSED_W` 60→44 (keeping the same ~0.39-ish ratio against the now-
  wider `STRIP_W=96`, capped from going narrower only by the existing
  `vu_meter` widget's own fixed width — matching TotalMix's exact ratio
  would need a narrower meter too, not attempted this pass).
- New `RotatedLabel` canvas widget (`rotated_label()`) — the collapsed
  strip's name is now rotated -90° (reads bottom-to-top), matching the
  reference exactly; a plain `text()` doesn't fit at 44px width for
  most channel names and iced's `text()` has no rotation of its own, so
  this is a small dedicated `canvas::Program` using `Frame::rotate`.
  `COLLAPSED_METER_H`'s tuning offset (76→30) was re-derived to
  compensate for the taller name canvas, same "measure a collapsed and
  full strip side by side" methodology as the original number.

Verified live: rotated name renders correctly ("IN3/4" reading bottom-
to-top, centered), collapsed strip visibly narrower and much closer to
the reference proportion, collapsed/full strip heights still closely
matched (a few px off, same tolerance as the pre-existing tuning).
`cargo test -p tuxmix-gui`: 27/27.

**Output-strip height parity — same session, next user catch.** *"je
crois que les Bus ont pas tous la même hauteur et largeur, et écart
entre eux, alors qu'il le faudrait comme sur totalmix nn ?"* Checked all
three (width, height, gap) rather than assuming: width and the gap
between cards were already uniform (`full_width()` returns the same
`STRIP_W` for every channel kind, and every row-building site in
`app.rs` uses the same unscaled `theme::SPACE_MD` — confirmed via pixel
measurement after ruling out a red herring: per-channel-type background
tinting was throwing off a naive brightness-threshold column scan,
making gaps look inconsistent when they weren't). Height was the one
real bug: Output strips skipped the pan-knob row entirely (`if
!matches!(cid, ChannelId::Output(_))`), making them measurably shorter
than Input/Playback strips (~257px vs ~307px in a live screenshot).

The real TotalMix reference (`Screenshot 2026-08-30 124744.png`,
Hardware Outputs section) shows every output strip *with* a knob in
that exact spot, labeled "C" same as an input's pan. Checked what that
knob actually is before assuming it's literally pan: `RmeDevice::
set_pan`'s signature is `(channel, output)` — built for routing a
channel's signal *into* a submix, meaningless for an output channel
itself — and neither `tuxmix-core` nor `tuxmix-usb`'s protocol layer
(`tuxmix-usb/src/{device,protocol}.rs`) has anything resembling an
output-master **balance** register, which is what that knob almost
certainly really is on real hardware. Rather than fake a knob that
would silently do nothing, `widgets/strip.rs::full_strip` now always
pushes the pan row, but for `ChannelId::Output` it's an empty
`iced::widget::Space` sized off the knob's own real footprint
(new `widgets/knob.rs::BOX_SIZE` constant, `pub(crate)`, replacing a
second hand-copied `DIAMETER + MARGIN * 2.0` — exists specifically so
this spacer can't drift out of sync with the real knob's size) — height
parity now, honest about not having a working control there yet.

**Flagged, not implemented — needs a future session on the protocol
level, not just the GUI:** confirm against real hardware / the sibling
`babyface-pro-linux` protocol docs whether the Babyface Pro FS actually
exposes an output-balance register at all before building a real control
for this slot. Until then this stays a blank spacer, not a fake knob.

Verified live (resized to 1200px tall to see all three sections at
once): Hardware Outputs cards now visibly match Input/Playback height
almost exactly (a live-screenshot pixel measurement still showed ~16px
off, but a direct crop-by-crop visual comparison of the two cards'
bottom edges shows them landing at the same relative position — the
16px reading is most likely leftover measurement noise from the same
per-type-tint issue as the gap check above, not a real residual gap).
`cargo build --workspace` + `cargo test -p tuxmix-gui`: 27/27.

**Meter/fader order — corrected back, same session, 4th user catch.**
User: *"c pas plutôt le VU meter qui est à gauche et le fader qui est au
centre... les indicateurs de dB en chiffres sont sur le VU lui-même et
les '-' qui montrent les paliers sont partagés sur le VU et sur le fader
nn ?"* Section 9's own earlier entry ("Fader/meter/ruler reorder") had
put the meter on the *right* of the fader (`[fader][ruler][meter]`) —
wrong, and this time confirmed wrong with hard evidence rather than
another crop guess: an 8x zoom on a live strip (`crop_ruler_full_height.
png`, cropping `(0,180)`-`(45,510)` from `Screenshot 2026-08-30
124744.png`) shows an actual **solid green meter-fill segment** sitting
low in the numbers/ruler column, immediately left of the fader rail —
unambiguous proof that column is a real, live VU meter, not just static
tick labels, and that it sits on the strip's *left*, with the fader to
its right. Exactly matches the user's description: dB numbers printed
on the meter itself, tick dashes shared between the meter and the fader
rail immediately next to it.

Reverted `Fader::layout_x` (`widgets/fader.rs`) to put the meter+ruler
group first (left) and the track second (right) — the mirror image of
what section 9's entry above describes, keeping the *actual* fix from
that entry (centering the whole 3-part group on `bounds_width`, not just
the track alone, so nothing clips regardless of which side anything is
on). `is_over_track`'s boundary check flipped to match (track-clickable
zone is now everything *right* of the meter, not everything left of it).
No `widgets/strip.rs` changes needed — the icon column already sits as
the fader row's trailing sibling, so `[meter][fader][icons]` falls out
automatically once the canvas's own internal order is fixed.

**Not resolved, still an open question**: the reference also shows a
separate thin orange peak-hold line + decaying marker further right,
between the fader and the T/gear/EQ column (visible in the same 8x zoom)
— unclear whether the user considers that part of "the VU meter" too, or
a distinct element. Not implemented either way this pass (already
tracked as the deferred "real TotalMix meter is a monochrome peak-hold
line" item above) — flagging so the *order* fix isn't mistaken for
having also resolved that separate, still-open item.

Verified live: `cargo test -p tuxmix-gui` 27/27, and a screenshot
confirms the meter (with its own dB numbers) now renders left of the
fader on every strip.

**Numbers overlaid on the meter, not beside it — 5th user catch, same
session, immediately after.** User: *"ça a tout décalé sur la droite...
il faudrait que les indications de niveau en dB soient superposés au VU
meter."* Root cause: `draw_meter` drew a slim 7px pill anchored to the
meter rect's own left edge, and `draw_ruler` drew its numbers *beside*
that pill (in the same 30px-wide rect, but their own lane to its right)
— so the combined column was sized for "pill width + gap + label width"
added together, wider than it needed to be, pushing the fader (and
everything after it) further right than the reference. The file's own
`METER_RULER_W` doc comment already *claimed* "the meter is a
translucent wash... ruler ticks drawn on top of it" — true of the
design intent, not of what the code actually did; the pill-beside-
numbers layout must have drifted in at some earlier point without that
comment being corrected.

Fixed both functions to match the comment's own (correct) description:
`draw_meter`'s fill now spans the *entire* rect width (`FILL_ALPHA`
constant, 0.55, added so the translucent color doesn't blot out text
drawn on top of it), `draw_ruler`'s numbers are now centered in that
same rect instead of offset into their own lane, and the tick dash moved
to the rect's own right edge (immediately before the fader track begins
— the "shared between meter and fader" position the user described).
`METER_RULER_W` narrowed 30→22 and `GAP` 6→4 now that the column only
needs to fit a 2-digit label, not a pill-plus-label combination —
`METER_PILL_W` became unused and was deleted rather than left as dead
code.

Verified live: `cargo test -p tuxmix-gui` 27/27, and a screenshot
confirms the "0/6/10/20/40/60" labels now render directly on the green/
red meter fill (legible, verified via a 4x zoom crop) with the fader
sitting noticeably closer to it than before — no more dead gap between
meter and fader.

**Meter narrowed further — 6th user catch, right after.** *"il faut que
les VU soient plus fins (moins larges)."* `METER_RULER_W` 22→17 —
tighter than what the overlay fix alone needed, since that fix's own
value (22) was still sized with some of the old pill-plus-label slack
left in it. 17px is the tightest this goes before actually clipping a
2-digit label at `theme::TEXT_MICRO`, confirmed via a live 5x zoom crop
(digits sit right up against the column edge but stay fully rendered).
Verified: `cargo test -p tuxmix-gui` 27/27.

**Pan knob drag color + Output balance knob — two independent fixes,
same message from the user.**

1. *"Quand on bouge le potard du pan ca s'allume en blanc, ca fait
   qu'on voit plus rien."* `widgets/knob.rs`'s `draw()` flipped the
   face to near-white (`0xf0f1f4`) while dragging, mirroring the fader
   cap's own drag-lit look — but a knob is small enough that the label
   text fills most of the face, so the flip blew the label away
   entirely, and the tick/arc/label all had their own compensating
   "go dark instead" branches bolted on to cope. Replaced with one
   subtle change instead of two opposing ones: face lightens by a small
   blend (`theme::blend(SURFACE, WHITE, 0.18)`, new `DRAG_LIGHTEN`
   constant) rather than flipping to near-white, so it stays dark
   enough that the tick/arc/label never need a dragging-only color of
   their own — removed all three of those branches, one visual state to
   reason about instead of two. `theme::blend` made `pub(crate)` (was
   private) so `widgets/knob.rs` could reuse it instead of duplicating
   the lerp math a second time.

2. *"sur les hardware outputs ya pas le potard de pan sur chaque bus."*
   The empty `Space` spacer added in the height-parity fix above (see
   its own entry) technically matched TotalMix's *height*, but the user
   caught that a blank gap doesn't read as "control not built yet" the
   way an actually-present, visibly-inert knob does. Added
   `Knob::interactive: bool` — when `false`, `update()` returns `None`
   immediately (before the `match`) so all mouse handling is genuinely
   unreachable, `mouse_interaction()` stays `Idle`, and `draw()` dims
   the border/tick/label to 40% alpha and skips the arc entirely (no
   real value to point at). Output strips now get a real `Knob` in that
   slot — same "C" label TotalMix shows, visibly dimmed, `on_change`/
   `on_reset` wired to `unreachable!()` since `interactive: false`
   makes them provably uncallable rather than silent no-ops. The
   now-unused `knob::BOX_SIZE`/`Space` plumbing from the earlier fix was
   removed along with it (`BOX_SIZE` reverted from `pub(crate)` back to
   private — nothing outside `knob.rs` needs it anymore).

Verified live: `cargo test -p tuxmix-gui` 27/27; a drag on AN1/2's pan
knob screenshotted mid-drag shows a legible "C" on a gently-lightened
(not blown-out) face; Hardware Outputs strips all show a dimmed pan
knob in the same spot Input/Playback strips show a working one.

**Collapse icon: chevron → "—".** User: *"pour faire un 'moins'?"*, then
*"c'est pas mieux un '—' plutôt ?"* Swapped the full strip's collapse
button (`widgets/strip.rs`'s `icon_col`) from "▼" to a plain hyphen
first, then to an em dash (U+2014) at the user's follow-up — reads as a
deliberate "minus" mark rather than a stray dash-shaped artifact at this
size. `TEXT_SM` (was `TEXT_MICRO`, too small for either glyph to read
clearly). Pairs with the collapsed strip's own "+" expand button
(unchanged). Verified via a 6x zoom crop each time. `cargo test
-p tuxmix-gui` 27/27.

**Collapse button relocated next to the dB readout, plain "-" again —
and a real layout bug found and fixed along the way.** User: *"il
faudrait remettre - plutôt... et le mettre plus bas, à côté du display
de la valeur du volume en dB comme sur les screens de totalmix."*
Confirmed against the reference: TotalMix shows "0.0" then a small "-"
pill on the *same row*, near the bottom of the strip — not up in the
gear/EQ icon column. Moved the collapse trigger out of `icon_col`
entirely into its own row alongside the dB display (`row![db_display,
Space::Fill, collapse_btn]`), reverted the glyph back to a plain "-"
now that it's sitting in the exact spot/shape the reference uses.

**Real bug, not just a relocation:** the first version of this button
used `centered_label("-", ...)` with no explicit `.width()`/`.height()`
pinned on the surrounding `button()` — every *other* `centered_label`
button in this file pins that down explicitly, because `centered_label`
wraps its text in `container(...).center(Length::Fill)`, which needs a
bounded parent to fill. Without that pin, the result wasn't just a
mis-sized button — **every strip in the app rendered as a blank card**
(section headers still showed, since those are separate from strip
building). Confirmed live in both QUICK and MIXER views, and bisected
by temporarily short-circuiting pieces of the new row until the exact
line was isolated, rather than guessing. Fixed by using a plain
`text("-")` instead of `centered_label` for this specific button — it's
meant to size to its own content anyway (pill-style, like the route/
LOOP buttons elsewhere on the strip), not fill a fixed box the way the
gear/EQ icon buttons do.

**Lesson**: `centered_label` is only safe inside a button/container that
gives it an explicit, bounded size — every future use needs that same
pin, or reach for a plain `text()` instead when the button should size
to its content. Verified: `cargo test -p tuxmix-gui` 27/27, and a fresh
screenshot after the fix shows strips rendering normally again with the
"-" sitting next to "0.0 dB" at the bottom of each strip.

---

## 10. Right sidebar ("control strip") — 2026-09-03

TotalMix's right-hand panel: device chip, Undo/Redo, an M/S/F row, "FX
show %", an Options panel (routing/meters/show/2-row/solo-mode), a
Snapshots panel (Mix 1-8 + store), a Groups panel (4 mute/solo/fader
groups + edit/clear), a Mixer Layout panel (Layout 1-6 + store,
all/submix). Full scope, functions included — user chose "tout d'un
coup" over phasing it, after confirming: (1) the M/S/F row and several
Options rows would be visual skeleton only (no real TotalMix semantics
to confirm from a static screenshot — see below), (2) two Options rows
("routing: free", "meters: RMS") don't map to this app's architecture
at all and should render disabled/dimmed, same treatment as the Output
strip's inert balance knob.

New files: `tuxmix-gui/src/sidebar.rs` (all sidebar rendering — mirrors
the existing `matrix.rs` precedent of a self-contained view region
getting its own module) and `tuxmix-gui/src/layouts.rs` (Mixer Layout
persistence, mirroring `scenes.rs`'s own file-per-slot pattern —
`~/.local/share/tuxmix/layouts/Layout N.json`, plain serde since
`ChannelId` already derives `Serialize`/`Deserialize`; needed adding
`serde_json` as a direct `tuxmix-gui` dependency, previously only
pulled in transitively through `tuxmix-core`). `app.rs::view()` changed
from `column![top, content]` to `column![top, row![content, sidebar::
sidebar(state)]]`.

**What's real, and what it's built on:**
- **Snapshots** — genuinely new work only in the sense of wiring; the
  underlying mechanism is the *existing* scene save/load
  (`scenes.rs::save_scene_file`/`load_scene_file`,
  `RmeDevice::capture_scene`/`apply_scene`), just 8 fixed slot names
  ("Mix 1".."Mix 8") instead of a user-typed one.
- **Undo/Redo** — `Vec<Scene>` stacks (`TuxMix::undo_stack`/
  `redo_stack`, capped at 50), using the same capture/apply pair.
  `is_undoable()` (a blacklist of UI-only messages, defaulting to
  "capture" for anything not explicitly excluded) decides which
  messages push a pre-action snapshot at the very top of `update()`,
  before the big `match`. Fader drags snapshot once, on `FaderPressed`
  (not on every `VolumeChanged` during the drag — that would flood the
  stack with one entry per mouse-move). Known, accepted gap: knob-driven
  values (pan, gain, trim, EQ params) have no distinct "drag start"
  message the way faders do, so a knob drag pushes one snapshot per
  tick rather than one per gesture. Also known: `Scene` only captures
  *device* state, so sidebar-only concepts (group membership, which
  snapshot/layout slot is selected, panel collapse) aren't touched by
  undo/redo at all — `is_undoable` excludes their messages not just to
  reduce noise but because capturing around them would be a no-op.
- **Groups** — `sidebar::Group { members: Vec<ChannelId>, mute_linked,
  solo_linked, fader_linked }`, 4 of them in `TuxMix::groups`.
  Membership assignment reuses the *existing* multi-select
  (`state.selected`, Ctrl/Shift-click) as the picker: `edit` toggles
  `group_editing`, then clicking a group's `M1`/`S1`/`1` cell assigns
  the current selection as that group's members and engages that link.
  Propagation is a **generalization of code that already existed** —
  `apply_grouped_volume`/`apply_grouped_pan` already did relative-delta
  multi-select propagation before this session; the Mute/Solo handlers
  did absolute (non-delta) multi-select propagation. New shared
  `propagation_set(state, cid, link)` unions "multi-selected, if `cid`
  is part of one" with "member of a `link`-engaged group `cid` belongs
  to," and all four handlers (Mute, Solo, `apply_grouped_volume`,
  `apply_grouped_pan`) now go through it — meaning fader-linked groups
  get the *real* relative-delta behavior TotalMix has, not a simplified
  lockstep fallback (the original implementation plan assumed lockstep
  would be needed; the existing selection-delta code turned out to be
  directly reusable once generalized).
- **Mixer Layout** — a layout is just `TuxMix::collapsed` (which strips
  are collapsed; no strip-reordering exists yet, so that's the whole
  story), persisted via `layouts.rs`. Recall clears `collapse_anim`
  too, so a stale in-flight collapse/expand animation can't fight the
  bulk-applied target state.

**What's deliberately skeleton** (per the user's own framing —
built to match the reference shape, not wired to guessed-at behavior):
the M/S/F row (no `on_press` at all — S shown lit, M cyan-bordered,
purely as a static visual matching the reference's own screenshot, not
read from any state), "FX show %" (fully static — no FX/reverb/echo
processing exists in this app), and three Options-panel pairs (`show:
names/trim`, `2 row/2 row in`, `solo/pfl mode: excl. solo/live`, plus
the Mixer Layout panel's own `all/submix` row) — these DO respond to
clicks (toggle which side is highlighted, `sidebar::SkeletonPairs`) so
they don't read as dead buttons, they just don't change anything else.
`solo/pfl mode` specifically was scoped to skeleton rather than real
because real non-exclusive solo would mean touching `usb.rs`'s
hardcoded exclusive-solo logic (`usb.rs:603-618`) — the kind of
hardware-adjacent backend change this project has repeatedly flagged
and scoped into its own dedicated session rather than folding into a
GUI pass (see the 2026-08-29 stereo-link-migration entry above for the
same caution applied to a different feature).

**A real bug in the first draft, caught by inspection before it shipped
further**: `segmented_row` (the shared 2-way toggle row builder) always
derives the right side's highlight as `!left_active` — correct for a
genuine exclusive pair, wrong for "both sides are simply off," which is
what the `meters: post fx / RMS` row needed (both disabled). Passing
`left_active: false` there lit "RMS" instead of leaving both dim,
confirmed via a live screenshot before being fixed with a dedicated
`disabled_segmented_row` (40% text alpha, no `on_press` on either
side, same dim treatment as the Output strip's inert balance knob).

**Verification — mixed, and said so rather than overclaiming.**
`cargo build --workspace` + `cargo test --workspace`: 133/133 (10 new
tests added directly against `app::update()` — undo/redo revert+reapply
+ redo-cleared-on-new-action, UI-only messages don't grow the undo
stack, group mute/solo/fader propagation including a negative case
(solo-link doesn't leak into mute), group clear, layout recall).
Visually confirmed live (`--mock`, resized to see all four panels at
once) that the whole sidebar renders correctly against the reference
crop, and that the `disabled_segmented_row` fix actually looks dim in
practice, not just in code.

**Whole-sidebar collapse ("rabattable"), same session, right after.**
User wanted the sidebar itself foldable — distinct from the existing
per-panel "−" headers (`sidebar_panels_open`), which only collapse
content *within* an expanded sidebar. New `TuxMix::sidebar_open: bool`
(default `true`) + `Message::ToggleSidebar`. `sidebar::sidebar()`
branches early: collapsed renders just a `COLLAPSED_WIDTH` (22px) rail
with a "‹" expand button — not nothing, which would leave no way back
— expanded gets a "›" collapse button as its own first row, above the
device chip. Verified live both ways: screenshotted the real default
(expanded, confirmed the new "›" button renders), then temporarily
flipped the `new()` default to `false` in code, rebuilt, and
screenshotted again to confirm the collapsed rail actually reclaims the
width and shows the "‹" button correctly — reverted the default
immediately after. Also added a unit test (`toggle_sidebar_flips_and_
is_not_undoable`) following this session's own pivot to testing
`update()` directly rather than fighting synthetic clicks (see below).
`cargo test --workspace`: 134/134.

**Top bar decluttered — the device chip was a straight duplicate, the
rest wasn't.** User asked whether the top bar's own elements were still
needed now that the sidebar exists. Checked rather than assuming either
"yes, keep everything" or "no, remove everything": the top bar has 5
things — the device identity chip, the Quick/Mixer/Matrix view tabs, and
a "session" toolbar (Scene name/Save/load picker, Submix picker, Clock
Source button). Of those, only the **device chip** is a genuine, content-
identical duplicate of the sidebar's own `sidebar::device_chip`. The
other four have no sidebar equivalent at all: the view tabs are core
navigation; the Submix picker selects which of the 6 output buses is
being edited (the sidebar's `routing: submix` in Options is a fixed
architecture readout, not a per-bus selector); the Clock Source button
opens the existing clock/sample-rate/SPDIF drawer; and Scene Save/Load
is a genuinely different feature from the new Snapshots panel — arbitrary
user-named presets vs. TotalMix's own fixed 8 numbered slots, not a
subset of it.

Removed just the device chip from `top_bar` (`app.rs`) — but it carried
information (the connected/simulated status dot + label) the sidebar's
own chip didn't have yet, so that moved over too rather than being lost:
`sidebar::device_chip` now shows the same "● Babyface Pro FS (mock) /
Simulated" the top bar used to. `theme::TEXT_LG` (the "device name gets
slight emphasis" type-scale tier) moved with it rather than being
deleted as dead code — same size, same purpose, just relocated, keeping
the 6-tier type scale's own design intact (`GUI-NOTES.md`'s type-scale
section is explicit that it's meant to be a complete, named vocabulary,
not a formula with gaps in it). Net effect: the top bar is shorter (one
fewer chip, and it was already the one flagged for overflow risk on
narrow windows back in the 2026-08-29 audit — this should help that too,
not just declutter), the sidebar's own device chip does slightly more
work, nothing was actually lost.

Verified: `cargo build --workspace` + `cargo test --workspace` 134/134,
and a live screenshot confirms the top bar now reads as title + tabs +
session tools with no duplicate identity chip, while the sidebar's own
chip correctly shows the status dot/label it absorbed.

**Two follow-up fixes, same day: device-chip overflow, and Scene
save/load merged into Snapshots.** (1) The sidebar's device chip
(absorbed from the top bar, above) overflowed its box in `--mock`: user
caught it directly ("le nom de la babyface ça dépasse de la case et
c'est sale comme ça"). Root cause: `model_name()` grows a `" (mock)"`
suffix for scene-compatibility checks (`mock.rs`), and that suffix
alone pushed `"Babyface Pro FS (mock)"` past the ~150px text budget
left in the 210px-wide sidebar chip once the status dot, dropdown
arrow, padding, and spacing are accounted for. Fixed by stripping the
suffix for *display only* (`strip_suffix(" (mock)")` in
`sidebar::device_chip` — `model_name()` itself is untouched, still used
verbatim for scene compatibility) since the status line right below
already says "Simulated," making the suffix redundant there anyway; the
name text also got an explicit `.width(Length::Fill)` so it's bounded
instead of able to overflow again for any future longer device name.
(2) User: now that Snapshots lives in the sidebar, fold the top bar's
separate Scene save/load chip into it — same underlying mechanism
(`save_scene_file`/`load_scene_file`), no reason for two homes. Checked
first that this was actually true rather than assumed: confirmed
`Message::SnapshotClicked`/`SnapshotStore` already call the exact same
`load_scene_file`/`save_scene_file` functions as the top bar's
`SceneLoad`/`SceneSave`, just with fixed `"Mix N"` names instead of a
free-form one — a real, not superficial, duplication. Moved the
text_input/Save button/load `pick_list` (unchanged `Message` variants:
`SceneNameChanged`, `SceneSave`, `SceneLoad`) into
`sidebar::snapshots_panel`, stacked vertically below the Mix 1-8 slots
behind a thin divider (the sidebar's 210px width doesn't fit the top
bar's horizontal layout), and removed them from `app.rs::top_bar`'s
`session` chip entirely, leaving just Submix + Clock Source there.
Verified: 134/134 tests, live screenshot confirms the top bar now only
has Submix/Clock, the sidebar's Snapshots panel has both the Mix 1-8
slots and the free-named save/load row beneath them, and the device
chip reads "Babyface Pro FS" cleanly with no overflow.

**Free-named Scene save/load removed outright, same day.** Right after
merging it into the Snapshots panel (above), user: "name save et load
tu peux supprimer, vu qu'on en a plus besoin" — the 8 fixed Mix 1-8
slots cover the need, no reason to keep the free-form variant at all.
Removed for real, not just hidden: the text_input/save/load block out
of `sidebar::snapshots_panel`; `Message::SceneNameChanged`/`SceneSave`/
`SceneLoad` and their `update()` handlers and `is_undoable()` blacklist
entries; `TuxMix::scene_name`/`scene_list` fields and their `new()`
init; the now-fully-unused `scenes::list_scene_files()` function
(deleted, not just unreferenced — nothing else called it, checked with
a repo-wide grep first). `load_scene_file`/`save_scene_file` stay —
Mix 1-8 still use them directly, untouched. 134/134 tests, clean
build (no dead-code warnings from the removal), live screenshot
confirms the Snapshots panel now ends cleanly at "store" after Mix 8.

**Device-chip font size, and M/S in the M/S/F row made real.** Two more
user catches, same day. (1) The device name at `TEXT_LG` (15px) still
read as too big for the sidebar's dense 210px column, even after the
overflow fix above — dropped to `TEXT_MD` (13px, the "default body
text" tier), which left `TEXT_LG` with zero remaining callers, so it
was deleted outright rather than kept as unused dead weight (per this
project's own stance on backwards-compat cruft). (2) The M/S/F row
(`sidebar::msf_row`) was deliberately built as pure visual skeleton
back when the sidebar first shipped — no `on_press` at all, since a
static reference screenshot couldn't confirm TotalMix's real semantics
there. User asked to make M and S real; asked back what exactly they
should do (real semantics still unconfirmed) rather than guess, via
`AskUserQuestion` — user picked "Mute/Solo globaux": **M** toggles
every channel's mute at once (all Input/Playback/Output channels;
lights when all are already muted, so a second press undoes the
first), **S** clears every currently-active solo (one-shot, not a
toggle — there's no sensible "solo everything," lights when at least
one channel is soloed so there's visibly something to clear). **F**
stays skeleton — nothing in this app corresponds to TotalMix's own
third button there either.

New `Message::GlobalMuteToggle`/`GlobalSoloClear`, handled in
`app.rs::update()` by iterating `all_channel_ids()` (a new helper:
every `ChannelId::Input`/`Playback`/`Output` the device currently
exposes) through the *existing* `set_channel_mute`/`set_channel_solo`
choke points — no new device-layer code, this is pure orchestration
like Groups was. Two new `pub(crate)` helpers
(`all_channels_muted`/`any_channel_soloed`, reading straight off
`RmeDevice::inputs()/playbacks()/outputs()`'s own `mute`/`solo` fields)
drive both the toggle direction and the buttons' lit state, shared
between `app.rs` (the handler) and `sidebar.rs` (the button styling).
Neither new message is undo-blacklisted — same as plain `Mute`/`Solo`,
a global mute/solo-clear is exactly as undoable as a per-channel one.
3 new unit tests (`update()`-driven, continuing this session's
click-testing-limitation workaround): mute-toggle mutes everything then
unmutes on a second press, solo-clear clears solos without touching
mute state, and mute-toggle is undoable. 137/137 tests, live screenshot
confirms the smaller device name and the row's baseline (unlit)
rendering; the toggle/clear behavior itself is verified by the new
unit tests rather than a synthetic click, per this sandbox's own
documented click-testing limitation.

**Global "S" made a real toggle, not one-way.** Right after M/S
shipped, user: "le S doit pouvoir toggle les anciens S nn ?" — a fair
catch, since `GlobalMuteToggle` was already symmetric (mute-all, then
unmute-all on a second press) but `GlobalSoloClear` was one-way
(clear, with no way back short of re-soloing channels by hand).
Renamed `Message::GlobalSoloClear` → `GlobalSoloToggle` and gave it a
memory: new `TuxMix::last_cleared_solos: Vec<ChannelId>`. First press
with something soloed clears every soloed channel and remembers
exactly which ones; a press with *nothing* currently soloed restores
solo on that remembered set instead (real "solo everything" has no
sane meaning, so restoring the prior set is the toggle's other
direction rather than that). Soloing something new between two presses
correctly replaces the remembered set rather than resurrecting a stale
one, since the handler always re-derives "what's soloed right now"
before deciding which branch to take. Caught and fixed a test bug
while adding coverage for this: a test using `Input(8)` alone expected
only that channel in the cleared set, but every input pair is
hardware-linked by default in mock (`mock.rs`'s own
`test_input_pair_linked_by_default_and_moves_both_channels`), so
soloing channel 8 mirrors onto channel 9 too — the test's assumption
was wrong, not the code; fixed the assertion to check set membership
instead of an exact channel list, and documented the linked-pair
behavior inline so it doesn't get re-discovered as a false bug later.
2 new tests (restore-after-clear, replace-stale-set-on-new-solo) plus
the existing clear test renamed to match — 139/139 total. Live
screenshot confirms baseline rendering unaffected (both buttons unlit
with nothing soloed, matching before).

**Global "M" reworked to match "S"'s precise-restore behavior, not just
blanket unmute-all.** Right after the S-toggle fix, user: "le M
centralisé ça marche comme ça, de la même manière que le S" — the
original `GlobalMuteToggle` was a toggle already, but a blunt one:
second press unmuted *everything*, even a channel that had been muted
independently (by hand, before the global press) and had nothing to do
with the global action. Rewrote it to the same shape as
`GlobalSoloToggle`: first press mutes only the channels that *aren't*
already muted and remembers exactly which ones it touched
(`TuxMix::last_globally_muted`); second press (once everything reads
muted) restores mute-off on exactly that remembered set, leaving any
independently-pre-muted channel untouched. New test
`global_mute_toggle_leaves_a_pre_muted_channel_muted_after_restoring`
covers the actual distinction (pre-mute one linked pair, global-mute
toggle twice, assert that pair is still muted while everything else
came back off) — the earlier, less precise test
(`..._mutes_everything_then_unmutes_on_second_press`) still passes
unchanged since it starts from a fully-unmuted baseline, where the old
and new logic happen to produce the same result. 140/140 tests, live
screenshot confirms baseline rendering unaffected.

**Matrix view: channel-type color coding, a bounded consistency pass —
not a full redesign.** After the hardware-validation audit wrapped up,
picked this up as the other flagged-but-deferred item ("Matrix view
and Quick view only inherited the shared widget changes, no dedicated
pass" — [[project_gui_tui_audit_2026_08]]). Checked Quick view first:
turned out to need nothing — it already renders via `strip::strip(...)`
directly (`app.rs::quick_view`, just scaled up 2x), so it inherited the
full redesign automatically, no separate work required. Matrix view
(`matrix.rs`) was the real gap: every column header rendered in flat
`TEXT_SEC` gray regardless of channel type, unlike every other view in
the app (Mixer strips, the sidebar) which color-codes by type. No
TotalMix reference screenshot of the actual Matrix/routing-grid view
exists in `Totalmix UI Screenshots/` (checked a sample before starting,
per this session's own hard rule about not inventing layout facts) —
so this was deliberately scoped to *safe, already-verified* colors
(`app::type_tag()` for inputs, `app::PB_TAG` for playbacks — the exact
same functions/constants Mixer strips already use) rather than a
structural redesign guessing at proportions with nothing to check
against. Verified live via `--mock`: AN1/AN2 read cyan (Mic), IN3/IN4
and AS1/AS2 orange (Instrument/Line), ADAT3-8 purple, every "PCM ..."
playback column teal — a real, immediately visible readability win
over the previous flat gray. 140/140 tests, clean build.

**Matrix view rebuilt from scratch against a real TotalMix screenshot,
superseding the "bounded consistency pass" above the same day.** User
shared an actual TotalMix Matrix-window screenshot right after that
color-coding pass shipped — revealing the whole structure was wrong,
not just the colors: real TotalMix has individual output *channels* as
columns (Out 1..12), individual input/playback *channels* as rows
(In 1..12, Pb 1..12), bare numeric dB cells (blank when unrouted, not
a fader), two-level headers (per-channel + per-pair-group), and a
separate master "Outputs" row — our version had output *pairs* as rows
and a tiny fader per cell, backwards on both axes. Confirmed before
rebuilding: no such reference screenshot existed in `Totalmix UI
Screenshots/` when the color-coding pass shipped (checked two samples,
found none), so that pass's own "no reference, so don't guess
structure" reasoning was sound at the time — this wasn't a mistake to
avoid, just new information arriving mid-stream.

Asked the user how far to take it (`AskUserQuestion`) rather than
assume "faithful" meant redoing the interaction model too — picked the
full rebuild. Rewrote `matrix.rs` entirely:
- **Row groups**: the two Mic-type inputs (AN1, AN2) are each their own
  physical jack, shown solo; every other input type is an inherent
  hardware pair (Instrument, Line, ADAT), shown as a 2-row group —
  derived from `ChannelType`, not guessed, and reuses `pair_bus_label`
  (made `pub(crate)`) for the combined name so the wording matches what
  Mixer strips already call a linked pair.
- **Playback groups**: one pair per hardware output bus, labeled with
  this app's own verified `OUT_LABELS` — not TotalMix's own
  instance-specific renaming in the reference screenshot (its "Main" is
  that user's own label for what this hardware's ALSA/USB surface
  actually calls "PH3/4"; copying it would have overridden a
  previously-verified fact with an unrelated screenshot's cosmetic
  choice).
- **Cells**: bare "-9.8"-style text, blank when the crosspoint is at
  (near-)zero, matching TotalMix leaving unrouted cells empty rather
  than printing "-inf" everywhere.
- **L/R column placement**: our own per-channel model stores one
  volume *per output pair*, not per individual physical output channel
  — matching real hardware, which (per `babyface.rs`'s own crosspoint
  comment) has no way to route a non-mono source differently into a
  pair's left vs right side at all. Each row's value is shown in
  whichever single column (left/right) that channel naturally feeds
  (`idx % 2`, the same convention `set_channel_volume` already uses
  everywhere), leaving the pair's other column blank for that row. The
  4 true-mono sources (AN1-4) are the one real exception — genuinely
  independent L/R — decoded on the fly from the same (volume, pan) pair
  the fader/pan knob already edit, so both of their columns show real,
  distinct values.
- **Master "Outputs" row**: each output channel's own master volume,
  visually separated by a divider from the crosspoint grid above it.

**Known, explicitly-flagged gap, not silently dropped**: cells are
display-only this pass — no click-to-type or scroll-to-adjust.
TotalMix's own editing gesture on a grid this dense is a real,
separate interaction-design question that deserves its own pass with
the user's input on the exact mechanism, not a rushed guess bolted onto
a structural rebuild already this large.

**Verified twice, mock and real hardware** (the card was still
connected from this same day's earlier audit): mock showed every
cell densely populated at first, which read as a possible bug — turned
out to be `InputChannel::new`'s own default (`volumes: vec![1.0;
outputs]`, i.e. mock ships with every crosspoint at unity into every
output, not a realistic sparse routing table). Re-checked against the
real card to be sure: AN1/AN2 correctly showed blank at the AN1/2 pair
(matching their `-∞ dB` Quick-view reading) while other pairs showed
real, pre-existing `-6.0` routes — confirming the density was real
hardware state, not a rendering bug, and that the natural-side L/R
column split was behaving correctly (alternating filled/blank columns
per row, as designed). One screenshot-capture mistake along the way:
`spectacle -a` grabbed an unrelated browser window instead of TuxMix
once (focus race after `windowactivate`) — caught immediately, the
image was discarded unread rather than analyzed, and the capture was
redone after explicitly confirming `getactivewindow` matched TuxMix
first. 140/140 tests, clean build (first try — the type-based
row-grouping and `pair_bus_label` reuse meant no surprises).

**Matrix cells made click-drag interactive, closing the gap flagged in
the rebuild above — same day.** User picked "clic-glisser vertical,
comme un mini-fader caché" over scroll-to-adjust (scroll was already
spoken for by the grid's own horizontal/vertical scrolling — a
scroll-on-cell gesture would fight it). Extended `widgets/fader.rs`'s
`Fader<Message>` with two new fields rather than a parallel widget:
`show_track: bool` (skips drawing the groove/cap entirely when
`false`, while every press/drag/release/wheel/double-click handler in
`canvas::Program::update` stays fully live — the whole canvas still
counts as "over track") and `label: Option<String>` (drawn centered via
the same `Frame::fill_text` the ruler ticks already use). Only one
other call site existed (`strip.rs`'s own strip fader) so extending the
struct instead of adding a new type was cheap — updated it with
`show_track: true, label: None` and nothing else about it changed.

Matrix cells now build a `Fader` with `show_track: false` and their own
bare dB text as `label` — a fully working drag hidden behind the
number, not a second visible control fighting the numeric display for
attention. Kept this view's own pre-existing convention (`on_press`/
`on_drag` write `Message::VolumeChanged` directly, not `strip.rs`'s
`FaderPressed`) — Matrix edits were never undo-tracked before this
pass either, and changing that is a separate decision from wiring up
the drag itself. Only the column a channel can actually reach is
interactive (`idx % 2`, matching `set_channel_volume`'s own left/right
convention); the other column of that pair — genuinely unreachable on
this hardware for every source except the 4 true-mono AN1-4 inputs —
stays a plain static cell, not a dead-but-visually-identical fader.
The master "Outputs" row got the same treatment for free (each output
channel's real master volume).

Caught and fixed one visual inconsistency before calling this done: the
Fader canvas sizes itself to the *track's* own width (`TRACK_W`, 26px)
when `show_meter` is `false`, narrower than a plain cell's 40px slot —
left bare, interactive cells looked like a different, unbordered kind
of thing next to their bordered static neighbors. Wrapped the fader in
a `container` matching `cell()`'s own size/border/background exactly.
**Known residual gap**: the click/drag *hit target* is still only the
inner ~26px canvas, not the full 40px visual cell (the container's
extra padding is click-through) — a real, if minor, target-size
mismatch, not chosen deliberately; left for a future pass rather than
reaching for a `Stack` to fix it (this codebase's own documented
click-handling scar tissue). 140/140 tests, clean build (first try),
live screenshot confirms consistent cell chrome. Not click-verified
for the actual drag gesture itself — this sandbox's synthetic-click
limitation applies here same as everywhere else — but the underlying
`Fader` press/drag/release mechanics are the exact same, already-proven
code path every strip fader already uses, just re-skinned.

**Real VU meter signal added for the ALSA backend — AN1/AN2 only, a
genuine correctness constraint, not a scope shortcut.** User: "il
faudrait qu'on mette le signal dans les VU meter". Before this, the
ALSA/kernel-driver backend's meters were 100% fake — the driver
exposes no meter register at all, unlike the USB backend, which
computes real peaks host-side from the raw isochronous stream it
already owns (`tuxmix-usb`'s `input_peaks()`). Confirmed the user
wanted the real thing (not a fake animation) and that it was worth
doing now despite being a genuinely different kind of work (a
real-time audio capture thread, not ALSA mixer controls).

Went to implement full input coverage and hit a real, not just
big-scope, blocker: the capture-channel-to-physical-input mapping is
**disputed inside the sibling repo's own documentation**.
`tools/usbdump/PROTOCOL.md` (the original RE) says capture words 0/1 =
AN1/2 (confirmed by an actual mic test), but words 2/3 are described
as *contextual* — IN3/4 at idle, or the PH3/4 output bus's loopback
signal if that's engaged, not a fixed channel at all — and the same
document explicitly flags an earlier "words 2/3 = AN3/AN4" reading as
wrong. `KERNEL-DRIVER.md` (newer, 2026-08-25) describes the tail end
differently again (words 12/13 = a fixed-gain playback tap, not
ADAT7/8). Asked the user how to handle this rather than guess — a
wrong mapping would show a level moving on the *wrong* input, worse
than no level at all. Scoped down to AN1/AN2 only, the one mapping
both documents and a live test agree on.

**Implementation**: new `tuxmix-core/src/capture_meter.rs`
(`alsa`-feature-gated) opens the card's own ALSA capture PCM directly
— just 2 channels requested, which (per the kernel driver's own
`babyface_capture_copy`, walking its channel map for `i in
0..channels_requested`) lands exactly on device words 0/1 without
needing to open all 12 and discard the rest. Runs its own thread
(`snd_pcm_readi` blocks) reading small 256-frame periods so `Drop` can
stop it quickly rather than being stuck in a long blocking read;
peaks accumulate in a `Mutex<[f32; 2]>`, drained (and reset) once per
poll, mirroring `tuxmix-usb`'s own draining convention exactly.
`BabyfacePro` now overrides `RmeDevice::meters()` (previously unused
by this backend — the default `None`), returning a full-length vec
with only indices 0/1 ever populated. `DeviceHandle::has_input_meters`
(app.rs) gained a per-channel sibling, `has_input_meter(idx)` — the
Mixer view's per-strip `meter_available` now checks the specific
channel instead of one blanket flag, so IN3/4 onward correctly keep
showing the dashed "N/A" meter treatment instead of a fabricated
silent reading.

**Verified against the real card** (still connected): launched
without `--mock`, confirmed no startup errors, and a zoomed screenshot
crop shows AN1/2's meter column rendering as a real (solid black,
currently-silent) meter while IN3/4's right next to it still shows the
dashed N/A pattern — the per-channel gate working exactly as designed.
Also confirmed the new background thread shuts down cleanly: killed
the process and checked `ps` immediately after — no lingering/zombie
process, meaning the `Drop`-triggered stop-flag-then-join shutdown
path works, not just the happy path. 140/140 tests, clean build (both
`cargo build -p tuxmix-core --features alsa` alone and the full
workspace) on the first real attempt — every `alsa` crate API guess
(`HwParams::any`, `Format::S32LE`, `PCM::state()`/`recover()`) landed
right without needing a second pass.

**Known, deliberately scoped gap**: IN3/4, AS1/2, ADAT3-8, and every
playback channel still read "N/A" on the ALSA backend — not a bug,
a direct consequence of the disputed/contextual mapping above.
Resolving it for real would need either empirical testing with an
actual signal plugged into each of those inputs, or reconciling
`PROTOCOL.md` against `KERNEL-DRIVER.md` at the driver-source level —
both out of scope for this pass, flagged rather than guessed past.

**Real crash in the new capture thread, caught by the user within
seconds of running it for real.** `thread '<unnamed>' panicked at
.../num/mod.rs:426:5: attempt to negate with overflow`. Root cause:
`capture_meter.rs`'s peak loop called `buf[...].abs()` on raw `i32`
audio samples — `i32::abs()` panics in debug builds on exactly
`i32::MIN`, a real bit pattern that showed up in practice within
seconds of real capture, not a theoretical edge case. Fixed with
`unsigned_abs()` (returns `u32`, has no such hole — `i32::MIN`'s
magnitude fits fine, just not as a same-signed `i32`), reworking the
per-buffer peak accumulator to `u32` throughout rather than patching
just the one call. While investigating, also added a real, separate
fix rather than declaring the panic fix sufficient and moving on:
`pcm.prepare()` alone doesn't start a capture stream collecting
samples the way a playback stream auto-starts on its first write —
without an explicit `pcm.start()`, `readi` was erroring immediately
every call (empty ring buffer), `recover()` "succeeding" (a legitimate
no-op from ALSA's own point of view), and the loop spinning as fast as
the CPU allowed instead of ever actually blocking on real audio.
Caught independently by checking CPU usage after the panic fix rather
than assuming a clean exit meant a clean fix — found ~70% CPU, then
ruled out it being new/caused by this session's own work at all by
comparing against `--mock` (which never touches ALSA capture and
showed the *same* ~70%, confirming this specific CPU level predates
today entirely and is a separate, out-of-scope characteristic — not
conflated with the actual bug being fixed). Verified the real fix by
running the app against the real card, unattended, for ~55 seconds
straight (`ps`-checked at 25s and 55s) — no panic, log stayed empty
both times. 140/140 tests, clean build.

**Real, pre-existing meter/ruler scale bug found by the user's own
sanity check ("t'es sûr que l'échelle est bonne ?"), same day.** Good
question to ask right after real signal started flowing for the first
time — this bug existed in `draw_meter`/`draw_ruler` before today, but
every real backend showed "N/A" dashes until this session's AN1/AN2
work, so it never had real data to expose it. Root cause:
`draw_meter`'s fill height was `track.height * l` — `l` being raw
*linear* amplitude (1.0 = 0 dBFS) — while `draw_ruler`'s dB gridlines
(and the fader track right next to the meter) are positioned on the
*tapered* curve (`db_to_t`/`vol_to_t`, the same power curve the fader
itself travels). Checked the actual numbers rather than eyeballing it:
a genuine -6 dBFS signal (linear amplitude 0.5) should top out right at
the ruler's "-6" gridline, but the old linear fill only reached 50% up
the column while `vol_to_t(0.5)` ≈ 0.63 — the fill was landing well
*below* where its own label said it was, and the gap widens further
down the scale (the whole point of a tapered curve is compressing the
quiet end). Fixed by filling to `track.height * vol_to_t(l)` instead —
now the meter and the ruler agree by construction, not by coincidence.

Also found, in the same investigation, a smaller **second** issue:
`draw_meter`'s fill track reserves a strip at the top for the clip LED
(`CLIP_H`+`CLIP_GAP`) that `draw_ruler`'s tick positions never
accounted for — a few-px offset stacked on top of the curve mismatch.
Extracted the inset into a named `meter_track()` helper and *tried*
applying it to both functions — but that would have fixed the meter/
ruler pairing at the cost of breaking the ruler/fader-cap pairing
instead (`draw_track`'s own cap position uses the full, uninset
canvas height, and was already correctly aligned with the ruler before
touching any of this). Reverted `draw_ruler` back to measuring off the
full rect deliberately, leaving that small, pre-existing gap alone
rather than trading one alignment bug for another under time pressure
— documented in both functions' own comments so it doesn't read as an
oversight later.

New test `meter_fill_curve_is_tapered_not_linear` locks in the fix
(asserts the tapered `t` differs from raw linear 0.5 by a real margin,
and that it matches `db_to_t` fed the same dB value the ruler itself
uses) — a case chosen specifically to fail against the old linear
code, not just any assertion. 141/141 tests, clean build, and a mock
screenshot with a genuinely non-zero meter reading (mock's own
synthetic level) confirms the fill still renders sensibly (green
mid-scale, red near the top with the clip LED lit) after the curve
change — not just "compiles."

**EQ frequency knobs were linear over a 3-decade range — same class of
bug as the meter, found by continuing the same audit lens.** After
fixing the meter's scale, kept looking for the same "wrong curve"
pattern elsewhere rather than treating that as a one-off. `Knob`'s
`value_to_t`/`t_to_value` are purely linear (`(value-lo)/(hi-lo)`) —
correct for pan/gain/Q/pitch/width/trim (already-additive quantities:
dB, percent, a linear position), but the EQ Band Freq and Low Cut Freq
knobs pass `range: (20.0, 20_000.0)` — three decades — through that
same linear map. A linear knob over 20 Hz-20 kHz squeezes the entire
bass/low-mid range (20 Hz-2 kHz, most of what EQ work actually targets)
into a sliver of the drag travel while the top octave alone eats
roughly a quarter of it — the standard, well-known reason every real
piece of audio software maps frequency controls logarithmically, never
linearly.

Added `Knob::log_scale: bool` rather than a second widget or a
special-cased branch scattered through call sites: `value_to_t`/
`t_to_value` switch to natural-log interpolation when set (falls back
to linear if `range`'s low end isn't strictly positive, rather than
taking `ln` of a non-positive number). Only 2 of the 8 `Knob{}`
call sites got `log_scale: true` (EQ Band Freq, Low Cut Freq); the
other 6 (pitch, width, gain, trim, Q, band gain, plus the 2 pan knobs
in `strip.rs` missed on the first pass and caught by the compiler)
got `log_scale: false` — correctly staying exactly as they were.
3 new tests, including one specifically checking the log-mapped
midpoint lands at the *geometric* mean (~632 Hz for 20-20,000 Hz), not
the arithmetic one (10,010 Hz) — the exact number that would still
pass if the fix silently degraded back to linear. 144/144 tests, clean
build (first try once the 2 missed `strip.rs` call sites were added —
the compiler caught both immediately via the new required field, no
silent gap possible), live screenshot confirms no crash/regression.

**Closed the Matrix cell hit-target gap flagged as a known residual —
same day, after the first commit landed.** `Fader`'s canvas used to be
hardcoded to `TRACK_W` (26px) whenever `show_meter: false`, even though
the Matrix view wraps it in a 40px `cell()`-sized container for visual
consistency — meaning about a third of every cell's visible width
looked clickable but wasn't. Added `Fader::compact_width: f32`
(ignored when `show_meter: true`, so `strip.rs`'s own fader — the only
other caller — just passes a dummy `0.0`), and `matrix.rs` now passes
`CELL_W` directly: the canvas *is* the full cell, so the wrapping
container's width/height are redundant with the canvas's own now, kept
only for the shared border/background styling. 144/144 tests, clean
build, live screenshot shows no visual regression (identical rendering
to before — this was a hit-testing fix, not a visual one).

**Hardware Output VU meters were reading wrong/zero for almost every
pair — found by continuing the same index-confusion audit that caught
the Matrix rebuild's own L/R placement earlier, applied to code that
predates this whole session.** `DeviceHandle::output_meters()`
power-sums every input/playback's contribution into each output, but
indexed `ch.volumes` — sized per *output pair* (`output_pair_count()`,
6 entries) — with the loop variable `o`, which actually ranges over
individual output *channels* (`outputs().len()`, 12, 2 per pair). Real
effect: every odd channel read a different, wrong pair's crosspoint
value instead of its own pair's; every channel whose pair index was
>= 6 (i.e. every pair past the first three: ADAT5/6, ADAT7/8, and
whichever pair happened to sit there) silently read a permanent zero
via `.get()`'s `None` fallback, regardless of actual routed audio.
Fixed by indexing with `o / 2` (the pair), the same convention
`set_channel_volume` already uses everywhere else for the identical
individual-channel-to-pair relationship.

Pulled the computation out into a free, pure `power_sum_output_meters`
function rather than leaving it as a `DeviceHandle` method, specifically
so it's testable with deterministic meter values — the mock backend's
own `input_meter`/`playback_meter` are randomized (`rand::thread_rng`),
so a test going through `DeviceHandle` itself could never assert an
exact number for either the bug or the fix. 2 new tests: one routes a
known signal into pair 2 only and confirms pairs 0/1 read exactly zero
while pair 2's two channels both read the full level; one specifically
exercises the old bug's out-of-bounds failure mode (a pair index past
a volumes array's own length) and confirms it stays a silent zero, not
a panic. 146/146 tests, clean build. Not screenshot-confirmed for the
live Hardware Outputs meters specifically — scrolling the Mixer view
down to them hit the same synthetic-input unreliability as every other
interaction test this session — but the fix targets the exact,
deterministically-reproduced bug scenario the two new tests assert
against, which is the stronger claim of the two anyway.

**Same Hardware Output meter bug existed in the TUI too — checked
rather than assumed, since it's a separate crate with its own
independent copy of the logic.** `tuxmix-tui` has its own `DeviceHandle`
(not shared with `tuxmix-gui`'s), and its `output_meters()` was a
byte-for-byte duplicate of the same buggy indexing. Ported the
identical fix: extracted `power_sum_output_meters` as a free, pure
function here too, fixed `o` → `o / 2`, added the same 2 deterministic
tests. 148/148 workspace tests. Not live-verified in an actual
terminal — this sandbox's bash tool has no real TTY, and a ratatui app
fails at terminal setup without one (`Os { code: 6, ... "No such
device or address" }`, an environment limitation, not a code issue);
the logic-level tests are the verification here, same reasoning as
every other test-only-verified fix this session.

**TUI Matrix view rebuilt to match the GUI's own axes — checked and
confirmed, not assumed, then rebuilt with the user's explicit "full
rebuild" choice.** `tuxmix-tui`'s `render_matrix` had the identical
backwards structure the GUI had before its own fix: rows were output
*pairs* (`OUT_LABELS`, 6), columns were input/playback channels
(capped at 8 for terminal width). Asked how to handle it given the
terminal-width constraint (full rebuild vs. a pair-granularity
compromise vs. leave it) — user picked the full rebuild, same axes as
real TotalMix and the GUI's own rebuild: individual output *channels*
as columns, individual input/playback *channels* as rows, with the
same row-grouping rule (`matrix_input_groups`, Mic-type solo/everything
else paired) and pair-label combining (`matrix_pair_label`) ported
over as this crate's own copies (no shared code between the GUI and
TUI binaries).

Since a terminal is rarely wide enough for all 12 output columns at
once (unlike the GUI's scrollable canvas), added real horizontal
scrolling: `matrix_col: usize` state in `run()`'s loop, Left/Right
guarded to only scroll while the Matrix view is showing (`KeyCode::Left
if show_matrix`, ahead of the plain `KeyCode::Left` arm that still
drives normal channel navigation everywhere else — first-match-wins
guard ordering, not a full key-interception branch, so nothing else
about existing key handling changed). `render_matrix` clamps the
requested offset against the *actual* visible column count computed
from the real terminal width at render time, so a stale/over-scrolled
offset can never go out of range even across a live window resize.
One deliberate simplification vs. the GUI, called out in the function's
own doc comment: every row (including the 4 true-mono AN1-4 inputs that
the GUI can show independent L/R values for via pan) shows on a single
fixed side per pair — not worth a second numeric column pair's width
cost for a first terminal implementation.

3 new tests (row-grouping split, pair-label combining, cell-text
formatting) plus the existing output-meter tests already covered the
shared `o/2` pair-index math. Live-verified for real this time — this
sandbox has real terminal emulators (alacritty) on the X display, so
launched one, confirmed the full un-scrolled grid renders correctly
(all 12 columns, correct row groups, correct alternating natural-side
cells), resized the window narrower, confirmed it correctly reports
"cols 1-9 of 12," then scrolled right 3 times and confirmed "cols
4-12 of 12" with the right cells shifting — the actual interactive
behavior working live, not just unit-tested logic. 151/151 tests,
clean build.

**Same log-scale class of bug, third occurrence, in the TUI's discrete
frequency stepping.** After fixing the GUI knob's drag curve, checked
whether the TUI's keyboard-driven EQ freq/low-cut editor (`Left`/
`Right`/`PgUp`/`PgDn` in `adjust_eq_field`) had the analogous issue —
it did, just manifesting differently: a *fixed additive* step (10 Hz
fine, 100 Hz coarse) across the same 20 Hz-20 kHz range meant reaching
20 kHz from 20 Hz took ~1800 fine presses or 180 coarse ones, while the
low end (where musically-relevant precision actually matters) already
had plenty of resolution at that same fixed step. Same root cause as
the knob (linear stepping over a multiplicative range), opposite
symptom (too slow instead of too imprecise) — replaced with a new
`step_freq_hz` helper that steps *multiplicatively* (~5%/press fine,
~25% coarse), covering the same 3 decades in roughly 140/30 presses
instead, and feeling like a constant amount of adjustment at any point
in the range rather than speeding up disproportionately as the value
grows.

3 new tests (near-constant percentage move at both ends of the range,
reaches 20 kHz in well under 100 coarse presses, stays in bounds).
Live-verified in a real alacritty window (now an established technique
for this crate, not a one-off): opened the EQ editor for AN1, pressed
Right 5 times on Band 1 Freq, watched 1000 Hz → 1277 Hz — matches
1000×1.05⁵≈1276 (small per-step rounding, expected) almost exactly,
confirming the live behavior matches the tested math, not just that
both happen to compile. 154/154 tests, clean build.

**Sensitivity wired to a mechanism that was already fully implemented
and hardware-verified — nothing new needed on the protocol side.**
Before assuming a real capture campaign was needed to finish Clock
Source/Sensitivity, checked `babyface-pro-linux`'s own `PROTOCOL.md`
for whether either had ever actually been captured — both had, and
more thoroughly than expected. Sensitivity (Instr 3/4's +4dBu/-10dBV
switch) turned out to be the *exact same hardware feature* as
`RmeDevice::set_ref_level` (Instr 3/4's ref-level switch,
`cap_reflevel2.pcap`, "hardware-verified live" per the doc), already
fully implemented in `tuxmix-usb`/`usb.rs` — just never wired to the
`set_sensitivity` entry point the GUI/TUI actually call, which still
returned its old "not mapped in the USB protocol yet" error even
though the underlying write had been real for weeks. Fixed by mapping
`Sensitivity::Plus4dBu`/`Minus10dBV` to the existing `REF_PLUS_4DBU`/
`REF_MINUS_10DBV` codes and delegating to `set_ref_level` directly —
no new protocol work, a pure wiring fix.

Clock Source turned out to need *nothing at all*: `usb.rs::
set_clock_source` was already fully implemented, using the same
`cap_clk.pcap`-derived keepalive-bit mechanism (hardware-verified via
`clktest.c` back in August), with a passing test
(`settings_word_matches_captured_keepalives`) already covering the
exact byte values. The earlier claim that Clock Source was "missing"
was accurate only for the ALSA/kernel-driver backend's own control
surface (confirmed via `amixer` — still true, that gap is real and
would need kernel-driver C work, a separate task); the USB/libusb
backend already has it, this session just hadn't checked that backend
specifically before drawing a general conclusion.

Updated the GUI's Sensitivity-knob dimming: it was gated on
`is_mock()` (a stand-in for "no real backend supports this yet," true
when it was written), now on a proper `DeviceHandle::
has_sensitivity_control()` that's `true` for Mock *and* USB, `false`
only for the still-genuinely-unsupported ALSA backend — the TUI's own
sensitivity toggle needed no code change at all, since it never had a
dimmed/disabled visual language to begin with; it was silently
failing before and now silently succeeds on the USB backend, same
code path either way. 154/154 tests, clean build. Not live-verified
against real hardware this pass — the card is currently running via
the kernel driver, and the USB/libusb backend can't open the device
while that driver owns it (by design); switching to test this
specific 6-line mapping fix would mean unloading the kernel module, a
more disruptive step than this change's risk warrants given the
underlying write mechanism was already hardware-verified independently.

**Not click-verified.** Attempting to actually click Snapshot/Group/
Layout controls this session hit something worse than the earlier
"coordinate drift" — `xdotool getactivewindow` after a synthetic click
returned a *different* window ID than TuxMix's, even immediately after
`windowactivate --sync` and a direct click on the title bar. This
sandbox appears to mix native-Wayland windows (this coding session's own
terminal) with XWayland ones (the iced-rendered TuxMix window) in a way
`xdotool` — an X11 tool — can observe and nominally "activate" but not
reliably move real input *focus* to, at the Wayland-compositor level
synthetic clicks actually need. This is a deeper, session-spanning
environment limitation, not a per-click coordinate offset (see the
`project_bus_redesign_2026_09` memory for the earlier, milder version of
this problem and how this session's understanding of it evolved).

**How this gap was actually closed**: rather than keep fighting
synthetic clicks, added unit tests that call `app::update()` directly
with the real `Message` variants a click would send — same code path,
no window/focus/coordinate cooperation needed from the environment.
This is *more* reliable than a screenshot-based click test would have
been even in a cooperative environment (a passing visual check can't
distinguish "worked for the right reason" from "worked by coincidence"
the way an assertion on the resulting `TuxMix` state can) — worth
reaching for this approach earlier next time a GUI feature's *logic*
(not its rendering) needs verifying, rather than defaulting to
screenshot-driven interaction tests.

**Trim ("T") button added on Hardware Inputs.** User: *"au-dessus du
bouton de la roue crantée faudrait mettre un bouton T pour le talkback,
sur les hardware inputs et les software playback comme sur totalmix."*
Checked before building anything: `RmeDevice::set_trim(idx, db)` already
exists (`tuxmix-core/src/device.rs`, -65..+6 dB on the master curve) —
this is **Trim**, not Talkback (no talkback concept exists anywhere in
the codebase); flagged the naming mismatch to the user, who confirmed
Trim was right. Also checked scope before implementing on both sections
as asked: `usb.rs`'s `set_trim` calls `input_source(idx)`, which only
maps to real hardware input sources (An1-4/As12/Adat34/56/78) — Software
Playback indices don't resolve through it at all. Asked the user how to
handle that gap rather than shipping something broken on Playback; they
picked "Trim, Hardware Inputs only."

**Implementation** (mirrors the existing Gain/EQ wiring shape exactly,
nothing novel):
- `tuxmix-core`: `InputChannel.trim: f32` (new field, `#[serde(default)]`
  for old scene JSON) — `set_trim` had no getter and no backend persisted
  the value anywhere, unlike every other setter in the trait (`set_gain`
  et al always write into the channel struct); added that write to
  `usb.rs`'s real override and to a new `mock.rs` override (the trait
  default is a silent no-op with nowhere to store a value, which would've
  made the "T" button look broken in `--mock`).
- `widgets/strip.rs`: `FlyoutKind::Trim`, `StripParams::has_trim`/`trim`,
  a "T" trigger in `icon_col` *above* the gear icon (per the user's own
  placement), gated on `has_trim` — `true` unconditionally for every
  `ChannelId::Input`, unlike `has_gain` (Mic/Instrument only) or `has_eq`
  (analog-only), matching the reference showing "T" on ADAT/AS strips
  too. Lights up whenever trim ≠ 0, same "shows live state" idea as the
  EQ trigger's `eq_open || eq_enabled`.
- `app.rs`: `Message::TrimChanged`/`TrimReset`, a `trim_popover` (one
  `Knob`, -65..+6 dB, `arc_from_center: false` since the range isn't
  centered around 0 the way Pan's is) — same "pushes the row" shape as
  `settings_popover`/`eq_popover`, wired into `mixer_view`'s *input* loop
  only (not the Playback loop, per the scope decision above), sized off
  the existing `strip::FLYOUT_W` (Route's own width) rather than a new
  constant, since it's a single knob with nothing wider to fit.

**Verified**: `cargo build --workspace` + `cargo test --workspace`
124/124. Visually confirmed live (MIXER view, `--mock`) that "T" renders
above the gear icon on every Hardware Input (analog and ADAT/AS alike)
and is absent from every Software Playback and Hardware Output strip —
exactly the intended gating. **Not confirmed**: actually clicking "T" to
open the flyout — every synthetic click attempt this session landed on
the wrong element (see the `project_bus_redesign_2026_09` memory's note
on `xdotool`/screenshot coordinate drift in this sandbox, which got
worse, not better, over the session). The flyout wiring itself is a
direct, mechanical copy of the already-proven Settings/EQ pattern (same
message, same `state.flyout_open` check, same row-push mechanism) —
high confidence, just not click-verified end-to-end. Worth a real click
test next time this file is open in an environment where that works.

**Fader centering + dB readout moved under the fader, not the meter.**
User: *"le fader sur le bus [devrait être] parfaitement centré (...un
tout petit peu à droite)... et la valeur des dB [devrait être] centrée
juste en dessous du fader et non du VU meter."* Measured before fixing:
a live screenshot pixel measurement put the fader cap's center 1.5-2px
right of the card's true center — small, but real and in the direction
the user described, not imagined.

**Root cause**: `Fader::layout_x` centered the `[meter+track]` *group*
on the canvas's own `bounds_width` — correct for keeping the meter
on-screen (that was the original fix this replaced), but the canvas
itself is narrower than the strip's full content width, because
`icon_col` (gear/EQ/T) is a `row!` sibling that shrinks the canvas's
`Length::Fill` allocation without the canvas ever knowing that
sibling exists. Self-centering on a narrower-than-true-content canvas,
plus the track sitting toward the *right* side of the meter+track group
rather than at the group's own center, compounded into a small but
real rightward bias.

**Fix**: new `Fader::reserved_right` field — the width `icon_col`
claims outside the canvas — lets `layout_x` center the *track itself*
(not the group) on `(bounds_width + reserved_right) / 2`, which reduces
to exactly `content_width / 2` (the strip's true center) once
`reserved_right` correctly accounts for everything the canvas doesn't
know about. The meter is then positioned `GAP` to the track's left,
with the same negative-x safety clamp the original fix had (shift the
whole group right instead of letting the meter clip). Matrix view's
compact fader (no meter, no `icon_col` sibling) passes `reserved_right:
0.0` and takes an early-return branch that reproduces its old
self-centering exactly — unaffected by this change.

For the dB readout: it previously sat in `row![db_display, Space::Fill,
collapse_btn]` — left-anchored under roughly where the *meter* is, not
the fader (TotalMix-accurate positionally, per the actual reference
screenshot, but not what the user asked for this time). Restructured
into a **symmetric** row instead: a blank spacer on the left exactly
`collapse_btn`'s own width (now pinned via `.width(ICON_BTN_W * scale)`,
previously content-sized), `db_display` centered in a `Length::Fill`
container between the two, `collapse_btn` on the right. Because both
bookends are equal width with equal gaps, the centered container's
midpoint is the row's true center — which, now that the fader row above
is centered the same way, is exactly where the fader sits. No `Stack`
needed (deliberately avoided — this file has direct prior experience
with `Stack`-based overlays breaking click handling; a plain symmetric
`row!` sidesteps that risk entirely).

Verified: `cargo build --workspace` + `cargo test --workspace` 124/124,
and a live screenshot pixel measurement confirms the fader cap now sits
within 0.5px of the card's true center (down from 1.5-2px), with "0.0
dB" visibly centered directly under the fader's own rail in a 4x zoom
crop.

**Hardware Inputs gap uniformity — real bug this time, not the
tint-color measurement illusion from earlier in this file.** User: *"que
sur les hardware input... y'ait le même écart partout que sur les
hardware outputs et software playbacks."* Section 9's own "width/height/
gap" entry above had checked this once already and concluded the gaps
*were* uniform, blaming an apparent inconsistency on per-channel-type
background tinting confusing a crude brightness-threshold pixel scan —
that conclusion was **wrong**. Re-measured properly this time (a hard
darkness threshold, `sum(rgb) < 100`, cleanly separating card from page
background regardless of tint) and found a real, reproducible pattern:
gaps between AN1/2↔IN3/4↔AS1/2↔ADAT3/4 (each a different `ChannelType`)
measured 13px, while ADAT3/4↔ADAT5/6↔ADAT7/8 (same type, ADAT) measured
6px.

**Root cause**: `mixer_view`'s Hardware Inputs loop (`app.rs`) inserted
a 1px `rule::vertical` divider every time `channel_type` changed between
consecutive pairs — a deliberate, intentional feature (a group divider
between Mic/Instrument/Line/ADAT sections). Since the row's own
`.spacing(theme::SPACE_MD)` applies on *both* sides of that extra
child, a divided gap came out to `SPACE_MD + 1 + SPACE_MD` (~13px)
against a plain `SPACE_MD` (~6px) everywhere else — exactly the
measured pattern. Software Playback's own loop already had a comment
noting it deliberately has "no channel-type dividers to worry about",
confirming Input was the *only* section with this asymmetry, not an
illusion affecting all three equally.

Removed the divider insertion entirely — every gap on every row (all
three sections) is now the same plain `SPACE_MD`. Verified via the same
darkness-threshold pixel scan: all 5 Hardware Input gaps now measure
exactly 6px, matching Playback/Outputs precisely (down from 13px at 3 of
the 5 transitions). `cargo build --workspace` + `cargo test --workspace`
124/124.

**Lesson, worth remembering**: the first "gap consistency" investigation
in this file used a weak measurement method, got a plausible-sounding
"it's just measurement noise" answer, and moved on — the user re-raised
the exact same complaint later in the session and this time it turned
out to be a real bug the weak method had actually missed. A pixel
measurement that produces a boring/expected answer isn't automatically
trustworthy just because it's convenient; the method matters as much as
the result, especially for anything below ~15px where a threshold choice
can flip the conclusion.

**Collapse animation growing the strip's height mid-transition.** User:
*"pendant que ça se minimise le bus augmente de hauteur un peu et après
ça se remet normal."* Root cause: `strip()`'s dispatch renders
`full_strip` (not `collapsed_strip`) for the *entire* ~160ms width
animation (`CollapseAnim::is_settling`), only switching to
`collapsed_strip` once it's fully settled — so every `text()` inside
`full_strip` is exposed to the animated, shrinking width the whole time.
iced's `text()` defaults to word-wrapping (`Wrapping::Word`); once the
animated width dropped below what a long channel name ("Instr. 3/4",
"ADAT7/8") needs on one line, it wrapped to two lines, growing that
row's — and so the whole card's — height for the remainder of the
animation, snapping back once the width animation finished and the name
had room again (or once it switched to `collapsed_strip`, which uses the
rotated-canvas name label, immune to this since it's not a `text()`
widget at all).

**Fix**: added `.wrapping(advanced_text::Wrapping::None)` to every plain
`text()` in `full_strip`/`header_row` that sits in the animated-width
flow — the header name/type-tag, the dB readout, the route button's bus
label, "LOOP". None of these are ever *meant* to wrap (every label in
this card is a deliberate single line), so this is a correctness fix for
the whole card's animation, not a narrow patch for one label — text that
doesn't fit now gets clipped by the card's own existing `.clip(true)`
instead of wrapping and pushing the card taller.

**Not directly observed mid-animation** — the transition is ~160ms,
well under what a screenshot round-trip in this sandbox can reliably
catch (and this session's own `xdotool` coordinate-drift problems, see
the Trim-button entry above, make even *triggering* the collapse
reliably via synthetic click a coin flip). Verified instead: resting-
state rendering (both expanded and, by inspection, the always-single-
line label set) is visually unchanged after the fix — `cargo build
--workspace` + `cargo test --workspace` 124/124. The fix itself is a
direct, well-understood application of iced's own documented `Wrapping`
API to the exact failure mode described, not a guess.

**VU meter scale, Sensitivity wiring, and three newly-wired proprietary-mode
features: CUE, EQ-for-Record, Optical Out format (2026-09-06).** User
caught the first bug directly by asking a pointed question: "t'es sur
que l'echelle est bonne ? entre le VU de TuxMix et le VU de la carte
physique." Root cause: `fader.rs::draw_meter`'s fill height used the raw
linear amplitude while `draw_ruler`'s tick marks (and the fader cap
itself) already went through the tapered `db_to_t` curve — so the meter
fill and its own ruler disagreed on where a given dB value should sit.
Fixed by routing the fill height through the same `vol_to_t(l)` curve
(new `meter_track(r, scale)` helper, used only by `draw_meter` —
deliberately *not* shared with `draw_ruler`, since that would have
reintroduced a separate, already-correct ruler/fader-cap alignment).
New test `meter_fill_curve_is_tapered_not_linear`.

That question led to a wider check: whether the remaining known gaps
(Clock Source, Sensitivity, plus two settings found not yet wired up)
actually needed new Windows USBPcap captures, or were already solved in
the sibling repo's `PROTOCOL.md` and just never connected to Rust code.
Checked before concluding either way (per explicit instruction — "Oui
tu peux faire ca" — to verify against existing documentation first
rather than guess): **Clock Source** was already fully implemented.
**Sensitivity** (`set_ref_level`, Instr 3/4 +4dBu/-10dBV/Boost) existed
in `tuxmix-usb` and was fully hardware-verified, but `usb.rs`'s
`set_sensitivity` was still a stub returning `Err("not mapped")` —
wired it to the real mechanism. Along the way, found two more
already-solved-but-never-wired settings sharing the same "keepalive
settings-word" register as Clock Source (`0x10` VendorRequest,
`0x05CF`, sent ~every 3s): **EQ for Record** (bit 6) and **Optical Out
format, ADAT vs SPDIF** (bit 10) — plus a genuinely new feature, **CUE**
(monitor-bus preview, `cap_cue.pcap`), which mutes every playback pair's
low-map crosspoint into the AN1/2 monitor bus except the one being
cued, reusing the existing `set_low_map_volume` mechanism already
proven for Mute/Solo. User approved all three via `AskUserQuestion`:
"Oui, les 3 (CUE + EQ-for-Record + format optique)."

Implementation ran the full stack: `tuxmix-usb::device::BabyfaceUsb`
gained tracked `clock_optical`/`eq_record`/`spdif_out` bool fields and a
private `send_settings_word()` so the three flags share one register
without one write stomping the others (the settings-word is a single
`u16`, not three independent writes — composing it from all three
tracked flags on every send was the actual fix, not just adding new
setter functions). `tuxmix-core::device::RmeDevice` gained
`set_eq_for_record`/`set_optical_out_format`/`set_cue` trait methods
(default `Err`, per this project's own established pattern for
backend-optional controls) plus two new `DeviceSettings` fields; all
four struct-literal construction sites (`scene.rs`, `usb.rs`, `mock.rs`,
`babyface.rs`) needed updating for the new fields to compile.
`OutputChannel` gained a non-persisted `cue: bool`
(`#[serde(default, skip_serializing)]`, mirroring `pitch_percent`'s own
momentary-state pattern) with CUE's exclusivity (only one output pair
can be cued at a time — there's one physical AN1/2 monitor bus to
share) implemented identically in `usb.rs` and `mock.rs`.

**Caught mid-implementation, not after**: adding the 3 new trait
methods with default bodies made `cargo build --workspace` pass clean
with zero errors — but that was misleading. `DeviceHandle` (the enum
`app.rs`/`tuxmix-tui/main.rs` use to abstract over backends via a
`delegate!` macro) needed *explicit* `delegate!` arms added for each of
the 3 new methods in both files, or calls would silently hit the
trait's own always-erroring default instead of ever reaching the real
backend — reasoned through the macro's mechanics before trusting the
clean build, rather than assuming green-build meant fully-wired.

GUI: new `Message::EqForRecordChanged`/`OpticalOutFormatChanged`/
`CueChanged`, two new toggle rows in the existing `device_panel()`
Global row ("EQ for Record"/"Opt Out: SPDIF", same `spdif_toggle`
closure as MS Proc/AN1>2/Input Link), and a new "C" button in
`full_strip`'s M/S row (Output strips only, `has_cue`/`cue` added to
`StripParams`) styled and positioned like Mute/Solo. TUI: three new
single-key bindings (`f`=EQ-for-Record, `w`=Opt-Out-SPDIF, both global;
`c`=CUE, Output section only, mirroring the existing `l`=Loopback
pattern), a `[CUE]` tag alongside the existing `[M]`/`[S]`/`[LOOP]` tags
on Output strips, and both new toggles added to the Overview status
line and the header key-legend. Live-verified all three in a real
`alacritty` window (`xdotool key` delivery is reliable there, unlike
synthetic clicks against this app's XWayland window — see the
click-testing-limits note elsewhere in this file): `f`/`w` flip
`EQRec:`/`SPDIFOut:` in the Overview line, `c` adds `[CUE]` to the
selected Output strip. The GUI's own device-panel toggle rows were
*not* independently click-verified — the triggering "Internal ▾" button
click didn't register in two attempts (same known sandbox limitation),
so that half relied on direct code review (the two new rows are
structurally identical to the already-shipped, already-working MS
Proc/AN1>2/Input Link rows in the same `row!`) plus a new `update()`-
driven unit test, `eq_for_record_and_optical_out_format_toggles_reach_device_settings`,
which specifically asserts that toggling EQ-for-Record does *not* stomp
the SPDIF-out flag — the regression the shared-settings-word register
made possible before `send_settings_word()` was written to compose all
three flags together.

**Flagged, deliberately not fixed**: `apply_scene` (`usb.rs`) only
updates `self.settings = scene.settings.clone()` for global settings —
it never re-issues the actual USB writes for `clock_source`, `an12`,
`ms_proc`, and now `eq_for_record`/`optical_out_spdif` too, so loading a
saved Scene doesn't actually restore these to real hardware, only to
the in-memory model. Pre-existing, affects many fields beyond the two
added here — out of scope for this pass, worth its own session.

`cargo build --workspace` clean, `cargo test --workspace` 158/158 (60
GUI + 51 core + 17 TUI + 30 tuxmix-usb, 8 ignored live-hardware-only).

**Follow-up, same day: user asked for an honest status check, then
"corrige tt ça" — fixed every gap that check surfaced.** Asked whether
the GUI "marche nickel," and answered with 4 concrete gaps rather than
a blanket yes: the `apply_scene` hardware-write gap (flagged just
above), the device-panel toggle rows not yet click-verified, several
deliberately-inert sidebar rows, and the meter/ruler clip-LED
misalignment left as a known residual. User: fix all of it. Tackled in
order of real impact:

1. **`apply_scene`'s hardware-write gap, both backends.** `usb.rs`:
   added unconditional re-application of `an12`/`ms_proc` (previously
   only re-applied when `true`, never explicitly turned back *off*) plus
   `eq_for_record`/`optical_out_spdif` (previously not re-applied at
   all) and `clock_source` (guarded by a new pure
   `should_reapply_clock_source(clock_source, clock_sources)` — skips
   an empty string or a value the scene's own `clock_sources` list
   doesn't recognize, so a stale/legacy field can't abort the whole
   scene load via `set_clock_source`'s own validation `Err`). New unit
   test for the guard function. `babyface.rs` (the ALSA/kernel-driver
   backend) turned out to have a *much* bigger version of the same gap,
   found by actually reading its `apply_scene` rather than assuming it
   mirrored `usb.rs`'s: it never re-applied mute, solo, loopback,
   ms_proc, an12, width, fx_send, or clock_source at all — only volume.
   Added all of them, but selectively: only the controls this backend
   actually implements (mute/solo/loopback/ms_proc/an12/width/fx_send/
   clock_source), explicitly skipping phase/ref_level/stereo_split/
   eq_for_record/optical_out_spdif/cue, none of which `babyface.rs`
   overrides (they'd hit the trait's always-`Err` default and abort the
   load over an unsupported field). `ms_proc`/`an12`/`width`/`fx_send`/
   `clock_source` are called best-effort (`let _ =`, not `?`) since
   `clock_source` specifically is a real ALSA control only under
   Class-Compliant mode, not the currently-loaded proprietary driver
   (see [[project_proprietary_usb_status]]) — a missing optional control
   shouldn't abort mute/solo/loopback, which always work. **Live-
   verified against the real, currently-connected Babyface Pro FS**, not
   just unit-tested: new `live_hardware_apply_scene_reissues_mute_and_
   loopback_not_just_the_model` (capture a baseline scene, apply a
   modified one with AN2 muted + loopback engaged, re-open a second
   handle to force a fresh ALSA read and confirm both actually changed
   on hardware, then re-apply the baseline and confirm loopback reads
   back off again) — ran with `--ignored` against the real card, passed,
   confirmed via `amixer` afterward that the card was left exactly as
   found. `usb.rs`'s half of the fix could *not* be live-tested the same
   way (the USB backend can't open the device while the kernel driver
   owns it, unloading that module felt disproportionate) — relies on
   code review plus the new unit test instead, consistent with how this
   gap's discovery was originally documented.

2. **Device-panel toggle rows, actually click-verified this time.**
   The earlier "Internal ▾" click to open `device_panel()` kept missing
   (same XWayland limitation as every other late-session flyout this
   project has hit). Sidestepped it the way this session's own sidebar
   work already established: temporarily flip `show_device_panel`'s
   `new()` default to `true`, rebuild, screenshot, revert — no click
   needed at all. Confirmed live: "EQ for Record" and "Opt Out: SPDIF"
   render correctly, identical styling to the already-shipped MS Proc/
   AN 1>2/Input Link rows beside them.

3. **Meter/ruler clip-LED misalignment, actually fixed, not left as a
   residual this time.** The earlier VU-meter fix (routing the fill
   through `vol_to_t`) left a smaller *second* mismatch: `draw_meter`'s
   fill/background measured against `meter_track(r)`, inset from the
   top by the clip-LED's reserved strip, while `draw_ruler`'s ticks (and
   the fader cap) measured against the full, uninset `r` — so a fill's
   top landed a few px below its own ruler gridline at every dB value,
   worst at 0 dBFS. Previously left alone specifically to avoid
   reinsetting the ruler and breaking its own correct pairing with the
   fader cap. Realized the fix everyone had been avoiding was simpler
   than assumed: make the *meter* match the ruler/cap's full-`r`
   coordinate system instead (the direction not previously tried) —
   removed the inset entirely (`meter_track` helper and `CLIP_GAP`
   constant both deleted, now dead once nothing insets against them),
   background box and fill now both measured against the same full `r`
   the ruler already uses. The clip LED still draws last and fully
   opaque, so at high levels the fill now correctly reaches up to meet
   it (matching how a real peak meter's top segment sits right under
   the clip lamp) instead of stopping short in a reserved gap. Live-
   verified via a zoomed `--mock` screenshot crop: the clip LED sits
   flush against the background box's rounded top corner with no seam,
   and the "0" gridline lines up with the fader cap's own rest position
   as expected. 159/159 workspace tests (158 plus the new
   `should_reapply_clock_source` test; the new babyface.rs
   live-hardware test is `#[ignore]`-gated like its siblings and ran
   separately with `--ignored` against the real card, passing).

4. **Two genuinely dead-code warnings, same day.** User ran `cargo run
   -p tuxmix-gui` directly (not through this session's usual `cargo
   build --workspace`) and saw a `never used` warning for
   `DeviceHandle::input_meter`/`playback_meter` (singular, per-channel)
   in `app.rs`. Checked before deleting: only the plural
   `input_meters()`/`playback_meters()` (whole-Vec) are ever called from
   anywhere in the GUI — the singular pair had no callers at all.
   `tuxmix-tui` had the identical pair plus a third,
   `outputs_one_per_pair` — checked that one too before removing it:
   its own doc comment claimed it was "needed to map an output-strip
   channel index back to the submix pair index `set_loopback` expects,"
   but the TUI's actual loopback key handler (`l`) computes that pair
   via `PairItem` matching instead, never calling it — the comment
   described an intended design that GUI (where the equivalent method
   *is* real code, called at `app.rs:1586`/`1735`) uses but this file
   never ended up wiring up. Deleted all 3 dead methods outright rather
   than leaving them for "later" — `cargo build -p tuxmix-gui` now
   matches the user's own report with zero warnings, 159/159 tests
   unaffected (none of the deleted methods had a test depending on
   them).

**Two more real bugs in `usb.rs`, found while porting Ref Level/Phase
to the sibling kernel driver (`babyface-pro-linux`) and fixed here too,
same day.** Implementing these in C required re-deriving exactly how
the shared preamp byte and the phase negation compose, which surfaced
that `usb.rs`'s own versions of both were subtly wrong:

- **`set_ref_level` forced 48V on for AN1/AN2 as a side effect of
  changing Instr 3/4's ref level.** It sent the raw captured bytes from
  `cap_reflevel2.pcap` verbatim (`0x000F`/`0x0003`/`0x0003`) — bytes
  that happened to have both mics' 48V bits baked in during that
  specific capture, since the preamp state is one shared byte (48V bits
  0-1, ref-level bits 2-3, PAD bits 4-5). Fixed by composing from
  `preamp_bits()` (the ALREADY-correct helper `write_preamp_state`/
  `write_preamp_block` use for 48V/PAD writes) `|` a new
  `ref_level_bits()`/`ref_level_commit()` pair — pulled out as a pure
  `ref_level_contribution(code)` free function specifically so the
  composition is unit-testable without a real USB handle. Also found
  the mirror-image gap while fixing this: `write_preamp_state`/
  `write_preamp_block` themselves hardcoded `PREAMP_BASE` (+4dBu) as
  the ref-level contribution on every 48V/PAD toggle, regardless of
  what the user had actually selected — silently resetting Instr 3/4
  back to +4dBu. Both directions fixed together (they share the same
  two helpers now, so they can't disagree again). Also mirrors
  `set_ref_level`'s write across both Instrument channels — there's
  only one physical switch for the pair, not independent per-channel
  bits, so `inputs[2].ref_level`/`inputs[3].ref_level` could previously
  disagree depending on which index was last written through.
- **`set_phase` double-negated for the AN1/2 destination specifically,
  silently no-opping phase invert there.** `protocol::set_phase`
  already negates its input internally (hardware-verified,
  `cap_ctrl.pcap`) — but `usb.rs`'s own caller pre-negated the value
  *before* passing it in for the `out==An12` case only, canceling the
  inversion right where it matters most (every other output correctly
  negated once, via a separate manual write path). Also fixed the
  known composition gap flagged when this was first shipped: a fader
  move on a phase-inverted analog input silently undid the inversion,
  since `set_volume` had no idea phase existed. Both fixed together via
  one new shared helper, `write_l_with_phase(out, src, raw, phase)` —
  `set_phase` calls it once per output when toggled, and `set_volume`
  now calls it as a follow-up correction after its normal mono write,
  whenever the channel being moved is phase-inverted. The exact same
  bug classes (byte composition, hot-path phase-awareness) were just
  fixed in the C kernel driver's own Ref Level/Phase controls — this
  is the reverse-flow benefit of implementing the same protocol twice.
- Not live-hardware-tested this round (the USB backend can't open the
  device while the kernel driver holds it) — relies on the existing
  `protocol::set_phase`/`ref_level_writes_labeled_pairs` tests (both
  already hardware-verified) plus a new
  `ref_level_contribution_never_bakes_in_an_unrelated_48v_bit` test and
  careful code review, consistent with how this session's other
  USB-backend-only changes were verified. 160/160 workspace tests.

**`babyface.rs` (ALSA/kernel-driver backend) wired to the 3 new
kernel-driver controls, same day.** Asked what to do next; user picked
closing the ALSA-backend gap over the driver's remaining "Input Trim"
follow-up. Three real fixes, all live-verified against the actual card
(fresh second-handle re-reads, the file's own established pattern):

- **`set_sensitivity`** used to look for a control name
  (`"Line-{name} Sens."`) that never existed on this driver, guessed
  from a naming grammar rather than a real capture — errored honestly,
  but genuinely unusable. Now targets `"Instrument Ref Level"` (the
  real control just added to the kernel driver), mapping this trait's
  2-state `Sensitivity` enum onto 2 of its 3 states (Boost, item 2,
  isn't representable here — see `RmeDevice::set_ref_level`'s own
  3-state `REF_*` codes, which the USB backend already uses for that).
  Mirrors the write across both Instrument channels — one real switch,
  not two independent ones. `DeviceHandle::has_sensitivity_control()`
  flipped to `true` for the ALSA backend too (was gated off).
- **`set_phase`/`set_stereo_split`** didn't exist in `babyface.rs` at
  all before this — the trait's default (`Err`) was silently the only
  behavior. Both now target the kernel driver's new per-channel/per-pair
  controls (`"<name> Phase"`, `"PBx Stereo Split"`).
- **All 3 also gained attach-time readback** (`attach_mixer_elements`),
  which didn't exist before either — a fresh `BabyfacePro::open()`
  previously had no way to know the real hardware's current Sensitivity/
  Phase/Split state at all (always defaulted to unset/false regardless
  of reality). Boost reads back as the closer of the 2 states
  (-10dBV, since it shares those state bits) rather than being silently
  lost.
- **Not yet wired to any UI** — neither Phase nor Stereo Split have a
  button/key anywhere in the GUI or TUI (checked via grep before
  assuming otherwise: both only had `delegate!` forwarding, dead ends
  with zero callers, same class of thing the dead-code cleanup earlier
  today removed). Backend-complete and live-verified; a UI pass is a
  separate, explicitly-scoped follow-up, not silently bundled in here.
- 3 new/replaced live-hardware tests (`live_hardware_sensitivity_
  round_trip` — replaces the old "honestly unmapped" test, which no
  longer describes reality — plus new `live_hardware_phase_round_trip`/
  `live_hardware_stereo_split_round_trip`), all ran with `--ignored`
  against the real card, passed, hardware confirmed left in its
  default state afterward. 160/160 non-ignored tests.

**Phase/Stereo Split given real UI in both GUI and TUI, same day —
user asked for this before committing the backend wiring above.** New
`StripParams::has_phase`/`phase` (reuses `ch.eq.is_some()` for the
gate — already exactly "the 4 analog inputs," no parallel condition to
drift out of sync) and `has_split`/`split` (Playback strips
unconditionally). GUI: a new "Ø" icon-column button (Hardware Input
strips, same row as T/gear/EQ) and a new "SP" button (Playback strips)
— `Message::PhaseChanged`/`StereoSplitChanged(ChannelId, bool)`,
undoable by default (not added to `is_undoable`'s blacklist, matching
CUE/EQ-for-Record's own treatment). TUI: new keys `i` (Phase, Input
section, gated to `idx < 4`) and `n` (Stereo Split, Playback section)
— picked from the session's remaining unused letters, no stronger
mnemonic than several already-shipped keys (`x`=MS proc, `k`=link);
new `[Ø]`/`[SPLIT]` strip tags and both keys added to the header
legend. **Deliberately named `"SP"` not `"SPL"`** on the GUI button —
checked `ICON_BTN_W`/the already-shipped 2-character "EQ" button
first rather than guessing a 3-character label would fit at
`TEXT_MICRO` size. Live-verified in both UIs: a `--mock` screenshot
zoom confirms the Ø/SP glyphs render legibly and only appear on the
correct strip types (Ø absent from AS1/2 and the ADAT strips, present
only on AN1/2 and IN3/4 — matching the analog-inputs-only gate); a
real alacritty window confirms `i`/`n` correctly toggle the tags and
mirror across the Playback pair (`[SPLIT]` appeared on both PCM AN1
and PCM AN2 together, PB1's two channels). 2 new `update()`-driven
GUI tests (`phase_changed_reaches_the_input_model`,
`stereo_split_changed_mirrors_across_the_playback_pair`). 162/162
non-ignored workspace tests.

**Input Trim added to the kernel driver (closing its last upstream
follow-up) and wired into `babyface.rs`, same day.** `set_trim`'s trait
default is a silent `Ok(())` no-op — real now, targeting the driver's
new per-mic "<name> Trim Volume" controls. **Restricted to AN1-4**
(`idx < 4`), even though the GUI's own `has_trim` gate is unconditional
for every hardware input strip: PROTOCOL.md's Trim captures only ever
verified the 4 analog inputs, and — found while checking this — the
USB backend's own `set_trim` has no such guard at all. Since
`input_source(idx)` and the register math both succeed for ANY input
index (not just 0-3), calling Trim on e.g. AS1/2 through the USB
backend would silently compute the WRONG sibling's register (the "+1"
adjacency trick that correctly pairs AN1/AN2 and AN3/4 does not hold
for other source types) and corrupt an unrelated channel's crosspoint
— a real latent bug in already-shipped code, surfaced but not fixed in
that same pass. Also added attach-time readback (didn't exist before
either). New `live_hardware_trim_round_trip` test, ran with `--ignored`
against the real card, passed (including a negative dB value), hardware
confirmed restored to 0 afterward. No GUI/TUI changes needed — Trim
already had a real knob in both, now it actually reaches hardware on
the ALSA backend too instead of silently doing nothing. 162/162
non-ignored workspace tests (plus the new ignored live-hardware test,
run separately).

**That USB-backend corruption bug fixed right after, user: "tu peux
t'y attaquer."** Added the identical `idx >= 4` guard to `usb.rs::
set_trim` that `babyface.rs` already had, turning a silent wrong-
channel write into an honest `Err`. Deliberately did **not** narrow the
GUI's own `has_trim` gate to match — that's a separate, previously
made design decision (TotalMix itself shows a T button on every
hardware input, analog and digital alike; TuxMix matched that scope
for Hardware Inputs on purpose in an earlier session), not something
to unilaterally reverse just because the underlying protocol write
needed restricting. Net effect: the T button still renders everywhere
it did before, but now safely errors instead of corrupting a sibling
channel's crosspoint when clicked on a non-analog input — matches
babyface.rs's own behavior exactly, full parity between both backends.
Not live-hardware-tested (same standing limitation: the USB backend
can't open the device while the kernel driver holds it) — a one-line
bounds guard, verified by code review and the already-passing
`cargo test --workspace`. 162/162 workspace tests, unchanged (no new
test added — the guard is too trivial to warrant extracting into its
own pure function, unlike this session's other composition fixes).

**Gear-icon flyouts made independently multi-open, closing a real UX
gap vs real TotalMix.** User: "sur totalmix on peut en avoir plusieurs,
[TuxMix] les ferment pas" [sic] — real TotalMix lets you have several
channels' settings panels open simultaneously; TuxMix forced a single
global `Option<(ChannelId, FlyoutKind)>`, so opening a second gear icon
silently closed whatever was already open. `TuxMix::flyout_open` is now
a `HashSet<(ChannelId, FlyoutKind)>` — any number of Settings/EQ/Trim
panels can be open across different strips at once, matching the
reference. Two exceptions kept, both real rendering constraints rather
than a design choice, documented inline: (1) `Route` still closes any
other open `Route` first — its Stack-based overlay (`with_flyout`) only
ever positions one popover at a time; (2) opening a second Settings/EQ/
Trim panel on the SAME strip replaces the first rather than both
lingering — the inline-push render loop is an `if`/`else if` chain,
only the first match ever renders. New `open_flyout()`/
`close_route_flyout()` helpers replace the old single-slot
`set_flyout_open()`; `FlyoutKind` gained `Hash` (needed for the
`HashSet`). 2 new `update()`-driven tests covering both the cross-strip
case (the actual bug) and the two exceptions. Live-verified via the
`new()`-default-flip trick (this sandbox's own established workaround
for the click-testing limitation): a `--mock` screenshot with Input(0)'s
Settings and Input(2)'s EQ both pre-opened confirms they render
side-by-side, neither closing the other. 164/164 non-ignored workspace
tests.
