//! Interactive frequency-response editor for the hardware EQ: the three
//! parametric bands plus the low cut, drawn on a log-frequency axis.
//!
//! The curve is a visual model, not a bit-exact copy of the device's own
//! fixed-point coefficients: the bands use the standard RBJ biquads at the
//! current sample rate, and the low cut is a Butterworth high-pass of
//! order `slope / 6`. That is enough to see the shape and where a band
//! sits, which is what a mixer's EQ plot is for; the device still does the
//! real filtering.
//!
//! Drag a band handle to set its frequency and gain at the same time,
//! scroll over a handle to change its Q, and double-click a handle to
//! reset that band. The low-cut handle at the bottom edge moves the
//! cutoff; its slope stays in the buttons below the plot.

use std::f32::consts::PI;
use std::time::Instant;

use iced::advanced::text as advanced_text;
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use tuxmix_core::channel::{EqBand, EqBandType, InputEq};

use super::fader::{DOUBLE_CLICK, SCROLL_IDLE};
use crate::theme;

/// Plot height at scale 1.
const PLOT_H: f32 = 64.0;
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

const HANDLE_R: f32 = 6.0;
const HIT_R: f32 = 13.0;
const LOW_CUT_HANDLE_R: f32 = 5.0;

/// Callbacks for direct manipulation in the plot. They carry one message
/// per interaction so a drag updates frequency and gain in a single
/// `update`, instead of two state changes arriving separately.
pub struct EqPlotActions<Message> {
    pub on_band: Box<dyn Fn(usize, u16, f32) -> Message>,
    pub on_band_reset: Box<dyn Fn(usize) -> Message>,
    pub on_band_q: Box<dyn Fn(usize, f32) -> Message>,
    pub on_low_cut: Box<dyn Fn(u16) -> Message>,
}

pub fn eq_plot<'a, Message: 'a>(
    eq: InputEq,
    sample_rate: u32,
    scale: f32,
    modifiers: Modifiers,
    actions: EqPlotActions<Message>,
) -> iced::Element<'a, Message> {
    canvas::Canvas::new(EqPlot {
        eq,
        sample_rate,
        scale,
        modifiers,
        actions,
    })
    .width(iced::Length::Fill)
    .height(iced::Length::Fixed(PLOT_H * scale))
    .into()
}

#[derive(Default)]
pub struct State {
    drag: Option<Drag>,
    last_click: Option<(Drag, Instant)>,
    last_band: Option<(u16, f32)>,
    last_low_cut: Option<u16>,
    scroll_q: Option<f32>,
    scroll_band: Option<usize>,
    last_scroll: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Band(usize),
    LowCut,
}

struct EqPlot<Message> {
    eq: InputEq,
    sample_rate: u32,
    scale: f32,
    modifiers: Modifiers,
    actions: EqPlotActions<Message>,
}

impl<Message> EqPlot<Message> {
    fn plot_rect(&self, size: Size) -> Rectangle {
        let scale = self.scale;
        Rectangle::new(
            Point::new(MARGIN_LEFT * scale, MARGIN_TOP * scale),
            Size::new(
                (size.width - (MARGIN_LEFT + MARGIN_RIGHT) * scale).max(1.0),
                (size.height - (MARGIN_TOP + MARGIN_BOTTOM) * scale).max(1.0),
            ),
        )
    }

    fn band_handle(&self, band: &EqBand, plot: Rectangle) -> Point {
        let r = HANDLE_R * self.scale;
        Point::new(
            freq_to_x(band.freq_hz as f32, plot).clamp(plot.x + r, plot.x + plot.width - r),
            db_to_y(band.gain_db.clamp(DB_MIN, DB_MAX), plot)
                .clamp(plot.y + r, plot.y + plot.height - r),
        )
    }

    fn low_cut_handle(&self, plot: Rectangle) -> Point {
        let r = LOW_CUT_HANDLE_R * self.scale;
        Point::new(
            freq_to_x(self.eq.low_cut_freq_hz as f32, plot)
                .clamp(plot.x + r, plot.x + plot.width - r),
            plot.y + plot.height,
        )
    }

    fn hit(&self, plot: Rectangle, pos: Point) -> Option<Drag> {
        let mut best: Option<(f32, Drag)> = None;
        for (i, band) in self.eq.bands.iter().enumerate() {
            if band.band_type == EqBandType::Off {
                continue;
            }
            let point = self.band_handle(band, plot);
            let distance = (point.x - pos.x).hypot(point.y - pos.y);
            if distance <= HIT_R * self.scale
                && best.is_none_or(|(best_distance, _)| distance < best_distance)
            {
                best = Some((distance, Drag::Band(i)));
            }
        }

        let point = self.low_cut_handle(plot);
        let distance = (point.x - pos.x).hypot(point.y - pos.y);
        if distance <= (LOW_CUT_HANDLE_R + 4.0) * self.scale
            && best.is_none_or(|(best_distance, _)| distance < best_distance)
        {
            best = Some((distance, Drag::LowCut));
        }

        best.map(|(_, drag)| drag)
    }
}

impl<Message> canvas::Program<Message> for EqPlot<Message> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let plot = self.plot_rect(bounds.size());

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = local_point(bounds, cursor.position_over(bounds)?);
                let target = self.hit(plot, pos)?;
                let now = Instant::now();
                let is_double = state.last_click.is_some_and(|(last, at)| {
                    last == target && now.duration_since(at) < DOUBLE_CLICK
                });

                if is_double {
                    state.last_click = None;
                    state.drag = None;
                    state.last_band = None;
                    state.last_low_cut = None;
                    let message = match target {
                        Drag::Band(band) => (self.actions.on_band_reset)(band),
                        Drag::LowCut => (self.actions.on_low_cut)(20),
                    };
                    return Some(canvas::Action::publish(message).and_capture());
                }

                state.last_click = Some((target, now));
                state.drag = Some(target);
                state.last_band = None;
                state.last_low_cut = None;
                state.scroll_q = None;
                state.scroll_band = None;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let target = state.drag?;
                let pos = local_point(bounds, cursor.land().position()?);
                match target {
                    Drag::Band(band) => {
                        let (freq, gain) = band_at(plot, pos);
                        if state.last_band == Some((freq, gain)) {
                            return None;
                        }
                        state.last_band = Some((freq, gain));
                        Some(
                            canvas::Action::publish((self.actions.on_band)(band, freq, gain))
                                .and_capture(),
                        )
                    }
                    Drag::LowCut => {
                        let freq = low_cut_at(plot, pos);
                        if state.last_low_cut == Some(freq) {
                            return None;
                        }
                        state.last_low_cut = Some(freq);
                        Some(canvas::Action::publish((self.actions.on_low_cut)(freq)).and_capture())
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.drag.take().is_none() {
                    return None;
                }
                state.last_band = None;
                state.last_low_cut = None;
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let pos = local_point(bounds, cursor.position_over(bounds)?);
                let Drag::Band(band) = self.hit(plot, pos)? else {
                    return None;
                };
                if self.modifiers.control() {
                    return None;
                }
                let (dy, base_step) = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => (*y, 0.12),
                    mouse::ScrollDelta::Pixels { y, .. } => (*y, 0.02),
                };
                if dy == 0.0 {
                    return None;
                }
                let mult = if self.modifiers.shift() { 0.25 } else { 1.0 };
                let now = Instant::now();
                let fresh_gesture = state
                    .last_scroll
                    .is_none_or(|last| now.duration_since(last) > SCROLL_IDLE)
                    || state.scroll_band != Some(band);
                let base = if fresh_gesture {
                    self.eq.bands[band].q
                } else {
                    state.scroll_q.unwrap_or(self.eq.bands[band].q)
                };
                let q = (base * 2f32.powf(dy * base_step * mult)).clamp(0.05, 10.0);
                state.scroll_q = Some(q);
                state.scroll_band = Some(band);
                state.last_scroll = Some(now);
                if (q - self.eq.bands[band].q).abs() < 0.001 {
                    return None;
                }
                Some(canvas::Action::publish((self.actions.on_band_q)(band, q)).and_capture())
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
        let scale = self.scale;
        let plot = self.plot_rect(bounds.size());

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

        // Individual band shapes first, dimmed, so it is obvious which
        // handle is responsible for which part of the combined curve.
        for (i, band) in self.eq.bands.iter().enumerate() {
            if band.band_type == EqBandType::Off {
                continue;
            }
            let points = response_points(plot, |f| band_db(band, f, rate));
            frame.stroke(
                &path_from(&points),
                Stroke::default()
                    .with_color(Color {
                        a: 0.28,
                        ..self.handle_color(state, i)
                    })
                    .with_width(scale),
            );
        }

        draw_low_cut(&mut frame, &self.eq, plot, scale, state.drag);

        let points = response_points(plot, |f| response_db(&self.eq, f, rate));
        let color = if self.eq.enabled {
            theme::ACCENT
        } else {
            theme::TEXT_SEC
        };
        frame.stroke(
            &path_from(&points),
            Stroke::default().with_color(color).with_width(1.6 * scale),
        );

        for (i, band) in self.eq.bands.iter().enumerate() {
            if band.band_type == EqBandType::Off {
                continue;
            }
            let point = self.band_handle(band, plot);
            let active = state.drag == Some(Drag::Band(i));
            let handle_color = self.handle_color(state, i);
            let radius = if active {
                (HANDLE_R + 1.0) * scale
            } else {
                HANDLE_R * scale
            };
            frame.fill(&Path::circle(point, radius), theme::BG_DEEP);
            frame.stroke(
                &Path::circle(point, radius),
                Stroke::default()
                    .with_color(handle_color)
                    .with_width(if active { 1.8 } else { 1.3 } * scale),
            );
            if active {
                frame.stroke(
                    &Path::circle(point, radius + 2.0 * scale),
                    Stroke::default()
                        .with_color(theme::ACCENT)
                        .with_width(scale),
                );
            }
            frame.fill_text(Text {
                content: (i + 1).to_string(),
                position: point,
                color: handle_color,
                size: (theme::TEXT_MICRO * scale).into(),
                align_x: advanced_text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                ..Text::default()
            });
        }

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
                    Point::new(
                        plot.x + plot.width - 2.0 * scale,
                        plot.y + plot.height - 2.0 * scale,
                    ),
                    advanced_text::Alignment::Right,
                    iced::alignment::Vertical::Bottom,
                );
            }
            if let Some(target) = state.drag {
                let (content, x, color) = match target {
                    Drag::Band(band) => {
                        let b = self.eq.bands[band];
                        (
                            format!(
                                "{}  {}  {:+.1} dB  Q {:.2}",
                                band + 1,
                                freq_label(b.freq_hz as f32),
                                b.gain_db,
                                b.q
                            ),
                            plot.x + 4.0 * scale,
                            theme::EQ_BAND[band.min(theme::EQ_BAND.len() - 1)],
                        )
                    }
                    Drag::LowCut => (
                        format!("LC  {}", freq_label(self.eq.low_cut_freq_hz as f32)),
                        plot.x + 4.0 * scale,
                        theme::ACCENT,
                    ),
                };
                frame.fill_text(Text {
                    content,
                    position: Point::new(x, plot.y + 3.0 * scale),
                    color,
                    size: (theme::TEXT_MICRO * scale).into(),
                    align_x: advanced_text::Alignment::Left,
                    align_y: iced::alignment::Vertical::Top,
                    ..Text::default()
                });
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.drag.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_over(bounds) {
            Some(pos) => match self.hit(self.plot_rect(bounds.size()), local_point(bounds, pos)) {
                Some(Drag::LowCut) => mouse::Interaction::ResizingHorizontally,
                Some(Drag::Band(_)) => mouse::Interaction::Grab,
                None => mouse::Interaction::None,
            },
            None => mouse::Interaction::None,
        }
    }
}

impl<Message> EqPlot<Message> {
    fn handle_color(&self, _state: &State, band: usize) -> Color {
        theme::EQ_BAND[band.min(theme::EQ_BAND.len() - 1)]
    }
}

fn band_at(plot: Rectangle, pos: Point) -> (u16, f32) {
    let t = ((pos.x - plot.x) / plot.width.max(1.0)).clamp(0.0, 1.0);
    let freq = F_MIN * (F_MAX / F_MIN).powf(t);
    let db = DB_MAX - ((pos.y - plot.y) / plot.height.max(1.0)).clamp(0.0, 1.0) * (DB_MAX - DB_MIN);
    let db = (db * 10.0).round() / 10.0;
    let db = if db.abs() < 0.15 { 0.0 } else { db };
    (
        freq.round().clamp(F_MIN, F_MAX) as u16,
        db.clamp(DB_MIN, DB_MAX),
    )
}

fn low_cut_at(plot: Rectangle, pos: Point) -> u16 {
    let t = ((pos.x - plot.x) / plot.width.max(1.0)).clamp(0.0, 1.0);
    (F_MIN * (F_MAX / F_MIN).powf(t))
        .round()
        .clamp(F_MIN, F_MAX) as u16
}

fn response_points(plot: Rectangle, db_at: impl Fn(f32) -> f32) -> Vec<Point> {
    (0..=CURVE_POINTS)
        .map(|i| {
            let t = i as f32 / CURVE_POINTS as f32;
            let f = F_MIN * (F_MAX / F_MIN).powf(t);
            let db = db_at(f).clamp(DB_MIN, DB_MAX);
            Point::new(plot.x + plot.width * t, db_to_y(db, plot))
        })
        .collect()
}

fn path_from(points: &[Point]) -> Path {
    Path::new(|p| {
        if let Some(first) = points.first() {
            p.move_to(*first);
        }
        for point in points.iter().skip(1) {
            p.line_to(*point);
        }
    })
}

fn draw_low_cut(frame: &mut Frame, eq: &InputEq, plot: Rectangle, scale: f32, drag: Option<Drag>) {
    let x = freq_to_x(eq.low_cut_freq_hz as f32, plot).clamp(
        plot.x + LOW_CUT_HANDLE_R * scale,
        plot.x + plot.width - LOW_CUT_HANDLE_R * scale,
    );
    let color = if drag == Some(Drag::LowCut) {
        theme::ACCENT
    } else {
        Color {
            a: 0.55,
            ..theme::TEXT_SEC
        }
    };
    let dash = 3.0 * scale;
    let gap = 2.0 * scale;
    let mut y = plot.y;
    while y < plot.y + plot.height {
        let end = (y + dash).min(plot.y + plot.height);
        frame.stroke(
            &Path::line(Point::new(x, y), Point::new(x, end)),
            Stroke::default().with_color(color).with_width(scale),
        );
        y += dash + gap;
    }

    let radius = LOW_CUT_HANDLE_R * scale;
    let triangle = Path::new(|p| {
        p.move_to(Point::new(x, plot.y + plot.height - radius));
        p.line_to(Point::new(x - radius, plot.y + plot.height));
        p.line_to(Point::new(x + radius, plot.y + plot.height));
        p.close();
    });
    frame.fill(&triangle, color);
}

fn db_to_y(db: f32, plot: Rectangle) -> f32 {
    plot.y + plot.height * (DB_MAX - db) / (DB_MAX - DB_MIN)
}

fn local_point(bounds: Rectangle, pos: Point) -> Point {
    Point::new(pos.x - bounds.x, pos.y - bounds.y)
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
    let num = (num_re * num_re + num_im * num_im).sqrt();
    let den = (den_re * den_re + den_im * den_im).sqrt();
    20.0 * (num / den).max(1e-9).log10()
}

fn band_coeffs(band: &EqBand, rate: f32) -> ([f32; 3], [f32; 3]) {
    let freq = (band.freq_hz as f32).clamp(10.0, rate * 0.49);
    let q = band.q.max(0.01);
    let gain = band.gain_db;
    let a = 10f32.powf(gain / 40.0);
    let w0 = 2.0 * PI * freq / rate;
    let cos = w0.cos();
    let sin = w0.sin();
    let alpha = sin / (2.0 * q);

    match band.band_type {
        EqBandType::Off => ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        EqBandType::Bell => (
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        ),
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
    use super::{
        band_at, db_to_y, freq_to_x, low_cut_at, low_cut_db, response_db, DB_MAX, DB_MIN, F_MAX,
        F_MIN,
    };
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

    #[test]
    fn plot_mapping_round_trips_a_band_position() {
        let plot = Rectangle::new(Point::new(10.0, 20.0), Size::new(200.0, 100.0));
        let pos = Point::new(freq_to_x(1_000.0, plot), db_to_y(6.0, plot));
        let (freq, gain) = band_at(plot, pos);
        assert!((freq as f32 - 1_000.0).abs() < 1.0, "read {freq} Hz");
        assert!((gain - 6.0).abs() < 0.1, "read {gain} dB");
    }

    #[test]
    fn low_cut_mapping_stays_inside_the_plot() {
        let plot = Rectangle::new(Point::new(10.0, 20.0), Size::new(200.0, 100.0));
        assert_eq!(low_cut_at(plot, Point::new(plot.x, plot.y)), F_MIN as u16);
        assert_eq!(
            low_cut_at(plot, Point::new(plot.x + plot.width, plot.y)),
            F_MAX as u16
        );
    }
}
