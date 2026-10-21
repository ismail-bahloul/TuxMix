//! Matrix (submix) view — TotalMix's own crosspoint grid: every input/
//! playback *channel* (not pair) as a row, every hardware output
//! *channel* (not pair) as a column, one small numeric dB cell per
//! crosspoint. Rebuilt 2026-09-06 against a real TotalMix Matrix
//! screenshot (`Totalmix UI Screenshots/`, provided directly by the
//! user) — the previous version (rows = output *pairs*, columns =
//! input/playback channels, one small fader per cell) had the axes
//! backwards and used a fader-per-cell instead of TotalMix's bare
//! numeric cells.
//!
//! Interaction and colors follow RME's own Matrix documentation
//! (docs.rme-audio.com, "The Matrix"), see `widgets::crosspoint`: drag a
//! cell up/down for its gain, Ctrl+click toggles it between off and 0 dB,
//! double-click resets it, the wheel scrolls the grid. Pointing at a cell
//! lights its row and column labels ("the outer labels light up ...
//! according to the mouse position") and the view's caption reads it out.
//! The column header, the output masters and the bus labels stay put
//! while the source rows scroll under them (TotalMix: "the labels are
//! floating. They won't leave the visible area when scrolling").
//!
//! Only the side of an output pair a channel can actually reach is
//! interactive; the other side (for every source except the 4 true-mono
//! AN1-4 inputs) is an inert cell, since the hardware itself has no way
//! to route there (see `push_channel_rows`'s own doc comment).

use iced::keyboard::Modifiers;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Color, Element, Length};

use tuxmix_core::{ChannelId, ChannelType, RmeDevice};

use crate::app::{pair_bus_label, short_label, Message, TuxMix, OUT_LABELS};
use crate::theme;
use crate::widgets::crosspoint::{cell_text, crosspoint, Crosspoint};

/// Column width / row height at `scale == 1.0` — sized for a signed,
/// one-decimal dB value.
const CELL_W: f32 = 42.0;
const CELL_H: f32 = 20.0;
/// Row-group label column ("AN1", "IN3/4", "AS1/2", ...).
const GROUP_LABEL_W: f32 = 64.0;
/// Per-row sequential label column ("In 1", "Pb 12", ...).
const IDX_LABEL_W: f32 = 40.0;
/// Matches `strip.rs`'s own "recall a sane starting point" reset value
/// (see `Message::Reset`) — a double-clicked crosspoint goes back to
/// this, not to full unity (Ctrl+click is the 0 dB toggle).
const RESET_VOLUME: f32 = 0.75;

/// Extra space after each output pair's two columns (and between source
/// groups' rows), on top of the grid's hairline: without it twelve equal
/// columns read as one undifferentiated wall of numbers.
const PAIR_GAP: f32 = theme::SPACE_SM;

/// One grid cell. `cid`/`pair` say what a gesture writes
/// (`Message::VolumeChanged(cid, pair, _)`), `col` is the physical output
/// column, for the hover crosshair.
#[allow(clippy::too_many_arguments)]
fn cell(
    state: &TuxMix,
    cid: ChannelId,
    pair: usize,
    col: usize,
    value: f32,
    shown: f32,
    interactive: bool,
    muted: bool,
    phase: bool,
) -> Element<'static, Message> {
    let scale = state.ui_scale;
    let modifiers: Modifiers = state.modifiers;
    crosspoint(
        Crosspoint {
            value,
            shown,
            interactive,
            muted,
            phase,
            in_sel: col / 2 == state.sel_out,
            hovered: state.matrix_hover == Some((cid, col)),
            modifiers,
            scale,
            reset_value: RESET_VOLUME,
            on_change: Box::new(move |v| Message::VolumeChanged(cid, pair, v)),
            on_release: Message::RangeCleared(cid),
            on_hover: Box::new(move |over| Message::MatrixHover(cid, col, over)),
        },
        CELL_W * scale,
        CELL_H * scale,
    )
}

fn header_cell(label: String, lit: bool, width: f32, scale: f32) -> Element<'static, Message> {
    container(text(label).size(theme::TEXT_MICRO * scale).color(if lit {
        theme::ACCENT
    } else {
        theme::TEXT_SEC
    }))
    .width(Length::Fixed(width * scale))
    .height(Length::Fixed(CELL_H * scale))
    .align_x(iced::Alignment::Center)
    .align_y(iced::Alignment::Center)
    .into()
}

fn label_cell(label: String, width: f32, color: Color, scale: f32) -> Element<'static, Message> {
    container(
        text(label)
            .size(theme::TEXT_MICRO * scale)
            .color(color)
            .wrapping(iced::advanced::text::Wrapping::None),
    )
    .width(Length::Fixed(width * scale))
    .height(Length::Fixed(CELL_H * scale))
    .align_x(iced::Alignment::Start)
    .align_y(iced::Alignment::Center)
    .clip(true)
    .into()
}

/// One row-group's worth of rows: a shared label ("AN1", "IN3/4", ...),
/// a starting sequential index (for "In N"/"Pb N"), and how many
/// physical channels it covers (1 for the two solo Mic inputs, 2 for
/// every hardware-paired source and every playback pair).
struct RowGroup {
    label: String,
    color: Color,
    first_idx: usize,
    count: usize,
}

/// Input row-groups: the two Mic-type channels (AN1, AN2) are each
/// their own physical jack, shown solo — every other input type comes
/// in an inherent hardware pair (Instrument, Line, ADAT), shown as one
/// 2-row group. Mirrors `mixer_view`'s own pairing assumption
/// (`n_input_pairs = inputs().len() / 2`) but, unlike the Mixer's
/// strips, never collapses a pair into one combined row regardless of
/// `input_pair_linked` — TotalMix's Matrix always shows every physical
/// channel's own crosspoints, link state or not.
fn input_row_groups(state: &TuxMix) -> Vec<RowGroup> {
    let inputs = state.device.inputs();
    let mut groups = Vec::new();
    let mut i = 0;
    while i < inputs.len() {
        let ty = inputs[i].channel_type;
        let (label, count) = if ty == ChannelType::Mic {
            (inputs[i].name.clone(), 1)
        } else if let Some(next) = inputs.get(i + 1) {
            (pair_bus_label(&inputs[i].name, &next.name), 2)
        } else {
            (inputs[i].name.clone(), 1)
        };
        let label = crate::app::user_label(state, ChannelId::Input(i))
            .map(str::to_string)
            .unwrap_or(label);
        groups.push(RowGroup {
            label,
            color: crate::app::type_tag(ty).1,
            first_idx: i,
            count,
        });
        i += count;
    }
    groups
}

/// Playback row-groups: always one pair per hardware output bus,
/// labeled with the same verified `OUT_LABELS` the rest of the app
/// uses (not TotalMix's own instance-specific renaming in the
/// reference screenshot, e.g. "Main" for what this hardware calls
/// "PH3/4" — that's this session's own established naming, kept).
fn playback_row_groups(state: &TuxMix) -> Vec<RowGroup> {
    let n = state.device.playbacks().len();
    (0..n / 2)
        .map(|pair| RowGroup {
            label: crate::app::user_label(state, ChannelId::Playback(pair * 2))
                .map(str::to_string)
                .or_else(|| OUT_LABELS.get(pair).map(|s| s.to_string()))
                .unwrap_or_default(),
            color: crate::app::PB_TAG,
            first_idx: pair * 2,
            count: 2,
        })
        .collect()
}

/// A mono source's (volume, pan) decoded into what reaches the left and
/// right outputs of a pair.
fn mono_sides(vol: f32, pan: i8) -> (f32, f32) {
    let pf = pan as f32 / 100.0;
    if pan == 0 {
        (vol, vol)
    } else if pf < 0.0 {
        (vol, vol * (1.0 + pf))
    } else {
        (vol * (1.0 - pf), vol)
    }
}

/// What one source row needs from the device model.
struct RowSource {
    volumes: Vec<f32>,
    /// `Some` for the true-mono sources (AN1-4), which reach both sides
    /// of every pair through their pan.
    pans: Option<Vec<i8>>,
    muted: bool,
    phase: bool,
}

fn row_source(state: &TuxMix, cid: ChannelId) -> Option<RowSource> {
    match cid {
        ChannelId::Input(i) => state.device.inputs().get(i).map(|ch| RowSource {
            volumes: ch.volumes.clone(),
            pans: (i < 4).then(|| ch.pans.clone()),
            muted: ch.mute,
            phase: ch.phase,
        }),
        ChannelId::Playback(i) => state.device.playbacks().get(i).map(|ch| RowSource {
            volumes: ch.volumes.clone(),
            pans: None,
            muted: ch.mute,
            phase: false,
        }),
        ChannelId::Output(_) => None,
    }
}

/// The gain a source row sends to physical output column `col`, as the
/// cell shows it (`None` = the hardware has no path there).
fn shown_gain(src: &RowSource, idx: usize, col: usize) -> Option<f32> {
    let pair = col / 2;
    let vol = *src.volumes.get(pair)?;
    match &src.pans {
        Some(pans) => {
            let (l, r) = mono_sides(vol, pans.get(pair).copied().unwrap_or(0));
            Some(if col % 2 == 0 { l } else { r })
        }
        None => (col % 2 == idx % 2).then_some(vol),
    }
}

/// Builds every data row for one section (inputs or playbacks) and
/// appends them to `rows`.
///
/// **Which columns are interactive**: our own per-channel model stores
/// one volume *per output pair*, not per individual physical output
/// channel — matching the hardware itself, which (per `babyface.rs`'s
/// own crosspoint comment) has no way to route a non-mono source
/// differently into an output pair's left vs right side at all. That
/// one value is editable through whichever single column (left/right)
/// this channel naturally feeds (`idx % 2`, the same convention
/// `set_channel_volume` already uses everywhere); the other column of
/// that pair is inert — there's nothing real on this hardware to drag.
/// The 4 true-mono sources (AN1-4) are the one genuine exception: both
/// columns show their pan-decoded values and both are interactive,
/// moving the *same* underlying `volumes[pair]` (this view edits level,
/// not pan).
fn push_channel_rows<'a>(
    state: &'a TuxMix,
    mut rows: iced::widget::Column<'a, Message>,
    groups: &[RowGroup],
    row_prefix: &'static str,
    cid_of: impl Fn(usize) -> ChannelId,
) -> iced::widget::Column<'a, Message> {
    let scale = state.ui_scale;
    let hovered_row = state.matrix_hover.map(|(c, _)| c);
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            rows = rows.push(iced::widget::Space::new().height(PAIR_GAP * scale));
        }
        for local in 0..g.count {
            let idx = g.first_idx + local;
            let cid = cid_of(idx);
            let Some(src) = row_source(state, cid) else {
                continue;
            };
            let lit = hovered_row == Some(cid);
            let mut r = row![
                label_cell(
                    if local == 0 {
                        g.label.clone()
                    } else {
                        String::new()
                    },
                    GROUP_LABEL_W,
                    if lit { theme::ACCENT } else { g.color },
                    scale
                ),
                label_cell(
                    format!("{row_prefix} {}", idx + 1),
                    IDX_LABEL_W,
                    if lit { theme::ACCENT } else { theme::TEXT_SEC },
                    scale
                ),
            ]
            .spacing(theme::SPACE_HAIRLINE);

            for col in 0..src.volumes.len() * 2 {
                let pair = col / 2;
                let stored = src.volumes[pair];
                r = r.push(match shown_gain(&src, idx, col) {
                    Some(shown) => cell(
                        state, cid, pair, col, stored, shown, true, src.muted, src.phase,
                    ),
                    None => cell(state, cid, pair, col, 0.0, 0.0, false, false, false),
                });
                if col % 2 == 1 {
                    r = r.push(pair_gap(scale));
                }
            }
            rows = rows.push(r);
        }
    }
    rows
}

/// A row naming the source section below it, in the Mixer's own section
/// vocabulary, so the playback rows don't run straight on from the inputs.
fn section_caption(label: &str, scale: f32) -> Element<'static, Message> {
    container(
        text(label.to_uppercase())
            .size(theme::TEXT_MICRO * scale)
            .color(theme::TEXT_SEC),
    )
    .height(Length::Fixed(CELL_H * scale))
    .align_y(iced::Alignment::End)
    .padding(iced::Padding {
        bottom: theme::SPACE_TIGHT * scale,
        ..iced::Padding::ZERO
    })
    .into()
}

fn pair_gap(scale: f32) -> Element<'static, Message> {
    iced::widget::Space::new().width(PAIR_GAP * scale).into()
}

fn hairline(width: f32) -> Element<'static, Message> {
    container(
        iced::widget::Space::new()
            .width(Length::Fixed(width))
            .height(1),
    )
    .style(|_theme: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(theme::BORDER)),
        ..container::Style::default()
    })
    .into()
}

/// The hovered cell, spelled out for the view's caption:
/// "AN1 -> Out 3 (PH3/4 L): -6.0 dB". `None` when nothing is hovered.
pub fn hover_readout(state: &TuxMix) -> Option<String> {
    let (cid, col) = state.matrix_hover?;
    let pair = col / 2;
    let bus = crate::app::user_label(state, ChannelId::Output(pair * 2))
        .unwrap_or(OUT_LABELS.get(pair).copied().unwrap_or_default())
        .to_string();
    let side = if col % 2 == 0 { "L" } else { "R" };
    let (source, gain) = match cid {
        ChannelId::Output(i) => {
            let out = state.device.outputs().get(i)?;
            ("Output master".to_string(), Some(out.volume))
        }
        ChannelId::Input(i) | ChannelId::Playback(i) => {
            let name = crate::app::user_label(state, cid)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    match cid {
                        ChannelId::Input(_) => state.device.inputs().get(i).map(|c| c.name.clone()),
                        _ => state
                            .device
                            .playbacks()
                            .get(i)
                            .map(|c| short_label(&c.name).to_string()),
                    }
                    .unwrap_or_default()
                });
            let src = row_source(state, cid)?;
            (name, shown_gain(&src, i, col))
        }
    };
    let level = match gain {
        None => "no hardware path".to_string(),
        Some(v) => match cell_text(v) {
            Some(db) => format!("{db} dB"),
            None => "off".to_string(),
        },
    };
    Some(format!(
        "{source} \u{2192} Out {} ({bus} {side}): {level}",
        col + 1
    ))
}

pub fn view(state: &TuxMix) -> Element<'_, Message> {
    iced::widget::responsive(move |size| grid_view(state, size)).into()
}

/// The whole grid for a panel of `size` (see the frozen-panes note below
/// for why the size matters).
fn grid_view(state: &TuxMix, size: iced::Size) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let n_outputs = state.device.outputs().len();
    let hovered_col = state.matrix_hover.map(|(_, c)| c);

    // ── Column header: individual output channels ("Out 1".."Out 12"),
    // lit for the selected submix's pair and for the hovered column.
    let mut header = row![
        label_cell(String::new(), GROUP_LABEL_W, theme::TEXT_SEC, scale),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for out in 0..n_outputs {
        header = header.push(header_cell(
            format!("Out {}", out + 1),
            out / 2 == state.sel_out || hovered_col == Some(out),
            CELL_W,
            scale,
        ));
        if out % 2 == 1 {
            header = header.push(pair_gap(scale));
        }
    }

    let mut sources =
        column![section_caption("Hardware inputs", scale)].spacing(theme::SPACE_HAIRLINE);
    sources = push_channel_rows(
        state,
        sources,
        &input_row_groups(state),
        "In",
        ChannelId::Input,
    );
    sources = sources.push(iced::widget::Space::new().height(PAIR_GAP * scale));
    sources = sources.push(section_caption("Software playback", scale));
    sources = push_channel_rows(
        state,
        sources,
        &playback_row_groups(state),
        "Pb",
        ChannelId::Playback,
    );

    // ── Master "Outputs" row: each output channel's own master volume,
    // distinct from (and separated by a divider from) the crosspoints.
    let out_lit = matches!(state.matrix_hover, Some((ChannelId::Output(_), _)));
    let mut out_row = row![
        label_cell(
            "Outputs".into(),
            GROUP_LABEL_W,
            if out_lit {
                theme::ACCENT
            } else {
                theme::TEXT_PRIMARY
            },
            scale
        ),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for (idx, out) in state.device.outputs()[..n_outputs].iter().enumerate() {
        out_row = out_row.push(cell(
            state,
            ChannelId::Output(idx),
            0,
            idx,
            out.volume,
            out.volume,
            true,
            out.mute,
            false,
        ));
        if idx % 2 == 1 {
            out_row = out_row.push(pair_gap(scale));
        }
    }

    // ── Column-group footer: each output pair's bus label, one label
    // spanning both its columns.
    let mut footer = row![
        label_cell(String::new(), GROUP_LABEL_W, theme::TEXT_SEC, scale),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for pair in 0..(n_outputs / 2) {
        let label = crate::app::user_label(state, ChannelId::Output(pair * 2))
            .unwrap_or(OUT_LABELS.get(pair).copied().unwrap_or_default())
            .to_string();
        footer = footer.push(header_cell(
            label,
            pair == state.sel_out || hovered_col.is_some_and(|c| c / 2 == pair),
            2.0 * CELL_W + theme::SPACE_HAIRLINE / scale,
            scale,
        ));
        footer = footer.push(pair_gap(scale));
    }

    // Frozen panes: only the source rows scroll vertically, under a fixed
    // header and above the fixed output row and bus labels; the whole grid
    // scrolls sideways as one, so the columns never drift from their
    // labels. The source area gets a fixed height only when its rows do
    // not fit: a column lays its children out in order, so a scrollable
    // left to size itself takes all the room before the output row is
    // placed, and pushes it off the panel.
    let n_inputs = state.device.inputs().len();
    let n_playbacks = state.device.playbacks().len();
    let group_gaps = input_row_groups(state).len().saturating_sub(1)
        + playback_row_groups(state).len().saturating_sub(1);
    let source_elements = n_inputs + n_playbacks + 2 + group_gaps + 1;
    let sources_h = (n_inputs + n_playbacks + 2) as f32 * CELL_H * scale
        + (group_gaps + 1) as f32 * PAIR_GAP * scale
        + source_elements.saturating_sub(1) as f32 * theme::SPACE_HAIRLINE;
    let fixed_h = 3.0 * CELL_H * scale + PAIR_GAP * scale + 1.0 + 5.0 * theme::SPACE_HAIRLINE;
    let grid_w = (GROUP_LABEL_W + IDX_LABEL_W) * scale
        + n_outputs as f32 * CELL_W * scale
        + (n_outputs / 2) as f32 * PAIR_GAP * scale
        + (n_outputs + 2) as f32 * theme::SPACE_HAIRLINE;
    let pad = theme::SPACE_LG;

    let sideways_bar = if grid_w > size.width - 2.0 * pad {
        10.0
    } else {
        0.0
    };
    let room = size.height - 2.0 * pad - fixed_h - sideways_bar;
    let body_h = if sources_h <= room {
        Length::Shrink
    } else {
        Length::Fixed(room.max(3.0 * CELL_H * scale))
    };
    let body = scrollable(sources)
        .direction(scrollable::Direction::Vertical(theme::thin_scrollbar()))
        .height(body_h)
        .style(theme::scrollable);
    let grid = column![
        header,
        body,
        iced::widget::Space::new().height(PAIR_GAP * scale),
        hairline(grid_w),
        out_row,
        footer,
    ]
    .spacing(theme::SPACE_HAIRLINE);
    // Sized to the grid, so the panel hugs it instead of running to the
    // window's edge as an empty frame; it still scrolls sideways when the
    // window is narrower than the grid.
    let sideways = scrollable(grid)
        .direction(scrollable::Direction::Horizontal(theme::thin_scrollbar()))
        .width(Length::Shrink)
        .style(theme::scrollable);
    container(container(sideways).style(theme::panel).padding(pad))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::{mono_sides, shown_gain, RowSource};

    #[test]
    fn a_stereo_source_reaches_only_its_own_side() {
        let src = RowSource {
            volumes: vec![0.5, 1.0],
            pans: None,
            muted: false,
            phase: false,
        };
        // An odd channel (right of its pair) reaches the odd columns.
        assert_eq!(shown_gain(&src, 1, 0), None);
        assert_eq!(shown_gain(&src, 1, 1), Some(0.5));
        assert_eq!(shown_gain(&src, 1, 3), Some(1.0));
    }

    #[test]
    fn a_mono_source_is_split_by_its_pan() {
        assert_eq!(mono_sides(1.0, 0), (1.0, 1.0));
        let (l, r) = mono_sides(1.0, -50);
        assert_eq!(l, 1.0);
        assert!((r - 0.5).abs() < 1e-6);
        let src = RowSource {
            volumes: vec![1.0],
            pans: Some(vec![100]),
            muted: false,
            phase: false,
        };
        assert_eq!(shown_gain(&src, 0, 0), Some(0.0));
        assert_eq!(shown_gain(&src, 0, 1), Some(1.0));
    }
}
