//! Goose recipes (YAML) for commands and, when a manifest asks for it, agents
//! (estudo 06 Goose (b)/(c), §0C.2/§0C.3): `{version, title, description,
//! instructions | prompt, parameters, extensions, settings}`. A command's body is
//! the recipe `prompt` with `{{ args }}` declared as a parameter; an agent's body
//! is the recipe `instructions`.

use serde_json::{json, Map, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct GooseRecipeConverter;

/// Recipe folder: `paths.recipes`, else `paths.commands`.
fn recipes_dir(
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Option<std::path::PathBuf> {
    kind_path(target, "recipes", scope, ctx).or_else(|| kind_path(target, "commands", scope, ctx))
}

/// Claude MCP record → Goose recipe extension.
fn extension(name: &str, v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    let mut o = Map::new();
    if let Some(u) = m.get("url") {
        if m.get("type").and_then(|t| t.as_str()) == Some("sse") {
            return None;
        }
        o.insert("type".into(), json!("streamable_http"));
        o.insert("name".into(), json!(name));
        o.insert("uri".into(), u.clone());
        if let Some(h) = m.get("headers") {
            o.insert("headers".into(), h.clone());
        }
    } else {
        o.insert("type".into(), json!("stdio"));
        o.insert("name".into(), json!(name));
        o.insert("cmd".into(), m.get("command")?.clone());
        o.insert("args".into(), m.get("args").cloned().unwrap_or(json!([])));
        if let Some(Value::Object(env)) = m.get("env") {
            o.insert("env_keys".into(), json!(env.keys().collect::<Vec<_>>()));
        }
    }
    o.insert("timeout".into(), json!(300));
    Some(Value::Object(o))
}

impl GooseRecipeConverter {
    /// Recipe for a command.
    pub fn command_recipe(
        c: &Component,
        cmd: &CommandSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>) {
        let (body, mut losses) = render_body(&cmd.body, &target.placeholder_map);
        let desc = if cmd.description.trim().is_empty() {
            format!("/{name}")
        } else {
            cmd.description.trim().to_string()
        };
        let mut e: Vec<(&str, Value)> = vec![
            ("version", json!("1.0.0")),
            ("title", json!(name)),
            ("description", json!(desc)),
            ("prompt", json!(clean_body(&body))),
        ];
        if cmd.body.contains("{{args}}") {
            e.push((
                "parameters",
                json!([{
                    "key": "args",
                    "input_type": "string",
                    "requirement": "required",
                    "description": cmd.argument_hint.clone().unwrap_or_else(|| "Arguments".into()),
                }]),
            ));
        }
        if let Some(m) = &cmd.model {
            let (mm, l) = model_for(Some(m), &c.origin_tool, target);
            if let Some(mm) = mm {
                e.push(("settings", json!({ "goose_model": mm })));
            }
            losses.extend(l);
        }
        if !cmd.allowed_tools.is_empty() {
            losses.push("allowed-tools (the recipe runs with Goose's normal permissions)".into());
        }
        (yaml_doc(&e), losses)
    }

    /// Recipe for an agent (body → `instructions`).
    pub fn agent_recipe(
        c: &Component,
        a: &AgentSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>) {
        let mut losses = Vec::new();
        let mut keep: Vec<&str> = vec![];
        let mut e: Vec<(&str, Value)> = vec![
            ("version", json!("1.0.0")),
            ("title", json!(name)),
            (
                "description",
                json!(parse::short_description(&a.description)),
            ),
            ("instructions", json!(clean_body(&a.prompt))),
        ];
        if !a.mcp_servers.is_empty() {
            let mut exts = Vec::new();
            for (n, v) in &a.mcp_servers {
                match extension(n, v) {
                    Some(x) => exts.push(x),
                    None => losses.push(format!("MCP server `{n}` (Goose has no SSE)")),
                }
            }
            exts.push(json!({"type": "platform", "name": "summon"}));
            e.push(("extensions", json!(exts)));
            keep.push("mcpServers");
        }
        let (model, ml) = model_for(a.model.as_deref(), &c.origin_tool, target);
        losses.extend(ml);
        let mut settings = Map::new();
        if let Some(m) = model {
            settings.insert("goose_model".into(), json!(m));
        }
        if let Some(n) = a.max_turns {
            settings.insert("max_turns".into(), json!(n));
            keep.push("maxTurns");
        }
        if !settings.is_empty() {
            e.push(("settings", Value::Object(settings)));
        }
        for l in dropped_fields(a, &keep, target) {
            push_unique(&mut losses, l);
        }
        (yaml_doc(&e), losses)
    }

    /// Recipe → canonical command (prompt) or agent (instructions only).
    pub fn read(
        text: &str,
        target: &TargetAdapter,
    ) -> Option<(Option<CommandSpec>, Option<AgentSpec>)> {
        let v = crate::core::agentkit::edit::yaml::parse(text).ok()?;
        let m = v.as_object()?;
        let desc = str_of(m, &["description"]).unwrap_or_default();
        if let Some(p) = m.get("prompt").and_then(|p| p.as_str()) {
            let hint = m
                .get("parameters")
                .and_then(|p| p.as_array())
                .and_then(|a| a.iter().find(|x| x["key"] == "args"))
                .and_then(|x| x.get("description"))
                .and_then(|d| d.as_str())
                .filter(|d| *d != "Arguments")
                .map(str::to_string);
            let body = canonical_body(p, &target.placeholder_map);
            return Some((
                Some(CommandSpec {
                    description: desc,
                    argument_hint: hint,
                    model: m
                        .get("settings")
                        .and_then(|s| s.get("goose_model"))
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    body: format!("\n{body}"),
                    ..Default::default()
                }),
                None,
            ));
        }
        let ins = m.get("instructions")?.as_str()?;
        Some((
            None,
            Some(AgentSpec {
                name: str_of(m, &["title"]).unwrap_or_default(),
                description: desc,
                prompt: format!("\n{ins}"),
                max_turns: m
                    .get("settings")
                    .and_then(|s| s.as_object())
                    .and_then(|s| u32_of(s, &["max_turns"])),
                ..Default::default()
            }),
        ))
    }
}

impl Converter for GooseRecipeConverter {
    fn id(&self) -> &'static str {
        "goose_recipe"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        matches!(kind, ComponentKind::Command | ComponentKind::Agent)
            && target.format(kind) == Some("goose_recipe")
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let name = ctx.name_for(c);
        let dir =
            recipes_dir(target, scope, ctx).ok_or_else(|| no_path(target, "recipes", scope))?;
        let (text, losses) = match &c.body {
            ComponentBody::Command(cmd) => Self::command_recipe(c, cmd, &name, target),
            ComponentBody::Agent(a) => Self::agent_recipe(c, a, &name, target),
            _ => return Ok(vec![]),
        };
        let file = dir.join(format!("{name}.yaml"));
        let mut pf = PlannedFile::write(&target.id, c, file.clone(), text, "recipe");
        pf.primary = true;
        pf.losses = losses;
        let mut out = vec![pf];
        if c.kind == ComponentKind::Command {
            // a slash command lives in the user config and points at the recipe
            match kind_path(target, "settings", Scope::Global, ctx) {
                Some(cfg) if scope != Scope::Project && scope != Scope::Local => {
                    let mut m = PlannedFile::merge(
                        &target.id,
                        c,
                        cfg,
                        DocFormat::Yaml,
                        vec![PatchOp::Append {
                            path: keys(["slash_commands"]),
                            value: json!({
                                "command": name,
                                "recipe_path": file.display().to_string(),
                            }),
                            dedupe: Dedupe::Fields {
                                fields: vec!["command".into()],
                            },
                        }],
                        "slash command",
                    );
                    m.notes.push(format!(
                        "/{name} runs the recipe (slash commands take at most one argument)"
                    ));
                    out.push(m);
                }
                _ => {
                    out[0].notes.push(format!(
                        "project recipes run with `goose run --recipe {name}`; install globally to get /{name}"
                    ));
                }
            }
        }
        Ok(out)
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if !matches!(kind, ComponentKind::Command | ComponentKind::Agent) {
            return Ok(vec![]);
        }
        let Some(dir) = recipes_dir(target, scope, ctx) else {
            return Ok(vec![]);
        };
        let mut out = Vec::new();
        for p in files_with(&dir, &[".yaml"]) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Some((cmd, agent)) = Self::read(&text, target) else {
                continue;
            };
            let stem = stem_of(&p, &[".yaml"]);
            match (kind, cmd, agent) {
                (ComponentKind::Command, Some(cmd), _) => {
                    out.extend(command_component(&stem, &cmd, target, &p));
                }
                (ComponentKind::Agent, _, Some(mut a)) => {
                    if a.name.is_empty() {
                        a.name = stem;
                    }
                    out.extend(agent_component(&a, target, &p));
                }
                _ => {}
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
    fn goose_command_recipe_round_trip() {
        let text = "---\nargument-hint: <feature-name>\ndescription: Create a feature branch\n---\n\nCreate **$ARGUMENTS**.\n- branch: !`git branch --show-current`\n";
        let files: parse::RawFiles = [("feature.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        let c = parse::parse_raw(ComponentKind::Command, "feature.md", &files).unwrap();
        let ComponentBody::Command(cmd) = &c.body else {
            unreachable!()
        };
        let t = targets::target("goose").unwrap();
        let (y, losses) = GooseRecipeConverter::command_recipe(&c, cmd, "feature", t);
        let v = crate::core::agentkit::edit::yaml::parse(&y).unwrap_or_else(|e| panic!("{e}\n{y}"));
        assert_eq!(v["title"], "feature");
        assert!(
            v["prompt"]
                .as_str()
                .unwrap()
                .contains("Create **{{ args }}**."),
            "{y}"
        );
        assert_eq!(v["parameters"][0]["key"], "args");
        assert_eq!(v["parameters"][0]["description"], "<feature-name>");
        assert!(losses.iter().any(|l| l.contains("shell")), "{losses:?}");
        let (back, _) = GooseRecipeConverter::read(&y, t).unwrap();
        let back = back.unwrap();
        assert!(back.body.contains("Create **{{args}}**."), "{}", back.body);
        assert_eq!(back.argument_hint.as_deref(), Some("<feature-name>"));
    }
}
