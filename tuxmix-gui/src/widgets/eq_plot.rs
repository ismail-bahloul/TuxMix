//! Frequency-response plot for the hardware EQ: the three parametric
//! bands plus the low cut, drawn on a log-frequency axis.
//!
//! The curve is a visual model, not a bit-exact copy of the device's own
//! fixed-point coefficients: the bands use the standard RBJ biquads at the
//! current sample rate, and the low cut is a Butterworth high-pass of
//! order `slope / 6`. That is enough to see the shape and where a band
//! sits, which is what a mixer's EQ plot is for; the device still does the
//! real filtering.

use std::f32::consts::PI;

use iced::advanced::text as advanced_text;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{mouse, Point, Rectangle, Renderer, Size, Theme};
use tuxmix_core::channel::{EqBand, EqBandType, InputEq};

use crate::theme;

/// Plot height at scale 1.
const PLOT_H: f32 = 108.0;
const MARGIN_LEFT: f32 = 26.0;
const MARGIN_RIGHT: f32 = 6.0;
const MARGIN_TOP: f32 = 6.0;
const MARGIN_BOTTOM: f32 = 14.0;

const DB_MIN: f32 = -24.0;
const DB_MAX: f32 = 24.0;
const F_MIN: f32 = 20.0;
const F_MAX: f32 = 20_000.0;

/// Samples along the curve. Enough for the Q=10 peaks to read as peaks at
/// any window width without rebuilding the path per pixel.
const CURVE_POINTS: usize = 180;

pub fn eq_plot<'a, Message: 'a>(
    eq: InputEq,
    sample_rate: u32,
    scale: f32,
) -> iced::Element<'a, Message> {
    canvas::Canvas::new(EqPlot {
        eq,
        sample_rate,
        scale,
    })
    .width(iced::Length::Fill)
    .height(iced::Length::Fixed(PLOT_H * scale))
    .into()
}

struct EqPlot {
    eq: InputEq,
    sample_rate: u32,
    scale: f32,
}

impl<Message> canvas::Program<Message> for EqPlot {
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
        let scale = self.scale;
        let plot = Rectangle::new(
            Point::new(MARGIN_LEFT * scale, MARGIN_TOP * scale),
            Size::new(
                (bounds.width - (MARGIN_LEFT + MARGIN_RIGHT) * scale).max(1.0),
                (bounds.height - (MARGIN_TOP + MARGIN_BOTTOM) * scale).max(1.0),
            ),
        );

        frame.fill_rectangle(
            Point::new(plot.x, plot.y),
            Size::new(plot.width, plot.height),
            theme::BG_DEEP,
        );

        let grid = Stroke::default()
            .with_color(theme::BORDER)
            .with_width(scale);
        for db in [-24.0, -12.0, 0.0, 12.0, 24.0] {
            let y = db_to_y(db, plot);
            let zero = db == 0.0;
            frame.stroke(
                &Path::line(Point::new(plot.x, y), Point::new(plot.x + plot.width, y)),
                if zero {
                    Stroke::default()
                        .with_color(theme::ACCENT_DIM)
                        .with_width(scale)
                } else {
                    grid.clone()
                },
            );
        }
        for f in [
            20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0,
        ] {
            let x = freq_to_x(f, plot);
            frame.stroke(
                &Path::line(Point::new(x, plot.y), Point::new(x, plot.y + plot.height)),
                grid.clone(),
            );
        }

        let rate = if self.sample_rate == 0 {
            48_000.0
        } else {
            self.sample_rate as f32
        };
        let points: Vec<Point> = (0..=CURVE_POINTS)
            .map(|i| {
                let t = i as f32 / CURVE_POINTS as f32;
                let f = F_MIN * (F_MAX / F_MIN).powf(t);
                let db = response_db(&self.eq, f, rate);
                Point::new(
                    plot.x + plot.width * t,
                    db_to_y(db.clamp(DB_MIN, DB_MAX), plot),
                )
            })
            .collect();
        let path = Path::new(|p| {
            if let Some(first) = points.first() {
                p.move_to(*first);
            }
            for point in points.iter().skip(1) {
                p.line_to(*point);
            }
        });
        let color = if self.eq.enabled {
            theme::ACCENT
        } else {
            theme::TEXT_SEC
        };
        frame.stroke(
            &path,
            Stroke::default().with_color(color).with_width(1.6 * scale),
        );

        // The labels borrow the frame mutably; scope them so the frame is
        // free again for `into_geometry` below.
        {
            let mut label = |content: String, position: Point, align_x, align_y| {
                frame.fill_text(Text {
                    content,
                    position,
                    color: theme::TEXT_SEC,
                    size: (theme::TEXT_MICRO * scale).into(),
                    align_x,
                    align_y,
                    ..Text::default()
                });
            };
            label(
                "+24".into(),
                Point::new(MARGIN_LEFT * scale - 3.0 * scale, plot.y),
                advanced_text::Alignment::Right,
                iced::alignment::Vertical::Top,
            );
            label(
                "0".into(),
                Point::new(MARGIN_LEFT * scale - 3.0 * scale, db_to_y(0.0, plot)),
                advanced_text::Alignment::Right,
                iced::alignment::Vertical::Center,
            );
            label(
                "-24".into(),
                Point::new(MARGIN_LEFT * scale - 3.0 * scale, plot.y + plot.height),
                advanced_text::Alignment::Right,
                iced::alignment::Vertical::Bottom,
            );
            for f in [20.0, 100.0, 1_000.0, 10_000.0, 20_000.0] {
                label(
                    freq_label(f).into(),
                    Point::new(freq_to_x(f, plot), plot.y + plot.height + 2.0 * scale),
                    advanced_text::Alignment::Center,
                    iced::alignment::Vertical::Top,
                );
            }
            if !self.eq.enabled {
                label(
                    "BYPASS".into(),
                    Point::new(plot.x + plot.width - 2.0 * scale, plot.y + 2.0 * scale),
                    advanced_text::Alignment::Right,
                    iced::alignment::Vertical::Top,
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

fn db_to_y(db: f32, plot: Rectangle) -> f32 {
    plot.y + plot.height * (DB_MAX - db) / (DB_MAX - DB_MIN)
}

fn freq_to_x(f: f32, plot: Rectangle) -> f32 {
    let t = (f / F_MIN).log10() / (F_MAX / F_MIN).log10();
    plot.x + plot.width * t
}

fn freq_label(f: f32) -> String {
    if f >= 1_000.0 {
        format!("{}k", f / 1_000.0)
    } else {
        format!("{f:.0}")
    }
}

/// Combined response in dB at `f`, summing the enabled bands and the low
/// cut. Bands are independent biquads, so their magnitudes multiply and
/// their dB values add.
fn response_db(eq: &InputEq, f: f32, rate: f32) -> f32 {
    let bands = eq
        .bands
        .iter()
        .map(|band| band_db(band, f, rate))
        .sum::<f32>();
    bands + low_cut_db(eq, f)
}

fn band_db(band: &EqBand, f: f32, rate: f32) -> f32 {
    if band.band_type == EqBandType::Off {
        return 0.0;
    }
    let (b, a) = band_coeffs(band, rate);
    let w = 2.0 * PI * f / rate;
    let (num_re, num_im) = biquad_response(&b, w);
    let (den_re, den_im) = biquad_response(&a, w);
    let mag = ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im)).sqrt();
    20.0 * mag.max(1e-9).log10()
}

/// RBJ cookbook coefficients. Shelves use S = 1, the usual middle ground;
/// the hardware's own shelf Q is not exposed as a separate parameter, so
/// there is nothing more specific to follow.
fn band_coeffs(band: &EqBand, rate: f32) -> ([f32; 3], [f32; 3]) {
    let a = 10f32.powf(band.gain_db / 40.0);
    let w0 = 2.0 * PI * band.freq_hz as f32 / rate;
    let cos = w0.cos();
    let sin = w0.sin();
    match band.band_type {
        EqBandType::Off => ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        EqBandType::Bell => {
            let alpha = sin / (2.0 * band.q.max(0.01));
            (
                [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
                [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
            )
        }
        EqBandType::LowShelf => {
            let sqrt_a = a.sqrt();
            let alpha = sin / 2.0 * 2f32.sqrt();
            (
                [
                    a * ((a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
                ],
                [
                    (a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
                ],
            )
        }
        EqBandType::HighShelf => {
            let sqrt_a = a.sqrt();
            let alpha = sin / 2.0 * 2f32.sqrt();
            (
                [
                    a * ((a + 1.0) + (a - 1.0) * cos + 2.0 * sqrt_a * alpha),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - 2.0 * sqrt_a * alpha),
                ],
                [
                    (a + 1.0) - (a - 1.0) * cos + 2.0 * sqrt_a * alpha,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - 2.0 * sqrt_a * alpha,
                ],
            )
        }
    }
}

/// Real and imaginary parts of the numerator/denominator at `w`.
fn biquad_response(c: &[f32; 3], w: f32) -> (f32, f32) {
    let re = c[0] + c[1] * w.cos() + c[2] * (2.0 * w).cos();
    let im = -(c[1] * w.sin() + c[2] * (2.0 * w).sin());
    (re, im)
}

/// Butterworth high-pass of order `slope / 6`, normalized to 0 dB well
/// above the cutoff and -3 dB at it.
fn low_cut_db(eq: &InputEq, f: f32) -> f32 {
    let order = (eq.low_cut_slope_db_oct.max(6) / 6) as i32;
    let r = f / eq.low_cut_freq_hz.max(1) as f32;
    let mag = r.powi(order) / (1.0 + r.powi(2 * order)).sqrt();
    20.0 * mag.max(1e-9).log10()
}

#[cfg(test)]
mod tests {
    use super::{db_to_y, low_cut_db, response_db, DB_MAX, DB_MIN};
    use iced::{Point, Rectangle, Size};
    use tuxmix_core::channel::{EqBand, EqBandType, InputEq};

    fn bell(gain_db: f32) -> InputEq {
        let mut eq = InputEq::default();
        eq.bands[0] = EqBand {
            band_type: EqBandType::Bell,
            freq_hz: 1_000,
            q: 1.0,
            gain_db,
        };
        eq
    }

    #[test]
    fn bell_peaks_at_its_own_gain() {
        let eq = bell(12.0);
        let db = response_db(&eq, 1_000.0, 48_000.0);
        assert!((db - 12.0).abs() < 0.2, "bell at 1 kHz read {db} dB");
    }

    #[test]
    fn a_default_eq_is_flat_at_one_khz() {
        let eq = InputEq::default();
        let db = response_db(&eq, 1_000.0, 48_000.0);
        assert!(db.abs() < 0.01, "default EQ read {db} dB at 1 kHz");
    }

    #[test]
    fn low_cut_uses_the_expected_slope() {
        let mut eq = InputEq::default();
        eq.low_cut_freq_hz = 1_000;
        eq.low_cut_slope_db_oct = 24;
        let db = low_cut_db(&eq, 500.0);
        assert!((db + 24.0).abs() < 1.0, "24 dB/oct low cut read {db} dB");
    }

    #[test]
    fn zero_db_sits_in_the_vertical_middle() {
        let plot = Rectangle::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0));
        let middle = db_to_y((DB_MIN + DB_MAX) / 2.0, plot);
        assert!((middle - 60.0).abs() < 0.001);
    }
}
