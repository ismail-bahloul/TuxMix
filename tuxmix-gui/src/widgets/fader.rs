//! Custom vertical fader + VU meter, drawn and driven entirely through a
//! [`canvas::Program`] — the one widget egui's built-in slider couldn't give
//! us (shift-drag fine range, scroll-wheel nudge, double-click reset).

use iced::advanced::text as advanced_text;
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{mouse, window, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::hash::Hash;
use std::time::{Duration, Instant};

use crate::theme;

fn hash_of(f: impl FnOnce(&mut std::collections::hash_map::DefaultHasher)) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    f(&mut h);
    h.finish()
}

/// Whether meters are interpolated and redrawn *between* ticks, at the
/// display's refresh rate. Measured 2026-10-22 (release build, mixer view,
/// signal on every meter): 19 % of a core with it, 5.5 % without. Every
/// one of those in-between redraws walks the whole window, and with a
/// signal playing they ran continuously. Off: a meter moves once per tick,
/// and the tick is fast enough (30 Hz, `app::TICK_ACTIVE`) that it still
/// reads as motion; the cap's easing after an outside change keeps its own
/// short animation.
const INTERPOLATE_METERS: bool = false;

/// How long a meter keyframe transition takes to visually settle — matches
/// `app.rs`'s `Tick` interval, since that's how often a new keyframe
/// (`MeterFrame`) arrives.
const METER_INTERP_MS: f32 = 50.0;

/// A meter's ease-out keyframe pair — the ballistics in `app.rs` only
/// compute a new value once per `Tick` (50ms), which reads as a stair-step
/// rather than motion if drawn as-is. `Fader`/`VuMeter` interpolate between
/// `prev` and `value` using their own draw-time `Instant::now()` instead of
/// the single stale snapshot `view()` was built with, and self-drive extra
/// redraws while still catching up (`canvas::Action::request_redraw()`),
/// settling back to the normal `Tick`-driven redraw rate once caught up —
/// full-refresh-rate motion without polling the device or rebuilding the
/// view any more often than before.
#[derive(Clone, Copy, Debug)]
pub struct MeterFrame {
    pub prev: f32,
    pub value: f32,
    pub since: Instant,
}

impl MeterFrame {
    /// A meter with no in-flight transition — e.g. outputs, which don't
    /// have live meter data wired up.
    pub fn still(value: f32) -> Self {
        Self {
            prev: value,
            value,
            since: Instant::now(),
        }
    }

    /// What a meter shows at `now`: the interpolated value, or (not
    /// interpolating, `INTERPOLATE_METERS`) simply the newest one.
    pub(crate) fn shown(&self, now: Instant) -> f32 {
        if INTERPOLATE_METERS {
            self.at(now)
        } else {
            self.value
        }
    }

    pub(crate) fn at(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.since).as_secs_f32() * 1000.0 / METER_INTERP_MS)
            .clamp(0.0, 1.0);
        self.prev + (self.value - self.prev) * t
    }

    pub(crate) fn is_settling(&self, now: Instant) -> bool {
        (self.value - self.prev).abs() > f32::EPSILON
            && now.duration_since(self.since).as_secs_f32() * 1000.0 < METER_INTERP_MS
    }
}

/// The meter and the dB ruler share one column — the meter is a
/// translucent color wash filling the whole column, and the ruler's
/// numbers are drawn centered *on top of* it, like TotalMix — rather
/// than two separate side-by-side strips. The fader itself gets its own
/// column right next to it, sharing the ruler's tick marks (see
/// `draw_ruler`). Narrowed from `30` — that used to be sized for a
/// slim 7px pill *plus* the numbers in their own lane beside it; once
/// the numbers moved to sit on top of a full-width wash instead (see
/// `draw_meter`'s own doc comment — a live screenshot showed this is
/// what the reference actually does), the column only needs to be as
/// wide as the widest 2-digit label, not pill-width + gap + label-width
/// added together. The old, wider value was quietly pushing the fader
/// (and everything to its right — the icon column, the dB readout) well
/// past where the reference puts them.
///
/// These, and every other pixel constant in this file, are the sizes at
/// `scale == 1.0` (`theme::SCALE_DEFAULT`) — every widget here takes a
/// `scale` field/parameter and multiplies it in at both draw time and
/// hit-test time, so the window's adaptive scale (see
/// `app::recompute_ui_scale`) stays in sync between what's drawn and
/// what's clickable.
/// Narrowed again, 22→17, after the numbers-on-top-of-the-wash change
/// above made the wider value's original justification (room for a
/// pill *and* a label side by side) moot — 17px is the tightest this
/// goes before clipping a 2-digit label at `theme::TEXT_MICRO`, checked
/// via a live screenshot rather than guessed.
///
/// 2026-10-20: the numbers left the meter for a lane of their own. Printed
/// on the wash they collided with the fill (and, on a linked pair, with the
/// gap between the two bars), and the wash had to stay translucent just so
/// they could be read through it. The column is now `[meter][ruler lane]`,
/// the meter fully opaque; `METER_RULER_W` is still the whole column, so
/// `layout_x` and the collapsed `vu_meter` keep sizing against one number.
const METER_W: f32 = 9.0;
const LANE_GAP: f32 = 1.0;
const RULER_LANE_W: f32 = 12.0;
const METER_RULER_W: f32 = METER_W + LANE_GAP + RULER_LANE_W;
const GAP: f32 = 4.0;
const TRACK_W: f32 = 26.0;
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Gap after the last `WheelScrolled` event that means "this is a new
/// gesture" rather than a continuation — wheel scrolling has no
/// press/release framing to mark that boundary explicitly.
pub(crate) const SCROLL_IDLE: Duration = Duration::from_millis(200);
/// Shift-drag sensitivity reduction: cursor travel over the whole track
/// only moves the value by this fraction of what a normal drag would.
/// Earlier this remapped `range` to a narrow absolute value window instead
/// (cap tracking the cursor 1:1 inside a "zoomed" scale) — that made the
/// cap's screen position discontinuous the instant the drag ended and the
/// range snapped back to full, and moved the unity reference dot along
/// with it. A plain relative-delta scale-down has no such range to snap.
pub(crate) const FINE_SENSITIVITY: f32 = 0.15;
/// Magnetic snap zone around the unity/default reference mark, in pixels
/// at `scale == 1.0` — landing within this many pixels of it while
/// dragging snaps the cap exactly onto it, like a detent on a real
/// console fader. Small movements inside the zone re-snap back to
/// center (since the accumulator is re-seeded at the snapped value each
/// time), so leaving it takes a deliberate move past the edge rather
/// than the value drifting off by a pixel.
const SNAP_PX: f32 = 2.0;

/// Fader travel is dB-tapered (like real hardware / TotalMix), not linear
/// amplitude — a power curve (t = 1 - x^K, x = db/FLOOR_DB) gives generous
/// resolution near unity and compresses the bottom into silence, without
/// the sharp knee a two-segment mapping produces (which crowded the
/// 0/-6/-10 ruler ticks together). The track spans FLOOR_DB .. +6 dB
/// (TOP_DB), TotalMix's fader range.
const FLOOR_DB: f32 = -60.0;
/// The fader's top: +6 dB (2.0 linear) like TotalMix, not 0 dB.
const TOP_DB: f32 = 6.0;
const TAPER_K: f32 = 0.6;

fn db_to_t(db: f32) -> f32 {
    let db = db.clamp(FLOOR_DB, TOP_DB);
    let x = ((db - TOP_DB) / (FLOOR_DB - TOP_DB)).clamp(0.0, 1.0);
    (1.0 - x.powf(TAPER_K)).clamp(0.0, 1.0)
}

fn t_to_db(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let x = (1.0 - t).powf(1.0 / TAPER_K);
    (TOP_DB + x * (FLOOR_DB - TOP_DB)).clamp(FLOOR_DB, TOP_DB)
}

pub(crate) fn vol_to_t(vol: f32) -> f32 {
    if vol <= 0.0 {
        0.0
    } else {
        db_to_t(20.0 * vol.log10())
    }
}

pub(crate) fn t_to_vol(t: f32) -> f32 {
    if t <= 0.0 {
        0.0
    } else {
        (10f32.powf(t_to_db(t) / 20.0)).clamp(0.0, 2.0)
    }
}

pub struct Fader<Message> {
    pub value: f32,
    pub range: (f32, f32),
    pub default_value: f32,
    pub meter: MeterFrame,
    /// Whether `meter` reflects a real reading on this backend — see
    /// `draw_meter`'s doc comment. Irrelevant when `show_meter` is `false`.
    pub meter_available: bool,
    /// The right channel's own meter, `Some` only for a LINKED stereo
    /// pair's combined strip — splits the single meter column into two
    /// thin side-by-side bars instead of showing just `meter`'s (left/
    /// even channel's) level alone. `None` for every unlinked/mono
    /// strip, unaffected.
    pub meter2: Option<MeterFrame>,
    pub meter2_available: bool,
    pub height: f32,
    pub show_meter: bool,
    /// Extra width (at `scale == 1.0`... no, already-scaled pixels, same
    /// as every other field here) that exists *outside* this canvas's own
    /// bounds but should still count when centering the track — the strip
    /// card's icon column (gear/EQ/T), a `row!` sibling that shrinks this
    /// canvas's own `Length::Fill` allocation without the canvas ever
    /// knowing it's there. Ignored when `show_meter` is `false` (the
    /// Matrix view's compact fader, which has no such sibling — always
    /// `0.0` from that call site). See `layout_x`'s own doc comment for
    /// why this is needed at all, not just cosmetic tuning.
    pub reserved_right: f32,
    pub modifiers: Modifiers,
    pub scale: f32,
    /// `false` skips drawing the groove/cap entirely — the click/drag/
    /// wheel/double-click logic all stay fully live (the whole canvas
    /// still counts as "over track", see `layout_x`'s `!show_meter`
    /// branch), just invisible. Used by the Matrix view's crosspoint
    /// cells: a bare numeric dB label (see `label`) is the only visible
    /// thing, with a full drag-to-adjust fader hidden behind it —
    /// "hidden mini-fader" rather than a second, visually-competing
    /// control bolted onto the same cell.
    pub show_track: bool,
    /// Drawn centered in the canvas when set, regardless of
    /// `show_track` — the Matrix view's own cell text (e.g. "-9.8"),
    /// left empty by every strip fader (`None`).
    pub label: Option<String>,
    pub label_color: Color,
    /// Canvas width when `show_meter` is `false` — ignored otherwise
    /// (a strip's own fader always fills its row, see `fader()`).
    /// Matters for real click/drag hit-testing, not just visuals: the
    /// whole canvas counts as "over track" when there's no meter
    /// column to exclude (`layout_x`'s `!show_meter` branch), so this
    /// is the actual clickable width, not just a cosmetic size. Used
    /// to be hardcoded to `TRACK_W` regardless of caller, leaving the
    /// Matrix view's cells with a ~26px hit target inside a visually
    /// larger 40px cell — small residual gap between "looks clickable"
    /// and "is clickable" that a matching width closes for free.
    pub compact_width: f32,
    pub on_press: Box<dyn Fn(f32, Option<(f32, f32)>) -> Message>,
    pub on_drag: Box<dyn Fn(f32) -> Message>,
    pub on_release: Box<dyn Fn() -> Message>,
    pub on_reset: Box<dyn Fn() -> Message>,
}

impl<Message> Fader<Message> {
    /// Left edge of the meter/ruler column and of the track, both in
    /// canvas-local x — laid out left-to-right as `[meter+ruler][gap]
    /// [track]`, matching the real TotalMix reference: the dB numbers are
    /// printed directly on the VU meter itself (see `draw_ruler` drawing
    /// on top of `draw_meter`'s same rect) and that combined column sits
    /// to the *left* of the fader, not the right — confirmed via a live
    /// screenshot at high zoom showing an actual green meter-fill segment
    /// low in that left column, right next to the fader rail. An earlier
    /// pass in this file had these swapped (`[track][meter+ruler]`,
    /// meter on the right) from a shallower read of a narrower crop that
    /// missed the live fill entirely — the user caught it.
    ///
    /// The *group* used to be centered on `bounds_width` alone — correct
    /// for keeping the meter/ruler on-screen (that was this fix's
    /// original point), but it centers the track on this *canvas's* own
    /// bounds, not the strip's. Since the strip's icon column
    /// (`reserved_right`) shrinks the canvas without the canvas knowing,
    /// the true center of the strip's content sits to the *right* of the
    /// canvas's own center — self-centering the group left the track a
    /// couple pixels right of the strip's real center (confirmed via a
    /// live screenshot pixel measurement: cap center 1.5-2px off from the
    /// card's true center), which the user then caught by eye. Centering
    /// the *track specifically* — not the meter+track group — on
    /// `(bounds_width + reserved_right) / 2` fixes that: the meter just
    /// goes wherever `GAP` puts it to the track's left, with a safety
    /// clamp for narrow bounds identical in spirit to the original
    /// negative-x fix this replaced (never let the meter go negative;
    /// shift the whole group right instead of letting it clip).
    fn layout_x(&self, bounds_width: f32) -> (f32, f32) {
        let true_center = (bounds_width + self.reserved_right) / 2.0;
        let track_x = true_center - (TRACK_W * self.scale) / 2.0;
        if !self.show_meter {
            // No meter, no `reserved_right` sibling either (see that
            // field's own doc comment) — plain self-centering, exactly
            // the pre-fix behavior for the Matrix view's compact fader.
            return (track_x.max(0.0), 0.0);
        }
        let meter_x = track_x - (METER_RULER_W + GAP) * self.scale;
        if meter_x < 0.0 {
            (track_x - meter_x, 0.0)
        } else {
            (track_x, meter_x)
        }
    }

    /// True if `pos_x` — in the same (parent/absolute) coordinate space as
    /// the `bounds` a `canvas::Program` method receives, hence `bounds_x`
    /// to normalize it — falls on the track side of the meter/track gap.
    /// The meter/ruler column is a read-only VU display, not part of the
    /// control surface, even though it shares this canvas with the track.
    /// Every input handler that cares where on the canvas the cursor is
    /// (press, wheel, the grab cursor) goes through this one spot instead
    /// of repeating the boundary math, so a future change to the layout
    /// constants can't leave one of them checking a stale boundary.
    fn is_over_track(&self, pos_x: f32, bounds_x: f32, bounds_width: f32) -> bool {
        let (track_x, _) = self.layout_x(bounds_width);
        pos_x - bounds_x >= track_x - (GAP / 2.0) * self.scale
    }
}

#[derive(Default)]
pub struct State {
    /// The parts of the drawing that do not move with the signal, each
    /// rebuilt only when its key changes: the dB ruler (size and scale
    /// only) and the rail with its cap (value, default, range, drag). A
    /// mixer has ~70 faders and nearly all of them sit still, yet each
    /// re-tessellated its ruler text and rail on every redraw.
    ruler_cache: canvas::Cache,
    ruler_key: std::cell::Cell<u64>,
    track_cache: canvas::Cache,
    track_key: std::cell::Cell<u64>,
    dragging: bool,
    last_click: Option<Instant>,
    /// Last cursor Y seen during an active drag. Each move applies the
    /// delta since this point (scaled by `FINE_SENSITIVITY` while Shift is
    /// held) rather than an absolute cursor-to-value mapping, and is
    /// re-read fresh every move — so Shift can be pressed or released
    /// mid-drag and the cap keeps going from wherever it already was,
    /// instead of jumping (which a fixed press-time anchor, or falling
    /// back to absolute cursor tracking, would both cause).
    drag_pos: Option<f32>,
    /// Tapered position accumulated across the current drag. A real mouse
    /// fires several `CursorMoved` events per rendered frame — `self.value`
    /// only reflects the *last frame's* published value, so basing each
    /// event's target on `self.value` silently drops every event but the
    /// last one in a batch (each recomputes from the same stale baseline
    /// instead of building on what the previous event in that same batch
    /// already published). Accumulating here instead, in widget-local
    /// state that persists across every event regardless of render
    /// timing, makes the cap track the cursor 1:1 no matter how events
    /// happen to batch.
    drag_t: Option<f32>,
    /// Same accumulator problem as `drag_t`, for wheel-scroll nudges — a
    /// fast trackpad swipe fires many `Pixels` events per gesture, easily
    /// several per rendered frame. No press/release frames a scroll
    /// gesture the way a drag does, so a gesture boundary is inferred from
    /// an idle gap instead (`SCROLL_IDLE`).
    scroll_t: Option<f32>,
    last_scroll: Option<Instant>,
    /// Displayed (possibly still-easing) cap position, kept separate from
    /// `self.value` (the authoritative target) so a value change that
    /// *didn't* come from this fader's own drag — another channel moving
    /// together in a group selection, a scene load — eases into place
    /// instead of snapping, the same way the VU meter does. Not consulted
    /// while `dragging` is true: the user's own drag already updates on
    /// every mouse event, so layering interpolation on top of that would
    /// only add latency, not smoothness.
    display: Option<MeterFrame>,
}

impl<Message> canvas::Program<Message> for Fader<Message> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let locate = |y: f32| -> f32 {
            let (lo, hi) = self.range;
            let screen_t = 1.0 - ((y - bounds.y) / bounds.height).clamp(0.0, 1.0);
            let (t_lo, t_hi) = (vol_to_t(lo), vol_to_t(hi));
            t_to_vol(t_lo + screen_t * (t_hi - t_lo))
        };

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_over(bounds)?;
                if !self.is_over_track(pos.x, bounds.x, bounds.width) {
                    return None;
                }
                let now = Instant::now();
                let is_double = state
                    .last_click
                    .is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK);

                if is_double {
                    state.dragging = false;
                    state.last_click = None;
                    state.drag_pos = None;
                    state.drag_t = None;
                    return Some(canvas::Action::publish((self.on_reset)()).and_capture());
                }
                state.last_click = Some(now);
                state.dragging = true;

                let value = locate(pos.y);
                state.drag_pos = Some(pos.y);
                state.drag_t = Some(vol_to_t(value));
                Some(canvas::Action::publish((self.on_press)(value, None)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if !state.dragging {
                    return None;
                }
                let pos = cursor.land().position()?;
                let prev_y = state.drag_pos.unwrap_or(pos.y);
                let screen_dt = -(pos.y - prev_y) / bounds.height;
                let mult = if self.modifiers.shift() {
                    FINE_SENSITIVITY
                } else {
                    1.0
                };
                let base_t = state.drag_t.unwrap_or_else(|| vol_to_t(self.value));
                let mut t = (base_t + screen_dt * mult).clamp(0.0, 1.0);
                let snap_t = (SNAP_PX * self.scale) / bounds.height.max(1.0);
                let default_t = vol_to_t(self.default_value);
                if (t - default_t).abs() < snap_t {
                    t = default_t;
                }
                state.drag_pos = Some(pos.y);
                state.drag_t = Some(t);
                Some(canvas::Action::publish((self.on_drag)(t_to_vol(t))).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if !state.dragging {
                    return None;
                }
                state.dragging = false;
                state.drag_pos = None;
                state.drag_t = None;
                // While dragging, draw() reads self.value directly and
                // never touches `display` — resync it to the just-released
                // position now, so the next external change eases in from
                // here rather than from whatever `display` was cached at
                // before the drag started (which would show as a snap-back
                // glitch the instant dragging ends).
                state.display = Some(MeterFrame::still(self.value));
                Some(canvas::Action::publish((self.on_release)()).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(pos) = cursor.position_over(bounds) else {
                    return None;
                };
                if !self.is_over_track(pos.x, bounds.x, bounds.width) {
                    return None;
                }
                // Ctrl+scroll never nudges a value here — held down, it
                // reads as "the user means something other than a fine
                // adjustment," whatever that turns out to be (OS-level
                // zoom, a future shortcut, ...), so leave it uncaptured
                // rather than move the fader underneath it.
                if self.modifiers.control() {
                    return None;
                }
                // Lines are discrete wheel detents (dy is usually ±1 per
                // click); Pixels are a continuous trackpad/high-res stream
                // (many small events per gesture) — they need very
                // different per-event magnitudes or one feels dead and the
                // other feels like it teleports. Stepping in tapered
                // t-space (not raw linear volume) also keeps the nudge feel
                // consistent across the whole dB range instead of vanishing
                // near unity, where a fixed amplitude step is only a
                // fraction of a dB.
                let (dy, base_step) = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => (*y, 0.03),
                    mouse::ScrollDelta::Pixels { y, .. } => (*y, 0.0015),
                };
                if dy == 0.0 {
                    return None;
                }
                let mult = if self.modifiers.shift() { 0.25 } else { 1.0 };
                let now = Instant::now();
                let fresh_gesture = state
                    .last_scroll
                    .is_none_or(|t| now.duration_since(t) > SCROLL_IDLE);
                let base_t = if fresh_gesture {
                    vol_to_t(self.value)
                } else {
                    state.scroll_t.unwrap_or_else(|| vol_to_t(self.value))
                };
                let new_t = (base_t + dy * base_step * mult).clamp(0.0, 1.0);
                state.scroll_t = Some(new_t);
                state.last_scroll = Some(now);
                let value = t_to_vol(new_t);
                Some(canvas::Action::publish((self.on_drag)(value)).and_capture())
            }
            // Keeps redrawing at full display refresh rate while the meter
            // and/or the cap itself are still interpolating — settles back
            // to the normal Tick-driven rate on its own once caught up
            // (see `MeterFrame`).
            canvas::Event::Window(window::Event::RedrawRequested(now)) => {
                let mut still_animating =
                    INTERPOLATE_METERS && self.show_meter && self.meter.is_settling(*now);

                if !state.dragging {
                    let display = state
                        .display
                        .get_or_insert_with(|| MeterFrame::still(self.value));
                    if (display.value - self.value).abs() > f32::EPSILON {
                        *display = MeterFrame {
                            prev: display.at(*now),
                            value: self.value,
                            since: *now,
                        };
                    }
                    still_animating |= display.is_settling(*now);
                }

                if still_animating {
                    Some(canvas::Action::request_redraw())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let (track_x, meter_x) = self.layout_x(bounds.width);
        let mut layers: Vec<Geometry> = Vec::with_capacity(3);

        // Dynamic layer: the level (and the label, on the cells that have
        // one). Redrawn every time.
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        if self.show_meter {
            let (meter_rect, _) = meter_column(meter_x, bounds.height, self.scale);
            draw_meter(
                &mut frame,
                meter_rect,
                self.meter.shown(Instant::now()),
                self.scale,
                self.meter_available,
                self.meter2
                    .map(|m| (m.shown(Instant::now()), self.meter2_available)),
            );
        }
        if let Some(label) = &self.label {
            frame.fill_text(canvas::Text {
                content: label.clone(),
                position: Point::new(bounds.width / 2.0, bounds.height / 2.0),
                color: self.label_color,
                size: (theme::TEXT_MICRO * self.scale).into(),
                align_x: advanced_text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }
        layers.push(frame.into_geometry());

        // Static layer 1: the ruler's ticks and numbers.
        if self.show_meter {
            let key = hash_of(|h| {
                (
                    bounds.width.to_bits(),
                    bounds.height.to_bits(),
                    self.scale.to_bits(),
                )
                    .hash(h);
                meter_x.to_bits().hash(h);
            });
            if state.ruler_key.replace(key) != key {
                state.ruler_cache.clear();
            }
            layers.push(state.ruler_cache.draw(renderer, bounds.size(), |frame| {
                let (_, ruler_rect) = meter_column(meter_x, bounds.height, self.scale);
                draw_ruler(frame, ruler_rect, self.scale);
            }));
        }

        // Static layer 2: the rail and its cap, which only move with the
        // value.
        if self.show_track {
            let display_value = if state.dragging {
                self.value
            } else {
                state
                    .display
                    .map(|d| d.at(Instant::now()))
                    .unwrap_or(self.value)
            };
            let key = hash_of(|h| {
                (
                    bounds.width.to_bits(),
                    bounds.height.to_bits(),
                    self.scale.to_bits(),
                )
                    .hash(h);
                (display_value.to_bits(), self.default_value.to_bits()).hash(h);
                (self.range.0.to_bits(), self.range.1.to_bits()).hash(h);
                (state.dragging, track_x.to_bits()).hash(h);
            });
            if state.track_key.replace(key) != key {
                state.track_cache.clear();
            }
            layers.push(state.track_cache.draw(renderer, bounds.size(), |frame| {
                draw_track(
                    frame,
                    Rectangle::new(
                        Point::new(track_x, 0.0),
                        Size::new(TRACK_W * self.scale, bounds.height),
                    ),
                    display_value,
                    self.default_value,
                    self.range,
                    state.dragging,
                    self.scale,
                );
            }));
        }
        layers
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_over(bounds) {
            Some(pos) if self.is_over_track(pos.x, bounds.x, bounds.width) => {
                mouse::Interaction::Grab
            }
            _ => mouse::Interaction::None,
        }
    }
}

/// Splits the meter/ruler column starting at `x` into the bar itself and
/// the lane the dB numbers sit in, right of it (next to the fader track).
fn meter_column(x: f32, height: f32, scale: f32) -> (Rectangle, Rectangle) {
    let meter = Rectangle::new(Point::new(x, 0.0), Size::new(METER_W * scale, height));
    let ruler = Rectangle::new(
        Point::new(x + (METER_W + LANE_GAP) * scale, 0.0),
        Size::new(RULER_LANE_W * scale, height),
    );
    (meter, ruler)
}

const METER_RADIUS: f32 = 2.0;
const CLIP_H: f32 = 6.0;

/// `f32::clamp` panics if `lo > hi` — which a plain `min - margin, max -
/// margin` pair can produce for a single bad frame: iced 0.14 has a layout
/// quirk where a `Canvas` inside a horizontally-scrollable row can receive
/// a stale, much-too-small `bounds` for one frame right after its declared
/// `Length::Fixed` size changes (observed when a window resize changes
/// `ui_scale`, and so every strip's width, mid-session — see
/// `app::recompute_ui_scale`; the next frame recovers on its own).
/// Drawing code must never crash the whole app over a transient bad
/// layout, so every clamp against a widget's own bounds goes through this
/// instead of the raw method.
fn safe_clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if lo <= hi {
        v.clamp(lo, hi)
    } else {
        (lo + hi) / 2.0
    }
}

/// The meter's color zones, bottom to top, as (upper edge in dB, color):
/// signal green up to -12 dB, amber to -3 dB, red above. The last edge is
/// the top of the scale, so every level falls in some zone.
const METER_ZONES: [(f32, Color); 3] = [
    (-12.0, theme::MGREEN),
    (-3.0, theme::YSIM),
    (TOP_DB, theme::MRED),
];

/// Fill opacity. It used to be 0.55 so the ruler's numbers could be read
/// *through* the wash; they have their own lane now (`RULER_LANE_W`), so
/// the fill is all but solid, with just enough give to sit in the dark
/// well instead of glaring.
const FILL_ALPHA: f32 = 0.92;

/// A full-width wash spanning the whole ruler column — not a slim pill
/// off to one side — precisely so `draw_ruler`'s numbers can be
/// superimposed on it rather than pushed into their own extra lane
/// (which used to widen this whole column well past what the numbers
/// alone need, shoving the fader and everything after it further right
/// than the reference: see `METER_RULER_W`'s own doc comment). Tints
/// through `METER_ZONES` (green, amber, red), plus a
/// separate clip "LED" above the track that lights up near 0 dBFS.
/// `available` is a per-session backend capability (see `DeviceHandle::
/// has_input_meter`/`has_playback_meters`), not a per-tick reading —
/// when `false`, the channel has no real level data on this backend
/// (e.g. playback meters on every real backend today; input meters too
/// on the ALSA/kernel-driver backend, see `PROTOCOL.md`'s "VU meters:
/// conclusion"). Rendering that as a normal meter pinned at 0 would read
/// as "silence", which is a different claim than "not measured" — so
/// this draws a dashed, uncolored track instead of pretending there's a
/// live reading.
/// Dispatches to one full-width bar, or — when `right` is `Some` — two
/// thin bars side by side sharing `r` (same height as a single bar,
/// half the width each, a small gap between): a linked stereo pair's
/// meter in real TotalMix isn't one wide bar, it's the two mono meters
/// placed edge to edge (user-confirmed against the reference, having
/// caught this session's own earlier gap where a linked pair silently
/// only ever showed its left/even channel's level).
fn draw_meter(
    frame: &mut Frame,
    r: Rectangle,
    level: f32,
    scale: f32,
    available: bool,
    right: Option<(f32, bool)>,
) {
    let Some((level2, available2)) = right else {
        draw_meter_bar(frame, r, level, scale, available);
        return;
    };
    let gap = 1.0 * scale;
    let half_w = ((r.width - gap) / 2.0).max(0.0);
    let left_rect = Rectangle::new(r.position(), Size::new(half_w, r.height));
    let right_rect = Rectangle::new(
        Point::new(r.x + half_w + gap, r.y),
        Size::new(half_w, r.height),
    );
    draw_meter_bar(frame, left_rect, level, scale, available);
    draw_meter_bar(frame, right_rect, level2, scale, available2);
}

fn draw_meter_bar(frame: &mut Frame, r: Rectangle, level: f32, scale: f32, available: bool) {
    let l = level.clamp(0.0, 1.0);
    let fill_w = r.width;
    let radius = METER_RADIUS * scale;
    let clip_h = CLIP_H * scale;

    // Background/fill are measured against the *full* `r`, exactly like
    // `draw_ruler`'s ticks and the fader cap next to it — previously
    // this track was inset by the clip-LED's reserved strip, so a
    // signal's fill top landed a few px below its own ruler gridline at
    // every dB value, worst right at 0 dBFS (fixed as its own residual,
    // right after the linear-vs-tapered curve fix below fixed the
    // *shape* of that same mismatch). The clip LED is drawn last (see
    // below) and fully opaque, so it still reads as a distinct control
    // sitting on top of the meter, not blended into it — it just no
    // longer reserves its own dead strip that the fill could never
    // reach.
    frame.fill(
        &Path::new(|b| b.rounded_rectangle(r.position(), r.size(), radius.into())),
        Color::from_rgb8(0x08, 0x08, 0x0a),
    );

    if !available {
        // A handful of short dashes down the middle of the track, in a
        // dim neutral gray — reads as "N/A" without needing text layout
        // inside a canvas this narrow.
        let dash_w = fill_w * 0.5;
        let dash_h = 2.0 * scale;
        let dash_x = r.x + (fill_w - dash_w) / 2.0;
        let count = ((r.height / (dash_h * 2.5)).floor() as usize).max(1);
        for i in 0..count {
            let dash_y = r.y + r.height * (i as f32 + 0.5) / count as f32;
            frame.fill(
                &Path::new(|b| {
                    b.rounded_rectangle(
                        Point::new(dash_x, dash_y - dash_h / 2.0),
                        Size::new(dash_w, dash_h),
                        (dash_h / 2.0).into(),
                    )
                }),
                Color::from_rgb8(0x3a, 0x3a, 0x3e),
            );
        }
        return;
    }

    if l > 0.0 {
        // `l` is linear amplitude (1.0 = 0 dBFS), but the ruler's
        // ticks — and the fader track right next to this column — are
        // positioned on the *tapered* dB curve (`db_to_t`/`vol_to_t`),
        // not linearly. Filling by raw `l` used to land a -6 dBFS
        // signal's fill top well below the "-6" gridline instead of
        // right at it (checked: `vol_to_t(0.5)` ≈ 0.63, not 0.5) — real
        // signal data only started flowing through here this session
        // (previously every real backend read "N/A"), which is what
        // made this actually matter rather than being a latent bug.
        // Fixed color zones, console-style, instead of tinting the whole
        // bar by its peak: with an opaque fill, a hot signal used to turn
        // the entire column red, so "loud" and "clipping" looked alike.
        // Each zone is filled only as far as the level reaches into it.
        let top = r.y + r.height - r.height * vol_to_t(l);
        let mut lower_db = FLOOR_DB;
        for (upper_db, color) in METER_ZONES {
            let zone_bottom = r.y + r.height - r.height * db_to_t(lower_db);
            let zone_top = (r.y + r.height - r.height * db_to_t(upper_db)).max(top);
            if zone_top < zone_bottom {
                frame.fill(
                    &Path::rectangle(
                        Point::new(r.x, zone_top),
                        Size::new(fill_w, zone_bottom - zone_top),
                    ),
                    Color {
                        a: FILL_ALPHA,
                        ..color
                    },
                );
            }
            lower_db = upper_db;
        }
    }

    // Clip LED — a fixed indicator above the track, dim until triggered.
    let clip_rect = Rectangle::new(Point::new(r.x, r.y), Size::new(fill_w, clip_h));
    let clipping = l >= 0.95;
    let clip_color = if clipping {
        Color {
            a: FILL_ALPHA,
            ..theme::MRED
        }
    } else {
        Color::from_rgb8(0x3a, 0x16, 0x16)
    };
    if clipping {
        let glow = Rectangle::new(
            Point::new(r.x - 2.0 * scale, r.y - 2.0 * scale),
            Size::new(fill_w + 4.0 * scale, clip_h + 4.0 * scale),
        );
        frame.fill(
            &Path::new(|b| {
                b.rounded_rectangle(
                    glow.position(),
                    glow.size(),
                    (clip_h / 2.0 + 2.0 * scale).into(),
                )
            }),
            Color {
                a: 0.35,
                ..theme::MRED
            },
        );
    }
    frame.fill(
        &Path::new(|b| {
            b.rounded_rectangle(
                clip_rect.position(),
                clip_rect.size(),
                (clip_h / 2.0).into(),
            )
        }),
        clip_color,
    );
}

const RAIL_W: f32 = 3.5;
const CAP_W: f32 = 22.0;
const CAP_H: f32 = 15.0;
const CAP_RADIUS: f32 = 3.0;
/// Half-length of the unity/default reference tick — wider than the rail
/// so it reads as a mark crossing it, not a dot sitting on it.
const REF_HALF_W: f32 = 6.0;

fn draw_track(
    frame: &mut Frame,
    r: Rectangle,
    value: f32,
    default_value: f32,
    range: (f32, f32),
    dragging: bool,
    scale: f32,
) {
    let rail_w = RAIL_W * scale;
    let cap_w = CAP_W * scale;
    let cap_h = CAP_H * scale;
    let cap_radius = CAP_RADIUS * scale;
    let ref_half_w = REF_HALF_W * scale;

    let (lo, hi) = range;
    let (t_lo, t_hi) = (vol_to_t(lo), vol_to_t(hi));

    let pos_of = |v: f32| -> f32 {
        let t = if t_hi > t_lo {
            ((vol_to_t(v) - t_lo) / (t_hi - t_lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        r.y + r.height - r.height * t
    };

    let cx = r.x + r.width / 2.0;
    let half_cap = cap_h / 2.0;
    let raw_y = pos_of(value);
    let cap_y = safe_clamp(raw_y, r.y + half_cap, r.y + r.height - half_cap);

    // Groove — the full-length rail, unfilled color.
    let groove = Path::rectangle(
        Point::new(cx - rail_w / 2.0, r.y),
        Size::new(rail_w, r.height),
    );
    frame.fill(&groove, theme::BORDER);

    // Filled portion of the rail, from the bottom up to the cap.
    if raw_y < r.y + r.height {
        let fill = Path::rectangle(
            Point::new(cx - rail_w / 2.0, cap_y),
            Size::new(rail_w, r.y + r.height - cap_y),
        );
        frame.fill(&fill, theme::FADER);
    }

    // Unity/default reference — a tick crossing the rail, like the 0 dB
    // mark on a real console strip (a dot read as a blob at this scale;
    // a line reads immediately as "mark on a ruler").
    let ref_y = safe_clamp(pos_of(default_value), r.y, r.y + r.height);
    let ref_tick = Path::line(
        Point::new(cx - ref_half_w, ref_y),
        Point::new(cx + ref_half_w, ref_y),
    );
    frame.stroke(
        &ref_tick,
        Stroke::default()
            .with_color(theme::TEXT_SEC)
            .with_width(1.5),
    );

    // Cap — flat + ridged, like a real fader cap grip, no heavy bevel.
    let cap_left = cx - cap_w / 2.0;
    let cap_top = cap_y - half_cap;

    let body = Path::new(|b| {
        b.rounded_rectangle(
            Point::new(cap_left, cap_top),
            Size::new(cap_w, cap_h),
            cap_radius.into(),
        )
    });
    let body_color = if dragging {
        Color::from_rgb8(0xf0, 0xf1, 0xf4)
    } else {
        theme::FADER
    };
    frame.fill(&body, body_color);

    // Ridge lines — grip texture. The middle one doubles as a precise
    // position marker (thin bright line, like the reference line on some
    // analog console fader caps) rather than a fourth separate line drawn
    // on top of it at the same y.
    for i in 1..=3 {
        let y = cap_top + (cap_h / 4.0) * i as f32;
        let ridge = Path::rectangle(
            Point::new(cap_left + 3.0 * scale, y - 0.5),
            Size::new(cap_w - 6.0 * scale, 1.0),
        );
        let color = if i == 2 {
            Color::from_rgba8(0xff, 0xff, 0xff, 0.65)
        } else {
            Color::from_rgba8(0x00, 0x00, 0x00, 0.25)
        };
        frame.fill(&ridge, color);
    }

    frame.stroke(
        &body,
        Stroke::default().with_color(theme::BORDER).with_width(1.0),
    );
}

/// Drawn on top of the (translucent, see `FILL_ALPHA`) meter wash, so it
/// needs to stay legible against whatever color is lit behind it —
/// numbers centered in the same rect `draw_meter` just filled, TotalMix-
/// style, rather than pushed into their own lane to one side of it. The
/// tick dash sits at the rect's own right edge — right where the fader
/// track begins right after it (see `Fader::layout_x`), the same
/// "shared between the meter and the fader" position the reference uses.
fn draw_ruler(frame: &mut Frame, r: Rectangle, scale: f32) {
    const TICKS: [f32; 6] = [0.0, -6.0, -10.0, -20.0, -40.0, -60.0];
    let label_x = r.x + r.width / 2.0;
    // Measured against the *full* height, matching `draw_track`'s own
    // `pos_of` (the fader cap) and `draw_meter`'s fill: all three agree on
    // where a given dB value sits.
    for db in TICKS {
        let t = db_to_t(db);
        let y = r.y + r.height - r.height * t;
        let y = safe_clamp(y, r.y + 4.0 * scale, r.y + r.height - 4.0 * scale);
        let label = if db == 0.0 {
            "0".to_string()
        } else {
            format!("{}", -db as i32)
        };
        frame.fill_text(canvas::Text {
            content: label,
            position: Point::new(label_x, y),
            // 0 dB is the reference the cap rests on: one step brighter.
            color: if db == 0.0 {
                theme::TEXT_SEC
            } else {
                Color {
                    a: 0.6,
                    ..theme::TEXT_SEC
                }
            },
            size: (theme::TEXT_MICRO * scale).into(),
            align_x: advanced_text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
    }
}

pub fn fader<'a, Message: 'a>(fader: Fader<Message>) -> Element<'a, Message>
where
    Message: Clone,
{
    let height = fader.height;
    // With a meter, this is the main strip fader: `Fill` so the canvas
    // spans the strip's actual content width, and `Fader::track_x` centers
    // the track within whatever width that turns out to be. Without one,
    // it's a Matrix-view cell — small and `Fixed`, as before; `Fill` there
    // would blow up inside the Matrix's horizontally-scrollable row, which
    // has no bound on how wide a `Fill` child could claim.
    let width = if fader.show_meter {
        Length::Fill
    } else {
        Length::Fixed(fader.compact_width * fader.scale)
    };
    Canvas::new(fader)
        .width(width)
        .height(Length::Fixed(height))
        .into()
}

/// The meter+ruler column on its own, with no track/cap and no mouse
/// interaction at all — for a collapsed strip, which trades every control
/// (fader, mute/solo, pan) for a glance-only level readout.
struct VuMeter {
    level: MeterFrame,
    scale: f32,
    available: bool,
}

impl<Message> canvas::Program<Message> for VuMeter {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // See `Fader`'s identical arm.
        match event {
            canvas::Event::Window(window::Event::RedrawRequested(now))
                if INTERPOLATE_METERS && self.level.is_settling(*now) =>
            {
                Some(canvas::Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (meter_rect, ruler_rect) = meter_column(0.0, bounds.height, self.scale);
        draw_meter(
            &mut frame,
            meter_rect,
            self.level.shown(Instant::now()),
            self.scale,
            self.available,
            // Collapsed strips are a glance-only view with no room for a
            // second bar even if this were a linked pair — out of scope
            // for this pass, single-bar only here.
            None,
        );
        draw_ruler(&mut frame, ruler_rect, self.scale);
        vec![frame.into_geometry()]
    }
}

pub fn vu_meter<'a, Message: 'a>(
    level: MeterFrame,
    height: f32,
    scale: f32,
    available: bool,
) -> Element<'a, Message> {
    Canvas::new(VuMeter {
        level,
        scale,
        available,
    })
    .width(Length::Fixed(METER_RULER_W * scale))
    .height(Length::Fixed(height))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_fill_curve_is_tapered_not_linear() {
        // Guards `draw_meter`'s fill-height fix: a -6 dBFS signal
        // (linear amplitude 0.5) must land its fill top at the
        // ruler's own tapered "-6" position, not halfway up the
        // column — the bug this fixed used raw linear amplitude
        // directly as the fill fraction, which would make this
        // assertion fail (0.5 vs 0.5, no taper).
        let t = vol_to_t(0.5);
        assert!(
            (t - 0.5).abs() > 0.1,
            "expected the tapered curve to differ meaningfully from linear 0.5, got {t}"
        );
        // And it should match `db_to_t` fed the actual dB value,
        // since that's what the ruler's own ticks use — the whole
        // point is these two agree.
        let expected = db_to_t(20.0 * 0.5f32.log10());
        assert!((t - expected).abs() < 1e-4);
    }

    #[test]
    fn still_frame_is_never_settling() {
        // prev == value: nothing to interpolate, so no reason to keep
        // requesting redraws — this is what silence/idle looks like.
        let f = MeterFrame::still(0.3);
        assert!(!f.is_settling(f.since));
        assert!(!f.is_settling(f.since + Duration::from_millis(10)));
    }

    #[test]
    fn transitioning_frame_settles_after_the_interp_window() {
        let f = MeterFrame {
            prev: 0.2,
            value: 0.8,
            since: Instant::now(),
        };
        assert!(
            f.is_settling(f.since),
            "just started — should still be settling"
        );
        assert!(
            f.is_settling(f.since + Duration::from_millis(10)),
            "mid-transition — should still be settling"
        );
        assert!(
            !f.is_settling(f.since + Duration::from_millis(60)),
            "past the interp window — should have stopped requesting redraws"
        );
    }

    #[test]
    fn at_interpolates_linearly_between_prev_and_value() {
        let f = MeterFrame {
            prev: 0.0,
            value: 1.0,
            since: Instant::now(),
        };
        assert_eq!(f.at(f.since), 0.0);
        assert_eq!(f.at(f.since + Duration::from_millis(50)), 1.0);
        let mid = f.at(f.since + Duration::from_millis(25));
        assert!(
            (mid - 0.5).abs() < 0.01,
            "expected ~0.5 at the midpoint, got {mid}"
        );
    }

    #[test]
    fn meter_zones_cover_the_scale_in_order() {
        let mut lower = FLOOR_DB;
        for (upper, _) in METER_ZONES {
            assert!(upper > lower);
            lower = upper;
        }
        assert_eq!(lower, TOP_DB);
    }
}
