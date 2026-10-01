use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::blocking;
use super::db::{self, Annotation, BookRow};

fn changed(app: &AppHandle) {
    let _ = app.emit("read-library-changed", ());
}

#[tauri::command]
pub async fn read_books() -> Result<Vec<BookRow>, String> {
    blocking(db::all_books).await
}

#[tauri::command]
pub async fn read_book(id: i64) -> Result<BookRow, String> {
    blocking(move || db::book(id)).await
}

#[tauri::command]
pub async fn read_search(query: String) -> Result<Vec<i64>, String> {
    blocking(move || {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| t.replace('"', ""))
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{t}\"*"))
            .collect();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let q = terms.join(" ");
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT rowid FROM books_fts WHERE books_fts MATCH ?1 ORDER BY bm25(books_fts, 10.0, 6.0, 4.0, 1.0, 2.0, 2.0) LIMIT 500",
            )?;
            let rows = st.query_map([q], |r| r.get(0))?.collect();
            rows
        })
    })
    .await
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct BookPatch {
    pub title: Option<String>,
    pub authors: Option<Vec<String>>,
    pub series: Option<String>,
    pub series_index: Option<Option<f64>>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub subjects: Option<Vec<String>>,
    pub isbn: Option<String>,
    pub published: Option<String>,
    pub rating: Option<i64>,
    pub favorite: Option<bool>,
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
    pub prefs: Option<serde_json::Value>,
    pub rtl: Option<bool>,
    pub write_file: bool,
}

#[tauri::command]
pub async fn read_update_book(
    app: AppHandle,
    id: i64,
    patch: BookPatch,
) -> Result<BookRow, String> {
    let row = blocking(move || {
        let now = db::now_ms();
        db::with(|c| {
            let set = |col: &str, v: &dyn rusqlite::ToSql| -> rusqlite::Result<usize> {
                c.execute(&format!("UPDATE books SET {col} = ?1 WHERE id = ?2"), params![v, id])
            };
            let js = |v: &Vec<String>| serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
            if let Some(v) = &patch.title {
                set("title", v)?;
                set("sort_title", &db::sort_key(v))?;
            }
            if let Some(v) = &patch.authors {
                set("authors", &js(v))?;
            }
            if let Some(v) = &patch.series {
                set("series", v)?;
            }
            if let Some(v) = &patch.series_index {
                set("series_index", v)?;
            }
            if let Some(v) = &patch.language {
                set("language", v)?;
            }
            if let Some(v) = &patch.publisher {
                set("publisher", v)?;
            }
            if let Some(v) = &patch.description {
                set("description", v)?;
            }
            if let Some(v) = &patch.subjects {
                set("subjects", &js(v))?;
            }
            if let Some(v) = &patch.isbn {
                set("isbn", v)?;
            }
            if let Some(v) = &patch.published {
                set("published", v)?;
            }
            if let Some(v) = patch.rating {
                set("rating", &v.clamp(0, 5))?;
            }
            if let Some(v) = patch.favorite {
                set("favorite", &(v as i64))?;
            }
            if let Some(v) = patch.rtl {
                set("rtl", &(v as i64))?;
            }
            if let Some(v) = &patch.tags {
                set("tags", &js(v))?;
            }
            if let Some(v) = &patch.prefs {
                set("prefs", &v.to_string())?;
            }
            if let Some(v) = &patch.status {
                set("status", v)?;
                match v.as_str() {
                    "finished" => {
                        c.execute(
                            "UPDATE books SET finished_at = COALESCE(finished_at, ?1), progress = 1 WHERE id = ?2",
                            params![now, id],
                        )?;
                    }
                    "reading" => {
                        c.execute(
                            "UPDATE books SET started_at = COALESCE(started_at, ?1) WHERE id = ?2",
                            params![now, id],
                        )?;
                    }
                    "unread" => {
                        c.execute(
                            "UPDATE books SET progress = 0, location = '', finished_at = NULL WHERE id = ?1",
                            params![id],
                        )?;
                    }
                    _ => {}
                }
            }
            Ok(())
        })?;
        let b = db::book(id)?;
        if patch.write_file && (b.format == "epub" || b.format == "kepub") {
            let edit = omniget_core::core::read::epub::MetaEdit {
                title: patch.title.clone(),
                authors: patch.authors.clone(),
                language: patch.language.clone(),
                publisher: patch.publisher.clone(),
                description: patch.description.clone(),
                subjects: patch.subjects.clone(),
                series: patch.series.clone(),
                series_index: patch.series_index.flatten(),
                published: patch.published.clone(),
                isbn: patch.isbn.clone(),
            };
            omniget_core::core::read::epub::write_metadata(std::path::Path::new(&b.path), &edit)?;
            if let Ok(m) = std::fs::metadata(&b.path) {
                let mt = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                db::with(|c| {
                    c.execute(
                        "UPDATE books SET mtime_ms = ?1, size = ?2 WHERE id = ?3",
                        params![mt, m.len() as i64, id],
                    )
                })?;
            }
        }
        db::book(id)
    })
    .await?;
    changed(&app);
    Ok(row)
}

#[tauri::command]
pub async fn read_bulk_update(
    app: AppHandle,
    ids: Vec<i64>,
    status: Option<String>,
    favorite: Option<bool>,
    add_tags: Option<Vec<String>>,
    remove_tags: Option<Vec<String>>,
) -> Result<(), String> {
    for id in ids {
        let mut tags = None;
        if add_tags.is_some() || remove_tags.is_some() {
            let b = db::book(id)?;
            let mut t = b.tags;
            for a in add_tags.iter().flatten() {
                if !t.contains(a) {
                    t.push(a.clone());
                }
            }
            if let Some(r) = &remove_tags {
                t.retain(|x| !r.contains(x));
            }
            tags = Some(t);
        }
        read_update_book(
            app.clone(),
            id,
            BookPatch {
                status: status.clone(),
                favorite,
                tags,
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn read_remove_books(
    app: AppHandle,
    ids: Vec<i64>,
    delete_files: bool,
) -> Result<(), String> {
    blocking(move || {
        for id in &ids {
            if delete_files {
                if let Ok(b) = db::book(*id) {
                    let _ = trash(&b.path);
                }
            }
            db::with(|c| c.execute("DELETE FROM books WHERE id = ?1", [id]))?;
        }
        Ok(())
    })
    .await?;
    changed(&app);
    Ok(())
}

fn trash(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let dir = db::read_dir().ok_or("no data dir")?.join("trash");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = p.file_name().ok_or("bad path")?;
    let dest = dir.join(format!("{}-{}", db::now_ms(), name.to_string_lossy()));
    std::fs::rename(p, &dest)
        .or_else(|_| {
            if p.is_dir() {
                Err(std::io::Error::other("cross-device folder"))
            } else {
                std::fs::copy(p, &dest).and_then(|_| std::fs::remove_file(p))
            }
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_save_progress(
    id: i64,
    progress: f64,
    location: String,
    pages: Option<i64>,
) -> Result<(), String> {
    blocking(move || {
        let now = db::now_ms();
        let p = progress.clamp(0.0, 1.0);
        db::with(|c| {
            c.execute(
                "UPDATE books SET progress = ?1, location = ?2, last_read_at = ?3,
                    pages = COALESCE(?5, pages),
                    started_at = COALESCE(started_at, ?3),
                    status = CASE WHEN status IN ('unread', 'want') THEN 'reading' ELSE status END
                 WHERE id = ?4",
                params![p, location, now, id, pages],
            )
        })?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn read_log_session(
    book_id: Option<i64>,
    chapter_id: Option<i64>,
    started_at: i64,
    ms: i64,
    from_progress: f64,
    to_progress: f64,
    pages: i64,
) -> Result<(), String> {
    if ms < 5_000 {
        return Ok(());
    }
    blocking(move || {
        db::with(|c| {
            c.execute(
                "INSERT INTO sessions (book_id, chapter_id, started_at, ms, from_progress, to_progress, pages)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![book_id, chapter_id, started_at, ms, from_progress, to_progress, pages],
            )?;
            if let Some(b) = book_id {
                c.execute("UPDATE books SET read_ms = read_ms + ?1 WHERE id = ?2", params![ms, b])?;
            }
            Ok(())
        })
    })
    .await
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Shelf {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub smart: Option<SmartRules>,
    pub position: i64,
    pub book_ids: Vec<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct SmartRules {
    #[serde(default = "all")]
    pub matching: String,
    pub rules: Vec<SmartRule>,
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

fn all() -> String {
    "all".into()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SmartRule {
    pub field: String,
    pub op: String,
    pub value: serde_json::Value,
}

fn text_op(hay: &str, op: &str, v: &str) -> bool {
    let h = hay.to_lowercase();
    let v = v.to_lowercase();
    match op {
        "is" => h == v,
        "is_not" => h != v,
        "contains" => h.contains(&v),
        "not_contains" => !h.contains(&v),
        "starts" => h.starts_with(&v),
        "empty" => h.is_empty(),
        "not_empty" => !h.is_empty(),
        _ => false,
    }
}

fn num_op(x: f64, op: &str, v: f64) -> bool {
    match op {
        "eq" | "is" => (x - v).abs() < f64::EPSILON,
        "gt" => x > v,
        "gte" => x >= v,
        "lt" => x < v,
        "lte" => x <= v,
        "ne" | "is_not" => (x - v).abs() >= f64::EPSILON,
        _ => false,
    }
}

pub fn rule_matches(b: &BookRow, r: &SmartRule, now: i64) -> bool {
    let s = r.value.as_str().unwrap_or_default();
    let n = r.value.as_f64().or_else(|| s.parse().ok()).unwrap_or(0.0);
    let day = 86_400_000.0;
    let any_text = |list: &[String]| -> bool {
        match r.op.as_str() {
            "not_contains" | "is_not" => list.iter().all(|x| text_op(x, &r.op, s)),
            "empty" => list.is_empty(),
            "not_empty" => !list.is_empty(),
            _ => list.iter().any(|x| text_op(x, &r.op, s)),
        }
    };
    match r.field.as_str() {
        "title" => text_op(&b.title, &r.op, s),
        "author" => any_text(&b.authors),
        "series" => text_op(&b.series, &r.op, s),
        "publisher" => text_op(&b.publisher, &r.op, s),
        "language" => text_op(&b.language, &r.op, s),
        "format" => text_op(&b.format, &r.op, s),
        "status" => text_op(&b.status, &r.op, s),
        "source" => text_op(&b.source, &r.op, s),
        "subject" => any_text(&b.subjects),
        "tag" => any_text(&b.tags),
        "path" => text_op(&b.path, &r.op, s),
        "rating" => num_op(b.rating as f64, &r.op, n),
        "progress" => num_op(b.progress * 100.0, &r.op, n),
        "favorite" => b.favorite == r.value.as_bool().unwrap_or(s == "true"),
        "added_days" => num_op((now - b.added_at) as f64 / day, &r.op, n),
        "read_days" => b
            .last_read_at
            .map(|t| num_op((now - t) as f64 / day, &r.op, n))
            .unwrap_or(r.op == "gt" || r.op == "gte"),
        "size_mb" => num_op(b.size as f64 / 1_048_576.0, &r.op, n),
        "pages" => num_op(b.pages.unwrap_or(0) as f64, &r.op, n),
        "read_hours" => num_op(b.read_ms as f64 / 3_600_000.0, &r.op, n),
        _ => false,
    }
}

pub fn smart_books(rules: &SmartRules, books: &[BookRow]) -> Vec<i64> {
    let now = db::now_ms();
    let mut hits: Vec<&BookRow> = books
        .iter()
        .filter(|b| {
            if rules.rules.is_empty() {
                return true;
            }
            if rules.matching == "any" {
                rules.rules.iter().any(|r| rule_matches(b, r, now))
            } else {
                rules.rules.iter().all(|r| rule_matches(b, r, now))
            }
        })
        .collect();
    match rules.sort.as_str() {
        "added" => hits.sort_by_key(|b| Reverse(b.added_at)),
        "read" => hits.sort_by_key(|b| Reverse(b.last_read_at)),
        "rating" => hits.sort_by_key(|b| Reverse(b.rating)),
        "random" => {
            use rand::seq::SliceRandom;
            hits.shuffle(&mut rand::rng());
        }
        _ => {}
    }
    let mut ids: Vec<i64> = hits.into_iter().map(|b| b.id).collect();
    if let Some(l) = rules.limit {
        ids.truncate(l);
    }
    ids
}

pub fn shelf_book_ids(id: i64) -> Result<Vec<i64>, String> {
    let smart: Option<String> = db::with(|c| {
        c.query_row("SELECT smart FROM shelves WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map(|o| o.flatten())
    })?;
    if let Some(rules) = smart.and_then(|s| serde_json::from_str::<SmartRules>(&s).ok()) {
        return Ok(smart_books(&rules, &db::all_books()?));
    }
    db::with(|c| {
        let mut st =
            c.prepare("SELECT book_id FROM shelf_items WHERE shelf_id = ?1 ORDER BY position")?;
        let rows = st.query_map([id], |r| r.get(0))?.collect();
        rows
    })
}

pub fn shelves_raw() -> Result<Vec<(i64, String, usize)>, String> {
    let list: Vec<(i64, String)> = db::with(|c| {
        let mut st = c.prepare("SELECT id, name FROM shelves ORDER BY position, name")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
        rows
    })?;
    Ok(list
        .into_iter()
        .map(|(id, n)| {
            let c = shelf_book_ids(id).map(|v| v.len()).unwrap_or(0);
            (id, n, c)
        })
        .collect())
}

#[tauri::command]
pub async fn read_shelves() -> Result<Vec<Shelf>, String> {
    blocking(|| {
        let rows: Vec<(i64, String, String, Option<String>, i64)> = db::with(|c| {
            let mut st = c.prepare(
                "SELECT id, name, icon, smart, position FROM shelves ORDER BY position, name",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect();
            rows
        })?;
        let books = db::all_books()?;
        let mut out = Vec::new();
        for (id, name, icon, smart, position) in rows {
            let smart: Option<SmartRules> = smart.and_then(|s| serde_json::from_str(&s).ok());
            let book_ids = match &smart {
                Some(r) => smart_books(r, &books),
                None => shelf_book_ids(id)?,
            };
            out.push(Shelf {
                id,
                name,
                icon,
                smart,
                position,
                book_ids,
            });
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn read_shelf_save(
    app: AppHandle,
    id: Option<i64>,
    name: String,
    icon: Option<String>,
    smart: Option<SmartRules>,
) -> Result<i64, String> {
    let smart_json = smart.map(|s| serde_json::to_string(&s).unwrap_or_default());
    let id = blocking(move || {
        db::with(|c| match id {
            Some(id) => {
                c.execute(
                    "UPDATE shelves SET name = ?1, icon = ?2, smart = ?3 WHERE id = ?4",
                    params![name, icon.unwrap_or_default(), smart_json, id],
                )?;
                Ok(id)
            }
            None => {
                c.execute(
                    "INSERT INTO shelves (name, icon, smart, position, created_at)
                     VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), 0) + 1 FROM shelves), ?4)",
                    params![name, icon.unwrap_or_default(), smart_json, db::now_ms()],
                )?;
                Ok(c.last_insert_rowid())
            }
        })
    })
    .await?;
    changed(&app);
    Ok(id)
}

#[tauri::command]
pub async fn read_shelf_delete(app: AppHandle, id: i64) -> Result<(), String> {
    blocking(move || {
        db::with(|c| c.execute("DELETE FROM shelves WHERE id = ?1", [id])).map(|_| ())
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_shelf_reorder(app: AppHandle, ids: Vec<i64>) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            for (i, id) in ids.iter().enumerate() {
                c.execute(
                    "UPDATE shelves SET position = ?1 WHERE id = ?2",
                    params![i as i64, id],
                )?;
            }
            Ok(())
        })
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_shelf_add(
    app: AppHandle,
    shelf_id: i64,
    book_ids: Vec<i64>,
) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            for b in &book_ids {
                c.execute(
                    "INSERT OR IGNORE INTO shelf_items (shelf_id, book_id, position)
                     VALUES (?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM shelf_items WHERE shelf_id = ?1))",
                    params![shelf_id, b],
                )?;
            }
            Ok(())
        })
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_shelf_remove(
    app: AppHandle,
    shelf_id: i64,
    book_ids: Vec<i64>,
) -> Result<(), String> {
    blocking(move || {
        db::with(|c| {
            for b in &book_ids {
                c.execute(
                    "DELETE FROM shelf_items WHERE shelf_id = ?1 AND book_id = ?2",
                    params![shelf_id, b],
                )?;
            }
            Ok(())
        })
    })
    .await?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn read_annotations(book_id: Option<i64>) -> Result<Vec<Annotation>, String> {
    blocking(move || {
        db::with(|c| {
            let (q, p): (String, Vec<i64>) = match book_id {
                Some(b) => (
                    format!("SELECT {} FROM annotations WHERE book_id = ?1 ORDER BY COALESCE(progress, 0), created_at", db::ANNOTATION_COLS),
                    vec![b],
                ),
                None => (
                    format!("SELECT {} FROM annotations ORDER BY updated_at DESC", db::ANNOTATION_COLS),
                    vec![],
                ),
            };
            let mut st = c.prepare(&q)?;
            let rows = st
                .query_map(rusqlite::params_from_iter(p), db::row_to_annotation)?
                .collect();
            rows
        })
    })
    .await
}

#[tauri::command]
pub async fn read_annotation_save(_app: AppHandle, a: Annotation) -> Result<Annotation, String> {
    let saved = blocking(move || {
        let now = db::now_ms();
        let id = db::with(|c| {
            if a.id > 0 {
                c.execute(
                    "UPDATE annotations SET kind = ?1, locator = ?2, text = ?3, note = ?4, color = ?5,
                        style = ?6, chapter = ?7, progress = ?8, updated_at = ?9 WHERE id = ?10",
                    params![a.kind, a.locator, a.text, a.note, a.color, a.style, a.chapter, a.progress, now, a.id],
                )?;
                Ok(a.id)
            } else {
                c.execute(
                    "INSERT INTO annotations (book_id, kind, locator, text, note, color, style, chapter, progress, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    params![a.book_id, a.kind, a.locator, a.text, a.note,
                        if a.color.is_empty() { "yellow" } else { &a.color },
                        if a.style.is_empty() { "highlight" } else { &a.style },
                        a.chapter, a.progress, now],
                )?;
                Ok(c.last_insert_rowid())
            }
        })?;
        let row = db::with(|c| {
            c.query_row(
                &format!("SELECT {} FROM annotations WHERE id = ?1", db::ANNOTATION_COLS),
                [id],
                db::row_to_annotation,
            )
        })?;
        let _ = live_export(row.book_id);
        Ok(row)
    })
    .await?;
    Ok(saved)
}

#[tauri::command]
pub async fn read_annotation_delete(_app: AppHandle, id: i64) -> Result<(), String> {
    let _book = blocking(move || {
        let book: Option<i64> = db::with(|c| {
            c.query_row("SELECT book_id FROM annotations WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
        })?;
        db::with(|c| c.execute("DELETE FROM annotations WHERE id = ?1", [id]))?;
        if let Some(b) = book {
            let _ = live_export(b);
        }
        Ok(book)
    })
    .await?;
    Ok(())
}

fn book_annotations(book_id: i64) -> Result<Vec<Annotation>, String> {
    db::with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {} FROM annotations WHERE book_id = ?1 ORDER BY COALESCE(progress, 0), created_at",
            db::ANNOTATION_COLS
        ))?;
        let rows = st.query_map([book_id], db::row_to_annotation)?.collect();
        rows
    })
}

fn fmt_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub fn annotations_markdown(b: &BookRow, list: &[Annotation]) -> String {
    let mut o = format!("# {}\n\n", b.title);
    if !b.authors.is_empty() {
        o.push_str(&format!("*{}*\n\n", b.authors.join(", ")));
    }
    let mut chapter = String::new();
    for a in list {
        if a.chapter != chapter && !a.chapter.is_empty() {
            chapter = a.chapter.clone();
            o.push_str(&format!("## {chapter}\n\n"));
        }
        let link = format!(
            "omniget://read?book={}&loc={}",
            b.id,
            urlencoding::encode(&a.locator)
        );
        match a.kind.as_str() {
            "bookmark" => o.push_str(&format!(
                "- 🔖 [{}]({link}){}\n\n",
                if a.text.is_empty() {
                    "Bookmark"
                } else {
                    &a.text
                },
                a.progress
                    .map(|p| format!(" ({:.0}%)", p * 100.0))
                    .unwrap_or_default()
            )),
            _ => {
                if !a.text.is_empty() {
                    for line in a.text.lines() {
                        o.push_str(&format!("> {line}\n"));
                    }
                }
                if !a.note.is_empty() {
                    o.push_str(&format!("\n{}\n", a.note));
                }
                o.push_str(&format!(
                    "\n[{}]({link}) · {}\n\n",
                    fmt_date(a.created_at),
                    a.color
                ));
            }
        }
    }
    o
}

fn csv_cell(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[tauri::command]
pub async fn read_annotations_export(
    book_id: i64,
    format: String,
    path: String,
) -> Result<(), String> {
    blocking(move || {
        let b = db::book(book_id)?;
        let list = book_annotations(book_id)?;
        let body = match format.as_str() {
            "json" => serde_json::to_string_pretty(&serde_json::json!({
                "book": { "title": b.title, "authors": b.authors, "isbn": b.isbn },
                "annotations": list,
            }))
            .map_err(|e| e.to_string())?,
            "csv" => {
                let mut o = String::from("kind,chapter,text,note,color,progress,created\n");
                for a in &list {
                    o.push_str(&format!(
                        "{},{},{},{},{},{},{}\n",
                        a.kind,
                        csv_cell(&a.chapter),
                        csv_cell(&a.text),
                        csv_cell(&a.note),
                        a.color,
                        a.progress.map(|p| format!("{:.3}", p)).unwrap_or_default(),
                        fmt_date(a.created_at)
                    ));
                }
                o
            }
            "html" => {
                let md = annotations_markdown(&b, &list);
                let body = marked_like(&md);
                format!("<!doctype html><meta charset=utf-8><title>{}</title><style>body{{font:16px/1.6 Georgia,serif;max-width:720px;margin:2rem auto;padding:0 1rem}}blockquote{{border-left:4px solid #e5c100;margin:0;padding-left:1rem;color:#333}}</style>{body}", omniget_core::core::read::xml::escape(&b.title))
            }
            "txt" => list
                .iter()
                .map(|a| format!("{}\n{}\n", a.text, a.note))
                .collect::<Vec<_>>()
                .join("\n---\n"),
            _ => annotations_markdown(&b, &list),
        };
        std::fs::write(&path, body).map_err(|e| e.to_string())
    })
    .await
}

fn marked_like(md: &str) -> String {
    let esc = omniget_core::core::read::xml::escape;
    let link = regex::Regex::new(r"\[([^\]]*)\]\(([^)]*)\)").expect("static regex");
    let mut out = String::new();
    for line in md.lines() {
        let l = esc(line);
        let l = link.replace_all(&l, "<a href=\"$2\">$1</a>").to_string();
        if let Some(h) = l.strip_prefix("## ") {
            out.push_str(&format!("<h2>{h}</h2>\n"));
        } else if let Some(h) = l.strip_prefix("# ") {
            out.push_str(&format!("<h1>{h}</h1>\n"));
        } else if let Some(q) = l.strip_prefix("&gt; ") {
            out.push_str(&format!("<blockquote>{q}</blockquote>\n"));
        } else if !l.trim().is_empty() {
            out.push_str(&format!("<p>{l}</p>\n"));
        }
    }
    out
}

fn live_export(book_id: i64) -> Result<(), String> {
    let Some(dir) = db::kv_get::<String>("notes_dir").filter(|d| !d.is_empty()) else {
        return Ok(());
    };
    let b = db::book(book_id)?;
    let list = book_annotations(book_id)?;
    let name = sanitize_filename::sanitize(format!("{}.md", b.title));
    let path = std::path::Path::new(&dir).join(name);
    if list.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(path, annotations_markdown(&b, &list)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_notes_dir(dir: Option<String>) -> Result<String, String> {
    blocking(move || {
        if let Some(d) = dir {
            db::kv_set("notes_dir", &d)?;
            if !d.is_empty() {
                let ids: Vec<i64> = db::with(|c| {
                    let mut st = c.prepare("SELECT DISTINCT book_id FROM annotations")?;
                    let rows = st.query_map([], |r| r.get(0))?.collect();
                    rows
                })?;
                for id in ids {
                    let _ = live_export(id);
                }
            }
        }
        Ok(db::kv_get::<String>("notes_dir").unwrap_or_default())
    })
    .await
}

#[derive(Debug, Serialize, Default)]
pub struct Stats {
    pub books: i64,
    pub comics: i64,
    pub finished: i64,
    pub reading: i64,
    pub finished_this_year: i64,
    pub total_ms: i64,
    pub today_ms: i64,
    pub week_ms: i64,
    pub month_ms: i64,
    pub year_ms: i64,
    pub streak_days: i64,
    pub longest_streak: i64,
    pub days: BTreeMap<String, i64>,
    pub hours: Vec<i64>,
    pub top_authors: Vec<(String, i64)>,
    pub top_books: Vec<(i64, String, i64)>,
    pub formats: Vec<(String, i64)>,
    pub finished_by_month: BTreeMap<String, i64>,
    pub chapters_read: i64,
    pub annotations: i64,
    pub goal: Option<Goal>,
    pub avg_session_ms: i64,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Goal {
    pub year: i32,
    pub books: i64,
    pub minutes_per_day: i64,
}

#[tauri::command]
pub async fn read_stats() -> Result<Stats, String> {
    blocking(|| {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let now = Local::now();
        let books = db::all_books()?;
        let mut s = Stats {
            books: books.len() as i64,
            comics: books
                .iter()
                .filter(|b| matches!(b.format.as_str(), "cbz" | "cbr" | "cb7" | "cbt" | "folder"))
                .count() as i64,
            finished: books.iter().filter(|b| b.status == "finished").count() as i64,
            reading: books.iter().filter(|b| b.status == "reading").count() as i64,
            hours: vec![0; 24],
            ..Default::default()
        };
        let year_start = Local
            .with_ymd_and_hms(now.year(), 1, 1, 0, 0, 0)
            .single()
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        s.finished_this_year = books
            .iter()
            .filter(|b| b.finished_at.is_some_and(|f| f >= year_start))
            .count() as i64;
        for b in books.iter().filter_map(|b| b.finished_at) {
            if let Some(d) = Local.timestamp_millis_opt(b).single() {
                *s.finished_by_month
                    .entry(d.format("%Y-%m").to_string())
                    .or_default() += 1;
            }
        }
        let mut fmts: HashMap<String, i64> = HashMap::new();
        for b in &books {
            *fmts.entry(b.format.clone()).or_default() += 1;
        }
        s.formats = fmts.into_iter().collect();
        s.formats.sort_by_key(|f| Reverse(f.1));
        let sessions: Vec<(Option<i64>, i64, i64)> = db::with(|c| {
            let mut st = c.prepare("SELECT book_id, started_at, ms FROM sessions")?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect();
            rows
        })?;
        let today = now.date_naive();
        let mut per_book: HashMap<i64, i64> = HashMap::new();
        for (book, at, ms) in &sessions {
            s.total_ms += ms;
            let Some(d) = Local.timestamp_millis_opt(*at).single() else {
                continue;
            };
            let date = d.date_naive();
            let age = (today - date).num_days();
            if age == 0 {
                s.today_ms += ms;
            }
            if age < 7 {
                s.week_ms += ms;
            }
            if age < 30 {
                s.month_ms += ms;
            }
            if d.year() == now.year() {
                s.year_ms += ms;
            }
            if age <= 370 {
                *s.days
                    .entry(date.format("%Y-%m-%d").to_string())
                    .or_default() += ms;
            }
            s.hours[d.hour() as usize] += ms;
            if let Some(b) = book {
                *per_book.entry(*b).or_default() += ms;
            }
        }
        if !sessions.is_empty() {
            s.avg_session_ms = s.total_ms / sessions.len() as i64;
        }
        let mut day = today;
        let active = |d: &chrono::NaiveDate, days: &BTreeMap<String, i64>| {
            days.get(&d.format("%Y-%m-%d").to_string())
                .copied()
                .unwrap_or(0)
                >= 60_000
        };
        if !active(&day, &s.days) {
            day = day.pred_opt().unwrap_or(day);
        }
        while active(&day, &s.days) {
            s.streak_days += 1;
            day = match day.pred_opt() {
                Some(d) => d,
                None => break,
            };
        }
        let mut run = 0;
        let mut prev: Option<chrono::NaiveDate> = None;
        for (k, v) in &s.days {
            if *v < 60_000 {
                continue;
            }
            let Ok(d) = chrono::NaiveDate::parse_from_str(k, "%Y-%m-%d") else {
                continue;
            };
            run = match prev {
                Some(p) if (d - p).num_days() == 1 => run + 1,
                _ => 1,
            };
            s.longest_streak = s.longest_streak.max(run);
            prev = Some(d);
        }
        let mut authors: HashMap<String, i64> = HashMap::new();
        let by_id: HashMap<i64, &BookRow> = books.iter().map(|b| (b.id, b)).collect();
        for (id, ms) in &per_book {
            if let Some(b) = by_id.get(id) {
                for a in &b.authors {
                    *authors.entry(a.clone()).or_default() += ms;
                }
                s.top_books.push((*id, b.title.clone(), *ms));
            }
        }
        s.top_books.sort_by_key(|b| Reverse(b.2));
        s.top_books.truncate(10);
        s.top_authors = authors.into_iter().collect();
        s.top_authors.sort_by_key(|a| Reverse(a.1));
        s.top_authors.truncate(10);
        s.chapters_read = db::with(|c| {
            c.query_row("SELECT COUNT(*) FROM chapters WHERE read = 1", [], |r| {
                r.get(0)
            })
        })?;
        s.annotations =
            db::with(|c| c.query_row("SELECT COUNT(*) FROM annotations", [], |r| r.get(0)))?;
        s.goal = db::kv_get::<Goal>(&format!("goal:{}", now.year()));
        Ok(s)
    })
    .await
}

#[tauri::command]
pub async fn read_set_goal(goal: Goal) -> Result<(), String> {
    blocking(move || db::kv_set(&format!("goal:{}", goal.year), &goal)).await
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Word {
    pub id: i64,
    pub word: String,
    pub context: String,
    pub definition: String,
    pub lang: String,
    pub book_id: Option<i64>,
    pub book_title: String,
    pub created_at: i64,
}

#[tauri::command]
pub async fn read_vocab_add(
    word: String,
    context: String,
    definition: String,
    lang: String,
    book_id: Option<i64>,
) -> Result<i64, String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "INSERT INTO vocab (word, context, definition, lang, book_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![word.trim(), context, definition, lang, book_id, db::now_ms()],
            )?;
            Ok(c.last_insert_rowid())
        })
    })
    .await
}

#[tauri::command]
pub async fn read_vocab_list() -> Result<Vec<Word>, String> {
    blocking(|| {
        db::with(|c| {
            let mut st = c.prepare(
                "SELECT v.id, v.word, v.context, v.definition, v.lang, v.book_id, COALESCE(b.title, ''), v.created_at
                 FROM vocab v LEFT JOIN books b ON b.id = v.book_id ORDER BY v.created_at DESC",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(Word {
                        id: r.get(0)?,
                        word: r.get(1)?,
                        context: r.get(2)?,
                        definition: r.get(3)?,
                        lang: r.get(4)?,
                        book_id: r.get(5)?,
                        book_title: r.get(6)?,
                        created_at: r.get(7)?,
                    })
                })?
                .collect();
            rows
        })
    })
    .await
}

#[tauri::command]
pub async fn read_vocab_delete(id: i64) -> Result<(), String> {
    blocking(move || db::with(|c| c.execute("DELETE FROM vocab WHERE id = ?1", [id])).map(|_| ()))
        .await
}

#[tauri::command]
pub async fn read_vocab_export(path: String) -> Result<usize, String> {
    let words = read_vocab_list().await?;
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut out = String::from("#separator:tab\n#html:true\n");
    for w in &words {
        let ctx = clean(&w.context).replace(&w.word, &format!("<b>{}</b>", w.word));
        out.push_str(&format!(
            "{}<br><small>{}</small>\t{}<br><small>{}</small>\n",
            clean(&w.word),
            ctx,
            clean(&w.definition),
            clean(&w.book_title)
        ));
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(words.len())
}

#[tauri::command]
pub async fn read_duplicates() -> Result<Vec<Vec<i64>>, String> {
    blocking(|| {
        let books = db::all_books()?;
        let mut groups: HashMap<String, Vec<i64>> = HashMap::new();
        for b in &books {
            if !b.hash.is_empty() {
                groups
                    .entry(format!("h:{}", b.hash))
                    .or_default()
                    .push(b.id);
            }
            let norm: String = b
                .title
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            let author = b
                .authors
                .first()
                .map(|a| a.to_lowercase())
                .unwrap_or_default();
            if !norm.is_empty() {
                groups
                    .entry(format!(
                        "t:{norm}|{author}|{}",
                        b.series_index.unwrap_or(-1.0)
                    ))
                    .or_default()
                    .push(b.id);
            }
        }
        let mut out: Vec<Vec<i64>> = groups.into_values().filter(|g| g.len() > 1).collect();
        for g in &mut out {
            g.sort();
        }
        out.sort();
        out.dedup();
        Ok(out)
    })
    .await
}

pub fn render_template(tpl: &str, b: &BookRow) -> String {
    let field = |k: &str| -> String {
        match k {
            "author" => b.authors.first().cloned().unwrap_or_default(),
            "authors" => b.authors.join(", "),
            "title" => b.title.clone(),
            "series" => b.series.clone(),
            "index" => b
                .series_index
                .map(|i| {
                    if i.fract() == 0.0 {
                        format!("{:02}", i as i64)
                    } else {
                        i.to_string()
                    }
                })
                .unwrap_or_default(),
            "year" => b.published.get(..4).unwrap_or("").to_string(),
            "language" => b.language.clone(),
            "publisher" => b.publisher.clone(),
            "format" => b.format.clone(),
            _ => String::new(),
        }
    };
    let clean = |s: String| sanitize_filename::sanitize(s.replace(['/', '\\'], "-"));
    let re_opt = regex::Regex::new(r"<([^<>]*)>").expect("static regex");
    let re_f = regex::Regex::new(r"\{(\w+)\}").expect("static regex");
    let with_opts = re_opt.replace_all(tpl, |c: &regex::Captures| {
        let inner = &c[1];
        let empty = re_f
            .captures_iter(inner)
            .any(|f| field(&f[1]).trim().is_empty());
        if empty {
            String::new()
        } else {
            inner.to_string()
        }
    });
    re_f.replace_all(&with_opts, |c: &regex::Captures| {
        let v = field(&c[1]);
        if v.is_empty() {
            "Unknown".to_string()
        } else {
            clean(v)
        }
    })
    .to_string()
}

#[derive(Serialize)]
pub struct MovePlan {
    pub id: i64,
    pub from: String,
    pub to: String,
}

#[tauri::command]
pub async fn read_organize(
    app: AppHandle,
    ids: Vec<i64>,
    root: String,
    template: String,
    dry_run: bool,
) -> Result<Vec<MovePlan>, String> {
    let plan = blocking(move || {
        let mut plan = Vec::new();
        for id in ids {
            let b = db::book(id)?;
            let src = std::path::PathBuf::from(&b.path);
            if b.chapter_id.is_some() || src.is_dir() {
                continue;
            }
            let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = if name.to_lowercase().ends_with(".kepub.epub") {
                "kepub.epub".to_string()
            } else {
                src.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string()
            };
            let rel = render_template(&template, &b);
            let dest = std::path::Path::new(&root).join(format!("{rel}.{ext}"));
            if dest != src {
                plan.push(MovePlan {
                    id,
                    from: b.path.clone(),
                    to: dest.to_string_lossy().to_string(),
                });
            }
        }
        if !dry_run {
            for m in &plan {
                let dest = std::path::Path::new(&m.to);
                if dest.exists() {
                    continue;
                }
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::rename(&m.from, dest)
                    .or_else(|_| {
                        std::fs::copy(&m.from, dest).and_then(|_| std::fs::remove_file(&m.from))
                    })
                    .map_err(|e| format!("{}: {e}", m.from))?;
                let folder = dest
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                db::with(|c| {
                    c.execute(
                        "UPDATE books SET path = ?1, folder = ?2 WHERE id = ?3",
                        params![m.to, folder, m.id],
                    )
                })?;
            }
        }
        Ok(plan)
    })
    .await?;
    if !dry_run {
        changed(&app);
    }
    Ok(plan)
}
