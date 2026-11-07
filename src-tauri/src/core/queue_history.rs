use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use omniget_core::core::ytdlp::archive::{archive_entry_for_url, ArchiveEntry};

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
            ON history (completed_at DESC, id DESC);
        CREATE TABLE IF NOT EXISTS imported_archive (
            extractor TEXT NOT NULL,
            media_id TEXT NOT NULL,
            imported_at INTEGER NOT NULL,
            PRIMARY KEY (extractor, media_id)
        );",
    )
}

fn safe_url(url: &str) -> String {
    crate::core::flight_recorder::redact_url(url)
}

fn safe_title(title: &str) -> String {
    crate::core::flight_recorder::redact_urls(title)
}

fn db_upsert(conn: &Connection, e: &HistoryEntry) -> rusqlite::Result<()> {
    let url = safe_url(&e.url);
    let title = safe_title(&e.title);
    let error = e.error.as_deref().map(crate::core::flight_recorder::redact);
    let thumbnail_url = e.thumbnail_url.as_deref().map(safe_url);
    let kind = e.kind.as_ref().and_then(|k| serde_json::to_string(k).ok());
    conn.execute(
        "INSERT OR REPLACE INTO history
            (id, url, platform, title, file_path, file_size_bytes, total_bytes,
             success, error, completed_at, thumbnail_url, kind)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            e.id as i64,
            url,
            e.platform,
            title,
            e.file_path,
            e.file_size_bytes.map(|v| v as i64),
            e.total_bytes.map(|v| v as i64),
            e.success as i64,
            error,
            e.completed_at,
            thumbnail_url,
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
    let url: String = row.get(1)?;
    Ok(HistoryEntry {
        id: id as u64,
        url: safe_url(&url),
        platform: row.get(2)?,
        title: safe_title(&row.get::<_, String>(3)?),
        file_path: row.get(4)?,
        file_size_bytes: file_size.map(|v| v as u64),
        total_bytes: total.map(|v| v as u64),
        success: success != 0,
        error: row.get(8)?,
        completed_at: row.get(9)?,
        thumbnail_url: row.get::<_, Option<String>>(10)?.as_deref().map(safe_url),
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
    db::with_conn(scrub_existing_rows);
}

fn scrub_existing_rows(conn: &Connection) -> rusqlite::Result<()> {
    type Row = (i64, String, String, Option<String>, Option<String>);
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare("SELECT id, url, title, thumbnail_url, error FROM history")?;
        let mapped = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        mapped.collect::<rusqlite::Result<_>>()?
    };
    for (id, url, title, thumb, error) in rows {
        let new_url = safe_url(&url);
        let new_title = safe_title(&title);
        let new_thumb = thumb.as_deref().map(safe_url);
        let new_error = error.as_deref().map(crate::core::flight_recorder::redact);
        if new_url != url || new_title != title || new_thumb != thumb || new_error != error {
            conn.execute(
                "UPDATE history SET url = ?1, title = ?2, thumbnail_url = ?3, error = ?4 WHERE id = ?5",
                params![new_url, new_title, new_thumb, new_error, id],
            )?;
        }
    }
    Ok(())
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
        c.execute("DELETE FROM imported_archive", [])?;
        Ok(())
    });
}

fn db_import_archive(conn: &Connection, entries: &[ArchiveEntry]) -> rusqlite::Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut added = 0;
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO imported_archive (extractor, media_id, imported_at)
             VALUES (?1, ?2, ?3)",
        )?;
        let now = now_unix_seconds();
        for entry in entries {
            added += stmt.execute(params![entry.extractor, entry.id, now])?;
        }
    }
    tx.commit()?;
    Ok(added)
}

fn db_imported_archive(conn: &Connection) -> rusqlite::Result<Vec<ArchiveEntry>> {
    let mut stmt = conn.prepare(
        "SELECT extractor, media_id FROM imported_archive ORDER BY imported_at, extractor, media_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ArchiveEntry {
            extractor: r.get(0)?,
            id: r.get(1)?,
        })
    })?;
    rows.collect()
}

fn merged_archive(
    history: &[HistoryEntry],
    imported: Vec<ArchiveEntry>,
) -> (Vec<ArchiveEntry>, usize) {
    let mut entries = imported;
    let mut skipped = 0;
    for item in history.iter().filter(|h| h.success) {
        match archive_entry_for_url(&item.url) {
            Some(entry) => entries.push(entry),
            None => skipped += 1,
        }
    }
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.clone()));
    (entries, skipped)
}

pub fn import_archive(entries: &[ArchiveEntry]) -> usize {
    db::with_conn(|c| db_import_archive(c, entries)).unwrap_or(0)
}

pub fn imported_archive_count() -> usize {
    db::with_conn(|c| {
        c.query_row("SELECT COUNT(*) FROM imported_archive", [], |r| {
            r.get::<_, i64>(0)
        })
    })
    .unwrap_or(0) as usize
}

pub fn archive_export_entries() -> (Vec<ArchiveEntry>, usize) {
    let imported = db::with_conn(db_imported_archive).unwrap_or_default();
    merged_archive(&list(), imported)
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

    const SECRET: &str = "SYNTHETIC_SECRET_7f3a";

    #[test]
    fn legacy_rows_are_scrubbed_on_disk() {
        let c = conn();
        let url = format!("https://cdn.test/v.mp4?token={SECRET}");
        c.execute(
            "INSERT INTO history (id, url, platform, title, success, completed_at)
             VALUES (1, ?1, 'generic', ?1, 1, 1)",
            params![url],
        )
        .unwrap();
        let listed = &db_list(&c).unwrap()[0];
        assert!(!listed.url.contains(SECRET));
        assert!(
            !listed.title.contains(SECRET),
            "N-3 title on read: {}",
            listed.title
        );
        scrub_existing_rows(&c).unwrap();
        let (stored, title): (String, String) = c
            .query_row("SELECT url, title FROM history WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(!stored.contains(SECRET), "{stored}");
        assert!(
            stored.starts_with("https://cdn.test/v.mp4?token="),
            "{stored}"
        );
        assert!(!title.contains(SECRET), "N-3 title on disk: {title}");
        assert_eq!(title, stored);
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

#[cfg(test)]
mod archive_tests {
    use super::*;

    fn entry(url: &str, success: bool) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            url: safe_url(url),
            platform: "youtube".into(),
            title: "t".into(),
            file_path: None,
            file_size_bytes: None,
            total_bytes: None,
            success,
            error: None,
            completed_at: 0,
            thumbnail_url: None,
            kind: None,
        }
    }

    #[test]
    fn export_lists_imported_ids_and_finished_downloads_once_each() {
        let imported = vec![ArchiveEntry {
            extractor: "youtube".into(),
            id: "dQw4w9WgXcQ".into(),
        }];
        let history = [
            entry("https://www.youtube.com/watch?v=dQw4w9WgXcQ", true),
            entry("https://www.youtube.com/watch?v=jNQXAC9IVRw", true),
            entry("https://www.youtube.com/watch?v=aqz-KE-bpKQ", false),
        ];
        let (entries, skipped) = merged_archive(&history, imported);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["dQw4w9WgXcQ", "jNQXAC9IVRw"]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn export_counts_finished_links_it_cannot_identify() {
        let (entries, skipped) =
            merged_archive(&[entry("https://example.com/video", true)], Vec::new());
        assert!(entries.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn importing_the_same_archive_twice_adds_nothing_new() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        let entries = vec![ArchiveEntry {
            extractor: "youtube".into(),
            id: "abc123".into(),
        }];
        assert_eq!(db_import_archive(&conn, &entries).unwrap(), 1);
        assert_eq!(db_import_archive(&conn, &entries).unwrap(), 0);
        assert_eq!(db_imported_archive(&conn).unwrap(), entries);
    }
}
