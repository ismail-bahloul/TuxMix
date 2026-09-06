//! A single channel strip: label + type tag, mute/solo, 48V/PAD, fader+VU,
//! dB readout (double-click to edit), pan readout.

use iced::advanced::text as advanced_text;
use iced::keyboard::Modifiers;
use iced::widget::canvas::{self, Canvas, Frame, Geometry};
use iced::widget::{button, column, container, mouse_area, row, text, text_input, tooltip};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Theme, Vector};
use std::time::Instant;
use tuxmix_core::ChannelId;

use crate::app::{db_text, short_label, Message, OUT_LABELS};
use crate::theme;
use crate::widgets::fader::{fader, vu_meter, Fader, MeterFrame};
use crate::widgets::knob::{knob, Knob};

/// Base sizes at `scale == 1.0` (`theme::SCALE_DEFAULT`) — every dimension
/// in a strip is one of these times `StripParams::scale`, so the window's
/// adaptive scale (see `app::recompute_ui_scale`) resizes strips the same
/// way it resizes text.
const FADER_H: f32 = 170.0;
/// Widened from the original 80 — with `Fader::layout_x` centering a
/// `[track][ruler][meter]` group inside the fader canvas *and* a side
/// icon column (gear/EQ/collapse, see `full_strip`) claiming space in
/// the same row, 80 wasn't wide enough for both: the group overflowed
/// into the icon column's space, clipping the ruler's two-digit ticks
/// under the buttons (confirmed via a live screenshot). 96 gives ~5px
/// of headroom at `scale == 1.0` after both claims.
pub(crate) const STRIP_W: f32 = 96.0;
/// Collapsed strips need to stand the same total height as a full strip
/// (see `collapsed_strip`'s own doc comment) despite having far fewer
/// rows — name/M/S/meter/expand vs. the full strip's
/// header/pan/M-S/fader/dB/route. The meter is what absorbs the
/// difference. Was `76.0`, re-tuned down after the name label became a
/// tall rotated canvas (see `ROTATED_LABEL_LEN`) instead of one short
/// text line — that canvas already eats most of the vertical slack the
/// old flat offset existed to add. Empirically measured the same way the
/// original number was (pixel-measuring a collapsed and an adjacent full
/// strip side by side in a live screenshot), not derived on paper.
const COLLAPSED_METER_H: f32 = FADER_H + 30.0;
/// Collapsed strips are a glance-only readout: name, Mute/Solo (kept
/// live — see this function's own doc comment), and a VU meter; no
/// fader, no pan, no route. Width is set by the header (name + expand
/// button), not the meter, which is narrower than that on its own.
/// Deliberately the same `COLLAPSED_W` for every channel kind, output
/// strips included — collapsing is opting *out* of emphasis, so there's
/// nothing to differentiate here.
///
/// Narrowed from `60` after comparing against a live TotalMix screenshot
/// at pixel level (`Screenshot 2026-08-30 124715.png`): real collapsed
/// strips there measure ~29px against ~75px full strips — a ~0.39 ratio,
/// well under half, not the ~0.6 ours had. `44` keeps that same ratio
/// against the current (wider) `STRIP_W` while still leaving room for
/// the existing `vu_meter` widget's own fixed `METER_RULER_W` (30) plus
/// padding without clipping it — matching TotalMix's exact ~0.39 would
/// need a narrower meter widget too, not attempted this pass.
pub(crate) const COLLAPSED_W: f32 = 44.0;

/// Size of the small icon-only buttons (settings gear, collapse chevron)
/// — smaller than the strip's other buttons (M/S at 18x`btn_h`) since
/// both now share a row with the dB readout at the bottom of the strip
/// (see `full_strip`), which doesn't have much width to spare either.
const ICON_BTN_W: f32 = 15.0;
const ICON_BTN_H: f32 = 14.0;

/// Width (at `scale == 1.0`) of the Route flyout `app.rs` opens over a
/// strip's right neighbor — `pub(crate)` because `app.rs` needs it to
/// size the popover. Settings doesn't use this: it pushes the row instead
/// of overlaying it, sized to match the strip's own width (see the
/// reference design), not a fixed constant — see `FlyoutKind`.
pub(crate) const FLYOUT_W: f32 = 140.0;

/// Width (at `scale == 1.0`) of the EQ flyout — wide enough for 3 knobs
/// (freq/Q/gain) side by side per band (`knob()`'s own intrinsic width is
/// `DIAMETER + 2*MARGIN` from `widgets/knob.rs`, roughly 56px at scale 1),
/// unlike Settings which matches the strip's own (much narrower) width.
pub(crate) const EQ_FLYOUT_W: f32 = 220.0;

/// Which flyout (at most one, and only one per strip) is open — the gear
/// icon opens `Settings` (48V/PAD/Sensitivity, moved out of the strip's
/// own vertical flow so it doesn't grow/shrink the whole card; Gain stays
/// inline instead — see `app.rs::settings_popover`'s doc comment for why),
/// the route trigger opens `Route` (which output bus), the EQ trigger
/// (analog inputs only) opens `Eq` (3-band + low-cut, see
/// `app.rs::eq_popover`), and the "T" trigger (every hardware input, see
/// `StripParams::has_trim`) opens `Trim` (see `app.rs::trim_popover`).
/// All render as the same kind of animated panel sliding out over the
/// strip's right neighbor — see `app.rs::with_flyout` (Route) and
/// `mixer_view`'s input loop (Settings/Eq/Trim, which push the row
/// instead).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutKind {
    Route,
    Settings,
    Eq,
    Trim,
}

/// The full (uncollapsed) width for a given channel — every channel kind
/// renders at the same width. Takes `cid` (unused) rather than being a
/// bare constant so `app.rs::set_collapsed`/`rendered_strip_width` don't
/// need to change if a future kind ever needs to differ again.
pub(crate) fn full_width(_cid: ChannelId) -> f32 {
    STRIP_W
}

/// How long a strip's collapse/expand width transition takes — longer than
/// the meter's 50ms interp window since this is a much bigger, structural
/// change (the whole card growing or shrinking), not a small value nudge.
/// Same linear ease-style interpolation as `fader::MeterFrame`, just a
/// dedicated type since the duration and the thing being interpolated
/// (a pixel width, not a volume/pan value) are both different.
const COLLAPSE_INTERP_MS: f32 = 160.0;

#[derive(Clone, Copy, Debug)]
pub struct CollapseAnim {
    pub prev: f32,
    pub value: f32,
    pub since: Instant,
}

impl CollapseAnim {
    pub fn at(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.since).as_secs_f32() * 1000.0 / COLLAPSE_INTERP_MS)
            .clamp(0.0, 1.0);
        self.prev + (self.value - self.prev) * t
    }

    pub fn is_settling(&self, now: Instant) -> bool {
        (self.value - self.prev).abs() > f32::EPSILON
            && now.duration_since(self.since).as_secs_f32() * 1000.0 < COLLAPSE_INTERP_MS
    }
}

pub struct StripParams<'a> {
    pub cid: ChannelId,
    pub output_idx: usize,
    pub name: String,
    pub type_tag: Option<(&'static str, Color)>,
    pub vol: f32,
    pub pan: i8,
    pub meter: MeterFrame,
    /// Whether `meter` is a real reading on the active backend — see
    /// `fader::draw_meter`'s doc comment.
    pub meter_available: bool,
    pub has_48v: bool,
    pub has_pad: bool,
    pub phantom: bool,
    pub pad: bool,
    /// Whether this input has a preamp gain control at all (Mic and
    /// Instrument only — Line/SPDIF/ADAT inputs have no gain knob).
    pub has_gain: bool,
    /// Raw hardware gain units, not dB — see `Message::Gain`.
    pub gain: u32,
    pub gain_max: u32,
    /// Whether this input has a sensitivity switch (Instrument only).
    pub has_sensitivity: bool,
    /// `true` = +4dBu, `false` = -10dBV.
    pub sensitivity_plus4: bool,
    /// Whether this input has an EQ strip at all (the 4 analog inputs
    /// only — see `InputChannel::eq`).
    pub has_eq: bool,
    /// Whether the EQ is currently engaged — lights up the trigger button
    /// even when its flyout isn't open, like the 48V/PAD toggles.
    pub eq_enabled: bool,
    /// Whether this channel has a Trim control — every hardware input
    /// (analog and digital alike, unlike `has_gain`), `false` for
    /// Playback/Output (see `full_strip`'s "T" button doc comment for
    /// why Playback doesn't get one despite TotalMix showing it there).
    pub has_trim: bool,
    /// Current trim value in dB (-65..+6) — see `InputChannel::trim`.
    pub trim: f32,
    /// Output-only: `OutputChannel::loopback`.
    pub loopback: bool,
    /// Whether this channel's pair is currently a linked stereo bus
    /// (`RmeDevice::{input_pair,playback,output}_linked`, whichever
    /// matches `cid`'s kind) — TotalMix's default, shown/toggled from
    /// the Settings flyout for every channel kind.
    pub stereo_linked: bool,
    /// Which flyout (if any) is currently open *for this strip* — lights
    /// up the matching trigger (gear icon for `Settings`, the route
    /// button for `Route`); `app.rs` owns the actual popover content.
    pub open_flyout: Option<FlyoutKind>,
    pub mute: bool,
    pub solo: bool,
    /// `true` only for `ChannelId::Output` — every other channel kind
    /// has no CUE bus of its own (see `RmeDevice::set_cue`'s doc
    /// comment: CUE always monitors *some output's* dedicated
    /// playback pair through AN1/2, not an input/playback concept).
    pub has_cue: bool,
    pub cue: bool,
    pub default_vol: f32,
    pub editing: bool,
    pub edit_buf: &'a str,
    pub drag_range: Option<(f32, f32)>,
    pub modifiers: Modifiers,
    pub collapsed: bool,
    pub collapse_anim: Option<CollapseAnim>,
    pub scale: f32,
    pub selected: bool,
    pub hovered: bool,
}

/// A button's own padding-based centering isn't reliable across glyphs of
/// different intrinsic width (e.g. "S" sat visibly left of center while "M"
/// looked fine) — force it explicitly instead of trusting the default. The
/// default 1.2x line-height also reserves descender space these glyphs
/// (M, S, no descenders) never use, which reads as "sitting too high" once
/// centered — tightening it to 1:1 removes that residual vertical bias.
fn centered_label<'a>(s: &'a str, size: f32) -> Element<'a, Message> {
    container(
        text(s)
            .size(size)
            .line_height(iced::widget::text::LineHeight::Absolute(iced::Pixels(
                size,
            ))),
    )
    .center(Length::Fill)
    .into()
}

/// Wraps a control in a hover tooltip — for the abbreviations (M, S, 48V,
/// PAD) that read as pro-audio jargon to anyone not already fluent in it.
/// A short delay so it doesn't flash on every incidental mouse-over while
/// moving across the strip toward something else.
pub(crate) fn hint<'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    scale: f32,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(theme::TEXT_XS * scale).color(theme::TEXT_PRIMARY))
            .padding(theme::SPACE_SM * scale)
            .style(theme::panel),
        tooltip::Position::Top,
    )
    .gap(4.0 * scale)
    .delay(std::time::Duration::from_millis(400))
    .into()
}

/// How long (in canvas-local x, *before* the -90° rotation — i.e. how
/// tall the finished label actually stands once rotated) `rotated_label`
/// reserves for the name — sized for the longest real channel name this
/// app ever shows ("ADAT7/8 PB" and friends), not just the common case,
/// since unlike a plain `text()` widget this canvas has no shrink-to-fit:
/// anything longer than this would silently draw past its own bounds and
/// overlap the Mute button below it.
const ROTATED_LABEL_LEN: f32 = 64.0;

/// A read-only text canvas rotated -90° (reads bottom-to-top) — the
/// collapsed strip's own name label. Confirmed against a live TotalMix
/// screenshot: collapsed buses are narrow enough (see `COLLAPSED_W`'s own
/// doc comment) that a normal horizontal name doesn't fit at all, so
/// TotalMix rotates it instead of truncating or wrapping it. iced's plain
/// `text()` widget has no rotation of its own, hence a small dedicated
/// canvas rather than reusing `centered_label`.
struct RotatedLabel {
    text: String,
    size: f32,
}

impl<Message> canvas::Program<Message> for RotatedLabel {
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
        let center = frame.center();
        frame.with_save(|frame| {
            frame.translate(Vector::new(center.x, center.y));
            // -90°: in this canvas's clockwise-from-+x, y-down convention
            // (matching `iced_graphics::geometry::path::Arc` and
            // `widgets::knob`'s own `angle_of`), this maps local "reading
            // direction" (+x) to global "up" — i.e. the text climbs the
            // strip bottom-to-top, matching the reference exactly rather
            // than guessed-and-checked.
            frame.rotate(-std::f32::consts::FRAC_PI_2);
            frame.fill_text(canvas::Text {
                content: self.text.clone(),
                position: Point::ORIGIN,
                color: theme::TEXT_PRIMARY,
                size: self.size.into(),
                align_x: advanced_text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                ..canvas::Text::default()
            });
        });
        vec![frame.into_geometry()]
    }
}

fn rotated_label<'a, Msg: 'a + Clone>(text: String, size: f32, scale: f32) -> Element<'a, Msg> {
    Canvas::new(RotatedLabel { text, size })
        .width(Length::Fill)
        .height(Length::Fixed(ROTATED_LABEL_LEN * scale))
        .into()
}

/// Full-strip header — just the name (+ type tag), centered, TotalMix-style
/// (see the reference design: no icons in the header at rest). See
/// `collapsed_header_row` for the collapsed strip's own header, which
/// still needs its chevron.
fn header_row<'a>(
    name: &str,
    type_tag: Option<(&'static str, Color)>,
    scale: f32,
) -> Element<'a, Message> {
    // `.wrapping(Wrapping::None)` — without it, a long name ("Instr. 3/4",
    // "ADAT7/8") wraps to two lines partway through the collapse/expand
    // width animation, once the shrinking card gets narrower than the
    // name's natural width, growing the whole strip's *height* for the
    // ~160ms the animation is mid-flight before it settles back down —
    // user report: "pendant que ça se minimise le bus augmente de hauteur
    // un peu." No text in this card is meant to ever wrap (every label
    // here is a single short line by design), so this is a correctness
    // fix for the whole card, not just the header specifically — see the
    // matching `.wrapping(None)` added to every other `text()` in
    // `full_strip` below for the same reason.
    let mut header = row![text(short_label(name).to_string())
        .size(theme::TEXT_MD * scale)
        .wrapping(advanced_text::Wrapping::None)]
    .spacing(theme::SPACE_HAIRLINE);
    if let Some((tag, color)) = type_tag {
        header = header.push(
            text(tag)
                .color(color)
                .size(theme::TEXT_XS * scale)
                .wrapping(advanced_text::Wrapping::None),
        );
    }
    container(header)
        .width(Length::Fill)
        .center_x(Length::Fill)
        .align_y(iced::Alignment::Center)
        .into()
}

/// A collapsed strip is a glance-only readout, but per the reference
/// design keeps Mute/Solo live (unlike the fader/pan/route, still traded
/// away for space) — collapsing is for strips you're not actively
/// adjusting the level of, not ones you'd never want to silence in a
/// hurry. Only ever rendered fully settled (see `strip()`'s dispatch), so
/// `w` is always `COLLAPSED_W`, but it's threaded through rather than
/// hardcoded to keep this in lockstep with `full_strip`'s signature.
fn collapsed_strip<'a>(p: StripParams<'a>, w: f32) -> Element<'a, Message> {
    let cid = p.cid;
    let scale = p.scale;
    let btn_h = ICON_BTN_H * scale;

    let mute_btn = hint(
        button(centered_label("M", theme::TEXT_MICRO * scale))
            .width(Length::Fill)
            .height(btn_h)
            .style(theme::toggle_button(p.mute, theme::MUTE_COLOR))
            .on_press(Message::Mute(cid, !p.mute)),
        "Mute",
        scale,
    );
    let solo_btn = hint(
        button(centered_label("S", theme::TEXT_MICRO * scale))
            .width(Length::Fill)
            .height(btn_h)
            .style(theme::toggle_button(p.solo, theme::SOLO_COLOR))
            .on_press(Message::Solo(cid, !p.solo)),
        "Solo",
        scale,
    );
    // "+" rather than a chevron — the reference puts the expand trigger
    // at the *bottom* of the collapsed card, not a top corner (which is
    // also where the full strip's own gear/chevron column sits now, at
    // the fader's mid-height on the right — matching that position here
    // would put "expand" somewhere a collapsed card, with no fader, has
    // nothing to anchor to).
    let expand_btn = hint(
        button(centered_label("+", theme::TEXT_SM * scale))
            .padding(0)
            .width(Length::Fill)
            .height(btn_h)
            .style(theme::plain_button)
            .on_press(Message::ToggleCollapse(cid)),
        "Expand",
        scale,
    );

    let rows = column![
        rotated_label(short_label(&p.name).to_string(), theme::TEXT_SM * scale, scale),
        mute_btn,
        solo_btn,
        container(vu_meter(
            p.meter,
            COLLAPSED_METER_H * scale,
            scale,
            p.meter_available,
        ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        expand_btn,
    ]
    .spacing(theme::SPACE_HAIRLINE)
    .width(Length::Fill)
    .align_x(iced::Alignment::Center);

    mouse_area(
        container(rows)
            .style(theme::strip_panel(
                p.selected,
                p.hovered,
                p.type_tag.map(|(_, c)| c),
            ))
            .padding([theme::SPACE_SM * p.scale, theme::SPACE_MD * p.scale])
            .width(Length::Fixed(w * p.scale))
            .clip(true),
    )
    .on_press(Message::StripClicked(cid))
    .on_double_click(Message::ToggleCollapse(cid))
    .on_enter(Message::StripHovered(Some(cid)))
    .on_exit(Message::StripHovered(None))
    .into()
}

/// Picks between the two strip layouts and, while a collapse/expand
/// animation is in flight, the width the outer card should be drawn at
/// this frame. The full (uncollapsed) content is shown not just when
/// resting expanded but for the whole transition in *either* direction —
/// shrinking, it's the thing visibly getting clipped down to
/// `COLLAPSED_W`; growing, it's what's being revealed. Only once a
/// collapse has fully settled does rendering switch to the lighter,
/// control-free `collapsed_strip`.
pub fn strip<'a>(p: StripParams<'a>) -> Element<'a, Message> {
    let now = Instant::now();
    let (w, show_full) = match &p.collapse_anim {
        Some(a) => (a.at(now), a.is_settling(now) || !p.collapsed),
        None => {
            if p.collapsed {
                (COLLAPSED_W, false)
            } else {
                (full_width(p.cid), true)
            }
        }
    };

    if show_full {
        full_strip(p, w)
    } else {
        collapsed_strip(p, w)
    }
}

fn full_strip<'a>(p: StripParams<'a>, w: f32) -> Element<'a, Message> {
    let cid = p.cid;
    let out = p.output_idx;
    let scale = p.scale;
    let btn_h = 18.0 * scale;

    // Every channel kind has at least the STEREO link/split toggle now
    // (see `app.rs::settings_popover`) — 48V/PAD/Sensitivity/Gain are
    // just the input-only additions to the same panel.
    let has_settings = true;
    let header = header_row(&p.name, p.type_tag, scale);

    let mute_btn = hint(
        button(centered_label("M", theme::TEXT_SM * scale))
            .width(Length::Fill)
            .height(btn_h)
            .style(theme::toggle_button(p.mute, theme::MUTE_COLOR))
            .on_press(Message::Mute(cid, !p.mute)),
        "Mute",
        scale,
    );
    let solo_btn = hint(
        button(centered_label("S", theme::TEXT_SM * scale))
            .width(Length::Fill)
            .height(btn_h)
            .style(theme::toggle_button(p.solo, theme::SOLO_COLOR))
            .on_press(Message::Solo(cid, !p.solo)),
        "Solo",
        scale,
    );
    // Fixed-width buttons left dead space flanking them whenever the card
    // was sized for a wider sibling row (48V/PAD, or just a long channel
    // name) — filling the row makes every row use the card's full width
    // instead of only the widest one.
    let mut ms_row = row![mute_btn, solo_btn].spacing(theme::SPACE_TIGHT).width(Length::Fill);
    if p.has_cue {
        ms_row = ms_row.push(hint(
            button(centered_label("C", theme::TEXT_SM * scale))
                .width(Length::Fill)
                .height(btn_h)
                .style(theme::toggle_button(p.cue, theme::ACCENT))
                .on_press(Message::CueChanged(cid, !p.cue)),
            "CUE — exclusively monitor this output's own playback feed through AN1/2",
            scale,
        ));
    }

    let mut rows = column![header].spacing(theme::SPACE_HAIRLINE);

    // Pan sits directly under the header, TotalMix-style, rather than
    // trailing at the very bottom — with pan above the fader and the
    // route picker (below) after it, the fader ends up vertically
    // centered in the strip's own flow instead of front-loaded.
    //
    // Outputs get this row too now, even though there's nothing real to
    // back it with yet — the user caught that skipping it made Output
    // strips visibly *shorter* than Input/Playback ones (confirmed via a
    // live screenshot: ~257px vs ~307px), when the real TotalMix
    // reference shows every strip, outputs included, at the exact same
    // height *and* with a knob in this exact spot (labeled "C", same as
    // an input's pan). That knob is very likely a real output-bus
    // **balance** control, not pan-into-a-bus (which wouldn't make sense
    // for an output) — but `RmeDevice::set_pan`'s signature is
    // `(channel, output)`, built for routing a channel's signal *into* a
    // submix, and neither `tuxmix-core` nor `tuxmix-usb`'s protocol layer
    // has anything resembling an output-master balance register (checked
    // both before writing this).
    //
    // First attempt reserved a blank `Space` here instead of a knob — the
    // user pointed out that read as the control being entirely missing,
    // not "present but not applicable yet." A dimmed, non-interactive
    // knob (`Knob::interactive: false`, see its own doc comment) reads
    // correctly as the latter — same footprint, same "C" TotalMix shows,
    // just visibly inert — without shipping a knob that would silently
    // do nothing if someone tried to drag it. `on_change`/`on_reset` are
    // genuinely unreachable: `interactive: false` makes `Knob::update`
    // return early before either could ever be invoked.
    let pan_str = match p.pan.cmp(&0) {
        std::cmp::Ordering::Less => format!("L{}", -p.pan),
        std::cmp::Ordering::Greater => format!("R{}", p.pan),
        std::cmp::Ordering::Equal => "C".to_string(),
    };
    let is_output = matches!(cid, ChannelId::Output(_));
    let pan_knob = if is_output {
        Knob {
            value: 0.0,
            range: (-100.0, 100.0),
            label: "C".to_string(),
            arc_from_center: true,
            interactive: false,
            log_scale: false,
            modifiers: p.modifiers,
            scale,
            on_change: Box::new(|_| unreachable!("interactive: false suppresses all input")),
            on_reset: Box::new(|| unreachable!("interactive: false suppresses all input")),
        }
    } else {
        Knob {
            value: p.pan as f32,
            range: (-100.0, 100.0),
            label: pan_str,
            arc_from_center: true,
            interactive: true,
            log_scale: false,
            modifiers: p.modifiers,
            scale,
            on_change: Box::new(move |v| Message::PanChanged(cid, out, v.round() as i8)),
            on_reset: Box::new(move || Message::PanReset(cid, out)),
        }
    };
    let pan_hint = if is_output {
        "Output balance — not yet implemented (no known hardware register)"
    } else {
        "Pan — drag, scroll, or double-click to reset"
    };
    rows = rows.push(
        container(hint(knob(pan_knob), pan_hint, scale))
            .width(Length::Fill)
            .center_x(Length::Fill),
    );

    rows = rows.push(ms_row);
    // 48V/PAD/Sensitivity/Gain all live in the gear flyout now (see
    // `app.rs::settings_popover`), not inline here — keeps every strip's
    // own height fixed regardless of channel type, and matches the route
    // panel's own "opens beside the strip" shape instead of an inline
    // accordion that used to grow/shrink the card itself. Gain used to be
    // the one holdout (a `Knob` placed inside the old Stack-based flyout
    // broke click handling for the whole row), but the flyout isn't a
    // `Stack` anymore — it pushes the row instead of overlaying it — so
    // that failure mode doesn't apply and Gain moved in with the rest.

    let default_vol = p.default_vol;
    let fader_h = FADER_H * scale;
    let fader_widget = fader(Fader {
        value: p.vol,
        range: p.drag_range.unwrap_or((0.0, 2.0)),
        default_value: default_vol,
        meter: p.meter,
        meter_available: p.meter_available,
        height: fader_h,
        show_meter: true,
        // The icon column (`icon_col`, built below) is this canvas's
        // `row!` sibling — same width regardless of how many buttons it
        // holds (T/gear/EQ all share `ICON_BTN_W`), plus the row's own
        // spacing between them. See `Fader::reserved_right`'s own doc
        // comment for why the canvas needs to know this at all.
        // `ICON_BTN_W` scales with the strip (every icon button's width
        // does), but the row's own `.spacing(theme::SPACE_TIGHT)` below
        // — like several other spacings in this file — doesn't scale;
        // matching that exactly (not just approximately) is the point,
        // since this value feeds a pixel-level centering fix.
        reserved_right: ICON_BTN_W * scale + theme::SPACE_TIGHT,
        modifiers: p.modifiers,
        scale,
        show_track: true,
        label: None,
        label_color: theme::TEXT_SEC,
        // Unused: only matters when `show_meter: false` (see its own
        // doc comment) — this strip fader is never that.
        compact_width: 0.0,
        on_press: Box::new(move |v, range| Message::FaderPressed(cid, out, v, range)),
        on_drag: Box::new(move |v| Message::VolumeChanged(cid, out, v)),
        on_release: Box::new(move || Message::RangeCleared(cid)),
        on_reset: Box::new(move || Message::Reset(cid, out, default_vol)),
    });

    // Settings gear and EQ stacked on the strip's right flank, beside the
    // fader — confirmed against a live TotalMix screenshot at high zoom
    // (an earlier "put them in a row below the fader instead" call in
    // this file was a misreading of a narrower crop; a full-height zoom
    // on a real strip shows the gear/EQ column sitting to the right of
    // the meter, roughly spanning the fader's lower half, not underneath
    // it — the user caught this directly). `STRIP_W` was widened (see
    // its own doc comment) specifically so this column has room without
    // pushing the meter/ruler group past the fader canvas's own bounds.
    let mut icon_col = column![].spacing(theme::SPACE_TIGHT).align_x(iced::Alignment::Center);
    // Trim ("T") sits above the gear icon — every hardware input has one
    // (confirmed via the reference: ADAT/AS strips show "T" same as the
    // analog ones, unlike Gain which only exists on Mic/Instrument), but
    // not Playback or Output. TotalMix *does* show a "T" on Software
    // Playback strips too, but per the user's own call once told
    // `RmeDevice::set_trim` only maps to real hardware input sources
    // (`tuxmix_usb::usb.rs::input_source`, not playback indices): ship
    // the real, working control on Hardware Inputs, skip it on Playback
    // rather than either faking one there or blocking on new protocol
    // work.
    if p.has_trim {
        let trim_open = p.open_flyout == Some(FlyoutKind::Trim);
        icon_col = icon_col.push(hint(
            button(centered_label("T", theme::TEXT_MICRO * scale))
                .padding(0)
                .width(ICON_BTN_W * scale)
                .height(ICON_BTN_H * scale)
                // Lights up whenever trim is off unity too, not just
                // while its flyout is open — same "shows live state at a
                // glance" idea as the EQ trigger's own `eq_open ||
                // eq_enabled`.
                .style(theme::toggle_button(trim_open || p.trim != 0.0, theme::ACCENT))
                .on_press(Message::ToggleFlyout(cid, FlyoutKind::Trim)),
            if trim_open { "Hide trim" } else { "Show trim" },
            scale,
        ));
    }
    if has_settings {
        let settings_open = p.open_flyout == Some(FlyoutKind::Settings);
        icon_col = icon_col.push(hint(
            button(centered_label("⚙", theme::TEXT_MICRO * scale))
                .padding(0)
                .width(ICON_BTN_W * scale)
                .height(ICON_BTN_H * scale)
                .style(theme::toggle_button(settings_open, theme::ACCENT))
                .on_press(Message::ToggleFlyout(cid, FlyoutKind::Settings)),
            if settings_open { "Hide settings" } else { "Show settings (48V/PAD/Sensitivity)" },
            scale,
        ));
    }
    if p.has_eq {
        let eq_open = p.open_flyout == Some(FlyoutKind::Eq);
        icon_col = icon_col.push(hint(
            button(centered_label("EQ", theme::TEXT_MICRO * scale))
                .padding(0)
                .width(ICON_BTN_W * scale)
                .height(ICON_BTN_H * scale)
                .style(theme::toggle_button(eq_open || p.eq_enabled, theme::ACCENT))
                .on_press(Message::ToggleFlyout(cid, FlyoutKind::Eq)),
            if eq_open { "Hide EQ" } else { "Show EQ (3-band + low cut)" },
            scale,
        ));
    }
    rows = rows.push(
        row![fader_widget, icon_col]
            .spacing(theme::SPACE_TIGHT)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
    );

    let db_display: Element<'a, Message> = if p.editing {
        text_input("", p.edit_buf)
            .on_input(Message::EditChanged)
            .on_submit(Message::EditCommit)
            .style(theme::text_input)
            .size(theme::TEXT_SM * scale)
            .width(Length::Fixed(64.0 * scale))
            .into()
    } else {
        let initial = if p.vol > 0.0 {
            format!("{:.1}", 20.0 * p.vol.log10())
        } else {
            "-inf".into()
        };
        mouse_area(
            text(db_text(p.vol))
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_XS * scale)
                .wrapping(advanced_text::Wrapping::None),
        )
        .on_double_click(Message::EditStart(cid, initial))
        .into()
    };
    // Collapse trigger — a small pill next to the dB readout, TotalMix-
    // style (confirmed via the reference: "0.0" then a "-" pill on the
    // same row, at the bottom of the strip, not up near the fader/gear
    // column). Moved out of `icon_col` at the user's request; plain "-"
    // rather than the em dash it briefly was — now that it's sitting in
    // the exact spot and shape the reference uses, matching that glyph
    // too seemed like the better default over inventing a new one.
    //
    // Plain `text(...)` here, not `centered_label` — `centered_label`
    // wraps its text in a `container(...).center(Length::Fill)`, which
    // needs a *bounded* parent to fill. Every other `centered_label`
    // button in this file pins that down with an explicit `.width()`/
    // `.height()` on the button; this one doesn't (it's meant to size to
    // its own content, pill-style, like the route/LOOP buttons below).
    // Skipping that pin left a `Length::Fill` with nothing to fill,
    // which didn't just mis-size this one button — it broke layout for
    // the *entire* strip list (every strip rendered as a blank card,
    // confirmed live and bisected line-by-line before landing on this).
    //
    // `.width(ICON_BTN_W * scale)` added back on below (unlike the
    // "skip it" lesson above, this one's deliberate) so this button has
    // a known, fixed footprint — needed to build a *symmetric* row: a
    // same-width blank spacer on the left, this button on the right,
    // with `db_display` centered in what's left between them. That
    // symmetry is what makes `db_display` land exactly under the
    // fader's own center rather than the meter's — see the row below.
    let collapse_btn = hint(
        button(text("-").size(theme::TEXT_SM * scale))
            .padding([theme::SPACE_HAIRLINE * scale, theme::SPACE_SM * scale])
            .width(ICON_BTN_W * scale)
            .style(theme::plain_button)
            .on_press(Message::ToggleCollapse(cid)),
        "Collapse",
        scale,
    );
    // Centered on the fader, not the meter — the fader's own row above
    // is centered on the *full* row width (`Fader::reserved_right`
    // accounts for `icon_col` so the track lands at exactly
    // `content_width / 2`), so centering `db_display` in a container
    // spanning that same full width lines the two up automatically.
    // A plain `row![db_display, Space::Fill, collapse_btn]` (the
    // previous shape) instead centered it in the *meter's* half of the
    // row — TotalMix-accurate positionally (matches the reference), but
    // not what the user asked for this time, which was explicitly
    // "under the fader." The left spacer matches `collapse_btn`'s own
    // fixed width exactly so the centered container's *available* width
    // stays symmetric around the row's true center, rather than losing
    // width asymmetrically to a right-hand sibling the way the fader
    // row itself used to (see `Fader::layout_x`'s doc comment).
    rows = rows.push(
        row![
            iced::widget::Space::new().width(Length::Fixed(ICON_BTN_W * scale)),
            container(db_display).width(Length::Fill).center_x(Length::Fill),
            collapse_btn,
        ]
        .spacing(theme::SPACE_TIGHT)
        .align_y(iced::Alignment::Center)
        .width(Length::Fill),
    );

    // TotalMix-style per-strip route: a compact trigger, centered on the
    // strip, opening `app.rs::route_popover` — a flyout that slides out
    // over the strip to the right rather than a plain dropdown (see the
    // route-panel plan). Drives the same `state.sel_out` the top bar's own
    // Submix picker does — a shortcut for it, not an independent
    // per-channel destination (TuxMix's model has exactly one "current
    // bus" concept, not TotalMix's separate "free" per-channel routing
    // mode). Centering it (rather than right-aligning) is purely visual —
    // the flyout itself still opens from the strip's right edge, unrelated
    // to where this trigger sits within the strip.
    if !matches!(cid, ChannelId::Output(_)) {
        rows = rows.push(
            container(hint(
                button(
                    row![
                        text(OUT_LABELS[out])
                            .size(theme::TEXT_XS * scale)
                            .wrapping(advanced_text::Wrapping::None),
                        text("▸").size(theme::TEXT_XS * scale),
                    ]
                    .spacing(theme::SPACE_TIGHT),
                )
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                .style(theme::toggle_button(p.open_flyout == Some(FlyoutKind::Route), theme::ACCENT))
                .on_press(Message::ToggleFlyout(cid, FlyoutKind::Route)),
                "Change output bus",
                scale,
            ))
            .padding(iced::Padding {
                top: theme::SPACE_MD * scale,
                ..iced::Padding::ZERO
            })
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    } else if let ChannelId::Output(i) = cid {
        // Outputs have no route trigger (a single master, not a
        // per-submix crosspoint) — the same row slot shows the loopback
        // toggle instead (`OutputChannel::loopback`, bReq 0x15: feeds the
        // input signal back into this output's own playback path). The
        // STEREO link/split toggle lives in the Settings flyout now
        // (`app.rs::settings_popover`), same as every other channel kind.
        rows = rows.push(
            container(hint(
                button(
                    text("LOOP")
                        .size(theme::TEXT_XS * scale)
                        .wrapping(advanced_text::Wrapping::None),
                )
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                    .style(theme::toggle_button(p.loopback, theme::ACCENT))
                    .on_press(Message::LoopbackChanged(i, !p.loopback)),
                "Loopback — feed the input signal back into this output",
                scale,
            ))
            .padding(iced::Padding {
                top: theme::SPACE_MD * scale,
                ..iced::Padding::ZERO
            })
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }

    // Double-click anywhere on the card that isn't already claimed by a
    // specific control (the fader/pan canvases capture their own
    // double-click for reset-to-default, the dB readout for its edit
    // field, buttons for their own press) collapses the strip — a bigger,
    // more discoverable target than the tiny "-" button alone. A plain
    // click there is a no-op; Ctrl/Shift+click toggles multi-selection
    // (see `Message::StripClicked`) — mute/solo/collapse on any selected
    // strip then apply to the whole selection at once.
    mouse_area(
        container(
            rows.width(Length::Fill)
                .align_x(iced::Alignment::Center),
        )
        .style(theme::strip_panel(
            p.selected,
            p.hovered,
            p.type_tag.map(|(_, c)| c),
        ))
        // Bottom padding wider than the top three sides — the route
        // trigger (the card's last row) otherwise sat almost flush against
        // the card's own bottom edge, reading as cramped next to the dB
        // readout right above it.
        .padding(iced::Padding {
            top: theme::SPACE_SM * scale,
            right: theme::SPACE_MD * scale,
            bottom: theme::SPACE_XL * scale,
            left: theme::SPACE_MD * scale,
        })
        .width(Length::Fixed(w * scale))
        .clip(true),
    )
    .on_press(Message::StripClicked(cid))
    .on_double_click(Message::ToggleCollapse(cid))
    .on_enter(Message::StripHovered(Some(cid)))
    .on_exit(Message::StripHovered(None))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn settled_anim_is_never_settling() {
        let a = CollapseAnim { prev: STRIP_W, value: STRIP_W, since: Instant::now() };
        assert!(!a.is_settling(a.since));
        assert!(!a.is_settling(a.since + Duration::from_millis(10)));
    }

    #[test]
    fn transitioning_anim_settles_after_the_interp_window() {
        let a = CollapseAnim { prev: STRIP_W, value: COLLAPSED_W, since: Instant::now() };
        assert!(a.is_settling(a.since), "just started — should still be settling");
        assert!(
            a.is_settling(a.since + Duration::from_millis(80)),
            "mid-transition — should still be settling"
        );
        assert!(
            !a.is_settling(a.since + Duration::from_millis(200)),
            "past the interp window — should have stopped requesting redraws"
        );
    }

    #[test]
    fn at_interpolates_linearly_from_prev_to_value() {
        let a = CollapseAnim { prev: STRIP_W, value: COLLAPSED_W, since: Instant::now() };
        assert_eq!(a.at(a.since), STRIP_W);
        assert_eq!(a.at(a.since + Duration::from_millis(160)), COLLAPSED_W);
        let mid = a.at(a.since + Duration::from_millis(80));
        let expected_mid = (STRIP_W + COLLAPSED_W) / 2.0;
        assert!(
            (mid - expected_mid).abs() < 0.5,
            "expected ~{expected_mid} at the midpoint, got {mid}"
        );
    }
}
