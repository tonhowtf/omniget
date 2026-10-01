use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::core::db;

const CHANNELS_FILE: &str = "channels.json";
const MAX_SEEN_IDS: usize = 500;

fn default_true() -> bool {
    true
}

fn default_interval() -> u32 {
    60
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelFollow {
    pub id: String,
    pub url: String,
    pub title: String,
    pub added_at_ms: u64,
    #[serde(default)]
    pub last_checked_ms: Option<u64>,
    #[serde(default)]
    pub seen_ids: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_download: bool,
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ChannelsFile {
    #[serde(default)]
    channels: Vec<ChannelFollow>,
}

pub fn id_for_url(url: &str) -> String {
    let mut hasher = DefaultHasher::new();
    url.trim().hash(&mut hasher);
    format!("ch{:016x}", hasher.finish())
}

fn schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            url TEXT NOT NULL,
            title TEXT NOT NULL,
            added_at_ms INTEGER NOT NULL,
            last_checked_ms INTEGER,
            seen_ids TEXT NOT NULL DEFAULT '[]',
            enabled INTEGER NOT NULL DEFAULT 1,
            auto_download INTEGER NOT NULL DEFAULT 0,
            interval_minutes INTEGER NOT NULL DEFAULT 60
        );",
    )
}

fn row_to_channel(row: &rusqlite::Row) -> rusqlite::Result<ChannelFollow> {
    let added: i64 = row.get(3)?;
    let last: Option<i64> = row.get(4)?;
    let seen_json: String = row.get(5)?;
    let enabled: i64 = row.get(6)?;
    let auto: i64 = row.get(7)?;
    let interval: i64 = row.get(8)?;
    let id: String = row.get(0)?;

    // `seen_ids` ilegível não pode virar lista vazia em silêncio: o poller
    // trataria todo vídeo já arquivado como novo e re-baixaria o canal
    // inteiro. Sem saber o que já foi visto, o download automático fica
    // desligado nesta leitura até o usuário reabilitar.
    let (seen_ids, auto_download) = match serde_json::from_str::<Vec<String>>(&seen_json) {
        Ok(seen) => (seen, auto != 0),
        Err(e) => {
            tracing::error!(
                "[channels] historico de vistos do canal {} esta corrompido ({}). \
                 Download automatico desligado nesta sessao para nao re-baixar tudo.",
                id,
                e
            );
            (Vec::new(), false)
        }
    };

    Ok(ChannelFollow {
        id,
        url: row.get(1)?,
        title: row.get(2)?,
        added_at_ms: added as u64,
        last_checked_ms: last.map(|v| v as u64),
        seen_ids,
        enabled: enabled != 0,
        auto_download,
        interval_minutes: interval as u32,
    })
}

fn db_upsert(conn: &Connection, ch: &ChannelFollow) -> rusqlite::Result<()> {
    let seen = serde_json::to_string(&ch.seen_ids).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT OR REPLACE INTO channels
            (id, url, title, added_at_ms, last_checked_ms, seen_ids,
             enabled, auto_download, interval_minutes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            ch.id,
            ch.url,
            ch.title,
            ch.added_at_ms as i64,
            ch.last_checked_ms.map(|v| v as i64),
            seen,
            ch.enabled as i64,
            ch.auto_download as i64,
            ch.interval_minutes as i64,
        ],
    )?;
    Ok(())
}

fn db_get(conn: &Connection, id: &str) -> rusqlite::Result<Option<ChannelFollow>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, title, added_at_ms, last_checked_ms, seen_ids,
                enabled, auto_download, interval_minutes
         FROM channels WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], row_to_channel)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

fn db_all(conn: &Connection) -> rusqlite::Result<Vec<ChannelFollow>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, title, added_at_ms, last_checked_ms, seen_ids,
                enabled, auto_download, interval_minutes
         FROM channels ORDER BY added_at_ms ASC",
    )?;
    let rows = stmt.query_map([], row_to_channel)?;
    rows.collect()
}

fn json_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(CHANNELS_FILE))
}

fn import_legacy_json(conn: &Connection) {
    let Some(path) = json_path() else { return };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    match serde_json::from_str::<ChannelsFile>(&content) {
        Ok(parsed) => {
            for ch in parsed.channels {
                let _ = db_upsert(conn, &ch);
            }
            tracing::info!("[channels] imported legacy JSON into SQLite");
        }
        Err(e) => tracing::warn!("[channels] legacy JSON parse failed: {}", e),
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

pub fn list() -> Vec<ChannelFollow> {
    db::with_conn(db_all).unwrap_or_default()
}

pub fn get(id: &str) -> Option<ChannelFollow> {
    db::with_conn(|c| db_get(c, id)).flatten()
}

pub fn add(url: String, title: String) -> ChannelFollow {
    let id = id_for_url(&url);
    db::with_conn(|c| {
        let existing = db_get(c, &id)?;
        let mut ch = existing.unwrap_or_else(|| ChannelFollow {
            id: id.clone(),
            url: url.trim().to_string(),
            title: title.clone(),
            added_at_ms: now_ms(),
            last_checked_ms: None,
            seen_ids: Vec::new(),
            enabled: true,
            auto_download: false,
            interval_minutes: default_interval(),
        });
        if !title.is_empty() {
            ch.title = title.clone();
        }
        db_upsert(c, &ch)?;
        Ok(ch)
    })
    .unwrap_or_else(|| ChannelFollow {
        id,
        url: url.trim().to_string(),
        title,
        added_at_ms: now_ms(),
        last_checked_ms: None,
        seen_ids: Vec::new(),
        enabled: true,
        auto_download: false,
        interval_minutes: default_interval(),
    })
}

pub fn remove(id: &str) -> bool {
    db::with_conn(|c| {
        let n = c.execute("DELETE FROM channels WHERE id = ?1", params![id])?;
        Ok(n > 0)
    })
    .unwrap_or(false)
}

pub fn update(
    id: &str,
    enabled: Option<bool>,
    auto_download: Option<bool>,
    interval_minutes: Option<u32>,
) -> Option<ChannelFollow> {
    db::with_conn(|c| {
        let Some(mut ch) = db_get(c, id)? else {
            return Ok(None);
        };
        if let Some(v) = enabled {
            ch.enabled = v;
        }
        if let Some(v) = auto_download {
            ch.auto_download = v;
        }
        if let Some(v) = interval_minutes {
            ch.interval_minutes = v.max(5);
        }
        db_upsert(c, &ch)?;
        Ok(Some(ch))
    })
    .flatten()
}

fn fold_seen(ch: &mut ChannelFollow, fetched_ids: &[String]) -> Vec<String> {
    let first_poll = ch.seen_ids.is_empty();
    let known: std::collections::HashSet<&String> = ch.seen_ids.iter().collect();
    let new_ids: Vec<String> = fetched_ids
        .iter()
        .filter(|fid| !known.contains(fid))
        .cloned()
        .collect();

    for fid in fetched_ids {
        if !ch.seen_ids.iter().any(|s| s == fid) {
            ch.seen_ids.push(fid.clone());
        }
    }
    if ch.seen_ids.len() > MAX_SEEN_IDS {
        let overflow = ch.seen_ids.len() - MAX_SEEN_IDS;
        ch.seen_ids.drain(0..overflow);
    }
    ch.last_checked_ms = Some(now_ms());

    if first_poll {
        Vec::new()
    } else {
        new_ids
    }
}

pub fn record_poll(id: &str, fetched_ids: &[String]) -> Vec<String> {
    db::with_conn(|c| {
        let Some(mut ch) = db_get(c, id)? else {
            return Ok(Vec::new());
        };
        let new_ids = fold_seen(&mut ch, fetched_ids);
        db_upsert(c, &ch)?;
        Ok(new_ids)
    })
    .unwrap_or_default()
}
