use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::layout;
use super::model::{
    default_size, new_id, AgentKind, CanvasNode, Connection, ConnectionKind, FloorDoc, NodeKind,
    Point,
};
use super::ops::Op;

pub const NOTIFY_MAX: usize = 500;
pub const NOT_LEAD: &str =
    "CANVAS_FORBIDDEN: This terminal is not the Lead. Turn on Lead Mode on it first.";
pub const RECRUIT_PERMS: &[&str] = &["ask", "check"];

pub fn is_lead(node: &CanvasNode) -> bool {
    node.is_terminal()
        && node
            .data
            .get("isManager")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

pub fn name_of(node: &CanvasNode) -> String {
    node.title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| match node.kind {
            NodeKind::Note => "Note".into(),
            NodeKind::Portal => "Portal".into(),
            _ => "Terminal".into(),
        })
}

pub fn unique_name(taken: &HashSet<String>, base: &str) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "Recruit" } else { base };
    let lower: HashSet<String> = taken.iter().map(|s| s.to_lowercase()).collect();
    if !lower.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !lower.contains(&n.to_lowercase()))
        .expect("infinite")
}

pub fn names_on(doc: &FloorDoc) -> HashSet<String> {
    doc.nodes
        .iter()
        .filter(|n| n.is_on_canvas())
        .map(name_of)
        .collect()
}

pub fn team(doc: &FloorDoc, lead: &str) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::from([lead.to_string()]);
    let mut q: VecDeque<(String, u8)> = VecDeque::from([(lead.to_string(), 0)]);
    while let Some((id, depth)) = q.pop_front() {
        if depth >= 2 {
            continue;
        }
        for c in doc.connections.iter().filter(|c| c.touches(&id)) {
            if let Some(other) = c.other_end(&id) {
                if seen.insert(other.to_string()) {
                    q.push_back((other.to_string(), depth + 1));
                }
            }
        }
    }
    seen
}

pub fn find<'a>(
    doc: &'a FloorDoc,
    name: &str,
    within: Option<&HashSet<String>>,
) -> Result<&'a CanvasNode, String> {
    let hits: Vec<&CanvasNode> = doc
        .nodes
        .iter()
        .filter(|n| within.is_none_or(|w| w.contains(&n.id)))
        .filter(|n| name_of(n) == name.trim())
        .collect();
    match hits.len() {
        0 => Err(format!(
            "CANVAS_NOT_FOUND: no node named '{name}' in your team (run `list`)"
        )),
        1 => Ok(hits[0]),
        _ => Err(format!(
            "CANVAS_INVALID: Multiple nodes named '{name}'. Rename them to have unique names."
        )),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecruitSpec {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub role_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub preset_id: Option<String>,
}

pub fn recruit_node(
    lead: Option<&CanvasNode>,
    spec: &RecruitSpec,
    name: &str,
    at: Point,
    z: f64,
) -> CanvasNode {
    let size = lead.map(|m| (m.w, m.h)).unwrap_or_else(|| {
        let s = default_size(&NodeKind::Terminal);
        (s.w, s.h)
    });
    let mut data: Map<String, Value> = Map::new();
    let parent_agent = lead.and_then(|m| m.data_str("agent")).unwrap_or("claude");
    let agent = spec
        .agent
        .as_deref()
        .map(|a| AgentKind::parse(a.trim()))
        .filter(AgentKind::is_known);
    let own_command = spec
        .command
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(agent_of_command);
    let same_agent = agent.is_none() && own_command.is_none();
    data.insert("terminalId".into(), json!(new_id()));
    data.insert(
        "agent".into(),
        json!(agent
            .map(|a| a.as_str().to_string())
            .or(own_command.map(str::to_string))
            .unwrap_or_else(|| parent_agent.to_string())),
    );
    let command = spec.command.clone().or_else(|| {
        if same_agent {
            lead.and_then(|m| m.data_str("command")).map(str::to_string)
        } else {
            None
        }
    });
    if let Some(c) = command.filter(|c| !c.trim().is_empty()) {
        data.insert("command".into(), json!(c));
    }
    let cwd = spec
        .cwd
        .clone()
        .or_else(|| lead.and_then(|m| m.data_str("cwd")).map(str::to_string));
    if let Some(c) = cwd {
        data.insert("cwd".into(), json!(c));
    }
    if let Some(r) = &spec.role_id {
        data.insert("roleId".into(), json!(r));
    }
    if let Some(p) = &spec.preset_id {
        data.insert("presetId".into(), json!(p));
    }
    CanvasNode {
        id: new_id(),
        kind: lead.map(|m| m.kind.clone()).unwrap_or(NodeKind::Terminal),
        x: at.x,
        y: at.y,
        w: size.0,
        h: size.1,
        z,
        title: Some(name.to_string()),
        subtitle: None,
        icon: None,
        color: None,
        group_id: None,
        locked: false,
        stack_id: None,
        data: Value::Object(data),
    }
}

pub fn cord(from: &str, to: &str) -> Connection {
    Connection {
        id: new_id(),
        from: from.to_string(),
        to: to.to_string(),
        kind: ConnectionKind::Ask,
        perms: RECRUIT_PERMS.iter().map(|s| s.to_string()).collect(),
        condition: None,
        verify: None,
        label: None,
        points: Vec::new(),
        from_note_id: None,
        to_note_id: None,
        is_active: None,
    }
}

pub fn recruit_ops(
    doc: &FloorDoc,
    lead: &CanvasNode,
    specs: &[RecruitSpec],
) -> (Vec<Op>, Vec<(String, String)>) {
    let mut taken = names_on(doc);
    let spots = layout::place_recruits(doc, &lead.rect(), specs.len(), (lead.w, lead.h));
    let z0 = doc.max_z() + 1.0;
    let mut ops = Vec::new();
    let mut out = Vec::new();
    for (i, (spec, at)) in specs.iter().zip(spots).enumerate() {
        let name = unique_name(&taken, spec.name.as_deref().unwrap_or("Recruit"));
        taken.insert(name.clone());
        let node = recruit_node(Some(lead), spec, &name, at, z0 + i as f64);
        out.push((node.id.clone(), name));
        let c = cord(&lead.id, &node.id);
        ops.push(Op::AddNode { node });
        ops.push(Op::AddConnection { connection: c });
    }
    (ops, out)
}

pub fn swap_agent_patch(agent: &str, command: Option<&str>) -> Result<Value, String> {
    let a = AgentKind::parse(agent.trim());
    if !a.is_known() {
        return Err(format!(
            "CANVAS_INVALID: unknown agent '{agent}' ({})",
            AgentKind::KNOWN.join(", ")
        ));
    }
    Ok(json!({
        "data": {
            "agent": a.as_str(),
            "command": command.filter(|c| !c.trim().is_empty()).map(Value::from).unwrap_or(Value::Null),
            "terminalId": new_id(),
        }
    }))
}

pub fn split_command(command: &str) -> Vec<String> {
    shell_words::split(command.trim())
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect())
}

pub fn agent_of_command(command: &str) -> &'static str {
    let words = split_command(command);
    let prog = words.first().map(String::as_str).unwrap_or("");
    let base = std::path::Path::new(prog)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    match base.as_str() {
        "claude" => "claude",
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => "opencode",
        "ollama" => "ollama",
        _ => "shell",
    }
}

pub fn swap_command_patch(command: &str) -> Result<Value, String> {
    let c = command.trim();
    if c.is_empty() {
        return Err("CANVAS_INVALID: empty --command".into());
    }
    Ok(json!({
        "data": {
            "agent": agent_of_command(c),
            "command": c,
            "presetId": Value::Null,
            "terminalId": new_id(),
        }
    }))
}

pub fn swap_role_patch(role_id: Option<&str>) -> Value {
    json!({
        "data": {
            "roleId": role_id.map(Value::from).unwrap_or(Value::Null),
            "terminalId": new_id(),
        }
    })
}
