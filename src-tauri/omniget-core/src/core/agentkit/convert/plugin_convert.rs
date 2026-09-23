//! Claude plugins (`.claude-plugin/plugin.json` + components) for tools that do
//! not read the Claude plugin format (0C.8). Tools that do (Copilot, Cursor,
//! Droid, Devin, Grok, Auggie, Junie, Qoder, Codex marketplaces, Vibe) are
//! served by the Claude converter; this one writes:
//!
//! * **Gemini extensions** — `~/.gemini/extensions/<name>/gemini-extension.json`
//!   (MCP inline with `${extensionPath}`), commands rewritten to TOML, hooks to
//!   Gemini's dialect;
//! * **Qwen extensions** — `~/.qwen/extensions/<name>/qwen-extension.json`;
//! * **Agent Plugins 1.0** (Kiro Powers, OpenHands) — root `plugin.json`,
//!   `skills/`, `mcp.json`;
//! * **code plugins** (OpenCode, Kilo, Amp, Pi) — skills into the tool's skills
//!   folder, MCP into its config, hooks as a generated plugin, the plugin's
//!   files kept next to it for `${CLAUDE_PLUGIN_ROOT}`.
//!
//! Mods (function-hook plugins) stay Claude-only: the Claude converter writes
//! them into `.claude/skills/<name>/` with the `CLAUDE_CODE_ENABLE_FUNCTION_HOOKS=1` note.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::hook_code_plugin;
use super::hook_common::{missing, prepare};
use super::hook_json::gemini_groups;
use super::mcp::McpConverter;
use super::{kind_path, ConvertCtx, Converter, PlannedFile};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

pub const FORMATS: &[&str] = &[
    "gemini_extension",
    "qwen_extension",
    "agent_plugins",
    "code_plugin",
    "pi",
];

pub struct PluginConverter;

/// Folder prefix of the plugin inside `c.files` (`x/` for `x/.claude-plugin/plugin.json`).
fn prefix(c: &Component) -> String {
    let d = c.entry.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let d = d.trim_end_matches(".claude-plugin").trim_end_matches('/');
    if d.is_empty() {
        String::new()
    } else {
        format!("{d}/")
    }
}

/// `(path relative to the plugin root, file)`.
fn files(c: &Component) -> Vec<(String, &ComponentFile)> {
    let pre = prefix(c);
    c.files
        .iter()
        .filter_map(|f| {
            let rel = f.path.strip_prefix(&pre)?;
            if rel.split('/').any(|p| p == ".." || p.is_empty()) {
                return None;
            }
            Some((rel.to_string(), f))
        })
        .collect()
}

fn rel_file<'a>(all: &'a [(String, &'a ComponentFile)], rel: &str) -> Option<&'a ComponentFile> {
    all.iter().find(|(r, _)| r == rel).map(|(_, f)| *f)
}

fn json_file(all: &[(String, &ComponentFile)], rel: &str) -> Option<Value> {
    rel_file(all, rel).and_then(|f| serde_json::from_slice(&f.bytes).ok())
}

/// Replaces `${CLAUDE_PLUGIN_ROOT}` (and the bare form) in every string.
fn replace_root(v: &Value, to: &str) -> Value {
    match v {
        Value::String(s) => Value::String(replace_root_str(s, to)),
        Value::Array(a) => Value::Array(a.iter().map(|x| replace_root(x, to)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| (k.clone(), replace_root(x, to)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn replace_root_str(s: &str, to: &str) -> String {
    s.replace("${CLAUDE_PLUGIN_ROOT}", to)
        .replace("$CLAUDE_PLUGIN_ROOT", to)
}

/// MCP servers of the plugin (`.mcp.json` and manifest `mcpServers`).
fn mcp_servers(p: &PluginSpec, all: &[(String, &ComponentFile)]) -> Map<String, Value> {
    let mut out = Map::new();
    let mut take = |v: &Value| {
        let servers = v.get("mcpServers").unwrap_or(v);
        if let Some(m) = servers.as_object() {
            for (k, s) in m {
                if s.is_object() {
                    out.insert(k.clone(), s.clone());
                }
            }
        }
    };
    if let Some(v) = json_file(all, ".mcp.json") {
        take(&v);
    }
    match p.manifest.get("mcpServers") {
        Some(Value::String(path)) => {
            if let Some(v) = json_file(all, path.trim_start_matches("./")) {
                take(&v);
            }
        }
        Some(v @ Value::Object(_)) => take(v),
        _ => {}
    }
    out
}

/// Hook entries of the plugin (`hooks/hooks.json` and manifest `hooks`), with
/// the plugin root replaced.
fn hook_entries(p: &PluginSpec, all: &[(String, &ComponentFile)], root: &str) -> Vec<HookEntry> {
    let mut docs = Vec::new();
    if let Some(v) = json_file(all, "hooks/hooks.json") {
        docs.push(v);
    }
    match p.manifest.get("hooks") {
        Some(Value::String(path)) if path.trim_start_matches("./") != "hooks/hooks.json" => {
            if let Some(v) = json_file(all, path.trim_start_matches("./")) {
                docs.push(v);
            }
        }
        Some(v @ Value::Object(_)) => docs.push(json!({ "hooks": v })),
        _ => {}
    }
    let mut out = Vec::new();
    for d in docs {
        let hooks = d.get("hooks").cloned().unwrap_or(d);
        for mut e in parse::hook_entries(&replace_root(&hooks, root)) {
            if let Some(cmd) = &e.handler.command {
                e.handler.command = Some(replace_root_str(cmd, root));
            }
            out.push(e);
        }
    }
    out
}

fn top(rel: &str) -> &str {
    rel.split('/').next().unwrap_or("")
}

/// Parts of a Claude plugin that no other tool has.
fn claude_only_losses(all: &[(String, &ComponentFile)], p: &PluginSpec) -> Vec<String> {
    let mut out = Vec::new();
    for (rel, _) in all {
        let l = match top(rel) {
            "output-styles" => "output styles",
            "workflows" => "workflows",
            "monitors" => "monitors",
            "themes" => "themes",
            ".lsp.json" => "LSP servers",
            "settings.json" => "plugin settings.json",
            _ => continue,
        };
        let l = format!("plugin {l}");
        if !out.contains(&l) {
            out.push(l);
        }
    }
    if p.manifest.get("userConfig").is_some() {
        out.push("plugin userConfig (no settings UI outside Claude)".into());
    }
    if p.manifest.get("lspServers").is_some() && !out.iter().any(|l| l.contains("LSP")) {
        out.push("plugin LSP servers".into());
    }
    out
}

fn write_file(
    target: &TargetAdapter,
    c: &Component,
    root: &Path,
    rel: &str,
    bytes: Vec<u8>,
    executable: bool,
    label: &str,
) -> PlannedFile {
    let path = rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
    let mut pf = PlannedFile::write(&target.id, c, path, bytes, label);
    pf.executable = executable;
    pf.unit_root = Some(root.to_path_buf());
    pf
}

fn plugins_dir(target: &TargetAdapter, scope: Scope, ctx: &ConvertCtx) -> Result<PathBuf> {
    kind_path(target, "plugins", scope, ctx).ok_or_else(|| missing(target, "plugins", scope))
}

fn toml_basic(s: &str) -> String {
    let mut o = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn toml_multiline(s: &str) -> String {
    format!(
        "\"\"\"\n{}\"\"\"",
        s.replace('\\', "\\\\").replace("\"\"\"", "\"\"\\\"")
    )
}

/// A Claude command file → Gemini TOML command.
fn gemini_command(
    name: &str,
    bytes: &[u8],
    target: &TargetAdapter,
) -> Option<(String, Vec<String>)> {
    let mut raw = parse::RawFiles::new();
    raw.insert(format!("{name}.md"), bytes.to_vec());
    let comp = parse::parse_raw(ComponentKind::Command, &format!("{name}.md"), &raw).ok()?;
    let ComponentBody::Command(cmd) = comp.body else {
        return None;
    };
    let (body, lost) = parse::placeholders::render(&cmd.body, &target.placeholder_map);
    let mut t = String::new();
    if !cmd.description.is_empty() {
        t.push_str(&format!("description = {}\n", toml_basic(&cmd.description)));
    }
    t.push_str(&format!("prompt = {}\n", toml_multiline(body.trim_end())));
    Some((t, lost))
}

fn manifest_meta(p: &PluginSpec, c: &Component) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(p.name));
    m.insert(
        "version".into(),
        json!(p.version.clone().unwrap_or_else(|| "1.0.0".into())),
    );
    let desc = p
        .manifest
        .get("description")
        .and_then(|d| d.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| c.description.clone());
    if !desc.is_empty() {
        m.insert("description".into(), json!(desc));
    }
    m
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s.into_bytes()
}

// ---------------------------------------------------------------- Gemini

fn gemini_extension(
    c: &Component,
    p: &PluginSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let name = ctx
        .name_override
        .clone()
        .unwrap_or_else(|| p.dir_name.clone());
    let root = plugins_dir(target, scope, ctx)?.join(&name);
    let root_s = root.display().to_string();
    let all = files(c);
    let mut out = Vec::new();
    let mut losses = claude_only_losses(&all, p);
    for (rel, f) in &all {
        let t = top(rel);
        if t == ".claude-plugin" || rel == ".mcp.json" || rel == "hooks/hooks.json" {
            continue;
        }
        if t == "commands" && rel.ends_with(".md") {
            let stem = rel.trim_start_matches("commands/").trim_end_matches(".md");
            if let Some((toml, lost)) = gemini_command(stem, &f.bytes, target) {
                for l in lost {
                    let l = format!("command placeholder {l}");
                    if !losses.contains(&l) {
                        losses.push(l);
                    }
                }
                out.push(write_file(
                    target,
                    c,
                    &root,
                    &format!("commands/{stem}.toml"),
                    toml.into_bytes(),
                    false,
                    "extension command",
                ));
                continue;
            }
        }
        if t == "agents" && !losses.iter().any(|l| l.contains("agent frontmatter")) {
            losses.push(
                "agent frontmatter kept in Claude's form (tool names and model not translated)"
                    .into(),
            );
        }
        out.push(write_file(
            target,
            c,
            &root,
            rel,
            f.bytes.clone(),
            f.executable,
            "extension file",
        ));
    }
    // manifest
    let mut m = manifest_meta(p, c);
    let servers = mcp_servers(p, &all);
    if !servers.is_empty() {
        let v = replace_root(&Value::Object(servers), "${extensionPath}");
        // Gemini: streamable HTTP servers use httpUrl
        let mut fixed = Map::new();
        for (k, mut s) in v.as_object().cloned().unwrap_or_default() {
            if let Some(obj) = s.as_object_mut() {
                let ty = obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                if ty == "http" {
                    if let Some(u) = obj.remove("url") {
                        obj.insert("httpUrl".into(), u);
                    }
                }
                if matches!(ty.as_str(), "http" | "sse" | "stdio") {
                    obj.remove("type");
                }
            }
            fixed.insert(k, s);
        }
        m.insert("mcpServers".into(), Value::Object(fixed));
    }
    let mut manifest = write_file(
        target,
        c,
        &root,
        "gemini-extension.json",
        pretty(&Value::Object(m)),
        false,
        "extension manifest",
    );
    manifest.primary = true;
    out.insert(0, manifest);
    // hooks in Gemini's dialect
    let entries = hook_entries(p, &all, &root_s);
    if !entries.is_empty() {
        let spec = HookSpec {
            entries,
            supporting_files: vec![],
            tags: vec![],
        };
        let (prep, hl, cmds) = prepare(&spec, target, &[], &[]);
        losses.extend(hl);
        let mut hooks = Map::new();
        for (native, g) in gemini_groups(&prep, target, ctx, &name) {
            if let Some(a) = hooks
                .entry(native)
                .or_insert_with(|| json!([]))
                .as_array_mut()
            {
                a.push(g);
            }
        }
        let mut hf = write_file(
            target,
            c,
            &root,
            "hooks/hooks.json",
            pretty(&json!({ "hooks": hooks })),
            false,
            "extension hooks",
        );
        hf.commands = cmds;
        out.push(hf);
    }
    if let Some(f) = out.first_mut() {
        f.losses = losses;
        f.notes.push("Gemini loads extensions from ~/.gemini/extensions at start; `gemini extensions disable` turns it off".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------- Qwen

fn qwen_extension(
    c: &Component,
    p: &PluginSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let name = ctx
        .name_override
        .clone()
        .unwrap_or_else(|| p.dir_name.clone());
    let root = plugins_dir(target, scope, ctx)?.join(&name);
    let all = files(c);
    let mut losses = claude_only_losses(&all, p);
    let mut out = Vec::new();
    let mut dirs: Vec<&str> = Vec::new();
    for (rel, f) in &all {
        let t = top(rel);
        if t == ".claude-plugin" || rel == ".mcp.json" {
            continue;
        }
        if t == "hooks" {
            if !losses.iter().any(|l| l == "plugin hooks") {
                losses.push("plugin hooks".into());
            }
            continue;
        }
        if matches!(t, "commands" | "skills" | "agents") && !dirs.contains(&t) {
            dirs.push(t);
        }
        out.push(write_file(
            target,
            c,
            &root,
            rel,
            f.bytes.clone(),
            f.executable,
            "extension file",
        ));
    }
    let mut m = manifest_meta(p, c);
    let servers = mcp_servers(p, &all);
    if !servers.is_empty() {
        m.insert(
            "mcpServers".into(),
            replace_root(&Value::Object(servers), "${extensionPath}"),
        );
    }
    for d in dirs {
        m.insert(d.into(), json!(d));
    }
    let mut manifest = write_file(
        target,
        c,
        &root,
        "qwen-extension.json",
        pretty(&Value::Object(m)),
        false,
        "extension manifest",
    );
    manifest.primary = true;
    manifest.losses = losses;
    out.insert(0, manifest);
    Ok(out)
}

// ---------------------------------------------------------------- Agent Plugins 1.0

fn agent_plugin(
    c: &Component,
    p: &PluginSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let name = ctx
        .name_override
        .clone()
        .unwrap_or_else(|| p.dir_name.clone());
    let root = plugins_dir(target, scope, ctx)?.join(&name);
    let root_s = root.display().to_string();
    let all = files(c);
    let mut losses = claude_only_losses(&all, p);
    let mut out = Vec::new();
    for (rel, f) in &all {
        let t = top(rel);
        match t {
            ".claude-plugin" | ".mcp.json" => continue,
            "commands" | "agents" | "hooks" => {
                let l = format!("plugin {t} (Agent Plugins carry skills and MCP only)");
                if !losses.contains(&l) {
                    losses.push(l);
                }
                continue;
            }
            _ => {}
        }
        out.push(write_file(
            target,
            c,
            &root,
            rel,
            f.bytes.clone(),
            f.executable,
            "plugin file",
        ));
    }
    let mut m = Map::new();
    m.insert(
        "$schema".into(),
        json!("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"),
    );
    for (k, v) in manifest_meta(p, c) {
        m.insert(k, v);
    }
    for k in ["author", "homepage", "repository", "license", "keywords"] {
        if let Some(v) = p.manifest.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    let mut manifest = write_file(
        target,
        c,
        &root,
        "plugin.json",
        pretty(&Value::Object(m)),
        false,
        "plugin manifest",
    );
    manifest.primary = true;
    manifest.losses = losses;
    out.insert(0, manifest);
    let servers = mcp_servers(p, &all);
    if !servers.is_empty() {
        let v = json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": replace_root(&Value::Object(servers), &root_s),
        });
        out.push(write_file(
            target,
            c,
            &root,
            "mcp.json",
            pretty(&v),
            false,
            "plugin MCP",
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------- code plugins

fn code_plugin(
    c: &Component,
    p: &PluginSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<Vec<PlannedFile>> {
    let hooks_dir = kind_path(target, "hooks", scope, ctx)
        .ok_or_else(|| missing(target, "plugins folder", scope))?;
    let name = ctx
        .name_override
        .clone()
        .unwrap_or_else(|| p.dir_name.clone());
    // the plugin's own files, for ${CLAUDE_PLUGIN_ROOT}; outside the auto-loaded folder
    let support_root = hooks_dir
        .parent()
        .map(|d| d.join("omniget-plugins").join(&name))
        .ok_or_else(|| missing(target, "plugins folder", scope))?;
    let root_s = support_root.display().to_string();
    let all = files(c);
    let mut losses = claude_only_losses(&all, p);
    let mut out = Vec::new();
    let skills_dir = kind_path(target, "skills", scope, ctx);
    for (rel, f) in &all {
        let t = top(rel);
        if t == ".claude-plugin" {
            continue;
        }
        if matches!(t, "commands" | "agents") {
            let l = format!("plugin {t} (install them as components of their own)");
            if !losses.contains(&l) {
                losses.push(l);
            }
        }
        out.push(write_file(
            target,
            c,
            &support_root,
            rel,
            f.bytes.clone(),
            f.executable,
            "plugin file",
        ));
        if t == "skills" {
            if let Some(sd) = &skills_dir {
                let inner = rel.trim_start_matches("skills/");
                let mut pf = write_file(
                    target,
                    c,
                    sd,
                    inner,
                    f.bytes.clone(),
                    f.executable,
                    "plugin skill",
                );
                let skill_root = inner.split('/').next().map(|s| sd.join(s));
                pf.unit_root = skill_root;
                out.push(pf);
            }
        }
    }
    // hooks → generated plugin
    let entries = hook_entries(p, &all, &root_s);
    if !entries.is_empty() {
        let spec = HookSpec {
            entries,
            supporting_files: vec![],
            tags: vec![],
        };
        let (prep, hl, cmds) = prepare(&spec, target, &[], &[]);
        losses.extend(hl);
        let hooks = hook_code_plugin::plugin_hooks(&prep);
        if !hooks.is_empty() {
            let (fname, text) = hook_code_plugin::render(&target.id, &c.id, &name, ctx.env, &hooks);
            let mut pf =
                PlannedFile::write(&target.id, c, hooks_dir.join(fname), text, "plugin hooks");
            pf.primary = true;
            pf.commands = cmds;
            out.insert(0, pf);
        }
    }
    // MCP → the tool's own config, through the MCP converter
    let servers = mcp_servers(p, &all);
    if !servers.is_empty() {
        let servers = replace_root(&Value::Object(servers), &root_s);
        let list: Vec<McpServer> = servers
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| parse::mcp_from_record(k, v))
                    .collect()
            })
            .unwrap_or_default();
        let mut mc = c.clone();
        mc.kind = ComponentKind::Mcp;
        mc.body = ComponentBody::Mcp(McpSpec { servers: list });
        let files = McpConverter.convert(&mc, target, scope, ctx)?;
        for mut f in files {
            f.component_id = c.id.clone();
            out.push(f);
        }
    }
    if let Some(f) = out.first_mut() {
        for l in losses {
            if !f.losses.contains(&l) {
                f.losses.push(l);
            }
        }
    }
    Ok(out)
}

impl Converter for PluginConverter {
    fn id(&self) -> &'static str {
        "plugin_dialects"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Plugin
            && target
                .format(kind)
                .map(|f| FORMATS.contains(&f))
                .unwrap_or(false)
    }

    fn convert(
        &self,
        c: &Component,
        target: &TargetAdapter,
        scope: Scope,
        ctx: &ConvertCtx,
    ) -> Result<Vec<PlannedFile>> {
        let ComponentBody::Plugin(p) = &c.body else {
            return Err(AgentkitError::new(
                "AGENTKIT_KIND",
                format!("{} is not a plugin", c.id),
            ));
        };
        match target.format(ComponentKind::Plugin).unwrap_or("") {
            "gemini_extension" => gemini_extension(c, p, target, scope, ctx),
            "qwen_extension" => qwen_extension(c, p, target, scope, ctx),
            "agent_plugins" => agent_plugin(c, p, target, scope, ctx),
            "code_plugin" | "pi" => code_plugin(c, p, target, scope, ctx),
            other => Err(AgentkitError::new(
                "AGENTKIT_UNSUPPORTED",
                format!("{} plugin format `{other}` has no writer", target.name),
            )),
        }
    }
}
