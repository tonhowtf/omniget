use super::model::{CanvasNode, NodeKind, NoteData};
use super::ops::Op;
use super::store::{CanvasStore, NoteEvent, StoreEvent, WatchEntry, ORIGIN_SYSTEM};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const WATCH_TTL: Duration = Duration::from_secs(10 * 60);
pub const MAX_NOTE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContent {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    pub path: String,
    pub display_name: String,
    pub text: String,
    pub revision: String,
    pub is_external: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_at: Option<String>,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    pub path: String,
    pub display_name: String,
    pub revision: String,
    pub is_external: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_at: Option<String>,
    pub preview: String,
}

pub fn revision(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(digest)[..16].to_string()
}

pub fn derive_name(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim();
    let name: String = line.chars().take(80).collect();
    if name.is_empty() {
        "Untitled".into()
    } else {
        name
    }
}

fn preview(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(6)
        .collect();
    let p = lines.join("\n");
    p.chars().take(400).collect()
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn rfc3339(t: Option<SystemTime>) -> Option<String> {
    t.map(|t| {
        chrono::DateTime::<chrono::Utc>::from(t)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

fn read_text(path: &Path) -> Result<String, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("CANVAS_IO: {}: {e}", path.display()))?;
    if meta.len() as usize > MAX_NOTE_BYTES {
        return Err(format!(
            "CANVAS_INVALID: note {} is larger than 8 MB",
            path.display()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("CANVAS_IO: {}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn note_path(
    store: &CanvasStore,
    ws: &str,
    node: &CanvasNode,
) -> Result<(PathBuf, NoteData), String> {
    if node.kind != NodeKind::Note {
        return Err(format!("CANVAS_INVALID: node {} is not a note", node.id));
    }
    let d: NoteData = node.data_as().unwrap_or_default();
    if let Some(p) = &d.path {
        return Ok((PathBuf::from(p), d));
    }
    let f = d
        .file_name
        .clone()
        .ok_or_else(|| format!("CANVAS_INVALID: note {} has no file", node.id))?;
    Ok((store.notes_dir(ws).join(f), d))
}

fn node(store: &CanvasStore, ws: &str, floor: &str, node_id: &str) -> Result<CanvasNode, String> {
    store
        .read(ws, floor, |d| d.node(node_id).cloned())?
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id}"))
}

fn content(node: &CanvasNode, path: &Path, d: &NoteData, text: String) -> NoteContent {
    let display_name = if d.has_custom_name {
        node.title.clone().unwrap_or_else(|| derive_name(&text))
    } else {
        derive_name(&text)
    };
    NoteContent {
        node_id: node.id.clone(),
        file_name: d.file_name.clone(),
        path: path.to_string_lossy().into_owned(),
        display_name,
        revision: revision(&text),
        is_external: d.path.is_some() || d.is_external,
        last_modified_at: rfc3339(modified(path)),
        preview: preview(&text),
        text,
    }
}

fn watch(store: &CanvasStore, ws: &str, node_id: &str, path: &Path, rev: &str) {
    store
        .note_watch
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            (ws.to_string(), node_id.to_string()),
            WatchEntry {
                path: path.to_path_buf(),
                modified: modified(path),
                revision: rev.to_string(),
                last_read: Instant::now(),
            },
        );
}

pub fn backup_path(store: &CanvasStore, path: &Path) -> PathBuf {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("note")
        .chars()
        .take(60)
        .collect::<String>();
    store
        .root()
        .join("note-backups")
        .join(format!("{}-{name}.bak", &hex::encode(digest)[..16]))
}

pub fn write_note_file(store: &CanvasStore, path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("path has no parent")?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("CANVAS_IO: create {}: {e}", dir.display()))?;
    if path.exists() {
        let bak = backup_path(store, path);
        if let Some(parent) = bak.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::copy(path, bak);
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("note");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| format!("CANVAS_IO: {}: {e}", tmp.display()))?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("CANVAS_IO: {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("CANVAS_IO: rename to {}: {e}", path.display())
    })
}

pub fn read(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    node_id: &str,
) -> Result<NoteContent, String> {
    let n = node(store, ws, floor, node_id)?;
    let (path, d) = note_path(store, ws, &n)?;
    let text = match read_text(&path) {
        Ok(t) => t,
        Err(_) if !path.exists() && d.path.is_none() => String::new(),
        Err(e) => return Err(e),
    };
    let c = content(&n, &path, &d, text);
    watch(store, ws, node_id, &path, &c.revision);
    Ok(c)
}

pub fn write(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    node_id: &str,
    text: &str,
    if_revision: Option<&str>,
    origin: &str,
) -> Result<NoteContent, String> {
    if text.len() > MAX_NOTE_BYTES {
        return Err("CANVAS_INVALID: note is larger than 8 MB".into());
    }
    let n = node(store, ws, floor, node_id)?;
    let (path, d) = note_path(store, ws, &n)?;
    if d.is_content_locked {
        return Err(format!("CANVAS_NOTE_LOCKED: {node_id}"));
    }
    if let Some(expected) = if_revision {
        let current = if path.exists() {
            revision(&read_text(&path)?)
        } else {
            revision("")
        };
        if current != expected {
            return Err(format!("CANVAS_NOTE_CONFLICT: {current}"));
        }
    }
    write_note_file(store, &path, text.as_bytes())?;
    let c = content(&n, &path, &d, text.to_string());
    watch(store, ws, node_id, &path, &c.revision);

    if !d.has_custom_name && n.title.as_deref() != Some(c.display_name.as_str()) {
        let op = Op::update_node(node_id, serde_json::json!({ "title": c.display_name }));
        if let Err(e) = store.apply(ws, floor, vec![op], origin, None) {
            tracing::warn!("[canvas] note title update failed: {e}");
        }
    }
    store.emit(StoreEvent::Note(NoteEvent {
        workspace_id: ws.to_string(),
        node_id: node_id.to_string(),
        revision: c.revision.clone(),
        origin: origin.to_string(),
    }));
    Ok(c)
}

pub fn list(store: &CanvasStore, ws: &str) -> Result<Vec<NoteSummary>, String> {
    let meta = store.workspace(ws)?;
    let mut owned: Vec<(String, CanvasNode)> = Vec::new();
    for f in &meta.floors {
        let notes = store.read(ws, &f.id, |d| {
            d.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Note)
                .cloned()
                .collect::<Vec<_>>()
        })?;
        owned.extend(notes.into_iter().map(|n| (f.id.clone(), n)));
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (floor, n) in &owned {
        let Ok((path, d)) = note_path(store, ws, n) else {
            continue;
        };
        let text = read_text(&path).unwrap_or_default();
        let c = content(n, &path, &d, text);
        seen.insert(path.clone());
        out.push(NoteSummary {
            node_id: Some(n.id.clone()),
            floor_id: Some(floor.clone()),
            file_name: c.file_name,
            path: c.path,
            display_name: c.display_name,
            revision: c.revision,
            is_external: c.is_external,
            last_modified_at: c.last_modified_at,
            preview: c.preview,
        });
    }
    if let Ok(rd) = std::fs::read_dir(store.notes_dir(ws)) {
        for e in rd.flatten() {
            let p = e.path();
            if seen.contains(&p) || p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            let text = read_text(&p).unwrap_or_default();
            out.push(NoteSummary {
                node_id: None,
                floor_id: None,
                file_name: p.file_name().and_then(|n| n.to_str()).map(str::to_string),
                path: p.to_string_lossy().into_owned(),
                display_name: derive_name(&text),
                revision: revision(&text),
                is_external: false,
                last_modified_at: rfc3339(modified(&p)),
                preview: preview(&text),
            });
        }
    }
    out.sort_by(|a, b| b.last_modified_at.cmp(&a.last_modified_at));
    Ok(out)
}

pub fn create(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    x: f64,
    y: f64,
    text: &str,
    origin: &str,
) -> Result<String, String> {
    let id = super::model::new_id();
    let size = super::model::default_size(&NodeKind::Note);
    let z = store.read(ws, floor, |d| d.max_z())? + 1.0;
    let node = CanvasNode {
        id: id.clone(),
        kind: NodeKind::Note,
        x,
        y,
        w: size.w,
        h: size.h,
        z,
        title: Some(derive_name(text)),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: serde_json::json!({}),
    };
    store.apply(ws, floor, vec![Op::AddNode { node }], origin, None)?;
    if !text.is_empty() {
        write(store, ws, floor, &id, text, None, origin)?;
    }
    Ok(id)
}

pub fn resolve_note_target(
    store: &CanvasStore,
    ws: &str,
    raw: &str,
    name_hint: &str,
) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("CANVAS_INVALID: empty path".into());
    }
    let mut p = if let Some(rest) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or("CANVAS_IO: no home folder")?
            .join(rest)
    } else {
        PathBuf::from(raw)
    };
    if p.is_relative() {
        let base = store
            .workspace(ws)
            .ok()
            .and_then(|w| w.working_directory)
            .filter(|d| !d.trim().is_empty())
            .ok_or("CANVAS_INVALID: relative path and the workspace has no folder")?;
        p = PathBuf::from(base).join(p);
    }
    if p.is_dir() || raw.ends_with('/') {
        let slug = super::store::note_file_name(name_hint, "")
            .trim_end_matches(".md")
            .trim_end_matches('-')
            .to_string();
        let slug = if slug.is_empty() { "note".into() } else { slug };
        p = p.join(format!("{slug}.md"));
    }
    Ok(p)
}

#[allow(clippy::too_many_arguments)]
pub fn create_external(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    x: f64,
    y: f64,
    path: &Path,
    text: &str,
    origin: &str,
) -> Result<String, String> {
    if !path.exists() {
        write_note_file(store, path, text.as_bytes())?;
    } else if path.is_dir() {
        return Err(format!("CANVAS_INVALID: {} is a folder", path.display()));
    }
    let body = read_text(path)?;
    let title = if body.trim().is_empty() {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string()
    } else {
        derive_name(&body)
    };
    let id = super::model::new_id();
    let size = super::model::default_size(&NodeKind::Note);
    let z = store.read(ws, floor, |d| d.max_z())? + 1.0;
    let node = CanvasNode {
        id: id.clone(),
        kind: NodeKind::Note,
        x,
        y,
        w: size.w,
        h: size.h,
        z,
        title: Some(title),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: serde_json::json!({
            "path": path.to_string_lossy(),
            "isExternal": true,
        }),
    };
    store.apply(ws, floor, vec![Op::AddNode { node }], origin, None)?;
    Ok(id)
}

pub fn move_to(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    node_id: &str,
    dest: &str,
    origin: &str,
) -> Result<NoteContent, String> {
    let n = node(store, ws, floor, node_id)?;
    let (old, d) = note_path(store, ws, &n)?;
    let text = match read_text(&old) {
        Ok(t) => t,
        Err(_) if !old.exists() => String::new(),
        Err(e) => return Err(e),
    };
    let hint = n.title.clone().unwrap_or_else(|| derive_name(&text));
    let target = resolve_note_target(store, ws, dest, &hint)?;
    if target == old {
        return read(store, ws, floor, node_id);
    }
    if target.exists() {
        return Err(format!(
            "CANVAS_CONFLICT: {} already exists",
            target.display()
        ));
    }
    write_note_file(store, &target, text.as_bytes())?;
    let op = Op::update_node(
        node_id,
        serde_json::json!({
            "data": {
                "path": target.to_string_lossy(),
                "isExternal": true,
                "fileName": null,
            }
        }),
    );
    store.apply(ws, floor, vec![op], origin, None)?;
    if d.path.is_none() && old.exists() {
        if let Err(e) = super::persist::move_to_trash(store.root(), ws, &old) {
            tracing::warn!("[canvas] note move: {e}");
        }
    }
    store
        .note_watch
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(ws.to_string(), node_id.to_string()));
    read(store, ws, floor, node_id)
}

pub fn poll_changes(store: &CanvasStore) -> Vec<NoteEvent> {
    let now = Instant::now();
    let mut changed = Vec::new();
    {
        let mut w = store.note_watch.lock().unwrap_or_else(|e| e.into_inner());
        w.retain(|_, e| now.duration_since(e.last_read) < WATCH_TTL);
        for ((ws, node_id), e) in w.iter_mut() {
            let m = modified(&e.path);
            if m == e.modified {
                continue;
            }
            e.modified = m;
            let Ok(text) = read_text(&e.path) else {
                continue;
            };
            let rev = revision(&text);
            if rev != e.revision {
                e.revision = rev.clone();
                changed.push(NoteEvent {
                    workspace_id: ws.clone(),
                    node_id: node_id.clone(),
                    revision: rev,
                    origin: ORIGIN_SYSTEM.into(),
                });
            }
        }
    }
    for ev in &changed {
        store.emit(StoreEvent::Note(ev.clone()));
    }
    changed
}
