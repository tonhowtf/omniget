use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 300;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Recording,
    Export,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub kind: EntryKind,
    pub path: String,
    pub created_at: i64,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub exists: bool,
}

impl HistoryEntry {
    pub fn new(kind: EntryKind, path: &Path, duration: Option<f64>, source: Option<&Path>) -> Self {
        HistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            path: path.to_string_lossy().to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
            duration,
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            source: source.map(|s| s.to_string_lossy().to_string()),
            exists: true,
        }
    }
}

pub fn default_file() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("capture").join("history.json"))
}

fn read(file: &Path) -> Vec<HistoryEntry> {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<HistoryEntry>>(&b).ok())
        .unwrap_or_default()
}

pub fn write_atomic(file: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = file.parent().ok_or("History path has no folder.")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        file.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        uuid::Uuid::new_v4().simple()
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn save(file: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    let stored: Vec<HistoryEntry> = entries
        .iter()
        .cloned()
        .map(|mut e| {
            e.exists = false;
            e
        })
        .collect();
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    write_atomic(file, &bytes)
}

fn with_exists(mut entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    for e in &mut entries {
        e.exists = Path::new(&e.path).is_file();
    }
    entries
}

pub fn list(file: &Path) -> Vec<HistoryEntry> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    with_exists(read(file))
}

pub fn add(file: &Path, entry: HistoryEntry) -> Result<Vec<HistoryEntry>, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = read(file);
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
    save(file, &entries)?;
    Ok(with_exists(entries))
}

pub fn remove(file: &Path, id: &str) -> Result<Vec<HistoryEntry>, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = read(file);
    entries.retain(|e| e.id != id);
    save(file, &entries)?;
    Ok(with_exists(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-capture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn add_list_remove_keeps_files() {
        let dir = tmp_dir();
        let file = dir.join("h").join("history.json");
        let video = dir.join("a.mp4");
        std::fs::write(&video, b"1234").unwrap();
        let first = HistoryEntry::new(EntryKind::Recording, &video, Some(3.0), None);
        let ghost = HistoryEntry::new(EntryKind::Export, &dir.join("gone.mp4"), None, Some(&video));
        add(&file, first.clone()).unwrap();
        let after = add(&file, ghost.clone()).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].id, ghost.id);
        assert!(!after[0].exists);
        assert!(after[1].exists);
        assert_eq!(after[1].size_bytes, 4);

        let again = add(
            &file,
            HistoryEntry::new(EntryKind::Recording, &video, None, None),
        )
        .unwrap();
        assert_eq!(again.len(), 2);

        let left = remove(&file, &ghost.id).unwrap();
        assert_eq!(left.len(), 1);
        assert!(video.exists());
        assert_eq!(list(&file).len(), 1);
        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"exists\": false"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_reads_as_empty() {
        let dir = tmp_dir();
        let file = dir.join("history.json");
        std::fs::write(&file, b"{not json").unwrap();
        assert!(list(&file).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
