//! Kiro custom agents: `.kiro/agents/<name>.json` (estudo 06 §0C.2 and the Kiro
//! (b) sections): `{name, description, prompt, model, tools, allowedTools,
//! mcpServers, resources, hooks}`. Custom agents do not get steering or skills
//! on their own, so `resources` points at AGENTS.md, the steering folder and the
//! agent's skills.

use serde_json::{json, Map, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::edit::json as ejson;
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct KiroAgentConverter;

/// Claude tool → Kiro `tools` tag.
fn kiro_tag(t: &str) -> Option<String> {
    let (base, _) = tool_base(t);
    if let Some((srv, tool)) = split_mcp(base) {
        return Some(if tool == "*" {
            format!("@{srv}")
        } else {
            format!("@{srv}/{tool}")
        });
    }
    Some(
        match base {
            "Read" | "Glob" | "Grep" | "LS" | "NotebookRead" => "read",
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "write",
            "Bash" | "PowerShell" => "shell",
            "WebFetch" | "WebSearch" => "web",
            "Agent" | "Task" => "subagent",
            "TodoWrite" => "todo_list",
            _ => return None,
        }
        .to_string(),
    )
}

/// Kiro tag → Claude tools.
fn tag_back(tag: &str) -> Vec<String> {
    if let Some(rest) = tag.strip_prefix('@') {
        if matches!(rest, "mcp" | "builtin" | "powers") {
            return vec![];
        }
        return vec![match rest.split_once('/') {
            Some((s, t)) => format!("mcp__{s}__{t}"),
            None => format!("mcp__{rest}"),
        }];
    }
    let v: &[&str] = match tag {
        "read" | "fs_read" => &["Read", "Glob", "Grep"],
        "write" | "fs_write" => &["Edit", "Write"],
        "shell" | "execute_bash" => &["Bash"],
        "web" => &["WebFetch", "WebSearch"],
        "subagent" => &["Agent"],
        "todo_list" => &["TodoWrite"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// Claude agent hook events → Kiro agent-embedded (camelCase) triggers.
fn kiro_event(e: &str) -> Option<&'static str> {
    Some(match e {
        "SessionStart" => "agentSpawn",
        "UserPromptSubmit" => "userPromptSubmit",
        "PreToolUse" => "preToolUse",
        "PostToolUse" => "postToolUse",
        "Stop" => "stop",
        _ => return None,
    })
}

impl KiroAgentConverter {
    /// Agent JSON and losses.
    pub fn render(
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
        scope: Scope,
    ) -> (Value, Vec<String>) {
        let (tools, mut losses) = claude_tools(&a.tools, &c.origin_tool);
        let (model, mloss) = model_for(a.model.as_deref(), &c.origin_tool, target);
        losses.extend(mloss);
        let mut keep: Vec<&str> = Vec::new();
        let mut o = Map::new();
        o.insert("name".into(), json!(name));
        o.insert("description".into(), json!(a.description));
        o.insert("prompt".into(), json!(clean_body(&a.prompt)));
        if let Some(m) = model {
            o.insert("model".into(), json!(m));
        }
        let plan = a.permission_mode.as_deref() == Some("plan");
        let mut tags: Vec<String> = Vec::new();
        if !tools.is_empty() {
            keep.push("tools");
            for t in &tools {
                match kiro_tag(t) {
                    Some(x) => {
                        if !(plan && matches!(x.as_str(), "write" | "shell")) {
                            push_unique(&mut tags, x)
                        }
                    }
                    None => push_unique(&mut losses, format!("tool `{t}` (no Kiro tag)")),
                }
            }
            o.insert("tools".into(), json!(tags));
        } else if plan {
            tags = vec!["read".into(), "web".into()];
            o.insert("tools".into(), json!(tags));
        }
        if plan {
            keep.push("permissionMode");
        }
        if matches!(
            a.permission_mode.as_deref(),
            Some("bypassPermissions" | "dontAsk")
        ) {
            o.insert(
                "allowedTools".into(),
                json!(if tags.is_empty() {
                    vec!["*".to_string()]
                } else {
                    tags.clone()
                }),
            );
            keep.push("permissionMode");
        }
        if !a.disallowed_tools.is_empty() {
            let (dis, l) = claude_tools(&a.disallowed_tools, &c.origin_tool);
            losses.extend(l);
            let ex: Vec<String> = dis.iter().filter_map(|t| kiro_tag(t)).collect();
            if !ex.is_empty() {
                o.insert("excludedTools".into(), json!(ex));
                keep.push("disallowedTools");
            }
        }
        if !a.mcp_servers.is_empty() {
            let m: Map<String, Value> = a
                .mcp_servers
                .iter()
                .map(|(k, v)| (k.clone(), plain_mcp(v)))
                .collect();
            o.insert("mcpServers".into(), Value::Object(m));
            keep.push("mcpServers");
        }
        let mut resources: Vec<String> = Vec::new();
        if scope == Scope::Global {
            resources.push("file://~/.kiro/steering/**/*.md".into());
        } else {
            resources.push("file://AGENTS.md".into());
            resources.push("file://.kiro/steering/**/*.md".into());
        }
        for s in &a.skills {
            let base = if scope == Scope::Global {
                "~/.kiro/skills"
            } else {
                ".kiro/skills"
            };
            resources.push(format!(
                "skill://{base}/{}/SKILL.md",
                parse::sanitize_name(s)
            ));
        }
        if !a.skills.is_empty() {
            keep.push("skills");
        }
        o.insert("resources".into(), json!(resources));
        if let Some(h) = a.hooks.as_object() {
            let mut hooks = Map::new();
            for e in parse::hook_entries(&a.hooks) {
                let Some(ev) = kiro_event(&e.event) else {
                    push_unique(&mut losses, format!("agent hook event {}", e.event));
                    continue;
                };
                let Some(cmd) = &e.handler.command else {
                    push_unique(&mut losses, "non-command agent hooks");
                    continue;
                };
                let mut rec = Map::new();
                rec.insert("command".into(), json!(cmd));
                if let Some(m) = &e.matcher {
                    rec.insert("matcher".into(), json!(target.map_matcher(m)));
                }
                if let Some(t) = e.handler.timeout {
                    rec.insert("timeout_ms".into(), json!((t * 1000.0) as i64));
                }
                hooks
                    .entry(ev.to_string())
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .map(|arr| arr.push(Value::Object(rec)));
            }
            let _ = h;
            if !hooks.is_empty() {
                o.insert("hooks".into(), Value::Object(hooks));
            }
            keep.push("hooks");
        }
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        (Value::Object(o), losses)
    }

    /// Agent JSON → canonical spec. `dir` resolves `file://` prompts.
    pub fn read(v: &Value, dir: &std::path::Path, fallback: &str) -> AgentSpec {
        let m = v.as_object().cloned().unwrap_or_default();
        let prompt = match m.get("prompt").and_then(|x| x.as_str()) {
            Some(p) if p.starts_with("file://") => {
                let rel = p.trim_start_matches("file://");
                let path = if let Some(r) = rel.strip_prefix("./") {
                    dir.join(r)
                } else {
                    std::path::PathBuf::from(rel)
                };
                std::fs::read_to_string(path).unwrap_or_default()
            }
            Some(p) => p.to_string(),
            None => String::new(),
        };
        let mut tools = Vec::new();
        for t in m
            .get("tools")
            .and_then(|x| x.as_array())
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
        {
            for c in tag_back(t) {
                push_unique(&mut tools, c);
            }
        }
        let mut hooks = Map::new();
        if let Some(Value::Object(h)) = m.get("hooks") {
            for (ev, list) in h {
                let claude = match ev.as_str() {
                    "agentSpawn" => "SessionStart",
                    "userPromptSubmit" => "UserPromptSubmit",
                    "preToolUse" => "PreToolUse",
                    "postToolUse" => "PostToolUse",
                    "stop" => "Stop",
                    _ => continue,
                };
                let groups: Vec<Value> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        let mut g = Map::new();
                        if let Some(mt) = r.get("matcher") {
                            g.insert("matcher".into(), mt.clone());
                        }
                        let mut hh = Map::new();
                        hh.insert("type".into(), json!("command"));
                        if let Some(cmd) = r.get("command") {
                            hh.insert("command".into(), cmd.clone());
                        }
                        if let Some(ms) = r.get("timeout_ms").and_then(|x| x.as_f64()) {
                            hh.insert("timeout".into(), json!((ms / 1000.0).round() as i64));
                        }
                        g.insert("hooks".into(), json!([Value::Object(hh)]));
                        Value::Object(g)
                    })
                    .collect();
                hooks.insert(claude.into(), json!(groups));
            }
        }
        AgentSpec {
            name: str_of(&m, &["name"]).unwrap_or_else(|| fallback.to_string()),
            description: str_of(&m, &["description"]).unwrap_or_default(),
            prompt: format!("\n{prompt}"),
            model: str_of(&m, &["model"]),
            tools,
            mcp_servers: m
                .get("mcpServers")
                .and_then(|x| x.as_object())
                .cloned()
                .unwrap_or_default(),
            hooks: if hooks.is_empty() {
                Value::Null
            } else {
                Value::Object(hooks)
            },
            ..Default::default()
        }
    }
}

impl Converter for KiroAgentConverter {
    fn id(&self) -> &'static str {
        "agent_kiro_json"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Agent && target.format(kind) == Some("kiro_json")
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
        let (v, losses) = Self::render(c, a, &name, target, scope);
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(format!("{name}.json")),
            ejson::to_text(&v),
            "agent",
        );
        pf.primary = true;
        pf.losses = losses;
        pf.notes
            .push(format!("pick it in Kiro with /agent swap {name}"));
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
        let mut out = Vec::new();
        for p in files_with(&dir, &[".json"]) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = crate::core::agentkit::edit::jsonc::parse_value(&text) else {
                continue;
            };
            let spec = Self::read(&v, &dir, &stem_of(&p, &[".json"]));
            if let Some(c) = agent_component(&spec, target, &p) {
                out.push(c);
            }
        }
        // CLI 3.0 / IDE also accept Markdown agents (frontmatter = config)
        for p in files_with(&dir, &[".md"]) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let (fm, body) = parse::frontmatter(&text);
            let mut v = Value::Object(fm);
            v["prompt"] = json!(body);
            let spec = Self::read(&v, &dir, &stem_of(&p, &[".md"]));
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

    #[test]
    fn kiro_round_trip() {
        let text = "---\nname: backend\ndescription: Backend dev\ntools: Read, Edit, Bash, mcp__git__status\nskills: [api-design]\nhooks:\n  PostToolUse:\n    - matcher: Edit\n      hooks:\n        - type: command\n          command: cargo fmt\n          timeout: 5\n---\nYou build APIs.\n";
        let files: parse::RawFiles = [("backend.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        let c = parse::parse_raw(ComponentKind::Agent, "backend.md", &files).unwrap();
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let t = targets::target("kiro").unwrap();
        let (v, losses) = KiroAgentConverter::render(&c, a, "backend", t, Scope::Project);
        assert_eq!(v["tools"], json!(["read", "write", "shell", "@git/status"]));
        assert_eq!(v["prompt"], "You build APIs.\n");
        assert!(v["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "skill://.kiro/skills/api-design/SKILL.md"));
        assert_eq!(v["hooks"]["postToolUse"][0]["matcher"], "write");
        assert_eq!(v["hooks"]["postToolUse"][0]["timeout_ms"], 5000);
        assert!(losses.is_empty(), "{losses:?}");
        let back = KiroAgentConverter::read(&v, std::path::Path::new("/"), "x");
        assert!(back.tools.contains(&"Bash".to_string()));
        assert!(back.tools.contains(&"mcp__git__status".to_string()));
        assert_eq!(back.hooks["PostToolUse"][0]["hooks"][0]["timeout"], 5);
    }
}
