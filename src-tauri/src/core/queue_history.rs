use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::core::db;
use crate::core::queue::QueueKind;

const HISTORY_FILE: &str = "download-history.json";
const MAX_HISTORY_ENTRIES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: u64,
    pub url: String,
    pub platform: String,
    pub title: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_size_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    pub completed_at: i64,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub kind: Option<QueueKind>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<HistoryEntry>,
}

fn schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY,
            url TEXT NOT NULL,
            platform TEXT NOT NULL,
            title TEXT NOT NULL,
            file_path TEXT,
            file_size_bytes INTEGER,
            total_bytes INTEGER,
            success INTEGER NOT NULL,
            error TEXT,
            completed_at INTEGER NOT NULL,
            thumbnail_url TEXT,
            kind TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_history_completed
            ON history (completed_at DESC, id DESC);",
    )
}

fn db_upsert(conn: &Connection, e: &HistoryEntry) -> rusqlite::Result<()> {
    let kind = e.kind.as_ref().and_then(|k| serde_json::to_string(k).ok());
    conn.execute(
        "INSERT OR REPLACE INTO history
            (id, url, platform, title, file_path, file_size_bytes, total_bytes,
             success, error, completed_at, thumbnail_url, kind)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            e.id as i64,
            e.url,
            e.platform,
            e.title,
            e.file_path,
            e.file_size_bytes.map(|v| v as i64),
            e.total_bytes.map(|v| v as i64),
            e.success as i64,
            e.error,
            e.completed_at,
            e.thumbnail_url,
            kind,
        ],
    )?;
    conn.execute(
        "DELETE FROM history WHERE id NOT IN
            (SELECT id FROM history ORDER BY completed_at DESC, id DESC LIMIT ?1)",
        params![MAX_HISTORY_ENTRIES as i64],
    )?;
    Ok(())
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<HistoryEntry> {
    let id: i64 = row.get(0)?;
    let file_size: Option<i64> = row.get(5)?;
    let total: Option<i64> = row.get(6)?;
    let success: i64 = row.get(7)?;
    let kind_text: Option<String> = row.get(11)?;
    Ok(HistoryEntry {
        id: id as u64,
        url: row.get(1)?,
        platform: row.get(2)?,
        title: row.get(3)?,
        file_path: row.get(4)?,
        file_size_bytes: file_size.map(|v| v as u64),
        total_bytes: total.map(|v| v as u64),
        success: success != 0,
        error: row.get(8)?,
        completed_at: row.get(9)?,
        thumbnail_url: row.get(10)?,
        kind: kind_text.and_then(|t| serde_json::from_str(&t).ok()),
    })
}

fn db_list(conn: &Connection) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, platform, title, file_path, file_size_bytes, total_bytes,
                success, error, completed_at, thumbnail_url, kind
         FROM history ORDER BY completed_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([], row_to_entry)?;
    rows.collect()
}

fn json_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(HISTORY_FILE))
}

fn import_legacy_json(conn: &Connection) {
    let Some(path) = json_path() else { return };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    match serde_json::from_str::<HistoryFile>(&content) {
        Ok(parsed) => {
            for entry in parsed.entries.into_iter().take(MAX_HISTORY_ENTRIES) {
                let _ = db_upsert(conn, &entry);
            }
            tracing::info!("[history] imported legacy JSON into SQLite");
        }
        Err(e) => tracing::warn!("[history] legacy JSON parse failed: {}", e),
    }
    let _ = std::fs::rename(&path, path.with_extension("json.imported"));
}

pub fn init_from_disk() {
    db::with_conn(|c| {
        schema(c)?;
        Ok(())
    });
    db::with_conn(|c| {
        import_legacy_json(c);
        Ok(())
    });
}

pub fn record(entry: HistoryEntry) {
    db::with_conn(|c| db_upsert(c, &entry));
}

pub fn list() -> Vec<HistoryEntry> {
    db::with_conn(db_list).unwrap_or_default()
}

pub fn remove(id: u64) {
    db::with_conn(|c| {
        c.execute("DELETE FROM history WHERE id = ?1", params![id as i64])?;
        Ok(())
    });
}

pub fn clear_all() {
    db::with_conn(|c| {
        c.execute("DELETE FROM history", [])?;
        Ok(())
    });
}

pub fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: u64, completed_at: i64) -> HistoryEntry {
        HistoryEntry {
            id,
            url: format!("https://x.test/{}", id),
            platform: "youtube".into(),
            title: format!("Video {}", id),
            file_path: Some(format!("/tmp/{}.mp4", id)),
            file_size_bytes: Some(1234),
            total_bytes: Some(2000),
            success: true,
            error: None,
            completed_at,
            thumbnail_url: None,
            kind: Some(QueueKind::Video),
        }
    }

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        schema(&c).unwrap();
        c
    }

    #[test]
    fn upsert_list_roundtrip_newest_first() {
        let c = conn();
        db_upsert(&c, &mk(1, 100)).unwrap();
        db_upsert(&c, &mk(2, 200)).unwrap();
        let list = db_list(&c).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 2);
        assert_eq!(list[1].id, 1);
        assert_eq!(list[0].kind, Some(QueueKind::Video));
    }

    #[test]
    fn upsert_replaces_same_id() {
        let c = conn();
        db_upsert(&c, &mk(1, 100)).unwrap();
        let mut e = mk(1, 150);
        e.title = "Renamed".into();
        db_upsert(&c, &e).unwrap();
        let list = db_list(&c).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Renamed");
    }

    #[test]
    fn prune_keeps_only_max_entries() {
        let c = conn();
        for i in 0..(MAX_HISTORY_ENTRIES as u64 + 25) {
            db_upsert(&c, &mk(i + 1, i as i64)).unwrap();
        }
        let list = db_list(&c).unwrap();
        assert_eq!(list.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(list[0].id, MAX_HISTORY_ENTRIES as u64 + 25);
    }

    #[test]
    fn import_legacy_json_round_trips() {
        let c = conn();
        let file = HistoryFile {
            entries: vec![mk(7, 70), mk(8, 80)],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: HistoryFile = serde_json::from_str(&json).unwrap();
        for e in parsed.entries {
            db_upsert(&c, &e).unwrap();
        }
        let list = db_list(&c).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 8);
    }
}
