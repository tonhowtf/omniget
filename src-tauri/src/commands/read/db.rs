use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub fn read_dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("read"))
}

pub fn covers_dir() -> Option<PathBuf> {
    read_dir().map(|d| d.join("covers"))
}

fn open() -> Option<Connection> {
    let dir = read_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("read.db")).ok()?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    if let Err(e) = schema(&conn) {
        tracing::error!("[read] schema failed: {e}");
        return None;
    }
    Some(conn)
}

fn handle() -> Option<&'static Mutex<Connection>> {
    static CONN: OnceLock<Option<Mutex<Connection>>> = OnceLock::new();
    CONN.get_or_init(|| open().map(Mutex::new)).as_ref()
}

pub fn with<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("read library unavailable")?;
    let guard = m.lock().map_err(|_| "read library lock poisoned")?;
    f(&guard).map_err(|e| e.to_string())
}

pub fn with_mut<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let m = handle().ok_or("read library unavailable")?;
    let mut guard = m.lock().map_err(|_| "read library lock poisoned")?;
    f(&mut guard).map_err(|e| e.to_string())
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn schema(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS roots (
            path TEXT PRIMARY KEY,
            added_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS series (
            id INTEGER PRIMARY KEY,
            source TEXT NOT NULL,
            source_key TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            alt_titles TEXT NOT NULL DEFAULT '[]',
            authors TEXT NOT NULL DEFAULT '[]',
            description TEXT NOT NULL DEFAULT '',
            genres TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL DEFAULT '',
            lang TEXT NOT NULL DEFAULT '',
            url TEXT NOT NULL DEFAULT '',
            cover_url TEXT NOT NULL DEFAULT '',
            cover TEXT,
            in_library INTEGER NOT NULL DEFAULT 0,
            chapter_langs TEXT NOT NULL DEFAULT '[]',
            reader TEXT NOT NULL DEFAULT '{}',
            notes TEXT NOT NULL DEFAULT '',
            tracking TEXT NOT NULL DEFAULT '{}',
            auto_download INTEGER NOT NULL DEFAULT 0,
            added_at INTEGER NOT NULL,
            checked_at INTEGER,
            new_at INTEGER,
            UNIQUE(source, source_key)
        );
        CREATE TABLE IF NOT EXISTS chapters (
            id INTEGER PRIMARY KEY,
            series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            number REAL,
            volume REAL,
            lang TEXT NOT NULL DEFAULT '',
            scanlator TEXT NOT NULL DEFAULT '',
            published INTEGER,
            pages INTEGER,
            external_url TEXT,
            position INTEGER NOT NULL DEFAULT 0,
            read INTEGER NOT NULL DEFAULT 0,
            bookmarked INTEGER NOT NULL DEFAULT 0,
            last_page INTEGER NOT NULL DEFAULT 0,
            read_at INTEGER,
            fetched_at INTEGER NOT NULL,
            book_id INTEGER,
            UNIQUE(series_id, key)
        );
        CREATE INDEX IF NOT EXISTS chapters_series ON chapters(series_id);
        CREATE TABLE IF NOT EXISTS books (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            folder TEXT NOT NULL,
            format TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            mtime_ms INTEGER NOT NULL DEFAULT 0,
            hash TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            sort_title TEXT NOT NULL DEFAULT '',
            authors TEXT NOT NULL DEFAULT '[]',
            series TEXT NOT NULL DEFAULT '',
            series_index REAL,
            language TEXT NOT NULL DEFAULT '',
            publisher TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            subjects TEXT NOT NULL DEFAULT '[]',
            isbn TEXT NOT NULL DEFAULT '',
            identifiers TEXT NOT NULL DEFAULT '[]',
            published TEXT NOT NULL DEFAULT '',
            cover TEXT,
            pages INTEGER,
            rtl INTEGER NOT NULL DEFAULT 0,
            fixed_layout INTEGER NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT '',
            source_url TEXT NOT NULL DEFAULT '',
            series_id INTEGER REFERENCES series(id) ON DELETE SET NULL,
            chapter_id INTEGER REFERENCES chapters(id) ON DELETE SET NULL,
            rating INTEGER NOT NULL DEFAULT 0,
            favorite INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'unread',
            progress REAL NOT NULL DEFAULT 0,
            location TEXT NOT NULL DEFAULT '',
            last_read_at INTEGER,
            started_at INTEGER,
            finished_at INTEGER,
            read_ms INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]',
            prefs TEXT NOT NULL DEFAULT '{}',
            added_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS books_folder ON books(folder);
        CREATE INDEX IF NOT EXISTS books_series ON books(series);
        CREATE INDEX IF NOT EXISTS books_hash ON books(hash);
        CREATE TABLE IF NOT EXISTS shelves (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            icon TEXT NOT NULL DEFAULT '',
            smart TEXT,
            position INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS shelf_items (
            shelf_id INTEGER NOT NULL REFERENCES shelves(id) ON DELETE CASCADE,
            book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            position INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (shelf_id, book_id)
        );
        CREATE TABLE IF NOT EXISTS categories (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            position INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS series_categories (
            series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
            category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
            PRIMARY KEY (series_id, category_id)
        );
        CREATE TABLE IF NOT EXISTS annotations (
            id INTEGER PRIMARY KEY,
            book_id INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            locator TEXT NOT NULL,
            text TEXT NOT NULL DEFAULT '',
            note TEXT NOT NULL DEFAULT '',
            color TEXT NOT NULL DEFAULT 'yellow',
            style TEXT NOT NULL DEFAULT 'highlight',
            chapter TEXT NOT NULL DEFAULT '',
            progress REAL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS annotations_book ON annotations(book_id);
        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY,
            book_id INTEGER REFERENCES books(id) ON DELETE CASCADE,
            chapter_id INTEGER REFERENCES chapters(id) ON DELETE CASCADE,
            started_at INTEGER NOT NULL,
            ms INTEGER NOT NULL,
            from_progress REAL NOT NULL DEFAULT 0,
            to_progress REAL NOT NULL DEFAULT 0,
            pages INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS sessions_started ON sessions(started_at);
        CREATE TABLE IF NOT EXISTS catalogs (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            url TEXT NOT NULL,
            username TEXT NOT NULL DEFAULT '',
            password TEXT NOT NULL DEFAULT '',
            config TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            added_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kv (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS vocab (
            id INTEGER PRIMARY KEY,
            word TEXT NOT NULL,
            context TEXT NOT NULL DEFAULT '',
            definition TEXT NOT NULL DEFAULT '',
            lang TEXT NOT NULL DEFAULT '',
            book_id INTEGER REFERENCES books(id) ON DELETE SET NULL,
            created_at INTEGER NOT NULL,
            reviewed_at INTEGER,
            box INTEGER NOT NULL DEFAULT 0
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS books_fts USING fts5(
            title, authors, series, description, subjects, tags,
            content='books', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
        );
        CREATE TRIGGER IF NOT EXISTS books_ai AFTER INSERT ON books BEGIN
            INSERT INTO books_fts(rowid, title, authors, series, description, subjects, tags)
            VALUES (new.id, new.title, new.authors, new.series, new.description, new.subjects, new.tags);
        END;
        CREATE TRIGGER IF NOT EXISTS books_ad AFTER DELETE ON books BEGIN
            INSERT INTO books_fts(books_fts, rowid, title, authors, series, description, subjects, tags)
            VALUES ('delete', old.id, old.title, old.authors, old.series, old.description, old.subjects, old.tags);
        END;
        CREATE TRIGGER IF NOT EXISTS books_au AFTER UPDATE OF title, authors, series, description, subjects, tags ON books BEGIN
            INSERT INTO books_fts(books_fts, rowid, title, authors, series, description, subjects, tags)
            VALUES ('delete', old.id, old.title, old.authors, old.series, old.description, old.subjects, old.tags);
            INSERT INTO books_fts(rowid, title, authors, series, description, subjects, tags)
            VALUES (new.id, new.title, new.authors, new.series, new.description, new.subjects, new.tags);
        END;",
    )
}

pub fn kv_get<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
    with(|c| {
        c.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()
    })
    .ok()
    .flatten()
    .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn kv_set<T: Serialize>(key: &str, value: &T) -> Result<(), String> {
    let s = serde_json::to_string(value).map_err(|e| e.to_string())?;
    with(|c| {
        c.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, s],
        )
    })
    .map(|_| ())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BookRow {
    pub id: i64,
    pub path: String,
    pub folder: String,
    pub format: String,
    pub size: i64,
    pub hash: String,
    pub title: String,
    pub sort_title: String,
    pub authors: Vec<String>,
    pub series: String,
    pub series_index: Option<f64>,
    pub language: String,
    pub publisher: String,
    pub description: String,
    pub subjects: Vec<String>,
    pub isbn: String,
    pub identifiers: Vec<(String, String)>,
    pub published: String,
    pub cover: Option<String>,
    pub pages: Option<i64>,
    pub rtl: bool,
    pub fixed_layout: bool,
    pub source: String,
    pub source_url: String,
    pub series_id: Option<i64>,
    pub chapter_id: Option<i64>,
    pub rating: i64,
    pub favorite: bool,
    pub status: String,
    pub progress: f64,
    pub location: String,
    pub last_read_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub read_ms: i64,
    pub tags: Vec<String>,
    pub prefs: serde_json::Value,
    pub added_at: i64,
    pub mtime_ms: i64,
}

pub const BOOK_COLS: &str = "id, path, folder, format, size, hash, title, sort_title, authors, series, series_index, language, publisher, description, subjects, isbn, identifiers, published, cover, pages, rtl, fixed_layout, source, source_url, series_id, chapter_id, rating, favorite, status, progress, location, last_read_at, started_at, finished_at, read_ms, tags, prefs, added_at, mtime_ms";

fn json_col<T: serde::de::DeserializeOwned + Default>(r: &Row, i: usize) -> rusqlite::Result<T> {
    let s: String = r.get(i)?;
    Ok(serde_json::from_str(&s).unwrap_or_default())
}

pub fn row_to_book(r: &Row) -> rusqlite::Result<BookRow> {
    Ok(BookRow {
        id: r.get(0)?,
        path: r.get(1)?,
        folder: r.get(2)?,
        format: r.get(3)?,
        size: r.get(4)?,
        hash: r.get(5)?,
        title: r.get(6)?,
        sort_title: r.get(7)?,
        authors: json_col(r, 8)?,
        series: r.get(9)?,
        series_index: r.get(10)?,
        language: r.get(11)?,
        publisher: r.get(12)?,
        description: r.get(13)?,
        subjects: json_col(r, 14)?,
        isbn: r.get(15)?,
        identifiers: json_col(r, 16)?,
        published: r.get(17)?,
        cover: r.get(18)?,
        pages: r.get(19)?,
        rtl: r.get::<_, i64>(20)? != 0,
        fixed_layout: r.get::<_, i64>(21)? != 0,
        source: r.get(22)?,
        source_url: r.get(23)?,
        series_id: r.get(24)?,
        chapter_id: r.get(25)?,
        rating: r.get(26)?,
        favorite: r.get::<_, i64>(27)? != 0,
        status: r.get(28)?,
        progress: r.get(29)?,
        location: r.get(30)?,
        last_read_at: r.get(31)?,
        started_at: r.get(32)?,
        finished_at: r.get(33)?,
        read_ms: r.get(34)?,
        tags: json_col(r, 35)?,
        prefs: {
            let s: String = r.get(36)?;
            serde_json::from_str(&s).unwrap_or(serde_json::json!({}))
        },
        added_at: r.get(37)?,
        mtime_ms: r.get(38)?,
    })
}

pub fn all_books() -> Result<Vec<BookRow>, String> {
    with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {BOOK_COLS} FROM books ORDER BY sort_title, title"
        ))?;
        let rows = st.query_map([], row_to_book)?.collect();
        rows
    })
}

pub fn book(id: i64) -> Result<BookRow, String> {
    with(|c| {
        c.query_row(
            &format!("SELECT {BOOK_COLS} FROM books WHERE id = ?1"),
            [id],
            row_to_book,
        )
    })
}

pub fn books_by_ids(ids: &[i64]) -> Result<Vec<BookRow>, String> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(b) = book(*id) {
            out.push(b);
        }
    }
    Ok(out)
}

pub fn sort_key(title: &str) -> String {
    let t = title.trim().to_lowercase();
    for a in [
        "the ", "a ", "an ", "o ", "os ", "as ", "um ", "uma ", "el ", "la ", "los ", "las ",
        "le ", "les ", "l'", "der ", "die ", "das ", "il ", "lo ", "gli ",
    ] {
        if let Some(rest) = t.strip_prefix(a) {
            return rest.to_string();
        }
    }
    t
}

#[derive(Debug, Clone, Default)]
pub struct NewBook {
    pub path: String,
    pub folder: String,
    pub format: String,
    pub size: i64,
    pub mtime_ms: i64,
    pub hash: String,
    pub meta: omniget_core::core::read::epub::BookMeta,
    pub cover: Option<String>,
    pub source: String,
    pub source_url: String,
    pub series_id: Option<i64>,
    pub chapter_id: Option<i64>,
}

pub fn upsert_book(b: &NewBook) -> Result<i64, String> {
    let m = &b.meta;
    let authors = serde_json::to_string(&m.authors).unwrap_or_else(|_| "[]".into());
    let subjects = serde_json::to_string(&m.subjects).unwrap_or_else(|_| "[]".into());
    let ids = serde_json::to_string(&m.identifiers).unwrap_or_else(|_| "[]".into());
    let isbn = m.isbn().unwrap_or("").to_string();
    with(|c| {
        if !b.hash.is_empty() {
            let existing_by_path: Option<i64> = c
                .query_row("SELECT id FROM books WHERE path = ?1", [&b.path], |r| {
                    r.get(0)
                })
                .optional()?;
            if existing_by_path.is_none() {
                let existing_by_hash: Option<(i64, String)> = c
                    .query_row(
                        "SELECT id, path FROM books WHERE hash = ?1 LIMIT 1",
                        [&b.hash],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some((dup_id, dup_path)) = existing_by_hash {
                    if std::path::Path::new(&dup_path).exists() {
                        c.execute(
                            "UPDATE books SET
                                cover = COALESCE(?1, cover),
                                source = CASE WHEN source = '' OR source = 'local' THEN CASE WHEN ?2 = '' THEN source ELSE ?2 END ELSE source END,
                                source_url = CASE WHEN source_url = '' THEN ?3 ELSE source_url END
                             WHERE id = ?4",
                            params![b.cover, b.source, b.source_url, dup_id],
                        )?;
                        return Ok(dup_id);
                    }
                    c.execute(
                        "UPDATE books SET path = ?1, folder = ?2, mtime_ms = ?3 WHERE id = ?4",
                        params![b.path, b.folder, b.mtime_ms, dup_id],
                    )?;
                    return Ok(dup_id);
                }
            }
        }
        c.execute(
            "INSERT INTO books (path, folder, format, size, mtime_ms, hash, title, sort_title, authors,
                series, series_index, language, publisher, description, subjects, isbn, identifiers,
                published, cover, pages, rtl, fixed_layout, source, source_url, series_id, chapter_id, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)
             ON CONFLICT(path) DO UPDATE SET
                folder = excluded.folder, format = excluded.format, size = excluded.size,
                mtime_ms = excluded.mtime_ms, hash = excluded.hash, title = excluded.title,
                sort_title = excluded.sort_title, authors = excluded.authors, series = excluded.series,
                series_index = excluded.series_index, language = excluded.language,
                publisher = excluded.publisher, description = excluded.description,
                subjects = excluded.subjects, isbn = excluded.isbn, identifiers = excluded.identifiers,
                published = excluded.published, cover = COALESCE(excluded.cover, books.cover),
                pages = COALESCE(excluded.pages, books.pages), rtl = excluded.rtl,
                fixed_layout = excluded.fixed_layout,
                source = CASE WHEN excluded.source = '' THEN books.source ELSE excluded.source END,
                source_url = CASE WHEN excluded.source_url = '' THEN books.source_url ELSE excluded.source_url END,
                series_id = COALESCE(excluded.series_id, books.series_id),
                chapter_id = COALESCE(excluded.chapter_id, books.chapter_id)",
            params![
                b.path,
                b.folder,
                b.format,
                b.size,
                b.mtime_ms,
                b.hash,
                m.title,
                sort_key(&m.title),
                authors,
                m.series,
                m.series_index,
                m.language,
                m.publisher,
                m.description,
                subjects,
                isbn,
                ids,
                m.published,
                b.cover,
                m.page_count,
                m.rtl as i64,
                m.fixed_layout as i64,
                b.source,
                b.source_url,
                b.series_id,
                b.chapter_id,
                now_ms(),
            ],
        )?;
        c.query_row("SELECT id FROM books WHERE path = ?1", [&b.path], |r| {
            r.get(0)
        })
    })
}

pub fn stamps_under(root: &str) -> Result<std::collections::HashMap<String, (i64, i64)>, String> {
    let like = format!(
        "{}%",
        root.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    with(|c| {
        let mut st =
            c.prepare("SELECT path, mtime_ms, size FROM books WHERE path LIKE ?1 ESCAPE '\\'")?;
        let rows = st
            .query_map([like], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SeriesRow {
    pub id: i64,
    pub source: String,
    pub source_key: String,
    pub title: String,
    pub alt_titles: Vec<String>,
    pub authors: Vec<String>,
    pub description: String,
    pub genres: Vec<String>,
    pub status: String,
    pub lang: String,
    pub url: String,
    pub cover_url: String,
    pub cover: Option<String>,
    pub in_library: bool,
    pub chapter_langs: Vec<String>,
    pub reader: serde_json::Value,
    pub notes: String,
    pub tracking: serde_json::Value,
    pub auto_download: bool,
    pub added_at: i64,
    pub checked_at: Option<i64>,
    pub new_at: Option<i64>,
    pub categories: Vec<i64>,
    pub unread: i64,
    pub total: i64,
    pub downloaded: i64,
    pub last_read_at: Option<i64>,
}

pub const SERIES_COLS: &str = "s.id, s.source, s.source_key, s.title, s.alt_titles, s.authors, s.description, s.genres, s.status, s.lang, s.url, s.cover_url, s.cover, s.in_library, s.chapter_langs, s.reader, s.notes, s.tracking, s.auto_download, s.added_at, s.checked_at, s.new_at,
    (SELECT COUNT(*) FROM chapters c WHERE c.series_id = s.id AND c.read = 0),
    (SELECT COUNT(*) FROM chapters c WHERE c.series_id = s.id),
    (SELECT COUNT(*) FROM chapters c WHERE c.series_id = s.id AND c.book_id IS NOT NULL),
    (SELECT MAX(read_at) FROM chapters c WHERE c.series_id = s.id),
    (SELECT group_concat(category_id) FROM series_categories sc WHERE sc.series_id = s.id)";

fn json_val(r: &Row, i: usize) -> rusqlite::Result<serde_json::Value> {
    let s: String = r.get(i)?;
    Ok(serde_json::from_str(&s).unwrap_or(serde_json::json!({})))
}

pub fn row_to_series(r: &Row) -> rusqlite::Result<SeriesRow> {
    let cats: Option<String> = r.get(26)?;
    Ok(SeriesRow {
        id: r.get(0)?,
        source: r.get(1)?,
        source_key: r.get(2)?,
        title: r.get(3)?,
        alt_titles: json_col(r, 4)?,
        authors: json_col(r, 5)?,
        description: r.get(6)?,
        genres: json_col(r, 7)?,
        status: r.get(8)?,
        lang: r.get(9)?,
        url: r.get(10)?,
        cover_url: r.get(11)?,
        cover: r.get(12)?,
        in_library: r.get::<_, i64>(13)? != 0,
        chapter_langs: json_col(r, 14)?,
        reader: json_val(r, 15)?,
        notes: r.get(16)?,
        tracking: json_val(r, 17)?,
        auto_download: r.get::<_, i64>(18)? != 0,
        added_at: r.get(19)?,
        checked_at: r.get(20)?,
        new_at: r.get(21)?,
        unread: r.get(22)?,
        total: r.get(23)?,
        downloaded: r.get(24)?,
        last_read_at: r.get(25)?,
        categories: cats
            .unwrap_or_default()
            .split(',')
            .filter_map(|x| x.parse().ok())
            .collect(),
    })
}

pub fn series(id: i64) -> Result<SeriesRow, String> {
    with(|c| {
        c.query_row(
            &format!("SELECT {SERIES_COLS} FROM series s WHERE s.id = ?1"),
            [id],
            row_to_series,
        )
    })
}

pub fn series_list(only_library: bool) -> Result<Vec<SeriesRow>, String> {
    with(|c| {
        let q = format!(
            "SELECT {SERIES_COLS} FROM series s {} ORDER BY s.title COLLATE NOCASE",
            if only_library {
                "WHERE s.in_library = 1"
            } else {
                ""
            }
        );
        let mut st = c.prepare(&q)?;
        let rows = st.query_map([], row_to_series)?.collect();
        rows
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChapterRow {
    pub id: i64,
    pub series_id: i64,
    pub key: String,
    pub title: String,
    pub number: Option<f64>,
    pub volume: Option<f64>,
    pub lang: String,
    pub scanlator: String,
    pub published: Option<i64>,
    pub pages: Option<i64>,
    pub external_url: Option<String>,
    pub position: i64,
    pub read: bool,
    pub bookmarked: bool,
    pub last_page: i64,
    pub read_at: Option<i64>,
    pub fetched_at: i64,
    pub book_id: Option<i64>,
}

pub const CHAPTER_COLS: &str = "id, series_id, key, title, number, volume, lang, scanlator, published, pages, external_url, position, read, bookmarked, last_page, read_at, fetched_at, book_id";

pub fn row_to_chapter(r: &Row) -> rusqlite::Result<ChapterRow> {
    Ok(ChapterRow {
        id: r.get(0)?,
        series_id: r.get(1)?,
        key: r.get(2)?,
        title: r.get(3)?,
        number: r.get(4)?,
        volume: r.get(5)?,
        lang: r.get(6)?,
        scanlator: r.get(7)?,
        published: r.get(8)?,
        pages: r.get(9)?,
        external_url: r.get(10)?,
        position: r.get(11)?,
        read: r.get::<_, i64>(12)? != 0,
        bookmarked: r.get::<_, i64>(13)? != 0,
        last_page: r.get(14)?,
        read_at: r.get(15)?,
        fetched_at: r.get(16)?,
        book_id: r.get(17)?,
    })
}

pub fn chapters_of(series_id: i64) -> Result<Vec<ChapterRow>, String> {
    with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {CHAPTER_COLS} FROM chapters WHERE series_id = ?1 ORDER BY position"
        ))?;
        let rows = st.query_map([series_id], row_to_chapter)?.collect();
        rows
    })
}

pub fn chapter(id: i64) -> Result<ChapterRow, String> {
    with(|c| {
        c.query_row(
            &format!("SELECT {CHAPTER_COLS} FROM chapters WHERE id = ?1"),
            [id],
            row_to_chapter,
        )
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Annotation {
    #[serde(default)]
    pub id: i64,
    pub book_id: i64,
    pub kind: String,
    pub locator: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub chapter: String,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

pub fn row_to_annotation(r: &Row) -> rusqlite::Result<Annotation> {
    Ok(Annotation {
        id: r.get(0)?,
        book_id: r.get(1)?,
        kind: r.get(2)?,
        locator: r.get(3)?,
        text: r.get(4)?,
        note: r.get(5)?,
        color: r.get(6)?,
        style: r.get(7)?,
        chapter: r.get(8)?,
        progress: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

pub const ANNOTATION_COLS: &str =
    "id, book_id, kind, locator, text, note, color, style, chapter, progress, created_at, updated_at";
