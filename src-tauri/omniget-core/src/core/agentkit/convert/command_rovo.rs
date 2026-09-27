//! Rovo Dev saved prompts: an entry in `.rovodev/prompts.yml`
//! (`prompts: [{name, description, content_file}]`) plus the Markdown content
//! file next to it (estudo 06 Rovo (c)). Run with `/prompts <name> [extra]`.

use serde_json::json;

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct RovoPromptsConverter;

impl Converter for RovoPromptsConverter {
    fn id(&self) -> &'static str {
        "command_rovo_prompts"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Command && target.format(kind) == Some("rovo_prompts")
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let Some(cmd) = command_of(c) else {
            return Ok(vec![]);
        };
        let name = ctx.name_for(c);
        let yml = dir_for(target, "commands", scope, ctx)?;
        let content_dir = kind_path(target, "prompts", scope, ctx)
            .or_else(|| yml.parent().map(|p| p.join("prompts")))
            .ok_or_else(|| no_path(target, "prompts", scope))?;
        let content_path = content_dir.join(format!("{name}.md"));
        let base = yml.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let content_ref =
            rel_slash(&base, &content_path).unwrap_or_else(|| content_path.display().to_string());
        let (body, mut losses) = render_body(&cmd.body, &target.placeholder_map);
        if !cmd.allowed_tools.is_empty() {
            losses.push("allowed-tools (the prompt runs with Rovo's normal permissions)".into());
        }
        if cmd.model.is_some() {
            losses.push(format!("model `{}`", cmd.model.as_deref().unwrap_or("")));
        }
        let mut content =
            PlannedFile::write(&target.id, c, content_path, clean_body(&body), "prompt");
        content.primary = true;
        content.losses = losses;
        let desc = if cmd.description.trim().is_empty() {
            format!("/{name}")
        } else {
            cmd.description.trim().to_string()
        };
        let mut entry = PlannedFile::merge(
            &target.id,
            c,
            yml,
            DocFormat::Yaml,
            vec![PatchOp::Append {
                path: keys(["prompts"]),
                value: json!({"name": name, "description": desc, "content_file": content_ref}),
                dedupe: Dedupe::Fields {
                    fields: vec!["name".into()],
                },
            }],
            "prompt entry",
        );
        entry.notes.push(format!("run it with /prompts {name}"));
        Ok(vec![content, entry])
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        if kind != ComponentKind::Command {
            return Ok(vec![]);
        }
        let Some(yml) = kind_path(target, "commands", scope, ctx) else {
            return Ok(vec![]);
        };
        let Ok(text) = std::fs::read_to_string(&yml) else {
            return Ok(vec![]);
        };
        let Ok(v) = crate::core::agentkit::edit::yaml::parse(&text) else {
            return Ok(vec![]);
        };
        let base = yml.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut out = Vec::new();
        for p in v
            .get("prompts")
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
        {
            let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let Some(file) = p.get("content_file").and_then(|f| f.as_str()) else {
                continue;
            };
            let path = file.split('/').fold(base.clone(), |acc, s| acc.join(s));
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let spec = CommandSpec {
                description: p
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                body: canonical_body(&body, &target.placeholder_map),
                ..Default::default()
            };
            out.extend(command_component(name, &spec, target, &path));
        }
        Ok(out)
    }
}
