// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Ismaïl Bahloul

//! The thin FX fader at the top of a strip's right column, as TotalMix FX
//! has it: the strip's send into the effects (inputs, playback) or the
//! return of the effects into an output. -65 .. 0 dB, linear in dB;
//! double-click turns it off, the wheel nudges it a dB at a time.

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;

const DB_MIN: f32 = -65.0;
const DB_MAX: f32 = 0.0;
const RAIL_W: f32 = 4.0;
const CAP_W: f32 = 12.0;
const CAP_H: f32 = 16.0;

struct FxSlider<Message> {
    db: f32,
    scale: f32,
    on_change: Box<dyn Fn(f32) -> Message>,
    on_reset: Message,
}

#[derive(Default)]
struct State {
    dragging: bool,
    last_click: Option<std::time::Instant>,
}

impl<Message> FxSlider<Message> {
    /// dB at cursor height `y` (canvas-local): the cap's centre follows it.
    fn db_at(&self, y: f32, height: f32) -> f32 {
        let cap = CAP_H * self.scale;
        let span = (height - cap).max(1.0);
        let t = 1.0 - ((y - cap / 2.0) / span).clamp(0.0, 1.0);
        (DB_MIN + t * (DB_MAX - DB_MIN)).round()
    }
}

impl<Message: Clone> canvas::Program<Message> for FxSlider<Message> {
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
                let now = std::time::Instant::now();
                let double = state
                    .last_click
                    .is_some_and(|t| now.duration_since(t) < super::fader::DOUBLE_CLICK);
                state.last_click = Some(now);
                if double {
                    state.dragging = false;
                    return Some(canvas::Action::publish(self.on_reset.clone()).and_capture());
                }
                state.dragging = true;
                let db = self.db_at(pos.y - bounds.y, bounds.height);
                Some(canvas::Action::publish((self.on_change)(db)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let db = self.db_at(position.y - bounds.y, bounds.height);
                Some(canvas::Action::publish((self.on_change)(db)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging =>
            {
                state.dragging = false;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_over(bounds)?;
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                if dy == 0.0 {
                    return None;
                }
                let db = (self.db + dy.signum()).clamp(DB_MIN, DB_MAX);
                Some(canvas::Action::publish((self.on_change)(db)).and_capture())
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
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = self.scale;
        let cap_h = CAP_H * s;
        let cap_w = CAP_W * s;
        let rail_w = RAIL_W * s;
        let x = bounds.width / 2.0;
        let t = ((self.db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
        let cap_y = (1.0 - t) * (bounds.height - cap_h);

        let rail = Path::rectangle(
            Point::new(x - rail_w / 2.0, cap_h / 2.0),
            Size::new(rail_w, (bounds.height - cap_h).max(0.0)),
        );
        frame.fill(&rail, theme::BG_DEEP);
        if t > 0.0 {
            let top = cap_y + cap_h / 2.0;
            let fill = Path::rectangle(
                Point::new(x - rail_w / 2.0, top),
                Size::new(rail_w, (bounds.height - cap_h / 2.0 - top).max(0.0)),
            );
            frame.fill(&fill, theme::FX_COLOR);
        }

        let hot = state.dragging || cursor.is_over(bounds);
        let cap = Path::rounded_rectangle(
            Point::new(x - cap_w / 2.0, cap_y),
            Size::new(cap_w, cap_h),
            (3.0 * s).into(),
        );
        frame.fill(&cap, theme::SURFACE_HI);
        frame.stroke(
            &cap,
            Stroke::default()
                .with_width(1.0)
                .with_color(if hot { theme::FADER } else { theme::TEXT_SEC }),
        );
        let mid = Path::line(
            Point::new(x - 3.0 * s, cap_y + cap_h / 2.0),
            Point::new(x + 3.0 * s, cap_y + cap_h / 2.0),
        );
        frame.stroke(
            &mid,
            Stroke::default().with_width(1.0).with_color(theme::FADER),
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
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::None
        }
    }
}

/// See the module docs. `width`/`height` are in pixels (already scaled).
pub fn fx_slider<'a, Message: Clone + 'a>(
    db: f32,
    width: f32,
    height: f32,
    scale: f32,
    on_change: impl Fn(f32) -> Message + 'static,
    on_reset: Message,
) -> Element<'a, Message> {
    Canvas::new(FxSlider {
        db,
        scale,
        on_change: Box::new(on_change),
        on_reset,
    })
    .width(Length::Fixed(width))
    .height(Length::Fixed(height))
    .into()
}
