use std::future::Future;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::AppHandle;

use super::{db, PlayMode, PlaybackInfo, SourceRef};

const SLOW_AFTER: Duration = Duration::from_secs(8);
const DOWN_AFTER: i64 = 3;
const DOWN_RETRY_MS: i64 = 5 * 60 * 1000;

fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS sources (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            config_json TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'ok',
            last_error TEXT,
            fails INTEGER NOT NULL DEFAULT 0,
            position INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS source_kv (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS iptv_channels (
            id INTEGER PRIMARY KEY,
            source INTEGER NOT NULL,
            name TEXT NOT NULL,
            norm TEXT NOT NULL DEFAULT '',
            grp TEXT NOT NULL DEFAULT '',
            logo TEXT,
            tvg_id TEXT,
            epg_id TEXT,
            url TEXT NOT NULL,
            headers_json TEXT NOT NULL DEFAULT '[]',
            favorite INTEGER NOT NULL DEFAULT 0,
            position INTEGER NOT NULL DEFAULT 0,
            UNIQUE(source, url)
        );
        CREATE INDEX IF NOT EXISTS iptv_channels_grp ON iptv_channels(source, grp);
        CREATE INDEX IF NOT EXISTS iptv_channels_epg ON iptv_channels(source, epg_id);
        CREATE TABLE IF NOT EXISTS epg_programmes (
            source INTEGER NOT NULL,
            channel TEXT NOT NULL,
            start INTEGER NOT NULL,
            stop INTEGER NOT NULL,
            title TEXT NOT NULL,
            descr TEXT,
            category TEXT
        );
        CREATE INDEX IF NOT EXISTS epg_programmes_ch ON epg_programmes(source, channel, start);
        CREATE TABLE IF NOT EXISTS epg_staging (
            source INTEGER NOT NULL,
            channel TEXT NOT NULL,
            start INTEGER NOT NULL,
            stop INTEGER NOT NULL,
            title TEXT NOT NULL,
            descr TEXT,
            category TEXT
        );
        CREATE INDEX IF NOT EXISTS epg_staging_src ON epg_staging(source);
        CREATE TABLE IF NOT EXISTS epg_channels (
            source INTEGER NOT NULL,
            xml_id TEXT NOT NULL,
            norm TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS epg_channels_norm ON epg_channels(source, norm);
        CREATE INDEX IF NOT EXISTS epg_channels_id ON epg_channels(source, xml_id);
        CREATE TABLE IF NOT EXISTS sync_outbox (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            dedupe TEXT NOT NULL UNIQUE,
            payload TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_at INTEGER NOT NULL,
            last_error TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS http_cache (
            key TEXT PRIMARY KEY,
            body TEXT NOT NULL,
            fetched_at INTEGER NOT NULL
        );",
    )
}

pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    static READY: OnceLock<Result<(), String>> = OnceLock::new();
    READY.get_or_init(|| db::with(schema)).clone()?;
    db::with(f)
}

pub fn with_mut<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    with(|_| Ok(()))?;
    db::with_mut(f)
}

pub fn now_ms() -> i64 {
    db::now_ms()
}

pub fn kv_get(key: &str) -> Option<String> {
    with(|c| {
        c.query_row("SELECT value FROM source_kv WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()
    })
    .ok()
    .flatten()
}

pub fn kv_set(key: &str, value: &str) {
    let _ = with(|c| {
        c.execute(
            "INSERT INTO source_kv(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
    });
}

pub fn kv_del(key: &str) {
    let _ = with(|c| c.execute("DELETE FROM source_kv WHERE key = ?1", [key]));
}

pub fn cache_get(key: &str, max_age_ms: i64) -> Option<String> {
    let row: Option<(String, i64)> = with(|c| {
        c.query_row(
            "SELECT body, fetched_at FROM http_cache WHERE key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    })
    .ok()
    .flatten();
    let (body, at) = row?;
    (max_age_ms < 0 || now_ms() - at <= max_age_ms).then_some(body)
}

pub fn cache_put(key: &str, body: &str) {
    let _ = with(|c| {
        c.execute(
            "INSERT INTO http_cache(key, body, fetched_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
            params![key, body, now_ms()],
        )
    });
}

const SECRETS: crate::secrets::Namespace = crate::secrets::Namespace {
    service: "wtf.tonho.omniget.video",
    dir_name: "video",
    dir_env: "OMNIGET_VIDEO_SECRET_DIR",
};

pub fn secret_get(account: &str) -> Option<String> {
    crate::secrets::load_secret(SECRETS, account)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

pub fn secret_set(account: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return crate::secrets::delete_secret(SECRETS, account);
    }
    crate::secrets::save_secret(SECRETS, account, value)
}

pub fn secret_del(account: &str) {
    let _ = crate::secrets::delete_secret(SECRETS, account);
}

pub fn user_agent() -> String {
    format!("OmniGet/{}", env!("CARGO_PKG_VERSION"))
}

pub fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .user_agent(user_agent())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

pub fn net_err(e: reqwest::Error) -> String {
    tracing::warn!("[video] request failed: {e:?}");
    if e.is_timeout() {
        "Timed out".into()
    } else if e.is_connect() {
        "Can't reach the server".into()
    } else if e.is_decode() {
        "Unexpected answer from the server".into()
    } else if let Some(s) = e.status() {
        status_err(s)
    } else {
        "Network error".into()
    }
}

pub fn status_err(s: reqwest::StatusCode) -> String {
    match s.as_u16() {
        401 => "Sign-in expired or wrong credentials".into(),
        403 => "Access denied".into(),
        404 => "Not found".into(),
        429 => "Too many requests, try again later".into(),
        500..=599 => format!("Server error ({})", s.as_u16()),
        n => format!("HTTP {n}"),
    }
}

pub fn normalize_base(url: &str) -> Result<String, String> {
    let t = url.trim().trim_end_matches('/');
    if t.is_empty() {
        return Err("Enter a server address".into());
    }
    let full = if t.contains("://") {
        t.to_string()
    } else {
        format!("http://{t}")
    };
    let parsed = url::Url::parse(&full).map_err(|_| "Invalid address".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https addresses".into());
    }
    Ok(full)
}

#[derive(Serialize, Clone, Debug)]
pub struct SourceInfo {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub updated_at: i64,
    pub extra: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct SourceRow {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

const ROW_COLS: &str = "id, kind, name, url, config_json, enabled, status, last_error, updated_at";

fn map_row(r: &rusqlite::Row) -> rusqlite::Result<SourceRow> {
    let cfg: String = r.get(4)?;
    Ok(SourceRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        url: r.get(3)?,
        config: serde_json::from_str(&cfg).unwrap_or_else(|_| serde_json::json!({})),
        enabled: r.get::<_, i64>(5)? != 0,
        status: r.get(6)?,
        last_error: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

pub fn get(id: i64) -> Result<SourceRow, String> {
    with(|c| {
        c.query_row(
            &format!("SELECT {ROW_COLS} FROM sources WHERE id = ?1"),
            [id],
            map_row,
        )
        .optional()
    })?
    .ok_or_else(|| "Source not found".to_string())
}

pub fn get_kind(id: i64, kind: &str) -> Result<SourceRow, String> {
    let row = get(id)?;
    if row.kind != kind {
        return Err("Source not found".into());
    }
    Ok(row)
}

pub fn list(kind: Option<&str>) -> Result<Vec<SourceRow>, String> {
    with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {ROW_COLS} FROM sources WHERE ?1 IS NULL OR kind = ?1 ORDER BY position, id"
        ))?;
        let rows = st
            .query_map(params![kind], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn upsert(
    kind: &str,
    name: &str,
    url: &str,
    config: &serde_json::Value,
) -> Result<i64, String> {
    let key = config
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or(url)
        .to_string();
    let now = now_ms();
    let cfg = config.to_string();
    with(|c| {
        let existing: Option<i64> = c
            .query_row(
                "SELECT id FROM sources WHERE kind = ?1 AND json_extract(config_json, '$.key') = ?2",
                params![kind, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            c.execute(
                "UPDATE sources SET name = ?2, url = ?3, config_json = ?4, status = 'ok',
                 last_error = NULL, fails = 0, updated_at = ?5 WHERE id = ?1",
                params![id, name, url, cfg, now],
            )?;
            return Ok(id);
        }
        let pos: i64 = c.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM sources",
            [],
            |r| r.get(0),
        )?;
        c.execute(
            "INSERT INTO sources(kind, name, url, config_json, enabled, status, position, updated_at)
             VALUES(?1, ?2, ?3, ?4, 1, 'ok', ?5, ?6)",
            params![kind, name, url, cfg, pos, now],
        )?;
        Ok(c.last_insert_rowid())
    })
}

pub fn set_config(id: i64, config: &serde_json::Value) -> Result<(), String> {
    with(|c| {
        c.execute(
            "UPDATE sources SET config_json = ?2 WHERE id = ?1",
            params![id, config.to_string()],
        )
        .map(|_| ())
    })
}

pub fn info(row: &SourceRow) -> SourceInfo {
    let mut extra = serde_json::Map::new();
    match row.kind.as_str() {
        "jellyfin" => {
            for k in ["user", "server_name", "server_id", "product"] {
                if let Some(v) = row.config.get(k) {
                    extra.insert(k.into(), v.clone());
                }
            }
        }
        "iptv" => {
            let (channels, programmes): (i64, i64) = with(|c| {
                Ok((
                    c.query_row(
                        "SELECT COUNT(*) FROM iptv_channels WHERE source = ?1",
                        [row.id],
                        |r| r.get(0),
                    )?,
                    c.query_row(
                        "SELECT COUNT(*) FROM epg_programmes WHERE source = ?1",
                        [row.id],
                        |r| r.get(0),
                    )?,
                ))
            })
            .unwrap_or((0, 0));
            extra.insert("channels".into(), channels.into());
            extra.insert(
                "epg".into(),
                row.config
                    .get("epg_url")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            );
            extra.insert("programmes".into(), programmes.into());
            extra.insert(
                "type".into(),
                row.config
                    .get("type")
                    .cloned()
                    .unwrap_or_else(|| "m3u".into()),
            );
            for k in ["refreshed_at", "epg_refreshed_at", "expires_at"] {
                if let Some(v) = row.config.get(k) {
                    extra.insert(k.into(), v.clone());
                }
            }
            if let Some(last) = kv_get("iptv_last_channel").and_then(|v| v.parse::<i64>().ok()) {
                extra.insert("last_channel".into(), last.into());
            }
        }
        "addon" => {
            if let Some(m) = row.config.get("manifest") {
                extra.insert("manifest".into(), m.clone());
            }
        }
        _ => {}
    }
    SourceInfo {
        id: row.id,
        kind: row.kind.clone(),
        name: row.name.clone(),
        url: row.url.clone(),
        enabled: row.enabled,
        status: if row.enabled {
            row.status.clone()
        } else {
            "disabled".into()
        },
        last_error: row.last_error.clone(),
        updated_at: row.updated_at,
        extra: serde_json::Value::Object(extra),
    }
}

pub fn info_by_id(id: i64) -> Result<SourceInfo, String> {
    Ok(info(&get(id)?))
}

pub fn mark_status(id: i64, res: Result<(), &str>, elapsed: Duration) {
    let now = now_ms();
    let _ = with(|c| match res {
        Ok(()) => {
            let status = if elapsed > SLOW_AFTER { "slow" } else { "ok" };
            c.execute(
                "UPDATE sources SET status = ?2, fails = 0, last_error = NULL,
                 updated_at = CASE WHEN status = ?2 THEN updated_at ELSE ?3 END WHERE id = ?1",
                params![id, status, now],
            )
        }
        Err(e) => c.execute(
            "UPDATE sources SET fails = fails + 1, last_error = ?2,
             status = CASE WHEN fails + 1 >= ?3 THEN 'down' ELSE status END,
             updated_at = CASE WHEN fails + 1 >= ?3 AND status != 'down' THEN ?4 ELSE updated_at END
             WHERE id = ?1",
            params![id, e, DOWN_AFTER, now],
        ),
    });
}

pub async fn tracked<T>(
    id: i64,
    limit: Duration,
    fut: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let t = Instant::now();
    let r = match tokio::time::timeout(limit, fut).await {
        Ok(r) => r,
        Err(_) => Err("Timed out".to_string()),
    };
    mark_status(
        id,
        r.as_ref().map(|_| ()).map_err(|e| e.as_str()),
        t.elapsed(),
    );
    r
}

pub fn skip_for_now(row: &SourceRow) -> bool {
    !row.enabled || (row.status == "down" && now_ms() - row.updated_at < DOWN_RETRY_MS)
}

pub async fn resolve(
    app: &AppHandle,
    source: SourceRef,
    for_cast: bool,
) -> Result<PlaybackInfo, String> {
    match source {
        SourceRef::Local { .. } | SourceRef::Url { .. } => {
            let info = super::resolve_play(app, source.clone(), None).await?;
            if for_cast && matches!(info.mode, PlayMode::Remux | PlayMode::TranscodeAudio) {
                return super::resolve_play(app, source, Some(PlayMode::Transcode)).await;
            }
            Ok(info)
        }
        SourceRef::Jellyfin { source, item } => super::jellyfin::play(app, source, &item).await,
        SourceRef::Iptv { channel } => super::iptv::play(app, channel).await,
        SourceRef::Addon {
            addon,
            typ,
            id,
            stream_index,
        } => super::addons::replay(app, addon, &typ, &id, stream_index).await,
    }
}

#[tauri::command]
pub async fn video_sources() -> Result<Vec<SourceInfo>, String> {
    tokio::task::spawn_blocking(|| Ok(list(None)?.iter().map(info).collect()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn video_source_remove(id: i64) -> Result<(), String> {
    let row = get(id)?;
    match row.kind.as_str() {
        "jellyfin" => secret_del(&format!("jf:{id}")),
        "iptv" => secret_del(&format!("xtream:{id}")),
        _ => {}
    }
    tokio::task::spawn_blocking(move || {
        with_mut(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM iptv_channels WHERE source = ?1", [id])?;
            tx.execute("DELETE FROM epg_programmes WHERE source = ?1", [id])?;
            tx.execute("DELETE FROM epg_staging WHERE source = ?1", [id])?;
            tx.execute("DELETE FROM epg_channels WHERE source = ?1", [id])?;
            tx.execute("DELETE FROM sources WHERE id = ?1", [id])?;
            tx.commit()
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(dir) = db::video_dir() {
        let _ = std::fs::remove_file(dir.join("epg").join(format!("{id}.xml")));
    }
    Ok(())
}

#[tauri::command]
pub async fn video_source_toggle(id: i64, enabled: bool) -> Result<(), String> {
    with(|c| {
        c.execute(
            "UPDATE sources SET enabled = ?2, fails = 0,
             status = CASE WHEN ?2 = 1 THEN 'ok' ELSE status END, updated_at = ?3 WHERE id = ?1",
            params![id, enabled as i64, now_ms()],
        )
        .map(|_| ())
    })
}
