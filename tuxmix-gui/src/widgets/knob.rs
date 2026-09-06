//! A rotary knob — TotalMix-style value control (gain, pan) as an
//! alternative to a linear track. Drag vertically to turn it (a literal
//! circular drag is fiddly with a 2D mouse, so — like every DAW's own
//! virtual knobs, this included — "up" and "down" turn it, not tracing
//! an arc), shift-drag for fine adjustment, scroll wheel to nudge,
//! double-click to reset to default. Same interaction shape as
//! [`super::fader::Fader`], just driving an angle instead of a cap
//! position.

use iced::advanced::text;
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{alignment, mouse, window, Color, Element, Length, Radians, Rectangle, Renderer, Theme};
use std::time::Instant;

use super::fader::{MeterFrame, DOUBLE_CLICK, FINE_SENSITIVITY, SCROLL_IDLE};
use crate::theme;

/// TotalMix 2.0-style knob: a flat dark disc with a single position tick
/// and its value printed in the middle, rather than a colored arc + needle
/// — see `Bus_design.png`. Sized to read as a real focal point on the
/// strip (roughly what the reference shows relative to the card width),
/// not a small accent.
const DIAMETER: f32 = 42.0;
const BORDER_WIDTH: f32 = 1.5;
const TICK_LEN: f32 = 5.0;
/// The tick extends past `DIAMETER`'s own edge (see `draw`); the canvas
/// element's actual box (see `knob()`) is padded by this on every side so
/// it doesn't get clipped by the widget's own layout bounds — the drawn
/// circle itself (radius, face, tick — all still sized off `DIAMETER`) is
/// unchanged, just centered in a slightly bigger box.
const MARGIN: f32 = TICK_LEN + 1.0;
/// Vertical drag distance (at `scale == 1.0`) for a full sweep across
/// `range` — independent of the knob's own on-screen size, the same way a
/// real trim pot's turn radius has nothing to do with how far a mouse
/// drags it.
const DRAG_PX: f32 = 150.0;
/// Sweep geometry: clockwise from the positive x-axis (screen convention,
/// so 0=3 o'clock, 90°=6 o'clock/down, 180°=9 o'clock, 270°=12 o'clock).
/// Starting at 135° (lower-left, ~7:30) and sweeping 270° clockwise lands
/// on 45° (lower-right, ~4:30) — the standard knob shape, gap centered on
/// 6 o'clock, indicator pointing straight up at the midpoint value.
const START_DEG: f32 = 135.0;
const SWEEP_DEG: f32 = 270.0;

pub struct Knob<Message> {
    pub value: f32,
    pub range: (f32, f32),
    /// Printed centered inside the knob face — the reference design has no
    /// separate value readout below it, the knob doubles as the readout.
    pub label: String,
    /// Draws a filled orange arc from the 12-o'clock (center-of-range)
    /// position round to the current value, like the real TotalMix pan
    /// knob (confirmed in a live screenshot: e.g. "L57" shows a gold arc
    /// swept from top toward the left). Only meaningful for knobs whose
    /// range has a centered default — `false` for gain/freq/Q/low-cut,
    /// where TotalMix draws just the plain tick.
    pub arc_from_center: bool,
    /// `false` renders a dimmed, inert knob that ignores all mouse
    /// input — for a slot the reference visually has a knob in, but
    /// `tuxmix-core`/`tuxmix-usb` has no real control backing yet (e.g.
    /// an Output strip's own pan/balance — see `widgets/strip.rs`'s
    /// `full_strip` doc comment on why that one specifically has no
    /// working control). A knob that's just visually absent (a blank
    /// spacer) reads as "control not built yet"; a knob that's present
    /// but greyed out reads as "not applicable to this channel" — a
    /// clearer, more honest signal than either a working-looking fake or
    /// nothing at all.
    pub interactive: bool,
    /// `true` maps `value`↔drag-position logarithmically instead of
    /// linearly — for frequency knobs (EQ band freq, low-cut freq),
    /// where `range` spans 20-20,000 Hz. A linear mapping there
    /// compresses almost the entire musically-useful range (20 Hz-2
    /// kHz — bass through low-mid, most of what EQ work actually
    /// targets) into a sliver of the knob's drag travel, while the top
    /// octave (10-20 kHz) alone eats roughly a quarter of it. Every
    /// other knob (pan/gain/Q/pitch/width/trim) stays linear —
    /// correct there, since those values are already additive (dB,
    /// percent, a linear pan position) rather than multiplicative like
    /// frequency. Requires `range` to be strictly positive on both
    /// ends; falls back to linear otherwise rather than taking `ln`
    /// of a non-positive number.
    pub log_scale: bool,
    pub modifiers: Modifiers,
    pub scale: f32,
    pub on_change: Box<dyn Fn(f32) -> Message>,
    pub on_reset: Box<dyn Fn() -> Message>,
}

#[derive(Default)]
pub struct State {
    dragging: bool,
    last_click: Option<Instant>,
    /// See `Fader::State::drag_pos` — re-read fresh every move rather than
    /// anchored at press time, so Shift can toggle mid-drag without a jump.
    drag_pos: Option<f32>,
    /// See `Fader::State::drag_t` — accumulated across the drag rather
    /// than re-derived from `self.value`, so a batch of several
    /// `CursorMoved` events per rendered frame doesn't drop all but the
    /// last one.
    drag_t: Option<f32>,
    scroll_t: Option<f32>,
    last_scroll: Option<Instant>,
    /// See `Fader::State::display` — eases in value changes that didn't
    /// come from this knob's own drag (group selection, scene load).
    display: Option<MeterFrame>,
}

impl<Message> Knob<Message> {
    fn value_to_t(&self, value: f32) -> f32 {
        let (lo, hi) = self.range;
        if hi <= lo {
            return 0.0;
        }
        if self.log_scale && lo > 0.0 {
            let (log_lo, log_hi) = (lo.ln(), hi.ln());
            ((value.max(f32::MIN_POSITIVE).ln() - log_lo) / (log_hi - log_lo)).clamp(0.0, 1.0)
        } else {
            ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
        }
    }

    fn t_to_value(&self, t: f32) -> f32 {
        let (lo, hi) = self.range;
        let t = t.clamp(0.0, 1.0);
        if self.log_scale && lo > 0.0 && hi > lo {
            let (log_lo, log_hi) = (lo.ln(), hi.ln());
            (log_lo + t * (log_hi - log_lo)).exp()
        } else {
            lo + t * (hi - lo)
        }
    }
}

fn angle_of(t: f32) -> Radians {
    Radians((START_DEG + t.clamp(0.0, 1.0) * SWEEP_DEG).to_radians())
}

impl<Message> canvas::Program<Message> for Knob<Message> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.interactive {
            return None;
        }
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_over(bounds)?;
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
                state.drag_pos = Some(pos.y);
                state.drag_t = Some(self.value_to_t(self.value));
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if !state.dragging {
                    return None;
                }
                let pos = cursor.land().position()?;
                let prev_y = state.drag_pos.unwrap_or(pos.y);
                let screen_dt = -(pos.y - prev_y) / (DRAG_PX * self.scale);
                let mult = if self.modifiers.shift() {
                    FINE_SENSITIVITY
                } else {
                    1.0
                };
                let base_t = state.drag_t.unwrap_or_else(|| self.value_to_t(self.value));
                let t = (base_t + screen_dt * mult).clamp(0.0, 1.0);
                state.drag_pos = Some(pos.y);
                state.drag_t = Some(t);
                Some(canvas::Action::publish((self.on_change)(self.t_to_value(t))).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if !state.dragging {
                    return None;
                }
                state.dragging = false;
                state.drag_pos = None;
                state.drag_t = None;
                state.display = Some(MeterFrame::still(self.value));
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !cursor.is_over(bounds) {
                    return None;
                }
                // Ctrl+scroll never nudges a value here — see the
                // identical guard in `Fader::update`.
                if self.modifiers.control() {
                    return None;
                }
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
                    self.value_to_t(self.value)
                } else {
                    state.scroll_t.unwrap_or_else(|| self.value_to_t(self.value))
                };
                let t = (base_t + dy * base_step * mult).clamp(0.0, 1.0);
                state.scroll_t = Some(t);
                state.last_scroll = Some(now);
                Some(canvas::Action::publish((self.on_change)(self.t_to_value(t))).and_capture())
            }
            // See `Fader::update`'s identical arm.
            canvas::Event::Window(window::Event::RedrawRequested(now)) => {
                if state.dragging {
                    return None;
                }
                let display = state.display.get_or_insert_with(|| MeterFrame::still(self.value));
                if (display.value - self.value).abs() > f32::EPSILON {
                    *display = MeterFrame {
                        prev: display.at(*now),
                        value: self.value,
                        since: *now,
                    };
                }
                if display.is_settling(*now) {
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
        let center = frame.center();
        let radius = (DIAMETER / 2.0) * self.scale;
        let border_w = BORDER_WIDTH * self.scale;
        let tick_len = TICK_LEN * self.scale;

        let display_value = if state.dragging {
            self.value
        } else {
            state.display.map(|d| d.at(Instant::now())).unwrap_or(self.value)
        };
        let t = self.value_to_t(display_value);
        // Dims everything but the face (which is already dark enough not
        // to need it) when `!interactive` — a faded knob reads as "not
        // applicable here" at a glance, without a second visual language
        // to design for.
        let dim = if self.interactive { 1.0 } else { 0.4 };

        // Face — flat filled disc, near-black like the reference's knob
        // plastic. Used to flip to near-white while dragging (mirroring
        // the fader cap's own drag-lit look), but on a knob that's small
        // enough for the label text to fill most of the face, a full
        // light/dark flip blew the label away entirely — user report:
        // "ça s'allume en blanc, ça fait qu'on voit plus rien." A subtle
        // lighten instead keeps the face dark enough that the label/tick/
        // arc below don't need their own dragging-only color flip to stay
        // readable — one visual state to reason about, not two.
        const DRAG_LIGHTEN: f32 = 0.18;
        let face_color = if state.dragging {
            theme::blend(theme::SURFACE, Color::WHITE, DRAG_LIGHTEN)
        } else {
            theme::SURFACE
        };
        frame.fill(&Path::circle(center, radius.max(1.0)), face_color);
        frame.stroke(
            &Path::circle(center, radius.max(1.0)),
            Stroke::default()
                .with_color(Color { a: dim, ..theme::BORDER })
                .with_width(border_w),
        );

        // Orange arc from the centered (t=0.5) position round to the
        // current value — the real TotalMix pan-knob signature (see
        // `arc_from_center`'s doc comment). Skipped when `!interactive`
        // even if `arc_from_center` is set — there's no real value behind
        // it to point at (see `interactive`'s own doc comment), and an
        // arc implies a live reading a dimmed, inert knob shouldn't.
        if self.arc_from_center && self.interactive {
            let center_angle = angle_of(0.5).0;
            let value_angle = angle_of(t).0;
            let (start_angle, end_angle) = if center_angle <= value_angle {
                (center_angle, value_angle)
            } else {
                (value_angle, center_angle)
            };
            frame.stroke(
                &Path::new(|b| {
                    b.arc(canvas::path::Arc {
                        center,
                        radius: radius - border_w,
                        start_angle: Radians(start_angle),
                        end_angle: Radians(end_angle),
                    })
                }),
                Stroke::default().with_color(theme::ACCENT).with_width(2.5 * self.scale),
            );
        }

        // A single tick at the current angle, poking out past the rim —
        // the precise readout the label text backs up, not a full needle
        // or arc fill (see `Bus_design.png`: just a mark at 12 o'clock
        // when centered). No dragging-only color flip needed here or on
        // the label below (see `face_color`'s own doc comment) — both
        // stay their normal color regardless of drag state.
        let angle = angle_of(t).0;
        let (pc, ps) = (angle.cos(), angle.sin());
        frame.stroke(
            &Path::line(
                iced::Point::new(center.x + pc * (radius - border_w), center.y + ps * (radius - border_w)),
                iced::Point::new(center.x + pc * (radius + tick_len), center.y + ps * (radius + tick_len)),
            ),
            Stroke::default()
                .with_color(Color { a: dim, ..theme::TEXT_SEC })
                .with_width(1.5 * self.scale),
        );

        // Value label, centered in the face — the knob doubles as its own
        // readout instead of a separate text element below it.
        frame.fill_text(canvas::Text {
            content: self.label.clone(),
            position: center,
            color: Color { a: dim, ..theme::TEXT_PRIMARY },
            size: (theme::TEXT_MICRO * self.scale).into(),
            align_x: text::Alignment::Center,
            align_y: alignment::Vertical::Center,
            ..canvas::Text::default()
        });

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if !self.interactive {
            mouse::Interaction::Idle
        } else if state.dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::Idle
        }
    }
}

/// A knob's total footprint (at `scale == 1.0`).
const BOX_SIZE: f32 = DIAMETER + MARGIN * 2.0;

pub fn knob<'a, Message: 'a>(knob: Knob<Message>) -> Element<'a, Message>
where
    Message: Clone,
{
    let size = BOX_SIZE * knob.scale;
    Canvas::new(knob)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_knob(range: (f32, f32), log_scale: bool) -> Knob<()> {
        Knob {
            value: range.0,
            range,
            label: String::new(),
            arc_from_center: false,
            interactive: true,
            log_scale,
            modifiers: Modifiers::default(),
            scale: 1.0,
            on_change: Box::new(|_| ()),
            on_reset: Box::new(|| ()),
        }
    }

    #[test]
    fn log_scale_midpoint_t_is_the_geometric_not_arithmetic_mean() {
        // A linear-mapped 20-20,000 Hz knob would put t=0.5 at 10,010 Hz
        // (the arithmetic mean) — exactly the bug this field fixes: the
        // entire bass/low-mid range (20 Hz-2 kHz) would be squeezed into
        // a sliver of the knob's travel. Logarithmic mapping puts t=0.5
        // at the *geometric* mean instead (~632 Hz for this range).
        let k = test_knob((20.0, 20_000.0), true);
        let midpoint = k.t_to_value(0.5);
        let geometric_mean = (20.0f32 * 20_000.0).sqrt();
        assert!(
            (midpoint - geometric_mean).abs() < 1.0,
            "expected ~{geometric_mean} (geometric mean), got {midpoint}"
        );
        assert!(
            midpoint < 10_010.0 - 1000.0,
            "midpoint {midpoint} should be far below the arithmetic mean 10,010"
        );
    }

    #[test]
    fn log_scale_round_trips_value_and_t() {
        let k = test_knob((20.0, 20_000.0), true);
        for hz in [20.0, 100.0, 1000.0, 5000.0, 20_000.0] {
            let t = k.value_to_t(hz);
            let back = k.t_to_value(t);
            assert!((back - hz).abs() / hz < 0.001, "hz={hz} round-tripped to {back}");
        }
    }

    #[test]
    fn non_log_knob_stays_linear() {
        // A pan/gain-style knob (log_scale: false) must be completely
        // unaffected — the midpoint is the plain arithmetic mean.
        let k = test_knob((-100.0, 100.0), false);
        assert_eq!(k.t_to_value(0.5), 0.0);
    }

    #[test]
    fn angle_at_zero_is_start_angle() {
        let a = angle_of(0.0);
        assert!((a.0 - START_DEG.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn angle_at_one_is_full_sweep() {
        let a = angle_of(1.0);
        assert!((a.0 - (START_DEG + SWEEP_DEG).to_radians()).abs() < 1e-5);
    }

    #[test]
    fn angle_at_half_is_the_midpoint() {
        let a = angle_of(0.5);
        let expected = (START_DEG + SWEEP_DEG / 2.0).to_radians();
        assert!((a.0 - expected).abs() < 1e-5);
    }

    #[test]
    fn angle_clamps_out_of_range_t() {
        assert_eq!(angle_of(-1.0).0, angle_of(0.0).0);
        assert_eq!(angle_of(2.0).0, angle_of(1.0).0);
    }
}
