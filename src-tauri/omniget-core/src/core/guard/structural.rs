//! Structural validator: size, encoding, frontmatter, required fields per
//! kind, description length, tool names per origin tool, model id, sections.
//!
//! Improvements over the original: required fields follow what each tool
//! actually loads (Claude agents need `name` + `description`, not `tools`;
//! commands need no frontmatter at all), and a tool list in another tool's
//! vocabulary is reported as "another tool's format", never as an error.

use super::model::{Finding, Severity, Sink, Validator};
use super::rules::sev;
use super::text::{parse_frontmatter, Frontmatter, LineIndex};
use super::tools;
use super::{Ctx, Kind};

pub const MAX_FILE_SIZE: usize = 100 * 1024;
const MAX_SECTIONS: usize = 20;

fn f(code: &str, file: &str, detail: impl Into<String>) -> Finding {
    Finding::new(code, Validator::Structural, sev(code), file, detail)
}

/// Byte-level checks for any text file. Returns the text when it decodes.
pub fn check_bytes(
    file: &str,
    bytes: &[u8],
    sink: &mut Sink,
    size_limited: bool,
) -> Option<String> {
    if bytes.is_empty() {
        sink.push(f("STRUCT_E001", file, "0 bytes"));
        return Some(String::new());
    }
    if size_limited {
        if bytes.len() > MAX_FILE_SIZE {
            sink.push(f(
                "STRUCT_E003",
                file,
                format!("{:.1} KB", bytes.len() as f64 / 1024.0),
            ));
        } else if bytes.len() > MAX_FILE_SIZE * 8 / 10 {
            sink.push(f(
                "STRUCT_W002",
                file,
                format!("{:.1} KB", bytes.len() as f64 / 1024.0),
            ));
        }
    }
    if bytes.contains(&0) {
        sink.push(f("STRUCT_E005", file, "NUL byte"));
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_string()),
        Err(e) => {
            sink.push(f(
                "STRUCT_E004",
                file,
                format!("invalid byte at offset {}", e.valid_up_to()),
            ));
            None
        }
    }
}

/// The tool whose format a Markdown component is written in.
pub fn detect_tool(file: &str, fm: &Frontmatter) -> String {
    if let Some(t) = tools::detect_from_path(file) {
        return t.to_string();
    }
    for key in ["tools", "allowed-tools", "allowed_tools"] {
        if let Some(v) = fm.get(key) {
            if let Some(t) = tools::tools_fingerprint(&v.items()) {
                return t.to_string();
            }
        }
    }
    if fm.get("applyTo").is_some() {
        return "copilot".into();
    }
    if fm.get("alwaysApply").is_some() || fm.get("globs").is_some() {
        return "cursor".into();
    }
    if fm.get("inclusion").is_some() {
        return "kiro".into();
    }
    if fm
        .get("mode")
        .and_then(|m| m.as_str())
        .is_some_and(|m| matches!(m, "subagent" | "primary" | "all"))
    {
        return "opencode".into();
    }
    "claude".into()
}

/// Checks for a Markdown file of the component.
pub fn check_markdown(
    ctx: &mut Ctx,
    file: &str,
    text: &str,
    is_entry: bool,
    sink: &mut Sink,
) -> Frontmatter {
    let fm = parse_frontmatter(text);
    let idx = LineIndex::new(text);
    if !is_entry {
        return fm;
    }
    let detected = detect_tool(file, &fm);
    ctx.detected = detected.clone();
    let effective = ctx.origin.clone().unwrap_or_else(|| detected.clone());

    if let Some((line, msg)) = &fm.error {
        sink.push(
            f("STRUCT_E002", file, msg.clone())
                .at(*line, 1)
                .snippet(idx_line(text, &idx, *line)),
        );
    }
    let needs_fm = matches!(ctx.kind, Kind::Agent | Kind::Skill)
        && !(ctx.kind == Kind::Agent
            && matches!(
                effective.as_str(),
                "codex" | "cline" | "windsurf" | "aider" | "zed" | "warp" | "junie"
            ));
    if !fm.present {
        if needs_fm {
            sink.push(f(
                "STRUCT_W001",
                file,
                format!("{} without --- block", ctx.kind.as_str()),
            ));
        } else if matches!(ctx.kind, Kind::Command | Kind::Loop | Kind::Workflow) {
            sink.push(f("STRUCT_I006", file, ctx.kind.as_str()));
        }
    }

    // Required fields.
    if fm.present {
        let required: &[&str] = match (ctx.kind, effective.as_str()) {
            (Kind::Agent, "copilot") => &["description"],
            (Kind::Agent, "cursor") => &["description"],
            (Kind::Agent, "kiro") => &[],
            (Kind::Agent, "opencode") => &["description"],
            (Kind::Agent, _) => &["name", "description"],
            (Kind::Skill, _) => &["name", "description"],
            _ => &[],
        };
        for key in required {
            if fm.get(key).is_none_or(|v| v.is_empty()) {
                sink.push(f(
                    "STRUCT_E006",
                    file,
                    format!("`{key}` missing for {} ({effective})", ctx.kind.as_str()),
                ));
            }
        }
        if let Some(d) = fm.get("description") {
            match d.as_str() {
                None if !d.is_empty() => {
                    sink.push(f("STRUCT_E007", file, "description is a list or mapping"))
                }
                Some(s) => {
                    let n = s.trim().chars().count();
                    if n > 0 && n < 20 {
                        sink.push(f("STRUCT_W003", file, format!("{n} chars")));
                    } else if n > 1024 {
                        sink.push(f("STRUCT_W004", file, format!("{n} chars")));
                    }
                }
                None => {}
            }
        }
        // Tool list.
        let key = ["tools", "allowed-tools", "allowed_tools"]
            .into_iter()
            .find(|k| fm.get(k).is_some());
        if let Some(key) = key {
            let items = fm.get(key).map(|v| v.items()).unwrap_or_default();
            let raw_empty = fm.get(key).is_none_or(|v| v.is_empty());
            if items.is_empty() && raw_empty {
                sink.push(f("STRUCT_W005", file, format!("`{key}` is empty")));
            }
            let fp = tools::tools_fingerprint(&items);
            let line = line_of_key(text, key);
            if let Some(fp) = fp.filter(|fp| *fp != effective.as_str()) {
                // Written for another tool than the one claimed.
                ctx.foreign = Some(fp.to_string());
                let mut x = f(
                    "STRUCT_W012",
                    file,
                    format!("`{key}` uses {fp} tool ids in a {effective} item"),
                )
                .line(line);
                x.snippet = line.map(|l| super::text::clip(idx_line(text, &idx, l), 200));
                sink.push(x);
            } else {
                let unknown: Vec<String> = items
                    .iter()
                    .filter(|t| !tools::is_known(&effective, t))
                    .cloned()
                    .collect();
                if !unknown.is_empty() {
                    let mut x = f(
                        "STRUCT_W006",
                        file,
                        format!("{effective}: {}", unknown.join(", ")),
                    )
                    .line(line);
                    x.snippet = line.map(|l| super::text::clip(idx_line(text, &idx, l), 200));
                    sink.push(x);
                }
            }
            if effective != ctx.origin.clone().unwrap_or_default()
                && ctx.origin.is_none()
                && effective != "claude"
            {
                // Detected native format of another tool without a claim: info only.
                sink.push(f("STRUCT_I005", file, format!("native {effective} format")));
            }
            // Unrestricted Bash grant in a command/skill.
            if matches!(ctx.kind, Kind::Command | Kind::Skill)
                && items
                    .iter()
                    .any(|t| matches!(t.trim(), "Bash" | "Bash(*)" | "Bash(*:*)" | "*"))
            {
                let mut x = Finding::new(
                    "SEM_W006",
                    Validator::Semantic,
                    sev("SEM_W006"),
                    file,
                    format!("`{key}` grants Bash without a pattern"),
                )
                .line(line);
                x.snippet = line.map(|l| super::text::clip(idx_line(text, &idx, l), 200));
                sink.push(x);
            }
        } else if effective != "claude" && ctx.origin.is_none() {
            sink.push(f("STRUCT_I005", file, format!("native {effective} format")));
        }
        // Model.
        if let Some(m) = fm.str("model") {
            let m = m.trim();
            if effective == "claude" && !m.is_empty() {
                let ok = matches!(
                    m,
                    "sonnet"
                        | "opus"
                        | "haiku"
                        | "inherit"
                        | "fable"
                        | "default"
                        | "opusplan"
                        | "sonnet[1m]"
                        | "opus[1m]"
                ) || m.starts_with("claude-");
                if !ok {
                    sink.push(
                        f("STRUCT_W008", file, format!("`{m}` for claude"))
                            .line(line_of_key(text, "model")),
                    );
                }
            }
        }
        // Skill name ↔ folder.
        if ctx.kind == Kind::Skill {
            if let Some(name) = fm.str("name") {
                let kebab = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                let dir_ok = ctx.dir_name.as_deref().is_none_or(|d| d == name);
                if !kebab || !dir_ok {
                    sink.push(f(
                        "STRUCT_W013",
                        file,
                        format!(
                            "name `{name}`, folder `{}`",
                            ctx.dir_name.clone().unwrap_or_default()
                        ),
                    ));
                }
            }
        }
    }

    // Body.
    let body = &text[fm.body_offset.min(text.len())..];
    let body_len = body.trim().chars().count();
    if body_len < 50 && matches!(ctx.kind, Kind::Agent | Kind::Skill | Kind::Command) {
        sink.push(f("STRUCT_W009", file, format!("{body_len} chars")));
    }
    let sections = count_sections(body);
    if sections > MAX_SECTIONS {
        sink.push(f("STRUCT_W011", file, format!("{sections} headings")));
    }
    fm
}

/// Headings outside code fences.
pub fn count_sections(text: &str) -> usize {
    let mut n = 0;
    let mut in_fence = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence && t.starts_with('#') {
            let hashes = t.chars().take_while(|c| *c == '#').count();
            if hashes <= 6 && t[hashes..].starts_with(' ') {
                n += 1;
            }
        }
    }
    n
}

fn line_of_key(text: &str, key: &str) -> Option<u32> {
    let needle = format!("{key}:");
    text.lines()
        .position(|l| l.starts_with(&needle))
        .map(|i| i as u32 + 1)
}

fn idx_line<'a>(text: &'a str, _idx: &LineIndex, line: u32) -> &'a str {
    text.lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or("")
}

/// JSON shape required by kind.
pub fn check_json_shape(kind: Kind, file: &str, v: &serde_json::Value, sink: &mut Sink) {
    let obj = match v.as_object() {
        Some(o) => o,
        None => {
            sink.push(f("STRUCT_E009", file, "top level is not an object"));
            return;
        }
    };
    let servers = super::config::servers_of(v);
    match kind {
        Kind::Hook => {
            if !obj.contains_key("hooks") {
                sink.push(f("STRUCT_E009", file, "hook without `hooks`"));
            }
        }
        Kind::Mcp => {
            if servers.is_empty() {
                sink.push(f(
                    "STRUCT_E009",
                    file,
                    "no MCP servers (`mcpServers`/`servers`/`mcp`)",
                ));
            }
            for (name, s, _) in servers {
                let has = s.get("command").is_some()
                    || s.get("url").is_some()
                    || s.get("httpUrl").is_some()
                    || s.get("serverUrl").is_some();
                if !has {
                    sink.push(f(
                        "STRUCT_E009",
                        file,
                        format!("server `{name}` has neither command nor url"),
                    ));
                }
                if let Some(a) = s.get("args") {
                    if !a.is_array() {
                        sink.push(f(
                            "STRUCT_E009",
                            file,
                            format!("server `{name}`: args is not an array"),
                        ));
                    }
                }
                if let Some(e) = s.get("env") {
                    if !e.is_object() {
                        sink.push(f(
                            "STRUCT_E009",
                            file,
                            format!("server `{name}`: env is not an object"),
                        ));
                    }
                }
            }
        }
        Kind::Statusline => {
            if obj
                .get("statusLine")
                .and_then(|s| s.get("command"))
                .is_none()
            {
                sink.push(f(
                    "STRUCT_E009",
                    file,
                    "statusline without `statusLine.command`",
                ));
            }
        }
        _ => {}
    }
    let _ = Severity::Info;
}
