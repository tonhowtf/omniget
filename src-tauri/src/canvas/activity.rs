use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use omniget_core::core::canvas::model::NodeKind;
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::{
    CanvasStore, NoteEvent, PatchEvent, StoreEvent, ORIGIN_SYSTEM, ORIGIN_USER,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const EVENT_ACTIVITY: &str = "canvas://activity";
const PER_WORKSPACE: usize = 500;
const MERGE_MS: i64 = 4_000;
const TITLE_CACHE: usize = 5_000;
const UNDO_SUFFIX: &str = ":undo";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    pub seq: u64,
    pub rev: u64,
    pub at: i64,
    pub workspace_id: String,
    pub floor_id: String,
    pub origin: String,
    pub actor: String,
    pub actor_kind: &'static str,
    pub verb: &'static str,
    pub target: Option<String>,
    pub target_kind: Option<String>,
    pub count: usize,
    pub summary: String,
    pub node_ids: Vec<String>,
    pub undo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

enum Raw {
    Patch(PatchEvent),
    Note(NoteEvent),
}

#[derive(Default)]
struct Book {
    entries: HashMap<String, VecDeque<Entry>>,
    titles: HashMap<String, (String, String)>,
    actors: HashMap<String, String>,
    next: u64,
}

fn book() -> &'static Mutex<Book> {
    static BOOK: OnceLock<Mutex<Book>> = OnceLock::new();
    BOOK.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, Book> {
    book().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn name_actor(origin: &str, display: &str) {
    let display = display.trim();
    if origin.is_empty() || display.is_empty() {
        return;
    }
    let mut b = lock();
    if b.actors.len() > 512 && !b.actors.contains_key(origin) {
        b.actors.clear();
    }
    b.actors.insert(origin.to_string(), display.to_string());
}

pub fn install(app: &AppHandle, store: &Arc<CanvasStore>) {
    static TX: OnceLock<Sender<Raw>> = OnceLock::new();
    if TX.get().is_some() {
        return;
    }
    let (tx, rx) = channel::<Raw>();
    if TX.set(tx.clone()).is_err() {
        return;
    }
    let weak = Arc::downgrade(store);
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("canvas-activity".into())
        .spawn(move || {
            while let Ok(raw) = rx.recv() {
                if let Some(e) = record(&weak, raw) {
                    let _ = handle.emit(EVENT_ACTIVITY, &e);
                }
            }
        });
    if spawned.is_err() {
        return;
    }
    store.subscribe(move |ev| {
        let raw = match ev {
            StoreEvent::Patch(p) if !p.ops.is_empty() && !p.ops.iter().all(Op::is_camera) => {
                Raw::Patch(p.clone())
            }
            StoreEvent::Note(n) => Raw::Note(n.clone()),
            _ => return,
        };
        let _ = tx.send(raw);
    });
}

pub fn recent(
    workspace_id: &str,
    floor_id: Option<&str>,
    since: Option<u64>,
    limit: usize,
) -> Vec<Entry> {
    let b = lock();
    let Some(q) = b.entries.get(workspace_id) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = q
        .iter()
        .rev()
        .filter(|e| floor_id.is_none_or(|f| e.floor_id == f))
        .filter(|e| since.is_none_or(|s| e.id > s))
        .take(limit.max(1))
        .cloned()
        .collect();
    out.reverse();
    out
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn base_origin(origin: &str) -> (&str, bool) {
    match origin.strip_suffix(UNDO_SUFFIX) {
        Some(b) => (b, true),
        None => (origin, false),
    }
}

struct Via<'a> {
    by: &'a str,
    via: Option<&'a str>,
    step: Option<&'a str>,
    detail: Option<&'a str>,
}

fn split_via(origin: &str) -> Via<'_> {
    let Some((by, rest)) = origin.split_once('#') else {
        return Via {
            by: origin,
            via: None,
            step: None,
            detail: None,
        };
    };
    let mut parts = rest.splitn(3, ':');
    let via = parts.next().filter(|v| !v.is_empty());
    let step = parts.next().filter(|v| !v.is_empty());
    let detail = parts.next().map(str::trim).filter(|v| !v.is_empty());
    Via {
        by: if by.is_empty() { ORIGIN_SYSTEM } else { by },
        via,
        step,
        detail,
    }
}

fn step_verb(step: &str) -> Option<&'static str> {
    match step {
        "quoted" => Some("quoted"),
        "submitted" => Some("submitted"),
        "generated" => Some("generated"),
        "failed" => Some("failed"),
        "uncertain" => Some("uncertain"),
        _ => None,
    }
}

fn step_summary(verb: &str, title: &str, detail: Option<&str>) -> String {
    let what = if title.trim().is_empty() {
        "a generation".to_string()
    } else {
        format!("generation {}", quoted(title))
    };
    let extra = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    match verb {
        "quoted" => format!("got a price for {what}{extra}"),
        "submitted" => format!("started {what}{extra}"),
        "generated" => format!("{what} finished{extra}"),
        "failed" => format!("{what} failed{extra}"),
        _ => format!("{what} needs checking{extra}"),
    }
}

fn actor_of(
    b: &Book,
    store: Option<&Arc<CanvasStore>>,
    ws: &str,
    floor: &str,
    origin: &str,
) -> (String, &'static str) {
    if origin == ORIGIN_USER {
        return ("user".into(), "person");
    }
    if origin == ORIGIN_SYSTEM {
        return ("OmniGet".into(), "system");
    }
    if let Some(name) = b.actors.get(origin) {
        let kind = if origin.starts_with("agent:") {
            "agent"
        } else {
            "client"
        };
        return (name.clone(), kind);
    }
    if let Some(node) = origin.strip_prefix("agent:") {
        let title = b
            .titles
            .get(node)
            .map(|t| t.0.clone())
            .or_else(|| {
                store
                    .and_then(|s| {
                        s.read(ws, floor, |d| d.node(node).and_then(|n| n.title.clone()))
                            .ok()
                    })
                    .flatten()
            })
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| node.to_string());
        return (title, "agent");
    }
    if let Some(label) = origin.strip_prefix("mcp:") {
        return (label.to_string(), "client");
    }
    if origin == "mcp" {
        return ("MCP client".into(), "client");
    }
    (origin.to_string(), "system")
}

fn title_of(
    b: &mut Book,
    store: Option<&Arc<CanvasStore>>,
    ws: &str,
    floor: &str,
    id: &str,
) -> Option<(String, String)> {
    if let Some(s) = store {
        if let Ok(Some((t, k))) = s.read(ws, floor, |d| {
            d.node(id).map(|n| {
                (
                    n.title.clone().unwrap_or_default(),
                    n.kind.as_str().to_string(),
                )
            })
        }) {
            remember_title(b, id, &t, &k);
            return Some((t, k));
        }
    }
    b.titles.get(id).cloned()
}

fn remember_title(b: &mut Book, id: &str, title: &str, kind: &str) {
    if b.titles.len() > TITLE_CACHE {
        b.titles.clear();
    }
    b.titles
        .insert(id.to_string(), (title.to_string(), kind.to_string()));
}

fn quoted(t: &str) -> String {
    let t = t.trim();
    if t.chars().count() > 60 {
        let cut: String = t.chars().take(57).collect();
        format!("'{cut}…'")
    } else {
        format!("'{t}'")
    }
}

fn kind_noun(kind: &str) -> &str {
    match kind {
        "terminal" | "agent" => "terminal",
        "filetree" => "file tree",
        "" => "node",
        k => k,
    }
}

fn phrase(verb: &str) -> &str {
    match verb {
        "corded" => "added",
        "uncorded" => "removed",
        "grouped" => "changed",
        v => v,
    }
}

fn article(noun: &str) -> &'static str {
    if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

fn describe(verb: &str, nodes: &[(String, String)], other: usize, noun: &str) -> String {
    let said = phrase(verb);
    match nodes {
        [] if other > 1 => format!("{said} {other} {noun}s"),
        [] => format!("{said} {} {noun}", article(noun)),
        [(t, k)] if !t.trim().is_empty() => format!("{said} {} {}", kind_noun(k), quoted(t)),
        [(_, k)] => format!("{said} {} {}", article(kind_noun(k)), kind_noun(k)),
        many => format!("{said} {} nodes", many.len()),
    }
}

struct Shape {
    verb: &'static str,
    node_ids: Vec<String>,
    other: usize,
    noun: &'static str,
}

fn shape(ops: &[Op]) -> Shape {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut moved = Vec::new();
    let mut resized = Vec::new();
    let mut renamed = Vec::new();
    let mut edited = Vec::new();
    let (mut cords_added, mut cords_removed, mut cords_changed) = (0, 0, 0);
    let (mut drawn, mut erased, mut redrawn) = (0, 0, 0);
    let mut grouped = 0;
    for op in ops {
        match op {
            Op::AddNode { node } => added.push(node.id.clone()),
            Op::RemoveNode { id } => removed.push(id.clone()),
            Op::UpdateNode { id, patch } => {
                let keys: Vec<&str> = patch.keys().map(String::as_str).collect();
                let only = |set: &[&str]| keys.iter().all(|k| set.contains(k));
                if keys.contains(&"title") {
                    renamed.push(id.clone());
                } else if only(&["x", "y", "z", "groupId", "stackId"]) {
                    moved.push(id.clone());
                } else if only(&["x", "y", "w", "h", "z"]) {
                    resized.push(id.clone());
                } else {
                    edited.push(id.clone());
                }
            }
            Op::AddConnection { .. } => cords_added += 1,
            Op::RemoveConnection { .. } => cords_removed += 1,
            Op::UpdateConnection { .. } => cords_changed += 1,
            Op::AddDrawing { .. } => drawn += 1,
            Op::RemoveDrawing { .. } => erased += 1,
            Op::UpdateDrawing { .. } => redrawn += 1,
            Op::SetGroup { .. } | Op::SetCableTie { .. } => grouped += 1,
            Op::SetCamera { .. } => {}
        }
    }
    let pick = |verb: &'static str, ids: Vec<String>| Shape {
        verb,
        node_ids: ids,
        other: 0,
        noun: "node",
    };
    let count = |verb: &'static str, n: usize, noun: &'static str| Shape {
        verb,
        node_ids: Vec::new(),
        other: n,
        noun,
    };
    if !added.is_empty() {
        return pick("created", added);
    }
    if !removed.is_empty() {
        return pick("deleted", removed);
    }
    if !renamed.is_empty() {
        return pick("renamed", renamed);
    }
    if !edited.is_empty() {
        let mut ids = edited;
        ids.extend(moved);
        ids.extend(resized);
        return pick("edited", ids);
    }
    if !resized.is_empty() {
        let mut ids = resized;
        ids.extend(moved);
        return pick("resized", ids);
    }
    if !moved.is_empty() {
        return pick("moved", moved);
    }
    if cords_added > 0 {
        return count("corded", cords_added, "cord");
    }
    if cords_removed > 0 {
        return count("uncorded", cords_removed, "cord");
    }
    if cords_changed > 0 {
        return count("changed", cords_changed, "cord");
    }
    if drawn > 0 {
        return count("drew", drawn, "drawing");
    }
    if erased > 0 {
        return count("erased", erased, "drawing");
    }
    if redrawn > 0 {
        return count("changed", redrawn, "drawing");
    }
    count("grouped", grouped.max(1), "group")
}

fn cord_ends(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|o| match o {
            Op::AddConnection { connection } => {
                Some([connection.from.clone(), connection.to.clone()])
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn record(store: &Weak<CanvasStore>, raw: Raw) -> Option<Entry> {
    let store = store.upgrade();
    let store = store.as_ref();
    let at = now_ms();
    let mut b = lock();
    let (ws, floor, seq, rev, origin, verb, node_ids, summary_nodes, other, noun, link) = match raw
    {
        Raw::Patch(p) => {
            let tag = split_via(base_origin(&p.origin).0);
            if tag.via.is_some() && tag.step == Some("progress") {
                return None;
            }
            if let Some(v) = tag.step.and_then(step_verb) {
                let main = p
                    .ops
                    .iter()
                    .rev()
                    .find_map(|o| match o {
                        Op::UpdateNode { id, .. } => Some(id.clone()),
                        _ => None,
                    })
                    .or_else(|| shape(&p.ops).node_ids.first().cloned())
                    .unwrap_or_default();
                let named = title_of(&mut b, store, &p.workspace_id, &p.floor_id, &main)
                    .unwrap_or_default();
                let mut ids: Vec<String> = p
                    .ops
                    .iter()
                    .filter_map(|o| match o {
                        Op::AddNode { node } => {
                            let t = node.title.clone().unwrap_or_default();
                            remember_title(&mut b, &node.id, &t, node.kind.as_str());
                            Some(node.id.clone())
                        }
                        _ => None,
                    })
                    .collect();
                if !main.is_empty() && !ids.contains(&main) {
                    ids.insert(0, main);
                }
                let link = Some(step_summary(v, &named.0, tag.detail));
                (
                    p.workspace_id,
                    p.floor_id,
                    p.seq,
                    p.rev,
                    p.origin,
                    v,
                    ids,
                    vec![named],
                    1,
                    "generation",
                    link,
                )
            } else {
                for op in &p.ops {
                    if let Op::AddNode { node } = op {
                        let t = node.title.clone().unwrap_or_default();
                        remember_title(&mut b, &node.id, &t, node.kind.as_str());
                    }
                }
                let sh = shape(&p.ops);
                let mut named = Vec::new();
                if sh.node_ids.len() == 1 {
                    named.push(
                        title_of(&mut b, store, &p.workspace_id, &p.floor_id, &sh.node_ids[0])
                            .unwrap_or_default(),
                    );
                } else {
                    named.resize(sh.node_ids.len(), (String::new(), String::new()));
                }
                let mut ids = sh.node_ids;
                let mut link = None;
                if sh.verb == "corded" {
                    ids = cord_ends(&p.ops);
                    if ids.len() == 2 {
                        let mut end = |id: &str| {
                            title_of(&mut b, store, &p.workspace_id, &p.floor_id, id)
                                .map(|(t, _)| t)
                                .filter(|t| !t.trim().is_empty())
                                .unwrap_or_else(|| id.to_string())
                        };
                        let (from, to) = (end(&ids[0]), end(&ids[1]));
                        link = Some(format!("connected {} to {}", quoted(&from), quoted(&to)));
                    }
                }
                ids.dedup();
                (
                    p.workspace_id,
                    p.floor_id,
                    p.seq,
                    p.rev,
                    p.origin,
                    sh.verb,
                    ids,
                    named,
                    sh.other,
                    sh.noun,
                    link,
                )
            }
        }
        Raw::Note(n) => {
            let floor = store
                .and_then(|s| s.locate_node(&n.node_id))
                .map(|(_, f)| f)
                .unwrap_or_default();
            let named = title_of(&mut b, store, &n.workspace_id, &floor, &n.node_id)
                .map(|(t, _)| vec![(t, NodeKind::Note.as_str().to_string())])
                .unwrap_or_default();
            (
                n.workspace_id,
                floor,
                0,
                0,
                n.origin,
                "wrote",
                vec![n.node_id],
                named,
                1,
                "note",
                None,
            )
        }
    };
    let (raw_base, undo) = base_origin(&origin);
    let tag = split_via(raw_base);
    let base = tag.by;
    let via = tag.via.map(str::to_string);
    let origin = if undo {
        format!("{base}{UNDO_SUFFIX}")
    } else {
        base.to_string()
    };
    let (actor, actor_kind) = actor_of(&b, store, &ws, &floor, base);
    let mut summary = link.unwrap_or_else(|| describe(verb, &summary_nodes, other, noun));
    if undo {
        summary = format!("undid: {summary}");
    }
    let target = summary_nodes
        .first()
        .filter(|_| summary_nodes.len() == 1)
        .map(|(t, _)| t.clone())
        .filter(|t| !t.trim().is_empty());
    let target_kind = summary_nodes
        .first()
        .filter(|_| summary_nodes.len() == 1)
        .map(|(_, k)| k.clone())
        .filter(|k| !k.is_empty());
    let count = if node_ids.is_empty() {
        other.max(1)
    } else {
        node_ids.len()
    };
    let q = b.entries.entry(ws.clone()).or_default();
    if let Some(last) = q.back_mut() {
        if last.origin == origin
            && last.via == via
            && last.floor_id == floor
            && last.verb == verb
            && last.node_ids == node_ids
            && at - last.at < MERGE_MS
        {
            last.at = at;
            if seq > 0 {
                last.seq = seq;
                last.rev = rev;
            }
            return Some(last.clone());
        }
    }
    let id = {
        b.next += 1;
        b.next
    };
    let entry = Entry {
        id,
        seq,
        rev,
        at,
        workspace_id: ws.clone(),
        floor_id: floor,
        origin,
        actor,
        actor_kind,
        verb,
        target,
        target_kind,
        count,
        summary,
        node_ids,
        undo,
        via,
    };
    let q = b.entries.entry(ws).or_default();
    q.push_back(entry.clone());
    while q.len() > PER_WORKSPACE {
        q.pop_front();
    }
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_tags_split_into_requester_via_and_step() {
        let t = split_via("mcp:qa-rafa#generate:generated:v2, 1.5 credits");
        assert_eq!(t.by, "mcp:qa-rafa");
        assert_eq!(t.via, Some("generate"));
        assert_eq!(t.step, Some("generated"));
        assert_eq!(t.detail, Some("v2, 1.5 credits"));
        let plain = split_via("agent:abc");
        assert_eq!(plain.by, "agent:abc");
        assert!(plain.via.is_none() && plain.step.is_none());
        assert_eq!(split_via("#generate:quoted").by, ORIGIN_SYSTEM);
    }
}
