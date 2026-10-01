//! Keeping reading state in step elsewhere: KOReader sync (progress by
//! document hash, the protocol KOReader and many readers speak), a JSON
//! backup of everything that isn't a file, and importing a Mihon /
//! Tachiyomi backup.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use omniget_core::core::read::{kosync, tachibk};

use super::{blocking, db};

// ---------- KOReader sync ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KosyncConfig {
    pub enabled: bool,
    pub server: String,
    pub username: String,
    pub password: String,
    pub device: String,
}

fn kosync_config() -> KosyncConfig {
    let mut c: KosyncConfig = db::kv_get("kosync").unwrap_or(KosyncConfig {
        server: "https://sync.koreader.rocks".into(),
        device: "OmniGet".into(),
        ..Default::default()
    });
    c.password = super::secret("kosync");
    c
}

fn client() -> Option<kosync::Client> {
    let c = kosync_config();
    (c.enabled && !c.username.is_empty() && !c.password.is_empty())
        .then(|| kosync::Client::new(&c.server, &c.username, &c.password, false))
}

/// Saves the config; with `register` also creates the account. Always
/// checks the login.
#[tauri::command]
pub async fn read_kosync_config(
    set: Option<KosyncConfig>,
    register: Option<bool>,
) -> Result<KosyncConfig, String> {
    if let Some(mut c) = set {
        if c.password != "••••••" {
            super::set_secret("kosync", &c.password)?;
        }
        c.password = String::new();
        blocking(move || db::kv_set("kosync", &c)).await?;
        let cfg = kosync_config();
        if cfg.enabled {
            let cl = kosync::Client::new(&cfg.server, &cfg.username, &cfg.password, false);
            if register.unwrap_or(false) {
                cl.register().await?;
            }
            cl.auth().await?;
        }
    }
    let mut c = kosync_config();
    if !c.password.is_empty() {
        c.password = "••••••".into();
    }
    Ok(c)
}

#[tauri::command]
pub async fn read_kosync_push(id: i64) -> Result<(), String> {
    let Some(cl) = client() else { return Ok(()) };
    let b = blocking(move || db::book(id)).await?;
    if b.hash.is_empty() {
        return Ok(());
    }
    let device = kosync_config().device;
    cl.push(&kosync::Progress {
        document: b.hash,
        progress: b.location,
        percentage: b.progress,
        device: if device.is_empty() {
            "OmniGet".into()
        } else {
            device
        },
        device_id: device_id(),
        timestamp: None,
    })
    .await
}

/// What the server has for this book, when it is ahead of us.
#[tauri::command]
pub async fn read_kosync_pull(id: i64) -> Result<Option<kosync::Progress>, String> {
    let Some(cl) = client() else { return Ok(None) };
    let b = blocking(move || db::book(id)).await?;
    if b.hash.is_empty() {
        return Ok(None);
    }
    let remote = cl.pull(&b.hash).await?;
    Ok(remote.filter(|r| r.device_id != device_id() && (r.percentage - b.progress).abs() > 0.005))
}

fn device_id() -> String {
    if let Some(id) = db::kv_get::<String>("device_id") {
        return id;
    }
    let id = uuid::Uuid::new_v4().simple().to_string().to_uppercase();
    let _ = db::kv_set("device_id", &id);
    id
}

// ---------- JSON backup ----------

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Backup {
    pub version: u32,
    pub created_at: i64,
    pub books: Vec<BookState>,
    pub annotations: Vec<AnnotationState>,
    pub shelves: Vec<ShelfState>,
    pub series: Vec<SeriesState>,
    pub categories: Vec<String>,
    pub catalogs: Vec<(String, String, String)>,
    pub kv: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BookState {
    pub path: String,
    pub hash: String,
    pub title: String,
    pub status: String,
    pub progress: f64,
    pub location: String,
    pub rating: i64,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub prefs: serde_json::Value,
    pub last_read_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub read_ms: i64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AnnotationState {
    pub book_hash: String,
    pub book_path: String,
    pub a: db::Annotation,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ShelfState {
    pub name: String,
    pub icon: String,
    pub smart: Option<String>,
    pub books: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SeriesState {
    pub source: String,
    pub key: String,
    pub title: String,
    pub in_library: bool,
    pub reader: serde_json::Value,
    pub notes: String,
    pub auto_download: bool,
    pub chapter_langs: Vec<String>,
    pub categories: Vec<String>,
    /// Chapter keys read, and last page of the one in progress.
    pub read: Vec<String>,
    pub bookmarked: Vec<String>,
}

const KV_KEYS: &[&str] = &[
    "notes_dir",
    "update_interval_h",
    "delete_after_read",
    "download_ahead",
    "kosync",
    "mail",
    "share",
];

#[tauri::command]
pub async fn read_backup_export(path: String) -> Result<usize, String> {
    blocking(move || {
        let books = db::all_books()?;
        let by_id: HashMap<i64, &db::BookRow> = books.iter().map(|b| (b.id, b)).collect();
        let mut out = Backup {
            version: 1,
            created_at: db::now_ms(),
            ..Default::default()
        };
        for b in &books {
            out.books.push(BookState {
                path: b.path.clone(),
                hash: b.hash.clone(),
                title: b.title.clone(),
                status: b.status.clone(),
                progress: b.progress,
                location: b.location.clone(),
                rating: b.rating,
                favorite: b.favorite,
                tags: b.tags.clone(),
                prefs: b.prefs.clone(),
                last_read_at: b.last_read_at,
                finished_at: b.finished_at,
                read_ms: b.read_ms,
            });
        }
        let anns: Vec<db::Annotation> = db::with(|c| {
            let mut st = c.prepare(&format!("SELECT {} FROM annotations", db::ANNOTATION_COLS))?;
            let rows = st.query_map([], db::row_to_annotation)?.collect();
            rows
        })?;
        for a in anns {
            if let Some(b) = by_id.get(&a.book_id) {
                out.annotations.push(AnnotationState {
                    book_hash: b.hash.clone(),
                    book_path: b.path.clone(),
                    a,
                });
            }
        }
        let shelves: Vec<(i64, String, String, Option<String>)> = db::with(|c| {
            let mut st = c.prepare("SELECT id, name, icon, smart FROM shelves")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect();
            rows
        })?;
        for (id, name, icon, smart) in shelves {
            let ids: Vec<i64> = if smart.is_some() {
                vec![]
            } else {
                super::library::shelf_book_ids(id)?
            };
            out.shelves.push(ShelfState {
                name,
                icon,
                smart,
                books: ids
                    .iter()
                    .filter_map(|i| by_id.get(i))
                    .map(|b| {
                        if b.hash.is_empty() {
                            b.path.clone()
                        } else {
                            b.hash.clone()
                        }
                    })
                    .collect(),
            });
        }
        let cats: Vec<(i64, String)> = db::with(|c| {
            let mut st = c.prepare("SELECT id, name FROM categories ORDER BY position")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
            rows
        })?;
        out.categories = cats.iter().map(|(_, n)| n.clone()).collect();
        for s in db::series_list(false)? {
            if !s.in_library && s.checked_at.is_none() {
                continue;
            }
            let chs = db::chapters_of(s.id)?;
            out.series.push(SeriesState {
                source: s.source.clone(),
                key: s.source_key.clone(),
                title: s.title.clone(),
                in_library: s.in_library,
                reader: s.reader.clone(),
                notes: s.notes.clone(),
                auto_download: s.auto_download,
                chapter_langs: s.chapter_langs.clone(),
                categories: s
                    .categories
                    .iter()
                    .filter_map(|id| cats.iter().find(|(c, _)| c == id).map(|(_, n)| n.clone()))
                    .collect(),
                read: chs
                    .iter()
                    .filter(|c| c.read)
                    .map(|c| c.key.clone())
                    .collect(),
                bookmarked: chs
                    .iter()
                    .filter(|c| c.bookmarked)
                    .map(|c| c.key.clone())
                    .collect(),
            });
        }
        out.catalogs = super::sources::catalogs()?
            .into_iter()
            .filter(|c| c.kind != "declarative")
            .map(|c| (c.kind, c.title, c.url))
            .collect();
        for k in KV_KEYS {
            if let Some(v) = db::kv_get::<serde_json::Value>(k) {
                out.kv.insert(k.to_string(), v);
            }
        }
        let n = out.books.len() + out.series.len();
        let json = serde_json::to_vec_pretty(&out).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(n)
    })
    .await
}

/// Merges a backup: books are matched by hash, then path; series by source
/// and key. Existing progress wins when it is further along.
#[tauri::command]
pub async fn read_backup_import(app: AppHandle, path: String) -> Result<String, String> {
    let msg = blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let b: Backup = serde_json::from_slice(&bytes).map_err(|e| format!("not an OmniGet read backup: {e}"))?;
        let books = db::all_books()?;
        let find = |hash: &str, path: &str| -> Option<i64> {
            books
                .iter()
                .find(|x| (!hash.is_empty() && x.hash == hash) || x.path == path)
                .map(|x| x.id)
        };
        let mut matched = 0;
        db::with(|c| {
            for s in &b.books {
                let Some(id) = find(&s.hash, &s.path) else { continue };
                matched += 1;
                c.execute(
                    "UPDATE books SET
                        status = CASE WHEN ?1 > progress THEN ?2 ELSE status END,
                        location = CASE WHEN ?1 > progress THEN ?3 ELSE location END,
                        progress = MAX(progress, ?1),
                        rating = MAX(rating, ?4), favorite = MAX(favorite, ?5),
                        tags = CASE WHEN tags = '[]' THEN ?6 ELSE tags END,
                        prefs = CASE WHEN prefs = '{}' THEN ?7 ELSE prefs END,
                        last_read_at = MAX(COALESCE(last_read_at, 0), COALESCE(?8, 0)),
                        finished_at = COALESCE(finished_at, ?9),
                        read_ms = MAX(read_ms, ?10)
                     WHERE id = ?11",
                    params![
                        s.progress,
                        s.status,
                        s.location,
                        s.rating,
                        s.favorite as i64,
                        serde_json::to_string(&s.tags).unwrap_or_default(),
                        s.prefs.to_string(),
                        s.last_read_at,
                        s.finished_at,
                        s.read_ms,
                        id
                    ],
                )?;
            }
            for a in &b.annotations {
                let Some(id) = find(&a.book_hash, &a.book_path) else { continue };
                let exists: i64 = c.query_row(
                    "SELECT COUNT(*) FROM annotations WHERE book_id = ?1 AND locator = ?2 AND kind = ?3",
                    params![id, a.a.locator, a.a.kind],
                    |r| r.get(0),
                )?;
                if exists == 0 {
                    c.execute(
                        "INSERT INTO annotations (book_id, kind, locator, text, note, color, style, chapter, progress, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![id, a.a.kind, a.a.locator, a.a.text, a.a.note, a.a.color, a.a.style, a.a.chapter, a.a.progress, a.a.created_at, a.a.updated_at],
                    )?;
                }
            }
            for (i, name) in b.categories.iter().enumerate() {
                c.execute(
                    "INSERT INTO categories (name, position) SELECT ?1, ?2 WHERE NOT EXISTS (SELECT 1 FROM categories WHERE name = ?1)",
                    params![name, i as i64],
                )?;
            }
            for sh in &b.shelves {
                c.execute(
                    "INSERT INTO shelves (name, icon, smart, position, created_at)
                     SELECT ?1, ?2, ?3, 0, ?4 WHERE NOT EXISTS (SELECT 1 FROM shelves WHERE name = ?1)",
                    params![sh.name, sh.icon, sh.smart, db::now_ms()],
                )?;
                let sid: i64 = c.query_row("SELECT id FROM shelves WHERE name = ?1", [&sh.name], |r| r.get(0))?;
                for key in &sh.books {
                    if let Some(bid) = find(key, key) {
                        c.execute("INSERT OR IGNORE INTO shelf_items (shelf_id, book_id) VALUES (?1, ?2)", params![sid, bid])?;
                    }
                }
            }
            for s in &b.series {
                c.execute(
                    "INSERT INTO series (source, source_key, title, in_library, reader, notes, auto_download, chapter_langs, added_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(source, source_key) DO UPDATE SET in_library = MAX(series.in_library, excluded.in_library)",
                    params![
                        s.source, s.key, s.title, s.in_library as i64, s.reader.to_string(), s.notes,
                        s.auto_download as i64, serde_json::to_string(&s.chapter_langs).unwrap_or_default(), db::now_ms()
                    ],
                )?;
                let sid: i64 = c.query_row(
                    "SELECT id FROM series WHERE source = ?1 AND source_key = ?2",
                    params![s.source, s.key],
                    |r| r.get(0),
                )?;
                for cat in &s.categories {
                    c.execute(
                        "INSERT OR IGNORE INTO series_categories (series_id, category_id) SELECT ?1, id FROM categories WHERE name = ?2",
                        params![sid, cat],
                    )?;
                }
                // Read marks are applied after the next sync creates the rows.
                c.execute(
                    "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![format!("pending_read:{sid}"), serde_json::json!({ "read": s.read, "bookmarked": s.bookmarked }).to_string()],
                )?;
                for key in &s.read {
                    c.execute("UPDATE chapters SET read = 1 WHERE series_id = ?1 AND key = ?2", params![sid, key])?;
                }
            }
            for (k, v) in &b.kv {
                if KV_KEYS.contains(&k.as_str()) {
                    c.execute(
                        "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
                        params![k, v.to_string()],
                    )?;
                }
            }
            Ok(())
        })?;
        Ok(format!("{matched} books, {} series", b.series.len()))
    })
    .await?;
    let _ = app.emit("read-library-changed", ());
    let _ = app.emit("read-series-changed", ());
    Ok(msg)
}

// ---------- Mihon / Tachiyomi ----------

#[derive(Serialize)]
pub struct MihonImport {
    pub imported: usize,
    /// Titles from sources OmniGet can't open (install the matching
    /// declarative source or use the Suwayomi bridge).
    pub unmatched: Vec<(String, String)>,
}

/// Imports a `.tachibk`: MangaDex entries become followed series with
/// their read marks; everything else is listed so the user can bring it
/// through a source of their own.
#[tauri::command]
pub async fn read_import_mihon(app: AppHandle, path: String) -> Result<MihonImport, String> {
    let list = blocking(move || {
        let data = std::fs::read(&path).map_err(|e| e.to_string())?;
        tachibk::parse(&data)
    })
    .await?;
    let mut imported = 0;
    let mut unmatched = Vec::new();
    for m in list.into_iter().filter(|m| m.favorite) {
        let Some(id) = m.mangadex_id() else {
            unmatched.push((m.title.clone(), m.source_name.clone()));
            continue;
        };
        let full = match super::manga::read_series_open("mangadex".into(), id.clone(), None).await {
            Ok(f) => f,
            Err(e) => {
                unmatched.push((m.title.clone(), e));
                continue;
            }
        };
        let sid = full.series.id;
        let read_numbers: Vec<f32> = m
            .chapters
            .iter()
            .filter(|c| c.read)
            .filter_map(|c| c.number)
            .collect();
        let read_keys: Vec<String> = m
            .chapters
            .iter()
            .filter(|c| c.read)
            .filter_map(|c| {
                c.url
                    .strip_prefix("/chapter/")
                    .map(|k| k.trim_end_matches('/').to_string())
            })
            .collect();
        let cats = m.categories.clone();
        let notes = m.notes.clone();
        blocking(move || {
            db::with(|c| {
                c.execute("UPDATE series SET in_library = 1, notes = CASE WHEN notes = '' THEN ?1 ELSE notes END WHERE id = ?2", params![notes, sid])?;
                for k in &read_keys {
                    c.execute("UPDATE chapters SET read = 1 WHERE series_id = ?1 AND key = ?2", params![sid, k])?;
                }
                for n in &read_numbers {
                    c.execute(
                        "UPDATE chapters SET read = 1 WHERE series_id = ?1 AND ABS(number - ?2) < 0.001",
                        params![sid, *n as f64],
                    )?;
                }
                for cat in &cats {
                    c.execute(
                        "INSERT INTO categories (name, position) SELECT ?1, 0 WHERE NOT EXISTS (SELECT 1 FROM categories WHERE name = ?1)",
                        [cat],
                    )?;
                    c.execute(
                        "INSERT OR IGNORE INTO series_categories (series_id, category_id) SELECT ?1, id FROM categories WHERE name = ?2",
                        params![sid, cat],
                    )?;
                }
                Ok(())
            })
        })
        .await?;
        imported += 1;
        // MangaDex rate limit.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    let _ = app.emit("read-series-changed", ());
    Ok(MihonImport {
        imported,
        unmatched,
    })
}
