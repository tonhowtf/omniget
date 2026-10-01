use super::model::{
    edge_point, round_points, validate_connection, validate_drawing, validate_node, CableTie,
    Camera, CanvasNode, Connection, Drawing, DrawingAnchor, FloorDoc, Group, GroupTarget, NodeKind,
    Point, Size, StackData, ZOOM_MAX, ZOOM_MIN,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    AddNode {
        node: CanvasNode,
    },
    UpdateNode {
        id: String,
        patch: Map<String, Value>,
    },
    RemoveNode {
        id: String,
    },
    AddConnection {
        connection: Connection,
    },
    UpdateConnection {
        id: String,
        patch: Map<String, Value>,
    },
    RemoveConnection {
        id: String,
    },
    AddDrawing {
        drawing: Drawing,
    },
    UpdateDrawing {
        id: String,
        patch: Map<String, Value>,
    },
    RemoveDrawing {
        id: String,
    },
    SetGroup {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<Group>,
    },
    SetCableTie {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tie: Option<CableTie>,
    },
    SetCamera {
        camera: Camera,
    },
}

impl Op {
    pub fn is_structural(&self) -> bool {
        matches!(self, Op::AddNode { .. } | Op::RemoveNode { .. })
    }

    pub fn is_camera(&self) -> bool {
        matches!(self, Op::SetCamera { .. })
    }

    pub fn update_node(id: impl Into<String>, patch: Value) -> Op {
        Op::UpdateNode {
            id: id.into(),
            patch: patch.as_object().cloned().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub ops: Vec<Op>,
    pub inverse: Vec<Op>,
}

pub fn apply_batch(doc: &mut FloorDoc, ops: &[Op]) -> Result<Applied, String> {
    let mut work = doc.clone();
    let mut out = Applied::default();
    let mut inverse_rev: Vec<Op> = Vec::new();
    for op in ops {
        for step in expand(&work, op)? {
            let inv = apply_one(&mut work, &step)?;
            out.ops.push(step);
            inverse_rev.extend(inv.into_iter().rev());
        }
    }
    inverse_rev.reverse();
    out.inverse = inverse_rev;
    *doc = work;
    Ok(out)
}

fn expand(doc: &FloorDoc, op: &Op) -> Result<Vec<Op>, String> {
    let mut steps = Vec::new();
    match op {
        Op::RemoveNode { id } => {
            let node = doc
                .node(id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {id}"))?;
            let cords: Vec<String> = doc
                .connections
                .iter()
                .filter(|c| c.touches(id))
                .map(|c| c.id.clone())
                .collect();
            steps.extend(connection_removal(doc, &cords));
            if let Some(stack_id) = &node.stack_id {
                if let Some(stack) = doc.node(stack_id) {
                    let mut data: StackData = stack.data_as().unwrap_or_default();
                    data.tabs.retain(|t| t != id);
                    let mut p = Map::new();
                    p.insert("tabs".into(), serde_json::json!(data.tabs));
                    if data.front_note_id.as_deref() == Some(id.as_str()) {
                        p.insert(
                            "frontNoteId".into(),
                            data.tabs
                                .first()
                                .map(|t| Value::String(t.clone()))
                                .unwrap_or(Value::Null),
                        );
                    }
                    let mut patch = Map::new();
                    patch.insert("data".into(), Value::Object(p));
                    steps.push(Op::UpdateNode {
                        id: stack_id.clone(),
                        patch,
                    });
                }
            }
            if node.kind == NodeKind::Stack {
                for n in doc
                    .nodes
                    .iter()
                    .filter(|n| n.stack_id.as_deref() == Some(id.as_str()))
                {
                    steps.push(Op::update_node(
                        n.id.clone(),
                        serde_json::json!({ "stackId": null }),
                    ));
                }
            }
            steps.push(op.clone());
        }
        Op::RemoveConnection { id } => {
            steps.extend(connection_removal(doc, std::slice::from_ref(id)));
        }
        Op::SetGroup { id, group: None } => {
            if let Some(g) = doc.groups.iter().find(|g| &g.id == id) {
                let clear = serde_json::json!({ "groupId": null });
                if g.target == GroupTarget::Drawings {
                    for d in doc
                        .drawings
                        .iter()
                        .filter(|d| d.group_id.as_deref() == Some(id.as_str()))
                    {
                        steps.push(Op::UpdateDrawing {
                            id: d.id.clone(),
                            patch: clear.as_object().cloned().unwrap_or_default(),
                        });
                    }
                } else {
                    for n in doc
                        .nodes
                        .iter()
                        .filter(|n| n.group_id.as_deref() == Some(id.as_str()))
                    {
                        steps.push(Op::update_node(n.id.clone(), clear.clone()));
                    }
                }
            }
            steps.push(op.clone());
        }
        Op::AddDrawing { drawing } => {
            let mut d = drawing.clone();
            if let Some(ends) = anchored_ends(doc, &d, None) {
                set_arrow_ends(&mut d, ends);
            }
            steps.push(Op::AddDrawing { drawing: d });
        }
        Op::UpdateDrawing { id, patch } => {
            steps.push(op.clone());
            if let Some(cur) = doc.drawing(id) {
                if let Ok((next, _)) = patch_record::<Drawing>(cur, patch, false) {
                    steps.extend(follow_arrows(doc, &next));
                    if let Some(ends) = anchored_ends(doc, &next, None) {
                        let mut snapped = next.clone();
                        set_arrow_ends(&mut snapped, ends);
                        if snapped != next {
                            steps.push(arrow_patch(&snapped));
                        }
                    }
                }
            }
        }
        Op::RemoveDrawing { id } => {
            for d in doc.drawings.iter().filter(|d| &d.id != id) {
                let mut patch = Map::new();
                if d.source_connection.as_ref().map(|a| &a.drawing_id) == Some(id) {
                    patch.insert("sourceConnection".into(), Value::Null);
                }
                if d.target_connection.as_ref().map(|a| &a.drawing_id) == Some(id) {
                    patch.insert("targetConnection".into(), Value::Null);
                }
                if !patch.is_empty() {
                    steps.push(Op::UpdateDrawing {
                        id: d.id.clone(),
                        patch,
                    });
                }
            }
            steps.push(op.clone());
        }
        _ => steps.push(op.clone()),
    }
    Ok(steps)
}

fn anchored_ends(
    doc: &FloorDoc,
    arrow: &Drawing,
    moved: Option<&Drawing>,
) -> Option<(Point, Point)> {
    if arrow.source_connection.is_none() && arrow.target_connection.is_none() {
        return None;
    }
    let b = arrow.bounds();
    let n = arrow.points.len();
    let (sx, sy) = match (arrow.original_size, arrow.size) {
        (Some(o), Some(s)) if o.w > 0.0 && o.h > 0.0 => (s.w / o.w, s.h / o.h),
        _ => (1.0, 1.0),
    };
    let (mut start, mut end) = if n >= 4 {
        (
            Point {
                x: arrow.origin.x + arrow.points[0] * sx,
                y: arrow.origin.y + arrow.points[1] * sy,
            },
            Point {
                x: arrow.origin.x + arrow.points[n - 2] * sx,
                y: arrow.origin.y + arrow.points[n - 1] * sy,
            },
        )
    } else {
        (
            Point { x: b.x, y: b.y },
            Point {
                x: b.x + b.w,
                y: b.y + b.h,
            },
        )
    };
    let shape = |sid: &str| -> Option<Drawing> {
        match moved {
            Some(m) if m.id == sid => Some(m.clone()),
            _ => doc.drawing(sid).cloned(),
        }
    };
    let mut any = false;
    if let Some(a) = &arrow.source_connection {
        if let Some(sh) = shape(&a.drawing_id) {
            start = edge_point(sh.bounds(), &a.edge);
            any = true;
        }
    }
    if let Some(a) = &arrow.target_connection {
        if let Some(sh) = shape(&a.drawing_id) {
            end = edge_point(sh.bounds(), &a.edge);
            any = true;
        }
    }
    any.then_some((start, end))
}

fn set_arrow_ends(d: &mut Drawing, (s, e): (Point, Point)) {
    let ox = s.x.min(e.x);
    let oy = s.y.min(e.y);
    d.origin = Point { x: ox, y: oy };
    d.points = vec![s.x - ox, s.y - oy, e.x - ox, e.y - oy];
    round_points(&mut d.points);
    let size = Size {
        w: (e.x - s.x).abs(),
        h: (e.y - s.y).abs(),
    };
    d.size = Some(size);
    d.original_size = Some(size);
}

fn arrow_patch(d: &Drawing) -> Op {
    let mut patch = Map::new();
    patch.insert("origin".into(), serde_json::json!(d.origin));
    patch.insert("points".into(), serde_json::json!(d.points));
    patch.insert("size".into(), serde_json::json!(d.size));
    patch.insert("originalSize".into(), serde_json::json!(d.original_size));
    Op::UpdateDrawing {
        id: d.id.clone(),
        patch,
    }
}

fn follow_arrows(doc: &FloorDoc, moved: &Drawing) -> Vec<Op> {
    let mut out = Vec::new();
    for d in &doc.drawings {
        if d.id == moved.id {
            continue;
        }
        let hooked =
            |a: &Option<DrawingAnchor>| a.as_ref().is_some_and(|a| a.drawing_id == moved.id);
        if !hooked(&d.source_connection) && !hooked(&d.target_connection) {
            continue;
        }
        if let Some(ends) = anchored_ends(doc, d, Some(moved)) {
            let mut next = d.clone();
            set_arrow_ends(&mut next, ends);
            if next != *d {
                out.push(arrow_patch(&next));
            }
        }
    }
    out
}

fn connection_removal(doc: &FloorDoc, conn_ids: &[String]) -> Vec<Op> {
    let mut steps = Vec::new();
    for tie in doc
        .cable_ties
        .iter()
        .filter(|t| t.member_connection_ids.iter().any(|m| conn_ids.contains(m)))
    {
        let mut t = tie.clone();
        t.member_connection_ids.retain(|m| !conn_ids.contains(m));
        steps.push(Op::SetCableTie {
            id: t.id.clone(),
            tie: if t.member_connection_ids.is_empty() {
                None
            } else {
                Some(t)
            },
        });
    }
    for id in conn_ids {
        steps.push(Op::RemoveConnection { id: id.clone() });
    }
    steps
}

pub fn apply_one(doc: &mut FloorDoc, op: &Op) -> Result<Vec<Op>, String> {
    match op {
        Op::AddNode { node } => {
            validate_node(node).map_err(invalid)?;
            if doc.node(&node.id).is_some() {
                return Err(format!("CANVAS_CONFLICT: node {} already exists", node.id));
            }
            doc.nodes.push(node.clone());
            Ok(vec![Op::RemoveNode {
                id: node.id.clone(),
            }])
        }
        Op::UpdateNode { id, patch } => {
            let idx = doc
                .nodes
                .iter()
                .position(|n| &n.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {id}"))?;
            let (next, inverse) = patch_record(&doc.nodes[idx], patch, true)?;
            let next: CanvasNode = next;
            if next.kind != doc.nodes[idx].kind {
                return Err(invalid(format!("node {id} cannot change kind")));
            }
            validate_node(&next).map_err(invalid)?;
            doc.nodes[idx] = next;
            Ok(vec![Op::UpdateNode {
                id: id.clone(),
                patch: inverse,
            }])
        }
        Op::RemoveNode { id } => {
            let idx = doc
                .nodes
                .iter()
                .position(|n| &n.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {id}"))?;
            if doc.connections.iter().any(|c| c.touches(id)) {
                return Err(invalid(format!("node {id} still has connections")));
            }
            let node = doc.nodes.remove(idx);
            Ok(vec![Op::AddNode { node }])
        }
        Op::AddConnection { connection } => {
            let mut c = connection.clone();
            validate_connection(&c).map_err(invalid)?;
            round_points(&mut c.points);
            if doc.connection(&c.id).is_some() {
                return Err(format!(
                    "CANVAS_CONFLICT: connection {} already exists",
                    c.id
                ));
            }
            for end in [&c.from, &c.to] {
                if doc.node(end).is_none() {
                    return Err(format!("CANVAS_NOT_FOUND: node {end} (connection end)"));
                }
            }
            let id = c.id.clone();
            doc.connections.push(c);
            Ok(vec![Op::RemoveConnection { id }])
        }
        Op::UpdateConnection { id, patch } => {
            let idx = doc
                .connections
                .iter()
                .position(|c| &c.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: connection {id}"))?;
            let (mut next, inverse): (Connection, _) =
                patch_record(&doc.connections[idx], patch, false)?;
            validate_connection(&next).map_err(invalid)?;
            round_points(&mut next.points);
            for end in [&next.from, &next.to] {
                if doc.node(end).is_none() {
                    return Err(format!("CANVAS_NOT_FOUND: node {end} (connection end)"));
                }
            }
            doc.connections[idx] = next;
            Ok(vec![Op::UpdateConnection {
                id: id.clone(),
                patch: inverse,
            }])
        }
        Op::RemoveConnection { id } => {
            let idx = doc
                .connections
                .iter()
                .position(|c| &c.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: connection {id}"))?;
            let connection = doc.connections.remove(idx);
            Ok(vec![Op::AddConnection { connection }])
        }
        Op::AddDrawing { drawing } => {
            let mut d = drawing.clone();
            validate_drawing(&d).map_err(invalid)?;
            round_points(&mut d.points);
            if doc.drawing(&d.id).is_some() {
                return Err(format!("CANVAS_CONFLICT: drawing {} already exists", d.id));
            }
            let id = d.id.clone();
            doc.drawings.push(d);
            Ok(vec![Op::RemoveDrawing { id }])
        }
        Op::UpdateDrawing { id, patch } => {
            let idx = doc
                .drawings
                .iter()
                .position(|d| &d.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: drawing {id}"))?;
            let (mut next, inverse): (Drawing, _) = patch_record(&doc.drawings[idx], patch, false)?;
            validate_drawing(&next).map_err(invalid)?;
            round_points(&mut next.points);
            doc.drawings[idx] = next;
            Ok(vec![Op::UpdateDrawing {
                id: id.clone(),
                patch: inverse,
            }])
        }
        Op::RemoveDrawing { id } => {
            let idx = doc
                .drawings
                .iter()
                .position(|d| &d.id == id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: drawing {id}"))?;
            let drawing = doc.drawings.remove(idx);
            Ok(vec![Op::AddDrawing { drawing }])
        }
        Op::SetGroup { id, group } => {
            if let Some(g) = group {
                if &g.id != id {
                    return Err(invalid(format!("group id {} does not match {id}", g.id)));
                }
                if !g.target.is_known() {
                    return Err(invalid(format!("unknown group target '{}'", g.target)));
                }
            }
            let prev_idx = doc.groups.iter().position(|g| &g.id == id);
            let prev = prev_idx.map(|i| doc.groups[i].clone());
            match (prev_idx, group) {
                (Some(i), Some(g)) => doc.groups[i] = g.clone(),
                (Some(i), None) => {
                    doc.groups.remove(i);
                }
                (None, Some(g)) => doc.groups.push(g.clone()),
                (None, None) => {}
            }
            Ok(vec![Op::SetGroup {
                id: id.clone(),
                group: prev,
            }])
        }
        Op::SetCableTie { id, tie } => {
            if let Some(t) = tie {
                if &t.id != id {
                    return Err(invalid(format!(
                        "cable tie id {} does not match {id}",
                        t.id
                    )));
                }
                for m in &t.member_connection_ids {
                    if doc.connection(m).is_none() {
                        return Err(format!(
                            "CANVAS_NOT_FOUND: connection {m} (cable tie member)"
                        ));
                    }
                }
            }
            let prev_idx = doc.cable_ties.iter().position(|t| &t.id == id);
            let prev = prev_idx.map(|i| doc.cable_ties[i].clone());
            match (prev_idx, tie) {
                (Some(i), Some(t)) => doc.cable_ties[i] = t.clone(),
                (Some(i), None) => {
                    doc.cable_ties.remove(i);
                }
                (None, Some(t)) => doc.cable_ties.push(t.clone()),
                (None, None) => {}
            }
            Ok(vec![Op::SetCableTie {
                id: id.clone(),
                tie: prev,
            }])
        }
        Op::SetCamera { camera } => {
            if !camera.x.is_finite() || !camera.y.is_finite() || !camera.zoom.is_finite() {
                return Err(invalid("camera has non-finite values".into()));
            }
            let prev = doc.camera;
            doc.camera = Camera {
                x: camera.x,
                y: camera.y,
                zoom: camera.zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            };
            Ok(vec![Op::SetCamera { camera: prev }])
        }
    }
}

fn invalid(msg: String) -> String {
    format!("CANVAS_INVALID: {msg}")
}

fn patch_record<T>(
    record: &T,
    patch: &Map<String, Value>,
    merge_data: bool,
) -> Result<(T, Map<String, Value>), String>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let mut obj = match serde_json::to_value(record).map_err(|e| e.to_string())? {
        Value::Object(m) => m,
        _ => return Err("record is not an object".into()),
    };
    let mut inverse = Map::new();
    for (k, v) in patch {
        if k == "id" {
            if obj.get("id") != Some(v) {
                return Err(invalid("id cannot change".into()));
            }
            continue;
        }
        if merge_data && k == "data" {
            if let Value::Object(dp) = v {
                let data = obj
                    .entry("data")
                    .or_insert_with(|| Value::Object(Map::new()));
                if !data.is_object() {
                    *data = Value::Object(Map::new());
                }
                let data = data.as_object_mut().expect("object");
                let mut inv = Map::new();
                for (dk, dv) in dp {
                    inv.insert(dk.clone(), data.get(dk).cloned().unwrap_or(Value::Null));
                    if dv.is_null() {
                        data.remove(dk);
                    } else {
                        data.insert(dk.clone(), dv.clone());
                    }
                }
                inverse.insert("data".into(), Value::Object(inv));
                continue;
            }
        }
        inverse.insert(k.clone(), obj.get(k).cloned().unwrap_or(Value::Null));
        if v.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), v.clone());
        }
    }
    let next: T = serde_json::from_value(Value::Object(obj))
        .map_err(|e| invalid(format!("patch does not fit: {e}")))?;
    Ok((next, inverse))
}
