//! "Config health" numbers: token weight of commands/rules/agents/AGENTS.md,
//! hooks per event (and whether their program exists), MCP servers by
//! category and complexity. Port of `--command-stats`, `--hook-stats` and
//! `--mcp-stats`, fixed: hooks are read in the real `{matcher, hooks:[…]}`
//! shape, every file the caller passes is read (not only `./.claude`), and
//! the token estimate counts words, code and Markdown separately.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::config::servers_of;
use super::shell;
use super::text::parse_frontmatter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocStat {
    pub path: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub bytes: u64,
    pub lines: usize,
    pub words: usize,
    pub sections: usize,
    pub code_blocks: usize,
    pub tokens: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookStat {
    pub file: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// `command` | `agent` | `prompt` | …
    pub handler: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// Whether the program resolves on PATH (or as a file). `None` for
    /// `agent`/`prompt` handlers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_found: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_found: Option<bool>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// `$CLAUDE_*` variables the hook reads that are never set.
    pub fake_env: Vec<String>,
    pub tokens: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventCount {
    pub total: usize,
    pub enabled: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpStat {
    pub file: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `stdio` | `http` | `sse`.
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub args: usize,
    pub env: usize,
    pub enabled: bool,
    pub category: String,
    /// 1–5, rules of the original `mcp-stats`.
    pub complexity: u8,
    pub config_bytes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatTotals {
    pub docs: usize,
    pub doc_tokens: usize,
    pub doc_bytes: u64,
    pub hooks: usize,
    pub hooks_enabled: usize,
    pub hooks_missing_program: usize,
    pub hooks_fake_env: usize,
    pub mcps: usize,
    pub mcps_enabled: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigStats {
    pub docs: Vec<DocStat>,
    pub hooks: Vec<HookStat>,
    pub hooks_by_event: BTreeMap<String, EventCount>,
    pub mcps: Vec<McpStat>,
    pub mcp_by_category: BTreeMap<String, usize>,
    pub totals: StatTotals,
    pub errors: Vec<StatError>,
}

// ------------------------------------------------------------------ tokens

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// Tokens of one whitespace-free chunk, BPE-style: short ASCII words are one
/// token, long ones split every ~4 chars; digits every 3; accented or other
/// non-ASCII letters cost more; CJK is about one per char; punctuation is one
/// per char except long runs of the same char (`----`, `====`).
fn chunk_tokens(chunk: &str, code: bool) -> f64 {
    let chars: Vec<char> = chunk.chars().collect();
    let mut t = 0.0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_alphabetic() || c == '_' {
            // Split identifiers at case changes/underscores for code.
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphabetic() || (chars[j] == '_' && !code))
            {
                if code && chars[j].is_ascii_uppercase() && chars[j - 1].is_ascii_lowercase() {
                    break;
                }
                j += 1;
            }
            let len = j - i;
            t += if len <= 7 {
                1.0
            } else {
                1.0 + ((len - 7) as f64 / 4.0).ceil()
            };
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            t += ((j - i) as f64 / 3.0).ceil();
            i = j;
        } else if is_cjk(c) {
            t += 1.0;
            i += 1;
        } else if c.is_alphabetic() {
            // Non-ASCII letter run (accents etc.), mixed with ASCII letters.
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_alphabetic() && !is_cjk(chars[j]) {
                j += 1;
            }
            t += ((j - i) as f64 / 2.5).ceil();
            i = j;
        } else {
            let mut j = i + 1;
            while j < chars.len() && chars[j] == c {
                j += 1;
            }
            let run = j - i;
            t += if run >= 3 {
                (run as f64 / 4.0).ceil()
            } else {
                run as f64
            };
            i = j;
        }
    }
    t
}

/// Estimated tokens of a Markdown/text document.
pub fn estimate_tokens(text: &str) -> usize {
    let mut total = 0.0;
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            total += 2.0;
            continue;
        }
        if in_fence {
            let indent = line.len() - trimmed.len();
            total += (indent as f64 / 4.0).ceil();
            for w in trimmed.split_whitespace() {
                total += chunk_tokens(w, true);
            }
        } else {
            for w in line.split_whitespace() {
                total += chunk_tokens(w, false);
            }
        }
        total += 1.0; // newline
    }
    total.round() as usize
}

// ------------------------------------------------------------------ docs

fn modified(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(m).to_rfc3339())
}

fn doc_files(p: &Path, kind: &str) -> Vec<PathBuf> {
    if p.is_file() {
        return vec![p.to_path_buf()];
    }
    if !p.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(p)
        .max_depth(if kind == "skill" { 3 } else { 4 })
        .into_iter()
        .flatten()
    {
        if !e.file_type().is_file() {
            continue;
        }
        let n = e.file_name().to_string_lossy().to_lowercase();
        let ok = if kind == "skill" {
            n == "skill.md"
        } else {
            n.ends_with(".md") || n.ends_with(".mdc") || n.ends_with(".toml") || n.ends_with(".txt")
        };
        if ok {
            out.push(e.path().to_path_buf());
        }
        if out.len() > 2000 {
            break;
        }
    }
    out.sort();
    out
}

pub fn doc_stat(p: &Path, kind: &str) -> Result<DocStat, String> {
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let fm = parse_frontmatter(&text);
    let body = &text[fm.body_offset.min(text.len())..];
    let title = fm.str("name").map(String::from).or_else(|| {
        body.lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l[2..].trim().to_string())
    });
    let name = if p
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("SKILL.md"))
    {
        p.parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        p.file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let mut code_blocks = 0;
    let mut in_fence = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            if !in_fence {
                code_blocks += 1;
            }
            in_fence = !in_fence;
        }
    }
    Ok(DocStat {
        path: p.display().to_string(),
        kind: kind.to_string(),
        name,
        title,
        bytes: bytes.len() as u64,
        lines: text.lines().count(),
        words: text.split_whitespace().count(),
        sections: super::structural::count_sections(&text),
        code_blocks,
        tokens: estimate_tokens(&text),
        modified: modified(p),
    })
}

// ------------------------------------------------------------------ hooks

const BUILTINS: &[&str] = &[
    "echo", "cd", "pwd", "exit", "export", "unset", "alias", "type", "which", "command", "builtin",
    "source", ".", "test", "[", "[[", "if", "then", "else", "fi", "case", "esac", "for", "while",
    "true", "false", "printf", "read", "set", "eval", "exec", "trap", "return", "shift", "wait",
    ":", "cat", "local",
];

fn expand(s: &str, project: Option<&Path>) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    let proj = project.map(Path::to_path_buf).unwrap_or_default();
    let mut t = s.trim_matches(|c| c == '"' || c == '\'').to_string();
    for (k, v) in [
        ("${CLAUDE_PROJECT_DIR}", &proj),
        ("$CLAUDE_PROJECT_DIR", &proj),
        ("${HOME}", &home),
        ("$HOME", &home),
    ] {
        if t.starts_with(k) {
            t = format!("{}{}", v.display(), &t[k.len()..]);
        }
    }
    if let Some(r) = t.strip_prefix("~/") {
        return home.join(r);
    }
    let p = PathBuf::from(&t);
    if p.is_absolute() {
        p
    } else {
        proj.join(p)
    }
}

fn program_exists(prog_word: &str, project: Option<&Path>) -> bool {
    let name = shell::program_name(prog_word);
    if BUILTINS.contains(&name.as_str()) {
        return true;
    }
    if prog_word.contains('/') || prog_word.contains('\\') {
        return expand(prog_word, project).is_file();
    }
    crate::core::skills::scan::find_in_path(std::env::var_os("PATH").as_deref(), &name).is_some()
        || (cfg!(windows)
            && crate::core::skills::scan::find_in_path(
                std::env::var_os("PATH").as_deref(),
                &format!("{name}.exe"),
            )
            .is_some())
}

fn hook_rows(file: &str, v: &serde_json::Value, project: Option<&Path>, out: &mut Vec<HookStat>) {
    let all_disabled = v.get("disableAllHooks").and_then(|x| x.as_bool()) == Some(true);
    let Some(events) = v.get("hooks").and_then(|h| h.as_object()) else {
        return;
    };
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for g in groups {
            let matcher = g.get("matcher").and_then(|m| m.as_str()).map(String::from);
            let handlers: Vec<&serde_json::Value> = match g.get("hooks").and_then(|h| h.as_array())
            {
                Some(hs) => hs.iter().collect(),
                None => vec![g],
            };
            for h in handlers {
                let handler = h
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or(if g.is_string() { "command" } else { "command" })
                    .to_string();
                let command = h
                    .as_str()
                    .or_else(|| h.get("command").and_then(|c| c.as_str()))
                    .or_else(|| h.get("prompt").and_then(|c| c.as_str()))
                    .unwrap_or("")
                    .to_string();
                let enabled = !all_disabled
                    && h.get("enabled").and_then(|e| e.as_bool()) != Some(false)
                    && h.get("disabled").and_then(|e| e.as_bool()) != Some(true);
                let (program, program_found, script, script_found) = if handler == "command" {
                    let script_ = shell::parse(&command);
                    // The first real program: skip `if [[ … ]]`, `echo`,
                    // assignments; fall back to the first builtin.
                    let mut all_cmds: Vec<shell::Cmd> = script_
                        .pipelines
                        .iter()
                        .flat_map(|p| p.cmds.iter().cloned())
                        .collect();
                    let subs: Vec<String> = all_cmds
                        .iter()
                        .flat_map(|c| c.subs.iter().cloned())
                        .collect();
                    for sub in subs {
                        all_cmds.extend(
                            shell::parse(&sub)
                                .pipelines
                                .into_iter()
                                .flat_map(|p| p.cmds),
                        );
                    }
                    let resolved: Vec<(shell::Cmd, (String, usize, bool))> = all_cmds
                        .iter()
                        .filter_map(|c| shell::resolve(c).map(|r| (c.clone(), r)))
                        .collect();
                    let pick = resolved
                        .iter()
                        .find(|(_, r)| {
                            !BUILTINS.contains(&r.0.as_str())
                                && r.0
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
                        })
                        .or_else(|| resolved.first())
                        .cloned();
                    match pick {
                        Some((c, (prog, args_at, _))) => {
                            let prog_word = c
                                .words
                                .get(args_at.saturating_sub(1))
                                .cloned()
                                .unwrap_or(prog.clone());
                            let found = program_exists(&prog_word, project);
                            let interp = prog.starts_with("python")
                                || matches!(
                                    prog.as_str(),
                                    "node"
                                        | "bash"
                                        | "sh"
                                        | "zsh"
                                        | "bun"
                                        | "deno"
                                        | "ruby"
                                        | "perl"
                                        | "uv"
                                        | "npx"
                                        | "tsx"
                                );
                            let script = interp
                                .then(|| {
                                    c.words
                                        .iter()
                                        .skip(args_at)
                                        .find(|w| {
                                            !w.starts_with('-')
                                                && (w.contains('/') || w.contains('.'))
                                        })
                                        .cloned()
                                })
                                .flatten()
                                .filter(|s| !s.contains("://") && !s.starts_with('@'));
                            let script_found =
                                script.as_ref().map(|s| expand(s, project).is_file());
                            (Some(prog), Some(found), script, script_found)
                        }
                        None => (None, None, None, None),
                    }
                } else {
                    (None, None, None, None)
                };
                let fake_env = shell::FAKE_ENV
                    .captures_iter(&command)
                    .map(|c| c[1].to_string())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                out.push(HookStat {
                    file: file.to_string(),
                    event: event.clone(),
                    matcher: matcher.clone(),
                    handler,
                    tokens: estimate_tokens(&serde_json::to_string(h).unwrap_or_default()),
                    command,
                    program,
                    program_found,
                    script,
                    script_found,
                    enabled,
                    timeout: h.get("timeout").and_then(|t| t.as_u64()),
                    fake_env,
                });
            }
        }
    }
}

// ------------------------------------------------------------------ mcp

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Category by keyword, in the original's order; words are matched whole
/// (the original's substring test put every "email" server under AI & ML).
pub fn mcp_category(name: &str, command: &str, description: &str) -> &'static str {
    let n = words(&format!("{name} {command}"));
    let d = words(description);
    let any = |ws: &[String], keys: &[&str]| ws.iter().any(|w| keys.contains(&w.as_str()));
    if any(
        &n,
        &[
            "ide",
            "vscode",
            "jupyter",
            "notebook",
            "lsp",
            "serena",
            "xcode",
            "android",
            "ios",
            "simulator",
        ],
    ) || any(&d, &["ide"])
    {
        "IDE & Development"
    } else if any(
        &n,
        &[
            "postgres",
            "postgresql",
            "mysql",
            "sqlite",
            "database",
            "db",
            "dbhub",
            "mongodb",
            "mongo",
            "redis",
            "supabase",
            "neon",
            "sql",
            "mssql",
            "elasticsearch",
        ],
    ) || any(&d, &["database", "sql"])
    {
        "Database"
    } else if any(
        &n,
        &[
            "web",
            "search",
            "api",
            "http",
            "fetch",
            "browser",
            "playwright",
            "puppeteer",
            "scrape",
            "crawl",
            "firecrawl",
            "brightdata",
            "apify",
            "searxng",
        ],
    ) || any(&d, &["web", "search", "browser"])
    {
        "Web & API"
    } else if any(&n, &["file", "files", "filesystem", "fs", "directory"])
        || any(&d, &["file", "files", "filesystem"])
    {
        "Filesystem"
    } else if any(
        &n,
        &[
            "git",
            "github",
            "gitlab",
            "bitbucket",
            "docker",
            "kubernetes",
            "k8s",
            "aks",
            "terraform",
            "pulumi",
            "aws",
            "azure",
            "circleci",
            "sentry",
            "grafana",
            "vercel",
            "railway",
            "jfrog",
        ],
    ) || any(&d, &["git", "docker", "kubernetes", "deploy", "ci"])
    {
        "DevOps"
    } else if any(
        &n,
        &[
            "ai",
            "ml",
            "model",
            "llm",
            "openai",
            "huggingface",
            "anthropic",
            "gemini",
        ],
    ) || any(&d, &["ai", "llm", "ml"])
        || description.to_lowercase().contains("machine learning")
    {
        "AI & ML"
    } else {
        "Other"
    }
}

/// 1 + args + env + settings + (env > 3) + (env > 6), capped at 5.
pub fn mcp_complexity(s: &serde_json::Value) -> u8 {
    let len = |k: &str| {
        s.get(k)
            .map(|v| {
                v.as_array()
                    .map(|a| a.len())
                    .or_else(|| v.as_object().map(|o| o.len()))
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    };
    let env = len("env").max(len("environment"));
    let mut c = 1u8;
    if len("args") > 0
        || s.get("command")
            .and_then(|c| c.as_array())
            .is_some_and(|a| a.len() > 1)
    {
        c += 1;
    }
    if env > 0 {
        c += 1;
    }
    if len("settings") > 0 {
        c += 1;
    }
    if env > 3 {
        c += 1;
    }
    if env > 6 {
        c += 1;
    }
    c.min(5)
}

fn mcp_rows(file: &str, v: &serde_json::Value, out: &mut Vec<McpStat>) {
    for (name, s, _) in servers_of(v) {
        let command = match s.get("command") {
            Some(serde_json::Value::String(c)) => Some(c.clone()),
            Some(serde_json::Value::Array(a)) => {
                a.first().and_then(|x| x.as_str()).map(String::from)
            }
            _ => None,
        };
        let url = s
            .get("url")
            .or_else(|| s.get("httpUrl"))
            .or_else(|| s.get("serverUrl"))
            .and_then(|u| u.as_str());
        let transport = match s
            .get("type")
            .or_else(|| s.get("transport"))
            .and_then(|t| t.as_str())
        {
            Some(t) if t.contains("sse") => "sse",
            Some(t) if t.contains("http") || t == "remote" => "http",
            _ if command.is_none() && url.is_some() => "http",
            _ => "stdio",
        }
        .to_string();
        let description = s
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from);
        let args = s
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let env = s
            .get("env")
            .or_else(|| s.get("environment"))
            .and_then(|e| e.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        let enabled = s.get("disabled").and_then(|d| d.as_bool()) != Some(true)
            && s.get("enabled").and_then(|d| d.as_bool()) != Some(false);
        out.push(McpStat {
            file: file.to_string(),
            category: mcp_category(
                &name,
                command.as_deref().unwrap_or(url.unwrap_or("")),
                description.as_deref().unwrap_or(""),
            )
            .to_string(),
            complexity: mcp_complexity(s),
            config_bytes: serde_json::to_string(s).map(|x| x.len()).unwrap_or(0),
            name,
            description,
            transport,
            command: command.or(url.map(String::from)),
            args,
            env,
            enabled,
        });
    }
}

// ------------------------------------------------------------------ entry

/// Build the stats. Keys of `paths_by_kind`: `command(s)`, `rule(s)`,
/// `agent(s)`, `skill(s)`, `memory` (AGENTS.md/CLAUDE.md…), `hook(s)` /
/// `setting(s)` (JSON with a `hooks` object), `mcp(s)`. A path may be a file
/// or a folder (folders are walked for `.md`/`.mdc`/`.toml`, or `SKILL.md`).
pub fn config_stats(
    paths_by_kind: &BTreeMap<String, Vec<String>>,
    project_dir: Option<&Path>,
) -> ConfigStats {
    let mut st = ConfigStats::default();
    for (kind_raw, paths) in paths_by_kind {
        let kind = kind_raw.trim().to_lowercase();
        let kind = kind.trim_end_matches('s').to_string();
        for p in paths {
            let path = PathBuf::from(p);
            match kind.as_str() {
                "hook" | "setting" | "mcp" => {
                    if !path.is_file() {
                        if path.exists() {
                            st.errors.push(StatError {
                                path: p.clone(),
                                error: "not a file".into(),
                            });
                        }
                        continue;
                    }
                    let raw = match std::fs::read_to_string(&path) {
                        Ok(r) => r,
                        Err(e) => {
                            st.errors.push(StatError {
                                path: p.clone(),
                                error: e.to_string(),
                            });
                            continue;
                        }
                    };
                    let v: serde_json::Value = match serde_json::from_str(&super::strip_jsonc(&raw))
                    {
                        Ok(v) => v,
                        Err(e) => {
                            st.errors.push(StatError {
                                path: p.clone(),
                                error: format!("JSON: {e}"),
                            });
                            continue;
                        }
                    };
                    if kind == "mcp" {
                        mcp_rows(p, &v, &mut st.mcps);
                    } else {
                        hook_rows(p, &v, project_dir, &mut st.hooks);
                        mcp_rows(p, &v, &mut st.mcps);
                    }
                }
                _ => {
                    for f in doc_files(&path, &kind) {
                        match doc_stat(&f, &kind) {
                            Ok(d) => st.docs.push(d),
                            Err(e) => st.errors.push(StatError {
                                path: f.display().to_string(),
                                error: e,
                            }),
                        }
                    }
                }
            }
        }
    }
    for h in &st.hooks {
        let e = st.hooks_by_event.entry(h.event.clone()).or_default();
        e.total += 1;
        if h.enabled {
            e.enabled += 1;
        }
    }
    for m in &st.mcps {
        *st.mcp_by_category.entry(m.category.clone()).or_default() += 1;
    }
    st.totals = StatTotals {
        docs: st.docs.len(),
        doc_tokens: st.docs.iter().map(|d| d.tokens).sum(),
        doc_bytes: st.docs.iter().map(|d| d.bytes).sum(),
        hooks: st.hooks.len(),
        hooks_enabled: st.hooks.iter().filter(|h| h.enabled).count(),
        hooks_missing_program: st
            .hooks
            .iter()
            .filter(|h| h.program_found == Some(false) || h.script_found == Some(false))
            .count(),
        hooks_fake_env: st.hooks.iter().filter(|h| !h.fake_env.is_empty()).count(),
        mcps: st.mcps.len(),
        mcps_enabled: st.mcps.iter().filter(|m| m.enabled).count(),
    };
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_reasonable() {
        // ~ 9 words of plain English is ~ 10–13 tokens.
        let n = estimate_tokens("The quick brown fox jumps over the lazy dog.");
        assert!((9..=14).contains(&n), "{n}");
        let code = estimate_tokens("```rust\nfn main() { println!(\"hi\"); }\n```");
        assert!(code > 8, "{code}");
    }

    #[test]
    fn category_and_complexity() {
        assert_eq!(mcp_category("postgres", "npx", ""), "Database");
        assert_eq!(mcp_category("gmail", "npx", "send email"), "Other");
        let s = serde_json::json!({"command":"npx","args":["-y","x"],"env":{"A":"1","B":"2","C":"3","D":"4"}});
        assert_eq!(mcp_complexity(&s), 4);
    }
}
