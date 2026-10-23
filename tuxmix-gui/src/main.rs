//! `tuxmix-gui` — Graphical RME interface controller.
//!
//! ```bash
//! cargo run -p tuxmix-gui              # with real hardware
//! cargo run -p tuxmix-gui -- --mock    # simulation
//! cargo run -p tuxmix-gui -- --mock --osc   # + OSC control surface
//! cargo run -p tuxmix-gui -- --backend usb  # force libusb over the kernel driver
//! cargo run -p tuxmix-gui -- --view mixer  # open straight on the Mixer page
//! cargo run -p tuxmix-gui -- --fullscreen  # start fullscreen (F11 toggles)
//! ```

mod about;
mod app;
mod icon;
mod layouts;
mod matrix;
mod osc;
mod prefs;
mod scenes;
mod sidebar;
mod theme;
mod tray;
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
    let saved_size = prefs::path()
        .and_then(|p| prefs::load(&p).window_size)
        .map(|(w, h)| Size::new(w.max(app::MIN_WINDOW.0), h.max(app::MIN_WINDOW.1)))
        .unwrap_or(Size::new(1280.0, 800.0));
    let start_fullscreen = args.iter().any(|a| a == "--fullscreen");

    // One TuxMix per session: a second launch asks the first to show its
    // window (it may be hidden in the tray) and leaves.
    if matches!(tray::claim(mock), tray::Instance::Secondary) {
        eprintln!("TuxMix is already running: asked it to show its window.");
        return Ok(());
    }
    // The tray icon (not for the simulation, which would put a second icon
    // beside the real one). `--minimized` starts hidden in it, and only when
    // there is one: a hidden window with no icon has no way back.
    let tray_active = !mock && tray::start(tray::Status { window_visible: true, dim: false });
    let start_hidden = tray_active && args.iter().any(|a| a == "--minimized");

    // A daemon, not a plain application: the window is opened and closed by
    // the app. Hiding it in the tray means closing it while the process
    // lives on (winit cannot hide a window on Wayland, and an application
    // exits with its last window).
    iced::daemon(
        move || {
            let mut state = app::new(mock, osc_config.clone(), backend.clone());
            // Restore the window's own layout (view, sidebar, rows, zoom,
            // channel names) before the command line overrides any of it.
            if let Some(path) = prefs::path() {
                app::apply_prefs(&mut state, prefs::load(&path));
                state.prefs_path = Some(path);
            }
            app::restore_engine(&mut state);
            if let Some(view) = start_view {
                state.view = view;
            }
            state.fullscreen = start_fullscreen;
            state.tray_active = tray_active;
            state.window_hidden = start_hidden;
            if start_hidden {
                return (state, iced::Task::none());
            }
            let (id, open) = window::open(window_settings(saved_size, start_fullscreen));
            state.window_id = Some(id);
            // First launch only (no saved zoom): ask the monitor's size to
            // pick a starting zoom.
            let task = if state.initial_zoom_pending {
                open.then(move |_| window::monitor_size(id))
                    .map(|size| app::Message::MonitorSize(size))
            } else {
                open.discard()
            };
            (state, task)
        },
        app::update,
        app::view,
    )
    .title(app::title)
    .subscription(app::subscription)
    .theme(|_state: &app::TuxMix, _id: window::Id| iced::Theme::Dark)
    .font(INTER_REGULAR)
    .default_font(Font::with_name("Inter"))
    .run()
}

/// The main window: `size` floored at the minimum, the app's icon and id.
pub fn window_settings(size: Size, fullscreen: bool) -> window::Settings {
    window::Settings {
        // The close button is handled by the app (`Message::CloseRequested`):
        // hide in the tray, or save and quit.
        exit_on_close_request: false,
        size,
        // See `app::MIN_WINDOW`.
        min_size: Some(Size::new(app::MIN_WINDOW.0, app::MIN_WINDOW.1)),
        fullscreen,
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
    }
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
