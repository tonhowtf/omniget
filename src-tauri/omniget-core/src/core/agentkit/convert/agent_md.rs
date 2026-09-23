//! Agents as Markdown with YAML frontmatter in each tool's own dialect (plan
//! §4.3, estudo 06 §0C.2): Gemini, Qwen, OpenCode/Kilo, Cursor, Copilot
//! `.agent.md`, Devin, Droid, Cline, Augment, Antigravity, OpenHands and Rovo.
//!
//! One converter per format id; each maps the Claude subagent fields it can
//! express and records the rest as losses. `import` reads the tool's files back
//! into canonical (Claude-shaped) agents.

use serde_json::{json, Map, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

/// Format ids served by [`MdAgentConverter`].
pub const FORMATS: &[&str] = &[
    "gemini_md",
    "qwen_md",
    "opencode_md",
    "cursor_md",
    "copilot_agent_md",
    "devin_md",
    "droid_md",
    "cline_md",
    "augment_md",
    "antigravity",
    "openhands_md",
    "rovo",
];

/// Markdown agent converter for one format id.
pub struct MdAgentConverter {
    pub format: &'static str,
}

/// OpenCode permission keys and the Claude tools each one covers.
const OPENCODE_KEYS: &[(&str, &[&str])] = &[
    ("read", &["Read"]),
    ("edit", &["Edit", "Write"]),
    ("glob", &["Glob"]),
    ("grep", &["Grep"]),
    ("list", &["Glob"]),
    ("bash", &["Bash"]),
    ("task", &["Agent"]),
    ("webfetch", &["WebFetch"]),
    ("websearch", &["WebSearch"]),
    ("todowrite", &["TodoWrite"]),
];

fn norm(t: &str) -> &str {
    match tool_base(t).0 {
        "Task" => "Agent",
        "MultiEdit" | "NotebookEdit" => "Edit",
        "LS" => "Glob",
        x => x,
    }
}

/// Gemini agent names: lowercase letters, digits, `-` and `_`.
fn gemini_name(n: &str) -> String {
    n.chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Claude MCP record → Gemini (`httpUrl` for streamable HTTP, `url` for SSE).
fn gemini_mcp(v: &Value) -> Value {
    let Some(m) = v.as_object() else {
        return v.clone();
    };
    let mut o = m.clone();
    let t = o.remove("type");
    if let Some(u) = o.remove("url") {
        let key = if t.as_ref().and_then(|x| x.as_str()) == Some("sse") {
            "url"
        } else {
            "httpUrl"
        };
        o.insert(key.into(), u);
    }
    Value::Object(o)
}

fn gemini_mcp_back(v: &Value) -> Value {
    let Some(m) = v.as_object() else {
        return v.clone();
    };
    let mut o = m.clone();
    if let Some(u) = o.remove("httpUrl") {
        o.insert("type".into(), json!("http"));
        o.insert("url".into(), u);
    } else if o.contains_key("url") {
        o.insert("type".into(), json!("sse"));
    }
    Value::Object(o)
}

fn list(v: &[String]) -> Value {
    Value::Array(v.iter().map(|s| json!(s)).collect())
}

impl MdAgentConverter {
    fn file_name(&self, name: &str) -> String {
        if self.format == "copilot_agent_md" {
            format!("{name}.agent.md")
        } else {
            format!("{name}.md")
        }
    }

    /// Text of the agent file, losses and notes.
    pub fn render(
        &self,
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>, Vec<String>) {
        let origin = c.origin_tool.as_str();
        let (tools, mut losses) = claude_tools(&a.tools, origin);
        let (dis, l2) = claude_tools(&a.disallowed_tools, origin);
        losses.extend(l2);
        let (model, mloss) = model_for(a.model.as_deref(), origin, target);
        losses.extend(mloss);
        let mut notes: Vec<String> = Vec::new();
        let mut fm: Vec<(String, Value)> = Vec::new();
        let mut keep: Vec<&str> = Vec::new();
        let mut push = |k: &str, v: Value| fm.push((k.to_string(), v));
        let body = a.prompt.clone();

        match self.format {
            "gemini_md" => {
                push("name", json!(gemini_name(name)));
                push("description", json!(a.description));
                push("kind", json!("local"));
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    if !m.is_empty() {
                        push("tools", list(&m));
                    }
                    keep.push("tools");
                }
                if !a.mcp_servers.is_empty() {
                    let mcp: Map<String, Value> = a
                        .mcp_servers
                        .iter()
                        .map(|(k, v)| (k.clone(), gemini_mcp(v)))
                        .collect();
                    push("mcpServers", Value::Object(mcp));
                    keep.push("mcpServers");
                }
                if let Some(m) = &model {
                    push("model", json!(m));
                }
                if let Some(n) = a.max_turns {
                    push("max_turns", json!(n));
                    keep.push("maxTurns");
                }
            }
            "qwen_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if let Some(m) = &model {
                    push("model", json!(m));
                }
                if let Some(p) = a.permission_mode.as_deref() {
                    let mode = match p {
                        "default" => Some("default"),
                        "plan" => Some("plan"),
                        "acceptEdits" | "auto" => Some("auto-edit"),
                        "bypassPermissions" | "dontAsk" => Some("yolo"),
                        _ => None,
                    };
                    if let Some(m) = mode {
                        push("approvalMode", json!(m));
                        keep.push("permissionMode");
                    }
                }
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("tools", list(&m));
                    keep.push("tools");
                }
                if !dis.is_empty() {
                    let (m, l) = map_tools(&dis, target);
                    losses.extend(l);
                    push("disallowedTools", list(&m));
                    keep.push("disallowedTools");
                }
                if let Some(n) = a.max_turns {
                    push("maxTurns", json!(n));
                    keep.push("maxTurns");
                }
                if let Some(col) = &a.color {
                    push("color", json!(col));
                    keep.push("color");
                }
                if !a.mcp_servers.is_empty() {
                    push("mcpServers", Value::Object(a.mcp_servers.clone()));
                    keep.push("mcpServers");
                }
                if !a.hooks.is_null() {
                    push("hooks", a.hooks.clone());
                    keep.push("hooks");
                }
            }
            "opencode_md" => {
                push("description", json!(a.description));
                push("mode", json!("subagent"));
                if let Some(m) = &model {
                    push("model", json!(m));
                }
                if let Some(n) = a.max_turns {
                    push("steps", json!(n));
                    keep.push("maxTurns");
                }
                if let Some(col) = &a.color {
                    push("color", json!(color_hex(col)));
                    keep.push("color");
                }
                let mut perm = Map::new();
                if !tools.is_empty() {
                    keep.push("tools");
                    for (key, covers) in OPENCODE_KEYS {
                        if !tools.iter().any(|t| covers.contains(&norm(t))) {
                            perm.insert((*key).into(), json!("deny"));
                        }
                    }
                    if tools.iter().any(|t| t.starts_with("mcp__")) {
                        push_unique(&mut losses, "MCP tools in the agent tool list (OpenCode grants them by server config)");
                    }
                }
                if !dis.is_empty() {
                    keep.push("disallowedTools");
                    for (key, covers) in OPENCODE_KEYS {
                        if dis.iter().any(|t| covers.contains(&norm(t))) {
                            perm.insert((*key).into(), json!("deny"));
                        }
                    }
                }
                match a.permission_mode.as_deref() {
                    Some("plan") => {
                        perm.insert("edit".into(), json!("deny"));
                        keep.push("permissionMode");
                    }
                    Some("acceptEdits") => {
                        perm.entry("edit").or_insert(json!("allow"));
                        keep.push("permissionMode");
                    }
                    _ => {}
                }
                if !perm.is_empty() {
                    push("permission", Value::Object(perm));
                }
            }
            "cursor_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                push(
                    "model",
                    json!(model.clone().unwrap_or_else(|| "inherit".into())),
                );
                let ro = is_read_only(&tools) || a.permission_mode.as_deref() == Some("plan");
                if ro {
                    push("readonly", json!(true));
                    keep.push("tools");
                    if a.permission_mode.as_deref() == Some("plan") {
                        keep.push("permissionMode");
                    }
                }
                if let Some(b) = a.background {
                    push("is_background", json!(b));
                    keep.push("background");
                }
            }
            "copilot_agent_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if !a.tools.is_empty() {
                    let ids: Vec<String> = if origin == "copilot" {
                        a.tools.clone()
                    } else {
                        let mut v = Vec::new();
                        for t in &tools {
                            match claude_to_copilot(t) {
                                Some(x) => push_unique(&mut v, x),
                                None => push_unique(
                                    &mut losses,
                                    format!("tool `{t}` (no Copilot alias)"),
                                ),
                            }
                        }
                        v
                    };
                    push("tools", list(&ids));
                    keep.push("tools");
                }
                if origin == "copilot" {
                    if let Some(m) = &a.model {
                        push("model", json!(m));
                    }
                } else if let Some(m) = &model {
                    push("model", json!(m));
                }
                if !a.mcp_servers.is_empty() {
                    let mcp: Map<String, Value> = a
                        .mcp_servers
                        .iter()
                        .map(|(k, v)| (k.clone(), plain_mcp(v)))
                        .collect();
                    push("mcp-servers", Value::Object(mcp));
                    keep.push("mcpServers");
                    notes.push("mcp-servers in a custom agent are read by the Copilot CLI and cloud agent, not VS Code".into());
                }
                if origin == "copilot" {
                    for (k, v) in &a.extra {
                        push(k, v.clone());
                        keep.push(k.as_str());
                    }
                }
            }
            "devin_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if let Some(m) = &model {
                    push("model", json!(m));
                }
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("allowed-tools", list(&m));
                    keep.push("tools");
                }
            }
            "droid_md" => {
                push("name", json!(parse::sanitize_name(name)));
                push("description", json!(a.description));
                push(
                    "model",
                    json!(model.clone().unwrap_or_else(|| "inherit".into())),
                );
                if let Some(e) = a.effort.as_deref() {
                    let (v, lossy) = match e {
                        "low" | "medium" | "high" => (e, false),
                        _ => ("high", true),
                    };
                    push("reasoningEffort", json!(v));
                    keep.push("effort");
                    if lossy {
                        losses.push(format!("effort `{e}` (Droid tops out at high)"));
                    }
                }
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("tools", list(&m));
                    keep.push("tools");
                }
                if !a.mcp_servers.is_empty() {
                    let names: Vec<String> = a.mcp_servers.keys().cloned().collect();
                    push("mcpServers", list(&names));
                    keep.push("mcpServers");
                    if a.mcp_servers.values().any(|v| v.is_object()) {
                        losses.push("inline MCP server configs (Droid references servers by name; add them to .factory/mcp.json)".into());
                    }
                }
            }
            "cline_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if !a.skills.is_empty() {
                    push("skills", list(&a.skills));
                    keep.push("skills");
                }
                if let Some(m) = &model {
                    if m.starts_with("claude") {
                        push("providerId", json!("anthropic"));
                    }
                    push("modelId", json!(m));
                }
                if let Some(n) = a.max_turns {
                    push("maxIterations", json!(n));
                    keep.push("maxTurns");
                }
                if !tools.is_empty() {
                    push_unique(
                        &mut losses,
                        "agent tool list (Cline agent tool ids are not documented; the agent gets Cline's defaults)",
                    );
                    keep.push("tools");
                }
            }
            "augment_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if let Some(col) = &a.color {
                    push("color", json!(col));
                    keep.push("color");
                }
                if let Some(m) = &model {
                    push("model", json!(m));
                }
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("tools", list(&m));
                    keep.push("tools");
                } else if !dis.is_empty() {
                    let (m, l) = map_tools(&dis, target);
                    losses.extend(l);
                    push("disabled_tools", list(&m));
                    keep.push("disallowedTools");
                }
            }
            "antigravity" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if let Some(m) = &model {
                    push("model", json!(m));
                }
            }
            "openhands_md" => {
                push("name", json!(name));
                push("description", json!(a.description));
                push(
                    "model",
                    json!(model.clone().unwrap_or_else(|| "inherit".into())),
                );
                if let Some(col) = &a.color {
                    push("color", json!(col));
                    keep.push("color");
                }
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("tools", list(&m));
                    keep.push("tools");
                }
                if !a.skills.is_empty() {
                    push("skills", list(&a.skills));
                    keep.push("skills");
                }
                if let Some(n) = a.max_turns {
                    push("max_iteration_per_run", json!(n));
                    keep.push("maxTurns");
                }
                if !a.mcp_servers.is_empty() {
                    let mcp: Map<String, Value> = a
                        .mcp_servers
                        .iter()
                        .map(|(k, v)| (k.clone(), plain_mcp(v)))
                        .collect();
                    push("mcp_servers", Value::Object(mcp));
                    keep.push("mcpServers");
                }
                if !a.hooks.is_null() {
                    push("hooks", a.hooks.clone());
                    keep.push("hooks");
                }
            }
            "rovo" => {
                push("name", json!(name));
                push("description", json!(a.description));
                if !tools.is_empty() {
                    let (m, l) = map_tools(&tools, target);
                    losses.extend(l);
                    push("tools", list(&m));
                    keep.push("tools");
                }
            }
            _ => {}
        }
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        let mut uniq: Vec<String> = Vec::new();
        for l in losses {
            push_unique(&mut uniq, l);
        }
        (parse::render_frontmatter(&fm, &body), uniq, notes)
    }

    /// Frontmatter of the tool's file → canonical agent spec.
    pub fn read_back(
        &self,
        fm: &Map<String, Value>,
        body: &str,
        target: &TargetAdapter,
        fallback_name: &str,
    ) -> AgentSpec {
        let mut a = AgentSpec {
            name: str_of(fm, &["name"]).unwrap_or_else(|| fallback_name.to_string()),
            description: str_of(fm, &["description"]).unwrap_or_default(),
            prompt: body.to_string(),
            ..Default::default()
        };
        let names = |k: &[&str]| -> Vec<String> {
            k.iter()
                .find_map(|x| fm.get(*x))
                .map(|v| parse::string_list(Some(v)))
                .unwrap_or_default()
        };
        match self.format {
            "gemini_md" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
                a.model = str_of(fm, &["model"]);
                a.max_turns = u32_of(fm, &["max_turns", "maxTurns"]);
                if let Some(Value::Object(m)) = fm.get("mcpServers") {
                    a.mcp_servers = m
                        .iter()
                        .map(|(k, v)| (k.clone(), gemini_mcp_back(v)))
                        .collect();
                }
            }
            "qwen_md" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
                a.disallowed_tools = reverse_tools(target, &names(&["disallowedTools"]));
                a.model = str_of(fm, &["model"]).filter(|m| m != "inherit");
                a.permission_mode = str_of(fm, &["permissionMode"]).or_else(|| {
                    str_of(fm, &["approvalMode"]).and_then(|m| {
                        Some(
                            match m.as_str() {
                                "plan" => "plan",
                                "auto-edit" => "acceptEdits",
                                "yolo" => "bypassPermissions",
                                _ => return None,
                            }
                            .to_string(),
                        )
                    })
                });
                a.max_turns = u32_of(fm, &["maxTurns"]);
                a.color = str_of(fm, &["color"]);
                if let Some(Value::Object(m)) = fm.get("mcpServers") {
                    a.mcp_servers = m.clone();
                }
                a.hooks = fm.get("hooks").cloned().unwrap_or(Value::Null);
            }
            "opencode_md" => {
                a.model = str_of(fm, &["model"]).map(|m| model_back(&m));
                a.max_turns = u32_of(fm, &["steps", "maxSteps"]);
                a.color = str_of(fm, &["color"]).map(|c| color_name(&c));
                if let Some(Value::Object(p)) = fm.get("permission") {
                    let denied = |k: &str| p.get(k).and_then(|v| v.as_str()) == Some("deny");
                    if OPENCODE_KEYS.iter().any(|(k, _)| denied(k)) {
                        let mut tools = Vec::new();
                        for (k, covers) in OPENCODE_KEYS {
                            if !denied(k) {
                                for t in *covers {
                                    push_unique(&mut tools, t.to_string());
                                }
                            }
                        }
                        a.tools = tools;
                    }
                }
            }
            "cursor_md" => {
                a.model = str_of(fm, &["model"]).filter(|m| m != "inherit");
                if fm.get("readonly").and_then(|v| v.as_bool()) == Some(true) {
                    a.tools = vec!["Read".into(), "Grep".into(), "Glob".into()];
                }
                a.background = fm.get("is_background").and_then(|v| v.as_bool());
            }
            "copilot_agent_md" => {
                let (t, _) = claude_tools(&names(&["tools"]), "copilot");
                a.tools = t;
                if let Some(Value::Object(m)) = fm.get("mcp-servers") {
                    a.mcp_servers = m.clone();
                }
            }
            "devin_md" => {
                a.tools = reverse_tools(target, &names(&["allowed-tools", "tools"]));
                a.model = str_of(fm, &["model"]);
            }
            "droid_md" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
                a.model = str_of(fm, &["model"]).filter(|m| m != "inherit");
                a.effort = str_of(fm, &["reasoningEffort"]);
            }
            "cline_md" => {
                a.skills = names(&["skills"]);
                a.model = str_of(fm, &["modelId"]);
                a.max_turns = u32_of(fm, &["maxIterations"]);
            }
            "augment_md" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
                a.disallowed_tools = reverse_tools(target, &names(&["disabled_tools"]));
                a.model = str_of(fm, &["model"]);
                a.color = str_of(fm, &["color"]);
            }
            "antigravity" => {
                a.model = str_of(fm, &["model"]);
            }
            "openhands_md" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
                a.model = str_of(fm, &["model"]).filter(|m| m != "inherit");
                a.color = str_of(fm, &["color"]);
                a.skills = names(&["skills"]);
                a.max_turns = u32_of(fm, &["max_iteration_per_run"]);
                if let Some(Value::Object(m)) = fm.get("mcp_servers").or(fm.get("mcp_config")) {
                    a.mcp_servers = m.clone();
                }
                a.hooks = fm.get("hooks").cloned().unwrap_or(Value::Null);
            }
            "rovo" => {
                a.tools = reverse_tools(target, &names(&["tools"]));
            }
            _ => {}
        }
        a
    }
}

impl Converter for MdAgentConverter {
    fn id(&self) -> &'static str {
        match self.format {
            "gemini_md" => "agent_gemini_md",
            "qwen_md" => "agent_qwen_md",
            "opencode_md" => "agent_opencode_md",
            "cursor_md" => "agent_cursor_md",
            "copilot_agent_md" => "agent_copilot_agent_md",
            "devin_md" => "agent_devin_md",
            "droid_md" => "agent_droid_md",
            "cline_md" => "agent_cline_md",
            "augment_md" => "agent_augment_md",
            "antigravity" => "agent_antigravity",
            "openhands_md" => "agent_openhands_md",
            "rovo" => "agent_rovo",
            _ => "agent_md",
        }
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Agent && target.format(kind) == Some(self.format)
    }

    fn native_for(&self, c: &Component, _target: &TargetAdapter) -> bool {
        self.format == "copilot_agent_md" && c.origin_tool == "copilot"
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let Some(a) = agent_of(c) else {
            return Ok(vec![]);
        };
        let name = ctx.name_for(c);
        let dir = dir_for(target, "agents", scope, ctx)?;
        // a Copilot chatmode going back to Copilot is written as it came
        let raw = (self.format == "copilot_agent_md"
            && c.origin_tool == "copilot"
            && ctx.name_override.is_none())
        .then(|| c.entry_text().map(str::to_string))
        .flatten();
        let (text, losses, notes) = match raw {
            Some(t) => (t, vec![], vec![]),
            None => self.render(c, a, &name, target),
        };
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(self.file_name(&name)),
            text,
            "agent",
        );
        pf.primary = true;
        pf.losses = losses;
        pf.notes = notes;
        Ok(vec![pf])
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if kind != ComponentKind::Agent {
            return Ok(vec![]);
        }
        let Some(dir) = kind_path(target, "agents", scope, ctx) else {
            return Ok(vec![]);
        };
        let mut paths = files_with(&dir, &[".md"]);
        if self.format == "devin_md" {
            // `<name>/AGENT.md` profiles
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut extra: Vec<_> = rd
                    .flatten()
                    .map(|e| e.path().join("AGENT.md"))
                    .filter(|p| p.is_file())
                    .collect();
                extra.sort();
                paths.extend(extra);
            }
        }
        let mut out = Vec::new();
        for p in paths {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let (fm, body) = parse::frontmatter(&text);
            let fallback = if p.file_name().and_then(|n| n.to_str()) == Some("AGENT.md") {
                p.parent()
                    .and_then(|d| d.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("agent")
                    .to_string()
            } else {
                stem_of(&p, &[".agent.md", ".chatmode.md", ".md"])
            };
            let spec = self.read_back(&fm, body, target, &fallback);
            if let Some(c) = agent_component(&spec, target, &p) {
                out.push(c);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::targets;

    pub(crate) fn claude_agent() -> Component {
        let text = "---\nname: reviewer\ndescription: Reviews diffs\ntools: Read, Grep, Glob\nmodel: sonnet\npermissionMode: plan\nmaxTurns: 12\ncolor: blue\n---\n\nYou review code.\n";
        let files: parse::RawFiles = [("reviewer.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        parse::parse_raw(ComponentKind::Agent, "reviewer.md", &files).unwrap()
    }

    fn round_trip(format: &'static str, target_id: &str) -> (String, Vec<String>, AgentSpec) {
        let t = targets::target(target_id).unwrap();
        let c = claude_agent();
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let conv = MdAgentConverter { format };
        let (text, losses, _) = conv.render(&c, a, "reviewer", t);
        let (fm, body) = parse::frontmatter(&text);
        let back = conv.read_back(&fm, body, t, "reviewer");
        (text, losses, back)
    }

    #[test]
    fn gemini_renames_tools_and_keeps_turns() {
        let (text, losses, back) = round_trip("gemini_md", "gemini");
        assert!(text.contains("kind: local"), "{text}");
        assert!(text.contains("read_file"), "{text}");
        assert!(text.contains("max_turns: 12"), "{text}");
        assert_eq!(back.tools, vec!["Read", "Grep", "Glob"]);
        assert_eq!(back.max_turns, Some(12));
        assert!(back.prompt.contains("You review code."));
        assert!(
            losses.iter().any(|l| l.contains("permissionMode")),
            "{losses:?}"
        );
        assert!(
            losses.iter().any(|l| l.contains("model `sonnet`")),
            "{losses:?}"
        );
    }

    #[test]
    fn opencode_turns_tools_into_permission_denies() {
        let (text, losses, back) = round_trip("opencode_md", "opencode");
        assert!(text.contains("mode: subagent"), "{text}");
        assert!(text.contains("steps: 12"), "{text}");
        let (fm, _) = parse::frontmatter(&text);
        assert_eq!(fm["permission"]["edit"], "deny");
        assert_eq!(fm["permission"]["bash"], "deny");
        assert!(fm["permission"].get("read").is_none());
        assert!(
            !text.contains("name:"),
            "OpenCode takes the name from the file"
        );
        assert_eq!(back.max_turns, Some(12));
        assert!(back.tools.contains(&"Read".to_string()));
        assert!(!back.tools.contains(&"Bash".to_string()));
        assert_eq!(back.color.as_deref(), Some("blue"));
        assert!(!losses.iter().any(|l| l.contains("`tools`")), "{losses:?}");
    }

    #[test]
    fn qwen_maps_permission_mode() {
        let (text, _, back) = round_trip("qwen_md", "qwen");
        assert!(text.contains("approvalMode: plan"), "{text}");
        assert!(text.contains("grep_search"), "{text}");
        assert_eq!(back.permission_mode.as_deref(), Some("plan"));
        assert_eq!(back.tools, vec!["Read", "Grep", "Glob"]);
    }

    #[test]
    fn cursor_readonly_and_copilot_aliases() {
        let (text, losses, back) = round_trip("cursor_md", "cursor");
        assert!(text.contains("readonly: true"), "{text}");
        assert!(!losses.iter().any(|l| l.contains("`tools`")));
        assert!(back.tools.contains(&"Read".to_string()));
        let (text, _, back) = round_trip("copilot_agent_md", "copilot");
        let (fm, _) = parse::frontmatter(&text);
        assert_eq!(fm["tools"], json!(["read", "search"]));
        assert!(back.tools.contains(&"Grep".to_string()));
    }

    #[test]
    fn devin_droid_rovo_use_their_tool_names() {
        let (text, _, back) = round_trip("devin_md", "devin");
        assert!(text.contains("allowed-tools:"), "{text}");
        assert!(text.contains("model: sonnet"), "{text}");
        assert_eq!(back.tools, vec!["Read", "Grep", "Glob"]);
        let (text, _, back) = round_trip("droid_md", "droid");
        assert!(text.contains("model: inherit"), "{text}");
        assert_eq!(back.tools, vec!["Read", "Grep", "Glob"]);
        let (text, _, _) = round_trip("rovo", "rovo");
        assert!(text.contains("open_files"), "{text}");
    }

    #[test]
    fn copilot_chatmode_goes_native_to_copilot_and_mapped_elsewhere() {
        let text = "---\ndescription: 'Plan things'\ntools: ['codebase', 'fetch', 'editFiles']\nmodel: GPT-4.1\n---\nPlan.\n";
        let files: parse::RawFiles =
            [("planner.chatmode.md".to_string(), text.as_bytes().to_vec())]
                .into_iter()
                .collect();
        let c = parse::parse_raw(ComponentKind::Agent, "planner.chatmode.md", &files).unwrap();
        assert_eq!(c.origin_tool, "copilot");
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let g = targets::target("gemini").unwrap();
        let (out, losses, _) = MdAgentConverter {
            format: "gemini_md",
        }
        .render(&c, a, "planner", g);
        assert!(
            out.contains("read_file") && out.contains("web_fetch") && out.contains("replace"),
            "{out}"
        );
        assert!(losses.iter().any(|l| l.contains("GPT-4.1")), "{losses:?}");
    }
}
