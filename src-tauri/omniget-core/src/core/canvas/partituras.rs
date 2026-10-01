//! Partitura library (Maestri "partituras", study 95 §3): saved canvas
//! arrangements, one JSON per partitura in `<app_data>/canvas/partituras/`,
//! identical to the exported file. Built on `export` (selection → document,
//! document → floor); this module adds the library, the import review
//! (every command listed before anything runs, role conflicts resolved one
//! by one) and the optional fields of study 96 idea 8: budget, permissions
//! and a setup script, so one file carries the whole team.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::export::{
    self, EmbeddedRole, ExportOptions, ImportResult, Partitura, Selection, PARTITURA_FORMAT,
};
use super::floors::{self, FloorHooks, HookCommand};
use super::model::{is_safe_id, new_id, now_rfc3339, NodeKind, Point, GROUND_FLOOR_ID};
use super::ops::Op;
use super::persist;
use super::store::CanvasStore;

pub const BUNDLE_FORMAT: &str = "omniget.partituras";

// ---------------------------------------------------------------- format extras

/// Spending cap the team should respect (read by the agents layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_minutes: Option<u64>,
}

/// How much the team may do without asking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// `supervised` | `acceptEdits` | `auto` | `full`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Cords may type raw keys (`ask --raw`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_raw: Option<bool>,
    /// Portals may run JS (`portal evaluate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_evaluate: Option<bool>,
}

/// What the floor needs before the team works (the Conductor-style scripts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetupSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teardown: Option<String>,
}

impl SetupSpec {
    pub fn is_empty(&self) -> bool {
        [&self.setup, &self.run, &self.teardown]
            .iter()
            .all(|s| s.as_deref().is_none_or(|t| t.trim().is_empty()))
    }
}

/// A partitura as stored in the library (and as exported).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPartitura {
    #[serde(flatten)]
    pub doc: Partitura,
    /// Workspace scope; absent = global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupSpec>,
    /// What the author left out when saving; "Update Existing…" keeps it so
    /// a new layout never brings back a command or a note taken out before.
    #[serde(default, skip_serializing_if = "Privacy::is_default")]
    pub privacy: Privacy,
    /// Format 2 (opt-in "Include chat history"): per partitura node id, its
    /// chat transcript with the attachments inlined. Stamping copies it into
    /// the new nodes' own conversations only with `acceptChat`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat: BTreeMap<String, Vec<CarriedEntry>>,
}

/// Version written when a partitura carries chat (an older app refuses it).
pub const CHAT_FORMAT_VERSION: u32 = 2;
/// Largest attachment carried inline; bigger ones travel as a name only.
pub const CHAT_ATTACHMENT_MAX: u64 = 8 * 1024 * 1024;
/// Total inline attachment bytes per partitura.
pub const CHAT_ATTACHMENTS_TOTAL: u64 = 32 * 1024 * 1024;

/// A chat entry travelling inside a partitura: the entry without machine
/// paths, plus the attachment bytes (base64).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedEntry {
    pub entry: super::chat::ChatEntry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<CarriedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedFile {
    pub id: String,
    pub filename: String,
    /// Base64; absent when the file was too big or unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// The privacy options a partitura was saved with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Privacy {
    /// Startup commands were left out.
    #[serde(default)]
    pub no_startup_commands: bool,
    /// Notes ship empty.
    #[serde(default)]
    pub blank_notes: bool,
}

impl Privacy {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Previous versions kept per partitura (Update Existing… can be undone).
pub const HISTORY_KEEP: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub partituras: Vec<LibraryPartitura>,
}

pub fn library_dir(store: &CanvasStore) -> PathBuf {
    store.root().join("partituras")
}

fn file_of(store: &CanvasStore, id: &str) -> Result<PathBuf, String> {
    if !is_safe_id(id) {
        return Err(format!("CANVAS_INVALID: partitura id '{id}'"));
    }
    Ok(library_dir(store).join(format!("{id}.json")))
}

fn write(store: &CanvasStore, p: &LibraryPartitura) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(p).map_err(|e| e.to_string())?;
    persist::write_atomic(&file_of(store, &p.doc.id)?, &bytes)
}

fn history_dir(store: &CanvasStore, id: &str) -> Result<PathBuf, String> {
    if !is_safe_id(id) {
        return Err(format!("CANVAS_INVALID: partitura id '{id}'"));
    }
    Ok(library_dir(store).join(".history").join(id))
}

/// Keeps the current file as a version before it is replaced (newest last,
/// at most [`HISTORY_KEEP`]).
fn push_history(store: &CanvasStore, old: &LibraryPartitura) -> Result<(), String> {
    let dir = history_dir(store, &old.doc.id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let bytes = serde_json::to_vec_pretty(old).map_err(|e| e.to_string())?;
    persist::write_atomic(&dir.join(format!("{stamp}.json")), &bytes)?;
    let mut files = versions_files(&dir);
    while files.len() > HISTORY_KEEP {
        let _ = std::fs::remove_file(files.remove(0));
    }
    Ok(())
}

fn versions_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// How many earlier versions a partitura has.
pub fn version_count(store: &CanvasStore, id: &str) -> usize {
    history_dir(store, id)
        .map(|d| versions_files(&d).len())
        .unwrap_or(0)
}

/// Undo of "Update Existing…": puts the newest saved version back (and
/// drops it from the history).
pub fn restore_previous(store: &CanvasStore, id: &str) -> Result<LibraryPartitura, String> {
    let dir = history_dir(store, id)?;
    let mut files = versions_files(&dir);
    let last = files
        .pop()
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: partitura {id} has no earlier version"))?;
    let bytes = std::fs::read(&last).map_err(|e| format!("CANVAS_IO: {e}"))?;
    let p: LibraryPartitura =
        serde_json::from_slice(&bytes).map_err(|e| format!("CANVAS_INVALID: {e}"))?;
    write(store, &p)?;
    let _ = std::fs::remove_file(last);
    Ok(p)
}

fn all(store: &CanvasStore) -> Vec<LibraryPartitura> {
    let Ok(rd) = std::fs::read_dir(library_dir(store)) else {
        return Vec::new();
    };
    let mut out: Vec<LibraryPartitura> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|b| serde_json::from_slice::<LibraryPartitura>(&b).ok())
        .filter(|p| p.doc.format == PARTITURA_FORMAT)
        .collect();
    out.sort_by_key(|p| p.doc.name.to_lowercase());
    out
}

pub fn get(store: &CanvasStore, id: &str) -> Result<LibraryPartitura, String> {
    let b = std::fs::read(file_of(store, id)?)
        .map_err(|_| format!("CANVAS_NOT_FOUND: partitura {id}"))?;
    serde_json::from_slice(&b).map_err(|e| format!("CANVAS_INVALID: partitura {id}: {e}"))
}

// ---------------------------------------------------------------- list

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartituraSummary {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Workspace scope (absent = global, or its workspace is not on this machine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    pub role_names: Vec<String>,
    pub node_count: usize,
    pub terminal_count: usize,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub bounds: super::model::Size,
    /// Schematic preview: node boxes and cords (by node id).
    pub shapes: Vec<Shape>,
    pub cords: Vec<(String, String)>,
    pub has_setup: bool,
    pub has_budget: bool,
    /// Options it was saved with (kept by Update Existing…).
    #[serde(default)]
    pub privacy: Privacy,
    /// Earlier versions kept (Update Existing… can be undone while > 0).
    #[serde(default)]
    pub versions: usize,
}

fn summarize(p: &LibraryPartitura, ws_names: &HashMap<String, String>) -> PartituraSummary {
    let d = &p.doc;
    let scope = p
        .scope_workspace_id
        .clone()
        .filter(|w| ws_names.contains_key(w));
    PartituraSummary {
        id: d.id.clone(),
        name: d.name.clone(),
        summary: d.summary.clone(),
        icon: d.icon.clone(),
        color: d.color.clone(),
        workspace_name: scope.as_ref().and_then(|w| ws_names.get(w).cloned()),
        scope_workspace_id: scope,
        role_names: d.roles.iter().map(|r| r.name.clone()).collect(),
        node_count: d.nodes.len(),
        terminal_count: d.nodes.iter().filter(|n| n.is_terminal()).count(),
        created_at: d.created_at.clone(),
        updated_at: p.updated_at.clone(),
        bounds: d.bounds,
        shapes: d
            .nodes
            .iter()
            .filter(|n| n.is_on_canvas())
            .map(|n| Shape {
                id: n.id.clone(),
                kind: n.kind.as_str().to_string(),
                x: n.x,
                y: n.y,
                w: n.w,
                h: n.h,
                title: n.title.clone(),
                color: n.color.clone(),
            })
            .collect(),
        cords: d
            .connections
            .iter()
            .map(|c| (c.from.clone(), c.to.clone()))
            .collect(),
        has_setup: p.setup.as_ref().is_some_and(|s| !s.is_empty()),
        has_budget: p.budget.is_some(),
        privacy: p.privacy.clone(),
        versions: 0,
    }
}

/// `scope`: `all` (default), `global`, or a workspace id (its own + global).
/// `query` matches name, summary, role names and the workspace name.
pub fn list(
    store: &CanvasStore,
    scope: Option<&str>,
    query: Option<&str>,
) -> Vec<PartituraSummary> {
    let ws_names: HashMap<String, String> = store
        .workspaces()
        .into_iter()
        .map(|w| (w.workspace.id, w.workspace.name))
        .collect();
    let q = query
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    all(store)
        .iter()
        .map(|p| {
            let mut s = summarize(p, &ws_names);
            s.versions = version_count(store, &s.id);
            s
        })
        .filter(|s| match scope.unwrap_or("all") {
            "all" => true,
            "global" => s.scope_workspace_id.is_none(),
            ws => s.scope_workspace_id.as_deref().is_none_or(|w| w == ws),
        })
        .filter(|s| {
            let Some(q) = &q else { return true };
            let hay = format!(
                "{} {} {} {}",
                s.name,
                s.summary.clone().unwrap_or_default(),
                s.role_names.join(" "),
                s.workspace_name.clone().unwrap_or_default()
            )
            .to_lowercase();
            q.split_whitespace().all(|w| hay.contains(w))
        })
        .collect()
}

// ---------------------------------------------------------------- save / edit / delete

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    pub selection: Selection,
    pub options: ExportOptions,
    /// `global`, or a workspace id (default: the source workspace).
    #[serde(default)]
    pub scope: Option<String>,
    /// "Update Existing…": replace this partitura's layout, keep its identity.
    #[serde(default)]
    pub update_existing: Option<String>,
    /// Per node: new startup command, or `null` to drop it.
    #[serde(default)]
    pub commands: HashMap<String, Option<String>>,
    #[serde(default)]
    pub budget: Option<Budget>,
    #[serde(default)]
    pub permissions: Option<Permissions>,
    #[serde(default)]
    pub setup: Option<SetupSpec>,
    /// Format 2: carry the chat history of the saved terminals.
    #[serde(default)]
    pub include_chat: bool,
}

/// Chat transcripts of the saved terminals, attachments inlined.
fn carry_chat(
    store: &CanvasStore,
    ws: &str,
    doc: &Partitura,
) -> BTreeMap<String, Vec<CarriedEntry>> {
    use base64::Engine as _;
    let mut budget = CHAT_ATTACHMENTS_TOTAL;
    let mut out = BTreeMap::new();
    for n in doc.nodes.iter().filter(|n| n.is_terminal()) {
        let Ok(entries) = super::chat::read_all(store, ws, &n.id) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        let carried: Vec<CarriedEntry> = entries
            .into_iter()
            .map(|mut e| {
                let files = e
                    .attachments
                    .iter_mut()
                    .map(|a| {
                        let data = std::fs::metadata(&a.path)
                            .ok()
                            .filter(|m| m.len() <= CHAT_ATTACHMENT_MAX && m.len() <= budget)
                            .and_then(|_| std::fs::read(&a.path).ok())
                            .map(|b| {
                                budget = budget.saturating_sub(b.len() as u64);
                                base64::engine::general_purpose::STANDARD.encode(b)
                            });
                        // No machine paths in the file.
                        a.path = String::new();
                        CarriedFile {
                            id: a.id.clone(),
                            filename: a.filename.clone(),
                            data,
                        }
                    })
                    .collect();
                CarriedEntry { entry: e, files }
            })
            .collect();
        out.insert(n.id.clone(), carried);
    }
    out
}

/// Writes carried transcripts into the stamped nodes' own (new) conversations.
fn land_chat(
    store: &CanvasStore,
    ws: &str,
    p: &LibraryPartitura,
    new_ids: &[String],
) -> Result<usize, String> {
    use base64::Engine as _;
    let mut written = 0;
    for (old, new) in p.doc.nodes.iter().zip(new_ids) {
        let Some(entries) = p.chat.get(&old.id) else {
            continue;
        };
        let dir = super::chat::attachments_dir(store, ws, new)?;
        for c in entries {
            let mut e = c.entry.clone();
            e.id = uuid::Uuid::new_v4().to_string();
            e.client_message_id = None;
            let mut kept = Vec::new();
            for mut a in e.attachments.drain(..) {
                let Some(bytes) = c
                    .files
                    .iter()
                    .find(|f| f.id == a.id)
                    .and_then(|f| f.data.as_deref())
                    .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
                else {
                    continue;
                };
                std::fs::create_dir_all(&dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
                let safe: String = a
                    .filename
                    .chars()
                    .map(|ch| if ch == '/' || ch == '\\' { '_' } else { ch })
                    .collect();
                let path = dir.join(format!("{}-{safe}", &a.id[..a.id.len().min(8)]));
                persist::write_atomic(&path, &bytes)?;
                a.path = path.to_string_lossy().into_owned();
                kept.push(a);
            }
            e.attachments = kept;
            super::chat::append(store, ws, new, &e)?;
            written += 1;
        }
    }
    Ok(written)
}

fn apply_commands(doc: &mut Partitura, commands: &HashMap<String, Option<String>>) {
    for n in doc.nodes.iter_mut().filter(|n| n.is_terminal()) {
        let Some(edit) = commands.get(&n.id) else {
            continue;
        };
        let Some(data) = n.data.as_object_mut() else {
            continue;
        };
        match edit.as_deref().map(str::trim) {
            Some(c) if !c.is_empty() => {
                data.insert("command".into(), Value::String(c.to_string()));
            }
            _ => {
                data.remove("command");
            }
        }
    }
}

pub fn save(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    mut req: SaveRequest,
) -> Result<LibraryPartitura, String> {
    // Update Existing…: what was left out stays out (stricter wins).
    let old = match req.update_existing.as_deref() {
        Some(id) => Some(get(store, id)?),
        None => None,
    };
    if let Some(o) = &old {
        if o.privacy.no_startup_commands {
            req.options.include_startup_commands = Some(false);
        }
        req.options.blank_notes |= o.privacy.blank_notes;
    }
    let privacy = Privacy {
        no_startup_commands: req.options.include_startup_commands == Some(false),
        blank_notes: req.options.blank_notes,
    };
    let mut doc = export::export_from_store(store, ws, floor, &req.selection, &req.options)?;
    apply_commands(&mut doc, &req.commands);
    let chat = if req.include_chat {
        carry_chat(store, ws, &doc)
    } else {
        BTreeMap::new()
    };
    if !chat.is_empty() {
        doc.version = CHAT_FORMAT_VERSION;
    }
    let scope = match req.scope.as_deref() {
        Some("global") => None,
        Some(w) if !w.is_empty() => Some(w.to_string()),
        _ => Some(ws.to_string()),
    };
    let mut p = LibraryPartitura {
        doc,
        scope_workspace_id: scope,
        updated_at: None,
        budget: req.budget,
        permissions: req.permissions,
        setup: req.setup.filter(|s| !s.is_empty()),
        privacy,
        chat,
    };
    if let Some(old) = old {
        push_history(store, &old)?;
        let d = &mut p.doc;
        d.id = old.doc.id;
        d.name = old.doc.name;
        d.summary = old.doc.summary;
        d.icon = old.doc.icon;
        d.color = old.doc.color;
        d.created_at = old.doc.created_at;
        p.scope_workspace_id = old.scope_workspace_id;
        p.updated_at = Some(now_rfc3339());
        p.budget = p.budget.or(old.budget);
        p.permissions = p.permissions.or(old.permissions);
        p.setup = p.setup.or(old.setup);
    }
    write(store, &p)?;
    Ok(p)
}

/// "Edit…": identity only (name, summary, icon, color, scope). `null`
/// clears summary/icon/color; scope `"global"` or `null` makes it global.
pub fn edit(
    store: &CanvasStore,
    id: &str,
    patch: serde_json::Map<String, Value>,
) -> Result<LibraryPartitura, String> {
    if patch
        .get("restore")
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("previous"))
    {
        return restore_previous(store, id);
    }
    let mut p = get(store, id)?;
    for (k, v) in patch {
        let s = v.as_str().map(str::to_string);
        match k.as_str() {
            "name" => {
                let n = s.unwrap_or_default();
                if n.trim().is_empty() {
                    return Err("CANVAS_INVALID: partitura name is empty".into());
                }
                p.doc.name = n.trim().to_string();
            }
            "summary" => p.doc.summary = s,
            "icon" => p.doc.icon = s,
            "color" => p.doc.color = s,
            "scope" | "scopeWorkspaceId" => {
                p.scope_workspace_id = s.filter(|w| w != "global" && !w.is_empty());
            }
            "budget" => p.budget = serde_json::from_value(v).ok(),
            "permissions" => p.permissions = serde_json::from_value(v).ok(),
            "setup" => {
                p.setup = serde_json::from_value::<SetupSpec>(v)
                    .ok()
                    .filter(|s| !s.is_empty())
            }
            _ => {}
        }
    }
    p.updated_at = Some(now_rfc3339());
    write(store, &p)?;
    Ok(p)
}

/// Deleting never touches what is already on a canvas.
pub fn delete(store: &CanvasStore, id: &str) -> Result<(), String> {
    let f = file_of(store, id)?;
    if !f.exists() {
        return Err(format!("CANVAS_NOT_FOUND: partitura {id}"));
    }
    if let Ok(h) = history_dir(store, id) {
        let _ = std::fs::remove_dir_all(h);
    }
    std::fs::remove_file(f).map_err(|e| format!("CANVAS_IO: {e}"))
}

// ---------------------------------------------------------------- files

/// Parses one exported partitura or a bundle.
pub fn parse_file(bytes: &[u8]) -> Result<Vec<LibraryPartitura>, String> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("CANVAS_INVALID: not JSON: {e}"))?;
    match v.get("format").and_then(|f| f.as_str()) {
        Some(BUNDLE_FORMAT) => {
            let b: Bundle =
                serde_json::from_value(v).map_err(|e| format!("CANVAS_INVALID: bundle: {e}"))?;
            Ok(b.partituras)
        }
        Some(PARTITURA_FORMAT) => Ok(vec![
            serde_json::from_value(v).map_err(|e| format!("CANVAS_INVALID: partitura: {e}"))?
        ]),
        other => Err(format!(
            "CANVAS_INVALID: not a partitura file ({})",
            other.unwrap_or("no format")
        )),
    }
}

pub fn read_file(path: &Path) -> Result<Vec<LibraryPartitura>, String> {
    let b = std::fs::read(path).map_err(|e| format!("CANVAS_IO: {}: {e}", path.display()))?;
    parse_file(&b)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LibraryImport {
    pub added: Vec<String>,
    /// Already in the library (reimporting is a no-op).
    pub skipped: Vec<String>,
}

/// Library transfer (files or bundles): one consent, existing ids skipped.
/// A partitura whose workspace is not here becomes global.
pub fn add_to_library(
    store: &CanvasStore,
    items: Vec<LibraryPartitura>,
) -> Result<LibraryImport, String> {
    let ws: HashSet<String> = store
        .workspaces()
        .into_iter()
        .map(|w| w.workspace.id)
        .collect();
    let mut out = LibraryImport::default();
    for mut p in items {
        if !is_safe_id(&p.doc.id) {
            p.doc.id = new_id();
        }
        if file_of(store, &p.doc.id)?.exists() {
            out.skipped.push(p.doc.id);
            continue;
        }
        if p.scope_workspace_id
            .as_ref()
            .is_some_and(|w| !ws.contains(w))
        {
            p.scope_workspace_id = None;
        }
        write(store, &p)?;
        out.added.push(p.doc.id);
    }
    Ok(out)
}

/// Writes one partitura (or a bundle for several) to `path`.
pub fn export_file(store: &CanvasStore, ids: &[String], path: &Path) -> Result<(), String> {
    let items: Vec<LibraryPartitura> = ids
        .iter()
        .map(|id| get(store, id))
        .collect::<Result<_, _>>()?;
    let bytes = if items.len() == 1 {
        serde_json::to_vec_pretty(&items[0])
    } else {
        serde_json::to_vec_pretty(&Bundle {
            format: BUNDLE_FORMAT.into(),
            version: 1,
            partituras: items,
        })
    }
    .map_err(|e| e.to_string())?;
    persist::write_atomic(path, &bytes)
}

// ---------------------------------------------------------------- roles

fn embedded(r: &super::roles::Role) -> EmbeddedRole {
    EmbeddedRole {
        id: r.id.clone(),
        name: r.name.clone(),
        prompt: r.prompt.clone(),
        icon: r.icon.clone(),
        color: r.color.clone(),
    }
}

/// Global roles folder: the roles library (`<app_data>/canvas/roles-library`).
pub fn global_roles_dir(store: &CanvasStore) -> PathBuf {
    super::roles::library_dir(store)
}

/// Roles visible from a workspace: its own first, then the library ones
/// (same storage and precedence as `roles::list`).
pub fn local_roles(store: &CanvasStore, ws: &str) -> Vec<EmbeddedRole> {
    super::roles::list(store, ws)
        .iter()
        .map(|i| embedded(&i.role))
        .collect()
}

/// Writes a role through the roles API: a library role is updated in the
/// library, anything else lands in the workspace (`<ws>/roles/<id>/`).
pub fn write_role(store: &CanvasStore, ws: &str, role: &EmbeddedRole) -> Result<(), String> {
    if !is_safe_id(&role.id) {
        return Err(format!("CANVAS_INVALID: role id '{}'", role.id));
    }
    let existing = super::roles::get(store, ws, &role.id);
    let target = match existing.as_ref().map(|(_, s)| *s) {
        Some(super::roles::Scope::Global) => None,
        _ => Some(ws),
    };
    let (summary, agent) = existing
        .map(|(r, _)| (r.summary, r.agent))
        .unwrap_or((None, None));
    super::roles::save(
        store,
        target,
        super::roles::RoleInput {
            id: Some(role.id.clone()),
            name: role.name.clone(),
            prompt: role.prompt.clone(),
            summary,
            icon: role.icon.clone(),
            color: role.color.clone(),
            agent,
        },
    )
    .map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoleAction {
    /// Use the local role (the partitura's terminals point to it).
    KeepMine,
    /// The local role takes the partitura's prompt, icon and color.
    UseTheirs,
    /// Keep both: the partitura's role is added under a new name.
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RolePlan {
    pub role: EmbeddedRole,
    /// `new` | `same` (identical local role) | `conflict`
    pub status: String,
    /// Matched by `id` or `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<EmbeddedRole>,
    pub default_action: RoleAction,
}

fn same_role(a: &EmbeddedRole, b: &EmbeddedRole) -> bool {
    a.prompt.trim() == b.prompt.trim() && a.icon == b.icon && a.color == b.color
}

pub fn plan_roles(local: &[EmbeddedRole], theirs: &[EmbeddedRole]) -> Vec<RolePlan> {
    theirs
        .iter()
        .map(|r| {
            let by_id = local.iter().find(|l| l.id == r.id);
            let by_name = local.iter().find(|l| l.name.eq_ignore_ascii_case(&r.name));
            let (m, how) = match (by_id, by_name) {
                (Some(l), _) => (Some(l), "id"),
                (None, Some(l)) => (Some(l), "name"),
                _ => (None, ""),
            };
            match m {
                None => RolePlan {
                    role: r.clone(),
                    status: "new".into(),
                    matched_by: None,
                    local: None,
                    default_action: RoleAction::UseTheirs,
                },
                Some(l) => RolePlan {
                    role: r.clone(),
                    status: if same_role(l, r) { "same" } else { "conflict" }.into(),
                    matched_by: Some(how.into()),
                    local: Some(l.clone()),
                    default_action: RoleAction::KeepMine,
                },
            }
        })
        .collect()
}

fn unique_role_name(local: &[EmbeddedRole], base: &str) -> String {
    let taken: HashSet<String> = local.iter().map(|r| r.name.to_lowercase()).collect();
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !taken.contains(&n.to_lowercase()))
        .expect("infinite")
}

/// Applies the choices and returns `partitura role id -> local role id`.
pub fn resolve_roles(
    store: &CanvasStore,
    ws: &str,
    theirs: &[EmbeddedRole],
    actions: &HashMap<String, RoleAction>,
) -> Result<HashMap<String, String>, String> {
    let mut local = local_roles(store, ws);
    let mut map = HashMap::new();
    for plan in plan_roles(&local.clone(), theirs) {
        let r = &plan.role;
        let action = actions.get(&r.id).copied().unwrap_or(plan.default_action);
        match (&plan.local, action) {
            (None, _) => {
                let mut role = r.clone();
                if !is_safe_id(&role.id) || local.iter().any(|l| l.id == role.id) {
                    role.id = new_id();
                }
                write_role(store, ws, &role)?;
                map.insert(r.id.clone(), role.id.clone());
                local.push(role);
            }
            (Some(l), RoleAction::KeepMine) => {
                map.insert(r.id.clone(), l.id.clone());
            }
            (Some(l), RoleAction::UseTheirs) => {
                let role = EmbeddedRole {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    prompt: r.prompt.clone(),
                    icon: r.icon.clone(),
                    color: r.color.clone(),
                };
                write_role(store, ws, &role)?;
                map.insert(r.id.clone(), l.id.clone());
            }
            (Some(_), RoleAction::Rename) => {
                let role = EmbeddedRole {
                    id: new_id(),
                    name: unique_role_name(&local, &r.name),
                    ..r.clone()
                };
                write_role(store, ws, &role)?;
                map.insert(r.id.clone(), role.id.clone());
                local.push(role);
            }
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------- import review

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandLine {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub agent: String,
    /// What will run (`None` = the agent's default command or a shell).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub partitura: PartituraSummary,
    pub commands: Vec<CommandLine>,
    pub roles: Vec<RolePlan>,
    pub portals: Vec<String>,
    pub note_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    /// Already in the library with the same id (adding would replace it).
    pub replaces_existing: bool,
    pub warnings: Vec<String>,
    /// Terminals whose chat history travels with it (format 2); copied only
    /// when the stamp opts in (`acceptChat`).
    #[serde(default)]
    pub chat_transcripts: usize,
}

pub fn preview(store: &CanvasStore, ws: &str, p: &LibraryPartitura) -> ImportPreview {
    let ws_names: HashMap<String, String> = store
        .workspaces()
        .into_iter()
        .map(|w| (w.workspace.id, w.workspace.name))
        .collect();
    let d = &p.doc;
    let role_names: HashMap<&str, &str> = d
        .roles
        .iter()
        .map(|r| (r.id.as_str(), r.name.as_str()))
        .collect();
    let commands = d
        .nodes
        .iter()
        .filter(|n| n.is_terminal())
        .map(|n| CommandLine {
            node_id: n.id.clone(),
            title: n.title.clone(),
            agent: n.data_str("agent").unwrap_or("shell").to_string(),
            command: n.data_str("command").map(str::to_string),
            cwd: n.data_str("cwd").map(str::to_string),
            role_name: n
                .data_str("roleId")
                .and_then(|r| role_names.get(r).map(|s| s.to_string())),
        })
        .collect();
    let portals = d
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Portal)
        .filter_map(|n| n.data_str("url").map(str::to_string))
        .collect();
    let mut warnings = vec!["Adding a partitura to a canvas starts its terminals and runs their commands on your machine.".to_string()];
    if p.permissions.as_ref().is_some_and(|x| {
        x.mode.as_deref() == Some("full")
            || x.allow_raw == Some(true)
            || x.allow_evaluate == Some(true)
    }) {
        warnings
            .push("This partitura asks for wide permissions: review them before adding.".into());
    }
    if p.setup.as_ref().is_some_and(|s| !s.is_empty()) {
        warnings.push(
            "It brings floor hooks (setup / run / teardown scripts) that run on your machine."
                .into(),
        );
    }
    ImportPreview {
        partitura: summarize(p, &ws_names),
        commands,
        roles: plan_roles(&local_roles(store, ws), &d.roles),
        portals,
        note_count: d.nodes.iter().filter(|n| n.kind == NodeKind::Note).count(),
        setup: p.setup.clone(),
        budget: p.budget.clone(),
        permissions: p.permissions.clone(),
        replaces_existing: file_of(store, &d.id).map(|f| f.exists()).unwrap_or(false),
        warnings,
        chat_transcripts: p.chat.len(),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StampOptions {
    /// Per node: edited command, or `null` to run the agent's default.
    #[serde(default)]
    pub commands: HashMap<String, Option<String>>,
    #[serde(default)]
    pub roles: HashMap<String, RoleAction>,
    /// Append the partitura's setup/run/teardown to the workspace's floor hooks.
    #[serde(default)]
    pub apply_hooks: bool,
    #[serde(default)]
    pub origin: Option<String>,
    /// Copy a format-2 partitura's chat history into the new terminals.
    #[serde(default)]
    pub accept_chat: bool,
}

fn append_hook(list: &mut Vec<HookCommand>, cmd: &Option<String>, name: &str) {
    let Some(c) = cmd.as_deref().map(str::trim).filter(|c| !c.is_empty()) else {
        return;
    };
    if list.iter().any(|h| h.command.trim() == c) {
        return;
    }
    list.push(HookCommand {
        id: new_id(),
        command: c.to_string(),
        name: Some(name.to_string()),
        enabled: true,
    });
}

/// Stamps a partitura on a floor after the review: commands edited, roles
/// merged, terminals on an isolated floor moved into its folder.
pub fn stamp(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    p: &LibraryPartitura,
    at: Point,
    opts: &StampOptions,
) -> Result<ImportResult, String> {
    let mut doc = p.doc.clone();
    if doc.version > CHAT_FORMAT_VERSION {
        return Err("CANVAS_INVALID: partitura made by a newer OmniGet".into());
    }
    // Format 2 only adds the chat, which is landed here.
    doc.version = doc.version.min(export::PARTITURA_VERSION);
    apply_commands(&mut doc, &opts.commands);
    let role_map = resolve_roles(store, ws, &doc.roles, &opts.roles)?;
    let origin = opts.origin.clone().unwrap_or_else(|| "user".into());
    let result = export::import_partitura(store, ws, floor, &doc, at, &origin, &role_map)?;
    if opts.accept_chat && !p.chat.is_empty() {
        land_chat(store, ws, p, &result.node_ids)?;
    }
    if floor != GROUND_FLOOR_ID {
        let ids: HashSet<&str> = result.node_ids.iter().map(String::as_str).collect();
        let ops: Vec<Op> = store.read(ws, floor, |d| {
            d.nodes
                .iter()
                .filter(|n| ids.contains(n.id.as_str()) && n.is_terminal())
                .filter_map(|n| {
                    let cwd = floors::remap_cwd(store, ws, floor, n.data_str("cwd"))?;
                    (Some(cwd.as_str()) != n.data_str("cwd"))
                        .then(|| Op::update_node(n.id.clone(), json!({ "data": { "cwd": cwd } })))
                })
                .collect()
        })?;
        if !ops.is_empty() {
            store.apply(ws, floor, ops, &origin, None)?;
        }
    }
    if opts.apply_hooks {
        if let Some(s) = &p.setup {
            let mut hooks: FloorHooks = floors::load_config(store, ws).hooks;
            append_hook(&mut hooks.setup, &s.setup, &p.doc.name);
            append_hook(&mut hooks.run, &s.run, &p.doc.name);
            append_hook(&mut hooks.teardown, &s.teardown, &p.doc.name);
            floors::set_hooks(store, ws, hooks)?;
        }
    }
    Ok(result)
}

/// Loads a library partitura or takes one handed in (a file being reviewed).
pub fn resolve(
    store: &CanvasStore,
    id: Option<&str>,
    inline: Option<LibraryPartitura>,
) -> Result<LibraryPartitura, String> {
    match (inline, id) {
        (Some(p), _) => Ok(p),
        (None, Some(id)) => get(store, id),
        _ => Err("CANVAS_INVALID: give a partitura id or a partitura".into()),
    }
}
