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
        if hi > lo {
            ((value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn t_to_value(&self, t: f32) -> f32 {
        let (lo, hi) = self.range;
        lo + t.clamp(0.0, 1.0) * (hi - lo)
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

        // Face — flat filled disc, near-black like the reference's knob
        // plastic, lit up while dragging the same way the fader cap is.
        let face_color = if state.dragging {
            Color::from_rgb8(0xf0, 0xf1, 0xf4)
        } else {
            theme::SURFACE
        };
        frame.fill(&Path::circle(center, radius.max(1.0)), face_color);
        frame.stroke(
            &Path::circle(center, radius.max(1.0)),
            Stroke::default().with_color(theme::BORDER).with_width(border_w),
        );

        // A single tick at the current angle, poking out past the rim —
        // the precise readout the label text backs up, not a full needle
        // or arc fill (see `Bus_design.png`: just a mark at 12 o'clock
        // when centered).
        //
        // Both the tick and the label below switch to a *dark* color while
        // dragging, not `ACCENT` — the face itself goes near-white then
        // (see `face_color`), and `ACCENT`'s a light cyan, so light-on-light
        // there read as the knob going blank mid-drag rather than lighting
        // up (the fader cap never had this problem: its dB readout is a
        // separate text sibling on the dark strip background, not drawn on
        // the cap itself).
        let angle = angle_of(t).0;
        let (pc, ps) = (angle.cos(), angle.sin());
        let tick_color = if state.dragging { theme::SURFACE } else { theme::TEXT_SEC };
        frame.stroke(
            &Path::line(
                iced::Point::new(center.x + pc * (radius - border_w), center.y + ps * (radius - border_w)),
                iced::Point::new(center.x + pc * (radius + tick_len), center.y + ps * (radius + tick_len)),
            ),
            Stroke::default().with_color(tick_color).with_width(1.5 * self.scale),
        );

        // Value label, centered in the face — the knob doubles as its own
        // readout instead of a separate text element below it.
        let label_color = if state.dragging { theme::SURFACE } else { theme::TEXT_PRIMARY };
        frame.fill_text(canvas::Text {
            content: self.label.clone(),
            position: center,
            color: label_color,
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
        if state.dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::Idle
        }
    }
}

pub fn knob<'a, Message: 'a>(knob: Knob<Message>) -> Element<'a, Message>
where
    Message: Clone,
{
    let size = (DIAMETER + MARGIN * 2.0) * knob.scale;
    Canvas::new(knob)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
