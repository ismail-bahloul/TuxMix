//! The GUI's own preferences — how the window was laid out, not what the
//! mixer is doing (that is the shared auto-saved scene, see
//! `tuxmix_core::scene`). One JSON file in the XDG config directory
//! (`$XDG_CONFIG_HOME/tuxmix/gui.json`, else `~/.config/tuxmix/gui.json`):
//! the open view, the sidebar's state, the mixer row count, the zoom, and
//! the user's channel names, the PipeWire rate / buffer size forced here.
//!
//! Every field is optional on read (`#[serde(default)]`), so a file from
//! an older build, or a hand-edited one missing keys, still loads and
//! leaves the rest at the app's defaults. A file that does not parse at
//! all is ignored the same way: preferences are a convenience, never a
//! reason not to start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tuxmix_core::ChannelId;

use crate::app::View;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub view: Option<View>,
    pub sidebar_open: Option<bool>,
    pub sidebar_width: Option<f32>,
    /// Options, Snapshots, Groups, Mixer Layout, in that order.
    pub panels_open: Option<[bool; 4]>,
    /// Superseded by `row_mode`; still read, so an older file keeps its
    /// choice, and no longer written.
    #[serde(skip_serializing)]
    pub two_row: Option<bool>,
    pub row_mode: Option<crate::app::RowMode>,
    pub ui_scale: Option<f32>,
    /// User channel names, keyed by the strip they were typed on. A list
    /// of pairs rather than a map: JSON object keys must be strings, and
    /// `ChannelId` serializes as a tagged value.
    pub labels: Vec<(ChannelId, String)>,
    /// TotalMix's "Show Names": display the original names even where the
    /// user renamed a channel.
    pub show_original_names: bool,
    /// The PipeWire rate / buffer size forced from the device settings
    /// (`0` = PipeWire's own), applied again at launch.
    pub engine_force_rate: Option<u32>,
    /// Logical size of the window at the last close.
    pub window_size: Option<(f32, f32)>,
    /// Closing the window hides it in the tray instead of quitting.
    pub keep_in_tray: Option<bool>,
    pub engine_force_quantum: Option<u32>,
}

/// Where the preferences live. `None` only if neither `XDG_CONFIG_HOME`
/// nor `HOME` is set, in which case nothing is persisted.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tuxmix").join("gui.json"))
}

pub fn load(path: &Path) -> Prefs {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, prefs: &Prefs) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    // Write-then-rename, so a crash mid-write never leaves a truncated
    // file behind (which `load` would then silently discard).
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_file_loads_with_defaults_for_the_rest() {
        let p: Prefs = serde_json::from_str(r#"{ "two_row": true }"#).unwrap();
        assert_eq!(p.two_row, Some(true));
        assert_eq!(p.view, None);
        assert!(p.labels.is_empty());
    }

    #[test]
    fn prefs_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("tuxmix-prefs-test-{}", std::process::id()));
        let file = dir.join("gui.json");
        let prefs = Prefs {
            view: Some(View::Matrix),
            sidebar_open: Some(false),
            sidebar_width: Some(260.0),
            panels_open: Some([true, false, true, false]),
            two_row: None,
            row_mode: Some(crate::app::RowMode::Two),
            ui_scale: Some(1.25),
            labels: vec![(ChannelId::Input(0), "Vocal".into())],
            show_original_names: true,
            engine_force_rate: Some(96_000),
            window_size: Some((1600.0, 900.0)),
            keep_in_tray: Some(false),
            engine_force_quantum: Some(128),
        };
        save(&file, &prefs).unwrap();
        assert_eq!(load(&file), prefs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("tuxmix-prefs-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("gui.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load(&file), Prefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
