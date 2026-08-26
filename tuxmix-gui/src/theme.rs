//! Color palette shared across the whole GUI, ported 1:1 from the v1 (egui) palette.

use iced::gradient::Linear;
use iced::widget::overlay::menu;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow};

pub const BG_DEEP: Color = Color::from_rgb8(0x0d, 0x0d, 0x0d);
pub const SURFACE: Color = Color::from_rgb8(0x18, 0x18, 0x1a);
pub const BORDER: Color = Color::from_rgb8(0x2a, 0x2a, 0x30);
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xe8, 0xe8, 0xec);
pub const TEXT_SEC: Color = Color::from_rgb8(0x88, 0x88, 0x94);
pub const ACCENT: Color = Color::from_rgb8(0x4f, 0xc3, 0xf7);
pub const ACCENT_DIM: Color = Color::from_rgb8(0x2a, 0x6a, 0x88);
/// Sober neutral used for the fader rail/handle — kept separate from
/// `ACCENT` so the fader doesn't compete visually with the blue brand color.
pub const FADER: Color = Color::from_rgb8(0xa8, 0xac, 0xb4);
pub const MGREEN: Color = Color::from_rgb8(0x4c, 0xaf, 0x50);
pub const MRED: Color = Color::from_rgb8(0xf4, 0x43, 0x36);
pub const PHANTOM: Color = Color::from_rgb8(0xff, 0x45, 0x45);
pub const GCONN: Color = Color::from_rgb8(0x4c, 0xaf, 0x50);
pub const YSIM: Color = Color::from_rgb8(0xff, 0xc1, 0x07);
pub const MUTE_COLOR: Color = Color::from_rgb8(0xff, 0x6b, 0x6b);
pub const SOLO_COLOR: Color = Color::from_rgb8(0xff, 0xc1, 0x07);
pub const ON_ACTIVE: Color = Color::from_rgb8(0x1a, 0x08, 0x08);

// ── Type scale ───────────────────────────────────────────────────
// Every text element in the app maps to one of these six tiers rather
// than a one-off pixel value chosen per call site — before this there
// were eight near-random sizes (6.5, 8, 9, 10, 11, 12, 13, 20) with no
// clear reason two of them (8 vs 9, 11 vs 12) were ever different.
//
// These are the sizes at `ui_scale == 1.0` — bumped up from the original
// pass (11px body text read as small for a desktop app) so the default,
// unscaled UI is already comfortable; `ui_scale` (see `SCALE_DEFAULT`)
// only moves off 1.0 when the window is too narrow/wide to fit the
// widest strip row at that baseline.

/// Canvas-drawn ruler tick labels on the fader — the smallest legible
/// size, only used where the fader canvas has no room for anything
/// bigger.
pub const TEXT_MICRO: f32 = 8.0;
/// Tertiary annotations: type tags, dB readouts, the pan L/R/C label,
/// matrix-view row/column labels.
pub const TEXT_XS: f32 = 10.0;
/// Buttons and compact controls: M/S, 48V/PAD, the collapse toggle,
/// the dB edit input.
pub const TEXT_SM: f32 = 11.0;
/// Default body text: channel names, section headers, top-bar labels
/// and pick lists.
pub const TEXT_MD: f32 = 13.0;
/// Emphasis: the connected device's model name in the top bar.
pub const TEXT_LG: f32 = 15.0;
/// The "TuxMix" wordmark.
pub const TEXT_XL: f32 = 22.0;

/// Adaptive UI scale — recomputed on every window resize (see
/// `recompute_ui_scale`) so the widest strip row always fits the window
/// without horizontal scrolling, TotalMix-2.0-style; not a manual zoom
/// control. Multiplied into every text size and widget dimension in the
/// mixer/matrix views — see `TuxMix::ui_scale`. `SCALE_DEFAULT` is the
/// fallback when there's nothing to size against yet (no channels); `MIN`/
/// `MAX` bound how far the auto-fit math is allowed to shrink/grow it.
pub const SCALE_DEFAULT: f32 = 1.0;
pub const SCALE_MIN: f32 = 0.75;
pub const SCALE_MAX: f32 = 1.75;

// ── Corner radius scale ─────────────────────────────────────────────
// Same idea as the type scale: before this, `panel` used 7 and `chip`
// used 6 with no reason they were ever different — just whatever felt
// fine in isolation when each was written.

/// The section-header accent tick — barely-there rounding on a thin bar.
pub const RADIUS_XS: f32 = 2.0;
/// Every interactive control: buttons, text inputs, pick lists/menus, the
/// scrollbar scroller.
pub const RADIUS_SM: f32 = 4.0;
/// Container chrome: strip cards (`panel`) and top-bar groups (`chip`) —
/// previously 7 and 6 respectively, for no reason they needed to differ.
pub const RADIUS_MD: f32 = 8.0;

// ── Spacing scale ────────────────────────────────────────────────────
// Same consolidation for `.spacing()`/`.padding()` call sites — before
// this there were eleven near-arbitrary values (1, 2, 3, 4, 5, 6, 8, 10,
// 12, 14, 16) scattered across app.rs/strip.rs/matrix.rs with no shared
// vocabulary. Values are the *smallest* unit of a given role, not a
// strict multiplier progression — the goal is a short, named list callers
// can pick from, not a formula.

/// Between rows stacked tightly inside one strip's control stack (header
/// → M/S → 48V/PAD → fader → dB → pan) and between a pan dot and its
/// label — these read as one continuous unit, not separate rows.
pub const SPACE_HAIRLINE: f32 = 1.0;
/// Between closely related inline siblings: a channel name and its type
/// tag, the M/S button pair, a matrix cell grid, the MIXER/MATRIX toggle.
pub const SPACE_TIGHT: f32 = 2.0;
/// Small control padding — e.g. the top-bar view-toggle buttons.
pub const SPACE_SM: f32 = 4.0;
/// Strip card padding, strip-to-strip gaps, a chip's internal row
/// spacing.
pub const SPACE_MD: f32 = 6.0;
/// Section-level spacing: the gap between HARDWARE INPUTS / SOFTWARE
/// PLAYBACK / HARDWARE OUTPUTS blocks, a section header's own row.
pub const SPACE_LG: f32 = 8.0;
/// Container-level padding: a chip's horizontal padding, the mixer/matrix
/// view's outer padding.
pub const SPACE_XL: f32 = 12.0;
/// The top bar's outer padding and the gap between its identity/nav/
/// session groups.
pub const SPACE_XXL: f32 = 16.0;

pub fn panel(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        text_color: Some(TEXT_PRIMARY),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Linear-interpolates `base` toward `tint` by `amount` (0 = pure `base`,
/// 1 = pure `tint`), keeping `base`'s own alpha — used to tint a strip
/// card just enough to read as "this channel's category" at a glance
/// without competing with the accent-colored selection border or making
/// the card itself look like a colored badge.
fn blend(base: Color, tint: Color, amount: f32) -> Color {
    Color {
        r: base.r + (tint.r - base.r) * amount,
        g: base.g + (tint.g - base.g) * amount,
        b: base.b + (tint.b - base.b) * amount,
        a: base.a,
    }
}

/// How strongly a strip's `tint` (its channel type's own color — the same
/// one already used for its type-tag badge) is blended into the card
/// background. Subtle on purpose: enough to sort channel types apart at a
/// glance in a long scrolled row, not so much it reads as a colored
/// button the way the type tag itself does.
const STRIP_TINT_AMOUNT: f32 = 0.10;

/// A strip's card — same as `panel`, but with an accent-colored border
/// when part of the active multi-selection (Ctrl/Shift+click), so grouped
/// mute/solo/collapse actions have a clear "this is what I'm about to
/// affect" indicator — and, if `tint` is given, a background nudged
/// toward that channel type's own color (the same color as its type-tag
/// badge), so a long row of strips sorts into visual groups before you
/// even read the labels.
/// `hovered` is ignored whenever `selected` is true — a strip that's part
/// of the active multi-selection already has the loudest border in the
/// row; a hover glow on top of that would just be visual noise competing
/// with itself. Plain `container::Style` closures don't get a `Status` the
/// way `button`'s do, so this hover state has to be tracked explicitly
/// (`TuxMix::hovered_strip`, driven by `mouse_area::on_enter`/`on_exit` in
/// `widgets/strip.rs`) rather than read off the widget itself.
pub fn strip_panel(
    selected: bool,
    hovered: bool,
    tint: Option<Color>,
) -> impl Fn(&iced::Theme) -> container::Style {
    move |theme| {
        let base = panel(theme);
        let base = match tint {
            Some(t) => container::Style {
                background: Some(Background::Color(blend(SURFACE, t, STRIP_TINT_AMOUNT))),
                ..base
            },
            None => base,
        };
        if selected {
            container::Style {
                border: Border {
                    color: ACCENT,
                    width: 2.0,
                    ..base.border
                },
                ..base
            }
        } else if hovered {
            container::Style {
                border: Border {
                    color: FADER,
                    width: 1.0,
                    ..base.border
                },
                shadow: Shadow {
                    color: Color { a: 0.25, ..ACCENT },
                    offset: iced::Vector::new(0.0, 0.0),
                    blur_radius: 8.0,
                },
                ..base
            }
        } else {
            base
        }
    }
}

/// A flat `BG_DEEP` read as an undifferentiated slab under the strip
/// cards. A barely-there top-to-bottom falloff (a few percent of white
/// blended in at the top, gone entirely by the bottom) reads as soft
/// ambient light instead — the same trick most DAW backgrounds use — without
/// the gradient itself ever being something you'd consciously notice.
pub fn root(_theme: &iced::Theme) -> container::Style {
    // `Radians::to_distance` subtracts another `FRAC_PI_2` internally
    // before turning this into a direction vector (its `0` is "to top",
    // matching CSS's `linear-gradient()` angle convention) — `PI`, not
    // `FRAC_PI_2`, is what actually points top-to-bottom. Verified against
    // `iced_core::angle::Radians::to_distance` source directly, not guessed
    // — a `FRAC_PI_2` first attempt rendered as a (barely visible,
    // near-flat) *horizontal* gradient instead.
    let gradient = Linear::new(std::f32::consts::PI)
        .add_stop(0.0, blend(BG_DEEP, Color::WHITE, 0.035))
        .add_stop(1.0, BG_DEEP);
    container::Style {
        background: Some(Background::from(gradient)),
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

/// Slightly darker than `SURFACE` (the strip-card background) — previously
/// identical, which flattened the top bar and the mixer strips into one
/// undifferentiated slab. Receding it toward `BG_DEEP` reads as a frame the
/// strip cards sit in front of, without introducing a third named color.
pub fn top_bar(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(blend(SURFACE, BG_DEEP, 0.45))),
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

/// A small square toggle button (M / S / 48V / PAD) that lights up `active_color`
/// when `active` — with a soft glow to sell the "lit" look — and responds to
/// hover when inactive so it reads as clickable before you ever press it.
/// How much a `Pressed` button's background darkens versus its resting
/// `Hovered` look — the two used to render identically, so a click gave
/// no tactile "it went down" feedback, just the same hover state held a
/// little longer.
const PRESS_DARKEN: f32 = 0.30;

pub fn toggle_button(
    active: bool,
    active_color: Color,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(
            status,
            button::Status::Hovered | button::Status::Pressed
        );
        let pressed = matches!(status, button::Status::Pressed);

        let mut background = if active {
            active_color
        } else if hovered {
            SURFACE
        } else {
            BG_DEEP
        };
        if pressed {
            background = blend(background, Color::BLACK, PRESS_DARKEN);
        }
        let border_color = if active {
            active_color
        } else if hovered {
            FADER
        } else {
            BORDER
        };
        let shadow = if active {
            Shadow {
                color: Color {
                    a: if pressed { 0.25 } else { 0.45 },
                    ..active_color
                },
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: if pressed { 3.0 } else { 6.0 },
            }
        } else {
            Shadow::default()
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color: if active { ON_ACTIVE } else { TEXT_SEC },
            border: Border {
                color: border_color,
                width: 1.0,
                radius: RADIUS_SM.into(),
            },
            shadow,
            snap: false,
        }
    }
}

/// The MIXER/MATRIX view switch in the top bar — a plain-text segmented
/// pair rather than another boxed `chip`, so it doesn't compete for
/// attention with the device-identity chip next to it. Active tab gets a
/// filled pill (same `ACCENT_DIM` used elsewhere for "selected"); inactive
/// stays flat until hovered.
pub fn tab_toggle(active: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let pressed = matches!(status, button::Status::Pressed);
        let background = if active {
            if pressed {
                blend(ACCENT_DIM, Color::BLACK, PRESS_DARKEN)
            } else {
                ACCENT_DIM
            }
        } else if pressed {
            // The resting/hover states here are SURFACE/TRANSPARENT, and
            // TRANSPARENT has nothing for `blend` toward black to darken —
            // give the inactive tab its own distinct, visible press color
            // instead of falling through to "looks like hover held longer."
            Color { a: 0.35, ..ACCENT_DIM }
        } else if hovered {
            SURFACE
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: if active { TEXT_PRIMARY } else { TEXT_SEC },
            border: Border {
                radius: RADIUS_SM.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

pub fn plain_button(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Pressed => blend(ACCENT_DIM, Color::BLACK, PRESS_DARKEN),
        button::Status::Hovered => ACCENT_DIM,
        _ => SURFACE,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// The small colored tick to the left of a section header label.
pub fn accent_bar(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ACCENT)),
        border: Border {
            radius: RADIUS_XS.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A recessed group container — used to visually cluster related top-bar
/// controls (e.g. the scene name/save/load trio) against the flatter
/// top-bar background.
pub fn chip(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_DEEP)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        text_color: Some(TEXT_PRIMARY),
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn text_input(
    _theme: &iced::Theme,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    use iced::widget::text_input::Status;
    let border_color = match status {
        Status::Focused { .. } => FADER,
        _ => BORDER,
    };
    iced::widget::text_input::Style {
        background: Background::Color(SURFACE),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        icon: TEXT_SEC,
        placeholder: TEXT_SEC,
        value: TEXT_PRIMARY,
        selection: ACCENT_DIM,
    }
}

pub fn pick_list(
    _theme: &iced::Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list::Status;
    let border_color = match status {
        Status::Opened { .. } | Status::Hovered => FADER,
        Status::Active => BORDER,
    };
    iced::widget::pick_list::Style {
        text_color: TEXT_PRIMARY,
        placeholder_color: TEXT_SEC,
        handle_color: TEXT_SEC,
        background: Background::Color(SURFACE),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
    }
}

/// The default 10px scrollbar/scroller reads chunky next to everything
/// else we've thinned down — 4px is still comfortably grabbable but looks
/// modern rather than like default browser chrome.
pub fn thin_scrollbar() -> iced::widget::scrollable::Scrollbar {
    iced::widget::scrollable::Scrollbar::new()
        .width(4)
        .scroller_width(4)
}

/// A thin, minimal scrollbar rail — the scroller brightens on hover/drag
/// instead of the default Iced chrome, matching the rest of the app.
pub fn scrollable(
    _theme: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::widget::scrollable::{AutoScroll, Rail, Scroller, Status};

    let (h_active, v_active) = match status {
        Status::Active { .. } => (false, false),
        Status::Hovered {
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
            ..
        } => (
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
        ),
        Status::Dragged {
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
            ..
        } => (
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
        ),
    };

    let rail = |active: bool| Rail {
        background: Some(Background::Color(Color::TRANSPARENT)),
        border: Border::default(),
        scroller: Scroller {
            background: Background::Color(if active { FADER } else { BORDER }),
            border: Border {
                radius: RADIUS_SM.into(),
                ..Border::default()
            },
        },
    };

    iced::widget::scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail(v_active),
        horizontal_rail: rail(h_active),
        gap: None,
        auto_scroll: AutoScroll {
            background: Background::Color(SURFACE),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: TEXT_PRIMARY,
        },
    }
}

pub fn menu(_theme: &iced::Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(SURFACE),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        text_color: TEXT_PRIMARY,
        selected_text_color: TEXT_PRIMARY,
        selected_background: Background::Color(ACCENT_DIM),
        shadow: Shadow::default(),
    }
}
