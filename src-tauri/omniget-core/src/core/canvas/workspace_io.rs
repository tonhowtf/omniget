//! Whole-workspace archive (one `.omniget` file) and rotating backups.
//!
//! Unlike partituras ("blueprints, not backups", `export.rs`), both keep the
//! workspace as it is: every floor document, note texts, workspace roles, chat
//! transcripts and attachments, floor hooks, the workspace environment and its
//! routines. What stays out: isolated floor working trees (`floor-trees/`), the
//! trash, `.bak` files, and by default anything that looks like a secret.
//!
//! Export writes a JSON file (`format: "omniget.workspace"`) with a file map
//! relative to the workspace folder. Import first previews it, then (with
//! confirmation) writes a NEW workspace with every node, cord, drawing, group,
//! tie, terminal and routine id remapped, so the same file imports twice
//! without clashing.
//!
//! Backups copy `<canvas>/<ws>/` into `<app_data>/canvas-backups/<ws>/<id>/`
//! with a manifest and a content hash; a backup identical to the latest one is
//! not written again. They live outside the workspace folder so a deleted
//! workspace can come back.

use super::model::{new_id, now_rfc3339, FloorDoc, Workspace, GROUND_FLOOR_ID};
use super::roles::{self, Role};
use super::routines::{self, Routine};
use super::store::CanvasStore;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const ARCHIVE_FORMAT: &str = "omniget.workspace";
pub const ARCHIVE_VERSION: u32 = 1;
pub const ARCHIVE_EXTENSION: &str = "omniget";

/// Placeholder for the exporting machine's workspace folder inside strings.
const WS_DIR_TOKEN: &str = "${OMNIGET_WS_DIR}";
/// Folders of a workspace that never travel (working trees, trash).
const SKIP_DIRS: &[&str] = &["floor-trees", "trash"];
/// One file larger than this is left out of an archive (reported).
const MAX_FILE: u64 = 8 * 1024 * 1024;
/// Whole archive budget.
const MAX_TOTAL: u64 = 256 * 1024 * 1024;
/// Archives larger than this are refused on import.
const MAX_ARCHIVE: u64 = 600 * 1024 * 1024;

const KEEP_RECENT: usize = 24;
const KEEP_DAILY: usize = 7;
const KEEP_OTHER: usize = 20;

// ---------------------------------------------------------------- archive types

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Omitted {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Archive {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub workspace: Workspace,
    /// Path relative to the workspace folder (always `/`).
    pub files: BTreeMap<String, ArchiveFile>,
    #[serde(default)]
    pub routines: Vec<Routine>,
    /// Global-library roles the terminals use (imported into the workspace).
    #[serde(default)]
    pub global_roles: Vec<Role>,
    #[serde(default)]
    pub omitted: Vec<Omitted>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    #[serde(default)]
    pub include_secrets: bool,
    #[serde(default = "yes")]
    pub include_chat: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub bytes: u64,
    pub files: usize,
    pub floors: usize,
    pub nodes: usize,
    pub routines: usize,
    pub omitted: Vec<Omitted>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewNode {
    pub floor_id: String,
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFloor {
    pub id: String,
    pub name: String,
    pub nodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub name: String,
    pub icon: Option<String>,
    pub working_directory: Option<String>,
    pub floors: Vec<PreviewFloor>,
    pub nodes: Vec<PreviewNode>,
    pub cords: usize,
    pub groups: usize,
    pub drawings: usize,
    pub notes: usize,
    pub chats: usize,
    pub roles: Vec<String>,
    pub routines: usize,
    pub omitted: Vec<Omitted>,
    /// Things that will be adjusted on import.
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Working directory of the new workspace (default: the original one when
    /// it exists on this machine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub workspace_id: String,
    pub name: String,
    pub node_ids: Vec<String>,
    pub floors: usize,
    pub routines: usize,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------- helpers

fn io(e: impl std::fmt::Display) -> String {
    format!("CANVAS_IO: {e}")
}

fn rel_of(base: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(base).ok()?;
    let parts: Vec<String> = r
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Every file of a workspace folder that travels (sorted, relative paths).
fn walk(ws_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![ws_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if dir == ws_dir && SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(p);
            } else if ft.is_file() {
                if name.ends_with(".bak") || name.ends_with(".tmp") || name.starts_with(".tmp") {
                    continue;
                }
                if let Some(r) = rel_of(ws_dir, &p) {
                    out.push((r, p));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && rel
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains(':'))
}

/// Env var names that look like credentials.
pub fn is_secret_name(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.starts_with("OMNIGET_")
        || [
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "API_KEY",
            "APIKEY",
            "_KEY",
            "AUTH",
            "COOKIE",
            "CREDENTIAL",
            "SESSION",
            "PRIVATE",
        ]
        .iter()
        .any(|w| n.contains(w))
        || n == "KEY"
}

/// Drops secret-looking `env` entries of terminal nodes in a floor document
/// (as JSON). Returns the removed `node title: VAR` labels.
pub fn strip_floor_secrets(doc: &mut Value) -> Vec<String> {
    let mut removed = Vec::new();
    let Some(nodes) = doc.get_mut("nodes").and_then(Value::as_array_mut) else {
        return removed;
    };
    for n in nodes {
        let title = n
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| n.get("id").and_then(Value::as_str))
            .unwrap_or("?")
            .to_string();
        let Some(env) = n
            .get_mut("data")
            .and_then(|d| d.get_mut("env"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let keys: Vec<String> = env.keys().filter(|k| is_secret_name(k)).cloned().collect();
        for k in keys {
            env.remove(&k);
            removed.push(format!("{title}: {k}"));
        }
    }
    removed
}

/// Replaces ids inside JSON string values: an exact match always, a substring
/// only for long ids (UUIDs), so short ids like `home` never touch prose.
pub fn remap_value(v: &mut Value, map: &HashMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(n) = map.get(s.as_str()) {
                *s = n.clone();
            } else {
                for (old, new) in map {
                    if old.len() >= 16 && s.contains(old.as_str()) {
                        *s = s.replace(old.as_str(), new);
                    }
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| remap_value(x, map)),
        Value::Object(o) => {
            let entries: Vec<(String, Value)> = std::mem::take(o).into_iter().collect();
            for (k, mut x) in entries {
                remap_value(&mut x, map);
                let k = map.get(&k).cloned().unwrap_or(k);
                o.insert(k, x);
            }
        }
        _ => {}
    }
}

/// Remaps each path component equal to an id, or `<id>.<ext>` file names.
pub fn remap_rel(rel: &str, map: &HashMap<String, String>) -> String {
    rel.split('/')
        .map(|c| {
            if let Some(n) = map.get(c) {
                return n.clone();
            }
            if let Some((stem, ext)) = c.split_once('.') {
                if let Some(n) = map.get(stem) {
                    return format!("{n}.{ext}");
                }
            }
            c.to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn replace_in_strings(v: &mut Value, from: &str, to: &str) {
    match v {
        Value::String(s) if !from.is_empty() && s.contains(from) => *s = s.replace(from, to),
        Value::Array(a) => a.iter_mut().for_each(|x| replace_in_strings(x, from, to)),
        Value::Object(o) => o.values_mut().for_each(|x| replace_in_strings(x, from, to)),
        _ => {}
    }
}

/// Applies a string transform to a text file: JSON / JSONL values only, any
/// other text untouched (notes are prose).
fn transform_text(rel: &str, text: &str, f: &dyn Fn(&mut Value)) -> String {
    if rel.ends_with(".json") {
        if let Ok(mut v) = serde_json::from_str::<Value>(text) {
            f(&mut v);
            return serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_string());
        }
    } else if rel.ends_with(".jsonl") {
        let mut out = String::with_capacity(text.len());
        for line in text.lines() {
            match serde_json::from_str::<Value>(line) {
                Ok(mut v) => {
                    f(&mut v);
                    out.push_str(&serde_json::to_string(&v).unwrap_or_else(|_| line.into()));
                }
                Err(_) => out.push_str(line),
            }
            out.push('\n');
        }
        return out;
    }
    text.to_string()
}

fn floor_docs_of(files: &BTreeMap<String, ArchiveFile>) -> Vec<(String, FloorDoc)> {
    files
        .iter()
        .filter_map(|(rel, f)| {
            let id = rel.strip_prefix("floors/")?.strip_suffix(".json")?;
            if id.contains('/') {
                return None;
            }
            let doc: FloorDoc = serde_json::from_str(f.text.as_deref()?).ok()?;
            Some((id.to_string(), doc))
        })
        .collect()
}

// ---------------------------------------------------------------- export

pub fn build_archive(
    store: &CanvasStore,
    ws: &str,
    opts: &ExportOptions,
) -> Result<Archive, String> {
    let _ = store.flush_workspace(ws);
    let meta = store.workspace(ws)?;
    let ws_dir = store.workspace_dir(ws);
    let ws_dir_str = ws_dir.to_string_lossy().into_owned();
    let mut files = BTreeMap::new();
    let mut omitted = Vec::new();
    let mut total = 0u64;
    for (rel, path) in store.with_io(|| walk(&ws_dir)) {
        if rel == "workspace.json" {
            continue;
        }
        if !opts.include_chat && rel.starts_with("chat/") {
            omitted.push(Omitted {
                path: rel,
                reason: "chat history not included".into(),
            });
            continue;
        }
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE || total + size > MAX_TOTAL {
            omitted.push(Omitted {
                path: rel,
                reason: format!("too large ({size} bytes)"),
            });
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        total += size;
        let entry = match String::from_utf8(bytes) {
            Ok(text) => {
                let mut text = text;
                if rel.starts_with("floors/") && rel.ends_with(".json") && !opts.include_secrets {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&text) {
                        for r in strip_floor_secrets(&mut v) {
                            omitted.push(Omitted {
                                path: rel.clone(),
                                reason: format!("secret env {r}"),
                            });
                        }
                        text = serde_json::to_string_pretty(&v).unwrap_or(text);
                    }
                }
                let text = transform_text(&rel, &text, &|v| {
                    replace_in_strings(v, &ws_dir_str, WS_DIR_TOKEN)
                });
                ArchiveFile {
                    text: Some(text),
                    base64: None,
                }
            }
            Err(e) => ArchiveFile {
                text: None,
                base64: Some(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
            },
        };
        files.insert(rel, entry);
    }
    // Global roles the terminals use travel along.
    let mut global_roles = Vec::new();
    let mut seen = HashSet::new();
    for (_, doc) in floor_docs_of(&files) {
        for n in &doc.nodes {
            let Some(rid) = n.data.get("roleId").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(rid.to_string())
                || files.contains_key(&format!("roles/{rid}/role.json"))
            {
                continue;
            }
            if let Some((role, _)) = roles::get(store, ws, rid) {
                if role.workspace_id.is_none() {
                    global_roles.push(role);
                }
            }
        }
    }
    let has_portal = floor_docs_of(&files)
        .iter()
        .any(|(_, d)| d.nodes.iter().any(|n| n.kind.as_str() == "portal"));
    if has_portal {
        omitted.push(Omitted {
            path: "portals".into(),
            reason: "portal sessions (cookies, storage) are not exported".into(),
        });
    }
    if meta.floors.iter().any(|f| f.is_git_isolated) {
        omitted.push(Omitted {
            path: "floor-trees".into(),
            reason:
                "isolated floor working trees stay on this machine; they import as plain floors"
                    .into(),
        });
    }
    Ok(Archive {
        format: ARCHIVE_FORMAT.into(),
        version: ARCHIVE_VERSION,
        exported_at: now_rfc3339(),
        workspace: meta,
        files,
        routines: routines::list(store, Some(ws)),
        global_roles,
        omitted,
    })
}

pub fn export_to(
    store: &CanvasStore,
    ws: &str,
    path: &Path,
    opts: &ExportOptions,
) -> Result<ExportReport, String> {
    let a = build_archive(store, ws, opts)?;
    let bytes = serde_json::to_vec(&a).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(io)?;
        }
    }
    fs::write(path, &bytes).map_err(io)?;
    let floors = floor_docs_of(&a.files);
    Ok(ExportReport {
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
        files: a.files.len(),
        floors: floors.len(),
        nodes: floors.iter().map(|(_, d)| d.nodes.len()).sum(),
        routines: a.routines.len(),
        omitted: a.omitted,
    })
}

// ---------------------------------------------------------------- import

pub fn read_archive(path: &Path) -> Result<Archive, String> {
    let size = fs::metadata(path).map_err(io)?.len();
    if size > MAX_ARCHIVE {
        return Err(format!(
            "CANVAS_INVALID: archive is too large ({size} bytes)"
        ));
    }
    let bytes = fs::read(path).map_err(io)?;
    parse_archive(&bytes)
}

pub fn parse_archive(bytes: &[u8]) -> Result<Archive, String> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("CANVAS_INVALID: not an OmniGet workspace file: {e}"))?;
    let format = v.get("format").and_then(Value::as_str).unwrap_or("");
    if format != ARCHIVE_FORMAT {
        return Err(format!(
            "CANVAS_INVALID: unknown format '{format}' (expected {ARCHIVE_FORMAT})"
        ));
    }
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version as u32 > ARCHIVE_VERSION {
        return Err(format!(
            "CANVAS_INVALID: workspace file version {version} is newer than this OmniGet ({ARCHIVE_VERSION})"
        ));
    }
    let a: Archive =
        serde_json::from_value(v).map_err(|e| format!("CANVAS_INVALID: workspace file: {e}"))?;
    if let Some(bad) = a.files.keys().find(|r| !safe_rel(r)) {
        return Err(format!("CANVAS_INVALID: unsafe path in archive: {bad}"));
    }
    Ok(a)
}

fn unique_name(store: &CanvasStore, base: &str) -> String {
    let names: HashSet<String> = store
        .workspaces()
        .into_iter()
        .map(|s| s.workspace.name)
        .collect();
    if !names.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !names.contains(n))
        .expect("unbounded")
}

pub fn preview(store: &CanvasStore, a: &Archive) -> ImportPreview {
    let docs = floor_docs_of(&a.files);
    let mut nodes = Vec::new();
    let (mut cords, mut groups, mut drawings) = (0, 0, 0);
    for (fid, d) in &docs {
        cords += d.connections.len();
        groups += d.groups.len();
        drawings += d.drawings.len();
        for n in &d.nodes {
            nodes.push(PreviewNode {
                floor_id: fid.clone(),
                id: n.id.clone(),
                kind: n.kind.as_str().to_string(),
                title: n.title.clone(),
            });
        }
    }
    let floors = a
        .workspace
        .floors
        .iter()
        .map(|f| PreviewFloor {
            id: f.id.clone(),
            name: f.name.clone(),
            nodes: docs
                .iter()
                .find(|(id, _)| id == &f.id)
                .map(|(_, d)| d.nodes.len())
                .unwrap_or(0),
        })
        .collect();
    let mut roles: Vec<String> = a
        .files
        .iter()
        .filter(|(r, _)| r.starts_with("roles/") && r.ends_with("/role.json"))
        .filter_map(|(_, f)| {
            serde_json::from_str::<Value>(f.text.as_deref()?)
                .ok()?
                .get("name")?
                .as_str()
                .map(str::to_string)
        })
        .collect();
    roles.extend(a.global_roles.iter().map(|r| r.name.clone()));
    let mut conflicts = Vec::new();
    let name = unique_name(store, &a.workspace.name);
    if name != a.workspace.name {
        conflicts.push(format!(
            "a workspace named '{}' exists; the import will be '{name}'",
            a.workspace.name
        ));
    }
    if let Some(d) = &a.workspace.working_directory {
        if !Path::new(d).is_dir() {
            conflicts.push(format!(
                "working directory {d} does not exist here; pass dir to choose one"
            ));
        }
    }
    for f in a.workspace.floors.iter().filter(|f| f.is_git_isolated) {
        conflicts.push(format!(
            "floor '{}' was git-isolated; it imports as a plain floor",
            f.name
        ));
    }
    ImportPreview {
        format: a.format.clone(),
        version: a.version,
        exported_at: a.exported_at.clone(),
        name: a.workspace.name.clone(),
        icon: a.workspace.icon.clone(),
        working_directory: a.workspace.working_directory.clone(),
        floors,
        nodes,
        cords,
        groups,
        drawings,
        notes: a.files.keys().filter(|r| r.starts_with("notes/")).count(),
        chats: a
            .files
            .keys()
            .filter(|r| r.starts_with("chat/") && r.ends_with(".jsonl"))
            .count(),
        roles,
        routines: a.routines.len(),
        omitted: a.omitted.clone(),
        conflicts,
    }
}

/// Old id → new id for everything that must not clash with the original.
pub fn id_map(a: &Archive, new_ws: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert(a.workspace.id.clone(), new_ws.to_string());
    for (_, d) in floor_docs_of(&a.files) {
        for n in &d.nodes {
            map.insert(n.id.clone(), new_id());
            if let Some(t) = n.data.get("terminalId").and_then(Value::as_str) {
                map.entry(t.to_string()).or_insert_with(new_id);
            }
        }
        for c in &d.connections {
            map.insert(c.id.clone(), new_id());
        }
        for x in &d.drawings {
            map.insert(x.id.clone(), new_id());
        }
        for g in &d.groups {
            map.insert(g.id.clone(), new_id());
        }
        for t in &d.cable_ties {
            map.insert(t.id.clone(), new_id());
        }
    }
    for r in &a.routines {
        map.insert(r.id.clone(), new_id());
    }
    // Never remap the reserved ids.
    map.remove(GROUND_FLOOR_ID);
    map
}

pub fn import(
    store: &CanvasStore,
    a: &Archive,
    opts: &ImportOptions,
) -> Result<ImportResult, String> {
    let new_ws = new_id();
    let map = id_map(a, &new_ws);
    let root = store.root().to_path_buf();
    let ws_dir = store.workspace_dir(&new_ws);
    let ws_dir_str = ws_dir.to_string_lossy().into_owned();
    let name = unique_name(
        store,
        opts.name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&a.workspace.name),
    );
    let dir = match opts.dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(d) => Some(d.to_string()),
        None => a
            .workspace
            .working_directory
            .clone()
            .filter(|d| Path::new(d).is_dir()),
    };

    let fix = |v: &mut Value| {
        remap_value(v, &map);
        replace_in_strings(v, WS_DIR_TOKEN, &ws_dir_str);
    };
    let mut node_ids = Vec::new();
    let mut notes = Vec::new();
    store.with_io(|| -> Result<(), String> {
        fs::create_dir_all(&ws_dir).map_err(io)?;
        for (rel, f) in &a.files {
            let out_rel = remap_rel(rel, &map);
            if !safe_rel(&out_rel) {
                continue;
            }
            let path = ws_dir.join(&out_rel);
            if let Some(p) = path.parent() {
                fs::create_dir_all(p).map_err(io)?;
            }
            let bytes = if let Some(t) = &f.text {
                let mut text = transform_text(rel, t, &fix);
                if rel == "floors.json" {
                    // Isolated floors' working trees did not travel.
                    if let Ok(mut v) = serde_json::from_str::<Value>(&text) {
                        if let Some(o) = v.as_object_mut() {
                            o.insert("floors".into(), json!({}));
                        }
                        text = serde_json::to_string_pretty(&v).unwrap_or(text);
                    }
                }
                if rel.starts_with("floors/") && rel.ends_with(".json") {
                    if let Ok(d) = serde_json::from_str::<FloorDoc>(&text) {
                        node_ids.extend(d.nodes.iter().map(|n| n.id.clone()));
                    }
                }
                if rel.starts_with("notes/") {
                    notes.push(out_rel.clone());
                }
                text.into_bytes()
            } else if let Some(b) = &f.base64 {
                base64::engine::general_purpose::STANDARD
                    .decode(b)
                    .map_err(|e| format!("CANVAS_INVALID: {rel}: {e}"))?
            } else {
                Vec::new()
            };
            fs::write(&path, bytes).map_err(io)?;
        }
        // Global roles become workspace roles (unless already present).
        for role in &a.global_roles {
            let dir = ws_dir.join("roles").join(&role.id);
            if dir.join("role.json").exists() {
                continue;
            }
            let mut r = role.clone();
            r.workspace_id = Some(new_ws.clone());
            r.builtin = false;
            fs::create_dir_all(&dir).map_err(io)?;
            fs::write(
                dir.join("role.json"),
                serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
            )
            .map_err(io)?;
        }
        let mut meta = a.workspace.clone();
        meta.id = new_ws.clone();
        meta.name = name.clone();
        meta.working_directory = dir.clone();
        meta.sort_index = None;
        meta.is_pinned = false;
        meta.created_at = now_rfc3339();
        meta.updated_at = meta.created_at.clone();
        for f in &mut meta.floors {
            f.is_git_isolated = false;
            f.path = None;
        }
        super::persist::save_workspace(&root, &meta)
    })?;

    let mut rs = Vec::new();
    for r in &a.routines {
        let mut v = serde_json::to_value(r).map_err(|e| e.to_string())?;
        remap_value(&mut v, &map);
        if let Ok(mut r) = serde_json::from_value::<Routine>(v) {
            r.workspace_id = new_ws.clone();
            r.last_run_at = None;
            rs.push(r);
        }
    }
    let n_routines = if rs.is_empty() {
        0
    } else {
        routines::replace_workspace(store, &new_ws, rs)?
    };
    store.reload_workspace(&new_ws)?;
    Ok(ImportResult {
        workspace_id: new_ws,
        name,
        node_ids,
        floors: a.workspace.floors.len(),
        routines: n_routines,
        notes,
    })
}

// ---------------------------------------------------------------- backups

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub floors: usize,
    pub nodes: usize,
    pub notes: usize,
    pub cords: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub workspace_name: String,
    pub created_at: String,
    pub reason: String,
    pub hash: String,
    pub size_bytes: u64,
    pub summary: BackupSummary,
    /// True when the call found nothing new and returned the latest backup.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unchanged: bool,
}

pub fn backups_root(store: &CanvasStore) -> PathBuf {
    let root = store.root();
    root.parent().unwrap_or(root).join("canvas-backups")
}

fn ws_backups_dir(store: &CanvasStore, ws: &str) -> Result<PathBuf, String> {
    if !super::model::is_safe_id(ws) {
        return Err(format!("CANVAS_INVALID: workspace id '{ws}'"));
    }
    Ok(backups_root(store).join(ws))
}

/// Content hash of a workspace folder. Floor camera and epoch are left out,
/// so looking around the canvas does not count as a change.
fn content_hash(files: &[(String, PathBuf)]) -> String {
    let mut h = Sha256::new();
    for (rel, p) in files {
        let Ok(bytes) = fs::read(p) else { continue };
        h.update(rel.as_bytes());
        h.update([0]);
        if rel.starts_with("floors/") && rel.ends_with(".json") {
            if let Ok(mut v) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(o) = v.as_object_mut() {
                    o.remove("camera");
                    o.remove("epoch");
                    o.remove("rev");
                }
                h.update(serde_json::to_vec(&v).unwrap_or_default());
                continue;
            }
        }
        h.update(&bytes);
    }
    hex::encode(h.finalize())
}

fn summary_of(dir: &Path) -> BackupSummary {
    let mut s = BackupSummary::default();
    if let Ok(rd) = fs::read_dir(dir.join("floors")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(d) = fs::read(&p)
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<FloorDoc>(&b).map_err(|e| e.to_string()))
            {
                s.floors += 1;
                s.nodes += d.nodes.len();
                s.cords += d.connections.len();
                s.notes += d.nodes.iter().filter(|n| n.kind.as_str() == "note").count();
            }
        }
    }
    s
}

pub fn list_backups(store: &CanvasStore, ws: &str) -> Result<Vec<BackupInfo>, String> {
    let dir = ws_backups_dir(store, ws)?;
    let mut out: Vec<BackupInfo> = fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let b = fs::read(e.path().join("manifest.json")).ok()?;
                    serde_json::from_slice(&b).ok()
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(out)
}

fn copy_files(files: &[(String, PathBuf)], dest: &Path) -> Result<u64, String> {
    let mut total = 0;
    for (rel, src) in files {
        let to = dest.join(rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p).map_err(io)?;
        }
        total += fs::copy(src, &to).map_err(io)?;
    }
    Ok(total)
}

/// Snapshots the workspace folder. When nothing changed since the latest
/// backup, returns that one with `unchanged: true` and writes nothing.
pub fn create_backup(store: &CanvasStore, ws: &str, reason: &str) -> Result<BackupInfo, String> {
    let _ = store.flush_workspace(ws);
    let meta = store.workspace(ws)?;
    let src = store.workspace_dir(ws);
    let root = ws_backups_dir(store, ws)?;
    let info = store.with_io(|| -> Result<BackupInfo, String> {
        let files = walk(&src);
        let hash = content_hash(&files);
        if let Some(mut last) = list_backups(store, ws)?.into_iter().next() {
            if last.hash == hash {
                last.unchanged = true;
                return Ok(last);
            }
        }
        let reason = reason
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(32)
            .collect::<String>();
        let reason = if reason.is_empty() {
            "manual".into()
        } else {
            reason
        };
        let now = chrono::Utc::now();
        let id = format!("{}-{reason}", now.format("%Y%m%dT%H%M%S%3f"));
        let dest = root.join(&id);
        let files_dir = dest.join("files");
        fs::create_dir_all(&files_dir).map_err(io)?;
        let size = copy_files(&files, &files_dir)?;
        let rs = routines::list(store, Some(ws));
        fs::write(
            dest.join("routines.json"),
            serde_json::to_vec_pretty(&rs).map_err(|e| e.to_string())?,
        )
        .map_err(io)?;
        let info = BackupInfo {
            id,
            workspace_id: ws.to_string(),
            workspace_name: meta.name.clone(),
            created_at: now_rfc3339(),
            reason,
            hash,
            size_bytes: size,
            summary: summary_of(&files_dir),
            unchanged: false,
        };
        fs::write(
            dest.join("manifest.json"),
            serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?,
        )
        .map_err(io)?;
        Ok(info)
    })?;
    if !info.unchanged {
        prune(store, ws);
    }
    Ok(info)
}

/// Which backups to delete: keep the newest `KEEP_RECENT` scheduled ones, the
/// newest of each of the `KEEP_DAILY` days before them, and the newest
/// `KEEP_OTHER` taken before a destructive operation. Input newest first.
pub fn prune_plan(list: &[BackupInfo]) -> Vec<String> {
    let mut drop = Vec::new();
    let (mut recent, mut other) = (0, 0);
    let mut days = Vec::<String>::new();
    for b in list {
        if b.reason == "hourly" {
            if recent < KEEP_RECENT {
                recent += 1;
                continue;
            }
            let day: String = b.created_at.chars().take(10).collect();
            if !days.contains(&day) && days.len() < KEEP_DAILY {
                days.push(day);
                continue;
            }
            drop.push(b.id.clone());
        } else if other < KEEP_OTHER {
            other += 1;
        } else {
            drop.push(b.id.clone());
        }
    }
    drop
}

fn prune(store: &CanvasStore, ws: &str) {
    let Ok(list) = list_backups(store, ws) else {
        return;
    };
    let Ok(dir) = ws_backups_dir(store, ws) else {
        return;
    };
    for id in prune_plan(&list) {
        let _ = fs::remove_dir_all(dir.join(id));
    }
}

/// Backs up every workspace (the hourly job). Returns how many were written.
pub fn backup_all(store: &CanvasStore, reason: &str) -> usize {
    store
        .workspaces()
        .into_iter()
        .filter_map(|s| create_backup(store, &s.workspace.id, reason).ok())
        .filter(|b| !b.unchanged)
        .count()
}

/// Best-effort backup before a destructive operation (never blocks it).
pub fn before(store: &CanvasStore, ws: &str, what: &str) {
    if let Err(e) = create_backup(store, ws, &format!("before-{what}")) {
        tracing::warn!("[canvas] backup before {what} of {ws} failed: {e}");
    }
}

// ---------------------------------------------------------------- diff

#[derive(Debug, Clone, Default)]
struct Content {
    floors: BTreeMap<String, FloorDoc>,
    notes: BTreeMap<String, String>,
    floor_names: BTreeMap<String, String>,
}

fn content_of(dir: &Path) -> Content {
    let mut c = Content::default();
    if let Ok(rd) = fs::read_dir(dir.join("floors")) {
        for e in rd.flatten() {
            let p = e.path();
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(d) = fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<FloorDoc>(&b).ok())
            {
                c.floors.insert(stem.to_string(), d);
            }
        }
    }
    if let Ok(rd) = fs::read_dir(dir.join("notes")) {
        for e in rd.flatten() {
            if let (Some(n), Ok(t)) = (
                e.file_name()
                    .to_str()
                    .filter(|n| !n.ends_with(".bak"))
                    .map(str::to_string),
                fs::read_to_string(e.path()),
            ) {
                c.notes.insert(n, t);
            }
        }
    }
    if let Some(ws) = fs::read(dir.join("workspace.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Workspace>(&b).ok())
    {
        for f in ws.floors {
            c.floor_names.insert(f.id, f.name);
        }
    }
    c
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDiff {
    /// Nodes the restore brings back (in the backup, not now).
    pub nodes_back: Vec<String>,
    /// Nodes made since the backup that the restore removes.
    pub nodes_removed: Vec<String>,
    pub nodes_changed: usize,
    pub cords_back: usize,
    pub cords_removed: usize,
    pub notes_changed: Vec<String>,
    pub floors_back: Vec<String>,
    pub floors_removed: Vec<String>,
    pub lines: Vec<String>,
    pub identical: bool,
}

fn node_label(n: &super::model::CanvasNode) -> String {
    match &n.title {
        Some(t) if !t.trim().is_empty() => format!("{} ({})", t, n.kind.as_str()),
        _ => format!("{} {}", n.kind.as_str(), &n.id[..n.id.len().min(8)]),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What restoring `backup` over `current` changes, in words.
fn diff_content(current: &Content, backup: &Content) -> RestoreDiff {
    let mut d = RestoreDiff::default();
    let index = |c: &Content| {
        let mut nodes = BTreeMap::new();
        let mut cords = HashSet::new();
        for doc in c.floors.values() {
            for n in &doc.nodes {
                let mut v = serde_json::to_value(n).unwrap_or(Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.remove("z");
                }
                nodes.insert(n.id.clone(), (node_label(n), v));
            }
            for c in &doc.connections {
                cords.insert(c.id.clone());
            }
        }
        (nodes, cords)
    };
    let (now_nodes, now_cords) = index(current);
    let (bak_nodes, bak_cords) = index(backup);
    for (id, (label, v)) in &bak_nodes {
        match now_nodes.get(id) {
            None => d.nodes_back.push(label.clone()),
            Some((_, nv)) if nv != v => d.nodes_changed += 1,
            _ => {}
        }
    }
    for (id, (label, _)) in &now_nodes {
        if !bak_nodes.contains_key(id) {
            d.nodes_removed.push(label.clone());
        }
    }
    d.cords_back = bak_cords.difference(&now_cords).count();
    d.cords_removed = now_cords.difference(&bak_cords).count();
    let names: HashSet<&String> = current.notes.keys().chain(backup.notes.keys()).collect();
    for n in names {
        if current.notes.get(n) != backup.notes.get(n) {
            d.notes_changed.push(n.clone());
        }
    }
    d.notes_changed.sort();
    let floor_name =
        |c: &Content, id: &String| c.floor_names.get(id).cloned().unwrap_or(id.clone());
    for id in backup.floors.keys() {
        if !current.floors.contains_key(id) {
            d.floors_back.push(floor_name(backup, id));
        }
    }
    for id in current.floors.keys() {
        if !backup.floors.contains_key(id) {
            d.floors_removed.push(floor_name(current, id));
        }
    }
    let list = |v: &[String]| {
        let mut s = v.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        if v.len() > 6 {
            s.push_str(&format!(" and {} more", v.len() - 6));
        }
        s
    };
    if !d.nodes_back.is_empty() {
        d.lines.push(format!(
            "brings back {}: {}",
            plural(d.nodes_back.len(), "node", "nodes"),
            list(&d.nodes_back)
        ));
    }
    if !d.nodes_removed.is_empty() {
        d.lines.push(format!(
            "removes {} made since: {}",
            plural(d.nodes_removed.len(), "node", "nodes"),
            list(&d.nodes_removed)
        ));
    }
    if d.nodes_changed > 0 {
        d.lines.push(format!(
            "{} change back (position, size or settings)",
            plural(d.nodes_changed, "node", "nodes")
        ));
    }
    if d.cords_back > 0 {
        d.lines.push(format!(
            "{} come back",
            plural(d.cords_back, "cord", "cords")
        ));
    }
    if d.cords_removed > 0 {
        d.lines.push(format!(
            "{} go away",
            plural(d.cords_removed, "cord", "cords")
        ));
    }
    if !d.notes_changed.is_empty() {
        d.lines.push(format!(
            "{} change: {}",
            plural(d.notes_changed.len(), "note file", "note files"),
            list(&d.notes_changed)
        ));
    }
    if !d.floors_back.is_empty() {
        d.lines
            .push(format!("floors back: {}", list(&d.floors_back)));
    }
    if !d.floors_removed.is_empty() {
        d.lines
            .push(format!("floors removed: {}", list(&d.floors_removed)));
    }
    d.identical = d.lines.is_empty();
    if d.identical {
        d.lines.push("the canvas is the same as this backup".into());
    }
    d
}

pub fn restore_diff(store: &CanvasStore, ws: &str, backup_id: &str) -> Result<RestoreDiff, String> {
    let _ = store.flush_workspace(ws);
    let bdir = backup_dir(store, ws, backup_id)?;
    let current = content_of(&store.workspace_dir(ws));
    let backup = content_of(&bdir.join("files"));
    Ok(diff_content(&current, &backup))
}

fn backup_dir(store: &CanvasStore, ws: &str, backup_id: &str) -> Result<PathBuf, String> {
    if !super::model::is_safe_id(backup_id) {
        return Err(format!("CANVAS_INVALID: backup id '{backup_id}'"));
    }
    let dir = ws_backups_dir(store, ws)?.join(backup_id);
    if !dir.join("manifest.json").exists() {
        return Err(format!("CANVAS_NOT_FOUND: backup {backup_id} of {ws}"));
    }
    Ok(dir)
}

/// Replaces the workspace folder with the backup (a safety backup first when
/// the workspace exists) and reloads it. Terminals are the caller's job.
pub fn restore_backup(store: &CanvasStore, ws: &str, backup_id: &str) -> Result<Workspace, String> {
    let bdir = backup_dir(store, ws, backup_id)?;
    let exists = store.workspace(ws).is_ok();
    if exists {
        create_backup(store, ws, "before-restore")?;
    }
    let dest = store.workspace_dir(ws);
    store.with_io(|| -> Result<(), String> {
        if dest.exists() {
            for e in fs::read_dir(&dest).map_err(io)?.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    fs::remove_dir_all(&p).map_err(io)?;
                } else {
                    fs::remove_file(&p).map_err(io)?;
                }
            }
        }
        fs::create_dir_all(&dest).map_err(io)?;
        let src = bdir.join("files");
        let files = walk(&src);
        copy_files(&files, &dest)?;
        Ok(())
    })?;
    if let Some(rs) = fs::read(bdir.join("routines.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Routine>>(&b).ok())
    {
        routines::replace_workspace(store, ws, rs)?;
    }
    store.reload_workspace(ws)
}
