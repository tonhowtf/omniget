use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension, Row};

use omniget_core::core::video::notes_md;

use super::{blocking, db, labels_for_key, link, server, study, MediaKind, Note};

fn note_from_row(r: &Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: r.get(0)?,
        key: r.get(1)?,
        t_ms: r.get(2)?,
        t_end_ms: r.get(3)?,
        text: r.get(4)?,
        image: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const NOTE_COLS: &str = "id, key, t_ms, t_end_ms, text, image, created_at, updated_at";

fn notes_of(key: &str) -> Result<Vec<Note>, String> {
    db::with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {NOTE_COLS} FROM notes WHERE key = ?1 ORDER BY t_ms, id"
        ))?;
        let rows = st.query_map([key], note_from_row)?;
        rows.collect()
    })
}

fn note_by_id(id: i64) -> Result<Note, String> {
    db::with(|c| {
        c.query_row(
            &format!("SELECT {NOTE_COLS} FROM notes WHERE id = ?1"),
            [id],
            note_from_row,
        )
        .optional()
    })?
    .ok_or_else(|| "note not found".into())
}

pub fn media_source(key: &str, media_id: Option<i64>) -> Option<String> {
    if let Some(id) = media_id {
        if let Some((path, ..)) = db::media_file(id) {
            return Some(path);
        }
    }
    if let Some(hash) = key.strip_prefix("local:") {
        if let Ok(Some(row)) = db::media_by_hash(hash) {
            return Some(row.item.path);
        }
    }
    link::stream_for_key(key)
}

pub fn img_path(image: &str) -> Option<PathBuf> {
    let name = image.strip_prefix("/img/")?;
    server::safe_file_name(name)
        .then(|| db::img_dir().map(|d| d.join(name)))
        .flatten()
}

#[tauri::command]
pub async fn video_notes(key: String) -> Result<Vec<Note>, String> {
    blocking(move || notes_of(&key)).await
}

#[tauri::command]
pub async fn video_note_add(
    key: String,
    t_ms: i64,
    t_end_ms: Option<i64>,
    text: String,
    capture: bool,
    media_id: Option<i64>,
) -> Result<Note, String> {
    let t_ms = t_ms.max(0);
    let mut image = None;
    if capture {
        let k = key.clone();
        let src = blocking(move || Ok(media_source(&k, media_id))).await?;
        if let (Some(src), Some(dir)) = (src, db::img_dir()) {
            let name = format!("{}.jpg", uuid::Uuid::new_v4().simple());
            let out = dir.join(&name);
            match server::capture_frame(&src, t_ms, &out).await {
                Ok(()) => image = Some(format!("/img/{name}")),
                Err(e) => tracing::warn!("[video] note capture failed: {e}"),
            }
        }
    }
    blocking(move || {
        let now = db::now_ms();
        let id: i64 = db::with(|c| {
            c.query_row(
                "INSERT INTO notes (key, t_ms, t_end_ms, text, image, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) RETURNING id",
                params![
                    key,
                    t_ms,
                    t_end_ms.filter(|e| *e > t_ms),
                    text.trim(),
                    image,
                    now
                ],
                |r| r.get(0),
            )
        })?;
        note_by_id(id)
    })
    .await
}

#[tauri::command]
pub async fn video_note_update(id: i64, text: String) -> Result<Note, String> {
    blocking(move || {
        db::with(|c| {
            c.execute(
                "UPDATE notes SET text = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, text.trim(), db::now_ms()],
            )
        })?;
        note_by_id(id)
    })
    .await
}

#[tauri::command]
pub async fn video_note_delete(id: i64) -> Result<(), String> {
    blocking(move || {
        let note = note_by_id(id)?;
        db::with(|c| c.execute("DELETE FROM notes WHERE id = ?1", [id]))?;
        if let Some(img) = note.image {
            let used: i64 = db::with(|c| {
                c.query_row("SELECT COUNT(*) FROM cards WHERE image = ?1", [&img], |r| {
                    r.get(0)
                })
            })?;
            if used == 0 {
                if let Some(p) = img_path(&img) {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        Ok(())
    })
    .await
}

fn export_root() -> Result<PathBuf, String> {
    let base = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .ok_or("no Downloads folder")?;
    let dir = base.join("OmniGet Notes");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let t = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.trim_matches('.').chars().take(120).collect();
    if t.is_empty() {
        "Notes".into()
    } else {
        t
    }
}

fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    (2..1000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(path)
}

fn doc_for(key: &str, notes: &[Note], out_dir: &Path) -> Result<(String, String), String> {
    let (title, subtitle, _) = labels_for_key(key);
    let mut doc = notes_md::NoteDoc {
        title: title.clone(),
        link_base: format!("omniget://watch?key={}", urlencoding::encode(key)),
        ..Default::default()
    };
    if let Some(hash) = key.strip_prefix("local:") {
        if let Ok(Some(row)) = db::media_by_hash(hash) {
            doc.source = row.item.path.clone();
            doc.duration_ms = (row.item.duration_ms > 0).then_some(row.item.duration_ms as u64);
            if row.item.kind == MediaKind::Lesson {
                if let Some(cid) = row.item.course_id {
                    doc.course = db::with(|c| {
                        c.query_row("SELECT title FROM courses WHERE id = ?1", [cid], |r| {
                            r.get(0)
                        })
                        .optional()
                    })?;
                }
                doc.module = row.item.module.clone();
                doc.lesson = row.number.clone().or(Some(row.item.title.clone()));
            }
        }
    } else if let Some(url) = key.strip_prefix("url:") {
        doc.source = url.to_string();
    } else {
        doc.source = subtitle;
    }
    let cues = study::cached_cues(key).unwrap_or_default();
    let assets = out_dir.join("assets");
    for n in notes {
        let mut image_rel = None;
        if let Some(src) = n.image.as_deref().and_then(img_path) {
            if std::fs::create_dir_all(&assets).is_ok() {
                if let Some(name) = src.file_name() {
                    if std::fs::copy(&src, assets.join(name)).is_ok() {
                        image_rel = Some(format!("assets/{}", name.to_string_lossy()));
                    }
                }
            }
        }
        let quote = cues
            .iter()
            .find(|c| c.start_ms <= n.t_ms && n.t_ms < c.end_ms.max(c.start_ms + 1))
            .map(|c| c.text.clone())
            .filter(|q| !q.trim().is_empty() && q.trim() != n.text.trim());
        doc.notes.push(notes_md::NoteLine {
            t_ms: n.t_ms.max(0) as u64,
            t_end_ms: n.t_end_ms.map(|e| e.max(0) as u64),
            text: n.text.clone(),
            image_rel,
            quote,
        });
    }
    Ok((title, notes_md::export(&doc)))
}

fn export_key(key: &str) -> Result<String, String> {
    let notes = notes_of(key)?;
    if notes.is_empty() {
        return Err("No notes to export".into());
    }
    let dir = export_root()?;
    let (title, md) = doc_for(key, &notes, &dir)?;
    let path = unique(dir.join(format!("{}.md", safe_name(&title))));
    std::fs::write(&path, md).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

fn export_course(course_id: i64) -> Result<String, String> {
    let title: String = db::with(|c| {
        c.query_row(
            "SELECT title FROM courses WHERE id = ?1",
            [course_id],
            |r| r.get(0),
        )
        .optional()
    })?
    .ok_or("course not found")?;
    let mut rows = db::media_where("WHERE m.course_id = ?1", &[&course_id])?;
    super::sort_course(&mut rows);
    let dir = unique(export_root()?.join(safe_name(&title)));
    let mut written = 0;
    for (i, r) in rows.iter().enumerate() {
        let notes = notes_of(&r.item.key)?;
        if notes.is_empty() {
            continue;
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (_, md) = doc_for(&r.item.key, &notes, &dir)?;
        let num = r.number.clone().unwrap_or_else(|| format!("{:02}", i + 1));
        let file = dir.join(format!(
            "{} - {}.md",
            safe_name(&num),
            safe_name(&r.item.title)
        ));
        std::fs::write(unique(file), md).map_err(|e| e.to_string())?;
        written += 1;
    }
    if written == 0 {
        return Err("No notes to export".into());
    }
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn video_notes_export(
    key: Option<String>,
    course_id: Option<i64>,
) -> Result<String, String> {
    blocking(move || match (key.filter(|k| !k.is_empty()), course_id) {
        (Some(k), _) => export_key(&k),
        (None, Some(c)) => export_course(c),
        (None, None) => Err("Nothing to export".into()),
    })
    .await
}
