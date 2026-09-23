//! Agents as TOML: Codex `.codex/agents/<name>.toml` (body → `developer_instructions`,
//! `permissionMode` → `sandbox_mode`, inline `[mcp_servers.x]`) and Mistral Vibe
//! `.vibe/agents/<name>.toml` (`agent_type = "subagent"`, `instructions`,
//! `safety`, tool lists). Estudo 06 §0C.2 and the Codex/Vibe (b) sections.

use serde_json::{json, Map, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::edit::toml as etoml;
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

/// TOML agent converter (`codex_toml` or `vibe_toml`).
pub struct TomlAgentConverter {
    pub format: &'static str,
}

impl TomlAgentConverter {
    /// Codex agent file text and losses.
    pub fn render_codex(
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>) {
        let (tools, mut losses) = claude_tools(&a.tools, &c.origin_tool);
        let (model, mloss) = model_for(a.model.as_deref(), &c.origin_tool, target);
        losses.extend(mloss);
        let mut keep: Vec<&str> = Vec::new();
        let mut out = String::new();
        out.push_str(&toml_line("name", &json!(name)));
        out.push_str(&toml_line("description", &json!(a.description.trim())));
        if let Some(m) = &model {
            out.push_str(&toml_line("model", &json!(m)));
        }
        if let Some(e) = a.effort.as_deref() {
            let (v, lossy) = match e {
                "low" | "medium" | "high" | "xhigh" => (e, false),
                "max" => ("xhigh", true),
                _ => ("medium", true),
            };
            out.push_str(&toml_line("model_reasoning_effort", &json!(v)));
            keep.push("effort");
            if lossy {
                losses.push(format!("effort `{e}` (mapped to {v})"));
            }
        }
        // sandbox: plan / a read-only tool list → read-only; acceptEdits → workspace-write
        let sandbox = match a.permission_mode.as_deref() {
            Some("plan") => Some("read-only"),
            Some("acceptEdits") | Some("auto") => Some("workspace-write"),
            _ if is_read_only(&tools) => Some("read-only"),
            _ => None,
        };
        if let Some(s) = sandbox {
            out.push_str(&toml_line("sandbox_mode", &json!(s)));
        }
        if matches!(
            a.permission_mode.as_deref(),
            Some("plan" | "acceptEdits" | "auto")
        ) {
            keep.push("permissionMode");
        }
        if !tools.is_empty() {
            keep.push("tools");
            if !is_read_only(&tools) {
                losses.push(
                    "per-agent tool list (Codex agents only restrict through sandbox_mode)".into(),
                );
            }
        }
        let body = clean_body(&a.prompt);
        out.push_str(&toml_line(
            "developer_instructions",
            &json!(ensure_nl(&body)),
        ));
        if !a.mcp_servers.is_empty() {
            keep.push("mcpServers");
            for (srv, v) in &a.mcp_servers {
                let (lines, loss) = codex_mcp_lines(v);
                if let Some(l) = loss {
                    losses.push(format!("MCP server `{srv}`: {l}"));
                    continue;
                }
                out.push_str(&format!("\n[mcp_servers.{}]\n", etoml::key_text(srv)));
                out.push_str(&lines);
            }
        }
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        (out, losses)
    }

    /// Vibe agent file text and losses.
    pub fn render_vibe(
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>) {
        let (tools, mut losses) = claude_tools(&a.tools, &c.origin_tool);
        let (dis, l2) = claude_tools(&a.disallowed_tools, &c.origin_tool);
        losses.extend(l2);
        let (model, mloss) = model_for(a.model.as_deref(), &c.origin_tool, target);
        losses.extend(mloss);
        let mut keep: Vec<&str> = Vec::new();
        let mut out = String::new();
        out.push_str(&toml_line("display_name", &json!(name)));
        out.push_str(&toml_line("description", &json!(a.description.trim())));
        out.push_str(&toml_line("agent_type", &json!("subagent")));
        let safety = match a.permission_mode.as_deref() {
            Some("plan") => Some("safe"),
            Some("acceptEdits") | Some("auto") => Some("neutral"),
            Some("bypassPermissions") | Some("dontAsk") => Some("yolo"),
            _ => None,
        };
        if let Some(s) = safety {
            out.push_str(&toml_line("safety", &json!(s)));
            keep.push("permissionMode");
        }
        if let Some(m) = &model {
            out.push_str(&toml_line("active_model", &json!(m)));
        }
        if !tools.is_empty() {
            let (m, l) = map_tools(&tools, target);
            losses.extend(l);
            out.push_str(&toml_line("enabled_tools", &json!(m)));
            keep.push("tools");
        }
        if !dis.is_empty() {
            let (m, l) = map_tools(&dis, target);
            losses.extend(l);
            out.push_str(&toml_line("disabled_tools", &json!(m)));
            keep.push("disallowedTools");
        }
        out.push_str(&toml_line(
            "instructions",
            &json!(ensure_nl(&clean_body(&a.prompt))),
        ));
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        (out, losses)
    }

    /// Codex agent TOML → canonical spec.
    pub fn read_codex(v: &Value, fallback: &str) -> AgentSpec {
        let m = v.as_object().cloned().unwrap_or_default();
        let mut a = AgentSpec {
            name: str_of(&m, &["name"]).unwrap_or_else(|| fallback.to_string()),
            description: str_of(&m, &["description"]).unwrap_or_default(),
            prompt: m
                .get("developer_instructions")
                .and_then(|x| x.as_str())
                .map(|s| format!("\n{s}"))
                .unwrap_or_default(),
            model: str_of(&m, &["model"]),
            effort: str_of(&m, &["model_reasoning_effort"]),
            ..Default::default()
        };
        a.permission_mode = match str_of(&m, &["sandbox_mode"]).as_deref() {
            Some("read-only") => Some("plan".into()),
            Some("workspace-write") => Some("acceptEdits".into()),
            _ => None,
        };
        if let Some(Value::Object(srv)) = m.get("mcp_servers") {
            a.mcp_servers = srv
                .iter()
                .map(|(k, x)| (k.clone(), codex_mcp_back(x)))
                .collect::<Map<String, Value>>();
        }
        a
    }

    /// Vibe agent TOML → canonical spec.
    pub fn read_vibe(v: &Value, target: &TargetAdapter, fallback: &str) -> AgentSpec {
        let m = v.as_object().cloned().unwrap_or_default();
        let list = |k: &str| -> Vec<String> {
            m.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        AgentSpec {
            name: str_of(&m, &["display_name", "name"]).unwrap_or_else(|| fallback.to_string()),
            description: str_of(&m, &["description"]).unwrap_or_default(),
            prompt: m
                .get("instructions")
                .and_then(|x| x.as_str())
                .map(|s| format!("\n{s}"))
                .unwrap_or_default(),
            model: str_of(&m, &["active_model"]),
            permission_mode: match str_of(&m, &["safety"]).as_deref() {
                Some("safe") => Some("plan".into()),
                Some("neutral") => Some("acceptEdits".into()),
                Some("yolo") => Some("bypassPermissions".into()),
                _ => None,
            },
            tools: reverse_tools(target, &list("enabled_tools")),
            disallowed_tools: reverse_tools(target, &list("disabled_tools")),
            ..Default::default()
        }
    }
}

fn ensure_nl(s: &str) -> String {
    if s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

impl Converter for TomlAgentConverter {
    fn id(&self) -> &'static str {
        if self.format == "codex_toml" {
            "agent_codex_toml"
        } else {
            "agent_vibe_toml"
        }
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Agent && target.format(kind) == Some(self.format)
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
        let (text, losses) = if self.format == "codex_toml" {
            Self::render_codex(c, a, &name, target)
        } else {
            Self::render_vibe(c, a, &name, target)
        };
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(format!("{name}.toml")),
            text,
            "agent",
        );
        pf.primary = true;
        pf.losses = losses;
        if self.format == "codex_toml" {
            pf.notes.push(
                "Codex asks to review a new project agent file before it loads it (trust review)"
                    .into(),
            );
        }
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
        for p in files_with(&dir, &[".toml"]) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = etoml::parse(&text) else {
                continue;
            };
            let stem = stem_of(&p, &[".toml"]);
            let spec = if self.format == "codex_toml" {
                Self::read_codex(&v, &stem)
            } else {
                Self::read_vibe(&v, target, &stem)
            };
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
    use crate::core::agentkit::parse;
    use crate::core::agentkit::targets;

    fn agent(text: &str) -> Component {
        let files: parse::RawFiles = [("a.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        parse::parse_raw(ComponentKind::Agent, "a.md", &files).unwrap()
    }

    #[test]
    fn codex_round_trip() {
        let c = agent("---\nname: reviewer\ndescription: Reviews \"diffs\"\ntools: Read, Grep\nmodel: opus\npermissionMode: plan\nmcpServers:\n  docs:\n    type: http\n    url: https://x.dev/mcp\n  fs:\n    command: npx\n    args: [\"-y\", \"fs\"]\n---\n\nReview like an owner.\nUse \\ and \"\"\" carefully.\n");
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let t = targets::target("codex").unwrap();
        let (text, losses) = TomlAgentConverter::render_codex(&c, a, "reviewer", t);
        let v = etoml::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(v["name"], "reviewer");
        assert_eq!(v["sandbox_mode"], "read-only");
        assert_eq!(v["mcp_servers"]["docs"]["url"], "https://x.dev/mcp");
        assert_eq!(v["mcp_servers"]["fs"]["args"][1], "fs");
        assert!(v["developer_instructions"]
            .as_str()
            .unwrap()
            .contains("Use \\ and \"\"\" carefully."));
        assert!(losses.iter().any(|l| l.contains("model `opus`")));
        let back = TomlAgentConverter::read_codex(&v, "x");
        assert_eq!(back.permission_mode.as_deref(), Some("plan"));
        assert_eq!(back.description, "Reviews \"diffs\"");
        assert!(back.prompt.contains("Review like an owner."));
        assert_eq!(back.mcp_servers["docs"]["type"], "http");
    }

    #[test]
    fn vibe_round_trip() {
        let c = agent("---\nname: redteam\ndescription: Adversarial\ntools: Read, Bash\npermissionMode: acceptEdits\n---\nAttack it.\n");
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let t = targets::target("vibe").unwrap();
        let (text, _) = TomlAgentConverter::render_vibe(&c, a, "redteam", t);
        let v = etoml::parse(&text).unwrap();
        assert_eq!(v["agent_type"], "subagent");
        assert_eq!(v["safety"], "neutral");
        assert_eq!(v["enabled_tools"], json!(["read_file", "bash"]));
        let back = TomlAgentConverter::read_vibe(&v, t, "x");
        assert_eq!(back.tools, vec!["Read", "Bash"]);
        assert_eq!(back.permission_mode.as_deref(), Some("acceptEdits"));
    }
}
