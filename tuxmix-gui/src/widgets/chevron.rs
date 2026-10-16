//! The sidebar fold tab's chevron, drawn rather than typed.
//!
//! The tab is `sidebar::TAB_W` (14px at scale 1) wide, and every
//! arrow-ish glyph tried there (`›`, `▸`) carries a side bearing that
//! pushes its ink visibly off the capsule's center at that width; a
//! canvas chevron is centered by construction at any scale, and crisper
//! than a scaled rasterized triangle besides.
//!
//! Coordinates are computed from the *size* only, never from `bounds`
//! itself: a `canvas::Frame`'s origin is the widget's top-left corner,
//! while `Rectangle::center()` works in layout coordinates (the widget's
//! absolute position), so feeding one into the other draws the whole
//! chevron outside the frame, invisibly. Caught by looking at the actual
//! screenshot, not by the (self-consistent) centering math.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

/// Half-height of the chevron's arms, at scale 1.
const HALF_H: f32 = 4.0;
/// How far the tip sticks out from the base's x, at scale 1. Smaller than
/// `HALF_H` so the shape reads as an arrow, not a box.
const DX: f32 = 2.5;
/// Stroke width at scale 1.
const STROKE: f32 = 1.6;

pub struct Chevron {
    pub color: Color,
    /// `true` = points right (the panel is about to fold away to the
    /// right), `false` = points left (bring it back).
    pub right: bool,
    pub scale: f32,
}

/// The three points of the chevron's polyline, around the center of a
/// `size`-sized frame: the tip at `center.x ± DX`, the two arm ends
/// mirroring each other across both axes. Pure, so the centering is
/// pinned by a test instead of by eye.
fn points(size: Size, right: bool, scale: f32) -> [Point; 3] {
    let center = Point::new(size.width / 2.0, size.height / 2.0);
    let half_h = HALF_H * scale;
    let dx = if right { DX * scale } else { -DX * scale };
    [
        Point::new(center.x - dx, center.y - half_h),
        Point::new(center.x + dx, center.y),
        Point::new(center.x - dx, center.y + half_h),
    ]
}

impl<Message> canvas::Program<Message> for Chevron {
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
        let [base_top, tip, base_bottom] = points(bounds.size(), self.right, self.scale);
        let path = Path::new(|p| {
            p.move_to(base_top);
            p.line_to(tip);
            p.line_to(base_bottom);
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(self.color)
                .with_width(STROKE * self.scale),
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::{points, DX, HALF_H};
    use iced::{Point, Size};

    /// The whole reason this widget exists instead of a font glyph: the
    /// drawn ink has to be centered in the frame, at any scale.
    #[test]
    fn the_chevron_bbox_is_centered_in_the_frame() {
        for scale in [0.75, 1.0, 1.75] {
            let size = Size::new(14.0 * scale, 52.0 * scale);
            let [a, b, c] = points(size, true, scale);
            let center = Point::new(size.width / 2.0, size.height / 2.0);

            let xs = [a.x, b.x, c.x];
            let ys = [a.y, b.y, c.y];
            let x_mid = (xs.iter().cloned().fold(f32::MAX, f32::min)
                + xs.iter().cloned().fold(f32::MIN, f32::max))
                / 2.0;
            let y_mid = (ys.iter().cloned().fold(f32::MAX, f32::min)
                + ys.iter().cloned().fold(f32::MIN, f32::max))
                / 2.0;
            assert!(
                (x_mid - center.x).abs() < 1e-4 && (y_mid - center.y).abs() < 1e-4,
                "scale {scale}: ink center {x_mid},{y_mid} must match frame center {center:?}"
            );
            // The arms mirror across the tip's row, and the tip points
            // the way asked.
            assert_eq!(a.y, center.y - HALF_H * scale);
            assert_eq!(c.y, center.y + HALF_H * scale);
            assert_eq!(b.x, center.x + DX * scale);
        }
    }

    #[test]
    fn direction_flips_the_tip_across_the_center() {
        let size = Size::new(14.0, 52.0);
        let [_, right_tip, _] = points(size, true, 1.0);
        let [_, left_tip, _] = points(size, false, 1.0);
        assert_eq!(right_tip.x, size.width / 2.0 + DX);
        assert_eq!(left_tip.x, size.width / 2.0 - DX);
    }
}
