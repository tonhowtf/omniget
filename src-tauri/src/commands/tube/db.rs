use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use omniget_core::core::tube::{AutoRule, ChannelRef, HistoryEntry, LocalSub, VideoItem};
use rusqlite::{params, Connection, OptionalExtension, Row};

pub fn tube_dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("tube"))
}

fn open() -> Option<Connection> {
    let dir = tube_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("tube.db")).ok()?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    if let Err(e) = schema(&conn) {
        tracing::error!("[tube] schema failed: {e}");
        return None;
    }
    Some(conn)
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("tube database unavailable")?;
    let guard = m.lock().map_err(|_| "tube database lock poisoned")?;
    f(&guard).map_err(|e| e.to_string())
}

pub fn with_mut<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("tube database unavailable")?;
    let mut guard = m.lock().map_err(|_| "tube database lock poisoned")?;
    f(&mut guard).map_err(|e| e.to_string())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS subs (
            channel_id TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            avatar TEXT,
            added_ms INTEGER NOT NULL,
            last_checked_ms INTEGER,
            notify INTEGER NOT NULL DEFAULT 0,
            auto_download INTEGER NOT NULL DEFAULT 0,
            fail_count INTEGER NOT NULL DEFAULT 0,
            next_check_ms INTEGER NOT NULL DEFAULT 0,
            etag TEXT
        );
        CREATE TABLE IF NOT EXISTS feed (
            channel_id TEXT NOT NULL,
            video_id TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            published_ms INTEGER,
            duration_s INTEGER,
            is_short INTEGER,
            is_live INTEGER NOT NULL DEFAULT 0,
            views INTEGER,
            thumb TEXT,
            state TEXT NOT NULL DEFAULT 'seen',
            path TEXT,
            bytes INTEGER,
            downloaded_ms INTEGER,
            pinned INTEGER NOT NULL DEFAULT 0,
            rule_id TEXT,
            dl_id INTEGER,
            queued_ms INTEGER,
            PRIMARY KEY (channel_id, video_id)
        );
        CREATE INDEX IF NOT EXISTS feed_published ON feed(published_ms);
        CREATE INDEX IF NOT EXISTS feed_state ON feed(state);
        CREATE TABLE IF NOT EXISTS history (
            video_id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            channel_id TEXT,
            channel_name TEXT,
            thumb TEXT,
            duration_s INTEGER,
            position_s INTEGER NOT NULL DEFAULT 0,
            watched_ms INTEGER NOT NULL,
            kind TEXT NOT NULL DEFAULT 'video',
            count INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS history_watched ON history(watched_ms);
        CREATE TABLE IF NOT EXISTS watch_later (
            video_id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            channel_id TEXT,
            channel_name TEXT,
            thumb TEXT,
            duration_s INTEGER,
            added_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rules (
            id TEXT PRIMARY KEY,
            json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS cache (
            key TEXT PRIMARY KEY,
            json TEXT NOT NULL,
            at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playlists (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            video_ids TEXT NOT NULL DEFAULT '[]',
            added_ms INTEGER NOT NULL
        );",
    )
}

#[derive(Debug, Clone, Default)]
pub struct SubRow {
    pub channel_id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub added_ms: i64,
    pub last_checked_ms: Option<i64>,
    pub notify: bool,
    pub auto_download: bool,
    pub fail_count: u32,
    pub next_check_ms: i64,
}

impl SubRow {
    pub fn to_local(&self) -> LocalSub {
        LocalSub {
            channel_id: self.channel_id.clone(),
            name: self.name.clone(),
            avatar: self.avatar.clone(),
            added_ms: self.added_ms,
            last_checked_ms: self.last_checked_ms,
            notify: self.notify,
            auto_download: self.auto_download,
        }
    }
}

const SUB_COLS: &str = "channel_id, name, avatar, added_ms, last_checked_ms, notify, \
     auto_download, fail_count, next_check_ms";

fn sub_from_row(r: &Row) -> rusqlite::Result<SubRow> {
    Ok(SubRow {
        channel_id: r.get(0)?,
        name: r.get(1)?,
        avatar: r.get(2)?,
        added_ms: r.get(3)?,
        last_checked_ms: r.get(4)?,
        notify: r.get::<_, i64>(5)? != 0,
        auto_download: r.get::<_, i64>(6)? != 0,
        fail_count: r.get::<_, i64>(7)?.max(0) as u32,
        next_check_ms: r.get(8)?,
    })
}

pub fn subs_all(c: &Connection) -> rusqlite::Result<Vec<SubRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {SUB_COLS} FROM subs ORDER BY name COLLATE NOCASE"
    ))?;
    let rows = st.query_map([], sub_from_row)?;
    rows.collect()
}

pub fn sub_get(c: &Connection, channel_id: &str) -> rusqlite::Result<Option<SubRow>> {
    c.query_row(
        &format!("SELECT {SUB_COLS} FROM subs WHERE channel_id = ?1"),
        [channel_id],
        sub_from_row,
    )
    .optional()
}

pub fn sub_upsert(
    c: &Connection,
    channel_id: &str,
    name: &str,
    avatar: Option<&str>,
    now: i64,
) -> rusqlite::Result<bool> {
    let n = c.execute(
        "INSERT OR IGNORE INTO subs (channel_id, name, avatar, added_ms) VALUES (?1, ?2, ?3, ?4)",
        params![channel_id, name, avatar, now],
    )?;
    if n == 0 {
        c.execute(
            "UPDATE subs SET
                name = CASE WHEN ?2 <> '' THEN ?2 ELSE name END,
                avatar = COALESCE(?3, avatar)
             WHERE channel_id = ?1",
            params![channel_id, name, avatar],
        )?;
    }
    Ok(n > 0)
}

pub fn sub_remove(c: &Connection, channel_id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM subs WHERE channel_id = ?1", [channel_id])?;
    c.execute(
        "DELETE FROM feed WHERE channel_id = ?1 AND path IS NULL AND state <> 'queued'",
        [channel_id],
    )?;
    Ok(())
}

pub fn sub_checked_ok(
    c: &Connection,
    channel_id: &str,
    now: i64,
    next_check_ms: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE subs SET last_checked_ms = ?2, next_check_ms = ?3, fail_count = 0
         WHERE channel_id = ?1",
        params![channel_id, now, next_check_ms],
    )?;
    Ok(())
}

pub fn sub_checked_fail(
    c: &Connection,
    channel_id: &str,
    fail_count: u32,
    next_check_ms: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE subs SET fail_count = ?2, next_check_ms = ?3 WHERE channel_id = ?1",
        params![channel_id, fail_count as i64, next_check_ms],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FeedRow {
    pub channel_id: String,
    pub video_id: String,
    pub title: String,
    pub published_ms: Option<i64>,
    pub duration_s: Option<u32>,
    pub is_short: Option<bool>,
    pub is_live: bool,
    pub views: Option<u64>,
    pub thumb: Option<String>,
    pub state: String,
    pub path: Option<String>,
    pub bytes: Option<u64>,
    pub downloaded_ms: Option<i64>,
    pub pinned: bool,
    pub rule_id: Option<String>,
    pub dl_id: Option<u64>,
    pub queued_ms: Option<i64>,
}

const FEED_COLS: &str = "channel_id, video_id, title, published_ms, duration_s, is_short, \
     is_live, views, thumb, state, path, bytes, downloaded_ms, pinned, rule_id, dl_id, queued_ms";

fn feed_from_row(r: &Row) -> rusqlite::Result<FeedRow> {
    Ok(FeedRow {
        channel_id: r.get(0)?,
        video_id: r.get(1)?,
        title: r.get(2)?,
        published_ms: r.get(3)?,
        duration_s: r.get::<_, Option<i64>>(4)?.map(|v| v.max(0) as u32),
        is_short: r.get::<_, Option<i64>>(5)?.map(|v| v != 0),
        is_live: r.get::<_, i64>(6)? != 0,
        views: r.get::<_, Option<i64>>(7)?.map(|v| v.max(0) as u64),
        thumb: r.get(8)?,
        state: r.get(9)?,
        path: r.get(10)?,
        bytes: r.get::<_, Option<i64>>(11)?.map(|v| v.max(0) as u64),
        downloaded_ms: r.get(12)?,
        pinned: r.get::<_, i64>(13)? != 0,
        rule_id: r.get(14)?,
        dl_id: r.get::<_, Option<i64>>(15)?.map(|v| v as u64),
        queued_ms: r.get(16)?,
    })
}

fn best_thumb(v: &VideoItem) -> Option<String> {
    v.thumbs
        .iter()
        .max_by_key(|t| t.width)
        .map(|t| t.url.clone())
}

pub fn feed_upsert(
    c: &Connection,
    channel_id: &str,
    v: &VideoItem,
    is_short: Option<bool>,
) -> rusqlite::Result<bool> {
    let is_live = v.badges.iter().any(|b| {
        matches!(
            b,
            omniget_core::core::tube::Badge::Live | omniget_core::core::tube::Badge::Upcoming
        )
    });
    let thumb = best_thumb(v);
    let n = c.execute(
        "INSERT OR IGNORE INTO feed (channel_id, video_id, title, published_ms, duration_s,
            is_short, is_live, views, thumb)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            channel_id,
            v.id,
            v.title,
            v.published_ms,
            v.duration_s,
            is_short,
            is_live,
            v.views.map(|n| n as i64),
            thumb
        ],
    )?;
    if n == 0 {
        c.execute(
            "UPDATE feed SET
                title = CASE WHEN ?3 <> '' THEN ?3 ELSE title END,
                published_ms = COALESCE(published_ms, ?4),
                duration_s = COALESCE(?5, duration_s),
                is_short = COALESCE(?6, is_short),
                is_live = ?7,
                views = COALESCE(?8, views),
                thumb = COALESCE(?9, thumb)
             WHERE channel_id = ?1 AND video_id = ?2",
            params![
                channel_id,
                v.id,
                v.title,
                v.published_ms,
                v.duration_s,
                is_short,
                is_live,
                v.views.map(|n| n as i64),
                thumb
            ],
        )?;
    }
    Ok(n > 0)
}

pub fn feed_recent(c: &Connection, limit: u32) -> rusqlite::Result<Vec<FeedRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {FEED_COLS} FROM feed
         WHERE channel_id IN (SELECT channel_id FROM subs)
         ORDER BY COALESCE(published_ms, 0) DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], feed_from_row)?;
    rows.collect()
}

pub fn feed_by_state(c: &Connection, state: &str) -> rusqlite::Result<Vec<FeedRow>> {
    let mut st = c.prepare(&format!("SELECT {FEED_COLS} FROM feed WHERE state = ?1"))?;
    let rows = st.query_map([state], feed_from_row)?;
    rows.collect()
}

pub fn feed_owned(c: &Connection) -> rusqlite::Result<Vec<FeedRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {FEED_COLS} FROM feed
         WHERE state = 'downloaded' AND path IS NOT NULL AND rule_id IS NOT NULL"
    ))?;
    let rows = st.query_map([], feed_from_row)?;
    rows.collect()
}

pub fn feed_published(c: &Connection, channel_id: &str, limit: u32) -> rusqlite::Result<Vec<i64>> {
    let mut st = c.prepare(
        "SELECT published_ms FROM feed WHERE channel_id = ?1 AND published_ms IS NOT NULL
         ORDER BY published_ms DESC LIMIT ?2",
    )?;
    let rows = st.query_map(params![channel_id, limit], |r| r.get(0))?;
    rows.collect()
}

pub fn feed_set_meta(
    c: &Connection,
    channel_id: &str,
    video_id: &str,
    duration_s: Option<u32>,
    is_short: Option<bool>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE feed SET duration_s = COALESCE(?3, duration_s), is_short = COALESCE(?4, is_short)
         WHERE channel_id = ?1 AND video_id = ?2",
        params![channel_id, video_id, duration_s, is_short],
    )?;
    Ok(())
}

pub fn feed_mark_queued(
    c: &Connection,
    channel_id: &str,
    video_id: &str,
    rule_id: &str,
    dl_id: u64,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE feed SET state = 'queued', rule_id = ?3, dl_id = ?4, queued_ms = ?5
         WHERE channel_id = ?1 AND video_id = ?2",
        params![channel_id, video_id, rule_id, dl_id as i64, now],
    )?;
    Ok(())
}

pub fn feed_mark_downloaded(
    c: &Connection,
    channel_id: &str,
    video_id: &str,
    path: &str,
    bytes: u64,
    now: i64,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE feed SET state = 'downloaded', path = ?3, bytes = ?4, downloaded_ms = ?5
         WHERE channel_id = ?1 AND video_id = ?2",
        params![channel_id, video_id, path, bytes as i64, now],
    )?;
    Ok(())
}

pub fn feed_set_state(
    c: &Connection,
    channel_id: &str,
    video_id: &str,
    state: &str,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE feed SET state = ?3 WHERE channel_id = ?1 AND video_id = ?2",
        params![channel_id, video_id, state],
    )?;
    Ok(())
}

const HISTORY_COLS: &str =
    "video_id, title, channel_id, channel_name, thumb, duration_s, position_s, watched_ms, kind";

fn history_from_row(r: &Row) -> rusqlite::Result<HistoryEntry> {
    let channel_id: Option<String> = r.get(2)?;
    let channel_name: Option<String> = r.get(3)?;
    let channel = match (channel_id, channel_name) {
        (None, None) => None,
        (id, name) => Some(ChannelRef {
            id: id.unwrap_or_default(),
            name: name.unwrap_or_default(),
            ..Default::default()
        }),
    };
    Ok(HistoryEntry {
        video_id: r.get(0)?,
        title: r.get(1)?,
        channel,
        thumb: r.get(4)?,
        duration_s: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u32),
        position_s: r.get::<_, i64>(6)?.max(0) as u32,
        watched_ms: r.get(7)?,
        kind: r.get(8)?,
    })
}

pub fn history_upsert(c: &Connection, e: &HistoryEntry) -> rusqlite::Result<()> {
    let (cid, cname) = e
        .channel
        .as_ref()
        .map(|ch| (Some(ch.id.as_str()), Some(ch.name.as_str())))
        .unwrap_or((None, None));
    c.execute(
        "INSERT INTO history (video_id, title, channel_id, channel_name, thumb, duration_s,
            position_s, watched_ms, kind, count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
         ON CONFLICT(video_id) DO UPDATE SET
            title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE title END,
            channel_id = COALESCE(excluded.channel_id, channel_id),
            channel_name = COALESCE(excluded.channel_name, channel_name),
            thumb = COALESCE(excluded.thumb, thumb),
            duration_s = COALESCE(excluded.duration_s, duration_s),
            position_s = excluded.position_s,
            count = count + (CASE WHEN excluded.watched_ms - watched_ms > 3600000 THEN 1 ELSE 0 END),
            watched_ms = MAX(watched_ms, excluded.watched_ms),
            kind = excluded.kind",
        params![
            e.video_id,
            e.title,
            cid,
            cname,
            e.thumb,
            e.duration_s,
            e.position_s,
            e.watched_ms,
            e.kind
        ],
    )?;
    Ok(())
}

pub fn history_import(c: &Connection, e: &HistoryEntry) -> rusqlite::Result<bool> {
    let known: Option<i64> = c
        .query_row(
            "SELECT watched_ms FROM history WHERE video_id = ?1",
            [&e.video_id],
            |r| r.get(0),
        )
        .optional()?;
    match known {
        Some(at) if at >= e.watched_ms => Ok(false),
        Some(_) => {
            c.execute(
                "UPDATE history SET watched_ms = ?2, position_s = MAX(position_s, ?3)
                 WHERE video_id = ?1",
                params![e.video_id, e.watched_ms, e.position_s],
            )?;
            Ok(false)
        }
        None => {
            history_upsert(c, e)?;
            Ok(true)
        }
    }
}

pub fn history_list(c: &Connection, limit: u32) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut st = c.prepare(&format!(
        "SELECT {HISTORY_COLS} FROM history ORDER BY watched_ms DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], history_from_row)?;
    rows.collect()
}

pub fn history_get(c: &Connection, video_id: &str) -> rusqlite::Result<Option<HistoryEntry>> {
    c.query_row(
        &format!("SELECT {HISTORY_COLS} FROM history WHERE video_id = ?1"),
        [video_id],
        history_from_row,
    )
    .optional()
}

pub fn history_remove(c: &Connection, video_id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM history WHERE video_id = ?1", [video_id])?;
    Ok(())
}

pub fn history_clear(c: &Connection) -> rusqlite::Result<()> {
    c.execute("DELETE FROM history", [])?;
    Ok(())
}

pub fn watch_later_add(c: &Connection, v: &VideoItem, now: i64) -> rusqlite::Result<()> {
    let (cid, cname) = v
        .channel
        .as_ref()
        .map(|ch| (Some(ch.id.clone()), Some(ch.name.clone())))
        .unwrap_or((None, None));
    c.execute(
        "INSERT INTO watch_later (video_id, title, channel_id, channel_name, thumb, duration_s, added_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(video_id) DO UPDATE SET
            title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE title END,
            thumb = COALESCE(excluded.thumb, thumb),
            duration_s = COALESCE(excluded.duration_s, duration_s)",
        params![v.id, v.title, cid, cname, best_thumb(v), v.duration_s, now],
    )?;
    Ok(())
}

pub fn watch_later_remove(c: &Connection, video_id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM watch_later WHERE video_id = ?1", [video_id])?;
    Ok(())
}

pub fn watch_later_list(c: &Connection, limit: u32) -> rusqlite::Result<Vec<VideoItem>> {
    let mut st = c.prepare(
        "SELECT video_id, title, channel_id, channel_name, thumb, duration_s
         FROM watch_later ORDER BY added_ms DESC LIMIT ?1",
    )?;
    let rows = st.query_map([limit], |r| {
        let cid: Option<String> = r.get(2)?;
        let cname: Option<String> = r.get(3)?;
        let thumb: Option<String> = r.get(4)?;
        Ok(VideoItem {
            id: r.get(0)?,
            title: r.get(1)?,
            channel: cid.map(|id| ChannelRef {
                id,
                name: cname.unwrap_or_default(),
                ..Default::default()
            }),
            thumbs: thumb
                .map(|url| {
                    vec![omniget_core::core::tube::Thumb {
                        url,
                        width: 0,
                        height: 0,
                    }]
                })
                .unwrap_or_default(),
            duration_s: r.get::<_, Option<i64>>(5)?.map(|v| v.max(0) as u32),
            ..Default::default()
        })
    })?;
    rows.collect()
}

pub fn watch_later_has(c: &Connection, video_id: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT 1 FROM watch_later WHERE video_id = ?1",
        [video_id],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
}

pub fn rules_all(c: &Connection) -> rusqlite::Result<Vec<AutoRule>> {
    let mut st = c.prepare("SELECT json FROM rules ORDER BY rowid")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for json in rows {
        if let Ok(rule) = serde_json::from_str::<AutoRule>(&json?) {
            out.push(rule);
        }
    }
    Ok(out)
}

pub fn rule_put(c: &Connection, rule: &AutoRule) -> rusqlite::Result<()> {
    let json = serde_json::to_string(rule).unwrap_or_else(|_| "{}".into());
    c.execute(
        "INSERT INTO rules (id, json) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json",
        params![rule.id, json],
    )?;
    Ok(())
}

pub fn rule_remove(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM rules WHERE id = ?1", [id])?;
    Ok(())
}

pub fn cache_get(c: &Connection, key: &str, max_age_ms: i64, now: i64) -> Option<String> {
    c.query_row(
        "SELECT json FROM cache WHERE key = ?1 AND at_ms >= ?2",
        params![key, now - max_age_ms],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

pub fn cache_put(c: &Connection, key: &str, json: &str, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO cache (key, json, at_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET json = excluded.json, at_ms = excluded.at_ms",
        params![key, json, now],
    )?;
    Ok(())
}

pub fn cache_prune(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM cache WHERE at_ms < ?1", [now - 24 * 3_600_000])?;
    Ok(())
}

pub fn playlist_import(
    c: &Connection,
    title: &str,
    video_ids: &[String],
    now: i64,
) -> rusqlite::Result<()> {
    let json = serde_json::to_string(video_ids).unwrap_or_else(|_| "[]".into());
    c.execute(
        "INSERT INTO playlists (title, video_ids, added_ms) VALUES (?1, ?2, ?3)",
        params![title, json, now],
    )?;
    Ok(())
}
