//! Background mode: the notification-area icon, one instance per session,
//! and starting with the session.
//!
//! Why this exists: some of what the card does is TuxMix's job while it is
//! running and nobody's otherwise. The kernel driver no longer toggles the
//! front-panel DIM button itself (it reports the presses, `DIM Button Press
//! Count`, and leaves the action to the mixer application), and TuxMix
//! re-applies the saved mix when the card comes back after a suspend or a
//! replug. Closing the window used to stop all of it silently. With the icon,
//! closing the window can hide it instead, as TotalMix does on Windows: the
//! app keeps running, and stays one click away.
//!
//! The icon is a StatusNotifierItem (`ksni`, over D-Bus): KDE Plasma and most
//! desktops with a tray speak it, GNOME needs an extension. When no service
//! answers, `start` says so and the app behaves as before (closing quits), so
//! a hidden window is never left without a way back.
//!
//! The icon's events reach the UI through one channel (`subscription`), the
//! same way a second launch does: it connects to this instance's socket,
//! asks it to show its window and exits (`claim`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use iced::futures::{stream::BoxStream, StreamExt};
use iced::Subscription;

/// What the icon (or a second launch) asks of the running app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Bring the window up.
    Show,
    /// The icon was clicked: show a hidden window, hide a shown one.
    Toggle,
    /// Show the window with the TuxMix settings open.
    Settings,
    /// Flip the card's Dim.
    Dim,
    /// Recall "Mix n" (1-8).
    Snapshot(u8),
    Quit,
}

type Channel = (
    UnboundedSender<TrayEvent>,
    Mutex<Option<UnboundedReceiver<TrayEvent>>>,
);

fn channel() -> &'static Channel {
    static CHANNEL: OnceLock<Channel> = OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (tx, rx) = unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

fn send(event: TrayEvent) {
    let _ = channel().0.unbounded_send(event);
}

fn events() -> BoxStream<'static, TrayEvent> {
    match channel().1.lock().ok().and_then(|mut rx| rx.take()) {
        Some(rx) => rx.boxed(),
        None => iced::futures::stream::pending().boxed(),
    }
}

/// The icon's and the second launch's events, as an iced subscription.
pub fn subscription() -> Subscription<TrayEvent> {
    Subscription::run(events)
}

// ── The icon ─────────────────────────────────────────────────────

/// What the menu shows: kept here, refreshed by `set_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub window_visible: bool,
    pub dim: bool,
}

struct Icon {
    status: Status,
}

impl ksni::Tray for Icon {
    fn id(&self) -> String {
        "tuxmix".into()
    }

    fn title(&self) -> String {
        "TuxMix".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        crate::icon::tray_rgba()
            .into_iter()
            .map(|(width, height, rgba)| ksni::Icon {
                width: width as i32,
                height: height as i32,
                data: argb_network_order(&rgba),
            })
            .collect()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
        let snapshots = (1..=8u8)
            .map(|n| {
                StandardItem {
                    label: format!("Mix {n}"),
                    activate: Box::new(move |_| send(TrayEvent::Snapshot(n))),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        vec![
            StandardItem {
                label: if self.status.window_visible {
                    "Hide window".into()
                } else {
                    "Show TuxMix".into()
                },
                activate: Box::new(|_| send(TrayEvent::Toggle)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            CheckmarkItem {
                label: "Dim".into(),
                checked: self.status.dim,
                activate: Box::new(|_| send(TrayEvent::Dim)),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Recall mix".into(),
                submenu: snapshots,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings".into(),
                activate: Box::new(|_| send(TrayEvent::Settings)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// RGBA8 to the ARGB32, network byte order (bytes A, R, G, B) that the
/// StatusNotifierItem icon format wants.
fn argb_network_order(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect()
}

fn handle_cell() -> &'static OnceLock<ksni::blocking::Handle<Icon>> {
    static HANDLE: OnceLock<ksni::blocking::Handle<Icon>> = OnceLock::new();
    &HANDLE
}

/// Refreshes the menu's "Hide / Show" label and the Dim check mark. Cheap
/// when nothing changed: the caller may call it every tick.
pub fn set_status(status: Status) {
    static LAST: Mutex<Option<Status>> = Mutex::new(None);
    let Some(handle) = handle_cell().get() else {
        return;
    };
    let Ok(mut last) = LAST.lock() else { return };
    if *last != Some(status) {
        *last = Some(status);
        handle.update(|icon| icon.status = status);
    }
}

/// Puts the icon in the notification area. `false` when no service is
/// listening (no tray, or none that speaks StatusNotifierItem): then the
/// app must not hide its window, there would be no way back.
pub fn start(status: Status) -> bool {
    use ksni::blocking::TrayMethods;
    match (Icon { status }).spawn() {
        Ok(handle) => {
            // The service runs while the handle lives: keep it for the whole
            // process.
            let _ = handle_cell().set(handle);
            true
        }
        Err(e) => {
            log::info!("No notification area ({e}): closing the window quits");
            false
        }
    }
}

// ── One instance per session ─────────────────────────────────────

/// Whether this process is the session's TuxMix, or a second launch that
/// already told the first one to show itself.
pub enum Instance {
    Primary,
    Secondary,
}

fn socket_path(mock: bool) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // The simulation has its own, so a `--mock` window beside the real one
    // (or a test run) never answers for it.
    dir.join(if mock { "tuxmix-mock.sock" } else { "tuxmix.sock" })
}

/// Claims the session's single instance. A second launch connects to the
/// first one's socket, asks it to show its window and reports `Secondary`
/// (the caller exits). A stale socket left by a crash refuses the
/// connection and is taken over.
pub fn claim(mock: bool) -> Instance {
    if std::env::var_os("TUXMIX_MULTIPLE_INSTANCES").is_some() {
        return Instance::Primary;
    }
    let path = socket_path(mock);
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(b"show\n");
        return Instance::Secondary;
    }
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => {
            std::thread::Builder::new()
                .name("tuxmix-instance".into())
                .spawn(move || serve(listener))
                .ok();
            Instance::Primary
        }
        // Could not bind (read-only runtime dir...): run without the
        // guarantee rather than not at all.
        Err(_) => Instance::Primary,
    }
}

fn serve(listener: UnixListener) {
    for stream in listener.incoming().flatten() {
        let mut line = String::new();
        if BufReader::new(stream).read_line(&mut line).is_ok() && line.trim() == "show" {
            send(TrayEvent::Show);
        }
    }
}

// ── Starting with the session ────────────────────────────────────

fn autostart_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart").join("tuxmix.desktop"))
}

/// Whether the session starts TuxMix (the user's own autostart entry).
pub fn autostart_enabled() -> bool {
    autostart_path().is_some_and(|p| p.exists())
}

/// The autostart entry: starts TuxMix hidden in the tray (`--minimized`).
fn autostart_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=TuxMix\n\
         Comment=Mixer for RME interfaces, kept in the tray\n\
         Exec=\"{exe}\" --minimized\nIcon=tuxmix\nTerminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Turns starting with the session on or off.
pub fn set_autostart(on: bool) -> std::io::Result<()> {
    let Some(path) = autostart_path() else {
        return Ok(());
    };
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, autostart_entry(&exe.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_converted_to_argb_in_network_byte_order() {
        // One opaque red pixel and one transparent one.
        let rgba = [255, 0, 0, 255, 10, 20, 30, 0];
        assert_eq!(argb_network_order(&rgba), vec![255, 255, 0, 0, 0, 10, 20, 30]);
    }

    #[test]
    fn the_autostart_entry_starts_hidden_and_quotes_the_path() {
        let entry = autostart_entry("/opt/my apps/tuxmix");
        assert!(entry.contains("Exec=\"/opt/my apps/tuxmix\" --minimized"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn a_second_launch_asks_the_first_to_show_and_a_stale_socket_is_taken_over() {
        let dir = std::env::temp_dir().join(format!("tuxmix-sock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");

        // A stale file (nobody listening) refuses the connection.
        std::fs::write(&path, b"").unwrap();
        assert!(UnixStream::connect(&path).is_err());
        let _ = std::fs::remove_file(&path);

        // A live listener answers "show".
        let listener = UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            line
        });
        let mut s = UnixStream::connect(&path).unwrap();
        s.write_all(b"show\n").unwrap();
        assert_eq!(t.join().unwrap().trim(), "show");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
