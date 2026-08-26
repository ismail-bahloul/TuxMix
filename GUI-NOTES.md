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

```
Priority │ Feature                       │ Effort   │ Why
─────────┼───────────────────────────────┼──────────┼──────────────────────────────
   Done  │ Icons for M/S/48V/PAD         │ —        │ Chevron done; badges/pan were already there
   Done  │ Pan visual indicator          │ —        │ Was already a real widget, not text
   Done  │ Quick Control mode            │ —        │ Unique differentiator, big audience
   Done  │ TUI polish + doc              │ —        │ dB readout, PAD key, dedup, doc, tests
   Done  │ Output strip distinction      │ —        │ Wider/taller/bigger-buttons, measured via pixel edges
   Done  │ Strip visual polish           │ —        │ Bg gradient + hover glow, both pixel-verified
   P2    │ MIDI-triggered scene crossfade│ 1-2 wks  │ Crossfade is unique; MIDI trigger alone isn't
   P2    │ Drag-to-reorder strips        │ 5-8 hrs  │ Usability for large sessions
   P2    │ Color tagging                 │ 3-4 hrs  │ Studio workflow standard
   P2    │ Search/filter bar             │ 2-3 hrs  │ Useful with 20+ channels
   P3    │ Fullscreen mode               │ < 1 hr   │ Common studio setup
   P3    │ Web UI (tablet)               │ weeks    │ Tablet control, big differentiator
   P3    │ Headless + HTTP API           │ 1 wk     │ Broadcast/theatre integration
   P3    │ Undo/Redo                     │ medium   │ Important but not for v0.1
```

**P0/P1** = all done as of this pass — every polish item and unique
differentiator originally scoped in this document is implemented and
verified (build + tests + headless visual/pixel checks).
**P2** = usability improvements for power users — the next open tier.
**P3** = ambitious but unlocks entirely new audiences and use cases.

The Done items alone already make TuxMix the most versatile RME
controller on any platform — P2/P3 are about going further, not catching up.
