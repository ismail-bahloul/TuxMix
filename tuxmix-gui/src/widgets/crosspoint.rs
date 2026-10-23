//! One Matrix crosspoint: a gain cell, drawn and driven the way TotalMix's
//! own Matrix is (docs.rme-audio.com, "The Matrix").
//!
//! * Drag up/down from the cell to change its gain. The drag is *relative*:
//!   pressing never moves the value, and the full -inf..+6 dB range takes
//!   `DRAG_TRAVEL` pixels, not the cell's own 20. Shift drags finely.
//! * Ctrl+click toggles the crosspoint between unrouted and 0 dB ("Two
//!   green 0.0 dB field pop in, another click removes them").
//! * Double-click recalls the caller's reset value.
//! * The mouse wheel is left alone, so it scrolls the grid. These cells
//!   used to be hidden faders, which took every wheel event (a wheel
//!   scroll over the grid rewrote crosspoints instead of scrolling) and
//!   mapped the whole range onto the cell's 20px height.
//! * Colors follow TotalMix: green = 0 dB, raised grey + number = any
//!   other gain, blue = the source is muted, red = its phase is inverted,
//!   a dark well = no routing.

use std::time::Instant;

use iced::advanced::text as advanced_text;
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::widgets::fader::{t_to_vol, vol_to_t, DOUBLE_CLICK, FINE_SENSITIVITY};

/// Pointer travel, at scale 1, that sweeps the whole fader range.
const DRAG_TRAVEL: f32 = 150.0;
/// Within this many dB of 0 a crosspoint counts as unity (green).
const UNITY_DB: f32 = 0.05;

pub struct Crosspoint<Message> {
    /// The stored linear gain the gestures start from; `0.0` = unrouted.
    pub value: f32,
    /// What the cell shows and is colored by, when that differs from
    /// `value`: a mono source's crosspoint is one stored gain plus a pan,
    /// decoded per side for display, while a drag keeps moving the stored
    /// gain (starting from the displayed number would jump it).
    pub shown: f32,
    /// `false` for the side of an output pair the source has no hardware
    /// path to: drawn as an empty well, inert.
    pub interactive: bool,
    pub muted: bool,
    pub phase: bool,
    /// Column of the submix picked in the top bar.
    pub in_sel: bool,
    /// Whether the app holds this cell as the hovered one. The cell
    /// compares the pointer against this, not against a flag of its own:
    /// during a drag the pointer crosses other cells, whose enter/exit
    /// messages move the app's hover away, and a local flag would then
    /// never announce this cell again when the pointer comes back.
    pub hovered: bool,
    pub modifiers: Modifiers,
    pub scale: f32,
    pub reset_value: f32,
    pub on_change: Box<dyn Fn(f32) -> Message>,
    /// End of a press/drag gesture (closes its undo step).
    pub on_release: Message,
    /// The pointer entered (`true`) or left the cell.
    pub on_hover: Box<dyn Fn(bool) -> Message>,
}

#[derive(Default)]
pub struct State {
    pressed: bool,
    dragging: bool,
    last_y: Option<f32>,
    drag_t: f32,
    last_click: Option<Instant>,
}

/// The cell text: bare dB with one decimal, signed when above unity.
pub fn cell_text(vol: f32) -> Option<String> {
    (vol > f32::EPSILON).then(|| {
        let db = 20.0 * vol.log10();
        if db.abs() < UNITY_DB {
            "0.0".to_string()
        } else if db > 0.0 {
            format!("+{db:.1}")
        } else {
            format!("{db:.1}")
        }
    })
}

/// The Ctrl+click toggle: a routed crosspoint goes off, an unrouted one
/// to 0 dB.
pub fn toggled(value: f32) -> f32 {
    if value > f32::EPSILON {
        0.0
    } else {
        1.0
    }
}

/// One drag step: `dy` pixels (down = positive), in tapered fader space so
/// the feel is even across the dB range.
pub fn dragged(start_t: f32, dy: f32, scale: f32, fine: bool) -> f32 {
    let travel = DRAG_TRAVEL * scale.max(0.1);
    let mult = if fine { FINE_SENSITIVITY } else { 1.0 };
    (start_t - dy / travel * mult).clamp(0.0, 1.0)
}

impl<Message: Clone> canvas::Program<Message> for Crosspoint<Message> {
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
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.pressed {
                    let y = cursor.land().position()?.y;
                    let last = state.last_y.replace(y).unwrap_or(y);
                    if y == last {
                        return None;
                    }
                    state.dragging = true;
                    state.drag_t =
                        dragged(state.drag_t, y - last, self.scale, self.modifiers.shift());
                    return Some(
                        canvas::Action::publish((self.on_change)(t_to_vol(state.drag_t)))
                            .and_capture(),
                    );
                }
                let over = cursor.is_over(bounds);
                (over != self.hovered).then(|| canvas::Action::publish((self.on_hover)(over)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_over(bounds)?;
                let now = Instant::now();
                let double = state
                    .last_click
                    .is_some_and(|t| now.duration_since(t) < DOUBLE_CLICK);
                state.last_click = if double { None } else { Some(now) };
                state.pressed = true;
                state.dragging = false;
                state.last_y = Some(pos.y);
                state.drag_t = vol_to_t(self.value);
                let value = if double {
                    self.reset_value
                } else if self.modifiers.control() {
                    toggled(self.value)
                } else {
                    // A plain press only arms a drag: nothing changes, so
                    // nothing is published (an unchanged value would still
                    // open an empty undo step). Captured all the same, so
                    // the page underneath does not clear the selection.
                    return Some(canvas::Action::request_redraw().and_capture());
                };
                state.drag_t = vol_to_t(value);
                Some(canvas::Action::publish((self.on_change)(value)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if !state.pressed {
                    return None;
                }
                state.pressed = false;
                state.dragging = false;
                state.last_y = None;
                Some(canvas::Action::publish(self.on_release.clone()).and_capture())
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
        let routed = self.interactive && self.shown > f32::EPSILON;
        let well = if self.in_sel {
            theme::SURFACE_HI
        } else {
            theme::blend(theme::BG_DEEP, theme::SURFACE, 0.6)
        };
        let unity = routed && (20.0 * self.shown.log10()).abs() < UNITY_DB;
        let fill = if !routed {
            well
        } else if self.muted {
            theme::blend(well, theme::MUTE_COLOR, 0.38)
        } else if self.phase {
            theme::blend(well, theme::MRED, 0.38)
        } else if unity {
            theme::blend(well, theme::MGREEN, 0.38)
        } else {
            theme::blend(well, theme::FADER, 0.16)
        };
        let radius = theme::RADIUS_XS * self.scale;
        let cell = Path::new(|p| p.rounded_rectangle(Point::ORIGIN, bounds.size(), radius.into()));
        frame.fill(&cell, fill);

        let ring = if state.dragging {
            Some(theme::ACCENT)
        } else if self.interactive && self.hovered {
            Some(theme::FADER)
        } else if self.in_sel {
            Some(theme::BORDER)
        } else {
            None
        };
        if let Some(color) = ring {
            let inset = Path::new(|p| {
                p.rounded_rectangle(
                    Point::new(0.5, 0.5),
                    Size::new(bounds.width - 1.0, bounds.height - 1.0),
                    radius.into(),
                )
            });
            frame.stroke(&inset, Stroke::default().with_color(color).with_width(1.0));
        }

        if let Some(label) = self.interactive.then(|| cell_text(self.shown)).flatten() {
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(bounds.width / 2.0, bounds.height / 2.0),
                color: if routed {
                    theme::TEXT_PRIMARY
                } else {
                    theme::TEXT_SEC
                },
                size: (theme::TEXT_XS * self.scale).into(),
                align_x: advanced_text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if !self.interactive {
            return mouse::Interaction::None;
        }
        if state.pressed || cursor.is_over(bounds) {
            mouse::Interaction::ResizingVertically
        } else {
            mouse::Interaction::None
        }
    }
}

pub fn crosspoint<'a, Message: Clone + 'a>(
    cell: Crosspoint<Message>,
    width: f32,
    height: f32,
) -> Element<'a, Message> {
    Canvas::new(cell)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_text_signs_gain_above_unity_and_blanks_unrouted() {
        assert_eq!(cell_text(0.0), None);
        assert_eq!(cell_text(1.0).as_deref(), Some("0.0"));
        assert_eq!(cell_text(2.0).as_deref(), Some("+6.0"));
        assert_eq!(cell_text(0.5).as_deref(), Some("-6.0"));
    }

    #[test]
    fn ctrl_click_toggles_between_off_and_unity() {
        assert_eq!(toggled(0.0), 1.0);
        assert_eq!(toggled(0.4), 0.0);
    }

    #[test]
    fn a_drag_needs_real_travel_to_sweep_the_range() {
        // A 20px move (one cell height) used to sweep everything.
        let t = dragged(0.5, -20.0, 1.0, false);
        assert!(t > 0.5 && t < 0.7, "20px up moves {t}");
        assert_eq!(dragged(0.5, -DRAG_TRAVEL, 1.0, false), 1.0);
        assert_eq!(dragged(0.5, DRAG_TRAVEL, 1.0, false), 0.0);
        // Fine drag moves much less for the same travel.
        assert!(dragged(0.5, -20.0, 1.0, true) - 0.5 < (t - 0.5) / 2.0);
    }
}
