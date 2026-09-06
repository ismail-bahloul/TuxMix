//! The right-hand "control strip" sidebar — TotalMix's own device chip,
//! Undo/Redo, Snapshots, Groups, and Mixer Layout panels, plus a
//! visual-only M/S/F row and Options panel.
//!
//! Not everything here is wired to real behavior — see each panel's own
//! doc comment. Snapshots/Undo-Redo/Groups/Mixer-Layout are real;
//! the M/S/F row, "FX show %", and several Options rows are deliberate
//! visual skeleton (per the user's own framing: build the shape, don't
//! guess at behavior a static reference screenshot can't confirm).

use iced::widget::{button, column, container, row, text};
use iced::{Color, Element, Length};

use tuxmix_core::{ChannelId, RmeDevice};

use crate::app::{Message, TuxMix};
use crate::theme;

/// Fixed sidebar width (at `scale == 1.0`) — wide enough for the longest
/// label in the Options panel ("excl. solo") without wrapping.
pub const WIDTH: f32 = 210.0;
/// Width of the collapsed rail — just enough for the expand button, so
/// collapsing the sidebar reclaims most of its width for the mixer
/// without losing the one control needed to bring it back (an
/// entirely-gone sidebar would have nowhere left to click).
pub const COLLAPSED_WIDTH: f32 = 22.0;

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
        Self { options: true, snapshots: true, groups: true, layout: true }
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

/// Top-level entry point — one scrollable column, fixed width, filling
/// the window's full height alongside the main content (see
/// `app.rs::view`). Collapsible: `state.sidebar_open == false` renders
/// just `COLLAPSED_WIDTH` of rail with an expand button, reclaiming the
/// rest of the width for the mixer — the whole point being asked for
/// ("rabattable"), not just a visual nicety.
pub fn sidebar(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    if !state.sidebar_open {
        return container(
            button(text("‹").size(theme::TEXT_SM * scale))
                .padding(theme::SPACE_TIGHT * scale)
                .style(theme::plain_button)
                .on_press(Message::ToggleSidebar),
        )
        .padding(theme::SPACE_SM * scale)
        .width(Length::Fixed(COLLAPSED_WIDTH * scale))
        .height(Length::Fill)
        .style(theme::top_bar)
        .into();
    }

    let collapse_btn = button(text("›").size(theme::TEXT_SM * scale))
        .padding(theme::SPACE_TIGHT * scale)
        .style(theme::plain_button)
        .on_press(Message::ToggleSidebar);

    let col = column![
        collapse_btn,
        device_chip(state, scale),
        undo_redo_row(state, scale),
        msf_row(state, scale),
        fx_show_chip(scale),
        options_panel(state, scale),
        snapshots_panel(state, scale),
        groups_panel(state, scale),
        layout_panel(state, scale),
    ]
    .spacing(theme::SPACE_MD * scale)
    .padding(theme::SPACE_MD * scale)
    .width(Length::Fixed(WIDTH * scale));

    container(iced::widget::scrollable(col).style(theme::scrollable))
        .width(Length::Fixed(WIDTH * scale))
        .height(Length::Fill)
        .style(theme::top_bar)
        .into()
}

/// Absorbs what used to be the top bar's own device chip (status dot +
/// "Simulated"/"Connected" label, see `top_bar`) now that this one is
/// the sidebar's device chip and the top bar's copy was removed as a
/// straight duplicate — the status info still needs to live *somewhere*.
fn device_chip(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let status_color = if state.device.is_mock() { theme::YSIM } else { theme::GCONN };
    let status_label = if state.device.is_mock() { "Simulated" } else { "Connected" };
    // `model_name()` grows a " (mock)" suffix in mock mode (see
    // `mock.rs`) — that's needed for scene-compatibility checks, but
    // redundant here since the status line right below already says
    // "Simulated", and the suffix alone was enough to push
    // "Babyface Pro FS (mock)" past this chip's ~150px text budget
    // in a 210px-wide sidebar. Strip it for display only.
    let display_name =
        state.device.model_name().strip_suffix(" (mock)").unwrap_or(state.device.model_name());
    container(
        column![
            row![
                text("●").size(theme::TEXT_XS * scale).color(status_color),
                text(display_name)
                    .size(theme::TEXT_MD * scale)
                    .color(theme::TEXT_PRIMARY)
                    .width(Length::Fill),
                text("▾").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC),
            ]
            .spacing(theme::SPACE_SM * scale)
            .align_y(iced::Alignment::Center),
            text(status_label).size(theme::TEXT_XS * scale).color(status_color),
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
fn undo_redo_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    row![
        button(text("‹ undo").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::plain_button)
            .on_press_maybe((!state.undo_stack.is_empty()).then_some(Message::Undo)),
        button(text("redo ›").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
            .width(Length::Fill)
            .style(theme::plain_button)
            .on_press_maybe((!state.redo_stack.is_empty()).then_some(Message::Redo)),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .into()
}

/// `M` and `S` are real, both true toggles: `M` toggles every
/// channel's mute at once (lit when all channels are already muted),
/// `S` clears every active solo and remembers which channels those
/// were, restoring them on the next press if nothing's been soloed
/// again since (lit when at least one channel is currently soloed) —
/// see `Message::GlobalMuteToggle`/`GlobalSoloToggle` in `app.rs`. `F`
/// stays skeleton (no `on_press`) — nothing in this app corresponds to
/// TotalMix's own third button here, and a static screenshot can't
/// confirm what it should do.
fn msf_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let muted_all = crate::app::all_channels_muted(state);
    let any_solo = crate::app::any_channel_soloed(state);
    row![
        button(text("M").size(theme::TEXT_SM * scale))
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(muted_all, theme::MUTE_COLOR))
            .on_press(Message::GlobalMuteToggle),
        button(text("S").size(theme::TEXT_SM * scale))
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(any_solo, theme::SOLO_COLOR))
            .on_press(Message::GlobalSoloToggle),
        button(text("F").size(theme::TEXT_SM * scale))
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
            .style(theme::toggle_button(false, theme::ACCENT)),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .into()
}

/// Not applicable — no FX/reverb/echo processing exists in this app at
/// all (unlike TotalMix FX, which this chip reports the send level of).
/// Static, no interaction.
fn fx_show_chip(scale: f32) -> Element<'static, Message> {
    container(
        row![
            text("FX").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC),
            text("show").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC),
            iced::widget::Space::new().width(Length::Fill),
            text("0%").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC),
        ]
        .spacing(theme::SPACE_SM * scale)
        .align_y(iced::Alignment::Center),
    )
    .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
    .width(Length::Fill)
    .style(theme::chip)
    .into()
}

/// A panel header shared by all 4 collapsible panels — label + a
/// "−"/"+" collapse toggle, matching the reference's own per-panel
/// header bar (simplified to one shared color rather than the
/// reference's 4 different per-panel background hues, consistent with
/// this session's own restrained orange/cyan palette rather than
/// introducing 4 new one-off colors for a single header row each).
fn panel_header(label: &str, open: bool, on_toggle: Message, scale: f32) -> Element<'_, Message> {
    container(
        row![
            text(label.to_uppercase())
                .size(theme::TEXT_XS * scale)
                .color(theme::TEXT_PRIMARY),
            iced::widget::Space::new().width(Length::Fill),
            button(text(if open { "−" } else { "+" }).size(theme::TEXT_XS * scale))
                .padding(0)
                .style(theme::plain_button)
                .on_press(on_toggle),
        ]
        .align_y(iced::Alignment::Center),
    )
    .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
    .width(Length::Fill)
    .style(theme::chip)
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
    row![
        button(text(left_label).size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(left_active, theme::ACCENT))
            .on_press_maybe(on_left),
        button(text(right_label).size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::toggle_button(!left_active, theme::ACCENT))
            .on_press_maybe(on_right),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .into()
}

/// A 2-way segmented row where *neither* side is real — both rendered
/// dimmed (40% text alpha, same treatment as the Output strip's inert
/// balance knob, `Knob::interactive: false`) and unpressable, for
/// Options rows that don't map to anything in this app's architecture
/// (`meters: post fx / RMS` — no on-device meter register, no RMS
/// ballistics, no post-FX tap point since no FX exists at all).
fn disabled_segmented_row(left_label: &str, right_label: &str, scale: f32) -> Element<'static, Message> {
    let dim = Color { a: 0.4, ..theme::TEXT_SEC };
    let cell = |label: String| -> Element<'static, Message> {
        button(text(label).size(theme::TEXT_XS * scale).color(dim))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fill)
            .style(theme::plain_button)
            .into()
    };
    row![cell(left_label.to_string()), cell(right_label.to_string())]
        .spacing(theme::SPACE_TIGHT * scale)
        .into()
}

/// Real where the app has something to report (`routing`/`meters` are
/// permanently-fixed readouts of this app's own architecture, not
/// switches), skeleton everywhere the reference's exact behavior isn't
/// confirmed (`show`/`2 row`/`solo mode`) — see `SkeletonPair`'s own
/// doc comment.
fn options_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let mut col = column![panel_header(
        "options",
        state.sidebar_panels_open.options,
        Message::ToggleSidebarPanel(Panel::Options),
        scale
    )]
    .spacing(theme::SPACE_SM * scale);

    if !state.sidebar_panels_open.options {
        return col.into();
    }

    let label = |s: &'static str| -> Element<'_, Message> {
        text(s).size(theme::TEXT_XS * scale).color(theme::TEXT_SEC).into()
    };

    col = col.push(label("routing"));
    // Permanently fixed — this app only ever has one routing model
    // (see `RmeDevice`'s own "the device exposes a matrix/submix
    // mixer" framing), there's no "free" mode to switch into.
    col = col.push(segmented_row("submix", "free", true, None, None, scale));

    col = col.push(label("meters"));
    // Permanently fixed, both disabled — no on-device meter register,
    // no RMS ballistics, no post-FX tap point (no FX exists at all).
    // See `GUI-NOTES.md`'s "VU meters: conclusion".
    col = col.push(disabled_segmented_row("post fx", "RMS", scale));

    col = col.push(label("show"));
    col = col.push(segmented_row(
        "names",
        "trim",
        state.skeleton_pairs.show_first,
        Some(Message::ToggleSkeletonPair(SkeletonPair::Show)),
        Some(Message::ToggleSkeletonPair(SkeletonPair::Show)),
        scale,
    ));

    col = col.push(segmented_row(
        "2 row",
        "2 row in",
        state.skeleton_pairs.row_mode_first,
        Some(Message::ToggleSkeletonPair(SkeletonPair::RowMode)),
        Some(Message::ToggleSkeletonPair(SkeletonPair::RowMode)),
        scale,
    ));

    col = col.push(label("solo/pfl mode"));
    col = col.push(segmented_row(
        "excl. solo",
        "live",
        state.skeleton_pairs.solo_mode_first,
        Some(Message::ToggleSkeletonPair(SkeletonPair::SoloMode)),
        Some(Message::ToggleSkeletonPair(SkeletonPair::SoloMode)),
        scale,
    ));

    col.into()
}

const SNAPSHOT_COUNT: u8 = 8;

/// Real — "Mix 1".."Mix 8" through the existing scene save/load
/// machinery (`scenes.rs`, `RmeDevice::capture_scene`/`apply_scene`),
/// mirroring `Message::SceneSave`/`SceneLoad` almost verbatim (see
/// `app.rs`'s own handlers).
fn snapshots_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let mut col = column![panel_header(
        "snapshots",
        state.sidebar_panels_open.snapshots,
        Message::ToggleSidebarPanel(Panel::Snapshots),
        scale
    )]
    .spacing(theme::SPACE_HAIRLINE * scale);

    if !state.sidebar_panels_open.snapshots {
        return col.into();
    }

    for n in 1..=SNAPSHOT_COUNT {
        let active = state.active_snapshot == Some(n as usize);
        col = col.push(
            button(text(format!("Mix {n}")).size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(active, theme::ACCENT))
                .on_press(Message::SnapshotClicked(n)),
        );
    }
    col = col.push(
        container(
            button(text("store").size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .style(theme::plain_button)
                .on_press(Message::SnapshotStore),
        )
        .width(Length::Fill)
        .align_x(iced::Alignment::End),
    );

    col.into()
}

/// Real — 4 groups, membership from the existing multi-select
/// (`state.selected`), independent mute/solo/fader link toggles per
/// group. See `Group`'s own doc comment for the membership model and
/// `app.rs`'s propagation logic for how a linked group actually
/// affects Mute/Solo/Volume.
fn groups_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let mut col = column![panel_header(
        "groups",
        state.sidebar_panels_open.groups,
        Message::ToggleSidebarPanel(Panel::Groups),
        scale
    )]
    .spacing(theme::SPACE_HAIRLINE * scale);

    if !state.sidebar_panels_open.groups {
        return col.into();
    }

    col = col.push(
        row![
            text("mute").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC).width(Length::Fill),
            text("solo").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC).width(Length::Fill),
            text("fader").size(theme::TEXT_XS * scale).color(theme::TEXT_SEC).width(Length::Fill),
        ]
        .spacing(theme::SPACE_TIGHT * scale),
    );

    for (i, g) in state.groups.iter().enumerate() {
        // Cells reflect the group's real link state regardless of edit
        // mode — only the "edit" button itself (below) shows whether a
        // click here will assign a fresh membership or just flip a link.
        let cell = |lit: bool, label: String, link: GroupLink| -> Element<'_, Message> {
            button(text(label).size(theme::TEXT_XS * scale))
                .padding(theme::SPACE_TIGHT * scale)
                .width(Length::Fill)
                .style(theme::toggle_button(lit, theme::ACCENT))
                .on_press(Message::GroupLinkToggle(i, link))
                .into()
        };
        col = col.push(
            row![
                cell(g.mute_linked, format!("M{}", i + 1), GroupLink::Mute),
                cell(g.solo_linked, format!("S{}", i + 1), GroupLink::Solo),
                cell(g.fader_linked, format!("{}", i + 1), GroupLink::Fader),
            ]
            .spacing(theme::SPACE_TIGHT * scale),
        );
    }

    col = col.push(
        row![
            button(text("edit").size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(state.group_editing, theme::ACCENT))
                .on_press(Message::GroupEditToggle),
            button(text("clear").size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::plain_button)
                .on_press(Message::GroupClear),
        ]
        .spacing(theme::SPACE_TIGHT * scale),
    );

    col.into()
}

const LAYOUT_COUNT: u8 = 6;

/// Real — a layout is `state.collapsed` (which strips are collapsed;
/// there's no strip-reordering yet, so that's the whole story),
/// persisted to disk per slot (see `layouts.rs`, mirroring `scenes.rs`'s
/// own file-per-slot pattern). "all"/"submix" scope is skeleton — see
/// `SkeletonPair::LayoutScope`'s own doc comment.
fn layout_panel(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let mut col = column![panel_header(
        "mixer layout",
        state.sidebar_panels_open.layout,
        Message::ToggleSidebarPanel(Panel::Layout),
        scale
    )]
    .spacing(theme::SPACE_HAIRLINE * scale);

    if !state.sidebar_panels_open.layout {
        return col.into();
    }

    col = col.push(segmented_row(
        "all",
        "submix",
        state.skeleton_pairs.layout_scope_first,
        Some(Message::ToggleSkeletonPair(SkeletonPair::LayoutScope)),
        Some(Message::ToggleSkeletonPair(SkeletonPair::LayoutScope)),
        scale,
    ));

    for n in 1..=LAYOUT_COUNT {
        let active = state.active_layout == Some(n as usize);
        col = col.push(
            button(text(format!("Layout {n}")).size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .width(Length::Fill)
                .style(theme::toggle_button(active, theme::ACCENT))
                .on_press(Message::LayoutClicked(n)),
        );
    }
    col = col.push(
        container(
            button(text("store").size(theme::TEXT_XS * scale))
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_MD * scale])
                .style(theme::plain_button)
                .on_press(Message::LayoutStore),
        )
        .width(Length::Fill)
        .align_x(iced::Alignment::End),
    );
    col.into()
}
