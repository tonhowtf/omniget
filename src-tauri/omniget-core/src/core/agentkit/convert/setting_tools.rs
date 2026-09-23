//! Setting components (Claude `settings.json` fragments, OmniGet profiles) on
//! the other tools (plan §4.3, estudo 06 (g) of each tool):
//!
//! | format        | where                                   | permissions as                         |
//! |---------------|-----------------------------------------|----------------------------------------|
//! | `qwen`        | `.qwen/settings.json`                   | Claude rules + `tools.approvalMode`    |
//! | `qoder`       | `.qoder/settings.json`                  | Claude rules + `general.defaultPermissionMode` |
//! | `letta`       | `.letta/settings.json`                  | Claude rules (allow/deny) + `env`      |
//! | `cursor_cli`  | `.cursor/cli.json` / `~/.cursor/cli-config.json` | `Shell()/Read()/Write()/WebFetch()/Mcp()` |
//! | `codex_toml`  | `config.toml` + `rules/omniget-*.rules` | `sandbox_mode`/`approval_policy`, Starlark `prefix_rule` |
//! | `opencode`    | `opencode.json` / `kilo.jsonc`          | `permission` map with globs            |
//! | `gemini`      | `~/.gemini/policies/omniget-*.toml` (user) or `tools.allowed/exclude` (project) | Policy Engine `[[rule]]` |
//! | `devin`       | `.devin/config.json`                    | `Read()/Write()/Exec()/Fetch()`        |
//! | `kiro`        | `~/.kiro/settings/permissions.yaml`     | `rules: [{capability, match, effect}]` |
//! | `copilot_cli` | `~/.copilot/settings.json`              | `allowedUrls/deniedUrls` (+ flags note)|
//! | `droid`       | `~/.factory/settings.json`              | `commandAllowlist/Denylist`, autonomy  |
//! | `goose`       | `permission.yaml` + `config.yaml`       | `always_allow/ask_before/never_allow`, `GOOSE_MODE` |
//!
//! Keys only Claude has (spinner, output style, announcements …) become losses;
//! `env` that points at another endpoint (GLM, MiniMax, a proxy, OmniGet's
//! "alternative provider" profile) is mapped where the tool has an equivalent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::setting_perm::{
    self as sp, Decision, Family, Mode, PermissionSet, Protocol, Provider, ProviderKind, Rule,
};
use super::{format_for_path, kind_path, ConvertCtx, Converter, Dedupe, PatchOp, PlannedFile};
use crate::core::agentkit::edit::{keys, DocFormat, Seg};
use crate::core::agentkit::model::*;
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

/// Setting formats this module writes.
pub const FORMATS: &[&str] = &[
    "qwen",
    "qoder",
    "letta",
    "cursor_cli",
    "codex_toml",
    "opencode",
    "gemini",
    "devin",
    "kiro",
    "copilot_cli",
    "droid",
    "goose",
];

pub struct SettingConverter;

impl Converter for SettingConverter {
    fn id(&self) -> &'static str {
        "setting_tools"
    }

    fn supports(&self, kind: ComponentKind, target: &TargetAdapter) -> bool {
        kind == ComponentKind::Setting
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
        let ComponentBody::Setting(spec) = &c.body else {
            return Ok(vec![]);
        };
        let mut o = Out::new(c, target, scope, ctx);
        let perms = PermissionSet::from_values(&spec.values);
        let provider = sp::provider_from(&spec.values);
        let fmt = target.format(ComponentKind::Setting).unwrap_or("");
        match fmt {
            "qwen" | "qoder" | "letta" => claude_like(&mut o, &perms)?,
            "cursor_cli" => cursor(&mut o, &perms)?,
            "codex_toml" => codex(&mut o, &perms, provider.as_ref())?,
            "opencode" => opencode(&mut o, &perms, provider.as_ref())?,
            "gemini" => gemini(&mut o, &perms)?,
            "devin" => devin(&mut o, &perms)?,
            "kiro" => kiro(&mut o, &perms)?,
            "copilot_cli" => copilot(&mut o, &perms)?,
            "droid" => droid(&mut o, &perms)?,
            "goose" => goose(&mut o, &perms, provider.as_ref())?,
            _ => {}
        }
        if !perms.other.is_empty() {
            o.loss(format!(
                "permission keys only Claude Code has: {}",
                perms.other.join(", ")
            ));
        }
        common(&mut o, fmt, spec, provider.as_ref())?;
        Ok(o.finish())
    }
}

// ------------------------------------------------------------------ output collector

struct Out<'a> {
    c: &'a Component,
    t: &'a TargetAdapter,
    scope: Scope,
    ctx: &'a ConvertCtx<'a>,
    merges: Vec<(PathBuf, DocFormat, Vec<PatchOp>, &'static str)>,
    files: Vec<PlannedFile>,
    losses: Vec<String>,
    notes: Vec<String>,
}

impl<'a> Out<'a> {
    fn new(c: &'a Component, t: &'a TargetAdapter, scope: Scope, ctx: &'a ConvertCtx<'a>) -> Self {
        Out {
            c,
            t,
            scope,
            ctx,
            merges: vec![],
            files: vec![],
            losses: vec![],
            notes: vec![],
        }
    }

    fn loss(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.losses.contains(&s) {
            self.losses.push(s);
        }
    }

    fn note(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    fn missing(&self, what: &str) -> AgentkitError {
        AgentkitError::new(
            "AGENTKIT_NO_PATH",
            format!(
                "{} has no {what} location for scope {}",
                self.t.name,
                self.scope.as_str()
            ),
        )
    }

    /// The tool's settings file for the scope.
    fn settings(&self) -> Result<PathBuf> {
        kind_path(self.t, "settings", self.scope, self.ctx)
            .ok_or_else(|| self.missing("settings file"))
    }

    /// Settings file of another scope (Cursor model/approval live in the user file).
    fn settings_at(&self, scope: Scope) -> Option<PathBuf> {
        kind_path(self.t, "settings", scope, self.ctx)
    }

    fn op(&mut self, file: &Path, fmt: DocFormat, op: PatchOp, label: &'static str) {
        if let Some(m) = self.merges.iter_mut().find(|m| m.0 == file) {
            m.2.push(op);
            return;
        }
        self.merges.push((file.to_path_buf(), fmt, vec![op], label));
    }

    fn set(&mut self, file: &Path, fmt: DocFormat, path: Vec<Seg>, value: Value) {
        self.op(
            file,
            fmt,
            PatchOp::Set {
                path,
                value,
                rename_at: None,
            },
            "settings",
        );
    }

    fn append(&mut self, file: &Path, fmt: DocFormat, path: Vec<Seg>, value: Value) {
        self.op(
            file,
            fmt,
            PatchOp::Append {
                path,
                value,
                dedupe: Dedupe::Equal,
            },
            "settings",
        );
    }

    fn write(&mut self, path: PathBuf, content: String, label: &str) {
        let mut pf = PlannedFile::write(&self.t.id, self.c, path, content, label);
        pf.primary = true;
        self.files.push(pf);
    }

    fn finish(self) -> Vec<PlannedFile> {
        let mut out: Vec<PlannedFile> = Vec::new();
        for (path, fmt, ops, label) in self.merges {
            out.push(PlannedFile::merge(
                &self.t.id, self.c, path, fmt, ops, label,
            ));
        }
        out.extend(self.files);
        if let Some(first) = out.first_mut() {
            for l in self.losses {
                if !first.losses.contains(&l) {
                    first.losses.push(l);
                }
            }
            for n in self.notes {
                if !first.notes.contains(&n) {
                    first.notes.push(n);
                }
            }
        }
        out
    }
}

fn json_fmt(path: &Path) -> DocFormat {
    format_for_path(path)
}

/// Model value for a tool: only explicit `model_map` entries (and qualified
/// `provider/model` ids for OpenCode-style tools).
fn tool_model(t: &TargetAdapter, model: &str, qualified_ok: bool) -> Option<String> {
    match t.model_map.get(model) {
        Some(m) if !m.is_empty() => Some(m.clone()),
        Some(_) => None,
        None if qualified_ok && model.contains('/') => Some(model.to_string()),
        None => None,
    }
}

fn component_slug(c: &Component, ctx: &ConvertCtx) -> String {
    ctx.name_for(c)
}

// ------------------------------------------------------------------ shared keys

/// `env`, `model`, `statusLine`, `hooks`, `apiKeyHelper` and the Claude-only keys.
fn common(o: &mut Out, fmt: &str, spec: &SettingSpec, provider: Option<&Provider>) -> Result<()> {
    let values = &spec.values;
    // env
    if let Some(Value::Object(env)) = values.get("env") {
        let mut claude_only = Vec::new();
        let mut proxy = Vec::new();
        for (k, v) in env {
            if sp::PROVIDER_ENV.contains(&k.as_str()) && provider.is_some() {
                continue;
            }
            if sp::TELEMETRY_ENV.contains(&k.as_str()) {
                continue;
            }
            if matches!(
                k.as_str(),
                "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "NO_PROXY"
                    | "http_proxy"
                    | "https_proxy"
                    | "no_proxy"
            ) {
                proxy.push(k.clone());
                continue;
            }
            if fmt == "letta" {
                let file = o.settings()?;
                o.set(&file, json_fmt(&file), keys(["env", k.as_str()]), v.clone());
                continue;
            }
            claude_only.push(k.clone());
        }
        if !proxy.is_empty() {
            o.loss(format!(
                "{} has no proxy setting in its file: export {} in the shell that starts it",
                o.t.name,
                proxy.join("/")
            ));
        }
        if !claude_only.is_empty() {
            o.loss(format!(
                "environment variables only Claude Code reads: {}",
                claude_only.join(", ")
            ));
        }
        if let Some(off) = sp::telemetry_off(env) {
            telemetry(o, fmt, !off)?;
        }
    }
    // model
    if let Some(m) = values.get("model").and_then(|m| m.as_str()) {
        model(o, fmt, m)?;
    }
    // provider on tools whose writer did not take it
    if let Some(p) = provider {
        if !matches!(fmt, "codex_toml" | "opencode" | "goose" | "qwen") {
            o.loss(provider_loss(o.t, p));
        } else if fmt == "qwen" {
            qwen_provider(o, p)?;
        }
    }
    // statusLine
    if let Some(Value::Object(sl)) = values.get("statusLine") {
        let cmd = sl.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let sl_fmt = o.t.format(ComponentKind::Statusline).unwrap_or("");
        if !cmd.is_empty() && super::statusline_stdin::FORMATS.contains(&sl_fmt) {
            let files = super::statusline_stdin::statusline_files(
                o.c,
                o.t,
                o.scope,
                o.ctx,
                cmd,
                sl.get("padding").and_then(|p| p.as_i64()),
                &spec.supporting_files,
            )?;
            o.files.extend(files);
        } else {
            o.loss(format!("{} has no command statusline", o.t.name));
        }
    }
    if values.contains_key("hooks") {
        o.loss(
            "hooks inside a setting install only on Claude Code; install them as a hook component",
        );
    }
    if values.contains_key("apiKeyHelper") && provider.is_none() {
        o.loss("apiKeyHelper is Claude-only");
    }
    let other: Vec<&String> = values
        .keys()
        .filter(|k| !sp::MODELLED_KEYS.contains(&k.as_str()))
        .collect();
    if !other.is_empty() {
        o.loss(format!(
            "settings only Claude Code has: {}",
            other
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

fn provider_loss(t: &TargetAdapter, p: &Provider) -> String {
    match p.kind {
        ProviderKind::Bedrock => format!("{} has no Amazon Bedrock setting in its file", t.name),
        ProviderKind::Vertex => format!("{} has no Vertex AI setting in its file", t.name),
        ProviderKind::Custom => format!(
            "{} cannot point at another {} endpoint from its settings file",
            t.name,
            match p.protocol {
                Protocol::Anthropic => "Anthropic-compatible",
                Protocol::OpenAi => "OpenAI-compatible",
            }
        ),
    }
}

fn telemetry(o: &mut Out, fmt: &str, enabled: bool) -> Result<()> {
    match fmt {
        "gemini" | "qwen" => {
            let file = o.settings()?;
            o.set(
                &file,
                json_fmt(&file),
                keys(["telemetry", "enabled"]),
                json!(enabled),
            );
        }
        "goose" => {
            let file = o
                .settings_at(Scope::Global)
                .ok_or_else(|| o.missing("config.yaml"))?;
            o.set(
                &file,
                DocFormat::Yaml,
                keys(["GOOSE_TELEMETRY_ENABLED"]),
                json!(enabled),
            );
        }
        "codex_toml" => {
            let file = o.settings()?;
            o.set(
                &file,
                DocFormat::Toml,
                keys(["analytics", "enabled"]),
                json!(enabled),
            );
        }
        _ => o.loss(format!(
            "{} has no telemetry switch in its settings",
            o.t.name
        )),
    }
    Ok(())
}

fn model(o: &mut Out, fmt: &str, m: &str) -> Result<()> {
    let qualified = matches!(fmt, "opencode");
    let Some(v) = tool_model(o.t, m, qualified) else {
        o.loss(format!("model `{m}` has no equivalent on {}", o.t.name));
        return Ok(());
    };
    let (file, path): (Option<PathBuf>, Vec<Seg>) = match fmt {
        "qwen" | "gemini" => (Some(o.settings()?), keys(["model", "name"])),
        "codex_toml" | "opencode" | "copilot_cli" | "droid" => {
            (Some(o.settings()?), keys(["model"]))
        }
        "cursor_cli" => (o.settings_at(Scope::Global), keys(["model"])),
        "devin" => (Some(o.settings()?), keys(["agent", "model"])),
        "goose" => (o.settings_at(Scope::Global), keys(["GOOSE_MODEL"])),
        _ => (None, vec![]),
    };
    match file {
        Some(f) => {
            let fm = if fmt == "devin" {
                DocFormat::Jsonc
            } else {
                json_fmt(&f)
            };
            o.set(&f, fm, path, json!(v));
        }
        None => o.loss(format!("{} has no model setting in its file", o.t.name)),
    }
    Ok(())
}

// ------------------------------------------------------------------ Claude-syntax tools

fn claude_like(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = json_fmt(&file);
    let pc = o.t.permissions.clone().unwrap_or_default();
    let base: Vec<String> = if pc.key.is_empty() {
        vec!["permissions".into()]
    } else {
        pc.key.clone()
    };
    let lists: Vec<String> = if pc.lists.is_empty() {
        vec!["allow".into(), "ask".into(), "deny".into()]
    } else {
        pc.lists.clone()
    };
    for (d, r) in &perms.rules {
        let list = d.as_str();
        if !lists.iter().any(|l| l == list) {
            o.loss(format!(
                "{} has no `{list}` list: `{}` dropped",
                o.t.name,
                r.to_claude()
            ));
            continue;
        }
        let mut path: Vec<Seg> = base.iter().map(|k| Seg::Key(k.clone())).collect();
        path.push(Seg::Key(list.into()));
        o.append(&file, fmt, path, json!(r.to_claude()));
    }
    let fmt_id = o.t.format(ComponentKind::Setting).unwrap_or("").to_string();
    if !perms.additional_dirs.is_empty() {
        if fmt_id == "qoder" {
            for d in &perms.additional_dirs {
                o.append(
                    &file,
                    fmt,
                    keys(["permissions", "additionalDirectories"]),
                    json!(d),
                );
            }
        } else {
            o.loss(format!("{} has no additionalDirectories", o.t.name));
        }
    }
    if let Some(m) = perms.mode {
        let v = match (fmt_id.as_str(), m) {
            ("qwen", Mode::Default) => Some(("tools.approvalMode", "default")),
            ("qwen", Mode::AcceptEdits) => Some(("tools.approvalMode", "auto-edit")),
            ("qwen", Mode::Plan) => Some(("tools.approvalMode", "plan")),
            ("qwen", Mode::Auto) => Some(("tools.approvalMode", "auto")),
            ("qwen", Mode::Bypass) => Some(("tools.approvalMode", "yolo")),
            ("qoder", Mode::Default) => Some(("general.defaultPermissionMode", "default")),
            ("qoder", Mode::AcceptEdits) => Some(("general.defaultPermissionMode", "accept_edits")),
            ("qoder", Mode::Plan) => Some(("general.defaultPermissionMode", "plan")),
            ("qoder", Mode::Auto) => Some(("general.defaultPermissionMode", "auto")),
            ("qoder", Mode::Bypass) => {
                Some(("general.defaultPermissionMode", "bypass_permissions"))
            }
            ("qoder", Mode::DontAsk) => Some(("general.defaultPermissionMode", "dont_ask")),
            _ => None,
        };
        match v {
            Some((k, val)) => o.set(&file, fmt, keys(k.split('.')), json!(val)),
            None => o.loss(format!(
                "permission mode `{}` has no setting on {}",
                m.as_claude(),
                o.t.name
            )),
        }
    }
    Ok(())
}

fn qwen_provider(o: &mut Out, p: &Provider) -> Result<()> {
    if p.kind != ProviderKind::Custom || p.protocol != Protocol::OpenAi {
        o.loss(provider_loss(o.t, p));
        return Ok(());
    }
    let Some(model) = p.model.clone() else {
        o.loss("Qwen needs a model id for a custom OpenAI-compatible provider");
        return Ok(());
    };
    let file = o.settings()?;
    let fmt = json_fmt(&file);
    let mut entry = Map::new();
    entry.insert("id".into(), json!(model));
    if let Some(u) = &p.base_url {
        entry.insert("baseUrl".into(), json!(u));
    }
    if let Some(k) = &p.key_env {
        entry.insert("envKey".into(), json!(k));
    }
    o.op(
        &file,
        fmt,
        PatchOp::Append {
            path: keys(["modelProviders", "openai"]),
            value: Value::Object(entry),
            dedupe: Dedupe::Fields {
                fields: vec!["id".into()],
            },
        },
        "settings",
    );
    o.set(&file, fmt, keys(["model", "name"]), json!(model));
    Ok(())
}

// ------------------------------------------------------------------ Cursor CLI

/// Claude rule → Cursor CLI permission token.
pub fn cursor_token(d: Decision, r: &Rule) -> Option<String> {
    let spec = r.spec.as_deref();
    Some(match r.family() {
        Family::Shell => match spec {
            Some(s) => {
                let p = sp::shell_prefix(s);
                if p.is_empty() {
                    "Shell(*)".into()
                } else {
                    format!("Shell({p})")
                }
            }
            None => "Shell(*)".into(),
        },
        Family::Read => format!(
            "Read({})",
            spec.map(sp::path_glob).unwrap_or_else(|| "**".into())
        ),
        // listing and searching are reads for Cursor; denying them alone is not expressible
        Family::Search if d == Decision::Allow => "Read(**)".into(),
        Family::Edit => format!(
            "Write({})",
            spec.map(sp::path_glob).unwrap_or_else(|| "**".into())
        ),
        Family::WebFetch => match spec.and_then(sp::domain_of) {
            Some(dom) => format!("WebFetch({dom})"),
            None => "WebFetch(*)".into(),
        },
        Family::Mcp => match sp::mcp_parts(&r.tool) {
            Some((s, Some(t))) => format!("Mcp({s}:{t})"),
            Some((s, None)) => format!("Mcp({s}:*)"),
            None => "Mcp(*:*)".into(),
        },
        _ => return None,
    })
}

fn cursor(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = DocFormat::Json;
    if o.scope == Scope::Global {
        o.set(&file, fmt, keys(["version"]), json!(1));
    }
    for (d, r) in &perms.rules {
        if *d == Decision::Ask {
            o.loss(format!(
                "Cursor CLI has no ask list: `{}` falls back to its approval prompt",
                r.to_claude()
            ));
            continue;
        }
        match cursor_token(*d, r) {
            Some(tok) => o.append(&file, fmt, keys(["permissions", d.as_str()]), json!(tok)),
            None => o.loss(format!(
                "Cursor CLI has no permission for `{}`",
                r.to_claude()
            )),
        }
    }
    if let Some(m) = perms.mode {
        let v = match m {
            Mode::Bypass => Some("unrestricted"),
            Mode::Auto => Some("auto-review"),
            Mode::Default | Mode::Plan | Mode::DontAsk => Some("allowlist"),
            Mode::AcceptEdits => None,
        };
        match (v, o.settings_at(Scope::Global)) {
            (Some(v), Some(g)) => o.set(&g, DocFormat::Json, keys(["approvalMode"]), json!(v)),
            _ => o.loss(format!(
                "permission mode `{}` has no Cursor CLI equivalent",
                m.as_claude()
            )),
        }
    }
    if !perms.additional_dirs.is_empty() {
        o.loss("Cursor CLI has no additionalDirectories");
    }
    Ok(())
}

// ------------------------------------------------------------------ Codex

/// Starlark exec-policy file for the shell rules.
pub fn codex_rules_file(perms: &PermissionSet, origin: &str) -> (String, Vec<String>) {
    let mut out = format!(
        "# Written by OmniGet from `{origin}`. OmniGet removes this file on uninstall.\n# Exec policy (experimental in Codex): https://developers.openai.com/codex/rules\n\n"
    );
    let mut lost = Vec::new();
    let mut n = 0;
    for (d, r) in &perms.rules {
        if r.family() != Family::Shell {
            continue;
        }
        let Some(spec) = &r.spec else { continue };
        let Some(tokens) = sp::shell_tokens(spec) else {
            lost.push(format!(
                "shell rule `{}` has a wildcard inside it",
                r.to_claude()
            ));
            continue;
        };
        let decision = match d {
            Decision::Allow => "allow",
            Decision::Ask => "prompt",
            Decision::Deny => "forbidden",
        };
        let pat: Vec<String> = tokens
            .iter()
            .map(|t| format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        out.push_str(&format!(
            "prefix_rule(pattern=[{}], decision=\"{decision}\", justification=\"{}\")\n",
            pat.join(", "),
            r.to_claude().replace('\\', "\\\\").replace('"', "\\\"")
        ));
        n += 1;
    }
    if n == 0 {
        return (String::new(), lost);
    }
    (out, lost)
}

fn codex(o: &mut Out, perms: &PermissionSet, provider: Option<&Provider>) -> Result<()> {
    let file = o.settings()?;
    let fmt = DocFormat::Toml;
    if !perms.is_empty() {
        let shell = perms.bare(Family::Shell);
        let (sandbox, approval) = match perms.mode {
            Some(Mode::Plan) => ("read-only", "on-request"),
            Some(Mode::Bypass) => ("danger-full-access", "never"),
            _ if perms.writes_denied() => ("read-only", "on-request"),
            Some(Mode::AcceptEdits | Mode::Auto | Mode::DontAsk)
                if shell == Some(Decision::Allow) =>
            {
                ("workspace-write", "never")
            }
            _ => ("workspace-write", "on-request"),
        };
        o.set(&file, fmt, keys(["sandbox_mode"]), json!(sandbox));
        o.set(&file, fmt, keys(["approval_policy"]), json!(approval));
        if perms.shell_denied() {
            o.loss("Codex cannot turn its shell off: the sandbox blocks writes and the approval policy asks instead");
        }
        match perms.bare(Family::WebFetch) {
            Some(Decision::Deny) if sandbox == "workspace-write" => o.set(
                &file,
                fmt,
                keys(["sandbox_workspace_write", "network_access"]),
                json!(false),
            ),
            Some(Decision::Allow) if sandbox == "workspace-write" => o.set(
                &file,
                fmt,
                keys(["sandbox_workspace_write", "network_access"]),
                json!(true),
            ),
            _ => {}
        }
        if perms.bare(Family::WebSearch) == Some(Decision::Deny) {
            o.set(&file, fmt, keys(["web_search"]), json!("disabled"));
        }
        if !perms.additional_dirs.is_empty() && sandbox == "workspace-write" {
            for d in &perms.additional_dirs {
                o.append(
                    &file,
                    fmt,
                    keys(["sandbox_workspace_write", "writable_roots"]),
                    json!(d),
                );
            }
        }
        let (rules, lost) = codex_rules_file(perms, &o.c.id);
        for l in lost {
            o.loss(l);
        }
        if !rules.is_empty() {
            let dir = file
                .parent()
                .map(|p| p.join("rules"))
                .ok_or_else(|| o.missing("rules"))?;
            let name = component_slug(o.c, o.ctx);
            o.write(
                dir.join(format!("omniget-{name}.rules")),
                rules,
                "exec policy",
            );
            o.note("shell rules go to a Codex exec-policy file (experimental in Codex)");
        }
        for (_, r) in &perms.rules {
            match r.family() {
                Family::Shell | Family::Edit | Family::WebFetch | Family::WebSearch
                    if r.spec.is_none() => {}
                Family::Shell => {}
                Family::Read | Family::Edit if r.spec.is_some() => o.loss(format!(
                    "Codex has no per-path rule: `{}` (the sandbox is per workspace)",
                    r.to_claude()
                )),
                Family::Read | Family::Search => {}
                _ => o.loss(format!("Codex has no rule for `{}`", r.to_claude())),
            }
        }
    }
    if let Some(p) = provider {
        match (p.kind, p.protocol) {
            (ProviderKind::Custom, Protocol::OpenAi) => {
                let id = sp::provider_id(p);
                let mut prov = Map::new();
                prov.insert("name".into(), json!(id));
                if let Some(u) = &p.base_url {
                    prov.insert("base_url".into(), json!(u));
                }
                if let Some(k) = &p.key_env {
                    prov.insert("env_key".into(), json!(k));
                }
                o.set(
                    &file,
                    fmt,
                    keys(["model_providers", id.as_str()]),
                    Value::Object(prov),
                );
                o.set(&file, fmt, keys(["model_provider"]), json!(id));
                if let Some(m) = &p.model {
                    o.set(&file, fmt, keys(["model"]), json!(m));
                }
            }
            _ => o.loss(provider_loss(o.t, p)),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ OpenCode / Kilo

/// Claude rule → OpenCode `permission` key and pattern.
pub fn opencode_key(r: &Rule) -> Option<(String, Option<String>)> {
    let spec = r.spec.clone();
    let key = match r.tool.as_str() {
        "Glob" | "FindFiles" => "glob",
        "Grep" | "SearchFiles" => "grep",
        "LS" | "ListFiles" => "list",
        _ => match r.family() {
            Family::Shell => {
                let pat = spec.map(|s| {
                    let s = s.trim().to_string();
                    match s.strip_suffix(":*") {
                        Some(x) => format!("{x} *"),
                        None => s,
                    }
                });
                return Some(("bash".into(), pat));
            }
            Family::Read => return Some(("read".into(), spec.map(|s| sp::path_glob(&s)))),
            Family::Edit => return Some(("edit".into(), spec.map(|s| sp::path_glob(&s)))),
            Family::WebFetch => {
                return Some((
                    "webfetch".into(),
                    spec.map(|s| match sp::domain_of(&s) {
                        Some(d) => format!("https://{d}/*"),
                        None => s,
                    }),
                ))
            }
            Family::WebSearch => "websearch",
            Family::Agent => return Some(("task".into(), spec)),
            Family::Skill => return Some(("skill".into(), spec)),
            Family::Todo => "todowrite",
            Family::Mcp => {
                return match sp::mcp_parts(&r.tool) {
                    Some((s, Some(t))) => Some((format!("{s}_{t}"), None)),
                    Some((s, None)) => Some((format!("{s}_*"), None)),
                    None => None,
                }
            }
            Family::All => "*",
            _ => return None,
        },
    };
    Some((key.to_string(), spec))
}

fn opencode(o: &mut Out, perms: &PermissionSet, provider: Option<&Provider>) -> Result<()> {
    let file = o.settings()?;
    let fmt = if file.extension().and_then(|e| e.to_str()) == Some("json")
        && !file.to_string_lossy().ends_with("kilo.json")
        && !file.to_string_lossy().ends_with("opencode.json")
    {
        DocFormat::Json
    } else {
        DocFormat::Jsonc
    };
    let base =
        o.t.permissions
            .as_ref()
            .filter(|p| !p.key.is_empty())
            .map(|p| p.key.clone())
            .unwrap_or_else(|| vec!["permission".into()]);
    // key → (bare decision, [(pattern, decision)]) ; Claude: deny > ask > allow,
    // OpenCode: last matching rule wins, so patterns go allow → ask → deny.
    let mut map: BTreeMap<String, (Option<Decision>, Vec<(String, Decision)>)> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for (d, r) in &perms.rules {
        let Some((k, pat)) = opencode_key(r) else {
            o.loss(format!(
                "{} has no permission for `{}`",
                o.t.name,
                r.to_claude()
            ));
            continue;
        };
        if !order.contains(&k) {
            order.push(k.clone());
        }
        let e = map.entry(k).or_default();
        match pat {
            None => e.0 = Some(e.0.map_or(*d, |x| x.max(*d))),
            Some(p) => {
                if !e.1.iter().any(|(q, x)| *q == p && *x == *d) {
                    e.1.push((p, *d));
                }
            }
        }
    }
    let path = |extra: &[&str]| -> Vec<Seg> {
        let mut v: Vec<Seg> = base.iter().map(|k| Seg::Key(k.clone())).collect();
        v.extend(extra.iter().map(|k| Seg::Key(k.to_string())));
        v
    };
    // mode first so explicit rules can refine it
    match perms.mode {
        Some(Mode::Bypass) if !map.contains_key("*") => {
            o.set(&file, fmt, path(&["*"]), json!("allow"))
        }
        Some(Mode::AcceptEdits) if !map.contains_key("edit") => {
            o.set(&file, fmt, path(&["edit"]), json!("allow"))
        }
        Some(Mode::Plan) => o.set(&file, fmt, keys(["default_agent"]), json!("plan")),
        Some(Mode::Auto | Mode::DontAsk) => o.loss(format!(
            "permission mode `{}` has no {} equivalent",
            perms.mode.map(|m| m.as_claude()).unwrap_or(""),
            o.t.name
        )),
        _ => {}
    }
    for k in order {
        let (bare, mut pats) = map.remove(&k).unwrap_or_default();
        if pats.is_empty() {
            if let Some(d) = bare {
                o.set(&file, fmt, path(&[&k]), json!(d.as_str()));
            }
            continue;
        }
        if let Some(d) = bare {
            o.set(&file, fmt, path(&[&k, "*"]), json!(d.as_str()));
        }
        pats.sort_by_key(|(_, d)| *d);
        for (p, d) in pats {
            o.set(&file, fmt, path(&[&k, &p]), json!(d.as_str()));
        }
    }
    for d in &perms.additional_dirs {
        let glob = format!("{}/**", d.trim_end_matches('/'));
        o.set(
            &file,
            fmt,
            path(&["external_directory", &glob]),
            json!("allow"),
        );
    }
    if let Some(p) = provider {
        match (p.kind, p.protocol) {
            (ProviderKind::Custom, Protocol::Anthropic) => {
                if let Some(u) = &p.base_url {
                    o.set(
                        &file,
                        fmt,
                        keys(["provider", "anthropic", "options", "baseURL"]),
                        json!(u),
                    );
                }
                if let Some(k) = &p.key_env {
                    o.set(
                        &file,
                        fmt,
                        keys(["provider", "anthropic", "options", "apiKey"]),
                        json!(format!("{{env:{k}}}")),
                    );
                }
                if let Some(m) = &p.model {
                    o.set(&file, fmt, keys(["model"]), json!(format!("anthropic/{m}")));
                }
            }
            (ProviderKind::Custom, Protocol::OpenAi) => {
                let id = sp::provider_id(p);
                let mut prov = json!({
                    "npm": "@ai-sdk/openai-compatible",
                    "name": id,
                    "options": {}
                });
                if let Some(u) = &p.base_url {
                    prov["options"]["baseURL"] = json!(u);
                }
                if let Some(k) = &p.key_env {
                    prov["options"]["apiKey"] = json!(format!("{{env:{k}}}"));
                }
                if let Some(m) = &p.model {
                    prov["models"] = json!({ m.as_str(): {} });
                    o.set(&file, fmt, keys(["model"]), json!(format!("{id}/{m}")));
                }
                o.set(&file, fmt, keys(["provider", id.as_str()]), prov);
            }
            (ProviderKind::Bedrock, _) | (ProviderKind::Vertex, _) => {
                o.note(format!(
                    "{} reads cloud credentials from the environment: pick a `{}/…` model in it",
                    o.t.name,
                    sp::provider_id(p)
                ));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ Gemini

fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Gemini tool name(s) for a rule family.
fn gemini_tools(t: &TargetAdapter, r: &Rule) -> Vec<String> {
    let m = |k: &str, dflt: &str| {
        t.tool_name_map
            .get(k)
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| dflt.to_string())
    };
    match r.tool.as_str() {
        "Glob" => return vec![m("Glob", "glob")],
        "Grep" => return vec![m("Grep", "grep_search")],
        "LS" => return vec!["list_directory".into()],
        "Write" => return vec![m("Write", "write_file")],
        "Edit" | "MultiEdit" => return vec![m("Edit", "replace")],
        _ => {}
    }
    match r.family() {
        Family::Shell => vec![m("Bash", "run_shell_command")],
        Family::Read => vec![m("Read", "read_file"), "read_many_files".into()],
        Family::Edit => vec![m("Edit", "replace"), m("Write", "write_file")],
        Family::WebFetch => vec![m("WebFetch", "web_fetch")],
        Family::WebSearch => vec![m("WebSearch", "google_web_search")],
        Family::Todo => vec![m("TodoWrite", "write_todos")],
        Family::Skill => vec![m("Skill", "activate_skill")],
        Family::All => vec!["*".into()],
        _ => vec![],
    }
}

/// Policy Engine file (`~/.gemini/policies/*.toml`) for a permission set.
pub fn gemini_policy(
    t: &TargetAdapter,
    perms: &PermissionSet,
    origin: &str,
) -> (String, Vec<String>) {
    let mut out = format!(
        "# Written by OmniGet from `{origin}`. OmniGet removes this file on uninstall.\n\n"
    );
    let mut lost = Vec::new();
    let mut n = 0;
    for (d, r) in &perms.rules {
        let decision = match d {
            Decision::Allow => "allow",
            Decision::Ask => "ask_user",
            Decision::Deny => "deny",
        };
        let mut priority = match d {
            Decision::Allow => 300,
            Decision::Ask => 400,
            Decision::Deny => 500,
        };
        let mut lines: Vec<String> = Vec::new();
        if r.family() == Family::Mcp {
            match sp::mcp_parts(&r.tool) {
                Some((s, tool)) => {
                    lines.push(format!("mcpName = {}", toml_str(&s)));
                    if let Some(tool) = tool {
                        lines.push(format!("toolName = {}", toml_str(&tool)));
                    }
                }
                None => lines.push("toolName = \"mcp_*\"".into()),
            }
        } else {
            let tools = gemini_tools(t, r);
            if tools.is_empty() {
                lost.push(format!("Gemini has no policy for `{}`", r.to_claude()));
                continue;
            }
            if tools.len() == 1 {
                lines.push(format!("toolName = {}", toml_str(&tools[0])));
            } else {
                let arr: Vec<String> = tools.iter().map(|x| toml_str(x)).collect();
                lines.push(format!("toolName = [{}]", arr.join(", ")));
            }
        }
        if let Some(spec) = &r.spec {
            priority += 10;
            match r.family() {
                Family::Shell => {
                    let p = sp::shell_prefix(spec);
                    if p.contains('*') || p.contains('?') {
                        let re = format!("^{}", sp::glob_to_regex(spec).replace("[^/\"]*", ".*"));
                        lines.push(format!("commandRegex = {}", toml_str(&re)));
                    } else {
                        lines.push(format!("commandPrefix = {}", toml_str(&p)));
                    }
                }
                Family::Read | Family::Edit => {
                    let re = format!(
                        "\"(file_path|absolute_path|path|dir_path)\":\"([^\"]*/)?{}\"",
                        sp::glob_to_regex(&sp::path_glob(spec))
                    );
                    lines.push(format!("argsPattern = {}", toml_str(&re)));
                }
                Family::WebFetch => {
                    let dom = sp::domain_of(spec).unwrap_or(spec);
                    let re = format!("https?://([^/\"]*\\.)?{}", sp::glob_to_regex(dom));
                    lines.push(format!("argsPattern = {}", toml_str(&re)));
                }
                Family::Agent => lines.push(format!("subagent = {}", toml_str(spec))),
                _ => {
                    lost.push(format!(
                        "Gemini policy ignores the pattern of `{}`",
                        r.to_claude()
                    ));
                }
            }
        }
        lines.push(format!("decision = \"{decision}\""));
        lines.push(format!("priority = {priority}"));
        let block = format!("[[rule]]\n{}\n\n", lines.join("\n"));
        if out.contains(&block) {
            continue;
        }
        out.push_str(&block);
        n += 1;
    }
    if n == 0 {
        return (String::new(), lost);
    }
    (out, lost)
}

fn gemini(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = json_fmt(&file);
    if matches!(o.scope, Scope::Global | Scope::Managed) {
        let (policy, lost) = gemini_policy(o.t, perms, &o.c.id);
        for l in lost {
            o.loss(l);
        }
        if !policy.is_empty() {
            let dir = file
                .parent()
                .map(|p| p.join("policies"))
                .ok_or_else(|| o.missing("policies"))?;
            let name = component_slug(o.c, o.ctx);
            o.write(dir.join(format!("omniget-{name}.toml")), policy, "policy");
        }
    } else {
        // workspace policies are disabled in Gemini: use the settings lists
        for (d, r) in &perms.rules {
            let tools = gemini_tools(o.t, r);
            if tools.is_empty() {
                o.loss(format!("Gemini has no tool for `{}`", r.to_claude()));
                continue;
            }
            match (d, &r.spec) {
                (Decision::Deny, None) => {
                    for t in tools {
                        o.append(&file, fmt, keys(["tools", "exclude"]), json!(t));
                    }
                }
                (Decision::Allow, None) => {
                    for t in tools {
                        o.append(&file, fmt, keys(["tools", "allowed"]), json!(t));
                    }
                }
                (Decision::Allow, Some(s)) if r.family() == Family::Shell => {
                    o.append(
                        &file,
                        fmt,
                        keys(["tools", "allowed"]),
                        json!(format!("{}({})", tools[0], sp::shell_prefix(s))),
                    );
                }
                _ => o.loss(format!(
                    "Gemini reads fine-grained rules only from user policies: `{}` needs global scope",
                    r.to_claude()
                )),
            }
        }
    }
    if let Some(m) = perms.mode {
        let v = match m {
            Mode::Default => Some("default"),
            Mode::AcceptEdits => Some("auto_edit"),
            Mode::Plan => Some("plan"),
            Mode::Bypass => Some("yolo"),
            _ => None,
        };
        match v {
            Some(v) => o.set(
                &file,
                fmt,
                keys(["general", "defaultApprovalMode"]),
                json!(v),
            ),
            None => o.loss(format!(
                "permission mode `{}` has no Gemini equivalent",
                m.as_claude()
            )),
        }
    }
    if !perms.additional_dirs.is_empty() {
        for d in &perms.additional_dirs {
            o.append(
                &file,
                fmt,
                keys(["context", "includeDirectories"]),
                json!(d),
            );
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ Devin CLI

pub fn devin_token(r: &Rule) -> Option<String> {
    let spec = r.spec.as_deref();
    Some(match (r.tool.as_str(), r.family()) {
        ("Glob", _) => "glob".into(),
        ("Grep", _) => "grep".into(),
        (_, Family::Shell) => match spec {
            Some(s) => format!("Exec({})", sp::shell_prefix(s)),
            None => "exec".into(),
        },
        (_, Family::Read) => match spec {
            Some(s) => format!("Read({})", sp::path_glob(s)),
            None => "read".into(),
        },
        (_, Family::Search) => "read".into(),
        (_, Family::Edit) => match spec {
            Some(s) => format!("Write({})", sp::path_glob(s)),
            None => "edit".into(),
        },
        (_, Family::WebFetch) => match spec {
            Some(s) => match sp::domain_of(s) {
                Some(d) => format!("Fetch(domain:{d})"),
                None => format!("Fetch({s})"),
            },
            None => "Fetch(*)".into(),
        },
        (_, Family::Mcp) => match sp::mcp_parts(&r.tool) {
            Some((s, Some(t))) => format!("mcp__{s}__{t}"),
            Some((s, None)) => format!("mcp__{s}__*"),
            None => "mcp__*".into(),
        },
        _ => return None,
    })
}

fn devin(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = DocFormat::Jsonc;
    for (d, r) in &perms.rules {
        match devin_token(r) {
            Some(tok) => o.append(&file, fmt, keys(["permissions", d.as_str()]), json!(tok)),
            None => o.loss(format!("Devin has no permission for `{}`", r.to_claude())),
        }
    }
    if let Some(m) = perms.mode {
        o.loss(format!(
            "Devin sets the permission mode per session, not `{}` in the file",
            m.as_claude()
        ));
    }
    if !perms.additional_dirs.is_empty() {
        o.loss("Devin has no additionalDirectories");
    }
    Ok(())
}

// ------------------------------------------------------------------ Kiro

pub fn kiro_rule(d: Decision, r: &Rule) -> Option<Value> {
    let spec = r.spec.as_deref();
    let (cap, pat): (&str, String) = match r.family() {
        Family::Shell => (
            "shell",
            spec.map(|s| s.trim().replace(":*", " *"))
                .unwrap_or_else(|| "*".into()),
        ),
        Family::Read | Family::Search => (
            "fs_read",
            spec.map(sp::path_glob).unwrap_or_else(|| "*".into()),
        ),
        Family::Edit => (
            "fs_write",
            spec.map(sp::path_glob).unwrap_or_else(|| "*".into()),
        ),
        Family::WebFetch => (
            "web_fetch",
            spec.map(|s| match sp::domain_of(s) {
                Some(dom) => format!("*{dom}*"),
                None => s.to_string(),
            })
            .unwrap_or_else(|| "*".into()),
        ),
        Family::WebSearch => (
            "web_search",
            spec.map(str::to_string).unwrap_or_else(|| "*".into()),
        ),
        Family::Mcp => (
            "mcp",
            match sp::mcp_parts(&r.tool) {
                Some((s, Some(t))) => format!("{s}/{t}"),
                Some((s, None)) => format!("{s}/*"),
                None => "*".into(),
            },
        ),
        Family::Agent => (
            "subagent",
            spec.map(str::to_string).unwrap_or_else(|| "*".into()),
        ),
        Family::Skill => (
            "skill",
            spec.map(str::to_string).unwrap_or_else(|| "*".into()),
        ),
        Family::All => ("all", "*".into()),
        _ => return None,
    };
    Some(json!({ "capability": cap, "match": [pat], "effect": d.as_str() }))
}

fn kiro(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    // workspace permissions live outside the repo, keyed by a hash of its root
    let file = kind_path(o.t, "permissions", Scope::Global, o.ctx)
        .ok_or_else(|| o.missing("permissions"))?;
    if o.scope != Scope::Global {
        o.note("Kiro keeps workspace permissions outside the repo; written to the user permissions.yaml");
    }
    for (d, r) in &perms.rules {
        match kiro_rule(*d, r) {
            Some(v) => o.op(
                &file,
                DocFormat::Yaml,
                PatchOp::Append {
                    path: keys(["rules"]),
                    value: v,
                    dedupe: Dedupe::Equal,
                },
                "permissions",
            ),
            None => o.loss(format!("Kiro has no capability for `{}`", r.to_claude())),
        }
    }
    if let Some(m) = perms.mode {
        o.loss(format!(
            "Kiro has no default permission mode setting (`{}`)",
            m.as_claude()
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ Copilot CLI

/// `--allow-tool`/`--deny-tool` flags equivalent to the rules Copilot cannot keep in a file.
pub fn copilot_flags(perms: &PermissionSet) -> Vec<String> {
    let mut out = Vec::new();
    for (d, r) in &perms.rules {
        let flag = match d {
            Decision::Allow => "--allow-tool",
            Decision::Deny => "--deny-tool",
            Decision::Ask => continue,
        };
        let tok = match r.family() {
            Family::Shell => match &r.spec {
                Some(s) => format!("shell({}:*)", sp::shell_prefix(s)),
                None => "shell".into(),
            },
            Family::Edit => "write".into(),
            Family::Mcp => match sp::mcp_parts(&r.tool) {
                Some((s, Some(t))) => format!("{s}({t})"),
                Some((s, None)) => s,
                None => continue,
            },
            _ => continue,
        };
        let f = format!("{flag}='{tok}'");
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

fn copilot(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = json_fmt(&file);
    let mut dropped = Vec::new();
    for (d, r) in &perms.rules {
        match (r.family(), r.spec.as_deref().and_then(sp::domain_of), d) {
            (Family::WebFetch, Some(dom), Decision::Allow) => {
                o.append(&file, fmt, keys(["allowedUrls"]), json!(dom))
            }
            (Family::WebFetch, Some(dom), Decision::Deny) => {
                o.append(&file, fmt, keys(["deniedUrls"]), json!(dom))
            }
            _ => dropped.push(r.to_claude()),
        }
    }
    if !dropped.is_empty() {
        o.loss(format!(
            "Copilot CLI takes tool permissions as flags, not settings: {}",
            dropped.join(", ")
        ));
        let flags = copilot_flags(perms);
        if !flags.is_empty() {
            o.note(format!("run Copilot CLI with: copilot {}", flags.join(" ")));
        }
    }
    if let Some(m) = perms.mode {
        if m == Mode::Bypass {
            o.note("run Copilot CLI with --allow-all for the same effect");
        }
        o.loss(format!(
            "Copilot CLI has no permission mode setting (`{}`)",
            m.as_claude()
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ Droid

fn droid(o: &mut Out, perms: &PermissionSet) -> Result<()> {
    if perms.is_empty() {
        return Ok(());
    }
    let file = o.settings()?;
    let fmt = json_fmt(&file);
    for (d, r) in &perms.rules {
        match (r.family(), &r.spec, d) {
            (Family::Shell, Some(s), Decision::Allow) => o.append(
                &file,
                fmt,
                keys(["commandAllowlist"]),
                json!(sp::shell_prefix(s)),
            ),
            (Family::Shell, Some(s), Decision::Deny) => o.append(
                &file,
                fmt,
                keys(["commandDenylist"]),
                json!(sp::shell_prefix(s)),
            ),
            (Family::Shell | Family::Edit | Family::Read | Family::Search, None, _) => {}
            _ => o.loss(format!(
                "Droid has no per-rule setting for `{}`",
                r.to_claude()
            )),
        }
    }
    let shell = perms.bare(Family::Shell);
    let level = match perms.mode {
        Some(Mode::Plan) => Some("off"),
        _ if perms.writes_denied() => Some("off"),
        Some(Mode::Bypass) => Some("high"),
        Some(Mode::AcceptEdits | Mode::Auto | Mode::DontAsk) if shell == Some(Decision::Allow) => {
            Some("medium")
        }
        Some(Mode::AcceptEdits) => Some("low"),
        _ if shell == Some(Decision::Ask) => Some("low"),
        _ => None,
    };
    if let Some(l) = level {
        o.set(
            &file,
            fmt,
            keys(["sessionDefaultSettings", "autonomyLevel"]),
            json!(l),
        );
    }
    if perms.shell_denied() {
        o.loss("Droid cannot turn its shell off; autonomy `off` asks before every command");
    }
    Ok(())
}

// ------------------------------------------------------------------ Goose

fn goose(o: &mut Out, perms: &PermissionSet, provider: Option<&Provider>) -> Result<()> {
    // Goose keeps its config and permissions per user only
    let cfg = o
        .settings_at(Scope::Global)
        .ok_or_else(|| o.missing("config.yaml"))?;
    if o.scope != Scope::Global {
        o.note("Goose has user settings only; written to its config.yaml / permission.yaml");
    }
    if !perms.is_empty() {
        let file = kind_path(o.t, "permissions", Scope::Global, o.ctx)
            .ok_or_else(|| o.missing("permissions"))?;
        let name = |r: &Rule| -> Option<String> {
            if let Some((s, t)) = sp::mcp_parts(&r.tool) {
                return Some(match t {
                    Some(t) => format!("{s}__{t}"),
                    None => format!("{s}__*"),
                });
            }
            let key = match r.tool.as_str() {
                "MultiEdit" | "NotebookEdit" => "Edit",
                "Task" => "Agent",
                t => t,
            };
            o.t.tool_name_map
                .get(key)
                .filter(|v| !v.is_empty())
                .cloned()
        };
        for (d, r) in &perms.rules {
            if r.spec.is_some() {
                o.loss(format!(
                    "Goose permissions are per tool, not per pattern: `{}`",
                    r.to_claude()
                ));
                continue;
            }
            let list = match d {
                Decision::Allow => "always_allow",
                Decision::Ask => "ask_before",
                Decision::Deny => "never_allow",
            };
            let mut names: Vec<String> = name(r).into_iter().collect();
            if r.tool == "Edit" || r.tool == "Write" {
                // both edit tools share the family on Goose
                for k in ["Edit", "Write"] {
                    if let Some(n) = o.t.tool_name_map.get(k).filter(|v| !v.is_empty()) {
                        if !names.contains(n) {
                            names.push(n.clone());
                        }
                    }
                }
            }
            if names.is_empty() {
                o.loss(format!("Goose has no tool for `{}`", r.to_claude()));
                continue;
            }
            for n in names {
                o.op(
                    &file,
                    DocFormat::Yaml,
                    PatchOp::Append {
                        path: keys(["user", list]),
                        value: json!(n),
                        dedupe: Dedupe::Equal,
                    },
                    "permissions",
                );
            }
        }
        let mode = match perms.mode {
            Some(Mode::Bypass) => Some("auto"),
            Some(Mode::Plan) => Some("approve"),
            _ if perms.writes_denied() => Some("approve"),
            Some(Mode::AcceptEdits | Mode::Auto) => Some("smart_approve"),
            Some(Mode::Default) => Some("approve"),
            _ => None,
        };
        if let Some(m) = mode {
            o.set(&cfg, DocFormat::Yaml, keys(["GOOSE_MODE"]), json!(m));
        }
    }
    if let Some(p) = provider {
        match (p.kind, p.protocol) {
            (ProviderKind::Custom, proto) => {
                let (prov, host) = match proto {
                    Protocol::Anthropic => ("anthropic", "ANTHROPIC_HOST"),
                    Protocol::OpenAi => ("openai", "OPENAI_HOST"),
                };
                o.set(&cfg, DocFormat::Yaml, keys(["GOOSE_PROVIDER"]), json!(prov));
                if let Some(u) = &p.base_url {
                    o.set(&cfg, DocFormat::Yaml, keys([host]), json!(u));
                }
                if let Some(m) = &p.model {
                    o.set(&cfg, DocFormat::Yaml, keys(["GOOSE_MODEL"]), json!(m));
                }
                let std_key = match proto {
                    Protocol::Anthropic => "ANTHROPIC_API_KEY",
                    Protocol::OpenAi => "OPENAI_API_KEY",
                };
                if p.key_env.as_deref() != Some(std_key) {
                    o.note(format!(
                        "Goose reads the key from {std_key} (keyring or environment){}",
                        p.key_env
                            .as_deref()
                            .map(|k| format!(": export {std_key}=\"${k}\""))
                            .unwrap_or_default()
                    ));
                }
            }
            _ => o.loss(provider_loss(o.t, p)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: Value) -> PermissionSet {
        PermissionSet::from_values(v.as_object().unwrap())
    }

    #[test]
    fn token_maps() {
        let r = |s: &str| Rule::parse(s).unwrap();
        assert_eq!(
            cursor_token(Decision::Allow, &r("Bash(git *)")).unwrap(),
            "Shell(git)"
        );
        assert_eq!(
            cursor_token(Decision::Deny, &r("Read(./.env)")).unwrap(),
            "Read(.env)"
        );
        assert_eq!(
            cursor_token(Decision::Deny, &r("Edit")).unwrap(),
            "Write(**)"
        );
        assert_eq!(
            cursor_token(Decision::Allow, &r("mcp__github__get_issue")).unwrap(),
            "Mcp(github:get_issue)"
        );
        assert_eq!(
            opencode_key(&r("Bash(npm run test:*)")).unwrap(),
            ("bash".into(), Some("npm run test *".into()))
        );
        assert_eq!(
            opencode_key(&r("WebFetch(domain:x.dev)"))
                .unwrap()
                .1
                .unwrap(),
            "https://x.dev/*"
        );
        assert_eq!(
            devin_token(&r("Bash(git push *)")).unwrap(),
            "Exec(git push)"
        );
        assert_eq!(
            kiro_rule(Decision::Deny, &r("Write")).unwrap(),
            json!({"capability":"fs_write","match":["*"],"effect":"deny"})
        );
    }

    #[test]
    fn codex_rules_and_gemini_policy() {
        let p = set(
            json!({"permissions": {"allow": ["Bash(npm test)"], "deny": ["Bash(rm -rf *)", "Edit"], "ask": ["Bash(git push *)"]}}),
        );
        let (rules, lost) = codex_rules_file(&p, "x");
        assert!(lost.is_empty());
        assert!(rules.contains("prefix_rule(pattern=[\"rm\", \"-rf\"], decision=\"forbidden\""));
        assert!(rules.contains("pattern=[\"git\", \"push\"], decision=\"prompt\""));
        let gem = crate::core::agentkit::targets::target("gemini").unwrap();
        let (pol, _) = gemini_policy(gem, &p, "x");
        assert!(pol.contains(
            "toolName = \"run_shell_command\"\ncommandPrefix = \"rm -rf\"\ndecision = \"deny\""
        ));
        assert!(pol.contains("toolName = \"replace\"\ndecision = \"deny\""));
        assert!(pol.contains("decision = \"ask_user\""));
        let flags = copilot_flags(&p);
        assert!(flags.contains(&"--deny-tool='shell(rm -rf:*)'".to_string()));
    }
}
