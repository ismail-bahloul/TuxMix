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
//! Cells are click-drag interactive (a hidden `Fader`, see
//! `Fader::show_track`/`label`) — vertical drag adjusts the crosspoint,
//! same feel as every other fader in the app, just with no visible
//! track/cap so the bare number stays the only thing drawn. Only the
//! side of an output pair a channel can actually reach is interactive;
//! the other side (for every source except the 4 true-mono AN1-4
//! inputs) is a plain static cell, since the hardware itself has no
//! way to route there (see `push_channel_rows`'s own doc comment).

use iced::keyboard::Modifiers;
use iced::widget::{column, container, row, scrollable, text};
use iced::{Color, Element, Length};

use tuxmix_core::{ChannelId, ChannelType, RmeDevice};

use crate::app::{pair_bus_label, Message, TuxMix, OUT_LABELS};
use crate::theme;
use crate::widgets::fader::{fader, Fader, MeterFrame};

/// Column width / row height at `scale == 1.0` — sized for a 2-3
/// character dB value, not a fader track.
const CELL_W: f32 = 40.0;
const CELL_H: f32 = 20.0;
/// Row-group label column ("AN1", "IN3/4", "AS1/2", ...).
const GROUP_LABEL_W: f32 = 56.0;
/// Per-row sequential label column ("In 1", "Pb 12", ...).
const IDX_LABEL_W: f32 = 40.0;
/// Matches `strip.rs`'s own "recall a sane starting point" reset value
/// (see `Message::Reset`) — dragged crosspoints double-click back to
/// this, not to full unity.
const RESET_VOLUME: f32 = 0.75;

/// Bare "-9.8"-style cell text (no " dB" suffix, unlike `app::db_text`
/// — TotalMix's own matrix cells are bare numbers) — `None` for an
/// effectively-silent crosspoint, which TotalMix leaves blank rather
/// than printing "-inf" into every unrouted cell.
fn cell_text(vol: f32) -> Option<String> {
    (vol > f32::EPSILON).then(|| format!("{:.1}", 20.0 * vol.log10()))
}

/// A plain, non-interactive cell — used for the side of an output pair
/// a channel has no hardware path to (see the module doc comment).
fn cell(value: Option<String>, scale: f32) -> Element<'static, Message> {
    let active = value.is_some();
    container(
        text(value.unwrap_or_default())
            .size(theme::TEXT_MICRO * scale)
            .color(if active { theme::MGREEN } else { theme::TEXT_SEC }),
    )
    .width(Length::Fixed(CELL_W * scale))
    .height(Length::Fixed(CELL_H * scale))
    .align_x(iced::Alignment::Center)
    .align_y(iced::Alignment::Center)
    .style(theme::panel)
    .into()
}

/// A real crosspoint cell: a `Fader` with `show_track: false` and its
/// bare dB value as `label` — a fully working vertical drag hidden
/// behind the number, not a second visible control. `on_press`/
/// `on_drag` write straight to `Message::VolumeChanged`, matching this
/// view's own pre-existing (undo-exempt) convention rather than
/// `strip.rs`'s `FaderPressed` — Matrix edits have never fed the undo
/// stack, and changing that is a separate decision from rebuilding the
/// grid itself.
fn interactive_cell(
    vol: f32,
    cid: ChannelId,
    pair: usize,
    modifiers: Modifiers,
    scale: f32,
) -> Element<'static, Message> {
    let active = vol > f32::EPSILON;
    let f = fader(Fader {
        value: vol,
        range: (0.0, 2.0),
        default_value: 1.0,
        meter: MeterFrame::still(0.0),
        meter_available: false,
        height: CELL_H * scale,
        show_meter: false,
        reserved_right: 0.0,
        modifiers,
        scale,
        show_track: false,
        label: Some(cell_text(vol).unwrap_or_default()),
        label_color: if active { theme::MGREEN } else { theme::TEXT_SEC },
        on_press: Box::new(move |v, _| Message::VolumeChanged(cid, pair, v)),
        on_drag: Box::new(move |v| Message::VolumeChanged(cid, pair, v)),
        on_release: Box::new(move || Message::RangeCleared(cid)),
        on_reset: Box::new(move || Message::VolumeChanged(cid, pair, RESET_VOLUME)),
    });
    // `fader()` sizes its own canvas to the fader track's width
    // (narrower than a full cell) when `show_meter` is `false` — wrap
    // it so the clickable/visible area matches a plain `cell()`'s
    // footprint exactly, same border/background too, so an
    // interactive cell doesn't read as a different kind of thing from
    // its static neighbor.
    container(f)
        .width(Length::Fixed(CELL_W * scale))
        .height(Length::Fixed(CELL_H * scale))
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Center)
        .style(theme::panel)
        .into()
}

fn header_cell(label: String, active: bool, scale: f32) -> Element<'static, Message> {
    container(
        text(label)
            .size(theme::TEXT_MICRO * scale)
            .color(if active { theme::ACCENT } else { theme::TEXT_SEC }),
    )
    .width(Length::Fixed(CELL_W * scale))
    .height(Length::Fixed(CELL_H * scale))
    .align_x(iced::Alignment::Center)
    .align_y(iced::Alignment::Center)
    .into()
}

fn label_cell(label: String, width: f32, color: Color, scale: f32) -> Element<'static, Message> {
    container(text(label).size(theme::TEXT_MICRO * scale).color(color))
        .width(Length::Fixed(width * scale))
        .height(Length::Fixed(CELL_H * scale))
        .align_x(iced::Alignment::Start)
        .align_y(iced::Alignment::Center)
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
            label: OUT_LABELS.get(pair).map(|s| s.to_string()).unwrap_or_default(),
            color: crate::app::PB_TAG,
            first_idx: pair * 2,
            count: 2,
        })
        .collect()
}

/// Builds every data row for one section (inputs or playbacks) and
/// appends them to `rows`. `volumes_of`/`pans_of` read straight off
/// `state`, so they're plain closures rather than a borrowed slice —
/// keeps this generic over `InputChannel`/`PlaybackChannel` without a
/// shared trait between the two.
///
/// **Which columns are interactive**: our own per-channel model stores
/// one volume *per output pair*, not per individual physical output
/// channel — matching the hardware itself, which (per `babyface.rs`'s
/// own crosspoint comment) has no way to route a non-mono source
/// differently into an output pair's left vs right side at all. That
/// one value is editable through whichever single column (left/right)
/// this channel naturally feeds (`idx % 2`, the same convention
/// `set_channel_volume` already uses everywhere); the other column of
/// that pair is a plain, non-interactive blank — there's nothing real
/// on this hardware to drag. The 4 true-mono sources (AN1-4) are the
/// one genuine exception: both columns show independent (volume, pan)-
/// decoded values, and both are interactive, dragging the *same*
/// underlying `volumes[pair]` — this view edits level, not pan, so
/// either column moving the one real stored value (rather than solving
/// a new pan split per drag) is the correct, simpler answer.
#[allow(clippy::too_many_arguments)]
fn push_channel_rows(
    mut rows: iced::widget::Column<'static, Message>,
    groups: &[RowGroup],
    row_prefix: &'static str,
    scale: f32,
    modifiers: Modifiers,
    cid_of: impl Fn(usize) -> ChannelId,
    volumes_of: impl Fn(usize) -> Vec<f32>,
    pans_of: impl Fn(usize) -> Option<Vec<i8>>,
) -> iced::widget::Column<'static, Message> {
    for g in groups {
        for local in 0..g.count {
            let idx = g.first_idx + local;
            let cid = cid_of(idx);
            let mut r = row![
                label_cell(
                    if local == 0 { g.label.clone() } else { String::new() },
                    GROUP_LABEL_W,
                    g.color,
                    scale
                ),
                label_cell(format!("{row_prefix} {}", idx + 1), IDX_LABEL_W, theme::TEXT_SEC, scale),
            ]
            .spacing(theme::SPACE_HAIRLINE);

            let vols = volumes_of(idx);
            let pans = pans_of(idx);
            let is_mono = pans.is_some();
            let natural_side = idx % 2; // 0 = left, 1 = right
            for (pair, &vol) in vols.iter().enumerate() {
                if is_mono {
                    let pan = pans.as_ref().unwrap()[pair];
                    let (left_val, right_val) = if pan == 0 {
                        (vol, vol)
                    } else {
                        let pf = pan as f32 / 100.0;
                        if pf <= 0.0 {
                            (vol, vol * (1.0 + pf))
                        } else {
                            (vol * (1.0 - pf), vol)
                        }
                    };
                    r = r.push(interactive_cell(left_val, cid, pair, modifiers, scale));
                    r = r.push(interactive_cell(right_val, cid, pair, modifiers, scale));
                } else if natural_side == 0 {
                    r = r.push(interactive_cell(vol, cid, pair, modifiers, scale));
                    r = r.push(cell(None, scale));
                } else {
                    r = r.push(cell(None, scale));
                    r = r.push(interactive_cell(vol, cid, pair, modifiers, scale));
                }
            }
            rows = rows.push(r);
        }
    }
    rows
}

pub fn view(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;
    let modifiers = state.modifiers;
    let n_outputs = state.device.outputs().len();

    // ── Column header: individual output channels ("Out 1".."Out 12"),
    // active (accent) when it's the pair currently selected in the
    // Submix picker.
    let mut header = row![
        label_cell(String::new(), GROUP_LABEL_W, theme::TEXT_SEC, scale),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for out in 0..n_outputs {
        header = header.push(header_cell(format!("Out {}", out + 1), out / 2 == state.sel_out, scale));
    }

    let mut rows = column![header].spacing(theme::SPACE_HAIRLINE);

    rows = push_channel_rows(
        rows,
        &input_row_groups(state),
        "In",
        scale,
        modifiers,
        ChannelId::Input,
        |idx| state.device.inputs()[idx].volumes.clone(),
        |idx| (idx < 4).then(|| state.device.inputs()[idx].pans.clone()),
    );
    rows = push_channel_rows(
        rows,
        &playback_row_groups(state),
        "Pb",
        scale,
        modifiers,
        ChannelId::Playback,
        |idx| state.device.playbacks()[idx].volumes.clone(),
        |_| None,
    );

    // ── Master "Outputs" row: each output channel's own master volume,
    // distinct from (and separated by a divider from) the crosspoint
    // matrix above it — also a real, interactive fader-behind-a-number,
    // same as every crosspoint cell.
    let mut out_row = row![
        label_cell("Outputs".into(), GROUP_LABEL_W, theme::TEXT_PRIMARY, scale),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for (idx, out) in state.device.outputs()[..n_outputs].iter().enumerate() {
        out_row = out_row.push(interactive_cell(out.volume, ChannelId::Output(idx), 0, modifiers, scale));
    }
    rows = rows.push(container(iced::widget::Space::new().width(Length::Fill).height(1)).style(
        |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::BORDER)),
            ..container::Style::default()
        },
    ));
    rows = rows.push(out_row);

    // ── Column-group footer: each output pair's bus label
    // (`OUT_LABELS`), spanning both its columns.
    let mut footer = row![
        label_cell(String::new(), GROUP_LABEL_W, theme::TEXT_SEC, scale),
        label_cell(String::new(), IDX_LABEL_W, theme::TEXT_SEC, scale),
    ]
    .spacing(theme::SPACE_HAIRLINE);
    for pair in 0..(n_outputs / 2) {
        let label = OUT_LABELS.get(pair).copied().unwrap_or_default().to_string();
        footer = footer.push(header_cell(label.clone(), pair == state.sel_out, scale));
        footer = footer.push(header_cell(label, pair == state.sel_out, scale));
    }
    rows = rows.push(footer);

    let scroller = scrollable(rows)
        .direction(scrollable::Direction::Both {
            horizontal: theme::thin_scrollbar(),
            vertical: theme::thin_scrollbar(),
        })
        .style(theme::scrollable);

    container(scroller)
        .style(theme::panel)
        .padding(theme::SPACE_LG)
        .into()
}
