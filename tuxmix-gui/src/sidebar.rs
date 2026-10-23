//! The right-hand "control strip" sidebar — TotalMix's own device chip,
//! Undo/Redo, the global M/S/F row, Options, Snapshots, Groups and Mixer
//! Layout panels.
//!
//! Everything here is wired to real behavior. It used to carry visual
//! skeleton copied from a TotalMix screenshot before its semantics were
//! known ("FX show %", the `F` button, most Options rows, the layout
//! panel's "all/submix"); on 2026-10-20 each was checked against RME's
//! Control Strip documentation, this app and the kernel driver, and either
//! made real (`F`, "2 rows", `DIM`) or removed (see `GUI-NOTES.md`).
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

use iced::widget::{button, column, container, mouse_area, row, text};
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
/// Width of the collapsed rail: the panel toggle plus its padding, so
/// collapsing reclaims most of the width for the mixer while the button
/// that brings the column back stays exactly where it was.
pub const COLLAPSED_WIDTH: f32 = 34.0;
/// The panel toggle's hit box, at scale 1 — the same in both states, so
/// the button never moves when it is pressed.
const TOGGLE_W: f32 = 26.0;
const TOGGLE_H: f32 = 22.0;

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

/// Which sidebar panel a `Message::ToggleSidebarPanel` collapses/expands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Options,
    Snapshots,
    Groups,
    Layout,
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

/// Width of the drag strip that resizes this column, on its inner edge.
/// Sized to the column's own left padding (`SPACE_MD`), and the column's
/// left padding is in turn dropped in its favour (`sidebar`'s own
/// `scrollable`): the strip occupies exactly the space nothing was drawn
/// in, so resizing costs the layout no width and every row still starts
/// `SPACE_MD` from the column's left edge.
const RESIZE_W: f32 = theme::SPACE_MD;

/// The fold control, part of the panel itself: the "toggle sidebar"
/// icon (`widgets::panel_icon`) at the right end of the panel's title row
/// when open, and at the top of the rail when collapsed — the same spot
/// and size either way, the way editors and design tools do it. It
/// replaced a capsule tab that hung over the mixer/sidebar boundary as an
/// overlay: findable, but it belonged to neither side and read as a
/// scrollbar fragment.
fn panel_toggle(open: bool, scale: f32) -> Element<'static, Message> {
    let toggle = button(
        iced::widget::canvas(crate::widgets::panel_icon::PanelIcon {
            color: theme::TEXT_SEC,
            open,
            scale,
        })
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(0)
    .width(Length::Fixed(TOGGLE_W * scale))
    .height(Length::Fixed(TOGGLE_H * scale))
    .style(theme::ghost_button)
    .on_press(Message::ToggleSidebar);
    iced::widget::tooltip(
        toggle,
        container(
            text(if open { "Hide panel" } else { "Show panel" }).size(theme::TEXT_XS * scale),
        )
        .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
        .style(theme::chip),
        iced::widget::tooltip::Position::Left,
    )
    .into()
}

/// The panel's title row: its name on the left, the fold toggle on the
/// right — what makes the column read as one panel with a header instead
/// of a stack of rows that happens to sit at the window's edge.
///
/// Double-clicking anywhere empty on the panel folds it (see `sidebar`);
/// the toggle button captures its own presses.
fn title_row(scale: f32) -> Element<'static, Message> {
    // As tall as the top bar minus the column's padding above and the
    // spacing below it, so it sits on the bar's line and the device chip
    // starts exactly where the views do.
    row![
        text("CONTROLS")
            .size(theme::TEXT_XS * scale)
            .color(theme::TEXT_SEC),
        iced::widget::Space::new().width(Length::Fill),
        panel_toggle(true, scale),
    ]
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .height(Length::Fixed(
        (theme::TOP_BAR_H - 2.0 * theme::SPACE_MD) * scale,
    ))
    .into()
}

/// The 1px line on the panel's inner edge while collapsed. Open, the
/// resize strip draws the same line at rest (see `widgets::splitter`).
fn edge_line(scale: f32) -> Element<'static, Message> {
    container(
        iced::widget::Space::new()
            .width(Length::Fixed(scale.max(1.0)))
            .height(Length::Fill),
    )
    .style(|_theme: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(theme::BORDER)),
        ..container::Style::default()
    })
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
    // Capped (`app::sidebar_scale`): a column of small controls gains
    // nothing from the strips' full zoom.
    let scale = crate::app::sidebar_scale(state);

    // Collapsed, the rail is exactly the tab plus its own padding — the
    // one control that brings the column back *is* the whole rail, and
    // the tab is centered so it doesn't move height between states.
    if !state.sidebar_open {
        // The rail keeps the device's status dot under the toggle, so a
        // card that drops out is still visible with the panel folded.
        let (status_color, _) = device_state(state);
        let rail = column![
            panel_toggle(false, scale),
            text("●").size(theme::TEXT_XS * scale).color(status_color),
            // The device card is folded away with the panel; its settings
            // stay one click off.
            hint(
                button(text("\u{2699}").size(theme::TEXT_SM * scale).center())
                    .padding(0)
                    .width(Length::Fixed(TOGGLE_W * scale))
                    .height(Length::Fixed(TOGGLE_H * scale))
                    .style(theme::ghost_button)
                    .on_press(Message::ToggleDevicePanel),
                "Device settings",
                scale,
            ),
        ]
        .spacing(theme::SPACE_LG * scale)
        // The toggle centered on the top bar's line, where it sits when
        // the panel is open.
        .padding(iced::Padding {
            top: (theme::TOP_BAR_H - TOGGLE_H) / 2.0 * scale,
            ..iced::Padding::ZERO
        })
        .align_x(iced::Alignment::Center)
        .width(Length::Fill);
        // The whole rail is the way back: one click anywhere on it opens
        // the panel, not just on the 26px button at its top. The button
        // captures its own press, so a click on it is not counted twice.
        return mouse_area(
            container(row![edge_line(scale), rail])
                .width(Length::Fixed(COLLAPSED_WIDTH * scale))
                .height(Length::Fill)
                .style(theme::top_bar),
        )
        // Double-click, like the open panel: one gesture both ways. The
        // button at the top still opens it with a single click.
        .on_double_click(Message::ToggleSidebar)
        .into();
    }

    let col = column![
        title_row(scale),
        device_chip(state, scale),
        undo_redo_row(state, scale),
        msf_row(state, scale),
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

    // Double-click on any empty part of the column folds it. Controls
    // capture their own presses, so only the gaps, captions and the
    // title row reach this.
    mouse_area(
        container(
            row![
                resize_strip(scale),
                column![content, zoom_bar(state, scale)].width(Length::Fill)
            ]
            .height(Length::Fill),
        )
        .width(Length::Fixed(state.sidebar_width * scale))
        .height(Length::Fill)
        .style(theme::top_bar),
    )
    .on_double_click(Message::ToggleSidebar)
    .into()
}

/// The zoom control, fixed at the foot of the column: `-  100%  +`.
/// The zoom was Ctrl+wheel / Ctrl+= only, which nothing on screen
/// mentioned. The percentage resets to 100 %. (A fit-to-window mode lived
/// next to it for a day and was removed: see GUI-NOTES.md.)
fn zoom_bar(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let pct = format!("{:.0}%", state.ui_scale * 100.0);
    let cell = |label: &'static str| {
        button(text(label).size(theme::TEXT_SM * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fixed(26.0 * scale))
    };
    container(
        row![
            hint(
                cell("\u{2212}")
                    .style(theme::ghost_button)
                    .on_press(Message::ZoomOut),
                "Zoom out (Ctrl -)",
                scale,
            ),
            hint(
                button(text(pct).size(theme::TEXT_XS * scale).center())
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                    .width(Length::Fill)
                    .style(theme::ghost_button)
                    .on_press(Message::ZoomReset),
                "Reset to 100% (Ctrl 0)",
                scale,
            ),
            hint(
                cell("+")
                    .style(theme::ghost_button)
                    .on_press(Message::ZoomIn),
                "Zoom in (Ctrl +)",
                scale,
            ),
        ]
        .spacing(theme::SPACE_TIGHT * scale)
        .align_y(iced::Alignment::Center),
    )
    .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
    .width(Length::Fill)
    .into()
}

/// The resize grip on the column's inner edge — a `splitter` that reports
/// horizontal drag deltas as `Message::SidebarResize`. Only ever reached
/// with the column open (the collapsed rail has a fixed width), and it
/// needs no overlay of its own: it sits in the column's own left padding.
fn resize_strip(scale: f32) -> Element<'static, Message> {
    crate::widgets::splitter::splitter(Message::SidebarResize, scale, RESIZE_W * scale)
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
/// wrapping state word is worse than a second line. It is a button that
/// opens the device settings (2026-10-20), so its `▾` means something
/// again: an earlier one decorated a chip nothing could click, and was
/// removed for that.
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
    let settings = state.device.settings();
    // The device card is the one way into the device's settings: it says
    // what the card is, what state it is in and how it is clocked, and a
    // click on it opens Device settings beside it. The gear
    // says "settings"; a caret said "dropdown", and the top bar's clock
    // field opening the same popover made two doors for one room.
    hint(
        button(
            column![
                row![
                    text("\u{25CF}")
                        .size(theme::TEXT_XS * scale)
                        .color(status_color),
                    text(display_name)
                        .size(theme::TEXT_MD * scale)
                        .color(theme::TEXT_PRIMARY)
                        .wrapping(iced::advanced::text::Wrapping::None),
                    iced::widget::Space::new().width(Length::Fill),
                    text("\u{2699}")
                        .size(theme::TEXT_SM * scale)
                        .color(theme::TEXT_SEC),
                ]
                .spacing(theme::SPACE_SM * scale)
                .align_y(iced::Alignment::Center),
                text(status_label)
                    .size(theme::TEXT_XS * scale)
                    .color(status_color),
                text(clock_line(state, &settings.clock_source))
                    .size(theme::TEXT_XS * scale)
                    .color(theme::TEXT_SEC)
                    .wrapping(iced::advanced::text::Wrapping::None),
            ]
            .spacing(theme::SPACE_HAIRLINE * scale),
        )
        .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
        .width(Length::Fill)
        .style(theme::chip_button(state.show_device_panel))
        .on_press(Message::ToggleDevicePanel)
        // A narrow sidebar clips a long model name instead of letting it
        // run past the card's border.
        .clip(true),
        "Device settings: clock, sample rate, pitch, global options",
        scale,
    )
}

/// The device card's third line: what the card is running at. While the card
/// plays, its real rate and period ("48 kHz · 256 smp · Internal"); idle, the
/// sound server's rate; the USB backend sets the rate itself. With the kernel
/// driver and no PipeWire there is no rate to report truthfully (the model's
/// own field is a constant), so only the clock source shows.
pub(crate) fn clock_line(state: &TuxMix, clock_source: &str) -> String {
    use crate::app::rate_label;
    // The card's own playback stream wins while it plays: its period is the
    // card's real buffer. With nothing playing there is no real period to
    // show (the graph's quantum is not it: the card opens at what its
    // clients ask, 256 by default), so the line gives the rate only.
    if let Some(card) = state.card_stream {
        return format!(
            "{} \u{00b7} {} smp \u{00b7} {clock_source}",
            rate_label(card.rate),
            card.period
        );
    }
    match state.engine {
        Some(e) => format!("{} \u{00b7} {clock_source}", rate_label(e.effective_rate())),
        None if state.device.rate_follows_sound_server() => clock_source.to_string(),
        None => format!(
            "{} \u{00b7} {clock_source}",
            rate_label(state.device.settings().sample_rate)
        ),
    }
}

/// Real: pops/pushes `undo_stack`/`redo_stack` (see `Message::Undo`'s
/// own doc comment for the capture granularity). Disabled (no
/// `on_press`) when the relevant stack is empty, via `on_press_maybe`.
/// Flat, and the only two rows here with nothing else to say: an empty
/// stack now reads as "nothing to do" instead of as two disabled boxes.
fn undo_redo_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    row![
        hint(
            button(text("\u{2039} undo").size(theme::TEXT_XS * scale).center())
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                .width(Length::Fill)
                .style(theme::ghost_button)
                .on_press_maybe((!state.undo_stack.is_empty()).then_some(Message::Undo)),
            "Undo (Ctrl+Z)",
            scale,
        ),
        hint(
            button(text("redo \u{203a}").size(theme::TEXT_XS * scale).center())
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                .width(Length::Fill)
                .style(theme::ghost_button)
                .on_press_maybe((!state.redo_stack.is_empty()).then_some(Message::Redo)),
            "Redo (Ctrl+Shift+Z)",
            scale,
        ),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .width(Length::Fill)
    .into()
}

/// The global row, TotalMix's Mute/Solo/Fader masters plus the hardware
/// Dim. All four report real state, so all keep `toggle_button`'s full
/// fill when lit.
///
/// * `M` lights while any channel is muted; a press clears those mutes and
///   the next one brings them back (`Message::GlobalMuteToggle`). In
///   between, it and the channels it holds show as armed (dimmed outline,
///   `theme::armed_toggle_button`), as TotalMix keeps them marked.
/// * `S` is the same for solos (`Message::GlobalSoloToggle`).
/// * `F` is TotalMix's temporary fader group, which here is the strip
///   multi-selection (Ctrl/Shift-click): its faders already move together
///   relatively. Lit while a selection exists, a press clears it; with
///   nothing selected it is disabled, since there is nothing to clear.
/// * `DIM` flips `RmeDevice::set_dim`, the switch the front-panel DIM
///   button toggles, which had no on-screen control at all. RME-protocol
///   only, so it is absent in Class Compliant mode.
fn msf_row(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let any_mute = crate::app::any_channel_muted(state);
    let any_solo = crate::app::any_channel_soloed(state);
    // Armed: the master switched its mutes (solos) off and a press brings
    // them back; the channels it holds show the same dimmed state.
    let mute_held = !any_mute && !state.last_cleared_mutes.is_empty();
    let solo_held = !any_solo && !state.last_cleared_solos.is_empty();
    let has_group = !state.selected.is_empty();
    let cell = |label: &'static str| {
        button(text(label).size(theme::TEXT_SM * scale).center())
            .padding(theme::SPACE_SM * scale)
            .width(Length::Fill)
    };
    let mut r = row![
        hint(
            cell("M")
                .style(theme::armed_toggle_button(
                    any_mute,
                    mute_held,
                    theme::MUTE_COLOR
                ))
                .on_press(Message::GlobalMuteToggle),
            if mute_held {
                "Mute master: bring the held mutes back"
            } else {
                "Mute master: switch the active mutes off"
            },
            scale,
        ),
        hint(
            cell("S")
                .style(theme::armed_toggle_button(
                    any_solo,
                    solo_held,
                    theme::SOLO_COLOR
                ))
                .on_press(Message::GlobalSoloToggle),
            if solo_held {
                "Solo master: bring the held solos back"
            } else {
                "Solo master: switch the active solos off"
            },
            scale,
        ),
        hint(
            cell("F")
                .style(theme::toggle_button(has_group, theme::ACCENT))
                .on_press_maybe(has_group.then_some(Message::ClearSelection)),
            "Fader group (Ctrl/Shift-click strips): clear it",
            scale,
        ),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .width(Length::Fill);
    if state.device.protocol_features() {
        r = r.push(hint(
            cell("DIM")
                .style(theme::toggle_button(
                    state.device.settings().dim,
                    theme::ACCENT,
                ))
                .on_press(Message::DimToggle),
            "Dim the phones output (also the front-panel DIM button)",
            scale,
        ));
    }
    r.into()
}

/// A short tooltip under a sidebar control, in the same chip as the panel
/// toggle's.
fn hint<'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
    scale: f32,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        content,
        container(text(label).size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .style(theme::chip),
        iced::widget::tooltip::Position::Bottom,
    )
    .into()
}

/// One panel's header — the sidebar's only structure. The same
/// vocabulary as the mixer's `section_header` (accent tick, label, rule
/// trailing right) at sidebar scale, with a collapse chevron at the end. This replaced a boxed chip: four
/// identical bordered headers stacked in one column gave the panels
/// exactly the weight of the controls inside them, so neither read as
/// either.
fn panel_header(label: &str, open: bool, on_toggle: Message, scale: f32) -> Element<'_, Message> {
    // The whole header is the toggle, the way collapsible sections work
    // in editors and design tools: the old "−"/"+" was a 10px target at
    // the far end of the rule, and nothing about the label said it folds.
    // The chevron (Inter covers U+25BE/U+25B8) says which way it is.
    button(
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
            // No color of its own: it takes `header_button`'s, which is
            // what lights up under the cursor.
            text(if open { "\u{25BE}" } else { "\u{25B8}" }).size(theme::TEXT_XS * scale),
        ]
        .spacing(theme::SPACE_SM * scale)
        .align_y(iced::Alignment::Center)
        .width(Length::Fill),
    )
    .padding([theme::SPACE_TIGHT * scale, 0.0])
    .width(Length::Fill)
    .style(theme::header_button)
    .on_press(on_toggle)
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

/// A 2-way segmented switch: one side lit, a press on either side
/// sends that side's message (`None` leaves a side inert).
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

/// An N-way segmented switch in the shared track: `(label, lit, message)`
/// per cell, `None` leaving a cell inert.
fn segmented<'a>(cells: &[(&'a str, bool, Option<Message>)], scale: f32) -> Element<'a, Message> {
    let mut r = row![].spacing(theme::SPACE_HAIRLINE * scale);
    for (label, lit, msg) in cells {
        r = r.push(
            button(text(*label).size(theme::TEXT_XS * scale).center())
                .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                .width(Length::Fill)
                .style(theme::segment_cell(*lit))
                .on_press_maybe(msg.clone()),
        );
    }
    container(r)
        .padding(theme::SPACE_TIGHT * scale)
        .width(Length::Fill)
        .style(theme::segment_track)
        .into()
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
    dirty: bool,
    stored: &[bool],
    on_press: fn(u8) -> Message,
    scale: f32,
) -> Element<'a, Message> {
    let mut grid = column![].spacing(theme::SPACE_TIGHT * scale);
    for first in (1..=count).step_by(2) {
        let mut line = row![].spacing(theme::SPACE_TIGHT * scale);
        for n in first..=(first + 1).min(count) {
            let is_active = active == Some(n as usize);
            // An empty slot is still clickable (selecting it is how you
            // pick where "store" writes), but it reads as empty.
            let is_stored = stored.get((n - 1) as usize).copied().unwrap_or(false);
            let name_color = if is_stored || is_active {
                None
            } else {
                Some(Color {
                    a: 0.45,
                    ..theme::TEXT_SEC
                })
            };
            // The selected slot, modified since it was recalled or stored,
            // carries an amber dot: TotalMix flashes the button instead.
            let label: Element<'a, Message> = if is_active && dirty {
                row![
                    text(format!("{prefix} {n}"))
                        .size(theme::TEXT_XS * scale)
                        .color_maybe(name_color),
                    text("\u{25CF}")
                        .size(theme::TEXT_MICRO * scale)
                        .color(theme::YSIM),
                ]
                .spacing(theme::SPACE_SM * scale)
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                text(format!("{prefix} {n}"))
                    .size(theme::TEXT_XS * scale)
                    .color_maybe(name_color)
                    .into()
            };
            let tip = if is_stored {
                format!("Recall {prefix} {n}")
            } else {
                "Empty: select it, then store".to_string()
            };
            line = line.push(iced::widget::tooltip(
                button(container(label).center_x(Length::Fill))
                    .padding([theme::SPACE_SM * scale, theme::SPACE_SM * scale])
                    .width(Length::Fill)
                    .style(theme::segment_button(is_active))
                    .on_press(on_press(n)),
                container(text(tip).size(theme::TEXT_XS * scale))
                    .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
                    .style(theme::chip),
                iced::widget::tooltip::Position::Bottom,
            ));
        }
        grid = grid.push(line);
    }
    grid.width(Length::Fill).into()
}

/// The trailing "store" action of the Snapshot and Mixer Layout panels —
/// right-aligned, flat text, the same weight as the panel's own collapse
/// toggle. It names the slot it will write ("store to Mix 3"), so an
/// overwrite is never a surprise, and is disabled, saying why, until a
/// slot is selected (it used to look live and do nothing).
fn store_row(
    prefix: &str,
    active: Option<usize>,
    on_press: Message,
    scale: f32,
) -> Element<'static, Message> {
    let label = match active {
        Some(n) => format!("store to {prefix} {n}"),
        None => "select a slot to store".to_string(),
    };
    container(
        button(text(label).size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .style(theme::ghost_button)
            .on_press_maybe(active.map(|_| on_press)),
    )
    .width(Length::Fill)
    .align_x(iced::Alignment::End)
    .into()
}

/// TotalMix's Options panel, cut to what this app can actually do. The
/// mixer layout ("2 Row" in TotalMix, `Message::SetRowMode`) and "names"
/// (TotalMix's "Show Names", `Message::SetShowOriginalNames`) are real.
/// The rest of TotalMix's panel was dropped rather than shown as dead
/// switches: "routing: free" (editing several submixes at once) and
/// "show: trim" have no counterpart here, "meters: post FX" is moot without an FX
/// engine and "RMS" needs a numeric level readout the strips do not have.
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

    use crate::app::RowMode;
    body = body.push(options_row(
        // "rows" as the caption keeps the three cells one word each: in
        // a 210 px column "3 rows" / "2 rows" wrapped onto two lines.
        Some("mixer rows"),
        segmented(
            &[
                ("auto", RowMode::Auto),
                ("3", RowMode::Three),
                ("2", RowMode::Two),
            ]
            .map(|(label, mode)| {
                (
                    label,
                    state.row_mode == mode,
                    Some(Message::SetRowMode(mode)),
                )
            }),
            scale,
        ),
        scale,
    ));

    // TotalMix's "Show Names": the hardware names, even where a strip was
    // renamed. With nothing renamed there is nothing to switch, so the row
    // is inert and says how to rename instead of lighting up and doing
    // nothing.
    let has_labels = !state.labels.is_empty();
    let names = segmented_row(
        "custom",
        "original",
        !state.show_original_names || !has_labels,
        has_labels.then_some(Message::SetShowOriginalNames(false)),
        has_labels.then_some(Message::SetShowOriginalNames(true)),
        scale,
    );
    body = body.push(options_row(
        Some("names"),
        if has_labels {
            names
        } else {
            hint(names, "Double-click a strip's name to rename it", scale)
        },
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
        state.snapshot_dirty,
        &state.stored_snapshots,
        Message::SnapshotClicked,
        scale,
    ));
    body = body.push(store_row(
        "Mix",
        state.active_snapshot,
        Message::SnapshotStore,
        scale,
    ));

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
/// own file-per-slot pattern). TotalMix's "all/submix" scope row is not
/// here: a layout is not scoped per submix in this app's model.
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

    let layouts_stored: Vec<bool> = state.layouts.iter().map(Option::is_some).collect();
    body = body.push(slot_grid(
        LAYOUT_COUNT,
        "Layout",
        state.active_layout,
        false,
        &layouts_stored,
        Message::LayoutClicked,
        scale,
    ));
    body = body.push(store_row(
        "Layout",
        state.active_layout,
        Message::LayoutStore,
        scale,
    ));

    column![header, body]
        .spacing(theme::SPACE_SM * scale)
        .width(Length::Fill)
        .into()
}
