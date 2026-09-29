//! `video.db`: the video library, courses, watch state, notes, cards and
//! transcripts. The source modules (Jellyfin, IPTV, addons, AniList) add their
//! own tables through `with` with `CREATE TABLE IF NOT EXISTS`.
//!
//! Kept apart from `omniget.db` for the same reason as `music.db`: a scan
//! writes thousands of rows and must not hold the download queue's lock.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{MediaItem, MediaKind, Progress, Root, RootKind, SourceRef};

pub fn video_dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("video"))
}

/// `video/cache/<sub>`, created on demand. Thumbnails, extracted subtitles and
/// silence maps live here, never next to the user's files.
pub fn cache_dir(sub: &str) -> Option<PathBuf> {
    let d = video_dir()?.join("cache").join(sub);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// `video/img`: frames captured for notes.
pub fn img_dir() -> Option<PathBuf> {
    let d = video_dir()?.join("img");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn open() -> Option<Connection> {
    let dir = video_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("video.db")).ok()?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    if let Err(e) = schema(&conn) {
        tracing::error!("[video] schema failed: {e}");
        return None;
    }
    Some(conn)
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("video library unavailable")?;
    let guard = m.lock().map_err(|_| "video library lock poisoned")?;
    f(&guard).map_err(|e| e.to_string())
}

pub fn with_mut<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("video library unavailable")?;
    let mut guard = m.lock().map_err(|_| "video library lock poisoned")?;
    f(&mut guard).map_err(|e| e.to_string())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS roots (
            path TEXT PRIMARY KEY,
            kind TEXT NOT NULL DEFAULT 'library',
            added_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS media (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            root TEXT NOT NULL,
            folder TEXT NOT NULL,
            rel TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            mtime_ms INTEGER NOT NULL DEFAULT 0,
            hash TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL DEFAULT 'video',
            title TEXT NOT NULL DEFAULT '',
            show TEXT,
            season INTEGER,
            episode INTEGER,
            episode_end INTEGER,
            year INTEGER,
            lesson_no INTEGER,
            module TEXT,
            module_index INTEGER,
            number TEXT,
            ord INTEGER NOT NULL DEFAULT 0,
            course_id INTEGER,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            width INTEGER,
            height INTEGER,
            vcodec TEXT NOT NULL DEFAULT '',
            acodec TEXT NOT NULL DEFAULT '',
            container TEXT NOT NULL DEFAULT '',
            bitrate INTEGER,
            streams_json TEXT NOT NULL DEFAULT '[]',
            chapters_json TEXT NOT NULL DEFAULT '[]',
            probe_json TEXT NOT NULL DEFAULT '',
            probed INTEGER NOT NULL DEFAULT 0,
            has_subs INTEGER NOT NULL DEFAULT 0,
            thumb TEXT,
            added_at INTEGER NOT NULL,
            offline INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS media_root ON media(root);
        CREATE INDEX IF NOT EXISTS media_folder ON media(folder);
        CREATE INDEX IF NOT EXISTS media_hash ON media(hash);
        CREATE INDEX IF NOT EXISTS media_show ON media(show);
        CREATE INDEX IF NOT EXISTS media_course ON media(course_id);
        CREATE TABLE IF NOT EXISTS courses (
            id INTEGER PRIMARY KEY,
            root_path TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            attachments_json TEXT NOT NULL DEFAULT '[]'
        );
        CREATE TABLE IF NOT EXISTS progress (
            key TEXT PRIMARY KEY,
            position_ms INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            watched INTEGER NOT NULL DEFAULT 0,
            play_count INTEGER NOT NULL DEFAULT 0,
            last_played INTEGER,
            seen BLOB,
            hidden INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS progress_last ON progress(last_played);
        CREATE TABLE IF NOT EXISTS prefs (
            scope TEXT PRIMARY KEY,
            json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS notes (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            t_ms INTEGER NOT NULL,
            t_end_ms INTEGER,
            text TEXT NOT NULL DEFAULT '',
            image TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS notes_key ON notes(key, t_ms);
        CREATE TABLE IF NOT EXISTS cards (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            front TEXT NOT NULL,
            back TEXT NOT NULL,
            t_ms INTEGER,
            image TEXT,
            due INTEGER NOT NULL,
            interval_days REAL NOT NULL DEFAULT 0,
            ease REAL NOT NULL DEFAULT 2.5,
            reps INTEGER NOT NULL DEFAULT 0,
            lapses INTEGER NOT NULL DEFAULT 0,
            anki_id INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS cards_due ON cards(due);
        CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY,
            key TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            subtitle TEXT NOT NULL DEFAULT '',
            source_json TEXT NOT NULL,
            at INTEGER NOT NULL,
            position_ms INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS history_at ON history(at);
        CREATE TABLE IF NOT EXISTS items_meta (
            key TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            subtitle TEXT NOT NULL DEFAULT '',
            thumb TEXT,
            source_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS transcripts (
            key TEXT PRIMARY KEY,
            lang TEXT NOT NULL DEFAULT '',
            cues_json TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS transcripts_fts USING fts5(
            key, start_ms UNINDEXED, text, tokenize = 'unicode61 remove_diacritics 2'
        );",
    )
}

// ---------- roots ----------

pub fn roots() -> Result<Vec<Root>, String> {
    with(|c| {
        let mut st = c.prepare("SELECT path, kind, added_at FROM roots ORDER BY added_at")?;
        let rows = st.query_map([], |r| {
            let path: String = r.get(0)?;
            let kind: String = r.get(1)?;
            Ok(Root {
                online: std::path::Path::new(&path).is_dir(),
                path,
                kind: RootKind::parse(&kind),
                added_at: r.get(2)?,
            })
        })?;
        rows.collect()
    })
}

// ---------- media ----------

pub const MEDIA_COLS: &str = "m.id, m.path, m.root, m.folder, m.kind, m.title, m.show, m.season, \
     m.episode, m.episode_end, m.year, m.lesson_no, m.module, m.course_id, m.duration_ms, m.width, \
     m.height, m.vcodec, m.acodec, m.container, m.size, m.has_subs, m.added_at, m.offline, m.hash, \
     p.position_ms, p.duration_ms, p.watched, p.play_count, p.last_played, p.seen, m.number, \
     m.module_index, m.ord, m.rel";

/// `FROM media m LEFT JOIN progress p` with the key derived from the hash.
pub const MEDIA_FROM: &str = "FROM media m LEFT JOIN progress p ON p.key = 'local:' || m.hash";

/// A media row plus the course placement columns that `MediaItem` does not
/// carry.
#[derive(Clone, Debug)]
pub struct MediaRow {
    pub item: MediaItem,
    pub hash: String,
    pub number: Option<String>,
    pub module_index: Option<i64>,
    pub ord: i64,
    pub rel: String,
}

pub fn media_key(hash: &str) -> String {
    format!("local:{hash}")
}

pub fn seen_ratio(seen: Option<Vec<u8>>, duration_ms: i64) -> f64 {
    match seen {
        Some(b) if duration_ms > 0 => {
            let bits = omniget_core::core::video::progress::SeenBits::from_bytes(b);
            bits.ratio((duration_ms / 1000).max(1) as u64)
        }
        _ => 0.0,
    }
}

pub fn media_from_row(r: &Row) -> rusqlite::Result<MediaRow> {
    let id: i64 = r.get(0)?;
    let hash: String = r.get(24)?;
    let key = media_key(&hash);
    let media_dur: i64 = r.get(14)?;
    let progress = match r.get::<_, Option<i64>>(25)? {
        Some(pos) => {
            let pdur: i64 = r.get(26)?;
            let dur = if pdur > 0 { pdur } else { media_dur };
            Some(Progress {
                key: key.clone(),
                position_ms: pos,
                duration_ms: dur,
                watched: r.get::<_, i64>(27)? != 0,
                play_count: r.get(28)?,
                last_played: r.get(29)?,
                seen_ratio: seen_ratio(r.get(30)?, dur),
            })
        }
        None => None,
    };
    let kind: String = r.get(4)?;
    Ok(MediaRow {
        item: MediaItem {
            id,
            key,
            path: r.get(1)?,
            root: r.get(2)?,
            folder: r.get(3)?,
            kind: MediaKind::parse(&kind),
            title: r.get(5)?,
            show: r.get(6)?,
            season: r.get(7)?,
            episode: r.get(8)?,
            episode_end: r.get(9)?,
            year: r.get(10)?,
            lesson_no: r.get(11)?,
            module: r.get(12)?,
            course_id: r.get(13)?,
            duration_ms: media_dur,
            width: r.get(15)?,
            height: r.get(16)?,
            vcodec: r.get(17)?,
            acodec: r.get(18)?,
            container: r.get(19)?,
            size: r.get(20)?,
            has_subs: r.get::<_, i64>(21)? != 0,
            thumb: Some(format!("/thumb/{id}")),
            added_at: r.get(22)?,
            offline: r.get::<_, i64>(23)? != 0,
            progress,
        },
        hash,
        number: r.get(31)?,
        module_index: r.get(32)?,
        ord: r.get(33)?,
        rel: r.get(34)?,
    })
}

/// Rows matching a `WHERE` clause (may be empty), unsorted.
pub fn media_where(clause: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<MediaRow>, String> {
    let sql = format!("SELECT {MEDIA_COLS} {MEDIA_FROM} {clause}");
    with(|c| {
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(args, media_from_row)?;
        rows.collect()
    })
}

pub fn media_by_id(id: i64) -> Result<MediaRow, String> {
    media_where("WHERE m.id = ?1", &[&id])?
        .into_iter()
        .next()
        .ok_or_else(|| "video not found".to_string())
}

pub fn media_by_hash(hash: &str) -> Result<Option<MediaRow>, String> {
    Ok(
        media_where("WHERE m.hash = ?1 ORDER BY m.offline, m.id", &[&hash])?
            .into_iter()
            .next(),
    )
}

/// Path, hash and video codec for the server routes.
pub fn media_file(id: i64) -> Option<(String, String, String, i64)> {
    with(|c| {
        c.query_row(
            "SELECT path, hash, vcodec, duration_ms FROM media WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
    })
    .ok()
    .flatten()
}

// ---------- progress ----------

/// Progress, the raw seen bitmap and the hidden flag.
pub type ProgressRow = (Progress, Option<Vec<u8>>, bool);

pub fn progress_get(key: &str) -> Result<Option<ProgressRow>, String> {
    with(|c| {
        c.query_row(
            "SELECT position_ms, duration_ms, watched, play_count, last_played, seen, hidden
             FROM progress WHERE key = ?1",
            [key],
            |r| {
                let dur: i64 = r.get(1)?;
                let seen: Option<Vec<u8>> = r.get(5)?;
                Ok((
                    Progress {
                        key: key.to_string(),
                        position_ms: r.get(0)?,
                        duration_ms: dur,
                        watched: r.get::<_, i64>(2)? != 0,
                        play_count: r.get(3)?,
                        last_played: r.get(4)?,
                        seen_ratio: seen_ratio(seen.clone(), dur),
                    },
                    seen,
                    r.get::<_, i64>(6)? != 0,
                ))
            },
        )
        .optional()
    })
}

// ---------- items_meta ----------

pub struct ItemMeta {
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<String>,
    pub source: Option<SourceRef>,
}

pub fn item_meta(key: &str) -> Result<Option<ItemMeta>, String> {
    with(|c| {
        c.query_row(
            "SELECT title, subtitle, thumb, source_json FROM items_meta WHERE key = ?1",
            [key],
            |r| {
                let src: String = r.get(3)?;
                Ok(ItemMeta {
                    title: r.get(0)?,
                    subtitle: r.get(1)?,
                    thumb: r.get(2)?,
                    source: serde_json::from_str(&src).ok(),
                })
            },
        )
        .optional()
    })
}

pub fn set_item_meta(
    key: &str,
    title: &str,
    subtitle: &str,
    thumb: Option<&str>,
    source: &SourceRef,
) -> Result<(), String> {
    let src = serde_json::to_string(source).map_err(|e| e.to_string())?;
    with(|c| {
        c.execute(
            "INSERT INTO items_meta (key, title, subtitle, thumb, source_json)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET title = excluded.title,
               subtitle = excluded.subtitle, thumb = COALESCE(excluded.thumb, items_meta.thumb),
               source_json = excluded.source_json",
            params![key, title, subtitle, thumb, src],
        )
    })?;
    Ok(())
}

// ---------- prefs ----------

pub fn pref_json(scope: &str) -> Result<Option<serde_json::Value>, String> {
    let raw: Option<String> = with(|c| {
        c.query_row("SELECT json FROM prefs WHERE scope = ?1", [scope], |r| {
            r.get(0)
        })
        .optional()
    })?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}
