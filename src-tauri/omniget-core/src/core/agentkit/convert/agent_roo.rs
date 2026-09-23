//! Roo Code custom modes (legacy, frozen since 2026-05): one entry per agent in
//! `.roomodes` (project) or `custom_modes.yaml` (global):
//! `customModes: [{slug, name, roleDefinition, whenToUse, description, groups}]`.

use serde_json::{json, Map, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct RooModesConverter;

fn slug(n: &str) -> String {
    let s: String = n
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.trim_matches('-').to_string()
}

fn title(n: &str) -> String {
    n.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `.roomodes` may be YAML or JSON; an existing JSON file stays JSON.
fn doc_format(path: &std::path::Path) -> DocFormat {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim_start().starts_with('{') => DocFormat::Json,
        _ => DocFormat::Yaml,
    }
}

impl RooModesConverter {
    /// The mode record and losses.
    pub fn mode(
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (Value, Vec<String>) {
        let (tools, mut losses) = claude_tools(&a.tools, &c.origin_tool);
        let mut keep: Vec<&str> = vec![];
        let mut groups: Vec<String> = Vec::new();
        if tools.is_empty() {
            groups = ["read", "edit", "command", "mcp"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        } else {
            keep.push("tools");
            for t in &tools {
                let (b, _) = tool_base(t);
                let g = if b.starts_with("mcp__") {
                    "mcp"
                } else {
                    match b {
                        "Read" | "Glob" | "Grep" | "LS" | "WebFetch" | "WebSearch" => "read",
                        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "edit",
                        "Bash" | "PowerShell" => "command",
                        _ => continue,
                    }
                };
                push_unique(&mut groups, g);
            }
        }
        if a.permission_mode.as_deref() == Some("plan") {
            groups.retain(|g| g != "edit" && g != "command");
            keep.push("permissionMode");
        }
        let (_, mloss) = model_for(a.model.as_deref(), &c.origin_tool, target);
        if a.model.is_some() {
            losses.push(
                mloss.unwrap_or_else(|| "agent model (Roo modes use the API profile)".into()),
            );
        }
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        let mut m = Map::new();
        m.insert("slug".into(), json!(slug(name)));
        m.insert("name".into(), json!(title(name)));
        m.insert(
            "description".into(),
            json!(parse::short_description(&a.description)),
        );
        m.insert(
            "roleDefinition".into(),
            json!(clean_body(&a.prompt).trim_end()),
        );
        m.insert("whenToUse".into(), json!(a.description));
        m.insert("groups".into(), json!(groups));
        (Value::Object(m), losses)
    }

    /// A mode record → canonical spec.
    pub fn read(m: &Value) -> Option<AgentSpec> {
        let o = m.as_object()?;
        let slug = str_of(o, &["slug"])?;
        let mut prompt = str_of(o, &["roleDefinition"]).unwrap_or_default();
        if let Some(ci) = str_of(o, &["customInstructions"]) {
            prompt.push_str("\n\n");
            prompt.push_str(&ci);
        }
        let mut tools = Vec::new();
        for g in o
            .get("groups")
            .and_then(|g| g.as_array())
            .into_iter()
            .flatten()
        {
            // a group is a name or `[name, {fileRegex}]`
            let name = g
                .as_str()
                .or_else(|| {
                    g.as_array()
                        .and_then(|x| x.first())
                        .and_then(|x| x.as_str())
                })
                .unwrap_or("");
            let add: &[&str] = match name {
                "read" => &["Read", "Glob", "Grep"],
                "edit" => &["Edit", "Write"],
                "command" => &["Bash"],
                _ => &[],
            };
            for t in add {
                push_unique(&mut tools, t.to_string());
            }
        }
        Some(AgentSpec {
            name: slug,
            description: str_of(o, &["whenToUse", "description"]).unwrap_or_default(),
            prompt: format!("\n{prompt}\n"),
            tools,
            ..Default::default()
        })
    }
}

impl Converter for RooModesConverter {
    fn id(&self) -> &'static str {
        "agent_roo_modes"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Agent && target.format(kind) == Some("roo_modes")
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
        let file = dir_for(target, "agents", scope, ctx)?;
        let (mode, losses) = Self::mode(c, a, &name, target);
        let mut pf = PlannedFile::merge(
            &target.id,
            c,
            file.clone(),
            doc_format(&file),
            vec![PatchOp::Append {
                path: keys(["customModes"]),
                value: mode,
                dedupe: Dedupe::Fields {
                    fields: vec!["slug".into()],
                },
            }],
            "custom mode",
        );
        pf.losses = losses;
        pf.notes
            .push("Roo Code is discontinued; the mode is written for existing installs".into());
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
        let Some(file) = kind_path(target, "agents", scope, ctx) else {
            return Ok(vec![]);
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return Ok(vec![]);
        };
        let Ok(v) = crate::core::agentkit::edit::parse_value(doc_format(&file), &text) else {
            return Ok(vec![]);
        };
        Ok(v.get("customModes")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
            .filter_map(Self::read)
            .filter_map(|s| agent_component(&s, target, &file))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::targets;
    use crate::core::agentkit::writer;

    #[test]
    fn roomodes_append_and_read_back() {
        let text = "---\nname: docs-writer\ndescription: Writes docs\ntools: Read, Edit\n---\nYou write docs.\n";
        let files: parse::RawFiles = [("docs-writer.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        let c = parse::parse_raw(ComponentKind::Agent, "docs-writer.md", &files).unwrap();
        let ComponentBody::Agent(a) = &c.body else {
            unreachable!()
        };
        let t = targets::target("roo").unwrap();
        let (mode, _) = RooModesConverter::mode(&c, a, "docs-writer", t);
        assert_eq!(mode["groups"], json!(["read", "edit"]));
        let existing = "customModes:\n  - slug: mine\n    name: Mine\n    roleDefinition: x\n    groups: [read]\n";
        let op = PatchOp::Append {
            path: keys(["customModes"]),
            value: mode,
            dedupe: Dedupe::Fields {
                fields: vec!["slug".into()],
            },
        };
        let applied = writer::apply_ops(DocFormat::Yaml, existing, &[op]).unwrap();
        let v = crate::core::agentkit::edit::yaml::parse(&applied.text).unwrap();
        let modes = v["customModes"].as_array().unwrap();
        assert_eq!(modes.len(), 2);
        let back = RooModesConverter::read(&modes[1]).unwrap();
        assert_eq!(back.name, "docs-writer");
        assert_eq!(back.tools, vec!["Read", "Glob", "Grep", "Edit", "Write"]);
        let (undone, _, _) =
            writer::undo_ops(DocFormat::Yaml, &applied.text, &applied.undo, false).unwrap();
        assert_eq!(undone, existing);
    }
}
