//! Custom vertical fader + VU meter, drawn and driven entirely through a
//! [`canvas::Program`] — the one widget egui's built-in slider couldn't give
//! us (shift-drag fine range, scroll-wheel nudge, double-click reset).

use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{mouse, window, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};
use std::time::{Duration, Instant};

use crate::theme;

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
/// translucent color wash filling the whole column, and the ruler ticks
/// are drawn on top of it, like TotalMix — rather than two separate
/// side-by-side strips. The fader itself gets its own centered column.
///
/// These, and every other pixel constant in this file, are the sizes at
/// `scale == 1.0` (`theme::SCALE_DEFAULT`) — every widget here takes a
/// `scale` field/parameter and multiplies it in at both draw time and
/// hit-test time, so the window's adaptive scale (see
/// `app::recompute_ui_scale`) stays in sync between what's drawn and
/// what's clickable.
const METER_RULER_W: f32 = 30.0;
const GAP: f32 = 6.0;
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
const FLOOR_DB: f32 = -65.0;
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

fn vol_to_t(vol: f32) -> f32 {
    if vol <= 0.0 {
        0.0
    } else {
        db_to_t(20.0 * vol.log10())
    }
}

fn t_to_vol(t: f32) -> f32 {
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
    pub height: f32,
    pub show_meter: bool,
    pub modifiers: Modifiers,
    pub scale: f32,
    pub on_press: Box<dyn Fn(f32, Option<(f32, f32)>) -> Message>,
    pub on_drag: Box<dyn Fn(f32) -> Message>,
    pub on_release: Box<dyn Fn() -> Message>,
    pub on_reset: Box<dyn Fn() -> Message>,
}

impl<Message> Fader<Message> {
    /// Where the track starts, in canvas-local x — the track itself
    /// centered on `bounds_width` (not the meter+track group, which would
    /// pull the track off toward the meter's side). The strip's own column
    /// gives this canvas `Length::Fill`, so `bounds_width` *is* the strip's
    /// content width: centering the track on it, with the meter tucked
    /// against its left, is what puts the slider in the middle of the
    /// strip and the VU meter at the strip's left edge, TotalMix-style.
    fn track_x(&self, bounds_width: f32) -> f32 {
        ((bounds_width - TRACK_W * self.scale) / 2.0).max(0.0)
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
        pos_x - bounds_x >= self.track_x(bounds_width) - (GAP / 2.0) * self.scale
    }
}

#[derive(Default)]
pub struct State {
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
                let mut still_animating = self.show_meter && self.meter.is_settling(*now);

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
        let mut frame = Frame::new(renderer, bounds.size());
        let track_x = self.track_x(bounds.width);
        if self.show_meter {
            let meter_rect = Rectangle::new(
                Point::new(track_x - (GAP + METER_RULER_W) * self.scale, 0.0),
                Size::new(METER_RULER_W * self.scale, bounds.height),
            );
            // Meter first, as a translucent wash; ruler ticks drawn on top
            // of it so both share the column instead of splitting the strip.
            draw_meter(
                &mut frame,
                meter_rect,
                self.meter.at(Instant::now()),
                self.scale,
            );
            draw_ruler(&mut frame, meter_rect, self.scale);
        }
        let display_value = if state.dragging {
            self.value
        } else {
            state
                .display
                .map(|d| d.at(Instant::now()))
                .unwrap_or(self.value)
        };
        draw_track(
            &mut frame,
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
        vec![frame.into_geometry()]
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
            _ => mouse::Interaction::Idle,
        }
    }
}

const METER_PILL_W: f32 = 7.0;
const METER_RADIUS: f32 = 3.5;
const CLIP_H: f32 = 6.0;
const CLIP_GAP: f32 = 3.0;

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

/// Linear interpolation between two colors — `t` is clamped to [0, 1].
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Above this level the fill starts tinting toward red — a continuous
/// warning rather than a binary clip light, so the meter reads as "getting
/// hot" well before it actually clips.
const HOT_THRESHOLD: f32 = 0.85;

/// A slim rounded pill instead of a wide block — one calm color for the
/// signal, tinting progressively from green toward red above
/// `HOT_THRESHOLD`, plus a separate clip "LED" above the track that lights
/// up near 0 dBFS. Reads as a minimal modern level indicator, not a
/// traffic light, while still giving continuous feedback as it climbs.
fn draw_meter(frame: &mut Frame, r: Rectangle, level: f32, scale: f32) {
    let l = level.clamp(0.0, 1.0);
    let pill_w = METER_PILL_W * scale;
    let radius = METER_RADIUS * scale;
    let clip_h = CLIP_H * scale;
    let clip_gap = CLIP_GAP * scale;

    let track = Rectangle::new(
        Point::new(r.x, r.y + clip_h + clip_gap),
        Size::new(pill_w, (r.height - clip_h - clip_gap).max(0.0)),
    );

    frame.fill(
        &Path::new(|b| b.rounded_rectangle(track.position(), track.size(), radius.into())),
        Color::from_rgb8(0x08, 0x08, 0x0a),
    );

    if l > 0.0 {
        let fill_h = track.height * l;
        let fill_pos = Point::new(track.x, track.y + track.height - fill_h);
        let hot_t = (l - HOT_THRESHOLD) / (1.0 - HOT_THRESHOLD);
        let fill_color = lerp_color(theme::MGREEN, theme::MRED, hot_t);
        frame.fill(
            &Path::new(|b| b.rounded_rectangle(fill_pos, Size::new(pill_w, fill_h), radius.into())),
            fill_color,
        );
    }

    // Clip LED — a fixed indicator above the track, dim until triggered.
    let clip_rect = Rectangle::new(Point::new(r.x, r.y), Size::new(pill_w, clip_h));
    let clipping = l >= 0.95;
    let clip_color = if clipping {
        theme::MRED
    } else {
        Color::from_rgb8(0x3a, 0x16, 0x16)
    };
    if clipping {
        let glow = Rectangle::new(
            Point::new(r.x - 2.0 * scale, r.y - 2.0 * scale),
            Size::new(pill_w + 4.0 * scale, clip_h + 4.0 * scale),
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

/// Drawn on top of the (translucent) meter wash, so it needs to stay
/// legible against whatever color is lit behind it — brighter than the
/// usual secondary text color, with a tick + number per breakpoint.
fn draw_ruler(frame: &mut Frame, r: Rectangle, scale: f32) {
    const TICKS: [f32; 7] = [6.0, 0.0, -6.0, -10.0, -20.0, -40.0, -65.0];
    let label_color = theme::TEXT_SEC;
    let x0 = r.x + METER_PILL_W * scale + 4.0 * scale;

    for db in TICKS {
        let t = db_to_t(db);
        let y = r.y + r.height - r.height * t;
        let y = safe_clamp(y, r.y + 4.0 * scale, r.y + r.height - 4.0 * scale);

        let tick = Path::line(Point::new(x0, y), Point::new(x0 + 3.0 * scale, y));
        frame.stroke(
            &tick,
            Stroke::default().with_color(label_color).with_width(1.0),
        );

        let label = if db == 0.0 {
            "0".to_string()
        } else {
            format!("{}", -db as i32)
        };
        frame.fill_text(canvas::Text {
            content: label,
            position: Point::new(x0 + 5.0 * scale, y - 4.0 * scale),
            color: label_color,
            size: (theme::TEXT_MICRO * scale).into(),
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
        Length::Fixed(TRACK_W * fader.scale)
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
                if self.level.is_settling(*now) =>
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
        let meter_rect = Rectangle::new(
            Point::ORIGIN,
            Size::new(METER_RULER_W * self.scale, bounds.height),
        );
        draw_meter(
            &mut frame,
            meter_rect,
            self.level.at(Instant::now()),
            self.scale,
        );
        draw_ruler(&mut frame, meter_rect, self.scale);
        vec![frame.into_geometry()]
    }
}

pub fn vu_meter<'a, Message: 'a>(
    level: MeterFrame,
    height: f32,
    scale: f32,
) -> Element<'a, Message> {
    Canvas::new(VuMeter { level, scale })
        .width(Length::Fixed(METER_RULER_W * scale))
        .height(Length::Fixed(height))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn fill_stays_green_below_hot_threshold() {
        let c = lerp_color(
            theme::MGREEN,
            theme::MRED,
            (0.5 - HOT_THRESHOLD) / (1.0 - HOT_THRESHOLD),
        );
        assert_eq!(c, theme::MGREEN);
    }

    #[test]
    fn fill_turns_fully_red_at_full_level() {
        let hot_t = (1.0 - HOT_THRESHOLD) / (1.0 - HOT_THRESHOLD);
        let c = lerp_color(theme::MGREEN, theme::MRED, hot_t);
        assert!((c.r - theme::MRED.r).abs() < 1e-4);
        assert!((c.g - theme::MRED.g).abs() < 1e-4);
        assert!((c.b - theme::MRED.b).abs() < 1e-4);
    }

    #[test]
    fn fill_is_between_green_and_red_mid_hot_zone() {
        let hot_t = ((HOT_THRESHOLD + 1.0) / 2.0 - HOT_THRESHOLD) / (1.0 - HOT_THRESHOLD);
        let c = lerp_color(theme::MGREEN, theme::MRED, hot_t);
        assert!(c.r > theme::MGREEN.r && c.r < theme::MRED.r);
        assert!(c.g < theme::MGREEN.g && c.g > theme::MRED.g);
    }
}
