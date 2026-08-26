//! Scene (preset) file I/O — ported verbatim from v1.

use std::path::PathBuf;
use tuxmix_core::Scene;

pub fn scenes_dir() -> PathBuf {
    tuxmix_core::scene::scenes_dir()
}

pub fn list_scene_files() -> Vec<String> {
    let dir = scenes_dir();
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str() {
                if n.ends_with(".json") {
                    names.push(n[..n.len() - 5].to_string());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn load_scene_file(name: &str) -> Option<Scene> {
    let content = std::fs::read_to_string(scenes_dir().join(format!("{}.json", name))).ok()?;
    Scene::from_json(&content).ok()
}

pub fn save_scene_file(name: &str, scene: &Scene) -> Result<(), String> {
    std::fs::write(
        scenes_dir().join(format!("{}.json", name)),
        scene.to_json().map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
