//! The "toggle right sidebar" icon, drawn rather than typed: a rounded
//! window outline with its right-hand column marked off, the column
//! filled while the sidebar is open. It is the convention editors and
//! design tools use for the same button, so it needs no label.
//!
//! Inter has no such glyph, and drawing it also keeps the ink centered
//! by construction at any scale. Coordinates are computed from the frame
//! *size* only, never from `bounds`: a `canvas::Frame`'s origin is the
//! widget's top-left corner, while `Rectangle::center()` works in layout
//! coordinates, so mixing the two draws outside the frame, invisibly.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

/// Outline size at scale 1.
const ICON_W: f32 = 14.0;
const ICON_H: f32 = 11.0;
/// Width of the marked-off sidebar column inside the outline, at scale 1.
const COLUMN_W: f32 = 4.5;
const RADIUS: f32 = 2.0;
const STROKE: f32 = 1.3;

pub struct PanelIcon {
    pub color: Color,
    /// Whether the sidebar is currently open (its column is drawn filled).
    pub open: bool,
    pub scale: f32,
}

/// The outline's rectangle, centered in a `size`-sized frame. Pure, so
/// the centering is pinned by a test instead of by eye.
fn outline(size: Size, scale: f32) -> Rectangle {
    let (w, h) = (ICON_W * scale, ICON_H * scale);
    Rectangle::new(
        Point::new((size.width - w) / 2.0, (size.height - h) / 2.0),
        Size::new(w, h),
    )
}

impl<Message> canvas::Program<Message> for PanelIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let r = outline(bounds.size(), self.scale);
        let stroke = Stroke::default()
            .with_color(self.color)
            .with_width(STROKE * self.scale);
        let column_x = r.x + r.width - COLUMN_W * self.scale;

        if self.open {
            frame.fill(
                &Path::new(|p| {
                    p.rounded_rectangle(
                        Point::new(column_x, r.y),
                        Size::new(COLUMN_W * self.scale, r.height),
                        iced::border::Radius::default()
                            .top_right(RADIUS * self.scale)
                            .bottom_right(RADIUS * self.scale),
                    )
                }),
                Color {
                    a: 0.55,
                    ..self.color
                },
            );
        }
        frame.stroke(
            &Path::new(|p| {
                p.rounded_rectangle(r.position(), r.size(), (RADIUS * self.scale).into())
            }),
            stroke,
        );
        frame.stroke(
            &Path::line(
                Point::new(column_x, r.y),
                Point::new(column_x, r.y + r.height),
            ),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::{outline, ICON_H, ICON_W};
    use iced::Size;

    #[test]
    fn the_outline_is_centered_in_the_frame_at_any_scale() {
        for scale in [0.75, 1.0, 1.75] {
            let size = Size::new(26.0 * scale, 24.0 * scale);
            let r = outline(size, scale);
            assert!((r.x + r.width / 2.0 - size.width / 2.0).abs() < 1e-4);
            assert!((r.y + r.height / 2.0 - size.height / 2.0).abs() < 1e-4);
            assert_eq!(r.width, ICON_W * scale);
            assert_eq!(r.height, ICON_H * scale);
        }
    }
}
