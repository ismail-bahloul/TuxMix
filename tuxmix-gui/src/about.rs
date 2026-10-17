//! The About dialog: what the app is, which version of it is running, and
//! which device it is driving. Opened from the brand lockup in the top bar
//! (the mark + wordmark, see `app::top_bar`), it draws as a centered modal
//! above the whole window (see `about_modal`) rather than the docked drawer
//! it used to be: that drawer was pushed after the page content, so on the
//! Quick view it landed below the fold and on the Mixer view it came after
//! every strip row.

use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{Alignment, Background, Color, Element, Length};

use tuxmix_core::RmeDevice;

use crate::app::{Message, TuxMix};
use crate::theme;

/// Where the source and the releases live: the same URL `tuxmix.desktop`
/// carries as its homepage, and the one the AUR package points at.
pub const REPO_URL: &str = "https://github.com/ismail-bahloul/TuxMix";

/// The card's width cap at `ui_scale == 1.0`: wide enough for the repo URL
/// on one line, narrow enough to read as a dialog, and capped rather than
/// fixed so a narrow window shrinks it instead of clipping it.
const CARD_W: f32 = 420.0;

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
    .align_y(Alignment::Center)
    .into()
}

/// The whole overlay: a dimming scrim that closes on click and blocks the
/// window behind it, with the card centered above it. Both layers are
/// `opaque`, so a click inside the card (its padding included) never falls
/// through to the scrim and closes the dialog under the cursor.
pub fn about_modal(state: &TuxMix) -> Element<'_, Message> {
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
        .align_y(Alignment::Center),
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

    // The mark beside the identity block: the same asset the window icon
    // wears, decoded by the crate's own `png` decoder and shown through
    // iced's codec-free image widget (see `icon::mark`).
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
            .align_y(Alignment::Center)
            .into()
        }
        None => identity.into(),
    };

    let card = opaque(
        container(
            column![header(scale), identity_row]
                .spacing(theme::SPACE_LG * scale)
                .width(Length::Fill),
        )
        .style(theme::dialog)
        .padding(theme::SPACE_XL * scale)
        .width(Length::Fill)
        .max_width(CARD_W * scale),
    );

    // The scrim is the invisible half of the modal: dim it, catch the
    // click that closes the dialog, and keep every event from reaching
    // the window underneath.
    let scrim = opaque(
        mouse_area(
            container(Space::new().width(Length::Fill).height(Length::Fill)).style(
                |_theme: &iced::Theme| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.55,
                        ..Color::BLACK
                    })),
                    ..container::Style::default()
                },
            ),
        )
        .on_press(Message::ToggleAbout),
    );

    stack(vec![
        scrim,
        container(card)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
