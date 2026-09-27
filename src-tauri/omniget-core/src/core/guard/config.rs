//! Config validator for JSON components: hooks, MCP servers, settings and
//! status lines. Every command that will be executed is parsed and analysed,
//! and recorded in the report so the UI can show it before anything is
//! written.

use serde_json::Value;
use std::collections::BTreeMap;

use super::model::{ExecCommand, Finding, Severity, Sink, Validator};
use super::rules::sev;
use super::shell::{self, Analysis};
use super::text::{is_placeholder, is_secret_name, looks_real_secret, redact, LineIndex};
use super::tools;
use super::Ctx;

fn cfg(code: &str, file: &str, detail: impl Into<String>) -> Finding {
    Finding::new(code, Validator::Config, sev(code), file, detail)
}

/// Line of a JSON string value in the raw text.
pub fn line_of_value(raw: &str, idx: &LineIndex, value: &str) -> Option<u32> {
    let enc = serde_json::to_string(value).ok()?;
    let off = raw.find(&enc).or_else(|| raw.find(value))?;
    Some(idx.pos(raw, off).0)
}

/// Where the command came from, for [`record_command`].
pub struct CmdSite<'a> {
    pub file: &'a str,
    pub origin: &'a str,
    pub json_path: Option<String>,
    pub line: Option<u32>,
    pub context: Option<String>,
    pub auto_runs: bool,
    /// Lower every hit by this many steps (shell fences in Markdown).
    pub drop: u8,
}

/// Analyse one command line and record findings + the command itself.
pub fn record_command(site: CmdSite<'_>, command: &str, sink: &mut Sink) -> Analysis {
    let a = shell::analyze(command);
    record_analysis(site, command, a, sink)
}

pub fn record_analysis(site: CmdSite<'_>, command: &str, a: Analysis, sink: &mut Sink) -> Analysis {
    let mut codes = Vec::new();
    let mut worst: Option<Severity> = None;
    for h in &a.hits {
        // MCP launchers: the unpinned-package note is MCP_W004's job.
        if site.origin == "mcp" && h.code == "CMD_W009" {
            continue;
        }
        let mut f = Finding::new(
            h.code,
            Validator::Command,
            h.severity,
            site.file,
            h.detail.clone(),
        )
        .line(site.line)
        .snippet(command)
        .lowered(site.drop);
        f.json_path = site.json_path.clone();
        worst = worst.max(Some(f.severity));
        if !codes.contains(&h.code.to_string()) {
            codes.push(h.code.to_string());
        }
        sink.push(f);
    }
    if !a.fake_env.is_empty() && matches!(site.origin, "hook" | "statusline" | "script") {
        let mut f = cfg(
            "HOOK_W001",
            site.file,
            format!("${} never set", a.fake_env.join(", $")),
        )
        .line(site.line)
        .snippet(command);
        f.json_path = site.json_path.clone();
        worst = worst.max(Some(f.severity));
        codes.push("HOOK_W001".into());
        sink.push(f);
    }
    if site.auto_runs && matches!(site.origin, "inline") {
        let mut f = Finding::new(
            "CMD_I001",
            Validator::Command,
            sev("CMD_I001"),
            site.file,
            "runs when the command is invoked",
        )
        .line(site.line)
        .snippet(command);
        f.json_path = site.json_path.clone();
        sink.push(f);
    }
    if site.origin != "fence" || !codes.is_empty() {
        sink.commands.push(ExecCommand {
            file: site.file.to_string(),
            origin: site.origin.to_string(),
            json_path: site.json_path,
            line: site.line,
            context: site.context,
            command: command.to_string(),
            programs: a.programs.clone(),
            codes,
            worst,
            network: a.network,
            auto_runs: site.auto_runs,
        });
    }
    a
}

/// MCP servers in any of the shapes tools use: `mcpServers` (Claude, Cursor,
/// Gemini, Qwen, Cline…), `servers` (VS Code/Copilot), `mcp` (OpenCode),
/// `context_servers` (Zed). Returns `(name, server, json path)`.
pub fn servers_of(v: &Value) -> Vec<(String, &Value, String)> {
    let mut out = Vec::new();
    for key in [
        "mcpServers",
        "servers",
        "mcp",
        "context_servers",
        "mcp_servers",
    ] {
        if let Some(obj) = v.get(key).and_then(|x| x.as_object()) {
            for (name, s) in obj {
                if s.is_object() {
                    out.push((name.clone(), s, format!("{key}.{name}")));
                }
            }
        }
    }
    out
}

/// Entry point for one JSON file.
pub fn check_json(
    ctx: &Ctx,
    file: &str,
    raw: &str,
    v: &Value,
    files: &BTreeMap<String, Vec<u8>>,
    sink: &mut Sink,
) {
    let idx = LineIndex::new(raw);
    let tool = ctx.origin.clone().unwrap_or_else(|| ctx.detected.clone());
    if let Some(h) = v.get("hooks") {
        check_hooks(&tool, file, raw, &idx, h, "hooks", sink);
    }
    for (name, s, path) in servers_of(v) {
        check_mcp(file, raw, &idx, &name, s, &path, sink);
    }
    if let Some(sl) = v.get("statusLine") {
        if let Some(cmd) = sl.get("command").and_then(|c| c.as_str()) {
            let line = line_of_value(raw, &idx, cmd);
            sink.push(
                cfg("SET_W008", file, super::text::clip(cmd, 120))
                    .line(line)
                    .path("statusLine.command"),
            );
            record_command(
                CmdSite {
                    file,
                    origin: "statusline",
                    json_path: Some("statusLine.command".into()),
                    line,
                    context: None,
                    auto_runs: true,
                    drop: 0,
                },
                cmd,
                sink,
            );
        }
    }
    check_settings(file, raw, &idx, v, sink);
    // Supporting files (hooks) and inline files (settings).
    if let Some(arr) = v.get("supportingFiles").and_then(|x| x.as_array()) {
        for (i, sfile) in arr.iter().enumerate() {
            let src = sfile.get("source").and_then(|x| x.as_str()).unwrap_or("");
            let dst = sfile
                .get("destination")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let path = format!("supportingFiles[{i}]");
            check_destination(file, dst, &path, sink);
            if sfile.get("executable").and_then(|x| x.as_bool()) == Some(true) {
                sink.push(cfg("HOOK_I001", file, format!("{src} → {dst}")).path(path.clone()));
            }
            if !src.is_empty()
                && !files
                    .keys()
                    .any(|k| k == src || k.ends_with(&format!("/{src}")))
            {
                sink.push(cfg("HOOK_W006", file, src.to_string()).path(path));
            }
        }
    }
    if let Some(obj) = v.get("files").and_then(|x| x.as_object()) {
        for (dst, spec) in obj {
            let path = format!("files.{dst}");
            check_destination(file, dst, &path, sink);
            if let Some(content) = spec.get("content").and_then(|c| c.as_str()) {
                let a = if dst.ends_with(".sh") || !dst.contains('.') {
                    shell::analyze(content)
                } else {
                    shell::analyze_code(content)
                };
                record_analysis(
                    CmdSite {
                        file,
                        origin: "script",
                        json_path: Some(path),
                        line: None,
                        context: Some(dst.clone()),
                        auto_runs: true,
                        drop: 0,
                    },
                    &super::text::clip(content, 400),
                    a,
                    sink,
                );
            }
        }
    }
}

fn check_destination(file: &str, dst: &str, path: &str, sink: &mut Sink) {
    let d = dst.replace('\\', "/");
    let bad = d.starts_with('/')
        || d.starts_with('~')
        || d.split('/').any(|s| s == "..")
        || d.chars().nth(1) == Some(':')
        || d.starts_with(".git/")
        || d.contains("/.git/hooks")
        || d.starts_with("$HOME")
        || [".bashrc", ".zshrc", ".profile", "authorized_keys"]
            .iter()
            .any(|s| d.ends_with(s));
    if bad {
        sink.push(cfg("HOOK_W005", file, dst.to_string()).path(path.to_string()));
    }
}

fn check_hooks(
    tool: &str,
    file: &str,
    raw: &str,
    idx: &LineIndex,
    hooks: &Value,
    base: &str,
    sink: &mut Sink,
) {
    let Some(events) = hooks.as_object() else {
        if hooks.is_array() {
            sink.push(
                cfg("HOOK_W007", file, "`hooks` is an array (pre-2025 shape)")
                    .path(base.to_string()),
            );
            for (i, h) in hooks.as_array().unwrap().iter().enumerate() {
                if let Some(c) = h
                    .get("command")
                    .or_else(|| h.get("script"))
                    .and_then(|c| c.as_str())
                {
                    let line = line_of_value(raw, idx, c);
                    record_command(
                        CmdSite {
                            file,
                            origin: "hook",
                            json_path: Some(format!("{base}[{i}]")),
                            line,
                            context: None,
                            auto_runs: true,
                            drop: 0,
                        },
                        c,
                        sink,
                    );
                }
            }
        }
        return;
    };
    let known = tools::known_events(tool);
    for (event, groups) in events {
        if let Some(list) = known {
            if !list.contains(&event.as_str()) {
                sink.push(
                    cfg("HOOK_W003", file, format!("`{event}` for {tool}"))
                        .path(format!("{base}.{event}")),
                );
            }
        }
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (gi, group) in groups.iter().enumerate() {
            let gpath = format!("{base}.{event}[{gi}]");
            if let Some(c) = group.as_str() {
                sink.push(cfg("HOOK_W007", file, "string hook (legacy)").path(gpath.clone()));
                let line = line_of_value(raw, idx, c);
                record_command(
                    CmdSite {
                        file,
                        origin: "hook",
                        json_path: Some(gpath),
                        line,
                        context: Some(event.clone()),
                        auto_runs: true,
                        drop: 0,
                    },
                    c,
                    sink,
                );
                continue;
            }
            let matcher = group.get("matcher").and_then(|m| m.as_str()).unwrap_or("");
            let handlers: Vec<(String, &Value)> =
                match group.get("hooks").and_then(|h| h.as_array()) {
                    Some(hs) => hs
                        .iter()
                        .enumerate()
                        .map(|(hi, h)| (format!("{gpath}.hooks[{hi}]"), h))
                        .collect(),
                    None => {
                        // Cursor-style `{command}` directly, or the obsolete
                        // `{name, script, enabled}`.
                        if tool == "claude"
                            && (group.get("command").is_some() || group.get("script").is_some())
                        {
                            sink.push(
                                cfg("HOOK_W007", file, "handler without `hooks: [...]` wrapper")
                                    .path(gpath.clone()),
                            );
                        }
                        vec![(gpath.clone(), group)]
                    }
                };
            for (hpath, h) in handlers {
                let htype = h.get("type").and_then(|t| t.as_str()).unwrap_or("command");
                if h.get("if").is_some() {
                    let cond = h.get("if").map(|c| c.to_string()).unwrap_or_default();
                    sink.push(cfg("HOOK_W004", file, cond).path(format!("{hpath}.if")));
                }
                match htype {
                    "agent" | "prompt" => {
                        let prompt = h.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
                        let line = line_of_value(raw, idx, prompt);
                        let model = h.get("model").and_then(|m| m.as_str()).unwrap_or("default");
                        sink.push(
                            cfg(
                                "HOOK_W002",
                                file,
                                format!("{htype} handler on {event} (model {model})"),
                            )
                            .line(line)
                            .path(hpath.clone())
                            .snippet(prompt),
                        );
                        // The prompt goes to a model: injection rules apply.
                        let mut inner = Sink::default();
                        super::semantic::check_prose(file, prompt, false, &mut inner);
                        for mut f in inner.findings {
                            f.line = line;
                            f.json_path = Some(format!("{hpath}.prompt"));
                            sink.push(f);
                        }
                        sink.commands.push(ExecCommand {
                            file: file.to_string(),
                            origin: "agent_prompt".into(),
                            json_path: Some(hpath.clone()),
                            line,
                            context: Some(event.clone()),
                            command: prompt.to_string(),
                            programs: vec![format!("model:{model}")],
                            codes: vec!["HOOK_W002".into()],
                            worst: Some(sev("HOOK_W002")),
                            network: true,
                            auto_runs: true,
                        });
                    }
                    _ => {
                        let cmd = h
                            .get("command")
                            .or_else(|| h.get("script"))
                            .or_else(|| h.get("bash"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        if cmd.trim().is_empty() {
                            sink.push(
                                cfg("HOOK_E001", file, format!("{event} handler"))
                                    .path(hpath.clone()),
                            );
                            continue;
                        }
                        let line = line_of_value(raw, idx, cmd);
                        let a = record_command(
                            CmdSite {
                                file,
                                origin: "hook",
                                json_path: Some(format!("{hpath}.command")),
                                line,
                                context: Some(if matcher.is_empty() {
                                    event.clone()
                                } else {
                                    format!("{event} · {matcher}")
                                }),
                                auto_runs: true,
                                drop: 0,
                            },
                            cmd,
                            sink,
                        );
                        if a.network
                            && matches!(
                                event.as_str(),
                                "PreToolUse" | "PostToolUse" | "BeforeTool" | "AfterTool"
                            )
                            && (matcher.is_empty() || matcher == "*" || matcher == ".*")
                        {
                            sink.push(
                                cfg("HOOK_W008", file, format!("{event} without matcher"))
                                    .line(line)
                                    .path(hpath.clone())
                                    .snippet(cmd),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Shell-quote an argv for display and re-parsing.
pub fn join_argv(cmd: &str, args: &[String]) -> String {
    let q = |s: &str| {
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+%^~".contains(c))
        {
            s.to_string()
        } else {
            format!("'{}'", s.replace('\'', "'\\''"))
        }
    };
    let mut out = q(cmd);
    for a in args {
        out.push(' ');
        out.push_str(&q(a));
    }
    out
}

fn check_mcp(
    file: &str,
    raw: &str,
    idx: &LineIndex,
    name: &str,
    s: &Value,
    path: &str,
    sink: &mut Sink,
) {
    // OpenCode: `command: ["npx", "-y", …]`, `environment`.
    let (command, args): (Option<String>, Vec<String>) = match s.get("command") {
        Some(Value::String(c)) => (
            Some(c.clone()),
            s.get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        Some(Value::Array(parts)) => {
            let v: Vec<String> = parts
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            (v.first().cloned(), v.iter().skip(1).cloned().collect())
        }
        _ => (None, Vec::new()),
    };
    if let Some(cmd) = &command {
        let line = line_of_value(raw, idx, cmd);
        let full = join_argv(cmd, &args);
        let prog = shell::program_name(cmd);
        record_command(
            CmdSite {
                file,
                origin: "mcp",
                json_path: Some(format!("{path}.command")),
                line,
                context: Some(name.to_string()),
                auto_runs: true,
                drop: 0,
            },
            &full,
            sink,
        );
        if shell::program_name(cmd) == "sh"
            || matches!(
                prog.as_str(),
                "bash" | "zsh" | "cmd" | "powershell" | "pwsh"
            ) && args
                .iter()
                .any(|a| matches!(a.as_str(), "-c" | "/c" | "-Command"))
        {
            sink.push(
                cfg("MCP_W006", file, full.clone())
                    .line(line)
                    .path(format!("{path}.command")),
            );
        }
        if prog == "docker" || prog == "podman" {
            let bad = args.iter().enumerate().any(|(i, a)| {
                a == "--privileged"
                    || a.starts_with("--pid=host")
                    || ((a == "-v" || a == "--volume")
                        && args
                            .get(i + 1)
                            .is_some_and(|v| v.starts_with("/:") || v.contains("docker.sock")))
                    || a.starts_with("--volume=/:")
                    || a.contains("/var/run/docker.sock")
            });
            if bad {
                sink.push(
                    cfg("MCP_W005", file, full.clone())
                        .line(line)
                        .path(format!("{path}.args")),
                );
            }
        }
        if matches!(prog.as_str(), "npx" | "bunx" | "pnpx" | "uvx" | "pipx")
            || (prog == "pnpm" && args.first().is_some_and(|a| a == "dlx"))
        {
            if let Some(pkg) = args
                .iter()
                .find(|a| !a.starts_with('-') && *a != "run" && *a != "dlx")
            {
                let versioned = pkg
                    .rsplit_once('@')
                    .is_some_and(|(n, v)| !n.is_empty() && v != "latest")
                    || pkg.contains("==");
                if !versioned {
                    sink.push(
                        cfg("MCP_W004", file, pkg.clone())
                            .line(line)
                            .path(format!("{path}.args")),
                    );
                }
            }
        }
        let all = format!("{cmd} {}", args.join(" "));
        if all.contains("/path/to")
            || all.contains("<path")
            || all.contains("/abs/path")
            || all.contains("<your")
        {
            sink.push(
                cfg("MCP_W007", file, super::text::clip(&all, 120))
                    .line(line)
                    .path(path.to_string()),
            );
        }
        // Inline tokens in args: `--api-key X`, `--token=X`.
        for (i, a) in args.iter().enumerate() {
            let (flag, val) = match a.split_once('=') {
                Some((f, v)) if f.starts_with('-') => (f.to_string(), Some(v.to_string())),
                _ if a.starts_with('-') => (a.clone(), args.get(i + 1).cloned()),
                _ => continue,
            };
            if !is_secret_name(flag.trim_start_matches('-'))
                && !matches!(flag.as_str(), "-t" | "-k")
            {
                continue;
            }
            if let Some(v) = val {
                if v.starts_with('-') {
                    continue;
                }
                secret_value(
                    file,
                    raw,
                    idx,
                    &format!("{path}.args[{i}]"),
                    &flag,
                    &v,
                    sink,
                    "MCP_W001",
                );
            }
        }
    }
    let url = s
        .get("url")
        .or_else(|| s.get("httpUrl"))
        .or_else(|| s.get("serverUrl"))
        .and_then(|u| u.as_str());
    if let Some(u) = url {
        let line = line_of_value(raw, idx, u);
        let host = super::reference::host_of(u).unwrap_or_default();
        let local = matches!(
            super::reference::classify(&host),
            super::reference::HostClass::Loopback
        );
        if u.starts_with("http://") && !local {
            sink.push(
                cfg("MCP_W002", file, u.to_string())
                    .line(line)
                    .path(format!("{path}.url")),
            );
        }
        if let Some((_, q)) = u.split_once('?') {
            for kv in q.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    if is_secret_name(k) || k.eq_ignore_ascii_case("key") {
                        secret_value(
                            file,
                            raw,
                            idx,
                            &format!("{path}.url"),
                            k,
                            v,
                            sink,
                            "MCP_W001",
                        );
                    }
                }
            }
        }
        sink.commands.push(ExecCommand {
            file: file.to_string(),
            origin: "mcp".into(),
            json_path: Some(format!("{path}.url")),
            line,
            context: Some(name.to_string()),
            command: u.to_string(),
            programs: vec![],
            codes: vec![],
            worst: None,
            network: true,
            auto_runs: true,
        });
    }
    for env_key in [
        "env",
        "environment",
        "headers",
        "http_headers",
        "requestInit",
    ] {
        if let Some(obj) = s.get(env_key).and_then(|e| e.as_object()) {
            for (k, v) in obj {
                let Some(val) = v.as_str() else { continue };
                let p = format!("{path}.{env_key}.{k}");
                let header_auth = env_key.contains("header")
                    && (k.eq_ignore_ascii_case("authorization")
                        || k.to_lowercase().contains("api-key")
                        || k.to_lowercase().contains("token"));
                if is_secret_name(k) || header_auth {
                    let v2 = val
                        .trim_start_matches("Bearer ")
                        .trim_start_matches("bearer ")
                        .trim_start_matches("Token ");
                    secret_value(file, raw, idx, &p, k, v2, sink, "MCP_W001");
                }
            }
        }
    }
    for key in ["autoApprove", "alwaysAllow", "trust"] {
        match s.get(key) {
            Some(Value::Array(a)) if !a.is_empty() => {
                sink.push(
                    cfg("MCP_W008", file, format!("{key}: {} tools", a.len()))
                        .path(format!("{path}.{key}")),
                );
            }
            Some(Value::Bool(true)) => sink
                .push(cfg("MCP_W008", file, format!("{key}: true")).path(format!("{path}.{key}"))),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn secret_value(
    file: &str,
    raw: &str,
    idx: &LineIndex,
    path: &str,
    name: &str,
    val: &str,
    sink: &mut Sink,
    code: &str,
) {
    let line = line_of_value(raw, idx, val);
    if is_placeholder(val) {
        if !sink
            .findings
            .iter()
            .any(|f| f.code == "MCP_I001" && f.json_path.as_deref() == Some(path))
        {
            sink.push(
                cfg("MCP_I001", file, name.to_string())
                    .line(line)
                    .path(path.to_string()),
            );
        }
    } else if looks_real_secret(val) {
        sink.push(
            cfg(code, file, format!("{name} = {}", redact(val)))
                .line(line)
                .path(path.to_string())
                .snippet(format!("\"{name}\": \"{}\"", redact(val))),
        );
    }
}

const OFFICIAL_API_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "api.openai.com",
    "generativelanguage.googleapis.com",
    "aiplatform.googleapis.com",
    "bedrock-runtime",
    "amazonaws.com",
    "openai.azure.com",
    "api.githubcopilot.com",
];

fn check_settings(file: &str, raw: &str, idx: &LineIndex, v: &Value, sink: &mut Sink) {
    if let Some(perms) = v.get("permissions") {
        if let Some(allow) = perms.get("allow").and_then(|a| a.as_array()) {
            for (i, rule) in allow.iter().enumerate() {
                let Some(r) = rule.as_str() else { continue };
                let t = r.trim();
                let broad = matches!(t, "Bash" | "Bash(*)" | "Bash(*:*)" | "*" | "Bash(**)")
                    || [
                        "sudo", "rm", "curl", "wget", "ssh", "chmod", "eval", "sh", "bash",
                    ]
                    .iter()
                    .any(|p| {
                        t.starts_with(&format!("Bash({p}:")) || t.starts_with(&format!("Bash({p} "))
                    });
                if broad {
                    sink.push(
                        cfg("SET_W001", file, t.to_string())
                            .line(line_of_value(raw, idx, r))
                            .path(format!("permissions.allow[{i}]")),
                    );
                }
            }
        }
        if perms.get("defaultMode").and_then(|m| m.as_str()) == Some("bypassPermissions") {
            sink.push(
                cfg("SET_W002", file, "defaultMode: bypassPermissions")
                    .path("permissions.defaultMode"),
            );
        }
    }
    for k in [
        "dangerouslySkipPermissions",
        "skipDangerousModePermissionPrompt",
        "dangerously_skip_permissions",
        "yolo",
        "autoAccept",
    ] {
        if v.get(k).and_then(|x| x.as_bool()) == Some(true) {
            sink.push(cfg("SET_W002", file, format!("{k}: true")).path(k));
        }
    }
    if v.get("approval_policy").and_then(|x| x.as_str()) == Some("never")
        && v.get("sandbox_mode").and_then(|x| x.as_str()) == Some("danger-full-access")
    {
        sink.push(
            cfg("SET_W002", file, "approval never + danger-full-access").path("approval_policy"),
        );
    }
    if v.get("enableAllProjectMcpServers")
        .and_then(|x| x.as_bool())
        == Some(true)
    {
        sink.push(
            cfg("SET_W003", file, "enableAllProjectMcpServers: true")
                .path("enableAllProjectMcpServers"),
        );
    }
    if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
        for (k, val) in env {
            let Some(s) = val.as_str() else { continue };
            let p = format!("env.{k}");
            let line = line_of_value(raw, idx, s);
            let up = k.to_uppercase();
            if up.ends_with("_BASE_URL")
                || up == "HTTPS_PROXY"
                || up == "HTTP_PROXY"
                || up == "ALL_PROXY"
            {
                let host = super::reference::host_of(s).unwrap_or_default();
                let official = OFFICIAL_API_HOSTS
                    .iter()
                    .any(|h| host == *h || host.ends_with(&format!(".{h}")) || host.contains(h));
                let local = matches!(
                    super::reference::classify(&host),
                    super::reference::HostClass::Loopback
                );
                if !official && !local && !is_placeholder(s) {
                    sink.push(
                        cfg("SET_W004", file, format!("{k} → {host}"))
                            .line(line)
                            .path(p.clone()),
                    );
                }
            }
            if up.starts_with("OTEL_EXPORTER_OTLP") && up.ends_with("ENDPOINT") {
                let host = super::reference::host_of(s).unwrap_or_default();
                if !matches!(
                    super::reference::classify(&host),
                    super::reference::HostClass::Loopback
                ) && !is_placeholder(s)
                {
                    sink.push(
                        cfg("SET_W007", file, format!("{k} → {host}"))
                            .line(line)
                            .path(p.clone()),
                    );
                }
            }
            if is_secret_name(k) {
                secret_value(file, raw, idx, &p, k, s, sink, "SET_W005");
            }
        }
    }
    for k in [
        "apiKeyHelper",
        "awsAuthRefresh",
        "awsCredentialExport",
        "otelHeadersHelper",
        "fileSuggestion",
    ] {
        let cmd = match v.get(k) {
            Some(Value::String(s)) => Some(s.as_str()),
            Some(Value::Object(o)) => o.get("command").and_then(|c| c.as_str()),
            _ => None,
        };
        if let Some(c) = cmd {
            let line = line_of_value(raw, idx, c);
            sink.push(
                cfg(
                    "SET_W006",
                    file,
                    format!("{k}: {}", super::text::clip(c, 100)),
                )
                .line(line)
                .path(k),
            );
            record_command(
                CmdSite {
                    file,
                    origin: "setting",
                    json_path: Some(k.into()),
                    line,
                    context: Some(k.into()),
                    auto_runs: true,
                    drop: 0,
                },
                c,
                sink,
            );
        }
    }
}
