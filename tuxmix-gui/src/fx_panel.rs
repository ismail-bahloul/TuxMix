// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Ismaïl Bahloul

//! The sidebar's FX panel: TotalMix FX's reverb and echo, which the
//! Babyface Pro FS leaves to the host (see `tuxmix_core::host_fx`).
//!
//! The panel edits `TuxMix::host_fx`. A change is applied a moment after
//! the last one (`APPLY_AFTER`), because applying restarts the filter: a
//! knob drag would otherwise restart it on every step.

use std::time::{Duration, Instant};

use iced::widget::{button, column, pick_list, row, text};
use iced::{Element, Length};
use tuxmix_core::host_fx::{EchoParams, EchoType, HostFx, ReverbParams, ReverbType};
use tuxmix_core::RmeDevice;

use crate::app::{Message, TuxMix};
use crate::theme;
use crate::widgets::knob::{knob, Knob};

/// How long after the last change the filter restarts with it.
pub const APPLY_AFTER: Duration = Duration::from_millis(250);
/// How often a running filter is checked: it exits with PipeWire, and comes
/// back once PipeWire does.
pub const CHECK_EVERY: Duration = Duration::from_secs(2);

/// One change from the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FxEdit {
    ReverbOn(bool),
    ReverbKind(ReverbType),
    PreDelay(f32),
    RoomScale(f32),
    ReverbLowCut(f32),
    ReverbHighCut(f32),
    Smooth(f32),
    ReverbWidth(f32),
    ReverbVolume(f32),
    EchoOn(bool),
    EchoKind(EchoType),
    Delay(f32),
    Bpm(f32),
    Feedback(f32),
    /// The top of the knob is "off".
    EchoHighCut(f32),
    EchoWidth(f32),
    EchoVolume(f32),
}

/// The echo high cut knob's top: past this it reads "off".
const ECHO_CUT_OFF: f32 = 20_000.0;

/// Applies `e` to `fx`, within each setting's range.
pub fn edit(fx: &mut HostFx, e: FxEdit) {
    let (r, x) = (&mut fx.reverb, &mut fx.echo);
    match e {
        FxEdit::ReverbOn(on) => r.enabled = on,
        FxEdit::ReverbKind(k) => r.kind = k,
        FxEdit::PreDelay(v) => r.pre_delay_ms = v.clamp(0.0, 999.0).round(),
        FxEdit::RoomScale(v) => r.room_scale = (v.clamp(0.5, 3.0) * 100.0).round() / 100.0,
        FxEdit::ReverbLowCut(v) => r.low_cut_hz = v.clamp(20.0, 500.0).round(),
        FxEdit::ReverbHighCut(v) => r.high_cut_hz = (v.clamp(5_000.0, 20_000.0) / 100.0).round() * 100.0,
        FxEdit::Smooth(v) => r.smooth = (v.clamp(0.0, 1.0) * 100.0).round() / 100.0,
        FxEdit::ReverbWidth(v) => r.width = (v.clamp(-1.0, 1.0) * 100.0).round() / 100.0,
        FxEdit::ReverbVolume(v) => r.volume_db = (v.clamp(-65.0, 6.0) * 2.0).round() / 2.0,
        FxEdit::EchoOn(on) => x.enabled = on,
        FxEdit::EchoKind(k) => x.kind = k,
        FxEdit::Delay(v) => x.delay_s = (v.clamp(0.01, 2.0) * 100.0).round() / 100.0,
        FxEdit::Bpm(v) => x.set_bpm(v.clamp(30.0, 300.0).round()),
        FxEdit::Feedback(v) => x.feedback = (v.clamp(0.0, 0.99) * 100.0).round() / 100.0,
        FxEdit::EchoHighCut(v) => {
            x.high_cut_hz = (v < ECHO_CUT_OFF).then(|| (v.clamp(1_000.0, ECHO_CUT_OFF) / 100.0).round() * 100.0)
        }
        FxEdit::EchoWidth(v) => x.width = (v.clamp(-1.0, 1.0) * 100.0).round() / 100.0,
        FxEdit::EchoVolume(v) => x.volume_db = (v.clamp(-65.0, 6.0) * 2.0).round() / 2.0,
    }
}

/// Turns the effect on, off or to new settings on the card: the driver's
/// Host FX Switch and the filter. Errors land in `fx_status`.
pub fn apply(state: &mut TuxMix) {
    state.fx_dirty = None;
    state.fx_checked = Some(Instant::now());
    if !state.device.has_host_fx() {
        return;
    }
    let fx = state.host_fx;
    let Some(runner) = state.fx_runner.as_mut() else {
        // The simulation: nothing to run.
        return;
    };
    match runner.apply(&fx, None) {
        Ok(()) => {
            state.fx_status = None;
            if let Err(e) = state.device.set_host_fx(fx.active()) {
                state.fx_status = Some(e.to_string());
            }
        }
        Err(e) => {
            state.fx_status = Some(e.to_string());
            let _ = state.device.set_host_fx(false);
        }
    }
}

/// TuxMix is quitting: stops the filter and the card's FX return. The
/// filter would also die with TuxMix (see `Runner`), a moment later.
pub fn shutdown(state: &mut TuxMix) {
    if let Some(runner) = state.fx_runner.as_mut() {
        if runner.running().is_some() {
            runner.stop();
            let _ = state.device.set_host_fx(false);
        }
    }
}

/// The tick's part: applies a change once the knob has settled, and starts
/// the filter again if it exited.
pub fn tick(state: &mut TuxMix) {
    if state.fx_dirty.is_some_and(|t| t.elapsed() >= APPLY_AFTER) {
        apply(state);
        return;
    }
    let due = state.fx_checked.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
    if due && state.host_fx.active() {
        state.fx_checked = Some(Instant::now());
        let dead = state.fx_runner.as_mut().is_some_and(|r| !r.alive());
        if dead {
            apply(state);
        }
    }
}

fn fx_knob<'a>(
    state: &TuxMix,
    caption: &'a str,
    value: f32,
    range: (f32, f32),
    label: String,
    log_scale: bool,
    centered: bool,
    on: fn(f32) -> FxEdit,
    reset: FxEdit,
    scale: f32,
) -> Element<'a, Message> {
    column![
        knob(Knob {
            tint: None,
            value,
            range,
            label,
            arc_from_center: centered,
            interactive: true,
            log_scale,
            modifiers: state.modifiers,
            scale,
            on_change: Box::new(move |v| Message::HostFx(on(v))),
            on_reset: Box::new(move || Message::HostFx(reset)),
        }),
        text(caption).size(theme::TEXT_XS * scale).color(theme::TEXT_SEC),
    ]
    .spacing(theme::SPACE_TIGHT * scale)
    .align_x(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn knob_row<'a>(cells: Vec<Element<'a, Message>>, scale: f32) -> Element<'a, Message> {
    let mut r = row![].spacing(theme::SPACE_SM * scale).width(Length::Fill);
    let n = cells.len();
    for c in cells {
        r = r.push(c);
    }
    // Short rows keep the grid's columns.
    for _ in n..3 {
        r = r.push(iced::widget::Space::new().width(Length::Fill));
    }
    r.into()
}

/// The on/off button and the type list of one effect.
fn effect_head<'a, T>(
    name: &'a str,
    on: bool,
    toggle: FxEdit,
    kinds: &'a [T],
    kind: T,
    pick: fn(T) -> FxEdit,
    scale: f32,
) -> Element<'a, Message>
where
    T: Clone + PartialEq + std::fmt::Display + 'a,
{
    row![
        button(text(name).size(theme::TEXT_XS * scale).center())
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .width(Length::Fixed(64.0 * scale))
            .style(theme::toggle_button(on, theme::ACCENT))
            .on_press(Message::HostFx(toggle)),
        pick_list(kinds, Some(kind), move |k| Message::HostFx(pick(k)))
            .style(theme::pick_list)
            .menu_style(theme::menu)
            .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
            .text_size(theme::TEXT_XS * scale)
            .width(Length::Fill),
    ]
    .spacing(theme::SPACE_SM * scale)
    .align_y(iced::Alignment::Center)
    .into()
}

/// For the type lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kind<T>(pub T);

impl std::fmt::Display for Kind<ReverbType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

impl std::fmt::Display for Kind<EchoType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

const REVERB_KINDS: [Kind<ReverbType>; 7] = [
    Kind(ReverbType::SmallRoom),
    Kind(ReverbType::MediumRoom),
    Kind(ReverbType::LargeRoom),
    Kind(ReverbType::Hall),
    Kind(ReverbType::Plate),
    Kind(ReverbType::Space),
    Kind(ReverbType::Gated),
];

const ECHO_KINDS: [Kind<EchoType>; 3] = [
    Kind(EchoType::StereoEcho),
    Kind(EchoType::StereoCross),
    Kind(EchoType::PongEcho),
];

fn hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.1}k", v / 1000.0)
    } else {
        format!("{v:.0}")
    }
}

/// The panel body (the header is the sidebar's).
pub fn body(state: &TuxMix, scale: f32) -> Element<'_, Message> {
    let fx = &state.host_fx;
    let (r, x) = (&fx.reverb, &fx.echo);
    let d = (ReverbParams::default(), EchoParams::default());
    let mut col = column![].spacing(theme::SPACE_SM * scale).width(Length::Fill);

    col = col.push(effect_head(
        "reverb",
        r.enabled,
        FxEdit::ReverbOn(!r.enabled),
        &REVERB_KINDS,
        Kind(r.kind),
        |k: Kind<ReverbType>| FxEdit::ReverbKind(k.0),
        scale,
    ));
    col = col.push(knob_row(
        vec![
            fx_knob(state, "pre delay", r.pre_delay_ms, (0.0, 999.0), format!("{:.0}", r.pre_delay_ms), false, false, FxEdit::PreDelay, FxEdit::PreDelay(d.0.pre_delay_ms), scale),
            fx_knob(state, "room scale", r.room_scale, (0.5, 3.0), format!("{:.2}", r.room_scale), false, false, FxEdit::RoomScale, FxEdit::RoomScale(d.0.room_scale), scale),
            fx_knob(state, "smooth", r.smooth, (0.0, 1.0), format!("{:.0}%", r.smooth * 100.0), false, false, FxEdit::Smooth, FxEdit::Smooth(d.0.smooth), scale),
        ],
        scale,
    ));
    col = col.push(knob_row(
        vec![
            fx_knob(state, "low cut", r.low_cut_hz, (20.0, 500.0), hz(r.low_cut_hz), true, false, FxEdit::ReverbLowCut, FxEdit::ReverbLowCut(d.0.low_cut_hz), scale),
            fx_knob(state, "high cut", r.high_cut_hz, (5_000.0, 20_000.0), hz(r.high_cut_hz), true, false, FxEdit::ReverbHighCut, FxEdit::ReverbHighCut(d.0.high_cut_hz), scale),
            fx_knob(state, "width", r.width, (-1.0, 1.0), format!("{:+.2}", r.width), false, true, FxEdit::ReverbWidth, FxEdit::ReverbWidth(d.0.width), scale),
        ],
        scale,
    ));
    col = col.push(knob_row(
        vec![fx_knob(state, "volume", r.volume_db, (-65.0, 6.0), format!("{:.1}", r.volume_db), false, false, FxEdit::ReverbVolume, FxEdit::ReverbVolume(d.0.volume_db), scale)],
        scale,
    ));

    col = col.push(iced::widget::rule::horizontal(1));
    col = col.push(effect_head(
        "echo",
        x.enabled,
        FxEdit::EchoOn(!x.enabled),
        &ECHO_KINDS,
        Kind(x.kind),
        |k: Kind<EchoType>| FxEdit::EchoKind(k.0),
        scale,
    ));
    let cut = x.high_cut_hz.unwrap_or(ECHO_CUT_OFF);
    col = col.push(knob_row(
        vec![
            fx_knob(state, "delay", x.delay_s, (0.01, 2.0), format!("{:.2}", x.delay_s), false, false, FxEdit::Delay, FxEdit::Delay(d.1.delay_s), scale),
            fx_knob(state, "bpm", x.bpm(), (30.0, 300.0), format!("{:.0}", x.bpm()), false, false, FxEdit::Bpm, FxEdit::Delay(d.1.delay_s), scale),
            fx_knob(state, "feedback", x.feedback, (0.0, 0.99), format!("{:.0}%", x.feedback * 100.0), false, false, FxEdit::Feedback, FxEdit::Feedback(d.1.feedback), scale),
        ],
        scale,
    ));
    col = col.push(knob_row(
        vec![
            fx_knob(
                state,
                "high cut",
                cut,
                (1_000.0, ECHO_CUT_OFF),
                x.high_cut_hz.map_or("off".into(), hz),
                true,
                false,
                FxEdit::EchoHighCut,
                FxEdit::EchoHighCut(ECHO_CUT_OFF),
                scale,
            ),
            fx_knob(state, "width", x.width, (-1.0, 1.0), format!("{:+.2}", x.width), false, true, FxEdit::EchoWidth, FxEdit::EchoWidth(d.1.width), scale),
            fx_knob(state, "volume", x.volume_db, (-65.0, 6.0), format!("{:.1}", x.volume_db), false, false, FxEdit::EchoVolume, FxEdit::EchoVolume(d.1.volume_db), scale),
        ],
        scale,
    ));

    // Why nothing is heard, when that is the case.
    let note = if !state.device.has_host_fx() {
        Some("Needs kernel driver 0.2.5".to_string())
    } else if state.fx_runner.is_none() && fx.active() {
        Some("Simulation: no audio runs".to_string())
    } else {
        state.fx_status.clone()
    };
    if let Some(n) = note {
        col = col.push(text(n).size(theme::TEXT_XS * scale).color(theme::TEXT_SEC));
    }
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_stay_in_range_and_on_the_grid() {
        let mut fx = HostFx::default();
        edit(&mut fx, FxEdit::PreDelay(5000.0));
        assert_eq!(fx.reverb.pre_delay_ms, 999.0);
        edit(&mut fx, FxEdit::RoomScale(1.234));
        assert_eq!(fx.reverb.room_scale, 1.23);
        edit(&mut fx, FxEdit::ReverbVolume(-3.3));
        assert_eq!(fx.reverb.volume_db, -3.5);
        edit(&mut fx, FxEdit::Feedback(2.0));
        assert_eq!(fx.echo.feedback, 0.99);
        edit(&mut fx, FxEdit::Bpm(120.0));
        assert!((fx.echo.delay_s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_echo_high_cut_is_off_at_the_top() {
        let mut fx = HostFx::default();
        edit(&mut fx, FxEdit::EchoHighCut(4_321.0));
        assert_eq!(fx.echo.high_cut_hz, Some(4_300.0));
        edit(&mut fx, FxEdit::EchoHighCut(ECHO_CUT_OFF));
        assert_eq!(fx.echo.high_cut_hz, None);
    }

}
