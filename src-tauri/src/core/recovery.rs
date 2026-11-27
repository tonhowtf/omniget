use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const RECOVERY_FILE: &str = "recovery.json";
const WAL_FILE: &str = "queue.wal";
const COMPACT_THRESHOLD: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryItem {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub platform: String,
    pub output_dir: String,
    #[serde(default)]
    pub download_mode: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub referer: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecoveryFile {
    #[serde(default)]
    items: Vec<RecoveryItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WalRecord {
    Enqueued {
        id: u64,
        url: String,
        title: String,
        platform: String,
        output_dir: String,
        #[serde(default)]
        quality: Option<String>,
        #[serde(default)]
        download_mode: Option<String>,
        #[serde(default)]
        format_id: Option<String>,
        #[serde(default)]
        referer: Option<String>,
        #[serde(default)]
        position: u32,
    },
    Started {
        id: u64,
    },
    Progress {
        id: u64,
        #[serde(default)]
        percent: f64,
    },
    Completed {
        id: u64,
    },
    Failed {
        id: u64,
    },
    Cancelled {
        id: u64,
    },
    Removed {
        id: u64,
    },
}

impl WalRecord {
    fn terminal_id(&self) -> Option<u64> {
        match self {
            WalRecord::Completed { id }
            | WalRecord::Failed { id }
            | WalRecord::Cancelled { id }
            | WalRecord::Removed { id } => Some(*id),
            WalRecord::Enqueued { .. } | WalRecord::Started { .. } | WalRecord::Progress { .. } => {
                None
            }
        }
    }
}

pub struct RecoveryLog {
    wal_path: PathBuf,
    legacy_json_path: Option<PathBuf>,
    items: HashMap<u64, RecoveryItem>,
}

impl RecoveryLog {
    pub fn open(wal_path: PathBuf, legacy_json_path: Option<PathBuf>) -> Self {
        let mut log = Self {
            wal_path,
            legacy_json_path,
            items: HashMap::new(),
        };
        log.load();
        log
    }

    fn load(&mut self) {
        self.items.clear();
        if let Ok(contents) = std::fs::read_to_string(&self.wal_path) {
            let (records, dropped) = decode_log(&contents);
            if dropped > 0 {
                tracing::warn!("[recovery] {} truncated record(s) discarded", dropped);
            }
            let (items, stale) = replay_items(&records);
            if needs_rewrite(stale, &records, items.len()) {
                rewrite_wal(&self.wal_path, &items);
            }
            for item in items {
                self.items.insert(item.id, item);
            }
            return;
        }
        if let Some(json_path) = self.legacy_json_path.as_deref() {
            for item in migrate_from_json(&self.wal_path, json_path) {
                self.items.insert(item.id, item);
            }
        }
    }

    pub fn persist(&mut self, item: RecoveryItem) {
        let item = scrub(item);
        let position = self.items.len() as u32;
        append_record(&self.wal_path, &enqueued_record(&item, position));
        self.items.insert(item.id, item);
    }

    pub fn remove(&mut self, id: u64) {
        if self.items.remove(&id).is_some() {
            append_record(&self.wal_path, &WalRecord::Removed { id });
        }
    }

    pub fn list(&self) -> Vec<RecoveryItem> {
        self.items.values().cloned().collect()
    }

    pub fn clear_all(&mut self) {
        self.items.clear();
        let _ = std::fs::remove_file(&self.wal_path);
    }
}

static STORE: OnceLock<Mutex<Option<RecoveryLog>>> = OnceLock::new();

fn store() -> &'static Mutex<Option<RecoveryLog>> {
    STORE.get_or_init(|| Mutex::new(None))
}

fn default_paths() -> Option<(PathBuf, PathBuf)> {
    crate::core::paths::app_data_dir().map(|d| (d.join(WAL_FILE), d.join(RECOVERY_FILE)))
}

fn with_default_log<R>(f: impl FnOnce(&mut RecoveryLog) -> R) -> Option<R> {
    let mut guard = store().lock().unwrap();
    if guard.is_none() {
        let (wal, legacy) = default_paths()?;
        *guard = Some(RecoveryLog::open(wal, Some(legacy)));
    }
    guard.as_mut().map(f)
}

fn scrub(mut item: RecoveryItem) -> RecoveryItem {
    use crate::core::flight_recorder::{redact_url, redact_urls};
    item.url = redact_url(&item.url);
    item.title = redact_urls(&item.title);
    item.referer = item.referer.as_deref().map(redact_url);
    item
}

pub fn is_expired(item: &RecoveryItem) -> bool {
    crate::core::flight_recorder::is_redacted_url(&item.url)
}

fn enqueued_record(item: &RecoveryItem, position: u32) -> WalRecord {
    WalRecord::Enqueued {
        id: item.id,
        url: item.url.clone(),
        title: item.title.clone(),
        platform: item.platform.clone(),
        output_dir: item.output_dir.clone(),
        quality: item.quality.clone(),
        download_mode: item.download_mode.clone(),
        format_id: item.format_id.clone(),
        referer: item.referer.clone(),
        position,
    }
}

fn encode_record(record: &WalRecord) -> Option<String> {
    serde_json::to_string(record).ok()
}

fn decode_log(contents: &str) -> (Vec<WalRecord>, usize) {
    let mut records = Vec::new();
    let mut dropped = 0usize;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<WalRecord>(line) {
            Ok(r) => records.push(r),
            Err(_) => dropped += 1,
        }
    }
    (records, dropped)
}

fn replay_items(records: &[WalRecord]) -> (Vec<RecoveryItem>, bool) {
    let mut live: HashMap<u64, (u32, RecoveryItem)> = HashMap::new();
    let mut stale = false;

    for r in records {
        match r {
            WalRecord::Enqueued {
                id,
                url,
                title,
                platform,
                output_dir,
                quality,
                download_mode,
                format_id,
                referer,
                position,
            } => {
                let raw = RecoveryItem {
                    id: *id,
                    url: url.clone(),
                    title: title.clone(),
                    platform: platform.clone(),
                    output_dir: output_dir.clone(),
                    download_mode: download_mode.clone(),
                    quality: quality.clone(),
                    format_id: format_id.clone(),
                    referer: referer.clone(),
                };
                let clean = scrub(raw.clone());
                if clean.url != raw.url || clean.referer != raw.referer || clean.title != raw.title
                {
                    stale = true;
                }
                live.insert(*id, (*position, clean));
            }
            other => {
                if let Some(id) = other.terminal_id() {
                    live.remove(&id);
                }
            }
        }
    }

    let mut ordered: Vec<(u32, RecoveryItem)> = live.into_values().collect();
    ordered.sort_by_key(|(pos, item)| (*pos, item.id));
    (ordered.into_iter().map(|(_, item)| item).collect(), stale)
}

fn needs_rewrite(stale: bool, records: &[WalRecord], live_len: usize) -> bool {
    stale || (records.len() > COMPACT_THRESHOLD && live_len * 4 < records.len())
}

fn append_record(path: &Path, record: &WalRecord) {
    let Some(parent) = path.parent() else { return };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!("[recovery] create_dir_all failed: {}", e);
        return;
    }
    let Some(line) = encode_record(record) else {
        tracing::warn!("[recovery] encode failed");
        return;
    };
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!("[recovery] append failed: {}", e);
    }
}

fn rewrite_wal(path: &Path, items: &[RecoveryItem]) {
    let tmp = path.with_extension("wal.tmp");
    let mut body = String::new();
    for (pos, item) in items.iter().enumerate() {
        if let Some(line) = encode_record(&enqueued_record(item, pos as u32)) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        tracing::warn!("[recovery] rewrite failed: {}", e);
    }
}

fn json_to_records(content: &str) -> Vec<WalRecord> {
    let parsed: RecoveryFile = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut items: Vec<RecoveryItem> = parsed.items.into_iter().map(scrub).collect();
    items.sort_by_key(|i| i.id);
    items
        .iter()
        .enumerate()
        .map(|(pos, item)| enqueued_record(item, pos as u32))
        .collect()
}

fn migrate_from_json(wal_path: &Path, json_path: &Path) -> Vec<RecoveryItem> {
    let Ok(content) = std::fs::read_to_string(json_path) else {
        return Vec::new();
    };
    let records = json_to_records(&content);
    if records.is_empty() && !content.trim().is_empty() {
        tracing::warn!("[recovery] recovery.json unreadable or empty, nothing migrated");
    }
    for r in &records {
        append_record(wal_path, r);
    }
    let (items, _) = replay_items(&records);
    let dest = json_path.with_extension("json.migrated");
    if let Err(e) = std::fs::rename(json_path, &dest) {
        tracing::warn!("[recovery] could not rename the migrated json: {}", e);
    }
    tracing::info!(
        "[recovery] {} items migrated from recovery.json",
        items.len()
    );
    items
}

pub fn init_from_disk() {
    let mut guard = store().lock().unwrap();
    if let Some((wal, legacy)) = default_paths() {
        *guard = Some(RecoveryLog::open(wal, Some(legacy)));
    } else {
        *guard = None;
    }
}

pub fn persist(item: RecoveryItem) {
    let _ = with_default_log(|log| log.persist(item));
}

pub fn remove(id: u64) {
    let _ = with_default_log(|log| log.remove(id));
}

pub fn list() -> Vec<RecoveryItem> {
    with_default_log(|log| log.list()).unwrap_or_default()
}

pub fn clear_all() {
    let _ = with_default_log(|log| log.clear_all());
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_0_7_X: &str = r#"{
      "items": [
        {
          "id": 7,
          "url": "https://exemplo.com/b",
          "title": "Segundo",
          "platform": "youtube",
          "output_dir": "/downloads",
          "download_mode": "audio",
          "quality": "720p",
          "format_id": "140",
          "referer": "https://exemplo.com/pagina"
        },
        {
          "id": 2,
          "url": "https://exemplo.com/a",
          "title": "Primeiro",
          "platform": "instagram",
          "output_dir": "/downloads"
        }
      ]
    }"#;

    #[test]
    fn migration_preserves_fields_and_order() {
        let (items, _) = replay_items(&json_to_records(JSON_0_7_X));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 2);
        assert_eq!(items[1].id, 7);
        assert_eq!(
            items[1].referer.as_deref(),
            Some("https://exemplo.com/pagina")
        );
        assert_eq!(items[1].download_mode.as_deref(), Some("audio"));
        assert_eq!(items[1].quality.as_deref(), Some("720p"));
        assert_eq!(items[1].format_id.as_deref(), Some("140"));
        assert!(!is_expired(&items[1]));
    }

    #[test]
    fn corrupted_json_returns_empty() {
        assert!(json_to_records("{ invalid").is_empty());
        assert!(json_to_records("").is_empty());
    }

    #[test]
    fn signed_url_is_redacted_on_disk_and_marked_expired() {
        let dir = tempfile::tempdir().unwrap();
        let wal = dir.path().join("queue.wal");
        let mut log = RecoveryLog::open(wal.clone(), None);
        log.persist(RecoveryItem {
            id: 9,
            url: "https://cdn.example.com/v.mp4?X-Amz-Signature=SYNTHETIC_SECRET&token=SYNTHETIC_SECRET".into(),
            title: "https://cdn.example.com/v.mp4?X-Amz-Signature=SYNTHETIC_SECRET".into(),
            platform: "generic".into(),
            output_dir: "/d".into(),
            download_mode: None,
            quality: None,
            format_id: None,
            referer: Some("https://example.com/p?access_token=SYNTHETIC_SECRET".into()),
        });
        let raw = std::fs::read_to_string(&wal).unwrap();
        assert!(!raw.contains("SYNTHETIC_SECRET"), "{raw}");

        let reopened = RecoveryLog::open(wal, None);
        let items = reopened.list();
        assert_eq!(items.len(), 1);
        assert!(is_expired(&items[0]));
    }

    #[test]
    fn persist_remove_and_reopen_roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let wal = dir.path().join("queue.wal");
        {
            let mut log = RecoveryLog::open(wal.clone(), None);
            log.persist(RecoveryItem {
                id: 1,
                url: "https://example.com/1".into(),
                title: "One".into(),
                platform: "youtube".into(),
                output_dir: "/d".into(),
                download_mode: None,
                quality: None,
                format_id: None,
                referer: None,
            });
            log.persist(RecoveryItem {
                id: 2,
                url: "https://example.com/2".into(),
                title: "Two".into(),
                platform: "youtube".into(),
                output_dir: "/d".into(),
                download_mode: None,
                quality: None,
                format_id: None,
                referer: None,
            });
            log.remove(1);
        }
        let reopened = RecoveryLog::open(wal, None);
        let items = reopened.list();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 2);
    }

    #[test]
    fn truncated_lines_and_legacy_ops_are_handled_safely() {
        let raw = concat!(
            r#"{"op":"enqueued","id":1,"url":"https://example.com/1","title":"A","platform":"yt","output_dir":"/d","position":0}"#,
            "\n",
            r#"{"op":"started","id":1}"#,
            "\n",
            r#"{"op":"progress","id":1,"percent":50.0}"#,
            "\n",
            r#"{"op":"enqueued","id":2,"url":"https://example.com/2","title":"B","platform":"yt","output_dir":"/d","position":1}"#,
            "\n",
            r#"{"op":"completed","id":1,"file_path":"/d/a.mp4"}"#,
            "\n",
            "{truncated"
        );
        let (records, dropped) = decode_log(raw);
        assert_eq!(dropped, 1);
        let (items, _) = replay_items(&records);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, 2);
    }

    #[test]
    fn bloated_log_triggers_compaction_on_open() {
        let mut records = Vec::new();
        for id in 0..400u64 {
            records.extend(json_to_records(&format!(
                r#"{{"items":[{{"id":{id},"url":"https://exemplo.com/{id}","title":"t","platform":"youtube","output_dir":"/d"}}]}}"#
            )));
            records.push(WalRecord::Removed { id });
        }
        assert!(needs_rewrite(false, &records, 0));
    }
}
