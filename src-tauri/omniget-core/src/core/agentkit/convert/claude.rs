//! The Claude-native converter: writes every kind in Claude Code's own format.
//! Used for Claude itself, for tools whose format for a kind *is* Claude's (Codex
//! `hooks.json`, Droid, Qwen …) in their own paths, and for tools that read
//! Claude's files (`reads_claude`) in Claude's paths.
//!
//! Also [`SkillDirConverter`]: a skill folder copied as-is into the tool's own
//! skills directory.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::{
    format_for_path, kind_path, place_support, rewrite_command, ConvertCtx, Converter, Dedupe,
    PatchOp, PlannedFile,
};
use crate::core::agentkit::edit::{keys, textblock, DocFormat, Seg};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse::{self, placeholders};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

/// Kinds the Claude converter can write.
pub fn claude_supports(kind: ComponentKind) -> bool {
    !matches!(kind, ComponentKind::Stack | ComponentKind::Mcp)
}

/// `via = None`: write into the target's own paths. `via = Some(claude)`: write
/// into Claude's paths on behalf of a tool that reads them.
pub struct ClaudeConverter<'a> {
    via: Option<&'a TargetAdapter>,
}

impl ClaudeConverter<'static> {
    pub fn own_paths() -> Self {
        ClaudeConverter { via: None }
    }
}

impl<'a> ClaudeConverter<'a> {
    pub fn via(claude: &'a TargetAdapter) -> Self {
        ClaudeConverter { via: Some(claude) }
    }
}

fn missing(target: &TargetAdapter, what: &str, scope: Scope) -> AgentkitError {
    AgentkitError::new(
        "AGENTKIT_NO_PATH",
        format!(
            "{} has no {what} location for scope {}",
            target.name,
            scope.as_str()
        ),
    )
}

/// Frontmatter entries of a Claude agent file, in Claude's usual order.
pub fn agent_frontmatter(a: &AgentSpec, name: &str) -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = vec![
        ("name".into(), json!(name)),
        ("description".into(), json!(a.description)),
    ];
    if !a.tools.is_empty() {
        v.push(("tools".into(), json!(a.tools.join(", "))));
    }
    if !a.disallowed_tools.is_empty() {
        v.push((
            "disallowedTools".into(),
            json!(a.disallowed_tools.join(", ")),
        ));
    }
    let opt = |k: &str, x: &Option<String>| x.as_ref().map(|s| (k.to_string(), json!(s)));
    v.extend(opt("model", &a.model));
    v.extend(opt("permissionMode", &a.permission_mode));
    if let Some(n) = a.max_turns {
        v.push(("maxTurns".into(), json!(n)));
    }
    if !a.skills.is_empty() {
        v.push(("skills".into(), json!(a.skills)));
    }
    if !a.mcp_servers.is_empty() {
        v.push(("mcpServers".into(), Value::Object(a.mcp_servers.clone())));
    }
    if !a.hooks.is_null() {
        v.push(("hooks".into(), a.hooks.clone()));
    }
    v.extend(opt("color", &a.color));
    if let Some(b) = a.background {
        v.push(("background".into(), json!(b)));
    }
    v.extend(opt("effort", &a.effort));
    v.extend(opt("isolation", &a.isolation));
    v.extend(opt("memory", &a.memory));
    v.extend(opt("initialPrompt", &a.initial_prompt));
    for (k, x) in &a.extra {
        v.push((k.clone(), x.clone()));
    }
    v
}

fn command_frontmatter(cmd: &CommandSpec) -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = Vec::new();
    if !cmd.allowed_tools.is_empty() {
        v.push(("allowed-tools".into(), json!(cmd.allowed_tools.join(", "))));
    }
    if let Some(h) = &cmd.argument_hint {
        v.push(("argument-hint".into(), json!(h)));
    }
    if !cmd.description.is_empty() {
        v.push(("description".into(), json!(cmd.description)));
    }
    if let Some(m) = &cmd.model {
        v.push(("model".into(), json!(m)));
    }
    if let Some(a) = &cmd.agent {
        v.push(("agent".into(), json!(a)));
    }
    for (k, x) in &cmd.extra {
        v.push((k.clone(), x.clone()));
    }
    v
}

/// Raw entry text when the component is Claude-native and unchanged.
fn raw_entry(c: &Component) -> Option<String> {
    (c.origin_tool == "claude")
        .then(|| c.entry_text().map(str::to_string))
        .flatten()
}

/// Every file of a folder-shaped component under `root`.
fn folder_files(target: &str, c: &Component, root: &Path, label: &str) -> Vec<PlannedFile> {
    let prefix = c.entry.rsplit_once('/').map(|(d, _)| {
        // the entry's folder is the component root (skill: `x/SKILL.md` → `x/`)
        let d = d.trim_end_matches(".claude-plugin");
        if d.is_empty() {
            String::new()
        } else {
            format!("{}/", d.trim_end_matches('/'))
        }
    });
    let prefix = match (&c.body, prefix) {
        (ComponentBody::Mod(_) | ComponentBody::Plugin(_), Some(p)) => p,
        (ComponentBody::Skill(_), Some(p)) => p,
        _ => String::new(),
    };
    let mut out = Vec::new();
    for f in &c.files {
        let Some(rel) = f.path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.split('/').any(|p| p == ".." || p.is_empty()) {
            continue;
        }
        let path = rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
        let mut pf = PlannedFile::write(target, c, path, f.bytes.clone(), label);
        pf.executable = f.executable;
        pf.unit_root = Some(root.to_path_buf());
        pf.primary = f.path == c.entry;
        out.push(pf);
    }
    out
}

/// Claude hook group for one canonical entry, translated for a target.
pub(crate) fn hook_group(e: &HookEntry, target: &TargetAdapter, command: Option<String>) -> Value {
    let mut h = Map::new();
    h.insert("type".into(), json!(e.handler.kind));
    if let Some(cmd) = command.or_else(|| e.handler.command.clone()) {
        h.insert("command".into(), json!(cmd));
    }
    if let Some(u) = &e.handler.url {
        h.insert("url".into(), json!(u));
    }
    if let Some(p) = &e.handler.prompt {
        h.insert("prompt".into(), json!(p));
    }
    if let Some(t) = e.handler.timeout {
        let t = if target.hooks.timeout_unit.as_deref() == Some("ms") {
            t * 1000.0
        } else {
            t
        };
        h.insert(
            "timeout".into(),
            if t.fract() == 0.0 {
                json!(t as i64)
            } else {
                json!(t)
            },
        );
    }
    for (k, v) in &e.handler.extra {
        h.insert(k.clone(), v.clone());
    }
    let mut g = Map::new();
    if let Some(m) = &e.matcher {
        g.insert("matcher".into(), json!(target.map_matcher(m)));
    }
    g.insert("hooks".into(), json!([Value::Object(h)]));
    Value::Object(g)
}

impl ClaudeConverter<'_> {
    /// The adapter whose paths we write into.
    fn home_of<'t>(&'t self, target: &'t TargetAdapter) -> &'t TargetAdapter {
        self.via.unwrap_or(target)
    }

    fn support_files(
        &self,
        target_id: &str,
        c: &Component,
        paths_of: &TargetAdapter,
        files: &[SupportFile],
        dir_key: &str,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<(Vec<PlannedFile>, Vec<(String, String)>)> {
        let mut out = Vec::new();
        let mut rewrites = Vec::new();
        for sf in files {
            let Some((abs, shown)) = place_support(&sf.destination, paths_of, dir_key, scope, ctx)
            else {
                return Err(missing(paths_of, dir_key, scope));
            };
            let bytes = if let Some(inline) = sf.source.strip_prefix("inline:") {
                // setting `files` map: content lives in the setting JSON
                let _ = inline;
                continue;
            } else {
                c.file(&sf.source).map(|f| f.bytes.clone()).ok_or_else(|| {
                    AgentkitError::new(
                        "AGENTKIT_PARSE",
                        format!("support file `{}` is missing", sf.source),
                    )
                })?
            };
            let mut pf = PlannedFile::write(target_id, c, abs, bytes, "support script");
            pf.executable = sf.executable;
            out.push(pf);
            let from = sf.destination.trim_start_matches("./").to_string();
            if from != shown {
                rewrites.push((from, shown));
            }
        }
        Ok((out, rewrites))
    }

    fn apply_rewrites(cmd: &str, rewrites: &[(String, String)]) -> String {
        let mut out = cmd.to_string();
        for (from, to) in rewrites {
            out = rewrite_command(&out, from, to);
        }
        out
    }
}

impl Converter for ClaudeConverter<'_> {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        claude_supports(kind)
            && (self.via.is_some()
                || target.format(kind) == Some("claude")
                || target.id == "claude")
    }

    fn native_for(&self, _c: &Component, target: &TargetAdapter) -> bool {
        target.id == "claude" || self.via.is_some()
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let at = self.home_of(target);
        // files written on another tool's behalf are recorded under that tool
        let tid = target.id.as_str();
        let name = ctx.name_for(c);
        let mut out: Vec<PlannedFile> = Vec::new();
        match &c.body {
            ComponentBody::Agent(a) => {
                let dir = kind_path(at, "agents", scope, ctx)
                    .ok_or_else(|| missing(at, "agents", scope))?;
                let mut losses = Vec::new();
                let content = match raw_entry(c) {
                    Some(raw)
                        if name == parse::sanitize_name(&a.name) || ctx.name_override.is_none() =>
                    {
                        raw
                    }
                    // a Copilot chatmode: its tool ids become Claude tool names
                    _ if c.origin_tool == "copilot" => {
                        let (_, lost) = super::agent_common::claude_tools(&a.tools, &c.origin_tool);
                        losses.extend(lost);
                        super::agent_simulated::claude_text(c, a, &name)
                    }
                    _ => parse::render_frontmatter(&agent_frontmatter(a, &name), &a.prompt),
                };
                let mut pf =
                    PlannedFile::write(tid, c, dir.join(format!("{name}.md")), content, "agent");
                pf.primary = true;
                pf.losses = losses;
                out.push(pf);
            }
            ComponentBody::Command(cmd) => {
                let dir = kind_path(at, "commands", scope, ctx)
                    .ok_or_else(|| missing(at, "commands", scope))?;
                let content = raw_entry(c).unwrap_or_else(|| {
                    parse::render_frontmatter(
                        &command_frontmatter(cmd),
                        &placeholders::to_claude(&cmd.body),
                    )
                });
                let mut pf =
                    PlannedFile::write(tid, c, dir.join(format!("{name}.md")), content, "command");
                pf.primary = true;
                out.push(pf);
            }
            ComponentBody::Rule(r) => match r.scope {
                RuleScope::Scoped => {
                    let dir = kind_path(at, "scoped_rules", scope, ctx)
                        .ok_or_else(|| missing(at, "scoped rules", scope))?;
                    let mut fm: Vec<(String, Value)> = Vec::new();
                    if !r.description.is_empty() {
                        fm.push(("description".into(), json!(r.description)));
                    }
                    fm.push(("paths".into(), json!(r.globs)));
                    let mut pf = PlannedFile::write(
                        tid,
                        c,
                        dir.join(format!("{name}.md")),
                        parse::render_frontmatter(&fm, &r.markdown),
                        "rule",
                    );
                    pf.primary = true;
                    out.push(pf);
                }
                RuleScope::Root => {
                    let file = kind_path(at, "rules", scope, ctx)
                        .ok_or_else(|| missing(at, "rules file", scope))?;
                    let id = format!("{}:{}", c.id, name);
                    let pf = PlannedFile::merge(
                        tid,
                        c,
                        file,
                        DocFormat::Markdown,
                        vec![PatchOp::TextBlock {
                            id,
                            content: r.markdown.trim().to_string(),
                        }],
                        "rule",
                    );
                    out.push(pf);
                }
            },
            ComponentBody::Skill(s) => {
                let base = kind_path(at, "skills", scope, ctx)
                    .ok_or_else(|| missing(at, "skills", scope))?;
                let dir_name = ctx
                    .name_override
                    .clone()
                    .unwrap_or_else(|| s.dir_name.clone());
                out.extend(folder_files(tid, c, &base.join(dir_name), "skill"));
            }
            ComponentBody::Hook(h) => {
                let file = if scope == Scope::Local {
                    kind_path(at, "hooks", Scope::Local, ctx)
                } else {
                    kind_path(at, "hooks", scope, ctx)
                }
                .ok_or_else(|| missing(at, "hooks file", scope))?;
                let (files, rewrites) = self.support_files(
                    tid,
                    c,
                    at,
                    &h.supporting_files,
                    "hook_scripts",
                    scope,
                    ctx,
                )?;
                let mut ops = Vec::new();
                let mut losses = Vec::new();
                let mut commands = Vec::new();
                for e in &h.entries {
                    let events = at.map_event(&e.event);
                    if events.is_empty() {
                        losses.push(format!("hook event {}", e.event));
                        continue;
                    }
                    let cmd = e
                        .handler
                        .command
                        .as_deref()
                        .map(|x| Self::apply_rewrites(x, &rewrites));
                    if let Some(x) = &cmd {
                        commands.push(x.clone());
                    }
                    for ev in events {
                        ops.push(PatchOp::Append {
                            path: keys(["hooks", ev.as_str()]),
                            value: hook_group(e, at, cmd.clone()),
                            dedupe: Dedupe::ClaudeHook,
                        });
                    }
                }
                if c.files.iter().any(|f| {
                    f.text()
                        .map(|t| t.contains("CLAUDE_TOOL_FILE_PATH"))
                        .unwrap_or(false)
                }) {
                    losses.push("uses $CLAUDE_TOOL_FILE_PATH, which Claude Code does not set (read tool_input.file_path from stdin)".into());
                }
                if !ops.is_empty() {
                    let mut pf = PlannedFile::merge(
                        tid,
                        c,
                        file.clone(),
                        format_for_path(&file),
                        ops,
                        "hook",
                    );
                    pf.losses = losses;
                    pf.commands = commands;
                    out.push(pf);
                }
                out.extend(files);
            }
            ComponentBody::Setting(sp) => {
                let file = kind_path(at, "settings", scope, ctx)
                    .ok_or_else(|| missing(at, "settings file", scope))?;
                let (files, rewrites) = self.support_files(
                    tid,
                    c,
                    at,
                    &sp.supporting_files,
                    "statusline_scripts",
                    scope,
                    ctx,
                )?;
                let mut ops = Vec::new();
                let mut commands = Vec::new();
                for (k, v) in &sp.values {
                    match (k.as_str(), v) {
                        ("permissions", Value::Object(p)) => {
                            for (pk, pv) in p {
                                match pv {
                                    Value::Array(items) => {
                                        for it in items {
                                            ops.push(PatchOp::Append {
                                                path: keys(["permissions", pk.as_str()]),
                                                value: it.clone(),
                                                dedupe: Dedupe::Equal,
                                            });
                                        }
                                    }
                                    other => ops.push(PatchOp::Set {
                                        path: keys(["permissions", pk.as_str()]),
                                        value: other.clone(),
                                        rename_at: None,
                                    }),
                                }
                            }
                        }
                        ("env", Value::Object(envs)) => {
                            for (ek, ev) in envs {
                                ops.push(PatchOp::Set {
                                    path: keys(["env", ek.as_str()]),
                                    value: ev.clone(),
                                    rename_at: None,
                                });
                            }
                        }
                        ("hooks", hooks) => {
                            for e in parse::hook_entries(hooks) {
                                let cmd = e
                                    .handler
                                    .command
                                    .as_deref()
                                    .map(|x| Self::apply_rewrites(x, &rewrites));
                                if let Some(x) = &cmd {
                                    commands.push(x.clone());
                                }
                                for ev in at.map_event(&e.event) {
                                    ops.push(PatchOp::Append {
                                        path: keys(["hooks", ev.as_str()]),
                                        value: hook_group(&e, at, cmd.clone()),
                                        dedupe: Dedupe::ClaudeHook,
                                    });
                                }
                            }
                        }
                        ("statusLine", Value::Object(sl)) => {
                            let mut sl = sl.clone();
                            if let Some(Value::String(cmd)) = sl.get("command").cloned() {
                                let cmd = Self::apply_rewrites(&cmd, &rewrites);
                                commands.push(cmd.clone());
                                sl.insert("command".into(), json!(cmd));
                            }
                            ops.push(PatchOp::Set {
                                path: keys(["statusLine"]),
                                value: Value::Object(sl),
                                rename_at: None,
                            });
                        }
                        _ => ops.push(PatchOp::Set {
                            path: vec![Seg::Key(k.clone())],
                            value: v.clone(),
                            rename_at: None,
                        }),
                    }
                }
                let mut pf = PlannedFile::merge(
                    tid,
                    c,
                    file.clone(),
                    format_for_path(&file),
                    ops,
                    "settings",
                );
                pf.commands = commands;
                out.push(pf);
                out.extend(files);
            }
            ComponentBody::Statusline(sl) => {
                let file = kind_path(at, "statusline", scope, ctx)
                    .ok_or_else(|| missing(at, "statusline", scope))?;
                let (files, rewrites) = self.support_files(
                    tid,
                    c,
                    at,
                    &sl.supporting_files,
                    "statusline_scripts",
                    scope,
                    ctx,
                )?;
                let cmd = Self::apply_rewrites(&sl.command, &rewrites);
                let mut v = Map::new();
                v.insert("type".into(), json!("command"));
                v.insert("command".into(), json!(cmd));
                if let Some(p) = sl.padding {
                    v.insert("padding".into(), json!(p));
                }
                let key: Vec<Seg> = at
                    .statusline
                    .as_ref()
                    .filter(|s| !s.key.is_empty())
                    .map(|s| keys(s.key.iter().map(|k| k.as_str())))
                    .unwrap_or_else(|| keys(["statusLine"]));
                let mut pf = PlannedFile::merge(
                    tid,
                    c,
                    file.clone(),
                    format_for_path(&file),
                    vec![PatchOp::Set {
                        path: key,
                        value: Value::Object(v),
                        rename_at: None,
                    }],
                    "statusline",
                );
                pf.commands = vec![cmd];
                out.push(pf);
                out.extend(files);
            }
            ComponentBody::Loop(_) => {
                let dir = kind_path(at, "loops", scope, ctx)
                    .ok_or_else(|| missing(at, "loops", scope))?;
                let content = c.entry_text().unwrap_or_default().to_string();
                let mut pf =
                    PlannedFile::write(tid, c, dir.join(format!("{name}.md")), content, "loop");
                pf.primary = true;
                pf.notes.push("the loop's referenced components are installed with it; OmniGet runs the schedule".into());
                out.push(pf);
            }
            ComponentBody::Workflow(w) => {
                let dir = kind_path(at, "workflows", scope, ctx)
                    .ok_or_else(|| missing(at, "workflows", scope))?;
                let content = w
                    .yaml
                    .clone()
                    .or_else(|| c.entry_text().map(str::to_string))
                    .unwrap_or_default();
                let mut pf = PlannedFile::write(
                    tid,
                    c,
                    dir.join(format!("{name}.yaml")),
                    content,
                    "workflow",
                );
                pf.primary = true;
                out.push(pf);
            }
            ComponentBody::Mod(m) => {
                let base =
                    kind_path(at, "mods", scope, ctx).ok_or_else(|| missing(at, "mods", scope))?;
                let mut files = folder_files(tid, c, &base.join(&m.dir_name), "mod");
                if let Some(first) = files.first_mut() {
                    first.notes.push("function hooks need CLAUDE_CODE_ENABLE_FUNCTION_HOOKS=1 (Claude Code ≥ 2.1.259)".into());
                }
                out.extend(files);
            }
            ComponentBody::Plugin(p) => {
                let base = kind_path(at, "plugins", scope, ctx)
                    .or_else(|| kind_path(at, "mods", scope, ctx))
                    .ok_or_else(|| missing(at, "plugins", scope))?;
                out.extend(folder_files(tid, c, &base.join(&p.dir_name), "plugin"));
            }
            ComponentBody::ProjectTemplate(t) => {
                if let Some(md) = &t.agents_md {
                    let file = kind_path(at, "rules", scope, ctx)
                        .ok_or_else(|| missing(at, "rules file", scope))?;
                    out.push(PlannedFile::merge(
                        tid,
                        c,
                        file,
                        DocFormat::Markdown,
                        vec![PatchOp::TextBlock {
                            id: format!("{}:{}", c.id, name),
                            content: md.trim().to_string(),
                        }],
                        "template rules",
                    ));
                }
            }
            ComponentBody::SandboxRecipe(s) => {
                if scope != Scope::Project {
                    return Err(missing(at, "sandbox (project only)", scope));
                }
                let base = ctx
                    .project
                    .ok_or_else(|| missing(at, "project", scope))?
                    .join(".claude")
                    .join("sandbox")
                    .join(&s.provider);
                out.extend(folder_files(tid, c, &base, "sandbox"));
            }
            ComponentBody::Mcp(_) | ComponentBody::Stack(_) => {}
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
        let at = self.home_of(target);
        let mut out = Vec::new();
        let dir_kind = |key: &str| kind_path(at, key, scope, ctx);
        match kind {
            ComponentKind::Agent | ComponentKind::Command => {
                let key = if kind == ComponentKind::Agent {
                    "agents"
                } else {
                    "commands"
                };
                let Some(dir) = dir_kind(key) else {
                    return Ok(out);
                };
                for p in md_files(&dir) {
                    if let Ok(mut comp) = parse::parse_path(kind, &p) {
                        tag_installed(&mut comp, target, &p);
                        out.push(comp);
                    }
                }
            }
            ComponentKind::Skill => {
                let Some(dir) = dir_kind("skills") else {
                    return Ok(out);
                };
                let Ok(rd) = std::fs::read_dir(&dir) else {
                    return Ok(out);
                };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.join("SKILL.md").is_file() {
                        if let Ok(mut comp) = parse::parse_path(kind, &p) {
                            tag_installed(&mut comp, target, &p);
                            out.push(comp);
                        }
                    }
                }
            }
            ComponentKind::Hook => {
                let Some(file) = dir_kind("hooks") else {
                    return Ok(out);
                };
                if let Ok(text) = std::fs::read_to_string(&file) {
                    if let Ok(v) =
                        crate::core::agentkit::edit::parse_value(format_for_path(&file), &text)
                    {
                        let entries = parse::hook_entries(v.get("hooks").unwrap_or(&Value::Null));
                        if !entries.is_empty() {
                            let files: parse::RawFiles = [(
                                "hooks.json".to_string(),
                                serde_json::to_vec_pretty(&json!({"hooks": v.get("hooks")}))
                                    .unwrap_or_default(),
                            )]
                            .into_iter()
                            .collect();
                            if let Ok(mut comp) =
                                parse::parse_raw(ComponentKind::Hook, "hooks.json", &files)
                            {
                                comp.name = format!("{}-hooks", target.id);
                                tag_installed(&mut comp, target, &file);
                                out.push(comp);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    }
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().map(|x| x == "md").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub(crate) fn tag_installed(c: &mut Component, target: &TargetAdapter, path: &Path) {
    c.id = format!("installed:{}:{}/{}", target.id, c.kind.as_str(), c.name);
    c.source = Some(SourceRef {
        id: format!("installed:{}", target.id),
        path: Some(path.display().to_string()),
        ..Default::default()
    });
}

/// A skill folder copied into the tool's own skills directory.
pub struct SkillDirConverter;

impl Converter for SkillDirConverter {
    fn id(&self) -> &'static str {
        "skill_dir"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Skill && target.format(kind) == Some("skill_dir")
    }

    fn native_for(&self, _c: &Component, _target: &TargetAdapter) -> bool {
        true
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Skill(s) = &c.body else {
            return Ok(vec![]);
        };
        let base = kind_path(target, "skills", scope, ctx)
            .ok_or_else(|| missing(target, "skills", scope))?;
        let dir_name = ctx
            .name_override
            .clone()
            .unwrap_or_else(|| s.dir_name.clone());
        Ok(folder_files(&target.id, c, &base.join(dir_name), "skill"))
    }

    fn import(
        &self,
        kind: ComponentKind,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<Component>> {
        ClaudeConverter::own_paths().import(kind, target, scope, ctx)
    }
}

/// Text block helper re-exported for converters that append to rules files.
pub fn rules_block(text: &str, id: &str, content: &str) -> String {
    textblock::upsert(text, id, content)
}
