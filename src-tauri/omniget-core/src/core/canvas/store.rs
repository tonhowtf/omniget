use super::model::{
    is_safe_id, new_id, now_rfc3339, CanvasNode, FloorDoc, FloorMeta, NodeKind, NoteData, Snapshot,
    Workspace, WorkspaceInput, WorkspaceSummary, DEFAULT_WORKSPACE_ID, GROUND_FLOOR_ID,
};
use super::ops::{apply_batch, Op};
use super::persist::{self, SaveKey, Saver, CAMERA_DEBOUNCE, SAVE_DEBOUNCE};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime};

pub const ORIGIN_USER: &str = "user";
pub const ORIGIN_SYSTEM: &str = "system";

pub fn agent_origin(node_id: &str) -> String {
    format!("agent:{node_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub epoch: String,
    pub rev: u64,
    pub seq: u64,
    pub ops: Vec<Op>,
    pub inverse: Vec<Op>,
    pub rebased: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchEvent {
    pub workspace_id: String,
    pub floor_id: String,
    pub epoch: String,
    pub rev: u64,
    pub seq: u64,
    pub ops: Vec<Op>,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteEvent {
    pub workspace_id: String,
    pub node_id: String,
    pub revision: String,
    pub origin: String,
}

#[derive(Debug, Clone)]
pub enum StoreEvent {
    Patch(PatchEvent),
    Workspaces,
    Note(NoteEvent),
}

type Listener = Arc<dyn Fn(&StoreEvent) + Send + Sync>;

pub(super) struct FloorState {
    pub doc: FloorDoc,
    pub seq: u64,
    pub read_only: bool,
}

struct WsEntry {
    meta: Workspace,
    read_only: bool,
    floors: HashMap<String, FloorState>,
}

#[derive(Default)]
struct Inner {
    loaded: bool,
    workspaces: BTreeMap<String, WsEntry>,
}

pub(super) struct WatchEntry {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub revision: String,
    pub last_read: Instant,
}

pub struct CanvasStore {
    root: PathBuf,
    inner: Mutex<Inner>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next_listener: Mutex<u64>,
    saver: OnceLock<Saver>,
    io: Mutex<()>,
    pub(super) note_watch: Mutex<HashMap<(String, String), WatchEntry>>,
    history: Mutex<HashMap<HistoryKey, std::collections::VecDeque<Vec<Op>>>>,
}

type HistoryKey = (String, String, String);

const HISTORY_DEPTH: usize = 50;
pub const UNDO_SUFFIX: &str = ":undo";

static SHARED: OnceLock<Arc<CanvasStore>> = OnceLock::new();

pub fn shared() -> Arc<CanvasStore> {
    SHARED
        .get_or_init(|| CanvasStore::open(persist::canvas_root()))
        .clone()
}

fn epoch() -> String {
    new_id().replace('-', "")[..16].to_string()
}

fn not_found(what: impl std::fmt::Display) -> String {
    format!("CANVAS_NOT_FOUND: {what}")
}

impl CanvasStore {
    pub fn open(root: PathBuf) -> Arc<Self> {
        let store = Arc::new(Self {
            root,
            inner: Mutex::new(Inner::default()),
            listeners: Mutex::new(Vec::new()),
            next_listener: Mutex::new(1),
            saver: OnceLock::new(),
            io: Mutex::new(()),
            note_watch: Mutex::new(HashMap::new()),
            history: Mutex::new(HashMap::new()),
        });
        let weak: Weak<CanvasStore> = Arc::downgrade(&store);
        let _ = store
            .saver
            .set(Saver::spawn(Box::new(move |key| match weak.upgrade() {
                Some(s) => {
                    if let Err(e) = s.save_key(key) {
                        tracing::warn!("[canvas] save {key:?} failed: {e}");
                    }
                    true
                }
                None => false,
            })));
        store
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace_dir(&self, ws: &str) -> PathBuf {
        persist::workspace_dir(&self.root, ws)
    }

    pub fn notes_dir(&self, ws: &str) -> PathBuf {
        persist::notes_dir(&self.root, ws)
    }

    pub fn roles_dir(&self, ws: &str) -> PathBuf {
        persist::roles_dir(&self.root, ws)
    }

    pub fn inbox_dir(&self, ws: &str, node_id: &str) -> PathBuf {
        persist::inbox_dir(&self.root, ws, node_id)
    }

    pub fn subscribe(&self, f: impl Fn(&StoreEvent) + Send + Sync + 'static) -> u64 {
        let mut n = self.next_listener.lock().unwrap_or_else(|e| e.into_inner());
        let id = *n;
        *n += 1;
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, Arc::new(f)));
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(i, _)| *i != id);
    }

    pub(super) fn emit(&self, ev: StoreEvent) {
        let ls: Vec<Listener> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for l in ls {
            l(&ev);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.loaded {
            g.loaded = true;
            for id in persist::list_workspace_ids(&self.root) {
                match persist::load_workspace(&self.root, &id) {
                    Ok(Some(l)) => {
                        let mut meta = l.value;
                        if meta.id != id {
                            tracing::warn!(
                                "[canvas] workspace folder {id} holds id {}, using the folder",
                                meta.id
                            );
                            meta.id = id.clone();
                        }
                        if meta.floors.iter().all(|f| f.id != GROUND_FLOOR_ID) {
                            meta.floors.insert(
                                0,
                                Workspace::new(id.clone(), String::new()).floors.remove(0),
                            );
                        }
                        persist::purge_trash(&self.root, &id);
                        g.workspaces.insert(
                            id,
                            WsEntry {
                                meta,
                                read_only: l.read_only,
                                floors: HashMap::new(),
                            },
                        );
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("[canvas] workspace {id} unreadable: {e}"),
                }
            }
            if g.workspaces.is_empty() {
                let mut meta = Workspace::new(DEFAULT_WORKSPACE_ID.into(), "Home".into());
                meta.icon = Some("house".into());
                meta.sort_index = Some(0);
                if let Err(e) = persist::save_workspace(&self.root, &meta) {
                    tracing::warn!("[canvas] cannot create the Home workspace: {e}");
                }
                g.workspaces.insert(
                    meta.id.clone(),
                    WsEntry {
                        meta,
                        read_only: false,
                        floors: HashMap::new(),
                    },
                );
            }
        }
        g
    }

    fn ensure_floor<'a>(
        &self,
        g: &'a mut Inner,
        ws: &str,
        floor: &str,
    ) -> Result<&'a mut FloorState, String> {
        let root = self.root.clone();
        let entry = g
            .workspaces
            .get_mut(ws)
            .ok_or_else(|| not_found(format!("workspace {ws}")))?;
        if !entry.meta.has_floor(floor) {
            return Err(not_found(format!("floor {floor} in workspace {ws}")));
        }
        if !entry.floors.contains_key(floor) {
            let (doc, read_only) = match persist::load_floor(&root, ws, floor)? {
                Some(l) => (l.value, l.read_only),
                None => (FloorDoc::default(), false),
            };
            let mut doc = doc;
            doc.epoch = epoch();
            entry.floors.insert(
                floor.to_string(),
                FloorState {
                    doc,
                    seq: 0,
                    read_only: read_only || entry.read_only,
                },
            );
        }
        Ok(entry.floors.get_mut(floor).expect("inserted"))
    }

    pub fn workspaces(&self) -> Vec<WorkspaceSummary> {
        let mut g = self.lock();
        let ids: Vec<(String, Vec<String>)> = g
            .workspaces
            .values()
            .map(|e| {
                (
                    e.meta.id.clone(),
                    e.meta.floors.iter().map(|f| f.id.clone()).collect(),
                )
            })
            .collect();
        let mut out = Vec::new();
        for (ws, floors) in ids {
            let (mut nodes, mut terms) = (0, 0);
            let mut terminals = Vec::new();
            for f in floors {
                if let Ok(st) = self.ensure_floor(&mut g, &ws, &f) {
                    nodes += st.doc.nodes.len();
                    for n in st.doc.nodes.iter().filter(|n| n.is_terminal()) {
                        terms += 1;
                        terminals.push(super::model::WorkspaceTerminal {
                            node_id: n.id.clone(),
                            floor_id: f.clone(),
                            title: n.title.clone(),
                            terminal_id: n.data_str("terminalId").map(str::to_string),
                            agent: n.data_str("agent").map(str::to_string),
                            running: false,
                        });
                    }
                }
            }
            let meta = g.workspaces[&ws].meta.clone();
            out.push(WorkspaceSummary {
                workspace: meta,
                node_count: nodes,
                terminal_count: terms,
                terminals,
                loaded: None,
                icon_revision: None,
                agent_files: None,
            });
        }
        out.sort_by(|a, b| {
            let ka = (
                a.workspace.sort_index.unwrap_or(i64::MAX),
                &a.workspace.created_at,
            );
            let kb = (
                b.workspace.sort_index.unwrap_or(i64::MAX),
                &b.workspace.created_at,
            );
            ka.cmp(&kb)
        });
        drop(g);
        for s in out.iter_mut() {
            super::model::enrich_summary(s);
        }
        out
    }

    pub fn workspace(&self, ws: &str) -> Result<Workspace, String> {
        self.lock()
            .workspaces
            .get(ws)
            .map(|e| e.meta.clone())
            .ok_or_else(|| not_found(format!("workspace {ws}")))
    }

    pub fn create_workspace(&self, input: WorkspaceInput) -> Result<Workspace, String> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err("CANVAS_INVALID: workspace name is empty".into());
        }
        let id = input.id.unwrap_or_else(new_id);
        if !is_safe_id(&id) {
            return Err(format!("CANVAS_INVALID: workspace id '{id}'"));
        }
        let meta = {
            let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = self.lock();
            if g.workspaces.contains_key(&id) {
                return Err(format!("CANVAS_CONFLICT: workspace {id} exists"));
            }
            let mut meta = Workspace::new(id.clone(), name);
            meta.icon = input.icon;
            meta.color = input.color;
            meta.working_directory = input.working_directory;
            meta.environment = input.environment;
            meta.sort_index = Some(g.workspaces.len() as i64);
            persist::save_workspace(&self.root, &meta)?;
            g.workspaces.insert(
                id,
                WsEntry {
                    meta: meta.clone(),
                    read_only: false,
                    floors: HashMap::new(),
                },
            );
            meta
        };
        self.emit(StoreEvent::Workspaces);
        Ok(meta)
    }

    pub fn update_workspace(
        &self,
        ws: &str,
        patch: Map<String, Value>,
    ) -> Result<Workspace, String> {
        let meta = {
            let mut g = self.lock();
            let e = g
                .workspaces
                .get_mut(ws)
                .ok_or_else(|| not_found(format!("workspace {ws}")))?;
            if e.read_only {
                return Err("CANVAS_READ_ONLY: workspace was written by a newer OmniGet".into());
            }
            let mut obj = match serde_json::to_value(&e.meta).map_err(|e| e.to_string())? {
                Value::Object(m) => m,
                _ => unreachable!(),
            };
            for (k, v) in patch {
                if matches!(
                    k.as_str(),
                    "id" | "floors" | "createdAt" | "format" | "version"
                ) {
                    continue;
                }
                if v.is_null() {
                    obj.remove(&k);
                } else {
                    obj.insert(k, v);
                }
            }
            let mut next: Workspace = serde_json::from_value(Value::Object(obj))
                .map_err(|e| format!("CANVAS_INVALID: workspace patch: {e}"))?;
            if next.name.trim().is_empty() {
                return Err("CANVAS_INVALID: workspace name is empty".into());
            }
            if !next.has_floor(&next.active_floor_id) {
                return Err(not_found(format!("floor {}", next.active_floor_id)));
            }
            next.updated_at = now_rfc3339();
            e.meta = next.clone();
            next
        };
        self.schedule(SaveKey::Workspace(ws.to_string()), Duration::ZERO);
        self.emit(StoreEvent::Workspaces);
        Ok(meta)
    }

    pub fn delete_workspace(&self, ws: &str) -> Result<(), String> {
        if self.workspace(ws).is_ok() {
            super::workspace_io::before(self, ws, "delete");
        }
        {
            let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = self.lock();
            if g.workspaces.remove(ws).is_none() {
                return Err(not_found(format!("workspace {ws}")));
            }
            let src = self.workspace_dir(ws);
            if src.exists() {
                let dest_dir = self.root.join(".deleted");
                std::fs::create_dir_all(&dest_dir).map_err(|e| format!("CANVAS_IO: {e}"))?;
                let dest = dest_dir.join(format!(
                    "{ws}-{}",
                    chrono::Utc::now().format("%Y%m%d%H%M%S")
                ));
                std::fs::rename(&src, &dest).map_err(|e| format!("CANVAS_IO: {e}"))?;
            }
            self.note_watch
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(w, _), _| w != ws);
        }
        self.emit(StoreEvent::Workspaces);
        Ok(())
    }

    pub fn create_floor(&self, ws: &str, mut floor: FloorMeta) -> Result<Workspace, String> {
        if !is_safe_id(&floor.id) {
            return Err(format!("CANVAS_INVALID: floor id '{}'", floor.id));
        }
        if floor.created_at.is_empty() {
            floor.created_at = now_rfc3339();
        }
        let meta = {
            let mut g = self.lock();
            let e = g
                .workspaces
                .get_mut(ws)
                .ok_or_else(|| not_found(format!("workspace {ws}")))?;
            if e.meta.has_floor(&floor.id) {
                return Err(format!("CANVAS_CONFLICT: floor {} exists", floor.id));
            }
            e.meta.floors.push(floor);
            e.meta.updated_at = now_rfc3339();
            e.meta.clone()
        };
        self.schedule(SaveKey::Workspace(ws.to_string()), Duration::ZERO);
        self.emit(StoreEvent::Workspaces);
        Ok(meta)
    }

    pub fn update_floor(
        &self,
        ws: &str,
        floor_id: &str,
        patch: Map<String, Value>,
    ) -> Result<Workspace, String> {
        let meta = {
            let mut g = self.lock();
            let e = g
                .workspaces
                .get_mut(ws)
                .ok_or_else(|| not_found(format!("workspace {ws}")))?;
            let f = e
                .meta
                .floors
                .iter_mut()
                .find(|f| f.id == floor_id)
                .ok_or_else(|| not_found(format!("floor {floor_id}")))?;
            let mut obj = match serde_json::to_value(&*f).map_err(|e| e.to_string())? {
                Value::Object(m) => m,
                _ => unreachable!(),
            };
            for (k, v) in patch {
                if k == "id" {
                    continue;
                }
                if v.is_null() {
                    obj.remove(&k);
                } else {
                    obj.insert(k, v);
                }
            }
            *f = serde_json::from_value(Value::Object(obj))
                .map_err(|e| format!("CANVAS_INVALID: floor patch: {e}"))?;
            e.meta.updated_at = now_rfc3339();
            e.meta.clone()
        };
        self.schedule(SaveKey::Workspace(ws.to_string()), Duration::ZERO);
        self.emit(StoreEvent::Workspaces);
        Ok(meta)
    }

    pub fn delete_floor(&self, ws: &str, floor_id: &str) -> Result<Workspace, String> {
        if floor_id == GROUND_FLOOR_ID {
            return Err("CANVAS_INVALID: the ground floor cannot be deleted".into());
        }
        let meta = {
            let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = self.lock();
            let e = g
                .workspaces
                .get_mut(ws)
                .ok_or_else(|| not_found(format!("workspace {ws}")))?;
            let before = e.meta.floors.len();
            e.meta.floors.retain(|f| f.id != floor_id);
            if e.meta.floors.len() == before {
                return Err(not_found(format!("floor {floor_id}")));
            }
            e.floors.remove(floor_id);
            if e.meta.active_floor_id == floor_id {
                e.meta.active_floor_id = GROUND_FLOOR_ID.into();
            }
            e.meta.updated_at = now_rfc3339();
            let file = persist::floor_file(&self.root, ws, floor_id);
            if file.exists() {
                let _ = persist::move_to_trash(&self.root, ws, &file);
            }
            e.meta.clone()
        };
        self.schedule(SaveKey::Workspace(ws.to_string()), Duration::ZERO);
        self.emit(StoreEvent::Workspaces);
        Ok(meta)
    }

    pub fn snapshot(&self, ws: &str, floor: &str) -> Result<Snapshot, String> {
        let mut g = self.lock();
        let st = self.ensure_floor(&mut g, ws, floor)?;
        let d = &st.doc;
        Ok(Snapshot {
            workspace_id: ws.to_string(),
            floor_id: floor.to_string(),
            epoch: d.epoch.clone(),
            rev: d.rev,
            seq: st.seq,
            camera: d.camera,
            nodes: d.nodes.clone(),
            connections: d.connections.clone(),
            drawings: d.drawings.clone(),
            groups: d.groups.clone(),
            cable_ties: d.cable_ties.clone(),
        })
    }

    pub fn read<R>(
        &self,
        ws: &str,
        floor: &str,
        f: impl FnOnce(&FloorDoc) -> R,
    ) -> Result<R, String> {
        let mut g = self.lock();
        let st = self.ensure_floor(&mut g, ws, floor)?;
        Ok(f(&st.doc))
    }

    pub fn locate_node(&self, node_id: &str) -> Option<(String, String)> {
        self.locate(|d| d.node(node_id).is_some())
    }

    pub fn locate_terminal(&self, terminal_id: &str) -> Option<(String, String, String)> {
        let (ws, floor) = self.locate(|d| d.node_by_terminal(terminal_id).is_some())?;
        let node = self
            .read(&ws, &floor, |d| {
                d.node_by_terminal(terminal_id).map(|n| n.id.clone())
            })
            .ok()??;
        Some((ws, floor, node))
    }

    fn locate(&self, pred: impl Fn(&FloorDoc) -> bool) -> Option<(String, String)> {
        let mut g = self.lock();
        let pairs: Vec<(String, String)> = g
            .workspaces
            .values()
            .flat_map(|e| {
                e.meta
                    .floors
                    .iter()
                    .map(move |f| (e.meta.id.clone(), f.id.clone()))
            })
            .collect();
        for (ws, floor) in pairs {
            if let Ok(st) = self.ensure_floor(&mut g, &ws, &floor) {
                if pred(&st.doc) {
                    return Some((ws, floor));
                }
            }
        }
        None
    }

    pub fn apply(
        &self,
        ws: &str,
        floor: &str,
        ops: Vec<Op>,
        origin: &str,
        base_rev: Option<u64>,
    ) -> Result<ApplyResult, String> {
        let origin = origin.trim();
        if origin.is_empty() || origin.len() > 128 {
            return Err("CANVAS_INVALID: origin".into());
        }
        if ops.is_empty() {
            let mut g = self.lock();
            let st = self.ensure_floor(&mut g, ws, floor)?;
            return Ok(ApplyResult {
                epoch: st.doc.epoch.clone(),
                rev: st.doc.rev,
                seq: st.seq,
                ops: Vec::new(),
                inverse: Vec::new(),
                rebased: false,
            });
        }
        let ops = self.prepare(ops);
        let (result, removed_notes, added_notes, camera_only, structural) = {
            let mut g = self.lock();
            let st = self.ensure_floor(&mut g, ws, floor)?;
            if st.read_only {
                return Err("CANVAS_READ_ONLY: floor was written by a newer OmniGet".into());
            }
            let rebased = base_rev.is_some_and(|b| b < st.doc.rev);
            let applied = apply_batch(&mut st.doc, &ops)?;
            st.doc.rev += 1;
            st.seq += 1;
            let removed_ids: Vec<&str> = applied
                .ops
                .iter()
                .filter_map(|o| match o {
                    Op::RemoveNode { id } => Some(id.as_str()),
                    _ => None,
                })
                .collect();
            let removed_notes: Vec<String> = applied
                .inverse
                .iter()
                .filter_map(|o| match o {
                    Op::AddNode { node }
                        if node.kind == NodeKind::Note
                            && removed_ids.contains(&node.id.as_str()) =>
                    {
                        internal_note_file(node)
                    }
                    _ => None,
                })
                .collect();
            let added_notes: Vec<String> = applied
                .ops
                .iter()
                .filter_map(|o| match o {
                    Op::AddNode { node } if node.kind == NodeKind::Note => internal_note_file(node),
                    _ => None,
                })
                .collect();
            let camera_only = applied.ops.iter().all(Op::is_camera);
            let structural = applied.ops.iter().any(Op::is_structural);
            (
                ApplyResult {
                    epoch: st.doc.epoch.clone(),
                    rev: st.doc.rev,
                    seq: st.seq,
                    ops: applied.ops,
                    inverse: applied.inverse,
                    rebased,
                },
                removed_notes,
                added_notes,
                camera_only,
                structural,
            )
        };

        let notes = self.notes_dir(ws);
        let trash = persist::trash_dir(&self.root, ws);
        for f in &removed_notes {
            let p = notes.join(f);
            if p.exists() && !added_notes.contains(f) {
                if let Err(e) = persist::move_to_trash(&self.root, ws, &p) {
                    tracing::warn!("[canvas] {e}");
                }
            }
        }
        for f in &added_notes {
            let p = notes.join(f);
            if p.exists() {
                continue;
            }
            let _ = std::fs::create_dir_all(&notes);
            let t = trash.join(f);
            let restored = t.exists() && std::fs::rename(&t, &p).is_ok();
            if !restored {
                if let Err(e) = persist::write_atomic(&p, b"") {
                    tracing::warn!("[canvas] note file {f}: {e}");
                }
            }
        }

        let key = SaveKey::Floor(ws.to_string(), floor.to_string());
        let delay = if structural {
            Duration::ZERO
        } else if camera_only {
            CAMERA_DEBOUNCE
        } else {
            SAVE_DEBOUNCE
        };
        self.schedule(key, delay);
        if !camera_only && !origin.ends_with(UNDO_SUFFIX) && !result.inverse.is_empty() {
            if let Ok(mut h) = self.history.lock() {
                let q = h
                    .entry((ws.to_string(), floor.to_string(), origin.to_string()))
                    .or_default();
                q.push_back(result.inverse.clone());
                while q.len() > HISTORY_DEPTH {
                    q.pop_front();
                }
            }
        }
        self.emit(StoreEvent::Patch(PatchEvent {
            workspace_id: ws.to_string(),
            floor_id: floor.to_string(),
            epoch: result.epoch.clone(),
            rev: result.rev,
            seq: result.seq,
            ops: result.ops.clone(),
            origin: origin.to_string(),
        }));
        Ok(result)
    }

    pub fn undo_last(
        &self,
        ws: &str,
        floor: &str,
        origin: &str,
    ) -> Result<Option<ApplyResult>, String> {
        let inverse = {
            let mut h = self.history.lock().map_err(|_| "CANVAS_BUSY")?;
            h.get_mut(&(ws.to_string(), floor.to_string(), origin.to_string()))
                .and_then(|q| q.pop_back())
        };
        let Some(ops) = inverse else { return Ok(None) };
        self.apply(ws, floor, ops, &format!("{origin}{UNDO_SUFFIX}"), None)
            .map(Some)
            .map_err(|e| format!("CANVAS_UNDO_STALE: the canvas changed since that edit ({e})"))
    }

    pub fn undo_depth(&self, ws: &str, floor: &str, origin: &str) -> usize {
        self.history
            .lock()
            .ok()
            .and_then(|h| {
                h.get(&(ws.to_string(), floor.to_string(), origin.to_string()))
                    .map(|q| q.len())
            })
            .unwrap_or(0)
    }

    fn prepare(&self, mut ops: Vec<Op>) -> Vec<Op> {
        for op in &mut ops {
            let Op::AddNode { node } = op else { continue };
            if !node.data.is_object() {
                node.data = Value::Object(Map::new());
            }
            match node.kind {
                NodeKind::Note => {
                    let d: NoteData = node.data_as().unwrap_or_default();
                    if d.file_name.is_none() && d.path.is_none() {
                        let name =
                            note_file_name(node.title.as_deref().unwrap_or("note"), &node.id);
                        node.data["fileName"] = Value::String(name);
                    }
                }
                NodeKind::Terminal | NodeKind::Agent => {
                    let missing = node
                        .data_str("terminalId")
                        .is_none_or(|s| s.trim().is_empty());
                    if missing {
                        node.data["terminalId"] = Value::String(new_id());
                    }
                }
                _ => {}
            }
        }
        ops
    }

    pub fn with_io<R>(&self, f: impl FnOnce() -> R) -> R {
        let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    pub fn reload_workspace(&self, ws: &str) -> Result<Workspace, String> {
        if !is_safe_id(ws) {
            return Err(format!("CANVAS_INVALID: workspace id '{ws}'"));
        }
        let loaded = persist::load_workspace(&self.root, ws)?
            .ok_or_else(|| not_found(format!("workspace {ws} on disk")))?;
        let mut meta = loaded.value;
        meta.id = ws.to_string();
        if meta.floors.iter().all(|f| f.id != GROUND_FLOOR_ID) {
            meta.floors.insert(
                0,
                Workspace::new(ws.to_string(), String::new())
                    .floors
                    .remove(0),
            );
        }
        let floors: Vec<String> = meta.floors.iter().map(|f| f.id.clone()).collect();
        {
            let mut g = self.lock();
            if meta.sort_index.is_none() && !g.workspaces.contains_key(ws) {
                meta.sort_index = Some(g.workspaces.len() as i64);
            }
            g.workspaces.insert(
                ws.to_string(),
                WsEntry {
                    meta: meta.clone(),
                    read_only: loaded.read_only,
                    floors: HashMap::new(),
                },
            );
        }
        self.note_watch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(w, _), _| w != ws);
        self.emit(StoreEvent::Workspaces);
        for f in floors {
            if let Ok(snap) = self.snapshot(ws, &f) {
                self.emit(StoreEvent::Patch(PatchEvent {
                    workspace_id: ws.to_string(),
                    floor_id: f.clone(),
                    epoch: snap.epoch,
                    rev: snap.rev,
                    seq: snap.seq,
                    ops: Vec::new(),
                    origin: ORIGIN_SYSTEM.to_string(),
                }));
            }
        }
        Ok(meta)
    }

    fn schedule(&self, key: SaveKey, delay: Duration) {
        match self.saver.get() {
            Some(s) => s.schedule(key, delay),
            None => {
                let _ = self.save_key(&key);
            }
        }
    }

    fn save_key(&self, key: &SaveKey) -> Result<(), String> {
        let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        match key {
            SaveKey::Workspace(ws) => {
                let meta = {
                    let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                    match g.workspaces.get(ws) {
                        Some(e) if !e.read_only => e.meta.clone(),
                        _ => return Ok(()),
                    }
                };
                persist::save_workspace(&self.root, &meta)
            }
            SaveKey::Floor(ws, floor) => {
                let bytes = {
                    let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                    match g.workspaces.get(ws).and_then(|e| e.floors.get(floor)) {
                        Some(st) if !st.read_only => persist::floor_bytes(&st.doc)?,
                        _ => return Ok(()),
                    }
                };
                persist::write_atomic(&persist::floor_file(&self.root, ws, floor), &bytes)
            }
        }
    }

    pub fn flush(&self) -> Result<(), String> {
        let keys = self.saver.get().map(|s| s.take_all()).unwrap_or_default();
        let mut first_err = None;
        for k in keys {
            if let Err(e) = self.save_key(&k) {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn flush_workspace(&self, ws: &str) -> Result<(), String> {
        let mut first_err = self.flush().err();
        let floors: Vec<String> = {
            let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.workspaces
                .get(ws)
                .map(|e| e.floors.keys().cloned().collect())
                .unwrap_or_default()
        };
        let mut keys = vec![SaveKey::Workspace(ws.to_string())];
        keys.extend(
            floors
                .into_iter()
                .map(|f| SaveKey::Floor(ws.to_string(), f)),
        );
        for k in keys {
            if let Err(e) = self.save_key(&k) {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

pub(super) fn internal_note_file(node: &CanvasNode) -> Option<String> {
    let d: NoteData = node.data_as()?;
    if d.is_external || d.path.is_some() {
        return None;
    }
    d.file_name
}

pub fn note_file_name(title: &str, id: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "note" } else { slug };
    let short: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!("{slug}-{short}.md")
}
