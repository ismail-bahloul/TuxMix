//! The right-hand "control strip" sidebar — TotalMix's own device chip,
//! Undo/Redo, Snapshots, Groups, and Mixer Layout panels, plus a
//! visual-only M/S/F row and Options panel.
//!
//! Not everything here is wired to real behavior — see each panel's own
//! doc comment. Snapshots/Undo-Redo/Groups/Mixer-Layout are real;
//! the M/S/F row, "FX show %", and several Options rows are deliberate
//! visual skeleton (per the user's own framing: build the shape, don't
//! guess at behavior a static reference screenshot can't confirm).
//!
//! Visual weight is deliberate, not incidental. Every control in this
//! column is the same 210px wide, so the only things telling the eye what
//! is structure, what is content and what is merely available are
//! contrast and spacing. The rules this file follows, in one place:
//!
//! * The device chip is the anchor: the only boxed, `TEXT_MD` element.
//! * Panel headers carry the structure — accent tick + uppercase label +
//!   trailing rule, the vocabulary `app.rs::section_header` already uses
//!   in the mixer — and each panel's content is indented under them.
//! * Full-strength `ACCENT` is reserved for things that are actually on
//!   (`theme::toggle_button`: the M/S pair, a group link). "Which side a
//!   switch points at" and "which slot is selected" use the dim
//!   graphite thumb instead (`theme::segment_button`, and
//!   `theme::segment_cell` inside a joined `segmented_row`), so a
//!   position no longer reads as a state.
//! * Secondary actions (undo/redo, "store", the collapse toggle) are flat
//!   text (`theme::ghost_button`) instead of more boxes.
//! * Rows inside a panel are `SPACE_TIGHT` apart, panels are twice that
//!   (see `panel`), and the whole column is one scrollable strip.

use iced::widget::{button, column, container, row, text};
use iced::{Color, Element, Length};

use tuxmix_core::{ChannelId, RmeDevice};

use crate::app::{Message, TuxMix};
use crate::theme;

/// Default sidebar width (at `scale == 1.0`), used until the user drags
/// the resize strip — wide enough for the longest label in the Options
/// panel ("solo/pfl mode") without wrapping. Its rows act as a
/// self-contained readout, so this is a starting point rather than
/// something the layout negotiates with the mixer.
pub const DEFAULT_WIDTH: f32 = 210.0;
/// Bounds on `TuxMix::sidebar_width`. The floor is set by the same
/// measurement as `DEFAULT_WIDTH`: below roughly 200 the Options rows'
/// two segments stop fitting their labels, and iced wraps the text onto a
/// second line (a taller row, not a broken one) — allowed down to this
/// because reclaiming mixer space is the point of a resize, and the drag
/// is trivially reversible. The ceiling is where the mixer's own strips
/// stop having room to spare on a 1920px window.
pub const MIN_WIDTH: f32 = 170.0;
pub const MAX_WIDTH: f32 = 420.0;
/// Width of the collapsed rail — just enough for the expand button, so
/// collapsing the sidebar reclaims most of its width for the mixer
/// without losing the one control needed to bring it back (an
/// entirely-gone sidebar would have nowhere left to click).
pub const COLLAPSED_WIDTH: f32 = 22.0;

/// Left indent of a panel's *content*, so it sits under the panel header's
/// label rather than under its accent tick — the one bit of inset the
/// column has, and what makes a panel read as one block.
const CONTENT_INDENT: f32 = theme::SPACE_LG;

/// A left-only padding of `left` (iced 0.14's `Padding` has no
/// four-element array conversion, only `[vertical, horizontal]`).
const fn indent(left: f32) -> iced::Padding {
    iced::Padding {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left,
    }
}

/// Width of the Options panel's caption column. Sized for the longest
/// caption the reference uses there ("solo/pfl mode") at `TEXT_XS`.
const OPTIONS_LABEL_W: f32 = 76.0;

/// Width of the fold tab that straddles the sidebar's inner edge. Narrow
/// enough to read as a grip on the edge (`theme::tab_handle` finishes it
/// as a capsule) rather than a button parked next to it.
const TAB_W: f32 = 14.0;
/// Height of that tab — tall enough that the capsule reads as a handle,
/// short enough that it doesn't read as a scrollbar segment.
const TAB_H: f32 = 52.0;
/// How much of the tab overlaps the sidebar itself — exactly the column's
/// own left padding (`SPACE_MD`), so the tab hangs over the mixer's side
/// of the boundary without ever covering a row. The rest of it
/// (`TAB_W - TAB_INSIDE` = 12px) lands in the mixer page's own right
/// padding (`SPACE_XL`), which is empty by construction.
const TAB_INSIDE: f32 = theme::SPACE_MD;

/// Which sidebar panel a `Message::ToggleSidebarPanel` collapses/expands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Options,
    Snapshots,
    Groups,
    Layout,
}

/// A 2-way segmented toggle with no behavior behind it — the row still
/// flips which side is highlighted (so it *reads* as a working control,
/// not a dead button), it just doesn't change anything else. Used for
/// every Options-panel row whose real TotalMix semantics aren't
/// confirmed (`show`, `2 row`, `solo/pfl mode`) and for the Mixer
/// Layout panel's `all`/`submix` scope row, which has no real
/// distinction to key off yet — `state.collapsed` isn't scoped
/// per-submix anywhere in the current model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkeletonPair {
    /// "names" / "trim".
    Show,
    /// "2 row" / "2 row in".
    RowMode,
    /// "excl. solo" / "live" — real non-exclusive solo would mean
    /// touching `usb.rs`'s hardcoded exclusive-solo logic; scoped out
    /// of this pass, see `GUI-NOTES.md`.
    SoloMode,
    /// "all" / "submix", in the Mixer Layout panel.
    LayoutScope,
}

/// Which of a group's three independent links (`GroupLinkToggle`
/// flips) a click on one of its `M1`/`S1`/`1`-style cells targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupLink {
    Mute,
    Solo,
    Fader,
}

/// One of the 4 mute/solo/fader groups — a pure GUI-side convenience
/// over the existing per-channel `set_mute`/`set_solo`/volume calls
/// (not a device concept, so this lives here rather than in
/// `tuxmix-core`). All three link types share one membership list,
/// matching the reference's layout (one `M1`/`S1`/`1` row per group,
/// not three independent membership lists) — assigned via the
/// *existing* multi-select (`TuxMix::selected`) rather than a bespoke
/// picker UI: enter edit mode, select strips the normal Ctrl/Shift-click
/// way, click the group cell to assign.
#[derive(Clone, Debug, Default)]
pub struct Group {
    pub members: Vec<ChannelId>,
    pub mute_linked: bool,
    pub solo_linked: bool,
    pub fader_linked: bool,
}

/// Which sidebar panels are expanded — every panel defaults open,
/// matching the reference.
#[derive(Clone, Copy, Debug)]
pub struct PanelsOpen {
    pub options: bool,
    pub snapshots: bool,
    pub groups: bool,
    pub layout: bool,
}

impl Default for PanelsOpen {
    fn default() -> Self {
        Self {
            options: true,
            snapshots: true,
            groups: true,
            layout: true,
        }
    }
}

impl PanelsOpen {
    pub fn toggle(&mut self, panel: Panel) {
        let slot = match panel {
            Panel::Options => &mut self.options,
            Panel::Snapshots => &mut self.snapshots,
            Panel::Groups => &mut self.groups,
            Panel::Layout => &mut self.layout,
        };
        *slot = !*slot;
    }
}

/// State for every `SkeletonPair` — `true` = first option highlighted.
#[derive(Clone, Copy, Debug)]
pub struct SkeletonPairs {
    pub show_first: bool,
    pub row_mode_first: bool,
    pub solo_mode_first: bool,
    pub layout_scope_first: bool,
}

impl Default for SkeletonPairs {
    fn default() -> Self {
        Self {
            show_first: true,
            row_mode_first: true,
            solo_mode_first: true,
            layout_scope_first: true,
        }
    }
}

impl SkeletonPairs {
    pub fn toggle(&mut self, pair: SkeletonPair) {
        let slot = match pair {
            SkeletonPair::Show => &mut self.show_first,
            SkeletonPair::RowMode => &mut self.row_mode_first,
            SkeletonPair::SoloMode => &mut self.solo_mode_first,
            SkeletonPair::LayoutScope => &mut self.layout_scope_first,
        };
        *slot = !*slot;
    }
}

/// Width of the drag strip that resizes this column, on its inner edge.
/// Sized to the column's own left padding (`SPACE_MD`), and the column's
/// left padding is in turn dropped in its favour (`sidebar`'s own
/// `scrollable`): the strip occupies exactly the space nothing was drawn
/// in, so resizing costs the layout no width and every row still starts
/// `SPACE_MD` from the column's left edge.
const RESIZE_W: f32 = theme::SPACE_MD;

/// The fold control itself: a small grip whose chevron points the way
/// the column is about to move (right folds it away, left brings it
/// back). The chevron is canvas-drawn (`widgets::chevron`), not a font
/// glyph: at `TAB_W` wide, every candidate glyph's side bearing put its
/// ink visibly off the capsule's center, and `Button`'s own default
/// padding (5px a side) left no content box at all in a box this narrow —
/// padding zeroed, the canvas fills the whole tab.
fn tab_button(right: bool, scale: f32) -> Element<'static, Message> {
    button(
        iced::widget::canvas(crate::widgets::chevron::Chevron {
            color: theme::TEXT_SEC,
            right,
            scale,
        })
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(0)
    .width(Length::Fixed(TAB_W * scale))
    .height(Length::Fixed(TAB_H * scale))
    .style(theme::tab_handle)
    .on_press(Message::ToggleSidebar)
    .into()
}

/// Top-level entry point — one scrollable column, filling the window's
/// full height alongside the main content (see `app.rs::view`). Its width
/// is the user's (`TuxMix::sidebar_width`, dragged via `resize_strip`, so
/// a narrow one trades the Options labels' one-line fit for mixer space
/// and a wide one gives the Groups/Layout lists room). Collapsible:
/// `state.sidebar_open == false` renders just `COLLAPSED_WIDTH` of rail
/// with an expand button, reclaiming the rest of the width for the mixer —
/// the whole point being asked for ("rabattable"), not just a visual
/// nicety.
pub fn sidebar(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    // Collapsed, the rail is exactly the tab plus its own padding — the
    // one control that brings the column back *is* the whole rail, and
    // the tab is centered so it doesn't move height between states.
    if !state.sidebar_open {
        return container(tab_button(false, scale))
            .padding(theme::SPACE_TIGHT * scale)
            .align_y(iced::Alignment::Center)
            .width(Length::Fixed(COLLAPSED_WIDTH * scale))
            .height(Length::Fill)
            .style(theme::top_bar)
            .into();
    }

    let col = column![
        device_chip(state, scale),
        undo_redo_row(state, scale),
        msf_row(state, scale),
        fx_show_row(scale),
        panel(options_panel(state, scale), scale),
        panel(snapshots_panel(state, scale), scale),
        panel(groups_panel(state, scale), scale),
        panel(layout_panel(state, scale), scale),
    ]
    .spacing(theme::SPACE_MD * scale)
    // Left padding dropped in favour of the resize strip (`RESIZE_W`):
    // rows still start `SPACE_MD` from the column's edge, with the strip
    // sitting in what used to be that padding.
    .padding(iced::Padding {
        top: theme::SPACE_MD * scale,
        right: theme::SPACE_MD * scale,
        bottom: theme::SPACE_MD * scale,
        left: 0.0,
    })
    .width(Length::Fill);

    // Thin scrollbar, like `app.rs::page` — the default 10px one eats 5%
    // of a 210px column's width when (as with every panel open) the full
    // stack overflows.
    let content = iced::widget::scrollable(col)
        .direction(iced::widget::scrollable::Direction::Vertical(
            theme::thin_scrollbar(),
        ))
        .style(theme::scrollable)
        .width(Length::Fill)
        .height(Length::Fill);

    container(row![resize_strip(scale), content])
        .width(Length::Fixed(state.sidebar_width * scale))
        .height(Length::Fill)
        .style(theme::top_bar)
        .into()
}

/// The resize grip on the column's inner edge — a `splitter` that reports
/// horizontal drag deltas as `Message::SidebarResize`. Only ever reached
/// with the column open (the collapsed rail has a fixed width), and it
/// needs no overlay of its own: it sits in the column's own left padding.
fn resize_strip(scale: f32) -> Element<'static, Message> {
    crate::widgets::splitter::splitter(Message::SidebarResize, scale, RESIZE_W * scale)
}

/// The fold tab as an *overlay*, for `app.rs::view`'s own `stack`.
///
/// It used to be a widget inside the row, with a strip of its own to sit
/// in — which left a full-height column of empty width next to the mixer.
/// A row's children never draw outside their box, so the only way for the
/// tab to hang over the boundary it folds is to be layered above the
/// whole body row (see `TAB_INSIDE` for the split).
///
/// Deliberately nothing but `Space`s and the one button: no `opaque`, no
/// `mouse_area`, like the mixer's own `page`. Everything else in this
/// overlay has to stay transparent to clicks and wheel events so they
/// keep reaching the body row underneath (the page's own
/// click-to-clear-selection, the sidebar's scrollable).
///
/// Collapsed, the rail (`COLLAPSED_WIDTH`) *is* the tab, so there is
/// nothing to overlay and this contributes nothing.
pub fn fold_tab(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    if !state.sidebar_open {
        return iced::widget::Space::new().into();
    }

    row![
        iced::widget::Space::new().width(Length::Fill),
        tab_button(true, scale),
        iced::widget::Space::new().width(Length::Fixed((state.sidebar_width - TAB_INSIDE) * scale)),
    ]
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Gives one panel its own top margin on top of the outer column's
/// spacing, so two panels sit further apart than two rows inside one of
/// them (`SPACE_TIGHT`) — the sidebar's only structural cue that doesn't
/// rely on color.
fn panel<'a>(content: Element<'a, Message>, scale: f32) -> Element<'a, Message> {
    container(content)
        .padding(iced::Padding {
            top: theme::SPACE_MD * scale,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        })
        .width(Length::Fill)
        .into()
}

/// The device chip's own state word and color, in priority order: a
/// disconnected device outranks the personality it had (the green/amber
/// words describe a card that is actually there), and the mock's
/// "Simulated" would otherwise mask a mock-side disconnect. Pure, so the
/// mapping is pinned by a test (`app`'s own, where states are easy to
/// build).
pub(crate) fn device_state(state: &TuxMix) -> (Color, &'static str) {
    if !state.connected {
        (theme::MRED, "Disconnected")
    } else if state.device.is_mock() {
        (theme::YSIM, "Simulated")
    } else if !state.device.protocol_features() {
        (theme::YSIM, "Class Compliant")
    } else {
        (theme::GCONN, "Proprietary")
    }
}

/// Absorbs what used to be the top bar's own device chip (status dot +
/// "Simulated"/"Connected" label, see `top_bar`) now that this one is
/// the sidebar's device chip and the top bar's copy was removed as a
/// straight duplicate — the status info still needs to live *somewhere*.
///
/// The label names the card's own personality, not just a connection
/// flag: "Proprietary" (the kernel driver speaks RME's protocol and
/// everything in the app works), "Class Compliant" (routing, preamps and
/// clock only; the strips hide the rest), "Simulated" (no hardware). The
/// two non-default states were already named while the default said only
/// "Connected", which left the one users see every day as the one that
/// explained the least.
///
/// Full width like every other row in this column, so it tracks the
/// sidebar as it is dragged, with the identity on one line and the state
/// under it: one line for both wraps "Class Compliant", the longest word,
/// against the default column width (seen live), and a clipped or
/// wrapping state word is worse than a second line. The `▾` that used to
/// sit at the right edge suggested a dropdown that never existed —
/// nothing behind it was clickable — and the row-per-line stack around it
/// left most of the card empty.
fn device_chip(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    // What the connection actually is right now (see `device_state`);
    // the strips hide the controls that only exist on some of those
    // states, so this line is where the user finds out why they are
    // missing, and where a card that vanished shows up as gone.
    let (status_color, status_label) = device_state(state);
    // `model_name()` grows a " (mock)" suffix in mock mode (see
    // `mock.rs`) — that's needed for scene-compatibility checks, but
    // redundant here since the state right next to it already says
    // "Simulated", and the suffix alone was enough to push
    // "Babyface Pro FS (mock)" past this chip's text budget in a
    // 210px-wide sidebar. Strip it for display only.
    let display_name = state
        .device
        .model_name()
        .strip_suffix(" (mock)")
        .unwrap_or(state.device.model_name());
    container(
        column![
            row![
                text("●").size(theme::TEXT_XS * scale).color(status_color),
                text(display_name)
                    .size(theme::TEXT_MD * scale)
                    .color(theme::TEXT_PRIMARY),
            ]
            .spacing(theme::SPACE_SM * scale)
            .align_y(iced::Alignment::Center),
            text(status_label)
                .size(theme::TEXT_XS * scale)
                .color(status_color),
        ]
        .spacing(theme::SPACE_HAIRLINE * scale),
    )
    .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
    .width(Length::Fill)
    .style(theme::chip)
    .into()
}

/// Real: pops/pushes `undo_stack`/`redo_stack` (see `Message::Undo`'s
/// own doc comment for the capture granularity). Disabled (no
/// `on_press`) when the relevant stack is empty, via `on_press_maybe`.
/// Flat, and the only two rows here with nothing else to say: an empty
/// stack now reads as "nothing to do" instead of as two disabled boxes.
fn undo_redo_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    row![
        button(text("‹ undo").size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::ghost_button)
            .on_press_maybe((!state.undo_stack.is_empty()).then_some(Message::Undo)),
        button(text("redo ›").size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::ghost_button)
            .on_press_maybe((!state.redo_stack.is_empty()).then_some(Message::Redo)),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .width(Length::Fill)
    .into()
}

/// `M` and `S` are real, both true toggles: `M` toggles every
/// channel's mute at once (lit when all channels are already muted),
/// `S` clears every active solo and remembers which channels those
/// were, restoring them on the next press if nothing's been soloed
/// again since (lit when at least one channel is currently soloed) —
/// see `Message::GlobalMuteToggle`/`GlobalSoloToggle` in `app.rs`.
///
/// These two keep the full-strength fill (`theme::toggle_button`): they
/// are the one pair in this sidebar that reports a real state, which is
/// exactly what that look is now reserved for. `F` stays skeleton (no
/// `on_press`) — nothing in this app corresponds to TotalMix's own third
/// button here, and a static screenshot can't confirm what it should do —
/// and being unpressable is all it needs: `toggle_button` renders a
/// `Disabled` button with the same dimmed label the other inert controls
/// get (`disabled_segmented_row`), so it stops looking like a live toggle
/// that never lights up.
fn msf_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let muted_all = crate::app::all_channels_muted(state);
    let any_solo = crate::app::any_channel_soloed(state);
    row![
        button(text("M").size(theme::TEXT_SM * scale).center())
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(muted_all, theme::MUTE_COLOR))
            .on_press(Message::GlobalMuteToggle),
        button(text("S").size(theme::TEXT_SM * scale).center())
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(any_solo, theme::SOLO_COLOR))
            .on_press(Message::GlobalSoloToggle),
        button(text("F").size(theme::TEXT_SM * scale).center())
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(false, theme::ACCENT)),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .width(Length::Fill)
    .into()
}

/// Not applicable — no FX/reverb/echo processing exists in this app at
/// all (unlike TotalMix FX, which this row reports the send level of).
/// Static, no interaction, and flat: it's a readout, not a control, so it
/// must not wear the same box as the switches below it.
fn fx_show_row(scale: f32) -> Element<'static, Message> {
    row![
        text("FX show")
            .size(theme::TEXT_XS * scale)
            .color(theme::TEXT_SEC),
        iced::widget::Space::new().width(Length::Fill),
        text("0%")
            .size(theme::TEXT_XS * scale)
            .color(theme::TEXT_SEC),
    ]
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

/// One panel's header — the sidebar's only structure. The same
/// vocabulary as the mixer's `section_header` (accent tick, label, rule
/// trailing right) at sidebar scale, with the reference's own "−"/"+"
/// collapse toggle at the end. This replaced a boxed chip: four
/// identical bordered headers stacked in one column gave the panels
/// exactly the weight of the controls inside them, so neither read as
/// either.
fn panel_header(label: &str, open: bool, on_toggle: Message, scale: f32) -> Element<'_, Message> {
    row![
        container(
            iced::widget::Space::new()
                .width(3.0 * scale)
                .height(12.0 * scale)
        )
        .style(theme::accent_bar),
        text(label.to_uppercase())
            .size(theme::TEXT_SM * scale)
            .color(theme::TEXT_PRIMARY),
        iced::widget::rule::horizontal(1),
        button(text(if open { "−" } else { "+" }).size(theme::TEXT_XS * scale))
            .padding(theme::SPACE_SM * scale)
            .style(theme::ghost_button)
            .on_press(on_toggle),
    ]
    .spacing(theme::SPACE_SM * scale)
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

/// The caption cell of an Options row — dimmer than the control labels it
/// captions and fixed-width, so the four rows' controls line up in a
/// column of their own instead of every caption eating a full line of the
/// panel's width above its row.
fn sub_label(label: &str, scale: f32) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_XS * scale)
        .color(theme::TEXT_SEC)
        .width(Length::Fixed(OPTIONS_LABEL_W * scale))
        .into()
}

/// One Options row: caption on the left, control filling the rest.
/// `None` for the row that continues the caption above it (the `2 row` /
/// `2 row in` half of the `show` pair): the empty cell keeps its control
/// aligned with the row above rather than sliding left under the caption.
fn options_row<'a>(
    caption: Option<&'a str>,
    control: Element<'a, Message>,
    scale: f32,
) -> Element<'a, Message> {
    let caption: Element<'a, Message> = match caption {
        Some(label) => sub_label(label, scale),
        None => iced::widget::Space::new()
            .width(Length::Fixed(OPTIONS_LABEL_W * scale))
            .into(),
    };
    row![caption, control]
        .spacing(theme::SPACE_SM * scale)
        .align_y(iced::Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// A 2-way segmented row for the *skeleton* toggle pairs (`show`/
/// `2 row`/`solo mode`) and the permanently-fixed `routing` row — one
/// side highlighted, the other not, `on_press` wired on both (skeleton)
/// or neither (fixed `routing`: "submix" is always the highlighted
/// side, so there's nothing to press toward). For a row where *neither*
/// side should ever look engaged (the `meters` row — see
/// `disabled_segmented_row`), use that instead: passing `left_active:
/// false` here still lights the *right* side, which is correct for an
/// actual exclusive pair but wrong for "both are simply off."
fn segmented_row<'a>(
    left_label: &'a str,
    right_label: &'a str,
    left_active: bool,
    on_left: Option<Message>,
    on_right: Option<Message>,
    scale: f32,
) -> Element<'a, Message> {
    segment_track(
        button(text(left_label).size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::segment_cell(left_active))
            .on_press_maybe(on_left)
            .into(),
        button(text(right_label).size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::segment_cell(!left_active))
            .on_press_maybe(on_right)
            .into(),
        scale,
    )
}

/// Wraps two switch cells in the shared `theme::segment_track`.
fn segment_track<'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
    scale: f32,
) -> Element<'a, Message> {
    container(row![left, right].spacing(theme::SPACE_HAIRLINE * scale))
        .padding(theme::SPACE_TIGHT * scale)
        .width(Length::Fill)
        .style(theme::segment_track)
        .into()
}

/// A 2-way segmented row where *neither* side is real — both rendered
/// dimmed (40% text alpha, same treatment as the Output strip's inert
/// balance knob, `Knob::interactive: false`) on an unlit switch body, and
/// unpressable, for Options rows that don't map to anything in this app's
/// architecture (`meters: post fx / RMS` — no on-device meter register, no
/// RMS ballistics, no post-FX tap point since no FX exists at all).
fn disabled_segmented_row(
    left_label: &str,
    right_label: &str,
    scale: f32,
) -> Element<'static, Message> {
    let dim = Color {
        a: 0.4,
        ..theme::TEXT_SEC
    };
    let cell = |label: String| -> Element<'static, Message> {
        button(text(label).size(theme::TEXT_XS * scale).color(dim).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::segment_cell(false))
            .into()
    };
    segment_track(
        cell(left_label.to_string()),
        cell(right_label.to_string()),
        scale,
    )
}

/// Numbered slots (Snapshots, Mixer Layouts) as a two-column grid of
/// equal cells, like the slot buttons on a hardware console, instead of a
/// list of full-width rows: eight rows of "Mix n" were the tallest thing
/// in the column for the least information. The selected slot is a switch
/// position, not an "on" state, so it takes `segment_button`'s graphite
/// thumb, same as every other position in this column.
fn slot_grid<'a>(
    count: u8,
    prefix: &'a str,
    active: Option<usize>,
    on_press: fn(u8) -> Message,
    scale: f32,
) -> Element<'a, Message> {
    let mut grid = column![].spacing(theme::SPACE_TIGHT * scale);
    for first in (1..=count).step_by(2) {
        let mut line = row![].spacing(theme::SPACE_TIGHT * scale);
        for n in first..=(first + 1).min(count) {
            line = line.push(
                button(
                    text(format!("{prefix} {n}"))
                        .size(theme::TEXT_XS * scale)
                        .center(),
                )
                .padding([theme::SPACE_SM * scale, theme::SPACE_SM * scale])
                .width(Length::Fill)
                .style(theme::segment_button(active == Some(n as usize)))
                .on_press(on_press(n)),
            );
        }
        grid = grid.push(line);
    }
    grid.width(Length::Fill).into()
}

/// The trailing "store" action of the Snapshot and Mixer Layout panels —
/// right-aligned, flat text, the same weight as the panel's own collapse
/// toggle. It used to be a bordered box the size of a slot, which made
/// "save into the selected one" look like an eighth snapshot.
fn store_row(on_press: Message, scale: f32) -> Element<'static, Message> {
    container(
        button(text("store").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .style(theme::ghost_button)
            .on_press(on_press),
    )
    .width(Length::Fill)
    .align_x(iced::Alignment::End)
    .into()
}

/// Real where the app has something to report (`routing`/`meters` are
/// permanently-fixed readouts of this app's own architecture, not
/// switches), skeleton everywhere the reference's exact behavior isn't
/// confirmed (`show`/`2 row`/`solo mode`) — see `SkeletonPair`'s own
/// doc comment.
fn options_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let header = panel_header(
        "options",
        state.sidebar_panels_open.options,
        Message::ToggleSidebarPanel(Panel::Options),
        scale,
    );

    if !state.sidebar_panels_open.options {
        return header;
    }

    let mut body = column![]
        .spacing(theme::SPACE_SM * scale)
        .padding(indent(CONTENT_INDENT * scale))
        .width(Length::Fill);

    // Permanently fixed — this app only ever has one routing model
    // (see `RmeDevice`'s own "the device exposes a matrix/submix
    // mixer" framing), there's no "free" mode to switch into.
    body = body.push(options_row(
        Some("routing"),
        segmented_row("submix", "free", true, None, None, scale),
        scale,
    ));

    // Permanently fixed, both disabled. See `GUI-NOTES.md`'s
    // "VU meters: conclusion".
    body = body.push(options_row(
        Some("meters"),
        disabled_segmented_row("post fx", "RMS", scale),
        scale,
    ));

    body = body.push(options_row(
        Some("show"),
        segmented_row(
            "names",
            "trim",
            state.skeleton_pairs.show_first,
            Some(Message::ToggleSkeletonPair(SkeletonPair::Show)),
            Some(Message::ToggleSkeletonPair(SkeletonPair::Show)),
            scale,
        ),
        scale,
    ));
    body = body.push(options_row(
        None,
        segmented_row(
            "2 row",
            "2 row in",
            state.skeleton_pairs.row_mode_first,
            Some(Message::ToggleSkeletonPair(SkeletonPair::RowMode)),
            Some(Message::ToggleSkeletonPair(SkeletonPair::RowMode)),
            scale,
        ),
        scale,
    ));

    body = body.push(options_row(
        Some("solo/pfl mode"),
        segmented_row(
            "excl. solo",
            "live",
            state.skeleton_pairs.solo_mode_first,
            Some(Message::ToggleSkeletonPair(SkeletonPair::SoloMode)),
            Some(Message::ToggleSkeletonPair(SkeletonPair::SoloMode)),
            scale,
        ),
        scale,
    ));

    column![header, body]
        .spacing(theme::SPACE_SM * scale)
        .width(Length::Fill)
        .into()
}

const SNAPSHOT_COUNT: u8 = 8;

/// Real — "Mix 1".."Mix 8" through the existing scene save/load
/// machinery (`scenes.rs`, `RmeDevice::capture_scene`/`apply_scene`),
/// mirroring `Message::SceneSave`/`SceneLoad` almost verbatim (see
/// `app.rs`'s own handlers).
///
/// Slots are a two-column grid (`slot_grid`), the selected one in the
/// same graphite thumb as every other switch position in this column.
fn snapshots_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let header = panel_header(
        "snapshots",
        state.sidebar_panels_open.snapshots,
        Message::ToggleSidebarPanel(Panel::Snapshots),
        scale,
    );

    if !state.sidebar_panels_open.snapshots {
        return header;
    }

    let mut body = column![]
        .spacing(theme::SPACE_TIGHT * scale)
        .padding(indent(CONTENT_INDENT * scale))
        .width(Length::Fill);

    body = body.push(slot_grid(
        SNAPSHOT_COUNT,
        "Mix",
        state.active_snapshot,
        Message::SnapshotClicked,
        scale,
    ));
    body = body.push(store_row(Message::SnapshotStore, scale));

    column![header, body]
        .spacing(theme::SPACE_SM * scale)
        .width(Length::Fill)
        .into()
}

/// Real — 4 groups, membership from the existing multi-select
/// (`state.selected`), independent mute/solo/fader link toggles per
/// group. See `Group`'s own doc comment for the membership model and
/// `app.rs`'s propagation logic for how a linked group actually
/// affects Mute/Solo/Volume.
///
/// The three link cells keep the full-strength fill: a linked group is
/// the one thing in this panel that changes what the mixer does, so it
/// stays the loudest state the sidebar has.
fn groups_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let header = panel_header(
        "groups",
        state.sidebar_panels_open.groups,
        Message::ToggleSidebarPanel(Panel::Groups),
        scale,
    );

    if !state.sidebar_panels_open.groups {
        return header;
    }

    let mut body = column![]
        .spacing(theme::SPACE_TIGHT * scale)
        .padding(indent(CONTENT_INDENT * scale))
        .width(Length::Fill);

    // Column captions at `TEXT_MICRO`, a tier below the `M1`/`S1`/`1`
    // labels they sit over, so the cells read as the panel's content and
    // these as their headers.
    body = body.push(
        row![
            text("mute")
                .size(theme::TEXT_MICRO * scale)
                .color(theme::TEXT_SEC)
                .width(Length::Fill)
                .center(),
            text("solo")
                .size(theme::TEXT_MICRO * scale)
                .color(theme::TEXT_SEC)
                .width(Length::Fill)
                .center(),
            text("fader")
                .size(theme::TEXT_MICRO * scale)
                .color(theme::TEXT_SEC)
                .width(Length::Fill)
                .center(),
        ]
        .spacing(theme::SPACE_TIGHT * scale),
    );

    for (i, g) in state.groups.iter().enumerate() {
        // Cells reflect the group's real link state regardless of edit
        // mode — only the "edit" button itself (below) shows whether a
        // click here will assign a fresh membership or just flip a link.
        let cell = |lit: bool, label: String, link: GroupLink| -> Element<'_, Message> {
            button(text(label).size(theme::TEXT_XS * scale).center())
                .padding(theme::SPACE_TIGHT * scale)
                .width(Length::Fill)
                .style(theme::toggle_button(lit, theme::ACCENT))
                .on_press(Message::GroupLinkToggle(i, link))
                .into()
        };
        body = body.push(
            row![
                cell(g.mute_linked, format!("M{}", i + 1), GroupLink::Mute),
                cell(g.solo_linked, format!("S{}", i + 1), GroupLink::Solo),
                cell(g.fader_linked, format!("{}", i + 1), GroupLink::Fader),
            ]
            .spacing(theme::SPACE_TIGHT * scale),
        );
    }

    // Edit mode is a switch position, not a lit channel: it says where a
    // click on the cells above will land, so it gets the dim pill. "clear"
    // is the pair's destructive half and stays flat — as a `plain_button`
    // it was the lightest-colored box in the whole column, which read as
    // the most important thing in the panel.
    body = body.push(
        row![
            button(text("edit").size(theme::TEXT_XS * scale).center())
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::segment_button(state.group_editing))
                .on_press(Message::GroupEditToggle),
            button(text("clear").size(theme::TEXT_XS * scale).center())
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::ghost_button)
                .on_press(Message::GroupClear),
        ]
        .spacing(theme::SPACE_TIGHT * scale),
    );

    column![header, body]
        .spacing(theme::SPACE_SM * scale)
        .width(Length::Fill)
        .into()
}

const LAYOUT_COUNT: u8 = 6;

/// Real — a layout is `state.collapsed` (which strips are collapsed;
/// there's no strip-reordering yet, so that's the whole story),
/// persisted to disk per slot (see `layouts.rs`, mirroring `scenes.rs`'s
/// own file-per-slot pattern). "all"/"submix" scope is skeleton — see
/// `SkeletonPair::LayoutScope`'s own doc comment.
fn layout_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let header = panel_header(
        "mixer layout",
        state.sidebar_panels_open.layout,
        Message::ToggleSidebarPanel(Panel::Layout),
        scale,
    );

    if !state.sidebar_panels_open.layout {
        return header;
    }

    let mut body = column![]
        .spacing(theme::SPACE_TIGHT * scale)
        .padding(indent(CONTENT_INDENT * scale))
        .width(Length::Fill);

    body = body.push(segmented_row(
        "all",
        "submix",
        state.skeleton_pairs.layout_scope_first,
        Some(Message::ToggleSkeletonPair(SkeletonPair::LayoutScope)),
        Some(Message::ToggleSkeletonPair(SkeletonPair::LayoutScope)),
        scale,
    ));

    body = body.push(slot_grid(
        LAYOUT_COUNT,
        "Layout",
        state.active_layout,
        Message::LayoutClicked,
        scale,
    ));
    body = body.push(store_row(Message::LayoutStore, scale));

    column![header, body]
        .spacing(theme::SPACE_SM * scale)
        .width(Length::Fill)
        .into()
}
