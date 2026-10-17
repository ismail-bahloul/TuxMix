//! `tuxmix-gui` — Graphical RME interface controller.
//!
//! ```bash
//! cargo run -p tuxmix-gui              # with real hardware
//! cargo run -p tuxmix-gui -- --mock    # simulation
//! cargo run -p tuxmix-gui -- --mock --osc   # + OSC control surface
//! cargo run -p tuxmix-gui -- --backend usb  # force libusb over the kernel driver
//! cargo run -p tuxmix-gui -- --view mixer  # open straight on the Mixer page
//! ```

mod about;
mod app;
mod icon;
mod layouts;
mod matrix;
mod osc;
pub mod pw_meter;
mod scenes;
mod sidebar;
mod theme;
mod widgets;

use std::net::{IpAddr, Ipv4Addr};

use iced::window;
use iced::{Font, Size};

/// Inter (SIL OFL 1.1, see assets/fonts/Inter-LICENSE) replaces whatever
/// sans-serif the host system happens to default to — a plain-looking UI is
/// the single biggest thing separating this from a "finished" app, for very
/// little effort.
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The application id, matching the basename of `tuxmix.desktop`, so the
/// desktop environment associates the window with the app entry (icon
/// included) in the taskbar and the window list.
const APP_ID: &str = "tuxmix";

/// The window icon, from the same decoded PNG the About dialog draws (see
/// `icon`). `None` on a decode error: a missing icon is cosmetic, not a
/// reason to fail the launch.
fn window_icon() -> Option<window::Icon> {
    let (width, height, pixels) = icon::rgba()?;
    window::icon::from_rgba(pixels, width, height).ok()
}

/// Value following a `--flag <value>` pair, if present.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// `--view quick|mixer|matrix`: the page the window opens on. `Quick` is
/// the default (the landing view), but the Mixer is where the full work
/// happens and where the device panel lives, so the caller can land there
/// directly. Unknown values fall back to the default, the same way
/// `--backend` falls back to auto-detect instead of opening nothing.
fn parse_view(args: &[String]) -> Option<app::View> {
    match arg_value(args, "--view").as_deref() {
        Some("quick") => Some(app::View::Quick),
        Some("mixer") => Some(app::View::Mixer),
        Some("matrix") => Some(app::View::Matrix),
        Some(other) => {
            eprintln!("Unknown --view value {other:?} (use quick|mixer|matrix), starting on Quick");
            None
        }
        None => None,
    }
}

fn main() -> iced::Result {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let mock = args.iter().any(|a| a == "--mock");

    // `None` = auto-detect (ALSA/kernel-driver first, USB fallback — see
    // `DeviceHandle::open_real`). An unrecognized value falls back to
    // auto rather than silently opening nothing.
    let backend = arg_value(&args, "--backend").and_then(|v| match v.as_str() {
        "alsa" | "usb" => Some(v),
        other => {
            eprintln!("Unknown --backend value {other:?} (use alsa|usb), auto-detecting");
            None
        }
    });

    // Opt-in, loopback-only OSC control surface — see osc.rs. Off unless
    // `--osc` is passed, so enabling it is always a deliberate choice.
    let osc_config = args.iter().any(|a| a == "--osc").then(|| osc::OscConfig {
        recv_port: arg_value(&args, "--osc-port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(9000),
        send_port: arg_value(&args, "--osc-send-port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(9001),
        send_host: arg_value(&args, "--osc-host")
            .and_then(|v| v.parse::<IpAddr>().ok())
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    });

    let start_view = parse_view(&args);

    iced::application(
        move || {
            let mut state = app::new(mock, osc_config.clone(), backend.clone());
            if let Some(view) = start_view {
                state.view = view;
            }
            state
        },
        app::update,
        app::view,
    )
    .title(app::title)
    .subscription(app::subscription)
    .theme(|_state: &app::TuxMix| iced::Theme::Dark)
    .font(INTER_REGULAR)
    .default_font(Font::with_name("Inter"))
    .window(window::Settings {
        size: Size::new(1280.0, 800.0),
        icon: window_icon(),
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.to_owned(),
            ..Default::default()
        },
        // No enforced minimum — the adaptive scale (down to `SCALE_MIN`)
        // plus per-row horizontal scroll and the page's vertical scroll
        // handle small windows fine now, so a fixed 960×600 floor is just
        // an outdated restriction on shrinking.
        ..window::Settings::default()
    })
    .run()
}

#[cfg(test)]
mod tests {
    use super::parse_view;
    use crate::app::View;

    fn args_with_view(value: &str) -> Vec<String> {
        vec!["tuxmix-gui".into(), "--view".into(), value.into()]
    }

    #[test]
    fn view_flag_picks_the_starting_page() {
        assert_eq!(parse_view(&args_with_view("quick")), Some(View::Quick));
        assert_eq!(parse_view(&args_with_view("mixer")), Some(View::Mixer));
        assert_eq!(parse_view(&args_with_view("matrix")), Some(View::Matrix));
        assert_eq!(parse_view(&args_with_view("wat")), None);
    }

    #[test]
    fn view_flag_absent_starts_on_the_default() {
        assert_eq!(parse_view(&["tuxmix-gui".into()]), None);
        assert_eq!(parse_view(&["tuxmix-gui".into(), "--mock".into()]), None);
    }
}
