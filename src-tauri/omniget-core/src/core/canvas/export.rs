//! Partitura export/import of a selection: nodes, cords with both ends inside,
//! drawings, groups, the roles the terminals use (embedded) and the note texts.
//! "Blueprints, not backups" (Maestri): no scrollback, no run state, no
//! environment overrides and no absolute paths. A terminal's `cwd` inside the
//! workspace folder is kept relative and resolved against the destination.
//!
//! The partitura library itself (`~/.../partituras/`) belongs to the
//! partituras module; this file only turns a selection into the document and
//! stamps a document back onto a floor.

use super::model::{
    new_id, now_rfc3339, CanvasNode, Connection, Drawing, FloorDoc, Group, NodeKind, Point, Rect,
};
use super::notes;
use super::ops::Op;
use super::store::{ApplyResult, CanvasStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

pub const PARTITURA_FORMAT: &str = "omniget.partitura";
pub const PARTITURA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedRole {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Partitura {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_workspace_id: Option<String>,
    /// Size of the selection; node and drawing coordinates are relative to its top-left.
    #[serde(default)]
    pub bounds: super::model::Size,
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub drawings: Vec<Drawing>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub roles: Vec<EmbeddedRole>,
    /// Note node id -> markdown.
    #[serde(default)]
    pub note_texts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub drawing_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// Keep terminal `command` lines (default true).
    #[serde(default)]
    pub include_startup_commands: Option<bool>,
    /// Ship notes empty.
    #[serde(default)]
    pub blank_notes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub apply: ApplyResult,
    pub node_ids: Vec<String>,
    pub drawing_ids: Vec<String>,
    /// Roles the partitura carries; the roles module merges them (id, then name).
    pub roles: Vec<EmbeddedRole>,
}

/// Default role lookup: `<ws>/roles/<roleId>/role.json` (`{id, name, prompt, icon?, color?}`),
/// with the prompt falling back to a sibling `prompt.md`, `CLAUDE.md` or `AGENTS.md`.
pub fn role_from_disk(store: &CanvasStore, ws: &str, role_id: &str) -> Option<EmbeddedRole> {
    // One storage for roles: workspace roles, then the library (core::canvas::roles).
    let (r, _) = super::roles::get(store, ws, role_id)?;
    Some(EmbeddedRole {
        id: r.id,
        name: r.name,
        prompt: r.prompt,
        icon: r.icon,
        color: r.color,
    })
}

fn is_local_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("file:") || u.starts_with('/') || (u.len() > 2 && u.as_bytes()[1] == b':')
}

/// `cwd` relative to the workspace folder when inside it, else dropped.
fn relative_cwd(cwd: &str, base: Option<&str>) -> Option<String> {
    let base = base?;
    let rel = Path::new(cwd).strip_prefix(base).ok()?;
    let s = rel.to_string_lossy().replace('\\', "/");
    Some(if s.is_empty() {
        ".".into()
    } else {
        format!("./{s}")
    })
}

/// Builds a partitura from a selection of one floor.
pub fn export_selection(
    doc: &FloorDoc,
    workspace_id: &str,
    working_directory: Option<&str>,
    selection: &Selection,
    options: &ExportOptions,
    note_text: &dyn Fn(&CanvasNode) -> Option<String>,
    resolve_role: &dyn Fn(&str) -> Option<EmbeddedRole>,
) -> Result<Partitura, String> {
    let mut wanted: HashSet<&str> = selection.node_ids.iter().map(String::as_str).collect();
    // A selected stack brings its notes.
    for n in &doc.nodes {
        if let Some(s) = &n.stack_id {
            if wanted.contains(s.as_str()) {
                wanted.insert(n.id.as_str());
            }
        }
    }
    let keep_commands = options.include_startup_commands.unwrap_or(true);
    let mut nodes: Vec<CanvasNode> = Vec::new();
    let mut texts = BTreeMap::new();
    let mut role_ids: Vec<String> = Vec::new();

    for n in doc.nodes.iter().filter(|n| wanted.contains(n.id.as_str())) {
        let mut n = n.clone();
        let note_body = (n.kind == NodeKind::Note && !options.blank_notes)
            .then(|| note_text(&n))
            .flatten();
        let kind = n.kind.clone();
        let data = n.data.as_object_mut().ok_or("node data is not an object")?;
        match &kind {
            // Attachments and cross-floor chips don't travel.
            NodeKind::File | NodeKind::Connector | NodeKind::Unknown(_) => continue,
            NodeKind::Terminal | NodeKind::Agent => {
                for k in ["terminalId", "env", "botId", "appearance"] {
                    data.remove(k);
                }
                if !keep_commands {
                    data.remove("command");
                }
                if let Some(cwd) = data
                    .remove("cwd")
                    .and_then(|v| v.as_str().map(str::to_string))
                {
                    if let Some(rel) = relative_cwd(&cwd, working_directory) {
                        data.insert("cwd".into(), Value::String(rel));
                    }
                }
                if let Some(r) = data.get("roleId").and_then(|v| v.as_str()) {
                    if !role_ids.iter().any(|x| x == r) {
                        role_ids.push(r.to_string());
                    }
                }
            }
            NodeKind::Note => {
                texts.insert(n.id.clone(), note_body.unwrap_or_default());
                // External notes become internal ones on the other side.
                data.remove("path");
                data.remove("isExternal");
                data.remove("fileName");
            }
            NodeKind::FileTree => {
                let root = data
                    .get("rootPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let rel = relative_cwd(&root, working_directory).unwrap_or_else(|| ".".into());
                data.insert("rootPath".into(), Value::String(rel));
                for k in ["currentPath", "pinnedPaths", "expanded"] {
                    data.remove(k);
                }
            }
            NodeKind::Portal => {
                if data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .is_some_and(is_local_url)
                {
                    continue;
                }
            }
            NodeKind::Image => {
                data.remove("path");
                if data
                    .get("url")
                    .and_then(|v| v.as_str())
                    .is_none_or(is_local_url)
                {
                    continue;
                }
            }
            _ => {}
        }
        nodes.push(n);
    }
    if nodes.is_empty() && selection.drawing_ids.is_empty() {
        return Err("CANVAS_INVALID: nothing exportable in the selection".into());
    }

    let kept: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    // Stack tabs pointing at dropped notes go away.
    for n in nodes.iter_mut().filter(|n| n.kind == NodeKind::Stack) {
        if let Some(tabs) = n.data.get_mut("tabs").and_then(|t| t.as_array_mut()) {
            tabs.retain(|t| t.as_str().is_some_and(|s| kept.contains(s)));
        }
    }
    let connections: Vec<Connection> = doc
        .connections
        .iter()
        .filter(|c| kept.contains(&c.from) && kept.contains(&c.to))
        .cloned()
        .map(|mut c| {
            c.is_active = None;
            c
        })
        .collect();
    let drawing_set: HashSet<&str> = selection.drawing_ids.iter().map(String::as_str).collect();
    let drawings: Vec<Drawing> = doc
        .drawings
        .iter()
        .filter(|d| drawing_set.contains(d.id.as_str()))
        .cloned()
        .collect();
    let group_ids: HashSet<&str> = nodes
        .iter()
        .filter_map(|n| n.group_id.as_deref())
        .chain(drawings.iter().filter_map(|d| d.group_id.as_deref()))
        .collect();
    let groups: Vec<Group> = doc
        .groups
        .iter()
        .filter(|g| group_ids.contains(g.id.as_str()))
        .cloned()
        .collect();

    // Normalize coordinates to the selection's top-left.
    let bbox = nodes
        .iter()
        .filter(|n| n.is_on_canvas())
        .map(CanvasNode::rect)
        .chain(drawings.iter().map(Drawing::bounds))
        .reduce(|a, b| a.union(&b))
        .unwrap_or(Rect::default());
    for n in &mut nodes {
        n.x -= bbox.x;
        n.y -= bbox.y;
    }
    let mut drawings = drawings;
    for d in &mut drawings {
        d.origin.x -= bbox.x;
        d.origin.y -= bbox.y;
    }

    Ok(Partitura {
        format: PARTITURA_FORMAT.into(),
        version: PARTITURA_VERSION,
        id: new_id(),
        name: options.name.trim().to_string(),
        summary: options.summary.clone(),
        icon: options.icon.clone(),
        color: options.color.clone(),
        created_at: now_rfc3339(),
        source_workspace_id: Some(workspace_id.to_string()),
        bounds: super::model::Size {
            w: bbox.w,
            h: bbox.h,
        },
        nodes,
        connections,
        drawings,
        groups,
        roles: role_ids.iter().filter_map(|r| resolve_role(r)).collect(),
        note_texts: texts,
    })
}

/// Store-level export with the default role lookup.
pub fn export_from_store(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    selection: &Selection,
    options: &ExportOptions,
) -> Result<Partitura, String> {
    if options.name.trim().is_empty() {
        return Err("CANVAS_INVALID: partitura name is empty".into());
    }
    let meta = store.workspace(ws)?;
    let doc = store.read(ws, floor, |d| d.clone())?;
    let text = |n: &CanvasNode| notes::read(store, ws, floor, &n.id).ok().map(|c| c.text);
    let role = |id: &str| role_from_disk(store, ws, id);
    export_selection(
        &doc,
        ws,
        meta.working_directory.as_deref(),
        selection,
        options,
        &text,
        &role,
    )
}

/// Stamps a partitura onto a floor with its top-left at `at`. Every id is new;
/// note texts become note files; relative `cwd`s resolve against the
/// workspace folder. `role_map` remaps role ids (after the roles module merged
/// the embedded roles); unmapped ids are kept. Everything arrives in one batch.
pub fn import_partitura(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    partitura: &Partitura,
    at: Point,
    origin: &str,
    role_map: &HashMap<String, String>,
) -> Result<ImportResult, String> {
    if partitura.format != PARTITURA_FORMAT {
        return Err(format!(
            "CANVAS_INVALID: not a partitura ({})",
            partitura.format
        ));
    }
    if partitura.version > PARTITURA_VERSION {
        return Err("CANVAS_INVALID: partitura made by a newer OmniGet".into());
    }
    let meta = store.workspace(ws)?;
    let base = meta.working_directory.clone();
    let z0 = store.read(ws, floor, |d| d.max_z())? + 1.0;

    let mut ids: HashMap<String, String> = HashMap::new();
    for n in &partitura.nodes {
        ids.insert(n.id.clone(), new_id());
    }
    for d in &partitura.drawings {
        ids.insert(d.id.clone(), new_id());
    }
    for g in &partitura.groups {
        ids.insert(g.id.clone(), new_id());
    }
    let map = |id: &str| ids.get(id).cloned();

    let mut ops = Vec::new();
    for g in &partitura.groups {
        let mut g = g.clone();
        g.id = map(&g.id).unwrap_or_else(new_id);
        ops.push(Op::SetGroup {
            id: g.id.clone(),
            group: Some(g),
        });
    }
    let mut node_ids = Vec::new();
    let mut note_writes: Vec<(String, String)> = Vec::new();
    let min_z = partitura.nodes.iter().map(|n| n.z).fold(f64::MAX, f64::min);
    for n in &partitura.nodes {
        let mut n = n.clone();
        let old = n.id.clone();
        n.id = map(&old).expect("mapped");
        n.x += at.x;
        n.y += at.y;
        n.z = z0 + (n.z - min_z).max(0.0);
        n.group_id = n.group_id.as_deref().and_then(map);
        n.stack_id = n.stack_id.as_deref().and_then(map);
        let data = n.data.as_object_mut().ok_or("node data is not an object")?;
        match &n.kind {
            NodeKind::Terminal | NodeKind::Agent => {
                data.remove("terminalId");
                if let Some(cwd) = data.get("cwd").and_then(|v| v.as_str()).map(str::to_string) {
                    match (
                        &base,
                        cwd.strip_prefix("./")
                            .or(if cwd == "." { Some("") } else { None }),
                    ) {
                        (Some(b), Some(rel)) => {
                            let p = Path::new(b).join(rel);
                            data.insert(
                                "cwd".into(),
                                Value::String(p.to_string_lossy().into_owned()),
                            );
                        }
                        _ => {
                            data.remove("cwd");
                        }
                    }
                }
                if let Some(r) = data.get("roleId").and_then(|v| v.as_str()) {
                    if let Some(new) = role_map.get(r) {
                        data.insert("roleId".into(), Value::String(new.clone()));
                    }
                }
            }
            NodeKind::FileTree => {
                let rel = data
                    .get("rootPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".")
                    .to_string();
                let root = match &base {
                    Some(b) => Path::new(b)
                        .join(rel.trim_start_matches("./").trim_start_matches('.'))
                        .to_string_lossy()
                        .into_owned(),
                    None => dirs::home_dir()
                        .map(|h| h.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                data.insert("rootPath".into(), Value::String(root));
            }
            NodeKind::Note => {
                data.remove("path");
                data.remove("isExternal");
                data.remove("fileName");
                if let Some(t) = partitura.note_texts.get(&old) {
                    if !t.is_empty() {
                        note_writes.push((n.id.clone(), t.clone()));
                    }
                }
            }
            NodeKind::Stack => {
                if let Some(tabs) = data.get_mut("tabs").and_then(|t| t.as_array_mut()) {
                    let next: Vec<Value> = tabs
                        .iter()
                        .filter_map(|t| t.as_str().and_then(map))
                        .map(Value::String)
                        .collect();
                    *tabs = next;
                }
                if let Some(f) = data
                    .get("frontNoteId")
                    .and_then(|v| v.as_str())
                    .and_then(map)
                {
                    data.insert("frontNoteId".into(), Value::String(f));
                } else {
                    data.remove("frontNoteId");
                }
            }
            _ => {}
        }
        node_ids.push(n.id.clone());
        ops.push(Op::AddNode { node: n });
    }
    for c in &partitura.connections {
        let (Some(from), Some(to)) = (map(&c.from), map(&c.to)) else {
            continue;
        };
        let mut c = c.clone();
        c.id = new_id();
        c.from = from;
        c.to = to;
        c.from_note_id = c.from_note_id.as_deref().and_then(map);
        c.to_note_id = c.to_note_id.as_deref().and_then(map);
        c.points.clear();
        ops.push(Op::AddConnection { connection: c });
    }
    let mut drawing_ids = Vec::new();
    for d in &partitura.drawings {
        let mut d = d.clone();
        d.id = map(&d.id).expect("mapped");
        d.origin.x += at.x;
        d.origin.y += at.y;
        d.group_id = d.group_id.as_deref().and_then(map);
        for a in [&mut d.source_connection, &mut d.target_connection] {
            if let Some(anchor) = a {
                match map(&anchor.drawing_id) {
                    Some(id) => anchor.drawing_id = id,
                    None => *a = None,
                }
            }
        }
        drawing_ids.push(d.id.clone());
        ops.push(Op::AddDrawing { drawing: d });
    }

    let apply = store.apply(ws, floor, ops, origin, None)?;
    for (id, text) in note_writes {
        if let Err(e) = notes::write(store, ws, floor, &id, &text, None, origin) {
            tracing::warn!("[canvas] partitura note {id}: {e}");
        }
    }
    Ok(ImportResult {
        apply,
        node_ids,
        drawing_ids,
        roles: partitura.roles.clone(),
    })
}
