//! Cords across floors and workspaces (Maestri connectors). A cord between
//! terminals on different floors or workspaces is two halves: on each side a
//! `connector` chip next to the local terminal (its data points at the remote
//! terminal) and a local cord terminal ↔ chip. The cords module already
//! resolves a chip to its remote node (`list` shows "B @ W2", `ask` reaches
//! it). Both chips carry the same `pairId`; removing either chip, or the cord
//! that ties a chip, removes the whole pair (a sweep after each patch). Only
//! terminals cross; there is no undo for a pair (Maestri).

use std::sync::{Arc, OnceLock};

use omniget_core::core::canvas::layout;
use omniget_core::core::canvas::model::{
    new_id, CanvasNode, Connection, ConnectionKind, FloorDoc, NodeKind, Point,
};
use omniget_core::core::canvas::ops::Op;
use omniget_core::core::canvas::store::{CanvasStore, StoreEvent};
use serde_json::{json, Value};

const CHIP_W: f64 = 160.0;
const CHIP_H: f64 = 48.0;
/// Origin of the pair's ops (and of the sweep).
pub const ORIGIN: &str = "link";

fn store() -> Arc<CanvasStore> {
    super::doc::host().store().clone()
}

/// A node and where it lives.
#[derive(Debug, Clone)]
pub struct End {
    pub ws: String,
    pub floor: String,
    pub node: CanvasNode,
}

fn is_terminal(n: &CanvasNode) -> bool {
    matches!(n.kind, NodeKind::Terminal | NodeKind::Agent)
}

/// `crossFloor` or `crossWorkspace` for two ends, `None` on the same floor.
pub fn route(a: &End, b: &End) -> Option<&'static str> {
    if a.ws != b.ws {
        Some("crossWorkspace")
    } else if a.floor != b.floor {
        Some("crossFloor")
    } else {
        None
    }
}

fn floor_name(s: &CanvasStore, ws: &str, floor: &str) -> String {
    s.workspace(ws)
        .ok()
        .and_then(|m| {
            m.floors
                .iter()
                .find(|x| x.id == floor)
                .map(|x| x.name.clone())
        })
        .unwrap_or_default()
}

fn ws_name(s: &CanvasStore, ws: &str) -> String {
    s.workspace(ws)
        .map(|w| w.name)
        .unwrap_or_else(|_| ws.into())
}

/// Chip title as Maestri writes it: "Name @ Workspace" across workspaces,
/// "Name @ Floor" across floors.
fn chip_title(s: &CanvasStore, local: &End, remote: &End) -> String {
    let name = super::cords::node_name(&remote.node);
    if local.ws != remote.ws {
        format!("{name} @ {}", ws_name(s, &remote.ws))
    } else {
        format!("{name} @ {}", floor_name(s, &remote.ws, &remote.floor))
    }
}

fn chip(title: String, at: Point, z: f64, remote: &End, pair: &str, kind: &str) -> CanvasNode {
    CanvasNode {
        id: new_id(),
        kind: NodeKind::Connector,
        x: at.x,
        y: at.y,
        w: CHIP_W,
        h: CHIP_H,
        z,
        title: Some(title),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: json!({
            "remoteNodeId": remote.node.id,
            "remoteTerminalId": remote.node.data_str("terminalId"),
            "remoteFloorId": remote.floor,
            "remoteWorkspaceId": remote.ws,
            "pairId": pair,
            "route": kind,
        }),
    }
}

fn cord(from: &str, to: &str, perms: &[String], label: &str) -> Connection {
    Connection {
        id: new_id(),
        from: from.to_string(),
        to: to.to_string(),
        kind: ConnectionKind::Ask,
        perms: perms.to_vec(),
        condition: None,
        verify: None,
        label: Some(label.to_string()),
        points: Vec::new(),
        from_note_id: None,
        to_note_id: None,
        is_active: None,
    }
}

/// Chips of `doc` already pointing at `remote_id` from a node tied to `local_id`.
fn existing_chip(doc: &FloorDoc, local_id: &str, remote_id: &str) -> Option<String> {
    doc.nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Connector && n.data_str("remoteNodeId") == Some(remote_id))
        .find(|n| {
            doc.connections
                .iter()
                .any(|c| c.touches(&n.id) && c.touches(local_id))
        })
        .map(|n| n.id.clone())
}

/// Places one half: a chip next to `local` pointing at `remote`, plus the cord.
fn half(
    s: &CanvasStore,
    local: &End,
    remote: &End,
    pair: &str,
    kind: &str,
    perms: &[String],
    origin: &str,
) -> Result<(String, String), String> {
    let doc = s.read(&local.ws, &local.floor, |d| d.clone())?;
    let near = Point {
        x: local.node.x + local.node.w + 40.0,
        y: local.node.y,
    };
    let at = layout::free_spot_in(&doc, CHIP_W, CHIP_H, Some(near), &[]);
    let c = chip(
        chip_title(s, local, remote),
        at,
        doc.max_z() + 1.0,
        remote,
        pair,
        kind,
    );
    let k = cord(&local.node.id, &c.id, perms, kind);
    let (cid, kid) = (c.id.clone(), k.id.clone());
    s.apply(
        &local.ws,
        &local.floor,
        vec![Op::AddNode { node: c }, Op::AddConnection { connection: k }],
        origin,
        None,
    )?;
    Ok((cid, kid))
}

/// Ties two terminals on different floors or workspaces. Refuses notes,
/// portals and anything that is not a terminal (they stay on one floor).
pub fn link(a: &End, b: &End, perms: &[String], origin: &str) -> Result<Value, String> {
    ensure_watch();
    let Some(kind) = route(a, b) else {
        return Err("CANVAS_INVALID: both ends are on the same floor; use a plain cord".into());
    };
    if !is_terminal(&a.node) || !is_terminal(&b.node) {
        return Err(format!(
            "CANVAS_INVALID: only terminals connect across {}; notes and portals stay on one floor",
            if kind == "crossWorkspace" {
                "workspaces"
            } else {
                "floors"
            }
        ));
    }
    let s = store();
    let da = s.read(&a.ws, &a.floor, |d| d.clone())?;
    if let Some(chip) = existing_chip(&da, &a.node.id, &b.node.id) {
        return Err(format!(
            "CANVAS_CONFLICT: already connected (connector {chip})"
        ));
    }
    let perms: Vec<String> = if perms.is_empty() {
        vec!["ask".into(), "check".into()]
    } else {
        perms.to_vec()
    };
    let pair = new_id();
    let (chip_a, cord_a) = half(&s, a, b, &pair, kind, &perms, origin)?;
    let (chip_b, cord_b) = match half(&s, b, a, &pair, kind, &perms, origin) {
        Ok(r) => r,
        Err(e) => {
            let _ = s.apply(
                &a.ws,
                &a.floor,
                vec![Op::RemoveNode { id: chip_a }],
                origin,
                None,
            );
            return Err(e);
        }
    };
    Ok(json!({
        "pairId": pair,
        "kind": kind,
        "from": { "workspaceId": a.ws, "floorId": a.floor, "nodeId": a.node.id, "connectorId": chip_a, "cordId": cord_a },
        "to": { "workspaceId": b.ws, "floorId": b.floor, "nodeId": b.node.id, "connectorId": chip_b, "cordId": cord_b },
    }))
}

/// Every chip carrying `pair` in the whole store: `(ws, floor, node id)`.
fn chips_of_pair(s: &CanvasStore, pair: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for w in s.workspaces() {
        let ws = w.workspace.id.clone();
        for f in &w.workspace.floors {
            let ids = s
                .read(&ws, &f.id, |d| {
                    d.nodes
                        .iter()
                        .filter(|n| {
                            n.kind == NodeKind::Connector && n.data_str("pairId") == Some(pair)
                        })
                        .map(|n| n.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            out.extend(ids.into_iter().map(|id| (ws.clone(), f.id.clone(), id)));
        }
    }
    out
}

/// Removes both halves of `pair`.
pub fn unlink_pair(pair: &str, origin: &str) -> Result<Vec<String>, String> {
    let s = store();
    let mut removed = Vec::new();
    for (ws, floor, id) in chips_of_pair(&s, pair) {
        if s.apply(
            &ws,
            &floor,
            vec![Op::RemoveNode { id: id.clone() }],
            origin,
            None,
        )
        .is_ok()
        {
            removed.push(id);
        }
    }
    Ok(removed)
}

/// The pair of a chip, or of the cord tying a chip.
pub fn pair_of(ws: &str, floor: &str, id: &str) -> Option<String> {
    store()
        .read(ws, floor, |d| {
            let chip = match d.node(id) {
                Some(n) => Some(n.clone()),
                None => d.connection(id).and_then(|c| {
                    [c.from.as_str(), c.to.as_str()]
                        .into_iter()
                        .filter_map(|e| d.node(e))
                        .find(|n| n.kind == NodeKind::Connector)
                        .cloned()
                }),
            }?;
            chip.data_str("pairId").map(str::to_string)
        })
        .ok()
        .flatten()
}

/// Pairs anywhere in the store that lost a half, or whose chip lost its cord.
fn broken_pairs(s: &CanvasStore) -> Vec<String> {
    use std::collections::HashMap;
    // pair -> (chips seen, all tied)
    let mut seen: HashMap<String, (usize, bool)> = HashMap::new();
    for w in s.workspaces() {
        let ws = w.workspace.id.clone();
        for f in &w.workspace.floors {
            let chips: Vec<(String, bool)> = s
                .read(&ws, &f.id, |d| {
                    d.nodes
                        .iter()
                        .filter(|n| n.kind == NodeKind::Connector)
                        .filter_map(|n| {
                            let pair = n.data_str("pairId")?.to_string();
                            let tied = d.connections.iter().any(|c| c.touches(&n.id));
                            Some((pair, tied))
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (pair, tied) in chips {
                let e = seen.entry(pair).or_insert((0, true));
                e.0 += 1;
                e.1 &= tied;
            }
        }
    }
    seen.into_iter()
        .filter(|(_, (n, tied))| *n < 2 || !tied)
        .map(|(p, _)| p)
        .collect()
}

/// Watches patches: a removed chip or chip cord takes its twin with it.
pub fn ensure_watch() {
    static ON: OnceLock<()> = OnceLock::new();
    ON.get_or_init(|| {
        let s = store();
        s.subscribe(|ev| {
            let StoreEvent::Patch(p) = ev else { return };
            if p.origin == ORIGIN {
                return;
            }
            let removes = p
                .ops
                .iter()
                .any(|o| matches!(o, Op::RemoveNode { .. } | Op::RemoveConnection { .. }));
            if !removes {
                return;
            }
            // Off the store's call stack: the sweep applies ops itself.
            std::thread::spawn(move || {
                let s = store();
                for pair in broken_pairs(&s) {
                    let _ = unlink_pair(&pair, ORIGIN);
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> CanvasNode {
        CanvasNode {
            id: id.into(),
            kind: NodeKind::Terminal,
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            z: 0.0,
            title: None,
            subtitle: None,
            icon: None,
            color: None,
            group_id: None,
            locked: false,
            stack_id: None,
            data: json!({}),
        }
    }

    #[test]
    fn routes() {
        let e = |ws: &str, f: &str| End {
            ws: ws.into(),
            floor: f.into(),
            node: node("n"),
        };
        assert_eq!(route(&e("a", "g"), &e("a", "g")), None);
        assert_eq!(route(&e("a", "g"), &e("a", "f1")), Some("crossFloor"));
        assert_eq!(route(&e("a", "g"), &e("b", "g")), Some("crossWorkspace"));
    }
}
