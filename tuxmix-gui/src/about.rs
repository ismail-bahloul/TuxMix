//! The About drawer: what the app is, which version of it is running, and
//! where the source lives. Same docked-drawer pattern as the device settings
//! and the OSC log (see `device_panel`/`osc_log_panel` in `app`), toggled
//! from the top bar, rather than a second window: this GUI has exactly one
//! window everywhere else.

use iced::widget::{button, column, container, row, text, Space};
use iced::{Element, Length};

use tuxmix_core::RmeDevice;

use crate::app::{Message, TuxMix};
use crate::theme;

/// Where the source and the releases live: the same URL `tuxmix.desktop`
/// carries as its homepage, and the one the AUR package points at.
pub const REPO_URL: &str = "https://github.com/ismail-bahloul/TuxMix";

fn header(scale: f32) -> Element<'static, Message> {
    row![
        text("ABOUT")
            .color(theme::TEXT_PRIMARY)
            .size(theme::TEXT_SM * scale),
        Space::new().width(Length::Fill),
        button(text("Close").size(theme::TEXT_XS * scale))
            .padding([theme::SPACE_SM * scale, theme::SPACE_MD * scale])
            .style(theme::plain_button)
            .on_press(Message::ToggleAbout),
    ]
    .spacing(theme::SPACE_MD * scale)
    .align_y(iced::Alignment::Center)
    .into()
}

pub fn about_panel(state: &TuxMix) -> Element<'_, Message> {
    let scale = state.ui_scale;

    // Which build, and which device it is driving right now, in which
    // state (the same four words the sidebar chip uses, disconnect
    // included). Between them they are most of what a bug report needs
    // to start with, and the sidebar chip that says the same thing
    // scrolls away with the mixer. The mock suffix goes: the word right
    // after it already says it, and "(mock) (simulated)" reads like a
    // bug of its own.
    let model = state
        .device
        .model_name()
        .strip_suffix(" (mock)")
        .unwrap_or_else(|| state.device.model_name());
    let device = format!(
        "Driving {model} ({})",
        if !state.connected {
            "disconnected"
        } else if state.device.is_mock() {
            "simulated"
        } else if !state.device.protocol_features() {
            "class-compliant"
        } else {
            "proprietary"
        },
    );

    let identity = column![
        row![
            text("TuxMix")
                .color(theme::TEXT_PRIMARY)
                .size(theme::TEXT_XL * scale),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .color(theme::TEXT_SEC)
                .size(theme::TEXT_MD * scale),
        ]
        .spacing(theme::SPACE_LG)
        .align_y(iced::Alignment::Center),
        text("Open-source TotalMix replacement for RME audio interfaces")
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        text(device)
            .color(theme::TEXT_SEC)
            .size(theme::TEXT_XS * scale),
        button(
            text(REPO_URL)
                .color(theme::ACCENT)
                .size(theme::TEXT_XS * scale)
        )
        .padding([theme::SPACE_TIGHT * scale, theme::SPACE_SM * scale])
        .style(theme::plain_button)
        .on_press(Message::OpenRepoUrl),
    ]
    .spacing(theme::SPACE_SM * scale);

    // The mark next to the identity block, the one place in the app where
    // the product gets to show its face (the working views stay clean).
    // Same asset the window icon wears, decoded by the crate's own `png`
    // decoder and shown through iced's codec-free image widget: the full
    // `image` feature would pull ~79 crates of decoders for one
    // decoration. A missing asset just drops the mark.
    let identity_row: Element<'_, Message> = match crate::icon::mark() {
        Some((handle, ratio)) => {
            let mark_h = 44.0 * scale;
            row![
                iced::widget::image(handle)
                    .width(mark_h * ratio)
                    .height(mark_h)
                    .filter_method(iced::widget::image::FilterMethod::Linear),
                identity,
            ]
            .spacing(theme::SPACE_XL * scale)
            .align_y(iced::Alignment::Center)
            .into()
        }
        None => identity.into(),
    };

    // Sized by its content, like the device settings drawer: every line here
    // is static, so there is nothing to settle and no reason for a fixed
    // height that would have to be re-tuned by eye at every type size.
    container(
        column![header(scale), identity_row]
            .spacing(theme::SPACE_LG * scale)
            .width(Length::Fill),
    )
    .style(theme::top_bar)
    .padding(theme::SPACE_MD * scale)
    .width(Length::Fill)
    .into()
}
