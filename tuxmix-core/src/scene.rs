use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::channel::{InputChannel, OutputChannel, PlaybackChannel};
use crate::device::DeviceSettings;
use crate::error::Error;

/// Directory for saved scenes/presets (~/.local/share/tuxmix/scenes).
pub fn scenes_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home).join(".local/share/tuxmix/scenes");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Path of the SHARED auto-saved state. Both the GUI and the TUI load
/// this on open and save to it on change/exit — the device has no
/// gain/volume readback, so this single file is how the two UIs stay in
/// sync (the last writer wins; saving on exit avoids restoring a stale
/// state).
pub fn auto_scene_path() -> PathBuf {
    scenes_dir().join("auto.json")
}

/// Load the last auto-saved mixer state, if any.
pub fn load_auto_scene() -> Option<Scene> {
    let content = std::fs::read_to_string(auto_scene_path()).ok()?;
    Scene::from_json(&content).ok()
}

/// Save the mixer state to the shared auto file.
pub fn save_auto_scene(scene: &Scene) -> Result<(), String> {
    std::fs::write(
        auto_scene_path(),
        scene.to_json().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Read the shared auto file back WITHOUT re-applying it, returning the
/// raw JSON string (or `None` if absent/unreadable).
pub fn read_auto_scene_raw() -> Option<String> {
    std::fs::read_to_string(auto_scene_path()).ok()
}

/// Detect whether the OTHER UI (GUI ↔ TUI) has written the shared auto
/// file since our last write.
///
/// Both UIs auto-save their in-memory state every few seconds and on
/// exit, and neither has a hardware readback to re-derive the other's
/// changes — so without a check the last UI to save would clobber the
/// other's state with its own (possibly stale) copy. Before saving,
/// call this: if it returns `Some`, re-apply that scene (the other UI's
/// changes) to the device first, THEN save your own capture.
///
/// `our_last_json` = the JSON string of our most recent write (pass the
/// UI's `last_saved_json`). If the file on disk differs, another UI has
/// written since — return the parsed scene.
pub fn auto_scene_written_by_other(our_last_json: Option<&str>) -> Option<Scene> {
    let raw = read_auto_scene_raw()?;
    if our_last_json.is_some_and(|ours| ours == raw) {
        return None; // the file still holds OUR last write
    }
    // Either we never wrote (fresh start, file pre-exists) or another UI
    // wrote after us — reload their state.
    Scene::from_json(&raw).ok()
}

/// A snapshot of the full device state, serializable for
/// save/restore (scenes / presets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    /// Name of the scene (user-defined).
    pub name: String,

    /// The `RmeDevice::model_name()` this scene was captured from.
    /// Empty = legacy scene, captured before this field existed —
    /// treated as "unknown, skip the compatibility check" rather than
    /// a hard mismatch. Applying a scene by blind positional index to
    /// a different model's channel layout would silently write wrong
    /// values, so this is checked before `apply_scene` mutates anything.
    #[serde(default)]
    pub model: String,

    /// Hardware input channels.
    pub inputs: Vec<InputChannel>,

    /// Software playback channels.
    pub playbacks: Vec<PlaybackChannel>,

    /// Physical output channels.
    pub outputs: Vec<OutputChannel>,

    /// Global device-level settings.
    pub settings: DeviceSettings,
}

impl Scene {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            model: String::new(),
            inputs: Vec::new(),
            playbacks: Vec::new(),
            outputs: Vec::new(),
            settings: DeviceSettings {
                clock_source: "Internal".into(),
                clock_sources: Vec::new(),
                spdif_optical: false,
                spdif_emphasis: false,
                spdif_professional: false,
                spdif_enabled: false,
                pitch_percent: 0.0,
                ms_proc: false,
                an12: false,
                dim: false,
                fx_send_db: None,
                width: 0.0,
                sample_rate: 48_000,
            },
        }
    }

    /// Serialize the scene to a JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a scene from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Returns an error if this scene was captured on a different
    /// model than `device_model`. A blank `self.model` (legacy scene,
    /// or one built via `Scene::new`) is always treated as compatible.
    pub fn check_compatible(&self, device_model: &str) -> Result<(), Error> {
        if !self.model.is_empty() && self.model != device_model {
            return Err(Error::SceneModelMismatch {
                scene_model: self.model.clone(),
                device_model: device_model.to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point the shared auto file at a temp path so tests never touch
    /// the real `~/.local/share/tuxmix`. `scenes_dir()` is fixed, so
    /// instead we override the HOME env var for the duration.
    struct TempHome;
    impl TempHome {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("tuxmix-test-{}", std::process::id()));
            std::env::set_var("HOME", &dir);
            let _ = std::fs::remove_dir_all(&dir);
            Self
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            // Restore a sane HOME for the rest of the test process.
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn auto_scene_other_write_detection() {
        let _home = TempHome::new();
        let ours = Scene::new("ours");
        let ours_json = ours.to_json().unwrap();
        let theirs = Scene::new("theirs");

        // No file yet: nothing written by "the other" (nothing to
        // clobber) — but a pre-existing file from a previous run IS
        // "other" until we've written once.
        assert!(auto_scene_written_by_other(None).is_none());

        // We write, then the other UI overwrites with a different scene:
        // our last_json no longer matches the disk → detected.
        save_auto_scene(&ours).unwrap();
        save_auto_scene(&theirs).unwrap();
        let detected = auto_scene_written_by_other(Some(&ours_json));
        assert!(detected.is_some());
        assert_eq!(detected.unwrap().name, "theirs");

        // Disk still holds OUR write → not "other".
        save_auto_scene(&ours).unwrap();
        assert!(auto_scene_written_by_other(Some(&ours_json)).is_none());

        // Our last_json unknown (UI restarted) but disk exists → other.
        save_auto_scene(&theirs).unwrap();
        assert!(auto_scene_written_by_other(None).is_some());
    }
}
