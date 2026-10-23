//! A thin vertical drag strip: a splitter for the sidebar's inner edge.
//!
//! iced ships no splitter widget, and `mouse_area` can't stand in for one
//! honestly — it reports no cursor position, so a drag could only be
//! reconstructed through a window-wide event subscription, and it always
//! asks for the `Pointer` cursor. A `canvas::Program` gets the raw mouse
//! events *with* their coordinates, so it can emit drag deltas itself and
//! ask for the `ResizingHorizontally` cursor. Same shape as `fader.rs`,
//! minus most of the drawing: at rest the strip is the panel's own 1px
//! edge line, hover and drag thicken it into a 2px handle.

use std::rc::Rc;

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Theme};

use crate::theme;

/// The indicator line drawn across the strip while it is hovered or
/// dragged — narrow enough to read as "the edge is live" rather than as a
/// separator, which the mixer/sidebar boundary already is.
const LINE_W: f32 = 2.0;

pub struct Splitter<Message> {
    /// Called with the horizontal pointer movement since the previous
    /// event, in *logical* pixels (the caller's `ui_scale` is baked in by
    /// `splitter`), so callers can add it to their own layout constants
    /// without converting anything.
    on_drag: Rc<dyn Fn(f32) -> Message>,
    /// `ui_scale` at build time — the indicator line's own width.
    scale: f32,
}

/// Whether a drag is in flight. Lives in the widget tree (like the fader's
/// own state) rather than in `TuxMix`: nothing outside this widget needs to
/// know a resize gesture is happening, and keeping it here is what lets a
/// drag continue while the pointer is outside the strip's 6px (see
/// `land`, below).
#[derive(Debug, Default)]
pub struct State {
    dragging: bool,
    last_x: Option<f32>,
}

impl<Message> Splitter<Message> {
    /// `on_drag` receives a *delta* rather than an absolute position on
    /// purpose: deltas need no window width, no `x` of the strip, and stay
    /// correct while the strip itself moves under the pointer (which it
    /// does, since the drag is what moves it).
    pub fn new(on_drag: impl Fn(f32) -> Message + 'static) -> Self {
        Self {
            on_drag: Rc::new(on_drag),
            scale: 1.0,
        }
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

impl<Message> canvas::Program<Message> for Splitter<Message> {
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
                cursor.position_over(bounds)?;
                state.dragging = true;
                state.last_x = cursor.land().position().map(|p| p.x);
                Some(canvas::Action::request_redraw().and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if !state.dragging {
                    return None;
                }
                // `land`, not `position_over`: a resize drag leaves this
                // 6px-wide strip on the very first move, and the gesture has
                // to keep tracking the pointer outside it.
                let x = cursor.land().position()?.x;
                let last = state.last_x.replace(x).unwrap_or(x);
                if x == last {
                    return None;
                }
                Some(canvas::Action::publish((self.on_drag)(x - last)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if !state.dragging {
                    return None;
                }
                state.dragging = false;
                state.last_x = None;
                Some(canvas::Action::request_redraw().and_capture())
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
        if !state.dragging && !cursor.is_over(bounds) {
            // The panel's edge, on the strip's outer side.
            let x = self.scale.max(1.0) / 2.0;
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
                Stroke::default()
                    .with_width(self.scale.max(1.0))
                    .with_color(theme::BORDER),
            );
            return vec![frame.into_geometry()];
        }
        let x = bounds.width / 2.0;
        frame.stroke(
            &Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)),
            Stroke::default()
                .with_width(LINE_W * self.scale)
                .with_color(if state.dragging {
                    theme::ACCENT
                } else {
                    theme::FADER
                }),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::ResizingHorizontally
        } else {
            mouse::Interaction::None
        }
    }
}

/// `scale` is the caller's `ui_scale`; `on_drag` is wrapped so its own
/// argument is already divided by it (see `Splitter::new`).
pub fn splitter<'a, Message: 'a>(
    on_drag: impl Fn(f32) -> Message + 'static,
    scale: f32,
    width: f32,
) -> Element<'a, Message>
where
    Message: Clone,
{
    Canvas::new(Splitter::new(move |dx| on_drag(dx / scale)).scale(scale))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .into()
}
