//! A single channel strip: label + type tag, mute/solo, 48V/PAD, fader+VU,
//! dB readout (double-click to edit), pan readout.

use iced::keyboard::Modifiers;
use iced::widget::{button, column, container, mouse_area, row, text, text_input, tooltip};
use iced::{Color, Element, Length};
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
pub(crate) const STRIP_W: f32 = 80.0;
/// Collapsed strips need to stand the same total height as a full strip
/// (see `collapsed_strip`'s own doc comment) despite having far fewer
/// rows — name/M/S/meter/expand vs. the full strip's
/// header/pan/M-S/fader/dB/route. The meter is what absorbs the
/// difference. `76.0` is that difference, empirically measured (via
/// `run-gui-headless`, pixel-measuring both card heights side by side and
/// dividing by the known `ui_scale` at the time) rather than summed from
/// the two layouts' individual row heights — line-height metrics for
/// text aren't available as plain constants to sum by hand, and a
/// measured, working number is more trustworthy than a hand-derived one
/// that only looks right on paper.
const COLLAPSED_METER_H: f32 = FADER_H + 76.0;
/// Collapsed strips are a glance-only readout: name + VU meter, nothing
/// else — no fader, no mute/solo, no pan. Trading away every control for
/// space is the point; a strip you still need to touch shouldn't be
/// collapsed. Width is set by the header (name + expand button), not the
/// meter, which is narrower than that on its own. Deliberately the same
/// `COLLAPSED_W` for every channel kind, output strips included — collapsing
/// is opting *out* of emphasis, so there's nothing to differentiate here.
pub(crate) const COLLAPSED_W: f32 = 60.0;

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

/// Which flyout (at most one, and only one per strip) is open — the gear
/// icon opens `Settings` (48V/PAD/Sensitivity, moved out of the strip's
/// own vertical flow so it doesn't grow/shrink the whole card; Gain stays
/// inline instead — see `app.rs::settings_popover`'s doc comment for why),
/// the route trigger opens `Route` (which output bus). Both render as the
/// same kind of animated panel sliding out over the strip's right
/// neighbor — see `app.rs::with_flyout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutKind {
    Route,
    Settings,
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
    /// Which flyout (if any) is currently open *for this strip* — lights
    /// up the matching trigger (gear icon for `Settings`, the route
    /// button for `Route`); `app.rs` owns the actual popover content.
    pub open_flyout: Option<FlyoutKind>,
    pub mute: bool,
    pub solo: bool,
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
fn hint<'a>(content: impl Into<Element<'a, Message>>, label: &'a str, scale: f32) -> Element<'a, Message> {
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


/// Full-strip header — just the name (+ type tag), centered, TotalMix-style
/// (see the reference design: no icons in the header at rest). See
/// `collapsed_header_row` for the collapsed strip's own header, which
/// still needs its chevron.
fn header_row<'a>(
    name: &str,
    type_tag: Option<(&'static str, Color)>,
    scale: f32,
) -> Element<'a, Message> {
    let mut header = row![text(short_label(name).to_string()).size(theme::TEXT_MD * scale)]
        .spacing(theme::SPACE_HAIRLINE);
    if let Some((tag, color)) = type_tag {
        header = header.push(text(tag).color(color).size(theme::TEXT_XS * scale));
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
        text(short_label(&p.name).to_string()).size(theme::TEXT_SM * scale),
        mute_btn,
        solo_btn,
        container(vu_meter(p.meter, COLLAPSED_METER_H * scale, scale))
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

    let has_settings = p.has_48v || p.has_pad || p.has_sensitivity || p.has_gain;
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
    let ms_row = row![mute_btn, solo_btn].spacing(theme::SPACE_TIGHT).width(Length::Fill);

    let mut rows = column![header].spacing(theme::SPACE_HAIRLINE);

    // Pan sits directly under the header, TotalMix-style, rather than
    // trailing at the very bottom — with pan above the fader and the
    // route picker (below) after it, the fader ends up vertically
    // centered in the strip's own flow instead of front-loaded.
    // Outputs have no per-channel pan in the device model (a single
    // master volume covers the stereo pair) — only inputs/playbacks
    // route to a pan position within each output.
    if !matches!(cid, ChannelId::Output(_)) {
        let pan_str = match p.pan.cmp(&0) {
            std::cmp::Ordering::Less => format!("L{}", -p.pan),
            std::cmp::Ordering::Greater => format!("R{}", p.pan),
            std::cmp::Ordering::Equal => "C".to_string(),
        };
        rows = rows.push(
            container(hint(
                knob(Knob {
                    value: p.pan as f32,
                    range: (-100.0, 100.0),
                    label: pan_str,
                    modifiers: p.modifiers,
                    scale,
                    on_change: Box::new(move |v| Message::PanChanged(cid, out, v.round() as i8)),
                    on_reset: Box::new(move || Message::PanReset(cid, out)),
                }),
                "Pan — drag, scroll, or double-click to reset",
                scale,
            ))
            .width(Length::Fill)
            .center_x(Length::Fill),
        );
    }

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
        height: fader_h,
        show_meter: true,
        modifiers: p.modifiers,
        scale,
        on_press: Box::new(move |v, range| Message::FaderPressed(cid, out, v, range)),
        on_drag: Box::new(move |v| Message::VolumeChanged(cid, out, v)),
        on_release: Box::new(move || Message::RangeCleared(cid)),
        on_reset: Box::new(move || Message::Reset(cid, out, default_vol)),
    });

    // Settings gear (when this channel has any) and the collapse chevron
    // stacked on the strip's right edge, alongside the fader — TotalMix-
    // style (see the reference design: a small vertical icon stack next
    // to the fader, roughly at its mid-height, not in the header or down
    // by the dB readout). A gear placed inside the old Stack-based
    // settings flyout broke click handling for the whole row (see
    // `app::settings_popover`'s doc comment) — that failure mode doesn't
    // apply here, this column isn't a flyout, just an ordinary part of
    // the strip's own layout. The fader itself doesn't need to know this
    // column exists: its own width is `Length::Fill`, so it simply gets
    // whatever's left in the row and re-centers its track within that
    // (see `Fader::track_x`) — no coordination required.
    let mut icon_col = column![].spacing(theme::SPACE_TIGHT).align_x(iced::Alignment::Center);
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
    icon_col = icon_col.push(hint(
        button(centered_label("▼", theme::TEXT_MICRO * scale))
            .padding(0)
            .width(ICON_BTN_W * scale)
            .height(ICON_BTN_H * scale)
            .style(theme::plain_button)
            .on_press(Message::ToggleCollapse(cid)),
        "Collapse",
        scale,
    ));
    rows = rows.push(
        row![fader_widget, icon_col]
            .spacing(theme::SPACE_TIGHT)
            .align_y(iced::Alignment::Center)
            .width(Length::Fill),
    );

    let db_row: Element<'a, Message> = if p.editing {
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
                .size(theme::TEXT_XS * scale),
        )
        .on_double_click(Message::EditStart(cid, initial))
        .into()
    };
    rows = rows.push(db_row);

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
                        text(OUT_LABELS[out]).size(theme::TEXT_XS * scale),
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
