use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use omniget_core::core::music::tags::TrackMeta;

pub fn music_dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("music"))
}

fn open() -> Option<Connection> {
    let dir = music_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("music.db")).ok()?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    if let Err(e) = schema(&conn) {
        tracing::error!("[music] schema failed: {e}");
        return None;
    }
    Some(conn)
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("music library unavailable")?;
    let guard = m.lock().map_err(|_| "music library lock poisoned")?;
    f(&guard).map_err(|e| e.to_string())
}

pub fn with_mut<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("music library unavailable")?;
    let mut guard = m.lock().map_err(|_| "music library lock poisoned")?;
    f(&mut guard).map_err(|e| e.to_string())
}

fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS roots (
            path TEXT PRIMARY KEY,
            added_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tracks (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            folder TEXT NOT NULL,
            mtime_ms INTEGER NOT NULL DEFAULT 0,
            size INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '',
            artist TEXT NOT NULL DEFAULT '',
            album TEXT NOT NULL DEFAULT '',
            album_artist TEXT NOT NULL DEFAULT '',
            genre TEXT NOT NULL DEFAULT '',
            composer TEXT NOT NULL DEFAULT '',
            comment TEXT NOT NULL DEFAULT '',
            year INTEGER,
            track_no INTEGER,
            track_total INTEGER,
            disc_no INTEGER,
            disc_total INTEGER,
            compilation INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            bitrate INTEGER,
            sample_rate INTEGER,
            bit_depth INTEGER,
            channels INTEGER,
            codec TEXT NOT NULL DEFAULT '',
            cover TEXT,
            has_lyrics INTEGER NOT NULL DEFAULT 0,
            rg_track_gain REAL,
            rg_track_peak REAL,
            rg_album_gain REAL,
            rg_album_peak REAL,
            loudness_lufs REAL,
            loudness_range REAL,
            bpm REAL,
            musical_key TEXT NOT NULL DEFAULT '',
            camelot TEXT NOT NULL DEFAULT '',
            cutoff_hz REAL,
            intro_s REAL,
            outro_s REAL,
            end_s REAL,
            analyzed_at INTEGER,
            play_count INTEGER NOT NULL DEFAULT 0,
            skip_count INTEGER NOT NULL DEFAULT 0,
            last_played INTEGER,
            rating INTEGER NOT NULL DEFAULT 0,
            liked INTEGER NOT NULL DEFAULT 0,
            added_at INTEGER NOT NULL,
            mb_recording_id TEXT NOT NULL DEFAULT '',
            mb_release_id TEXT NOT NULL DEFAULT '',
            source_url TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS tracks_folder ON tracks(folder);
        CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album_artist, album);
        CREATE TABLE IF NOT EXISTS peaks (
            track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
            data BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playlists (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            smart TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playlist_items (
            playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS playlist_items_pl ON playlist_items(playlist_id, position);
        CREATE TABLE IF NOT EXISTS history (
            id INTEGER PRIMARY KEY,
            track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            played_at INTEGER NOT NULL,
            ms_played INTEGER NOT NULL,
            completed INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS history_time ON history(played_at);
        CREATE TABLE IF NOT EXISTS stations (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            url TEXT NOT NULL,
            homepage TEXT NOT NULL DEFAULT '',
            favicon TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            country TEXT NOT NULL DEFAULT '',
            codec TEXT NOT NULL DEFAULT '',
            bitrate INTEGER NOT NULL DEFAULT 0,
            hls INTEGER NOT NULL DEFAULT 0,
            added_at INTEGER NOT NULL,
            last_played INTEGER,
            play_count INTEGER NOT NULL DEFAULT 0
        );",
    )
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRow {
    pub id: i64,
    pub path: String,
    pub folder: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub composer: String,
    pub year: Option<i64>,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub codec: String,
    pub cover: Option<String>,
    pub has_lyrics: bool,
    pub rg_track_gain: Option<f64>,
    pub rg_track_peak: Option<f64>,
    pub rg_album_gain: Option<f64>,
    pub rg_album_peak: Option<f64>,
    pub loudness_lufs: Option<f64>,
    pub bpm: Option<f64>,
    pub musical_key: String,
    pub camelot: String,
    pub cutoff_hz: Option<f64>,
    pub intro_s: Option<f64>,
    pub outro_s: Option<f64>,
    pub end_s: Option<f64>,
    pub analyzed: bool,
    pub play_count: i64,
    pub skip_count: i64,
    pub last_played: Option<i64>,
    pub rating: i64,
    pub liked: bool,
    pub added_at: i64,
    pub size: i64,
    pub source_url: String,
}

pub const TRACK_COLUMNS: &str = "id, path, folder, title, artist, album, album_artist, genre, composer, \
    year, track_no, disc_no, duration_ms, bitrate, sample_rate, bit_depth, codec, cover, has_lyrics, \
    rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak, loudness_lufs, bpm, musical_key, camelot, \
    cutoff_hz, intro_s, outro_s, end_s, analyzed_at, play_count, skip_count, last_played, rating, liked, \
    added_at, size, source_url";

pub fn row_to_track(r: &rusqlite::Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: r.get(0)?,
        path: r.get(1)?,
        folder: r.get(2)?,
        title: r.get(3)?,
        artist: r.get(4)?,
        album: r.get(5)?,
        album_artist: r.get(6)?,
        genre: r.get(7)?,
        composer: r.get(8)?,
        year: r.get(9)?,
        track_no: r.get(10)?,
        disc_no: r.get(11)?,
        duration_ms: r.get(12)?,
        bitrate: r.get(13)?,
        sample_rate: r.get(14)?,
        bit_depth: r.get(15)?,
        codec: r.get(16)?,
        cover: r.get(17)?,
        has_lyrics: r.get::<_, i64>(18)? != 0,
        rg_track_gain: r.get(19)?,
        rg_track_peak: r.get(20)?,
        rg_album_gain: r.get(21)?,
        rg_album_peak: r.get(22)?,
        loudness_lufs: r.get(23)?,
        bpm: r.get(24)?,
        musical_key: r.get(25)?,
        camelot: r.get(26)?,
        cutoff_hz: r.get(27)?,
        intro_s: r.get(28)?,
        outro_s: r.get(29)?,
        end_s: r.get(30)?,
        analyzed: r.get::<_, Option<i64>>(31)?.is_some(),
        play_count: r.get(32)?,
        skip_count: r.get(33)?,
        last_played: r.get(34)?,
        rating: r.get(35)?,
        liked: r.get::<_, i64>(36)? != 0,
        added_at: r.get(37)?,
        size: r.get(38)?,
        source_url: r.get(39)?,
    })
}

pub fn all_tracks() -> Result<Vec<TrackRow>, String> {
    with(|c| {
        let mut st = c.prepare(&format!("SELECT {TRACK_COLUMNS} FROM tracks"))?;
        let rows = st.query_map([], row_to_track)?.collect();
        rows
    })
}

pub fn tracks_by_ids(ids: &[i64]) -> Result<Vec<TrackRow>, String> {
    with(|c| {
        let mut st = c.prepare(&format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1"))?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(t) = st.query_row([id], row_to_track).optional()? {
                out.push(t);
            }
        }
        Ok(out)
    })
}

pub fn track(id: i64) -> Result<TrackRow, String> {
    tracks_by_ids(&[id])?
        .into_iter()
        .next()
        .ok_or_else(|| "track not found".to_string())
}

pub fn track_path(id: i64) -> Result<String, String> {
    with(|c| {
        c.query_row("SELECT path FROM tracks WHERE id = ?1", [id], |r| r.get(0))
            .optional()
    })?
    .ok_or_else(|| "track not found".to_string())
}

pub struct FileStamp {
    pub mtime_ms: i64,
    pub size: i64,
}

pub fn upsert_track(
    c: &Connection,
    path: &str,
    folder: &str,
    stamp: &FileStamp,
    m: &TrackMeta,
    cover: Option<&str>,
    has_lyrics: bool,
) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO tracks (path, folder, mtime_ms, size, title, artist, album, album_artist, genre,
            composer, comment, year, track_no, track_total, disc_no, disc_total, compilation,
            duration_ms, bitrate, sample_rate, bit_depth, channels, codec, cover, has_lyrics,
            rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak, bpm, musical_key,
            mb_recording_id, mb_release_id, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34)
         ON CONFLICT(path) DO UPDATE SET
            folder = excluded.folder, mtime_ms = excluded.mtime_ms, size = excluded.size,
            title = excluded.title, artist = excluded.artist, album = excluded.album,
            album_artist = excluded.album_artist, genre = excluded.genre, composer = excluded.composer,
            comment = excluded.comment, year = excluded.year, track_no = excluded.track_no,
            track_total = excluded.track_total, disc_no = excluded.disc_no,
            disc_total = excluded.disc_total, compilation = excluded.compilation,
            duration_ms = excluded.duration_ms, bitrate = excluded.bitrate,
            sample_rate = excluded.sample_rate, bit_depth = excluded.bit_depth,
            channels = excluded.channels, codec = excluded.codec, cover = excluded.cover,
            has_lyrics = excluded.has_lyrics,
            rg_track_gain = COALESCE(excluded.rg_track_gain, tracks.rg_track_gain),
            rg_track_peak = COALESCE(excluded.rg_track_peak, tracks.rg_track_peak),
            rg_album_gain = COALESCE(excluded.rg_album_gain, tracks.rg_album_gain),
            rg_album_peak = COALESCE(excluded.rg_album_peak, tracks.rg_album_peak),
            bpm = COALESCE(excluded.bpm, tracks.bpm),
            musical_key = CASE WHEN excluded.musical_key <> '' THEN excluded.musical_key ELSE tracks.musical_key END,
            mb_recording_id = excluded.mb_recording_id, mb_release_id = excluded.mb_release_id",
        params![
            path,
            folder,
            stamp.mtime_ms,
            stamp.size,
            m.title,
            m.artist,
            m.album,
            m.album_artist,
            m.genre,
            m.composer,
            m.comment,
            m.year,
            m.track,
            m.track_total,
            m.disc,
            m.disc_total,
            m.compilation as i64,
            m.duration_ms as i64,
            m.bitrate,
            m.sample_rate,
            m.bit_depth,
            m.channels,
            m.codec,
            cover,
            has_lyrics as i64,
            m.rg_track_gain,
            m.rg_track_peak,
            m.rg_album_gain,
            m.rg_album_peak,
            m.bpm.map(|b| b as f64),
            m.musical_key,
            m.mb_recording_id,
            m.mb_release_id,
            now_ms(),
        ],
    )?;
    c.query_row("SELECT id FROM tracks WHERE path = ?1", [path], |r| {
        r.get(0)
    })
}

pub fn stamps_under(root: &str) -> Result<std::collections::HashMap<String, (i64, i64)>, String> {
    with(|c| {
        let mut st =
            c.prepare("SELECT path, mtime_ms, size FROM tracks WHERE path LIKE ?1 ESCAPE '\\'")?;
        let like = format!(
            "{}%",
            root.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let rows = st
            .query_map([like], |r| {
                Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?)))
            })?
            .collect();
        rows
    })
}

pub fn roots() -> Result<Vec<String>, String> {
    with(|c| {
        let mut st = c.prepare("SELECT path FROM roots ORDER BY added_at")?;
        let rows = st.query_map([], |r| r.get(0))?.collect();
        rows
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub favicon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate: i64,
    #[serde(default)]
    pub hls: bool,
    #[serde(default)]
    pub last_played: Option<i64>,
    #[serde(default)]
    pub play_count: i64,
}
