//! Mixer Layout (collapsed-strip set) persistence — mirrors `scenes.rs`'s
//! own file-per-slot pattern, one JSON file per numbered slot (1-6),
//! sibling directory to the scenes one. A layout is just `TuxMix::
//! collapsed` (which strips are collapsed) — there's no strip-reordering
//! yet, so that's the whole story; `ChannelId` already derives `Serialize`/
//! `Deserialize` (`tuxmix_core::channel`), so this is plain serde, no
//! bespoke format.

use std::collections::HashSet;
use std::path::PathBuf;

use tuxmix_core::ChannelId;

fn layouts_dir() -> PathBuf {
    let dir = crate::scenes::scenes_dir()
        .parent()
        .map(|p| p.join("layouts"))
        .unwrap_or_else(|| PathBuf::from("layouts"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn load_layout_file(slot: u8) -> Option<HashSet<ChannelId>> {
    let content =
        std::fs::read_to_string(layouts_dir().join(format!("Layout {slot}.json"))).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn save_layout_file(slot: u8, collapsed: &HashSet<ChannelId>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(collapsed).map_err(|e| e.to_string())?;
    std::fs::write(layouts_dir().join(format!("Layout {slot}.json")), json)
        .map_err(|e| e.to_string())
}
