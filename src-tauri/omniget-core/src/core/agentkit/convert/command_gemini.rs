//! Gemini CLI custom commands: `.gemini/commands/<name>.toml` with
//! `description` and `prompt` (estudo 06 Gemini (c), §0C.3): `$ARGUMENTS` →
//! `{{args}}`, `` !`cmd` `` → `!{cmd}`, `@path` → `@{path}`; positional
//! arguments do not exist (the text is kept readable and the loss recorded).

use serde_json::json;

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::edit::toml as etoml;
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

pub struct GeminiCommandConverter;

impl GeminiCommandConverter {
    /// TOML text and losses.
    pub fn render(cmd: &CommandSpec, target: &TargetAdapter) -> (String, Vec<String>) {
        let (body, mut losses) = render_body(&cmd.body, &target.placeholder_map);
        let mut out = String::new();
        if !cmd.description.trim().is_empty() {
            out.push_str(&toml_line("description", &json!(cmd.description.trim())));
        }
        let mut prompt = clean_body(&body);
        if !prompt.ends_with('\n') {
            prompt.push('\n');
        }
        out.push_str(&toml_line("prompt", &json!(prompt)));
        if !cmd.allowed_tools.is_empty() {
            losses.push("allowed-tools (the command runs with Gemini's normal permissions)".into());
        }
        if cmd.model.is_some() {
            losses.push(format!("model `{}`", cmd.model.as_deref().unwrap_or("")));
        }
        if cmd.agent.is_some() {
            losses.push("agent (run in a subagent)".into());
        }
        (out, losses)
    }

    /// TOML command → canonical spec.
    pub fn read(text: &str, target: &TargetAdapter) -> Option<CommandSpec> {
        let v = etoml::parse(text).ok()?;
        let prompt = v.get("prompt")?.as_str()?;
        Some(CommandSpec {
            description: v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string(),
            body: format!("\n{}", canonical_body(prompt, &target.placeholder_map)),
            ..Default::default()
        })
    }
}

impl Converter for GeminiCommandConverter {
    fn id(&self) -> &'static str {
        "command_gemini_toml"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Command && target.format(kind) == Some("gemini_toml")
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
        let dir = dir_for(target, "commands", scope, ctx)?;
        let (text, losses) = Self::render(cmd, target);
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(format!("{name}.toml")),
            text,
            "command",
        );
        pf.primary = true;
        pf.losses = losses;
        Ok(vec![pf])
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
        let Some(dir) = kind_path(target, "commands", scope, ctx) else {
            return Ok(vec![]);
        };
        let mut out = Vec::new();
        for p in files_with(&dir, &[".toml"]) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            if let Some(spec) = Self::read(&text, target) {
                if let Some(c) = command_component(&stem_of(&p, &[".toml"]), &spec, target, &p) {
                    out.push(c);
                }
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

    #[test]
    fn gemini_toml_round_trip() {
        let text = "---\ndescription: Fix an issue\n---\nFix: $ARGUMENTS\nDiff: !`git diff --staged`\nStyle: @docs/style.md\nQuote \"\"\" and \\ stay.\n";
        let files: parse::RawFiles = [("fix.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        let c = parse::parse_raw(ComponentKind::Command, "fix.md", &files).unwrap();
        let ComponentBody::Command(cmd) = &c.body else {
            unreachable!()
        };
        let t = targets::target("gemini").unwrap();
        let (toml, losses) = GeminiCommandConverter::render(cmd, t);
        assert!(losses.is_empty(), "{losses:?}");
        let v = etoml::parse(&toml).unwrap_or_else(|e| panic!("{e}\n{toml}"));
        let prompt = v["prompt"].as_str().unwrap();
        assert!(prompt.contains("Fix: {{args}}"), "{prompt}");
        assert!(prompt.contains("!{git diff --staged}"), "{prompt}");
        assert!(prompt.contains("@{docs/style.md}"), "{prompt}");
        assert!(prompt.contains("Quote \"\"\" and \\ stay."), "{prompt}");
        let back = GeminiCommandConverter::read(&toml, t).unwrap();
        assert_eq!(back.description, "Fix an issue");
        assert_eq!(
            parse::placeholders::to_claude(&back.body).trim(),
            text.split("---\n").nth(2).unwrap().trim()
        );
    }
}
