use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use rusqlite::params;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use omniget_core::core::video::{course, naming};

use super::{db, probe, RootKind, ScanProgress};

static SCANNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn is_scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

fn scan_lock() -> &'static tokio::sync::Mutex<()> {
    static L: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

const SKIP_DIRS: &[&str] = &[
    "@eaDir",
    "$RECYCLE.BIN",
    "System Volume Information",
    "lost+found",
    "node_modules",
];

pub fn content_hash(path: &Path) -> std::io::Result<String> {
    const CHUNK: u64 = 64 * 1024;
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut h = Sha256::new();
    h.update(len.to_le_bytes());
    let mut buf = vec![0u8; CHUNK as usize];
    let n = read_full(&mut f, &mut buf)?;
    h.update(&buf[..n]);
    if len > CHUNK {
        f.seek(SeekFrom::Start(len.saturating_sub(CHUNK).max(CHUNK)))?;
        let n = read_full(&mut f, &mut buf)?;
        h.update(&buf[..n]);
    }
    Ok(hex::encode(&h.finalize()[..8]))
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

struct Walk {
    videos: Vec<(PathBuf, i64, i64)>,
    others: Vec<PathBuf>,
    unreadable: Vec<PathBuf>,
}

fn walk(root: &Path, want_others: bool, nested: &[PathBuf]) -> Walk {
    let mut w = Walk {
        videos: Vec::new(),
        others: Vec::new(),
        unreadable: Vec::new(),
    };
    let it = walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || (!hidden(&e.file_name().to_string_lossy())
                    && !nested.iter().any(|n| e.path() == n.as_path()))
        });
    for e in it {
        match e {
            Ok(e) if e.file_type().is_file() => {
                let p = e.path();
                if probe::is_video_path(p) {
                    if let Ok(m) = e.metadata() {
                        w.videos
                            .push((p.to_path_buf(), mtime_ms(&m), m.len() as i64));
                    }
                } else if want_others {
                    w.others.push(p.to_path_buf());
                }
            }
            Ok(_) => {}
            Err(err) => {
                if let Some(p) = err.path() {
                    w.unreadable.push(p.to_path_buf());
                }
            }
        }
    }
    w
}

fn rel_of(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

pub async fn index_one(
    root: &str,
    kind: RootKind,
    path: PathBuf,
    mtime: i64,
    size: i64,
) -> Result<i64, String> {
    let p2 = path.clone();
    let hash = tokio::task::spawn_blocking(move || content_hash(&p2))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let pr = probe::probe(&path).await.ok();
    let rel = rel_of(Path::new(root), &path);
    let parsed = naming::parse(&rel);
    let mut media_kind = parsed.kind.as_str().to_string();
    if kind == RootKind::Course {
        media_kind = "lesson".into();
    } else if parsed.is_extra {
        media_kind = "video".into();
    }
    let ext_subs = !probe::external_subs(&path).is_empty();
    let has_subs = ext_subs
        || pr
            .as_ref()
            .map(|d| d.text_subs().next().is_some())
            .unwrap_or(false);
    let folder = path
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    let spath = path.to_string_lossy().to_string();
    let title = if parsed.title.trim().is_empty() {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        parsed.title.clone()
    };
    let d = pr.unwrap_or_default();
    let probed = !d.format.is_empty();
    let streams = serde_json::to_string(&d.streams).unwrap_or_else(|_| "[]".into());
    let chapters = serde_json::to_string(&d.chapters).unwrap_or_else(|_| "[]".into());
    let probe_json = serde_json::to_string(&d).unwrap_or_default();
    let lesson_no = parsed
        .lesson_no
        .as_ref()
        .and_then(|v| v.last())
        .map(|n| *n as i64);
    let root = root.to_string();
    db::with(|c| {
        c.query_row(
            "INSERT INTO media (path, root, folder, rel, size, mtime_ms, hash, kind, title, show,
                season, episode, episode_end, year, lesson_no, module, duration_ms, width, height,
                vcodec, acodec, container, bitrate, streams_json, chapters_json, probe_json, probed,
                has_subs, added_at, offline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, 0)
             ON CONFLICT(path) DO UPDATE SET root = excluded.root, folder = excluded.folder,
                rel = excluded.rel, size = excluded.size, mtime_ms = excluded.mtime_ms,
                hash = excluded.hash, kind = excluded.kind, title = excluded.title,
                show = excluded.show, season = excluded.season, episode = excluded.episode,
                episode_end = excluded.episode_end, year = excluded.year,
                lesson_no = excluded.lesson_no, module = excluded.module,
                duration_ms = excluded.duration_ms, width = excluded.width,
                height = excluded.height, vcodec = excluded.vcodec, acodec = excluded.acodec,
                container = excluded.container, bitrate = excluded.bitrate,
                streams_json = excluded.streams_json, chapters_json = excluded.chapters_json,
                probe_json = excluded.probe_json, probed = excluded.probed,
                has_subs = excluded.has_subs, offline = 0
             RETURNING id",
            params![
                spath,
                root,
                folder,
                rel,
                size,
                mtime,
                hash,
                media_kind,
                title,
                parsed.show,
                parsed.season.map(|v| v as i64),
                parsed.episode.map(|v| v as i64),
                parsed.episode_end.map(|v| v as i64),
                parsed.year.map(|v| v as i64),
                lesson_no,
                parsed.module,
                d.duration_ms,
                d.width,
                d.height,
                d.vcodec,
                d.acodec,
                d.format,
                d.bitrate,
                streams,
                chapters,
                probe_json,
                probed as i64,
                has_subs as i64,
                db::now_ms(),
            ],
            |r| r.get(0),
        )
    })
}

pub async fn index_path(path: &Path) -> Result<i64, String> {
    let roots = db::roots()?;
    let root = roots
        .iter()
        .filter(|r| path.starts_with(&r.path))
        .max_by_key(|r| r.path.len())
        .ok_or("not inside a video folder")?;
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    index_one(
        &root.path,
        root.kind,
        path.to_path_buf(),
        mtime_ms(&meta),
        meta.len() as i64,
    )
    .await
}

pub async fn index_loose(path: &Path) -> Result<i64, String> {
    if !path.is_file() {
        return Err("File not found".into());
    }
    if !probe::is_video_path(path) {
        return Err("Not a video file".into());
    }
    if let Ok(id) = index_path(path).await {
        return Ok(id);
    }
    let folder = path
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let id = index_one(
        &folder,
        RootKind::Library,
        path.to_path_buf(),
        mtime_ms(&meta),
        meta.len() as i64,
    )
    .await?;
    db::with(|c| {
        c.execute(
            "UPDATE media SET kind = 'video', show = NULL, season = NULL, episode = NULL,
                episode_end = NULL, course_id = NULL WHERE id = ?1",
            [id],
        )
    })?;
    Ok(id)
}

fn emit(app: &AppHandle, done: usize, total: usize, current: &str, finished: bool) {
    let _ = app.emit(
        "video-scan-progress",
        ScanProgress {
            done,
            total,
            current: current.to_string(),
            finished,
        },
    );
}

pub async fn scan(app: AppHandle, only: Option<String>) -> Result<(), String> {
    let _guard = scan_lock().lock().await;
    SCANNING.store(true, Ordering::SeqCst);
    CANCEL.store(false, Ordering::SeqCst);
    let res = scan_inner(&app, only).await;
    SCANNING.store(false, Ordering::SeqCst);
    let _ = app.emit("video-library-changed", ());
    res
}

async fn scan_inner(app: &AppHandle, only: Option<String>) -> Result<(), String> {
    let all_roots = db::roots()?;
    let roots: Vec<_> = all_roots
        .iter()
        .filter(|r| only.as_ref().is_none_or(|o| *o == r.path))
        .cloned()
        .collect();
    let mut plan: Vec<(String, RootKind, PathBuf, i64, i64)> = Vec::new();
    let mut course_roots: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for root in &roots {
        if !root.online {
            db::with(|c| c.execute("UPDATE media SET offline = 1 WHERE root = ?1", [&root.path]))?;
            continue;
        }
        let rp = PathBuf::from(&root.path);
        let want_others = root.kind == RootKind::Course;
        let nested: Vec<PathBuf> = all_roots
            .iter()
            .filter(|o| o.path != root.path && Path::new(&o.path).starts_with(&root.path))
            .map(|o| PathBuf::from(&o.path))
            .collect();
        let w = tokio::task::spawn_blocking(move || walk(&rp, want_others, &nested))
            .await
            .map_err(|e| e.to_string())?;
        let existing: HashMap<String, (i64, i64, i64, bool)> = db::with(|c| {
            let mut st =
                c.prepare("SELECT path, id, mtime_ms, size, probed FROM media WHERE root = ?1")?;
            let rows = st.query_map([&root.path], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0),
                ))
            })?;
            rows.collect()
        })?;
        let mut seen = HashSet::new();
        let mut unchanged = Vec::new();
        for (p, mtime, size) in &w.videos {
            let sp = p.to_string_lossy().to_string();
            seen.insert(sp.clone());
            match existing.get(&sp) {
                Some((id, m, s, probed)) if m == mtime && s == size && *probed => {
                    unchanged.push(*id)
                }
                _ => plan.push((root.path.clone(), root.kind, p.clone(), *mtime, *size)),
            }
        }
        let mut drop_ids = Vec::new();
        let mut offline_ids = Vec::new();
        for (path, (id, ..)) in &existing {
            if seen.contains(path) {
                continue;
            }
            let p = Path::new(path);
            let under_unreadable = w.unreadable.iter().any(|u| p.starts_with(u));
            let gone =
                matches!(std::fs::metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
            let parent_readable = p
                .parent()
                .map(|d| !d.exists() || std::fs::read_dir(d).is_ok())
                .unwrap_or(false);
            if gone && parent_readable && !under_unreadable {
                drop_ids.push(*id);
            } else {
                offline_ids.push(*id);
            }
        }
        db::with_mut(|c| {
            let tx = c.transaction()?;
            {
                let mut on = tx.prepare("UPDATE media SET offline = 0 WHERE id = ?1")?;
                for id in &unchanged {
                    on.execute([id])?;
                }
                let mut off = tx.prepare("UPDATE media SET offline = 1 WHERE id = ?1")?;
                for id in &offline_ids {
                    off.execute([id])?;
                }
                let mut del = tx.prepare("DELETE FROM media WHERE id = ?1")?;
                for id in &drop_ids {
                    del.execute([id])?;
                }
            }
            tx.commit()
        })?;
        if root.kind == RootKind::Course {
            let mut files: Vec<PathBuf> = w.videos.iter().map(|v| v.0.clone()).collect();
            files.extend(w.others);
            course_roots.push((root.path.clone(), files));
        }
    }

    let total = plan.len();
    emit(app, 0, total, "", total == 0);
    let mut done = 0usize;
    let mut stream = futures::stream::iter(plan.into_iter().map(
        |(root, kind, path, mtime, size)| async move {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let r = index_one(&root, kind, path, mtime, size).await;
            (name, r)
        },
    ))
    .buffer_unordered(4);
    while let Some((name, r)) = stream.next().await {
        done += 1;
        if let Err(e) = r {
            tracing::warn!("[video] index failed for {name}: {e}");
        }
        if done.is_multiple_of(5) || done == total {
            emit(app, done, total, &name, false);
        }
        if CANCEL.load(Ordering::SeqCst) {
            break;
        }
    }
    drop(stream);

    for (root, files) in course_roots {
        if let Err(e) = apply_course(&root, &files) {
            tracing::warn!("[video] course build failed for {root}: {e}");
        }
    }
    emit(app, done, total, "", true);
    Ok(())
}

pub fn attachment_kind(path: &str) -> String {
    serde_json::to_value(course::attachment_kind(path))
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "other".into())
}

pub fn course_title(root: &str) -> String {
    let name = Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string());
    let t = naming::clean_title(&name);
    if t.trim().is_empty() {
        name
    } else {
        t
    }
}

pub fn ensure_course(root: &str) -> Result<i64, String> {
    db::with(|c| {
        c.execute(
            "INSERT OR IGNORE INTO courses (root_path, title, created_at) VALUES (?1, ?2, ?3)",
            params![root, course_title(root), db::now_ms()],
        )?;
        c.query_row("SELECT id FROM courses WHERE root_path = ?1", [root], |r| {
            r.get(0)
        })
    })
}

fn apply_course(root: &str, files: &[PathBuf]) -> Result<(), String> {
    let course_id = ensure_course(root)?;
    let rp = Path::new(root);
    let inputs: Vec<course::CourseFile> = files
        .iter()
        .map(|p| course::CourseFile {
            path: rel_of(rp, p),
        })
        .collect();
    let root_name = rp
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tree = course::build_titled(&root_name, &inputs);
    let mut placement: HashMap<String, (i64, String, String, i64, Option<i64>)> = HashMap::new();
    let mut ord = 0i64;
    for m in &tree.modules {
        for l in &m.lessons {
            ord += 1;
            let last_no = l
                .number
                .rsplit(['.', '-'])
                .next()
                .and_then(|n| n.trim().parse::<i64>().ok());
            placement.insert(
                l.path.clone(),
                (
                    m.index as i64,
                    m.title.clone(),
                    l.number.clone(),
                    ord,
                    last_no,
                ),
            );
        }
    }
    let atts: Vec<(String, Option<String>)> = tree
        .attachments
        .iter()
        .map(|a| (a.path.clone(), a.lesson_path.clone()))
        .collect();
    let atts_json = serde_json::to_string(&atts).unwrap_or_else(|_| "[]".into());
    let titles: HashMap<String, String> = tree
        .modules
        .iter()
        .flat_map(|m| m.lessons.iter().map(|l| (l.path.clone(), l.title.clone())))
        .collect();
    db::with_mut(|c| {
        let tx = c.transaction()?;
        tx.execute(
            "UPDATE courses SET attachments_json = ?2,
                title = CASE WHEN ?3 = '' THEN title ELSE ?3 END WHERE id = ?1",
            params![course_id, atts_json, tree.title],
        )?;
        {
            let mut st = tx.prepare(
                "UPDATE media SET course_id = ?2, kind = 'lesson', module_index = ?3, module = ?4,
                    number = ?5, ord = ?6, lesson_no = COALESCE(?7, lesson_no),
                    title = CASE WHEN ?8 = '' THEN title ELSE ?8 END
                 WHERE root = ?1 AND rel = ?9",
            )?;
            for (rel, (mi, mt, num, ord, no)) in &placement {
                let title = titles.get(rel).cloned().unwrap_or_default();
                st.execute(params![root, course_id, mi, mt, num, ord, no, title, rel])?;
            }
        }
        tx.commit()
    })
}
