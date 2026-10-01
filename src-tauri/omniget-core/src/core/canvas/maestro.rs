//! Maestro Mode (Maestri, study 95 §2 / N): a terminal flagged `isManager`
//! may recruit, connect, swap, dismiss, schedule routines and create
//! workspaces and floors. The gate is checked on the server for every call;
//! this file holds the pure parts (flag, team tree, names, recruit nodes).

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::layout;
use super::model::{
    default_size, new_id, AgentKind, CanvasNode, Connection, ConnectionKind, FloorDoc, NodeKind,
    Point,
};
use super::ops::Op;

/// `maestro.notify` message cap (Maestri: 500 characters).
pub const NOTIFY_MAX: usize = 500;
pub const NOT_MAESTRO: &str =
    "CANVAS_FORBIDDEN: This terminal is not the Maestro. Turn on Maestro Mode on it first.";
/// Perms of the cord between a Maestro and a recruit.
pub const RECRUIT_PERMS: &[&str] = &["ask", "check"];

pub fn is_maestro(node: &CanvasNode) -> bool {
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

/// `base`, or `base (2)`, `(3)`… when taken (case-insensitive).
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

/// The Maestro's team tree: itself, what it is connected to, and what its
/// recruits are connected to (their notes, portals and peers).
pub fn team(doc: &FloorDoc, maestro: &str) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::from([maestro.to_string()]);
    let mut q: VecDeque<(String, u8)> = VecDeque::from([(maestro.to_string(), 0)]);
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

/// Finds a node by exact name among `within` (all nodes when `None`).
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
    /// Agent preset (`claude`, `codex`…); default: the Maestro's own.
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

/// A recruit node: a copy of the Maestro's agent unless told otherwise,
/// never a Maestro itself, with a fresh PTY id.
pub fn recruit_node(
    maestro: Option<&CanvasNode>,
    spec: &RecruitSpec,
    name: &str,
    at: Point,
    z: f64,
) -> CanvasNode {
    let size = maestro.map(|m| (m.w, m.h)).unwrap_or_else(|| {
        let s = default_size(&NodeKind::Terminal);
        (s.w, s.h)
    });
    let mut data: Map<String, Value> = Map::new();
    let parent_agent = maestro
        .and_then(|m| m.data_str("agent"))
        .unwrap_or("claude");
    let agent = spec
        .agent
        .as_deref()
        .map(|a| AgentKind::parse(a.trim()))
        .filter(AgentKind::is_known);
    // `--command parity-stub` alone is that program, not the Maestro's agent
    // running it: the agent follows the program (shell for anything else).
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
            maestro
                .and_then(|m| m.data_str("command"))
                .map(str::to_string)
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
        .or_else(|| maestro.and_then(|m| m.data_str("cwd")).map(str::to_string));
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
        kind: maestro
            .map(|m| m.kind.clone())
            .unwrap_or(NodeKind::Terminal),
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

/// Ops that add recruits in an even row under the Maestro, each tied to it.
/// Returns the ops and `(node id, name)` per recruit.
pub fn recruit_ops(
    doc: &FloorDoc,
    maestro: &CanvasNode,
    specs: &[RecruitSpec],
) -> (Vec<Op>, Vec<(String, String)>) {
    let mut taken = names_on(doc);
    let spots = layout::place_recruits(doc, &maestro.rect(), specs.len(), (maestro.w, maestro.h));
    let z0 = doc.max_z() + 1.0;
    let mut ops = Vec::new();
    let mut out = Vec::new();
    for (i, (spec, at)) in specs.iter().zip(spots).enumerate() {
        let name = unique_name(&taken, spec.name.as_deref().unwrap_or("Recruit"));
        taken.insert(name.clone());
        let node = recruit_node(Some(maestro), spec, &name, at, z0 + i as f64);
        out.push((node.id.clone(), name));
        let c = cord(&maestro.id, &node.id);
        ops.push(Op::AddNode { node });
        ops.push(Op::AddConnection { connection: c });
    }
    (ops, out)
}

/// Patch that restarts a terminal with another agent (same node, position,
/// cords and routines; a new PTY id makes the canvas start a new process).
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

/// A command line typed by a person or an agent (`recruit --command`,
/// a node's `data.command`) as argv, with POSIX quoting:
/// `bash -c "echo X; exec bash"` is three words. Unbalanced quotes fall
/// back to whitespace.
pub fn split_command(command: &str) -> Vec<String> {
    shell_words::split(command.trim())
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect())
}

/// The built-in agent a command line runs (`claude --model x` = claude),
/// `shell` for any other program.
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

/// Patch that restarts a terminal on its own command line (`--replace X
/// --command C` without an agent): the agent follows the program.
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

/// Patch that swaps (or clears) the role and restarts the process.
pub fn swap_role_patch(role_id: Option<&str>) -> Value {
    json!({
        "data": {
            "roleId": role_id.map(Value::from).unwrap_or(Value::Null),
            "terminalId": new_id(),
        }
    })
}
