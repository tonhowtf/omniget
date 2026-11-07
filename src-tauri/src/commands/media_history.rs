use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::core::media_diff::{diff, summarize, MediaSnapshot};

const SNAPSHOTS_FILE: &str = "media-snapshots.json";

const MAX_SNAPSHOTS: usize = 2000;

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(SNAPSHOTS_FILE))
}

fn load() -> BTreeMap<String, MediaSnapshot> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!(
            "[media-diff] {} unreadable, ignoring: {}",
            path.display(),
            e
        );
        BTreeMap::new()
    })
}

fn save(map: &BTreeMap<String, MediaSnapshot>) {
    let Some(path) = file_path() else { return };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_string(map) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn podar(
    mut map: BTreeMap<String, MediaSnapshot>,
    teto: usize,
) -> BTreeMap<String, MediaSnapshot> {
    while map.len() > teto {
        let Some(primeira) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&primeira);
    }
    map
}

#[tauri::command]
pub fn check_media_changed(url: String, current: MediaSnapshot) -> Option<String> {
    let mapa = load();
    let anterior = mapa.get(&url)?;
    let mudancas = diff(anterior, &current);
    summarize(&mudancas)
}

#[tauri::command]
pub fn record_media_snapshot(url: String, snapshot: MediaSnapshot) {
    let mut mapa = load();
    mapa.insert(url, snapshot);
    let mapa = podar(mapa, MAX_SNAPSHOTS);
    save(&mapa);
}
