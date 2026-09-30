//! Followed series: chapter lists synced from their source, read marks,
//! the updates feed, categories, migration between sources and chapter
//! downloads (saved as CBZ with ComicInfo.xml and added to the library).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use futures::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use omniget_core::core::read::comic::{self, ComicInfo};
use omniget_core::core::read::sources::{Hit, RemoteChapter};

use super::db::{self, ChapterRow, SeriesRow};
use super::{blocking, scan, sources};

/// Sources whose terms allow streaming only.
pub fn stream_only(source: &str) -> bool {
    source == "mangaplus"
}

fn changed(app: &AppHandle) {
    let _ = app.emit("read-series-changed", ());
}

fn store_series(hit: &Hit, source: &str, key: &str, in_library: bool) -> Result<i64, String> {
    let js = |v: &Vec<String>| serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
    let status = hit.extra["status"].as_str().unwrap_or("").to_string();
    db::with(|c| {
        c.execute(
            "INSERT INTO series (source, source_key, title, authors, description, genres, status, lang, url, cover_url, in_library, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(source, source_key) DO UPDATE SET
                title = CASE WHEN excluded.title = '' THEN series.title ELSE excluded.title END,
                authors = excluded.authors, description = excluded.description, genres = excluded.genres,
                status = excluded.status, url = excluded.url,
                cover_url = CASE WHEN excluded.cover_url = '' THEN series.cover_url ELSE excluded.cover_url END,
                in_library = MAX(series.in_library, excluded.in_library)",
            params![
                source,
                key,
                hit.title,
                js(&hit.authors),
                hit.summary,
                js(&hit.subjects),
                status,
                hit.language,
                hit.url,
                hit.cover.clone().unwrap_or_default(),
                in_library as i64,
                db::now_ms(),
            ],
        )?;
        c.query_row(
            "SELECT id FROM series WHERE source = ?1 AND source_key = ?2",
            params![source, key],
            |r| r.get(0),
        )
    })
}

/// Sources list chapters newest first, or sort volumes as text (MangaDex puts
/// volume 15 before volume 2), so order by chapter number. An unnumbered
/// chapter (an extra, a oneshot) stays after the numbered one it followed.
fn reading_order(list: &[RemoteChapter]) -> Vec<&RemoteChapter> {
    let mut last = f64::NEG_INFINITY;
    let mut keyed: Vec<(f64, &RemoteChapter)> = list
        .iter()
        .map(|ch| {
            if let Some(n) = ch.number {
                last = n;
            }
            (ch.number.unwrap_or(last), ch)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    keyed.into_iter().map(|(_, ch)| ch).collect()
}

/// Merges a fresh chapter list; returns the ids of chapters seen for the
/// first time. Read marks and downloads are kept.
fn store_chapters(series_id: i64, list: &[RemoteChapter]) -> Result<Vec<i64>, String> {
    let now = db::now_ms();
    let list = reading_order(list);
    db::with_mut(|c| {
        let tx = c.transaction()?;
        let known: HashSet<String> = {
            let mut st = tx.prepare("SELECT key FROM chapters WHERE series_id = ?1")?;
            let rows = st.query_map([series_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut new_ids = Vec::new();
        for (i, ch) in list.iter().enumerate() {
            let is_new = !known.contains(&ch.key);
            tx.execute(
                "INSERT INTO chapters (series_id, key, title, number, volume, lang, scanlator, published, pages, external_url, position, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(series_id, key) DO UPDATE SET
                    title = excluded.title, number = excluded.number, volume = excluded.volume,
                    lang = excluded.lang, scanlator = excluded.scanlator, published = excluded.published,
                    pages = COALESCE(excluded.pages, chapters.pages), external_url = excluded.external_url,
                    position = excluded.position",
                params![
                    series_id,
                    ch.key,
                    ch.title,
                    ch.number,
                    ch.volume,
                    ch.language,
                    ch.scanlator,
                    ch.published,
                    ch.pages.map(|p| p as i64),
                    ch.external_url,
                    i as i64,
                    now,
                ],
            )?;
            if is_new {
                new_ids.push(tx.last_insert_rowid());
            }
        }
        if !new_ids.is_empty() && !known.is_empty() {
            tx.execute(
                "UPDATE series SET new_at = ?1 WHERE id = ?2",
                params![now, series_id],
            )?;
        }
        tx.execute(
            "UPDATE series SET checked_at = ?1 WHERE id = ?2",
            params![now, series_id],
        )?;
        tx.commit()?;
        // A first sync isn't "new chapters".
        Ok(if known.is_empty() { vec![] } else { new_ids })
    })
}

/// Read marks restored from a backup before the chapter rows existed.
fn apply_pending(series_id: i64) -> Result<(), String> {
    let key = format!("pending_read:{series_id}");
    let Some(p) = db::kv_get::<serde_json::Value>(&key) else {
        return Ok(());
    };
    db::with(|c| {
        for k in p["read"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str())
        {
            c.execute(
                "UPDATE chapters SET read = 1 WHERE series_id = ?1 AND key = ?2",
                params![series_id, k],
            )?;
        }
        for k in p["bookmarked"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str())
        {
            c.execute(
                "UPDATE chapters SET bookmarked = 1 WHERE series_id = ?1 AND key = ?2",
                params![series_id, k],
            )?;
        }
        c.execute("DELETE FROM kv WHERE key = ?1", [&key])?;
        Ok(())
    })
}

#[derive(Serialize)]
pub struct SeriesFull {
    pub series: SeriesRow,
    pub chapters: Vec<ChapterRow>,
}

async fn sync(series_id: i64) -> Result<Vec<i64>, String> {
    let s = blocking(move || db::series(series_id)).await?;
    let d = sources::details(
        &s.source,
        &s.source_key,
        if s.lang.is_empty() { "en" } else { &s.lang },
        &s.chapter_langs,
    )
    .await?;
    let key = s.source_key.clone();
    let source = s.source.clone();
    blocking(move || {
        store_series(&d.hit, &source, &key, false)?;
        let new = store_chapters(series_id, &d.chapters)?;
        apply_pending(series_id)?;
        Ok(new)
    })
    .await
}

/// Opens a series from a source (without following it) so its page can
/// show chapters and read marks.
#[tauri::command]
pub async fn read_series_open(
    source: String,
    key: String,
    langs: Option<Vec<String>>,
) -> Result<SeriesFull, String> {
    let langs = langs.unwrap_or_default();
    let d = sources::details(
        &source,
        &key,
        langs.first().map(|s| s.as_str()).unwrap_or("en"),
        &langs,
    )
    .await?;
    blocking(move || {
        let id = store_series(&d.hit, &source, &key, false)?;
        if !langs.is_empty() {
            db::with(|c| {
                c.execute(
                    "UPDATE series SET chapter_langs = ?1 WHERE id = ?2",
                    params![serde_json::to_string(&langs).unwrap_or_default(), id],
                )
            })?;
        }
        store_chapters(id, &d.chapters)?;
        apply_pending(id)?;
        Ok(SeriesFull {
            series: db::series(id)?,
            chapters: db::chapters_of(id)?,
        })
    })
    .await
}

#[tauri::command]
pub async fn read_series_follow(app: AppHandle, id: i64, follow: bool) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE series SET in_library = ?1 WHERE id = ?2",
                params![follow as i64, id],
            )
        })
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_series_list(only_library: Option<bool>) -> Result<Vec<SeriesRow>, String> {
    blocking(move || db::series_list(only_library.unwrap_or(true))).await
}

#[tauri::command]
pub async fn read_series_get(id: i64) -> Result<SeriesFull, String> {
    blocking(move || {
        Ok(SeriesFull {
            series: db::series(id)?,
            chapters: db::chapters_of(id)?,
        })
    })
    .await
}

#[tauri::command]
pub async fn read_series_refresh(app: AppHandle, id: i64) -> Result<usize, String> {
    let new = sync(id).await?;
    changed(&app);
    Ok(new.len())
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SeriesPatch {
    pub title: Option<String>,
    pub reader: Option<serde_json::Value>,
    pub notes: Option<String>,
    pub auto_download: Option<bool>,
    pub chapter_langs: Option<Vec<String>>,
    pub categories: Option<Vec<i64>>,
    pub tracking: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn read_series_update(
    app: AppHandle,
    id: i64,
    patch: SeriesPatch,
) -> Result<SeriesRow, String> {
    let row = blocking(move || {
        db::with(|c| {
            if let Some(v) = &patch.title {
                c.execute("UPDATE series SET title = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = &patch.reader {
                c.execute("UPDATE series SET reader = ?1 WHERE id = ?2", params![v.to_string(), id])?;
            }
            if let Some(v) = &patch.tracking {
                c.execute("UPDATE series SET tracking = ?1 WHERE id = ?2", params![v.to_string(), id])?;
            }
            if let Some(v) = &patch.notes {
                c.execute("UPDATE series SET notes = ?1 WHERE id = ?2", params![v, id])?;
            }
            if let Some(v) = patch.auto_download {
                c.execute("UPDATE series SET auto_download = ?1 WHERE id = ?2", params![v as i64, id])?;
            }
            if let Some(v) = &patch.chapter_langs {
                c.execute(
                    "UPDATE series SET chapter_langs = ?1 WHERE id = ?2",
                    params![serde_json::to_string(v).unwrap_or_default(), id],
                )?;
            }
            if let Some(cats) = &patch.categories {
                c.execute("DELETE FROM series_categories WHERE series_id = ?1", [id])?;
                for cat in cats {
                    c.execute(
                        "INSERT OR IGNORE INTO series_categories (series_id, category_id) VALUES (?1, ?2)",
                        params![id, cat],
                    )?;
                }
            }
            Ok(())
        })?;
        db::series(id)
    })
    .await?;
    changed(&app);
    Ok(row)
}

#[tauri::command]
pub async fn read_series_remove(
    app: AppHandle,
    id: i64,
    delete_downloads: bool,
) -> Result<(), String> {
    blocking(move || {
        if delete_downloads {
            let paths: Vec<String> = db::with(|c| {
                let mut st = c.prepare(
                    "SELECT b.path FROM books b JOIN chapters ch ON ch.book_id = b.id WHERE ch.series_id = ?1",
                )?;
                let rows = st.query_map([id], |r| r.get(0))?.collect();
                rows
            })?;
            for p in paths {
                let _ = std::fs::remove_file(&p);
                let _ = db::with(|c| c.execute("DELETE FROM books WHERE path = ?1", [&p]));
            }
        }
        db::with(|c| c.execute("DELETE FROM series WHERE id = ?1", [id])).map(|_| ())
    })
    .await?;
    changed(&app);
    Ok(())
}

// ---------- chapters ----------

#[tauri::command]
pub async fn read_chapters_mark(app: AppHandle, ids: Vec<i64>, read: bool) -> Result<(), String> {
    let ids_for_push = ids.first().copied();
    blocking(move || {
        let now = db::now_ms();
        db::with(|c| {
            for id in &ids {
                c.execute(
                    "UPDATE chapters SET read = ?1, read_at = CASE WHEN ?1 = 1 THEN COALESCE(read_at, ?2) ELSE NULL END,
                        last_page = CASE WHEN ?1 = 1 THEN last_page ELSE 0 END WHERE id = ?3",
                    params![read as i64, now, id],
                )?;
            }
            Ok(())
        })?;
        for id in &ids {
            let _ = after_read(*id);
        }
        Ok(())
    })
    .await?;
    if read {
        if let Some(first) = ids_for_push {
            if let Ok(ch) = db::chapter(first) {
                tauri::async_runtime::spawn(push_tracking(ch.series_id));
            }
        }
    }
    changed(&app);
    Ok(())
}

/// Marks every chapter before this one (by position) as read.
#[tauri::command]
pub async fn read_chapters_mark_previous(app: AppHandle, id: i64) -> Result<(), String> {
    blocking(move || {
        let ch = db::chapter(id)?;
        db::with(|c| {
            c.execute(
                "UPDATE chapters SET read = 1, read_at = COALESCE(read_at, ?1) WHERE series_id = ?2 AND position < ?3",
                params![db::now_ms(), ch.series_id, ch.position],
            )
        })
        .map(|_| ())
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_chapter_bookmark(app: AppHandle, id: i64, on: bool) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE chapters SET bookmarked = ?1 WHERE id = ?2",
                params![on as i64, id],
            )
        })
        .map(|_| ())
    })
    .await?;
    changed(&app);
    Ok(())
}

/// Page position inside a chapter; reaching the last page marks it read.
#[tauri::command]
pub async fn read_chapter_progress(
    app: AppHandle,
    id: i64,
    page: i64,
    pages: i64,
) -> Result<bool, String> {
    let finished = blocking(move || {
        let done = pages > 0 && page >= pages - 1;
        let now = db::now_ms();
        db::with(|c| {
            c.execute(
                "UPDATE chapters SET last_page = ?1, pages = COALESCE(pages, ?2),
                    read = CASE WHEN ?3 THEN 1 ELSE read END,
                    read_at = CASE WHEN ?3 THEN ?4 ELSE COALESCE(read_at, NULL) END
                 WHERE id = ?5",
                params![page, pages, done, now, id],
            )
        })?;
        if done {
            let _ = after_read(id);
        }
        Ok(done)
    })
    .await?;
    if finished {
        if let Ok(ch) = db::chapter(id) {
            tauri::async_runtime::spawn(push_tracking(ch.series_id));
        }
        changed(&app);
    }
    Ok(finished)
}

/// "Delete after reading": when on, drops the downloaded file of a chapter
/// `keep` chapters behind the one just finished (bookmarked ones stay).
fn after_read(chapter_id: i64) -> Result<(), String> {
    let keep: Option<i64> = db::kv_get("delete_after_read");
    let Some(keep) = keep.filter(|k| *k >= 0) else {
        return Ok(());
    };
    let ch = db::chapter(chapter_id)?;
    let target: Option<(i64, Option<i64>)> = db::with(|c| {
        c.query_row(
            "SELECT id, book_id FROM chapters WHERE series_id = ?1 AND position = ?2 AND read = 1 AND bookmarked = 0",
            params![ch.series_id, ch.position - keep],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    })?;
    if let Some((cid, Some(book))) = target {
        if let Ok(b) = db::book(book) {
            let _ = std::fs::remove_file(&b.path);
            db::with(|c| c.execute("DELETE FROM books WHERE id = ?1", [book]))?;
        }
        db::with(|c| c.execute("UPDATE chapters SET book_id = NULL WHERE id = ?1", [cid]))?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ChapterOpen {
    /// Downloaded: read the local book.
    pub book_id: Option<i64>,
    /// Online: proxied page URLs.
    pub pages: Vec<String>,
    pub external_url: Option<String>,
    pub start_page: i64,
}

#[tauri::command]
pub async fn read_chapter_open(
    app: AppHandle,
    id: i64,
    data_saver: Option<bool>,
) -> Result<ChapterOpen, String> {
    let (ch, s) = blocking(move || {
        let ch = db::chapter(id)?;
        let s = db::series(ch.series_id)?;
        Ok((ch, s))
    })
    .await?;
    let start_page = if ch.read { 0 } else { ch.last_page };
    if let Some(book) = ch
        .book_id
        .filter(|b| db::book(*b).is_ok_and(|x| std::path::Path::new(&x.path).exists()))
    {
        return Ok(ChapterOpen {
            book_id: Some(book),
            pages: vec![],
            external_url: None,
            start_page,
        });
    }
    if let Some(u) = ch.external_url.clone() {
        return Ok(ChapterOpen {
            book_id: None,
            pages: vec![],
            external_url: Some(u),
            start_page,
        });
    }
    let info = super::server::ensure(&app).await?;
    let pages = sources::pages(&s.source, &ch.key, data_saver.unwrap_or(false))
        .await?
        .iter()
        .map(|p| sources::proxied(&info, p))
        .collect();
    Ok(ChapterOpen {
        book_id: None,
        pages,
        external_url: None,
        start_page,
    })
}

// ---------- updates ----------

#[derive(Serialize, Clone)]
pub struct UpdateItem {
    pub series_id: i64,
    pub series_title: String,
    pub cover_url: String,
    pub cover: Option<String>,
    pub chapter: ChapterRow,
}

#[tauri::command]
pub async fn read_updates_feed(limit: Option<i64>) -> Result<Vec<UpdateItem>, String> {
    blocking(move || {
        db::with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT s.id, s.title, s.cover_url, s.cover, {} FROM chapters ch JOIN series s ON s.id = ch.series_id
                 WHERE s.in_library = 1 ORDER BY ch.fetched_at DESC, COALESCE(ch.published, 0) DESC, ch.position DESC LIMIT ?1",
                db::CHAPTER_COLS.split(", ").map(|c| format!("ch.{c}")).collect::<Vec<_>>().join(", ")
            ))?;
            let rows = st
                .query_map([limit.unwrap_or(200)], |r| {
                    Ok(UpdateItem {
                        series_id: r.get(0)?,
                        series_title: r.get(1)?,
                        cover_url: r.get(2)?,
                        cover: r.get(3)?,
                        chapter: ChapterRow {
                            id: r.get(4)?,
                            series_id: r.get(5)?,
                            key: r.get(6)?,
                            title: r.get(7)?,
                            number: r.get(8)?,
                            volume: r.get(9)?,
                            lang: r.get(10)?,
                            scanlator: r.get(11)?,
                            published: r.get(12)?,
                            pages: r.get(13)?,
                            external_url: r.get(14)?,
                            position: r.get(15)?,
                            read: r.get::<_, i64>(16)? != 0,
                            bookmarked: r.get::<_, i64>(17)? != 0,
                            last_page: r.get(18)?,
                            read_at: r.get(19)?,
                            fetched_at: r.get(20)?,
                            book_id: r.get(21)?,
                        },
                    })
                })?
                .collect();
            rows
        })
    })
    .await
}

static CHECKING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone, Default)]
pub struct UpdateSummary {
    pub checked: usize,
    pub failed: usize,
    pub new_chapters: usize,
    pub series_with_new: Vec<(i64, String, usize)>,
}

/// Checks every followed series (skipping completed ones unless asked),
/// three at a time. New chapters of series with auto-download on are
/// queued.
pub async fn check_updates(
    app: &AppHandle,
    include_completed: bool,
) -> Result<UpdateSummary, String> {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return Err("already checking".into());
    }
    let res = async {
        let list: Vec<SeriesRow> = blocking(|| db::series_list(true)).await?;
        let list: Vec<SeriesRow> = list
            .into_iter()
            .filter(|s| include_completed || !s.status.eq_ignore_ascii_case("completed"))
            .collect();
        let total = list.len();
        let mut summary = UpdateSummary::default();
        let mut stream = futures::stream::iter(list.into_iter().map(|s| async move {
            let r = sync(s.id).await;
            (s, r)
        }))
        .buffer_unordered(3);
        let mut done = 0;
        let mut to_download = Vec::new();
        while let Some((s, r)) = stream.next().await {
            done += 1;
            summary.checked += 1;
            match r {
                Ok(new) if !new.is_empty() => {
                    summary.new_chapters += new.len();
                    summary
                        .series_with_new
                        .push((s.id, s.title.clone(), new.len()));
                    if s.auto_download && !stream_only(&s.source) {
                        to_download.extend(new);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    summary.failed += 1;
                    tracing::debug!("[read] update {}: {e}", s.title);
                }
            }
            let _ = app.emit(
                "read-updates-progress",
                serde_json::json!({ "done": done, "total": total, "title": s.title }),
            );
        }
        if !to_download.is_empty() {
            enqueue(app, to_download);
        }
        Ok(summary)
    }
    .await;
    CHECKING.store(false, Ordering::SeqCst);
    if let Ok(s) = &res {
        let _ = app.emit("read-updates-done", s);
        let _ = db::kv_set("last_update_check", &db::now_ms());
    }
    res
}

#[tauri::command]
pub async fn read_updates_check(
    app: AppHandle,
    include_completed: Option<bool>,
) -> Result<UpdateSummary, String> {
    let r = check_updates(&app, include_completed.unwrap_or(false)).await;
    changed(&app);
    r
}

/// Background loop: checks followed series every `update_interval_h`
/// hours (0 = off). Started once by `read_status`.
pub fn start_update_loop(app: AppHandle) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            let hours: i64 = db::kv_get("update_interval_h").unwrap_or(12);
            if hours <= 0 {
                continue;
            }
            let last: i64 = db::kv_get("last_update_check").unwrap_or(0);
            if db::now_ms() - last < hours * 3_600_000 {
                continue;
            }
            if let Ok(s) = check_updates(&app, false).await {
                if s.new_chapters > 0 {
                    notify_new(&app, &s);
                }
            }
        }
    });
}

fn notify_new(app: &AppHandle, s: &UpdateSummary) {
    use tauri_plugin_notification::NotificationExt;
    let body = s
        .series_with_new
        .iter()
        .take(4)
        .map(|(_, t, n)| format!("{t} (+{n})"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = app
        .notification()
        .builder()
        .title(format!("{} new chapters", s.new_chapters))
        .body(body)
        .show();
}

#[derive(Serialize, Deserialize)]
pub struct UpdateSettings {
    pub interval_h: i64,
    pub delete_after_read: i64,
    pub download_ahead: i64,
}

#[tauri::command]
pub async fn read_update_settings(set: Option<UpdateSettings>) -> Result<UpdateSettings, String> {
    blocking(move || {
        if let Some(s) = set {
            db::kv_set("update_interval_h", &s.interval_h)?;
            db::kv_set("delete_after_read", &s.delete_after_read)?;
            db::kv_set("download_ahead", &s.download_ahead)?;
        }
        Ok(UpdateSettings {
            interval_h: db::kv_get("update_interval_h").unwrap_or(12),
            delete_after_read: db::kv_get("delete_after_read").unwrap_or(-1),
            download_ahead: db::kv_get("download_ahead").unwrap_or(0),
        })
    })
    .await
}

// ---------- categories ----------

#[derive(Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

#[tauri::command]
pub async fn read_categories() -> Result<Vec<Category>, String> {
    blocking(|| {
        db::with(|c| {
            let mut st =
                c.prepare("SELECT id, name, position FROM categories ORDER BY position, name")?;
            let rows = st
                .query_map([], |r| {
                    Ok(Category {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        position: r.get(2)?,
                    })
                })?
                .collect();
            rows
        })
    })
    .await
}

#[tauri::command]
pub async fn read_category_save(id: Option<i64>, name: String) -> Result<i64, String> {
    blocking(move || {
        db::with(|c| match id {
            Some(id) => {
                c.execute("UPDATE categories SET name = ?1 WHERE id = ?2", params![name, id])?;
                Ok(id)
            }
            None => {
                c.execute(
                    "INSERT INTO categories (name, position) VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM categories))",
                    [name],
                )?;
                Ok(c.last_insert_rowid())
            }
        })
    })
    .await
}

#[tauri::command]
pub async fn read_category_delete(id: i64) -> Result<(), String> {
    blocking(move || {
        db::with(|c| c.execute("DELETE FROM categories WHERE id = ?1", [id])).map(|_| ())
    })
    .await
}

// ---------- migration ----------

/// Moves a followed series to another source: follows the new one, carries
/// read marks (matched by chapter number), categories, reader settings and
/// notes, then unfollows the old one.
#[tauri::command]
pub async fn read_series_migrate(
    app: AppHandle,
    from_id: i64,
    source: String,
    key: String,
) -> Result<i64, String> {
    let old = blocking(move || db::series(from_id)).await?;
    let fresh = read_series_open(source, key, Some(old.chapter_langs.clone())).await?;
    let to = fresh.series.id;
    blocking(move || {
        let read: Vec<f64> = db::chapters_of(from_id)?
            .into_iter()
            .filter(|c| c.read)
            .filter_map(|c| c.number)
            .collect();
        db::with(|c| {
            for n in &read {
                c.execute(
                    "UPDATE chapters SET read = 1, read_at = COALESCE(read_at, ?1) WHERE series_id = ?2 AND ABS(number - ?3) < 0.001",
                    params![db::now_ms(), to, n],
                )?;
            }
            c.execute(
                "INSERT OR IGNORE INTO series_categories (series_id, category_id)
                 SELECT ?1, category_id FROM series_categories WHERE series_id = ?2",
                params![to, from_id],
            )?;
            c.execute(
                "UPDATE series SET in_library = 1, reader = ?1, notes = ?2, tracking = ?3, auto_download = ?4 WHERE id = ?5",
                params![old.reader.to_string(), old.notes, old.tracking.to_string(), old.auto_download as i64, to],
            )?;
            c.execute("UPDATE series SET in_library = 0 WHERE id = ?1", [from_id])?;
            Ok(())
        })
    })
    .await?;
    changed(&app);
    Ok(to)
}

// ---------- downloads ----------

#[derive(Serialize, Clone)]
pub struct QueueItem {
    pub chapter_id: i64,
    pub series_title: String,
    pub chapter_title: String,
    pub state: String,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
}

static QUEUE: Mutex<VecDeque<i64>> = Mutex::new(VecDeque::new());
static STATUS: Mutex<Option<HashMap<i64, QueueItem>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

fn set_status(app: &AppHandle, item: QueueItem) {
    if let Ok(mut s) = STATUS.lock() {
        s.get_or_insert_with(HashMap::new)
            .insert(item.chapter_id, item.clone());
    }
    let _ = app.emit("read-chapter-download", item);
}

pub fn enqueue(app: &AppHandle, ids: Vec<i64>) {
    if let Ok(mut q) = QUEUE.lock() {
        for id in ids {
            if !q.contains(&id) {
                q.push_back(id);
            }
        }
    }
    if !RUNNING.swap(true, Ordering::SeqCst) {
        let app = app.clone();
        CANCEL.store(false, Ordering::SeqCst);
        tauri::async_runtime::spawn(async move {
            loop {
                if CANCEL.load(Ordering::SeqCst) {
                    if let Ok(mut q) = QUEUE.lock() {
                        q.clear();
                    }
                    break;
                }
                let next = QUEUE.lock().ok().and_then(|mut q| q.pop_front());
                let Some(id) = next else { break };
                if let Err(e) = download_chapter(&app, id).await {
                    tracing::warn!("[read] chapter {id}: {e}");
                }
            }
            RUNNING.store(false, Ordering::SeqCst);
            let _ = app.emit("read-chapter-queue-idle", ());
        });
    }
}

#[tauri::command]
pub async fn read_chapters_download(app: AppHandle, ids: Vec<i64>) -> Result<usize, String> {
    let ok: Vec<i64> = blocking(move || {
        let mut ok = Vec::new();
        for id in ids {
            let ch = db::chapter(id)?;
            let s = db::series(ch.series_id)?;
            if stream_only(&s.source) || ch.external_url.is_some() || ch.book_id.is_some() {
                continue;
            }
            ok.push(id);
        }
        Ok(ok)
    })
    .await?;
    let n = ok.len();
    for id in &ok {
        let (ch, s) = (db::chapter(*id)?, db::series(db::chapter(*id)?.series_id)?);
        set_status(
            &app,
            QueueItem {
                chapter_id: *id,
                series_title: s.title,
                chapter_title: chapter_label(&ch),
                state: "queued".into(),
                done: 0,
                total: 0,
                error: None,
            },
        );
    }
    enqueue(&app, ok);
    Ok(n)
}

#[tauri::command]
pub fn read_download_queue() -> Vec<QueueItem> {
    let mut v: Vec<QueueItem> = STATUS
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|m| m.values().cloned().collect()))
        .unwrap_or_default();
    v.sort_by_key(|i| (i.state == "done", i.chapter_id));
    v
}

#[tauri::command]
pub fn read_download_cancel(clear_finished: Option<bool>) {
    CANCEL.store(true, Ordering::SeqCst);
    if clear_finished.unwrap_or(false) {
        if let Ok(mut s) = STATUS.lock() {
            if let Some(m) = s.as_mut() {
                m.retain(|_, i| i.state == "running");
            }
        }
    }
}

pub fn chapter_label(ch: &ChapterRow) -> String {
    let mut s = String::new();
    if let Some(v) = ch.volume {
        s.push_str(&format!("Vol.{} ", trim_num(v)));
    }
    if let Some(n) = ch.number {
        s.push_str(&format!("Ch.{}", trim_num(n)));
    }
    if !ch.title.is_empty() && !ch.title.starts_with('#') {
        if !s.is_empty() {
            s.push_str(" - ");
        }
        s.push_str(&ch.title);
    }
    if s.is_empty() {
        s = ch.title.clone();
    }
    s
}

fn trim_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{:03}", n as i64)
    } else {
        let whole = n.trunc() as i64;
        let frac = format!("{:.3}", n.fract());
        format!(
            "{whole:03}{}",
            frac.trim_start_matches('0').trim_end_matches('0')
        )
    }
}

fn fail(app: &AppHandle, mut item: QueueItem, e: String) -> String {
    item.state = "failed".into();
    item.error = Some(e.clone());
    set_status(app, item);
    e
}

async fn download_chapter(app: &AppHandle, id: i64) -> Result<(), String> {
    let (ch, s) = blocking(move || {
        let ch = db::chapter(id)?;
        let s = db::series(ch.series_id)?;
        Ok((ch, s))
    })
    .await?;
    let label = chapter_label(&ch);
    let mut item = QueueItem {
        chapter_id: id,
        series_title: s.title.clone(),
        chapter_title: label.clone(),
        state: "running".into(),
        done: 0,
        total: 0,
        error: None,
    };
    set_status(app, item.clone());
    let pages = match sources::pages(&s.source, &ch.key, false).await {
        Ok(p) => p,
        Err(e) => return Err(fail(app, item, e)),
    };
    item.total = pages.len();
    let client = sources::http();
    let mut images: Vec<Option<(String, Vec<u8>)>> = vec![None; pages.len()];
    let mut stream = futures::stream::iter(pages.clone().into_iter().enumerate().map(
        move |(i, p)| async move {
            let mut last = String::new();
            for attempt in 0..3 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(800 * attempt)).await;
                }
                let started = std::time::Instant::now();
                let mut req = client.get(&p.url);
                for (k, v) in &p.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                match req.send().await {
                    Ok(r) if r.status().is_success() => {
                        let cached = r
                            .headers()
                            .get("x-cache")
                            .and_then(|v| v.to_str().ok())
                            .is_some_and(|v| v.starts_with("HIT"));
                        match r.bytes().await {
                            Ok(b) => {
                                let mut data = b.to_vec();
                                if let Some(k) = &p.xor_key {
                                    super::server::xor_decrypt(&mut data, k);
                                }
                                report_mangadex(
                                    client,
                                    &p.url,
                                    true,
                                    data.len(),
                                    started.elapsed(),
                                    cached,
                                );
                                let ext = if data.starts_with(b"\x89PNG") {
                                    "png"
                                } else if data.get(8..12) == Some(b"WEBP") {
                                    "webp"
                                } else if data.starts_with(b"GIF8") {
                                    "gif"
                                } else {
                                    "jpg"
                                };
                                return (i, Ok((format!("{i}.{ext}"), data)));
                            }
                            Err(e) => last = e.to_string(),
                        }
                    }
                    Ok(r) => {
                        report_mangadex(client, &p.url, false, 0, started.elapsed(), false);
                        last = r.status().to_string();
                    }
                    Err(e) => last = e.to_string(),
                }
            }
            (i, Err(last))
        },
    ))
    .buffer_unordered(4);
    while let Some((i, r)) = stream.next().await {
        if CANCEL.load(Ordering::SeqCst) {
            item.state = "cancelled".into();
            set_status(app, item);
            return Err("cancelled".into());
        }
        match r {
            Ok(img) => {
                images[i] = Some(img);
                item.done += 1;
                set_status(app, item.clone());
            }
            Err(e) => return Err(fail(app, item, format!("page {}: {e}", i + 1))),
        }
    }
    let pages: Vec<(String, Vec<u8>)> = images.into_iter().flatten().collect();
    let series_dir = PathBuf::from(super::default_download_dir())
        .join("Manga")
        .join(sanitize_filename::sanitize(&s.title));
    // The same chapter in two languages or by two groups gets its own file.
    let mut file = series_dir.join(sanitize_filename::sanitize(format!("{label}.cbz")));
    if file.exists() {
        let tag = [ch.lang.as_str(), ch.scanlator.as_str()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        file = series_dir.join(sanitize_filename::sanitize(format!("{label} [{tag}].cbz")));
        if file.exists() {
            file = series_dir.join(sanitize_filename::sanitize(format!(
                "{label} [{tag}] {id}.cbz"
            )));
        }
    }
    let info = ComicInfo {
        title: ch.title.clone(),
        series: s.title.clone(),
        number: ch.number.map(|n| n.to_string()).unwrap_or_default(),
        volume: ch.volume.map(|v| v as i64),
        summary: s.description.clone(),
        writer: s.authors.join(", "),
        translator: ch.scanlator.clone(),
        genre: s.genres.join(", "),
        web: s.url.clone(),
        page_count: Some(pages.len() as i64),
        language_iso: ch.lang.clone(),
        manga: if matches!(s.lang.as_str(), "ja" | "ja-ro") || s.source == "mangadex" {
            "YesAndRightToLeft".into()
        } else {
            "Yes".into()
        },
        year: ch
            .published
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| chrono::Datelike::year(&d) as i64),
        ..Default::default()
    };
    let (sid, cid, src_url) = (s.id, ch.id, s.url.clone());
    let res = blocking(move || {
        comic::write_cbz(&file, &pages, Some(&info))?;
        let book = scan::index_file(
            &file,
            &scan::IndexOpts {
                source: "manga".into(),
                source_url: src_url,
                series_id: Some(sid),
                chapter_id: Some(cid),
            },
        )?;
        db::with(|c| {
            c.execute(
                "UPDATE chapters SET book_id = ?1 WHERE id = ?2",
                params![book, cid],
            )
        })?;
        Ok(())
    })
    .await;
    match res {
        Ok(()) => {
            item.state = "done".into();
            set_status(app, item);
            let _ = app.emit("read-library-changed", ());
            changed(app);
            Ok(())
        }
        Err(e) => Err(fail(app, item, e)),
    }
}

/// MangaDex asks clients to report how its image servers behave.
fn report_mangadex(
    client: &reqwest::Client,
    url: &str,
    success: bool,
    bytes: usize,
    took: std::time::Duration,
    cached: bool,
) {
    if !url.contains(".mangadex.network/") || url.contains("uploads.mangadex.org") {
        return;
    }
    let client = client.clone();
    let body = serde_json::json!({
        "url": url,
        "success": success,
        "bytes": bytes,
        "duration": took.as_millis() as u64,
        "cached": cached,
    });
    tokio::spawn(async move {
        let _ = client
            .post("https://api.mangadex.network/report")
            .json(&body)
            .send()
            .await;
    });
}

/// Deletes downloaded chapter files (the chapters stay in the list).
#[tauri::command]
pub async fn read_chapters_delete_downloads(app: AppHandle, ids: Vec<i64>) -> Result<(), String> {
    blocking(move || {
        for id in ids {
            let ch = db::chapter(id)?;
            if let Some(b) = ch.book_id {
                if let Ok(book) = db::book(b) {
                    let _ = std::fs::remove_file(&book.path);
                }
                db::with(|c| c.execute("DELETE FROM books WHERE id = ?1", [b]))?;
            }
            db::with(|c| c.execute("UPDATE chapters SET book_id = NULL WHERE id = ?1", [id]))?;
        }
        Ok(())
    })
    .await?;
    changed(&app);
    let _ = app.emit("read-library-changed", ());
    Ok(())
}

/// While reading chapter `id`, keeps the next `download_ahead` chapters
/// downloaded (0 = off).
#[tauri::command]
pub async fn read_download_ahead(app: AppHandle, id: i64) -> Result<(), String> {
    let n: i64 = db::kv_get("download_ahead").unwrap_or(0);
    if n <= 0 {
        return Ok(());
    }
    let ids = blocking(move || {
        let ch = db::chapter(id)?;
        let s = db::series(ch.series_id)?;
        if stream_only(&s.source) {
            return Ok(vec![]);
        }
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT id FROM chapters WHERE series_id = ?1 AND position > ?2 AND book_id IS NULL AND external_url IS NULL
                 ORDER BY position LIMIT ?3",
            )?;
            let rows = st.query_map(params![ch.series_id, ch.position, n], |r| r.get(0))?.collect();
            rows
        })
    })
    .await?;
    if !ids.is_empty() {
        read_chapters_download(app, ids).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let ch = ChapterRow {
            number: Some(13.5),
            volume: Some(2.0),
            title: "The Duel".into(),
            ..Default::default()
        };
        assert_eq!(chapter_label(&ch), "Vol.002 Ch.013.5 - The Duel");
        let ch = ChapterRow {
            number: Some(1.0),
            title: "#001".into(),
            ..Default::default()
        };
        assert_eq!(chapter_label(&ch), "Ch.001");
    }

    #[test]
    fn orders_by_chapter_number() {
        let ch = |key: &str, number: Option<f64>| RemoteChapter {
            key: key.into(),
            number,
            ..Default::default()
        };
        let list = [
            ch("114", Some(114.0)),
            ch("2", Some(2.0)),
            ch("extra", None),
            ch("1", Some(1.0)),
        ];
        let keys: Vec<&str> = reading_order(&list)
            .iter()
            .map(|c| c.key.as_str())
            .collect();
        assert_eq!(keys, ["1", "2", "extra", "114"]);
    }
}

// ---------- trackers ----------

use omniget_core::core::read::trackers::{self, TrackHit};

#[derive(Serialize)]
pub struct TrackerStatus {
    pub anilist: Option<String>,
    pub mangaupdates: bool,
}

#[tauri::command]
pub async fn read_tracker_status() -> Result<TrackerStatus, String> {
    let token = super::secret("tracker:anilist");
    let anilist = if token.is_empty() {
        None
    } else {
        Some(
            trackers::anilist_viewer(sources::http(), &token)
                .await
                .unwrap_or_else(|_| "?".into()),
        )
    };
    Ok(TrackerStatus {
        anilist,
        mangaupdates: !super::secret("tracker:mangaupdates").is_empty(),
    })
}

/// AniList takes a personal access token; MangaUpdates a user and password
/// (only the session token is kept). Empty values log out.
#[tauri::command]
pub async fn read_tracker_login(
    service: String,
    token: Option<String>,
    user: Option<String>,
    password: Option<String>,
) -> Result<String, String> {
    match service.as_str() {
        "anilist" => {
            let t = token.unwrap_or_default();
            if t.is_empty() {
                super::set_secret("tracker:anilist", "")?;
                return Ok(String::new());
            }
            let name = trackers::anilist_viewer(sources::http(), t.trim()).await?;
            super::set_secret("tracker:anilist", t.trim())?;
            Ok(name)
        }
        "mangaupdates" => {
            let (u, p) = (user.unwrap_or_default(), password.unwrap_or_default());
            if u.is_empty() {
                super::set_secret("tracker:mangaupdates", "")?;
                return Ok(String::new());
            }
            let session = trackers::mangaupdates_login(sources::http(), &u, &p).await?;
            super::set_secret("tracker:mangaupdates", &session)?;
            Ok(u)
        }
        other => Err(format!("unknown tracker {other}")),
    }
}

#[tauri::command]
pub async fn read_tracker_search(service: String, query: String) -> Result<Vec<TrackHit>, String> {
    match service.as_str() {
        "anilist" => trackers::anilist_search(sources::http(), &query).await,
        "mangaupdates" => trackers::mangaupdates_search(sources::http(), &query).await,
        other => Err(format!("unknown tracker {other}")),
    }
}

/// Links (or with `hit = null` unlinks) a series to a tracker entry and
/// pushes the current progress right away.
#[tauri::command]
pub async fn read_tracker_link(
    app: AppHandle,
    series_id: i64,
    service: String,
    hit: Option<TrackHit>,
) -> Result<(), String> {
    let s = blocking(move || db::series(series_id)).await?;
    let mut tracking = if s.tracking.is_object() {
        s.tracking.clone()
    } else {
        serde_json::json!({})
    };
    match hit {
        Some(h) => {
            tracking[&service] = serde_json::json!({ "id": h.id, "title": h.title, "url": h.url })
        }
        None => {
            if let Some(o) = tracking.as_object_mut() {
                o.remove(&service);
            }
        }
    }
    read_series_update(
        app,
        series_id,
        SeriesPatch {
            tracking: Some(tracking),
            ..Default::default()
        },
    )
    .await?;
    push_tracking(series_id).await;
    Ok(())
}

/// Sends the highest chapter read to every linked tracker. Best effort.
pub async fn push_tracking(series_id: i64) {
    let Ok((s, last, total)) = blocking(move || {
        let s = db::series(series_id)?;
        let chs = db::chapters_of(series_id)?;
        let last = chs
            .iter()
            .filter(|c| c.read)
            .filter_map(|c| c.number)
            .fold(0.0f64, f64::max);
        let total = chs.iter().filter_map(|c| c.number).fold(0.0f64, f64::max);
        Ok((s, last, total))
    })
    .await
    else {
        return;
    };
    let chapter = last.floor() as i64;
    if chapter <= 0 {
        return;
    }
    let completed = s.status.to_lowercase().contains("complet") && last >= total && total > 0.0;
    if let Some(id) = s.tracking["anilist"]["id"].as_i64() {
        let token = super::secret("tracker:anilist");
        if !token.is_empty() {
            if let Err(e) =
                trackers::anilist_progress(sources::http(), &token, id, chapter, completed).await
            {
                tracing::debug!("[read] anilist: {e}");
            }
        }
    }
    if let Some(id) = s.tracking["mangaupdates"]["id"].as_i64() {
        let token = super::secret("tracker:mangaupdates");
        if !token.is_empty() {
            if let Err(e) =
                trackers::mangaupdates_progress(sources::http(), &token, id, chapter).await
            {
                tracing::debug!("[read] mangaupdates: {e}");
            }
        }
    }
}
