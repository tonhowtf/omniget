//! Slash commands as Markdown in each tool's dialect (estudo 06 §0C.3): Qwen,
//! OpenCode/Kilo, Cursor (plain), Copilot `.prompt.md`, Cline workflows,
//! Windsurf/Antigravity workflows, Crush (`$NAME`), Kiro prompts, Pi prompt
//! templates, Continue prompts, Junie (`$argName`) and Roo. Also the fallback for
//! tools without commands (a skill with `disable-model-invocation: true`) and the
//! deprecated Codex `~/.codex/prompts` (only on request, never auto-picked).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::agent_common::*;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{Result, Scope};

/// Format ids served by [`MdCommandConverter`].
pub const FORMATS: &[&str] = &[
    "qwen_md",
    "opencode_md",
    "cursor_md",
    "copilot_prompt",
    "cline_workflow",
    "devin_workflow",
    "antigravity_workflow",
    "crush_md",
    "kiro_prompt",
    "pi_prompt",
    "continue_prompt",
    "junie",
    "roo",
];

pub struct MdCommandConverter {
    pub format: &'static str,
}

/// Placeholder dialect for a format (manifest map plus format specifics).
pub(crate) fn dialect(
    format: &str,
    target: &TargetAdapter,
    hint: Option<&str>,
) -> BTreeMap<String, String> {
    let mut m = target.placeholder_map.clone();
    match format {
        "copilot_prompt" => {
            let h = hint
                .map(|h| h.replace(['}', '{', '\n'], " ").trim().to_string())
                .filter(|h| !h.is_empty());
            m.insert(
                "args".into(),
                match h {
                    Some(h) => format!("${{input:args:{h}}}"),
                    None => "${input:args}".into(),
                },
            );
        }
        "cursor_md" | "cline_workflow" | "devin_workflow" | "antigravity_workflow" => {
            for k in ["args", "arg", "shell", "file"] {
                m.insert(k.into(), String::new());
            }
        }
        "opencode_md" => {
            // OpenCode/Kilo read the Claude placeholders 1:1
            m.insert("args".into(), "$ARGUMENTS".into());
            m.insert("arg".into(), "$N".into());
            m.insert("shell".into(), "!`{cmd}`".into());
            m.insert("file".into(), "@{path}".into());
        }
        _ => {}
    }
    m
}

fn uses_args(body: &str) -> bool {
    body.contains("{{args}}")
}

impl MdCommandConverter {
    fn file_name(&self, name: &str) -> String {
        if self.format == "copilot_prompt" {
            format!("{name}.prompt.md")
        } else {
            format!("{name}.md")
        }
    }

    /// Command file text, losses and notes.
    pub fn render(
        &self,
        c: &Component,
        cmd: &CommandSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>, Vec<String>) {
        let map = dialect(self.format, target, cmd.argument_hint.as_deref());
        let (body, mut losses) = render_body(&cmd.body, &map);
        let mut notes: Vec<String> = Vec::new();
        let mut fm: Vec<(String, Value)> = Vec::new();
        let mut kept_tools = false;
        let mut kept_hint = false;
        let mut kept_model = false;
        let mut kept_agent = false;
        let desc = cmd.description.trim();
        match self.format {
            "qwen_md" | "devin_workflow" | "antigravity_workflow" | "kiro_prompt" => {
                if !desc.is_empty() && self.format != "kiro_prompt" {
                    fm.push(("description".into(), json!(desc)));
                }
            }
            "opencode_md" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                if let Some(a) = &cmd.agent {
                    fm.push(("agent".into(), json!(a)));
                    kept_agent = true;
                }
                let (m, l) = model_for(cmd.model.as_deref(), &c.origin_tool, target);
                if let Some(m) = m {
                    fm.push(("model".into(), json!(m)));
                }
                losses.extend(l);
                kept_model = true;
                if cmd.extra.get("context").and_then(|v| v.as_str()) == Some("fork") {
                    fm.push(("subtask".into(), json!(true)));
                }
            }
            "copilot_prompt" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                if let Some(h) = &cmd.argument_hint {
                    fm.push(("argument-hint".into(), json!(h)));
                    kept_hint = true;
                }
                fm.push((
                    "agent".into(),
                    json!(cmd.agent.clone().unwrap_or_else(|| "agent".into())),
                ));
                kept_agent = true;
                if c.origin_tool == "copilot" {
                    if let Some(m) = &cmd.model {
                        fm.push(("model".into(), json!(m)));
                    }
                    kept_model = true;
                }
                if !cmd.allowed_tools.is_empty() {
                    let mut ids: Vec<String> = Vec::new();
                    for t in &cmd.allowed_tools {
                        match claude_to_copilot(t) {
                            Some(x) => push_unique(&mut ids, x),
                            None if c.origin_tool == "copilot" => push_unique(&mut ids, t.clone()),
                            None => {
                                push_unique(&mut losses, format!("tool `{t}` (no Copilot alias)"))
                            }
                        }
                    }
                    fm.push(("tools".into(), json!(ids)));
                    kept_tools = true;
                }
                notes.push("prompt files are read by VS Code's Local agent; the Copilot CLI reads .claude/commands instead".into());
            }
            "pi_prompt" | "roo" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                if let Some(h) = &cmd.argument_hint {
                    fm.push(("argument-hint".into(), json!(h)));
                    kept_hint = true;
                }
            }
            "continue_prompt" => {
                fm.push(("name".into(), json!(name)));
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                fm.push(("invokable".into(), json!(true)));
            }
            "junie" => {
                if !desc.is_empty() {
                    fm.push(("description".into(), json!(desc)));
                }
                if uses_args(&cmd.body) {
                    fm.push(("allowPromptArgument".into(), json!(true)));
                }
            }
            "crush_md" => {
                notes.push("Crush asks for every $UPPER_NAME in the file, so shell variables like $HOME written in the text become prompts".into());
            }
            _ => {}
        }
        if !cmd.allowed_tools.is_empty() && !kept_tools {
            losses
                .push("allowed-tools (the command runs with the tool's normal permissions)".into());
        }
        // argument-hint is UI sugar: dropping it changes nothing the model sees
        let _ = kept_hint;
        if cmd.model.is_some() && !kept_model {
            losses.push(format!("model `{}`", cmd.model.as_deref().unwrap_or("")));
        }
        if cmd.agent.is_some() && !kept_agent {
            losses.push("agent (run in a subagent)".into());
        }
        let mut uniq = Vec::new();
        for l in losses {
            push_unique(&mut uniq, l);
        }
        (parse::render_frontmatter(&fm, &body), uniq, notes)
    }

    /// A command file of this format → canonical spec.
    pub fn read_back(&self, text: &str, target: &TargetAdapter) -> CommandSpec {
        let (fm, body) = if matches!(self.format, "crush_md" | "cursor_md" | "cline_workflow") {
            (Default::default(), text)
        } else {
            parse::frontmatter(text)
        };
        let hint = str_of(&fm, &["argument-hint"]);
        let map = dialect(self.format, target, hint.as_deref());
        let mut body = canonical_body(body, &map);
        if self.format == "copilot_prompt" {
            // `${input:args:hint}` → {{args}}
            while let Some(a) = body.find("${input:args") {
                let Some(e) = body[a..].find('}') else { break };
                body.replace_range(a..a + e + 1, "{{args}}");
            }
        }
        if self.format == "junie" {
            body = body.replace("$prompt", "{{args}}");
        }
        let allowed = if self.format == "copilot_prompt" {
            claude_tools(&parse::string_list(fm.get("tools")), "copilot").0
        } else {
            vec![]
        };
        CommandSpec {
            description: str_of(&fm, &["description"]).unwrap_or_default(),
            argument_hint: hint,
            allowed_tools: allowed,
            model: str_of(&fm, &["model"]).map(|m| model_back(&m)),
            agent: str_of(&fm, &["agent"])
                .filter(|a| self.format != "copilot_prompt" || a != "agent"),
            body,
            extra: Default::default(),
        }
    }
}

impl Converter for MdCommandConverter {
    fn id(&self) -> &'static str {
        match self.format {
            "qwen_md" => "command_qwen_md",
            "opencode_md" => "command_opencode_md",
            "cursor_md" => "command_cursor_md",
            "copilot_prompt" => "command_copilot_prompt",
            "cline_workflow" => "command_cline_workflow",
            "devin_workflow" => "command_devin_workflow",
            "antigravity_workflow" => "command_antigravity_workflow",
            "crush_md" => "command_crush_md",
            "kiro_prompt" => "command_kiro_prompt",
            "pi_prompt" => "command_pi_prompt",
            "continue_prompt" => "command_continue_prompt",
            "junie" => "command_junie",
            "roo" => "command_roo",
            _ => "command_md",
        }
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Command && target.format(kind) == Some(self.format)
    }

    fn native_for(&self, c: &Component, _target: &TargetAdapter) -> bool {
        self.format == "copilot_prompt" && c.origin_tool == "copilot"
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
        let (text, losses, notes) = self.render(c, cmd, &name, target);
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(self.file_name(&name)),
            text,
            "command",
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
        if kind != ComponentKind::Command {
            return Ok(vec![]);
        }
        let Some(dir) = kind_path(target, "commands", scope, ctx) else {
            return Ok(vec![]);
        };
        let suffix: &[&str] = if self.format == "copilot_prompt" {
            &[".prompt.md"]
        } else {
            &[".md"]
        };
        let mut out = Vec::new();
        for p in files_with(&dir, suffix) {
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let spec = self.read_back(&text, target);
            if let Some(c) = command_component(&stem_of(&p, suffix), &spec, target, &p) {
                out.push(c);
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ command → skill

/// Tools without commands (Codex, Devin CLI, Kimi, Vibe, Warp, Zed, Amp …): the
/// command becomes a user-invoked skill (`/name`), hidden from the model.
pub struct SkillCommandConverter;

/// Where command-skills go: the manifest's `paths.commands` (Devin, Vibe, Warp,
/// Zed point it at their skills folder) or the skills folder.
fn skill_base(
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Option<std::path::PathBuf> {
    kind_path(target, "commands", scope, ctx).or_else(|| kind_path(target, "skills", scope, ctx))
}

impl SkillCommandConverter {
    /// SKILL.md text and losses.
    pub fn render(
        c: &Component,
        cmd: &CommandSpec,
        name: &str,
        target: &TargetAdapter,
    ) -> (String, Vec<String>) {
        let (body, mut losses) = render_body(&cmd.body, &target.placeholder_map);
        let mut fm: Vec<(String, Value)> = vec![
            ("name".into(), json!(name)),
            (
                "description".into(),
                json!(if cmd.description.trim().is_empty() {
                    format!("/{name} command")
                } else {
                    cmd.description.trim().to_string()
                }),
            ),
            ("disable-model-invocation".into(), json!(true)),
        ];
        if target.id == "devin" {
            fm.push(("triggers".into(), json!(["user"])));
        }
        if let Some(h) = &cmd.argument_hint {
            fm.push(("argument-hint".into(), json!(h)));
        }
        if !cmd.allowed_tools.is_empty() {
            let mut out: Vec<String> = Vec::new();
            for t in &cmd.allowed_tools {
                let (base, _) = tool_base(t);
                let (m, l) = map_tools(&[base.to_string()], target);
                if m.len() == 1 && m[0] == base {
                    push_unique(&mut out, t.clone());
                } else {
                    for x in m {
                        push_unique(&mut out, x);
                    }
                    losses.extend(l);
                }
            }
            if !out.is_empty() {
                fm.push(("allowed-tools".into(), json!(out.join(", "))));
            }
        }
        if let Some(m) = &cmd.model {
            let (mm, l) = model_for(Some(m), &c.origin_tool, target);
            if let Some(mm) = mm {
                fm.push(("model".into(), json!(mm)));
            }
            losses.extend(l);
        }
        if cmd.agent.is_some() {
            losses.push("agent (run in a subagent)".into());
        }
        let mut uniq = Vec::new();
        for l in losses {
            push_unique(&mut uniq, l);
        }
        (parse::render_frontmatter(&fm, &body), uniq)
    }
}

impl Converter for SkillCommandConverter {
    fn id(&self) -> &'static str {
        "command_skill"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Command && target.format(kind) == Some("skill")
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
        let base =
            skill_base(target, scope, ctx).ok_or_else(|| no_path(target, "skills", scope))?;
        let root = base.join(&name);
        let (text, losses) = Self::render(c, cmd, &name, target);
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            root.join("SKILL.md"),
            text,
            "command (skill)",
        );
        pf.primary = true;
        pf.unit_root = Some(root.clone());
        pf.losses = losses;
        pf.notes.push(format!(
            "{} has no command files: installed as a skill invoked by the user as /{name}",
            target.name
        ));
        let mut out = vec![pf];
        if target.id == "codex" {
            // Codex's own switch for "only when asked"
            let mut meta = PlannedFile::write(
                &target.id,
                c,
                root.join("agents").join("openai.yaml"),
                "policy:\n  allow_implicit_invocation: false\n",
                "skill metadata",
            );
            meta.unit_root = Some(root);
            out.push(meta);
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
        if kind != ComponentKind::Command {
            return Ok(vec![]);
        }
        let Some(base) = skill_base(target, scope, ctx) else {
            return Ok(vec![]);
        };
        let Ok(rd) = std::fs::read_dir(&base) else {
            return Ok(vec![]);
        };
        let mut dirs: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        dirs.sort();
        let mut out = Vec::new();
        for d in dirs {
            let p = d.join("SKILL.md");
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let (fm, body) = parse::frontmatter(&text);
            if fm.get("disable-model-invocation").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            let spec = CommandSpec {
                description: str_of(&fm, &["description"]).unwrap_or_default(),
                argument_hint: str_of(&fm, &["argument-hint"]),
                allowed_tools: parse::string_list(fm.get("allowed-tools")),
                model: str_of(&fm, &["model"]),
                agent: None,
                body: canonical_body(body, &target.placeholder_map),
                extra: Default::default(),
            };
            let name = str_of(&fm, &["name"]).unwrap_or_else(|| {
                d.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("command")
                    .to_string()
            });
            if let Some(c) = command_component(&name, &spec, target, &p) {
                out.push(c);
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ Codex legacy prompts

/// Deprecated Codex custom prompts (`~/.codex/prompts/<name>.md`, user scope
/// only). Never picked by the registry (`supports` is false): call it through
/// `Registry::by_id("command_codex_prompts")` when the user asks for the legacy file.
pub struct CodexPromptConverter;

impl Converter for CodexPromptConverter {
    fn id(&self) -> &'static str {
        "command_codex_prompts"
    }

    fn supports(&self, _kind: ComponentKind, _target: &TargetAdapter) -> bool {
        false
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        _scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let Some(cmd) = command_of(c) else {
            return Ok(vec![]);
        };
        let name = ctx.name_for(c);
        let dir = dir_for(target, "prompts", Scope::Global, ctx)?;
        let (body, losses) = render_body(&cmd.body, &target.placeholder_map);
        let mut fm: Vec<(String, Value)> = Vec::new();
        if !cmd.description.is_empty() {
            fm.push(("description".into(), json!(cmd.description)));
        }
        if let Some(h) = &cmd.argument_hint {
            fm.push(("argument-hint".into(), json!(h)));
        }
        let mut pf = PlannedFile::write(
            &target.id,
            c,
            dir.join(format!("{name}.md")),
            parse::render_frontmatter(&fm, &body),
            "prompt (legacy)",
        );
        pf.primary = true;
        pf.losses = losses;
        pf.notes.push(format!(
            "deprecated in Codex: invoked as /prompts:{name}; prefer the skill"
        ));
        Ok(vec![pf])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::targets;

    pub(crate) fn feature_cmd() -> Component {
        let text = "---\nallowed-tools: Bash(git:*)\nargument-hint: <feature-name>\ndescription: Create a feature branch\n---\n\nCreate **$ARGUMENTS** from $1.\n- branch: !`git branch --show-current`\nSee @docs/flow.md\n";
        let files: parse::RawFiles = [("feature.md".to_string(), text.as_bytes().to_vec())]
            .into_iter()
            .collect();
        parse::parse_raw(ComponentKind::Command, "feature.md", &files).unwrap()
    }

    fn rt(format: &'static str, tid: &str) -> (String, Vec<String>, CommandSpec) {
        let c = feature_cmd();
        let ComponentBody::Command(cmd) = &c.body else {
            unreachable!()
        };
        let t = targets::target(tid).unwrap();
        let conv = MdCommandConverter { format };
        let (text, losses, _) = conv.render(&c, cmd, "feature", t);
        let back = conv.read_back(&text, t);
        (text, losses, back)
    }

    #[test]
    fn qwen_uses_gemini_placeholders() {
        let (text, losses, back) = rt("qwen_md", "qwen");
        assert!(text.contains("**{{args}}**"), "{text}");
        assert!(text.contains("!{git branch --show-current}"), "{text}");
        assert!(text.contains("@{docs/flow.md}"), "{text}");
        assert!(
            losses.iter().any(|l| l.contains("positional")),
            "{losses:?}"
        );
        assert!(back.body.contains("{{shell:git branch --show-current}}"));
        assert_eq!(back.description, "Create a feature branch");
    }

    #[test]
    fn opencode_is_one_to_one() {
        let (text, _, back) = rt("opencode_md", "opencode");
        assert!(text.contains("**$ARGUMENTS** from $1"), "{text}");
        assert!(text.contains("!`git branch --show-current`"), "{text}");
        let c = feature_cmd();
        let ComponentBody::Command(cmd) = &c.body else {
            unreachable!()
        };
        assert_eq!(back.body, cmd.body);
    }

    #[test]
    fn cursor_is_plain_and_copilot_uses_inputs() {
        let (text, losses, _) = rt("cursor_md", "cursor");
        assert!(!text.starts_with("---"), "{text}");
        assert!(text.contains("<arguments>"), "{text}");
        assert!(
            text.contains("(run `git branch --show-current` and use its output)"),
            "{text}"
        );
        assert!(losses.iter().any(|l| l.contains("allowed-tools")));
        let (text, _, back) = rt("copilot_prompt", "copilot");
        assert!(text.contains("${input:args:<feature-name>}"), "{text}");
        assert!(text.contains("#file:docs/flow.md"), "{text}");
        assert!(text.contains("tools:"), "{text}");
        assert!(back.body.contains("**{{args}}**"), "{}", back.body);
    }

    #[test]
    fn crush_and_junie_named_arguments() {
        let (text, _, _) = rt("crush_md", "crush");
        assert!(text.contains("**$ARGUMENTS** from $ARG1"), "{text}");
        let (text, _, back) = rt("junie", "junie");
        assert!(text.contains("allowPromptArgument: true"), "{text}");
        assert!(text.contains("**$prompt**"), "{text}");
        assert!(back.body.contains("**{{args}}**"));
    }

    #[test]
    fn command_as_codex_skill() {
        let c = feature_cmd();
        let ComponentBody::Command(cmd) = &c.body else {
            unreachable!()
        };
        let t = targets::target("codex").unwrap();
        let (text, _) = SkillCommandConverter::render(&c, cmd, "feature", t);
        let (fm, body) = parse::frontmatter(&text);
        assert_eq!(fm["disable-model-invocation"], true);
        assert_eq!(fm["name"], "feature");
        assert!(body.contains("**$ARGUMENTS**"), "{body}");
    }
}
