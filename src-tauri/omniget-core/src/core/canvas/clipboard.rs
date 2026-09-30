//! Structural copy and paste of a canvas selection (Maestri M-B21): nodes,
//! cords with both ends inside, drawings, groups, cable ties, note texts and
//! each terminal's chat history. Unlike a partitura this is not a blueprint:
//! terminal `cwd`/`command`, file tree roots and absolute paths travel as they
//! are. Pasting remaps every id, so the same clip can be pasted many times,
//! into any workspace or floor.
//!
//! The clip is mirrored to the OS clipboard as text (`CLIP_MARKER` + JSON), so
//! it survives a relaunch and reaches another window.

use super::chat::{self, ChatEntry};
use super::model::{
    new_id, now_rfc3339, CableTie, CanvasNode, Connection, Drawing, FloorDoc, Group, NodeKind,
    Point, Rect, Size,
};
use super::notes;
use super::ops::Op;
use super::store::{ApplyResult, CanvasStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const CLIP_FORMAT: &str = "omniget.canvas-clip";
pub const CLIP_VERSION: u32 = 1;
/// First line of the OS clipboard text that carries a clip.
pub const CLIP_MARKER: &str = "omniget-canvas-clip/v1";
/// Offset of each repeated paste at the view centre.
pub const CASCADE: f64 = 40.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub format: String,
    pub version: u32,
    pub id: String,
    #[serde(default)]
    pub source_workspace_id: String,
    #[serde(default)]
    pub source_floor_id: String,
    #[serde(default)]
    pub copied_at: String,
    /// Size of the selection; coordinates are relative to its top-left.
    #[serde(default)]
    pub bounds: Size,
    #[serde(default)]
    pub nodes: Vec<CanvasNode>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub drawings: Vec<Drawing>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub cable_ties: Vec<CableTie>,
    /// Note node id -> markdown.
    #[serde(default)]
    pub note_texts: BTreeMap<String, String>,
    /// Terminal node id -> chat transcript (every thread).
    #[serde(default)]
    pub chats: BTreeMap<String, Vec<ChatEntry>>,
}

impl Clip {
    pub fn summary(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "sourceWorkspaceId": self.source_workspace_id,
            "sourceFloorId": self.source_floor_id,
            "copiedAt": self.copied_at,
            "bounds": self.bounds,
            "nodes": self.nodes.iter().map(|n| serde_json::json!({
                "id": n.id, "kind": n.kind, "title": n.title,
            })).collect::<Vec<_>>(),
            "connections": self.connections.len(),
            "drawings": self.drawings.len(),
            "groups": self.groups.len(),
            "cableTies": self.cable_ties.len(),
            "notes": self.note_texts.len(),
            "chatEntries": self.chats.values().map(Vec::len).sum::<usize>(),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSelection {
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub drawing_ids: Vec<String>,
}

/// OS clipboard text for a clip.
pub fn encode_text(clip: &Clip) -> String {
    format!(
        "{CLIP_MARKER}\n{}",
        serde_json::to_string(clip).unwrap_or_default()
    )
}

/// The clip carried by an OS clipboard text, if it is one of ours.
pub fn decode_text(text: &str) -> Option<Clip> {
    let rest = text.trim_start().strip_prefix(CLIP_MARKER)?;
    let clip: Clip = serde_json::from_str(rest.trim()).ok()?;
    (clip.format == CLIP_FORMAT && clip.version <= CLIP_VERSION).then_some(clip)
}

/// Builds a clip from a selection of one floor. A selected stack brings its
/// notes; cross-floor connector chips stay behind.
pub fn build(
    doc: &FloorDoc,
    ws: &str,
    floor: &str,
    sel: &ClipSelection,
    note_text: &dyn Fn(&CanvasNode) -> Option<String>,
    chat_of: &dyn Fn(&CanvasNode) -> Vec<ChatEntry>,
) -> Result<Clip, String> {
    let mut wanted: HashSet<&str> = sel.node_ids.iter().map(String::as_str).collect();
    for n in &doc.nodes {
        if let Some(s) = &n.stack_id {
            if wanted.contains(s.as_str()) {
                wanted.insert(n.id.as_str());
            }
        }
    }
    let mut nodes: Vec<CanvasNode> = Vec::new();
    let mut texts = BTreeMap::new();
    let mut chats = BTreeMap::new();
    for n in doc.nodes.iter().filter(|n| wanted.contains(n.id.as_str())) {
        if n.kind == NodeKind::Connector {
            continue;
        }
        let mut n = n.clone();
        if n.kind == NodeKind::Note {
            texts.insert(n.id.clone(), note_text(&n).unwrap_or_default());
            // The copy is its own note file; an external file stays with the original.
            if let Some(d) = n.data.as_object_mut() {
                for k in ["path", "isExternal", "fileName"] {
                    d.remove(k);
                }
            }
        }
        if n.is_terminal() {
            let entries = chat_of(&n);
            if !entries.is_empty() {
                chats.insert(n.id.clone(), entries);
            }
        }
        nodes.push(n);
    }
    let dset: HashSet<&str> = sel.drawing_ids.iter().map(String::as_str).collect();
    let mut drawings: Vec<Drawing> = doc
        .drawings
        .iter()
        .filter(|d| dset.contains(d.id.as_str()))
        .cloned()
        .collect();
    if nodes.is_empty() && drawings.is_empty() {
        return Err("CANVAS_INVALID: nothing to copy in the selection".into());
    }
    let kept: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
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
    let cids: HashSet<&str> = connections.iter().map(|c| c.id.as_str()).collect();
    let cable_ties: Vec<CableTie> = doc
        .cable_ties
        .iter()
        .filter_map(|t| {
            let mut t = t.clone();
            t.member_connection_ids
                .retain(|m| cids.contains(m.as_str()));
            (!t.member_connection_ids.is_empty()).then_some(t)
        })
        .collect();
    // Groups travel when at least two of their members do.
    let mut members: HashMap<String, usize> = HashMap::new();
    for g in nodes
        .iter()
        .filter_map(|n| n.group_id.clone())
        .chain(drawings.iter().filter_map(|d| d.group_id.clone()))
    {
        *members.entry(g).or_default() += 1;
    }
    let groups: Vec<Group> = doc
        .groups
        .iter()
        .filter(|g| members.get(&g.id).copied().unwrap_or(0) >= 2)
        .cloned()
        .collect();
    let gids: HashSet<String> = groups.iter().map(|g| g.id.clone()).collect();
    for n in &mut nodes {
        if n.group_id.as_ref().is_some_and(|g| !gids.contains(g)) {
            n.group_id = None;
        }
    }
    for d in &mut drawings {
        if d.group_id.as_ref().is_some_and(|g| !gids.contains(g)) {
            d.group_id = None;
        }
    }

    let bbox = nodes
        .iter()
        .filter(|n| n.is_on_canvas())
        .map(CanvasNode::rect)
        .chain(drawings.iter().map(Drawing::bounds))
        .reduce(|a, b| a.union(&b))
        .unwrap_or_default();
    for n in &mut nodes {
        n.x -= bbox.x;
        n.y -= bbox.y;
    }
    for d in &mut drawings {
        d.origin.x -= bbox.x;
        d.origin.y -= bbox.y;
    }
    let mut cable_ties = cable_ties;
    for t in &mut cable_ties {
        t.position.x -= bbox.x;
        t.position.y -= bbox.y;
    }
    Ok(Clip {
        format: CLIP_FORMAT.into(),
        version: CLIP_VERSION,
        id: new_id(),
        source_workspace_id: ws.into(),
        source_floor_id: floor.into(),
        copied_at: now_rfc3339(),
        bounds: Size {
            w: bbox.w,
            h: bbox.h,
        },
        nodes,
        connections,
        drawings,
        groups,
        cable_ties,
        note_texts: texts,
        chats,
    })
}

/// Where a paste lands: an explicit top-left (right-click Paste), or centred
/// on a point with the cascade of repeated pastes.
#[derive(Debug, Clone, Copy)]
pub enum Placement {
    At(Point),
    Centered { center: Point, repeat: u32 },
}

impl Placement {
    pub fn top_left(&self, bounds: Size) -> Point {
        match *self {
            Placement::At(p) => p,
            // Centred pastes land on the grid (every stored frame is snapped).
            Placement::Centered { center, repeat } => Point {
                x: snap(center.x - bounds.w / 2.0) + CASCADE * repeat as f64,
                y: snap(center.y - bounds.h / 2.0) + CASCADE * repeat as f64,
            },
        }
    }
}

fn snap(v: f64) -> f64 {
    (v / super::model::GRID).round() * super::model::GRID
}

/// The ops and side writes of one paste, computed without touching disk.
#[derive(Debug, Clone, Default)]
pub struct PastePlan {
    pub ops: Vec<Op>,
    pub node_ids: Vec<String>,
    pub connection_ids: Vec<String>,
    pub drawing_ids: Vec<String>,
    pub group_ids: Vec<String>,
    /// New note node id -> text.
    pub note_writes: Vec<(String, String)>,
    /// New terminal node id -> transcript with fresh entry ids.
    pub chats: Vec<(String, Vec<ChatEntry>)>,
    /// New terminal nodes to start.
    pub terminals: Vec<String>,
}

fn unique_title(taken: &HashSet<String>, base: &str) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    let stem = base
        .rsplit_once(' ')
        .filter(|(_, n)| n.parse::<u32>().is_ok())
        .map(|(s, _)| s)
        .unwrap_or(base);
    (2..)
        .map(|i| format!("{stem} {i}"))
        .find(|t| !taken.contains(t))
        .expect("free title")
}

/// Pure: turns a clip into ops for `target` with every id remapped.
pub fn plan(clip: &Clip, target: &FloorDoc, placement: Placement) -> Result<PastePlan, String> {
    if clip.format != CLIP_FORMAT {
        return Err("CANVAS_INVALID: not a canvas clip".into());
    }
    let at = placement.top_left(clip.bounds);
    let mut ids: HashMap<String, String> = HashMap::new();
    for id in clip
        .nodes
        .iter()
        .map(|n| &n.id)
        .chain(clip.drawings.iter().map(|d| &d.id))
        .chain(clip.groups.iter().map(|g| &g.id))
        .chain(clip.connections.iter().map(|c| &c.id))
        .chain(clip.cable_ties.iter().map(|t| &t.id))
    {
        ids.insert(id.clone(), new_id());
    }
    let map = |id: &str| ids.get(id).cloned();
    let mut taken: HashSet<String> = target
        .nodes
        .iter()
        .filter_map(|n| n.title.clone())
        .collect();
    let z0 = target.max_z() + 1.0;
    let min_z = clip.nodes.iter().map(|n| n.z).fold(f64::MAX, f64::min);
    let mut p = PastePlan::default();

    for g in &clip.groups {
        let mut g = g.clone();
        g.id = map(&g.id).expect("mapped");
        p.group_ids.push(g.id.clone());
        p.ops.push(Op::SetGroup {
            id: g.id.clone(),
            group: Some(g),
        });
    }
    for n in &clip.nodes {
        let mut n = n.clone();
        let old = n.id.clone();
        n.id = map(&old).expect("mapped");
        n.x += at.x;
        n.y += at.y;
        n.z = z0 + (n.z - min_z).max(0.0);
        n.group_id = n.group_id.as_deref().and_then(map);
        n.stack_id = n.stack_id.as_deref().and_then(map);
        if let Some(t) = n.title.clone() {
            let t = unique_title(&taken, &t);
            taken.insert(t.clone());
            n.title = Some(t);
        }
        let is_terminal = n.is_terminal();
        let data = n.data.as_object_mut().ok_or("node data is not an object")?;
        if is_terminal {
            // A fresh process in the same cwd/command.
            data.insert("terminalId".into(), Value::String(new_id()));
            data.remove("isManager");
            p.terminals.push(n.id.clone());
            if let Some(entries) = clip.chats.get(&old) {
                let fresh = entries
                    .iter()
                    .cloned()
                    .map(|mut e| {
                        e.id = new_id();
                        e
                    })
                    .collect();
                p.chats.push((n.id.clone(), fresh));
            }
        }
        match &n.kind {
            NodeKind::Note => {
                if let Some(t) = clip.note_texts.get(&old) {
                    if !t.is_empty() {
                        p.note_writes.push((n.id.clone(), t.clone()));
                    }
                }
            }
            NodeKind::Stack => {
                let data = n.data.as_object_mut().expect("object");
                if let Some(tabs) = data.get_mut("tabs").and_then(|t| t.as_array_mut()) {
                    let next: Vec<Value> = tabs
                        .iter()
                        .filter_map(|t| t.as_str().and_then(map))
                        .map(Value::String)
                        .collect();
                    *tabs = next;
                }
                match data
                    .get("frontNoteId")
                    .and_then(|v| v.as_str())
                    .and_then(map)
                {
                    Some(f) => {
                        data.insert("frontNoteId".into(), Value::String(f));
                    }
                    None => {
                        data.remove("frontNoteId");
                    }
                }
            }
            _ => {}
        }
        p.node_ids.push(n.id.clone());
        p.ops.push(Op::AddNode { node: n });
    }
    for c in &clip.connections {
        let (Some(from), Some(to)) = (map(&c.from), map(&c.to)) else {
            continue;
        };
        let mut c = c.clone();
        c.id = map(&c.id).expect("mapped");
        c.from = from;
        c.to = to;
        c.from_note_id = c.from_note_id.as_deref().and_then(map);
        c.to_note_id = c.to_note_id.as_deref().and_then(map);
        c.points.clear();
        p.connection_ids.push(c.id.clone());
        p.ops.push(Op::AddConnection { connection: c });
    }
    for d in &clip.drawings {
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
        p.drawing_ids.push(d.id.clone());
        p.ops.push(Op::AddDrawing { drawing: d });
    }
    for t in &clip.cable_ties {
        let mut t = t.clone();
        t.id = map(&t.id).expect("mapped");
        t.position.x += at.x;
        t.position.y += at.y;
        t.member_connection_ids = t
            .member_connection_ids
            .iter()
            .filter_map(|m| map(m))
            .collect();
        if t.member_connection_ids.is_empty() {
            continue;
        }
        p.ops.push(Op::SetCableTie {
            id: t.id.clone(),
            tie: Some(t),
        });
    }
    Ok(p)
}

/// Clip of a selection straight from the store (note texts and chats read from disk).
pub fn copy_from_store(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    sel: &ClipSelection,
) -> Result<Clip, String> {
    let doc = store.read(ws, floor, |d| d.clone())?;
    let text = |n: &CanvasNode| notes::read(store, ws, floor, &n.id).ok().map(|c| c.text);
    let chats = |n: &CanvasNode| chat::read_all(store, ws, &n.id).unwrap_or_default();
    build(&doc, ws, floor, sel, &text, &chats)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteResult {
    pub apply: ApplyResult,
    pub node_ids: Vec<String>,
    pub connection_ids: Vec<String>,
    pub drawing_ids: Vec<String>,
    pub group_ids: Vec<String>,
    /// Terminal nodes the caller should start.
    pub terminals: Vec<String>,
}

/// Applies a paste in one batch, then writes the note files and the chat
/// transcripts (attachments copied next to the new node).
pub fn paste(
    store: &CanvasStore,
    ws: &str,
    floor: &str,
    clip: &Clip,
    placement: Placement,
    origin: &str,
) -> Result<PasteResult, String> {
    let target = store.read(ws, floor, |d| d.clone())?;
    let plan = plan(clip, &target, placement)?;
    let apply = store.apply(ws, floor, plan.ops, origin, None)?;
    for (id, text) in plan.note_writes {
        if let Err(e) = notes::write(store, ws, floor, &id, &text, None, origin) {
            tracing::warn!("[canvas] pasted note {id}: {e}");
        }
    }
    for (node, entries) in plan.chats {
        let dir = chat::attachments_dir(store, ws, &node).ok();
        for mut e in entries {
            for a in &mut e.attachments {
                let src = std::path::Path::new(&a.path);
                if let (Some(dir), true) = (&dir, src.is_file()) {
                    let _ = std::fs::create_dir_all(dir);
                    let dst = dir.join(format!("{}-{}", new_id(), a.filename));
                    if std::fs::copy(src, &dst).is_ok() {
                        a.path = dst.to_string_lossy().into_owned();
                    }
                }
            }
            if let Err(err) = chat::append(store, ws, &node, &e) {
                tracing::warn!("[canvas] pasted chat {node}: {err}");
                break;
            }
        }
    }
    Ok(PasteResult {
        apply,
        node_ids: plan.node_ids,
        connection_ids: plan.connection_ids,
        drawing_ids: plan.drawing_ids,
        group_ids: plan.group_ids,
        terminals: plan.terminals,
    })
}

/// Bounds of a set of rects, for callers that centre a paste.
pub fn bounds_of(rects: impl Iterator<Item = Rect>) -> Option<Rect> {
    rects.reduce(|a, b| a.union(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, kind: NodeKind, x: f64, y: f64, data: Value) -> CanvasNode {
        CanvasNode {
            id: id.into(),
            kind,
            x,
            y,
            w: 100.0,
            h: 80.0,
            z: 1.0,
            title: Some(id.to_uppercase()),
            subtitle: None,
            icon: None,
            color: None,
            group_id: None,
            locked: false,
            stack_id: None,
            data,
        }
    }

    fn doc() -> FloorDoc {
        let mut d = FloorDoc::default();
        let mut a = node(
            "a",
            NodeKind::Terminal,
            100.0,
            100.0,
            json!({"terminalId": "t-a", "agent": "shell", "cwd": "/abs/proj", "command": "ls"}),
        );
        let mut b = node(
            "b",
            NodeKind::Terminal,
            300.0,
            100.0,
            json!({"terminalId": "t-b", "agent": "shell", "cwd": "/abs/other"}),
        );
        a.group_id = Some("g".into());
        b.group_id = Some("g".into());
        let c = node("c", NodeKind::Note, 900.0, 900.0, json!({}));
        d.nodes = vec![a, b, c];
        d.groups = vec![Group {
            id: "g".into(),
            name: Some("G".into()),
            color: None,
            target: super::super::model::GroupTarget::Nodes,
        }];
        d.connections = vec![
            serde_json::from_value(
                json!({"id":"ab","from":"a","to":"b","kind":"ask","perms":["ask"]}),
            )
            .unwrap(),
            serde_json::from_value(json!({"id":"ac","from":"a","to":"c","kind":"note","perms":[]}))
                .unwrap(),
        ];
        d
    }

    #[test]
    fn copy_keeps_inner_cords_groups_and_paths_and_paste_remaps_everything() {
        let d = doc();
        let sel = ClipSelection {
            node_ids: vec!["a".into(), "b".into()],
            drawing_ids: vec![],
        };
        let chats = |n: &CanvasNode| -> Vec<ChatEntry> {
            if n.id == "a" {
                vec![serde_json::from_value(json!({
                    "id":"e1","thread":2,"role":"agent","kind":"final","ts":"2026-01-01T00:00:00Z","text":"hi"
                }))
                .unwrap()]
            } else {
                vec![]
            }
        };
        let clip = build(&d, "w", "f", &sel, &|_| None, &chats).unwrap();
        assert_eq!(clip.nodes.len(), 2);
        assert_eq!(clip.connections.len(), 1, "only the inner cord");
        assert_eq!(clip.groups.len(), 1);
        assert_eq!(clip.nodes[0].x, 0.0);
        assert_eq!(clip.bounds.w, 300.0);
        let back = decode_text(&encode_text(&clip)).expect("round trip");
        assert_eq!(back.id, clip.id);
        assert!(decode_text("hello").is_none());

        let first = plan(
            &clip,
            &d,
            Placement::Centered {
                center: Point {
                    x: 1000.0,
                    y: 1000.0,
                },
                repeat: 0,
            },
        )
        .unwrap();
        let second = plan(
            &clip,
            &d,
            Placement::Centered {
                center: Point {
                    x: 1000.0,
                    y: 1000.0,
                },
                repeat: 1,
            },
        )
        .unwrap();
        let added = |p: &PastePlan| -> Vec<CanvasNode> {
            p.ops
                .iter()
                .filter_map(|o| match o {
                    Op::AddNode { node } => Some(node.clone()),
                    _ => None,
                })
                .collect()
        };
        let n1 = added(&first);
        let n2 = added(&second);
        // Cascade: the second paste is +40,+40 from the first.
        assert_eq!(n2[0].x - n1[0].x, CASCADE);
        assert_eq!(n2[0].y - n1[0].y, CASCADE);
        // 1000 - 150 = 850, snapped to the 20 grid.
        assert_eq!(n1[0].x, 860.0);
        // Fresh ids everywhere, paths kept, unique titles, new group.
        for n in &n1 {
            assert!(n.id != "a" && n.id != "b");
            assert!(n.data["terminalId"] != "t-a" && n.data["terminalId"] != "t-b");
            assert_eq!(n.group_id.as_deref(), Some(first.group_ids[0].as_str()));
        }
        assert_eq!(n1[0].data["cwd"], "/abs/proj");
        assert_eq!(n1[0].data["command"], "ls");
        assert_eq!(n1[0].title.as_deref(), Some("A 2"));
        assert_ne!(first.group_ids[0], "g");
        let cord = first
            .ops
            .iter()
            .find_map(|o| match o {
                Op::AddConnection { connection } => Some(connection.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(cord.from, n1[0].id);
        assert_eq!(cord.to, n1[1].id);
        assert_eq!(first.terminals.len(), 2);
        assert_eq!(first.chats.len(), 1);
        assert_eq!(first.chats[0].0, n1[0].id);
        assert_ne!(first.chats[0].1[0].id, "e1");
        // Explicit placement: no cascade.
        let at = plan(&clip, &d, Placement::At(Point { x: 5.0, y: 7.0 })).unwrap();
        assert_eq!(added(&at)[0].x, 5.0);
    }
}
