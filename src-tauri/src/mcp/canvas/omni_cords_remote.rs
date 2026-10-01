use omniget_core::core::canvas::model::CanvasNode;
use omniget_core::core::canvas::store::CanvasStore;
use serde_json::{json, Value};

use super::{list, locate_in, st, ws_of};

fn find_in(
    s: &CanvasStore,
    ws: &str,
    floor: Option<&str>,
    name: &str,
) -> Result<(String, String, CanvasNode), String> {
    let meta = s.workspace(ws)?;
    let mut exact = Vec::new();
    let mut loose = Vec::new();
    for f in &meta.floors {
        if floor.is_some_and(|x| x != f.id) {
            continue;
        }
        let nodes = s.read(ws, &f.id, |d| d.nodes.clone())?;
        for n in nodes {
            if n.id == name {
                return Ok((ws.to_string(), f.id.clone(), n));
            }
            match n.title.as_deref() {
                Some(t) if t == name => exact.push((ws.to_string(), f.id.clone(), n)),
                Some(t) if t.eq_ignore_ascii_case(name) => {
                    loose.push((ws.to_string(), f.id.clone(), n))
                }
                _ => {}
            }
        }
    }
    let hits = if exact.is_empty() { loose } else { exact };
    match hits.len() {
        1 => Ok(hits.into_iter().next().expect("one")),
        0 => Err(format!(
            "CANVAS_NOT_FOUND: no node '{name}' in workspace {}",
            meta.name
        )),
        k => Err(format!(
            "CANVAS_AMBIGUOUS: {k} nodes are titled '{name}' in {}; pass the node id",
            meta.name
        )),
    }
}

fn workspace_by(s: &CanvasStore, key: &str) -> Option<String> {
    let key = key.trim();
    s.workspaces()
        .into_iter()
        .find(|w| w.workspace.id == key || w.workspace.name.eq_ignore_ascii_case(key))
        .map(|w| w.workspace.id)
}

fn floor_by(s: &CanvasStore, ws: &str, key: &str) -> Option<String> {
    let key = key.trim();
    s.workspace(ws).ok().and_then(|m| {
        m.floors
            .iter()
            .find(|f| f.id == key || f.name.eq_ignore_ascii_case(key))
            .map(|f| f.id.clone())
    })
}

pub fn locate_end(
    s: &CanvasStore,
    a: &Value,
    key: &str,
) -> Result<(String, String, CanvasNode), String> {
    let prefer = st(a, "workspaceId").map(|_| ws_of(a));
    let direct = locate_in(s, key, prefer.as_deref());
    if direct.is_ok() {
        return direct;
    }
    if let Some((name, place)) = key.rsplit_once(" @ ").or_else(|| key.rsplit_once('@')) {
        let (name, place) = (name.trim(), place.trim());
        if let Some(ws) = workspace_by(s, place) {
            return find_in(s, &ws, None, name);
        }
        let ws = prefer.clone().unwrap_or_else(|| ws_of(a));
        if let Some(f) = floor_by(s, &ws, place) {
            return find_in(s, &ws, Some(&f), name);
        }
    }
    if let Some((place, name)) = key.split_once('/') {
        let (place, name) = (place.trim(), name.trim());
        let ws = prefer.clone().unwrap_or_else(|| ws_of(a));
        if let Some(f) = floor_by(s, &ws, place) {
            return find_in(s, &ws, Some(&f), name);
        }
        if let Some(w) = workspace_by(s, place) {
            if let Some((fl, n2)) = name.split_once('/') {
                if let Some(f) = floor_by(s, &w, fl) {
                    return find_in(s, &w, Some(&f), n2.trim());
                }
            }
            return find_in(s, &w, None, name);
        }
    }
    direct
}

pub fn link(
    a: &Value,
    from: (String, String, CanvasNode),
    to: (String, String, CanvasNode),
) -> Result<Value, String> {
    let perms = match a.get("perms") {
        Some(_) => list(a, "perms"),
        None => vec![],
    };
    let end = |(ws, floor, node): (String, String, CanvasNode)| crate::canvas::link::End {
        ws,
        floor,
        node,
    };
    let r = crate::canvas::link::link(&end(from), &end(to), &perms, &super::who::origin())?;
    Ok(json!({
        "connection": {
            "id": r["from"]["cordId"],
            "kind": r["kind"],
            "from": r["from"]["nodeId"],
            "to": r["from"]["connectorId"],
        },
        "pair": r,
    }))
}
