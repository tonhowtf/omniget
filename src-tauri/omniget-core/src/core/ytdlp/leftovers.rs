use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeftoverFile {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
}

const STORE_FILE: &str = "download-leftovers.json";
const MAX_TASKS: usize = 300;

type Registry = HashMap<u64, Vec<PathBuf>>;

fn store_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::core::paths::app_data_dir().map(|d| d.join(STORE_FILE))
}

fn load_from(path: &Path) -> Registry {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, map: &Registry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut ids: Vec<u64> = map.keys().copied().collect();
    ids.sort_unstable();
    let keep: Registry = ids
        .into_iter()
        .rev()
        .take(MAX_TASKS)
        .filter_map(|id| map.get(&id).map(|p| (id, p.clone())))
        .collect();
    let json = serde_json::to_string(&keep).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

fn persist(map: &Registry) {
    if let Some(path) = store_path() {
        if let Err(e) = save_to(&path, map) {
            tracing::warn!("[leftovers] could not save {}: {}", path.display(), e);
        }
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(store_path().map(|p| load_from(&p)).unwrap_or_default()))
}

pub(super) fn note_task_destination(id: u64, path: &Path) {
    if let Ok(mut map) = registry().lock() {
        let paths = map.entry(id).or_default();
        if !paths.iter().any(|p| p == path) {
            paths.push(path.to_path_buf());
            persist(&map);
        }
    }
}

pub fn forget_task_destinations(id: u64) {
    if let Ok(mut map) = registry().lock() {
        if map.remove(&id).is_some() {
            persist(&map);
        }
    }
}

fn is_fragment_suffix(rest: &str) -> bool {
    let Some(tail) = rest.strip_prefix(".part-Frag") else {
        return false;
    };
    let digits = tail.strip_suffix(".part").unwrap_or(tail);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn is_stream_file_name(name: &str) -> bool {
    let mut parts = name.rsplit('.');
    let _extension = parts.next();
    parts.next().is_some_and(|format| {
        format
            .strip_prefix('f')
            .is_some_and(|f| f.chars().next().is_some_and(|c| c.is_ascii_digit()))
    })
}

fn merge_temp_name(destination: &str) -> Option<String> {
    let (stem, extension) = destination.rsplit_once('.')?;
    Some(format!("{stem}.temp.{extension}"))
}

fn is_partial_of(candidate: &str, destination: &str) -> bool {
    if merge_temp_name(destination).as_deref() == Some(candidate) {
        return true;
    }
    let Some(rest) = candidate.strip_prefix(destination) else {
        return false;
    };
    if rest.is_empty() {
        return is_stream_file_name(destination);
    }
    matches!(rest, ".part" | ".ytdl" | ".aria2" | ".temp") || is_fragment_suffix(rest)
}

pub(crate) fn find_leftovers(destinations: &[PathBuf], finished: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut folders: Vec<&Path> = destinations.iter().filter_map(|d| d.parent()).collect();
    folders.sort();
    folders.dedup();
    for folder in folders {
        let names: Vec<String> = destinations
            .iter()
            .filter(|d| d.parent() == Some(folder))
            .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if finished == Some(path.as_path()) {
                continue;
            }
            let is_regular = entry
                .metadata()
                .map(|m| m.file_type().is_file())
                .unwrap_or(false);
            if !is_regular {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if names.iter().any(|d| is_partial_of(&name, d)) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

pub fn task_leftovers(id: u64, finished: Option<&Path>) -> Vec<LeftoverFile> {
    let destinations = registry()
        .lock()
        .ok()
        .and_then(|map| map.get(&id).cloned())
        .unwrap_or_default();
    find_leftovers(&destinations, finished)
        .into_iter()
        .map(|path| LeftoverFile {
            size_bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
        })
        .collect()
}

pub fn delete_task_leftovers(id: u64, finished: Option<&Path>) -> usize {
    let mut deleted = 0;
    for file in task_leftovers(id, finished) {
        if std::fs::remove_file(&file.path).is_ok() {
            deleted += 1;
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_survive_a_restart() {
        let dir =
            std::env::temp_dir().join(format!("omniget-leftovers-store-{}", std::process::id()));
        let file = dir.join(STORE_FILE);
        let mut map = Registry::new();
        map.insert(7, vec![dir.join("Clip.mp4")]);
        save_to(&file, &map).unwrap();
        assert_eq!(load_from(&file), map);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_store_keeps_only_the_newest_tasks() {
        let dir =
            std::env::temp_dir().join(format!("omniget-leftovers-cap-{}", std::process::id()));
        let file = dir.join(STORE_FILE);
        let map: Registry = (0..(MAX_TASKS as u64 + 5))
            .map(|id| (id, vec![PathBuf::from("x")]))
            .collect();
        save_to(&file, &map).unwrap();
        let loaded = load_from(&file);
        assert_eq!(loaded.len(), MAX_TASKS);
        assert!(loaded.contains_key(&(MAX_TASKS as u64 + 4)));
        assert!(!loaded.contains_key(&0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-leftovers-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn finds_the_partial_files_of_the_task_and_nothing_else() {
        let dir = folder("own");
        let video = dir.join("Clip [a].f137.mp4");
        let merged = dir.join("Clip [a].mp4");
        touch(&dir, "Clip [a].f137.mp4");
        touch(&dir, "Clip [a].f137.mp4.part");
        touch(&dir, "Clip [a].f137.mp4.ytdl");
        touch(&dir, "Clip [a].f137.mp4.part-Frag12");
        touch(&dir, "Clip [a].f137.mp4.part-Frag13.part");
        touch(&dir, "Clip [a].temp.mp4");
        touch(&dir, "Other [b].f137.mp4.part");
        touch(&dir, "Other [b].mp4");
        touch(&dir, "notes.txt");
        let found = find_leftovers(&[video, merged], None);
        assert_eq!(
            names(&found),
            vec![
                "Clip [a].f137.mp4",
                "Clip [a].f137.mp4.part",
                "Clip [a].f137.mp4.part-Frag12",
                "Clip [a].f137.mp4.part-Frag13.part",
                "Clip [a].f137.mp4.ytdl",
                "Clip [a].temp.mp4",
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_finished_file_is_never_a_leftover() {
        let dir = folder("finished");
        let merged = touch(&dir, "Clip [a].mp4");
        assert!(find_leftovers(std::slice::from_ref(&merged), None).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_finished_path_is_kept_even_when_its_name_looks_partial() {
        let dir = folder("kept");
        let odd = touch(&dir, "Take.f1.mp4");
        assert!(find_leftovers(std::slice::from_ref(&odd), Some(&odd)).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_similar_title_in_the_same_folder_is_not_touched() {
        let dir = folder("similar");
        let dest = dir.join("Clip.mp4");
        touch(&dir, "Clip.mp4.part");
        let lookalike = touch(&dir, "Clip.mp4.part2");
        let found = find_leftovers(&[dest], None);
        assert_eq!(names(&found), vec!["Clip.mp4.part"]);
        assert!(lookalike.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deleting_removes_only_what_the_task_recorded() {
        let dir = folder("delete");
        let id = 900_000_001;
        note_task_destination(id, &dir.join("Clip.f137.mp4"));
        let part = touch(&dir, "Clip.f137.mp4.part");
        let other = touch(&dir, "Clip.mp4");
        assert_eq!(delete_task_leftovers(id, None), 1);
        assert!(!part.exists());
        assert!(other.exists());
        forget_task_destinations(id);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_task_that_recorded_nothing_offers_nothing() {
        assert!(task_leftovers(900_000_002, None).is_empty());
    }
}
