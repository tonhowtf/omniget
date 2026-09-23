//! Raw catalog files → canonical [`Component`] (the catalog contract's
//! `parse_raw`). Lenient on purpose: 7 cct agents and 53 commands have no
//! frontmatter, descriptions carry `<example>` blocks, tools come as a string or
//! a list, and 64 "agents" are Copilot chatmodes.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::edit::{jsonc, yaml};
use super::model::*;
use super::{AgentkitError, Result};

/// Files of one component: path relative to the component folder → bytes.
pub type RawFiles = BTreeMap<String, Vec<u8>>;

const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;

fn perr(msg: impl Into<String>) -> AgentkitError {
    AgentkitError::new("AGENTKIT_PARSE", msg)
}

/// Splits `---` frontmatter off a Markdown text. No fence → no frontmatter.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = t
        .strip_prefix("---\n")
        .or_else(|| t.strip_prefix("---\r\n"))
    else {
        return (None, t);
    };
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, t)
}

/// Frontmatter as an object (empty when absent or unreadable) plus the body.
pub fn frontmatter(text: &str) -> (Map<String, Value>, &str) {
    let (front, body) = split_frontmatter(text);
    let map = front
        .and_then(|f| yaml::parse(f).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    (map, body)
}

/// Renders frontmatter + body (keys in the given order).
pub fn render_frontmatter(entries: &[(String, Value)], body: &str) -> String {
    if entries.is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    for (k, v) in entries {
        if v.is_null() {
            continue;
        }
        out.push_str(&yaml::entry_text(k, v, 0));
    }
    out.push_str("---\n");
    if !body.starts_with('\n') && !body.is_empty() {
        out.push('\n');
    }
    out.push_str(body);
    out
}

/// A list field that may be a YAML list or a comma/space separated string.
pub fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => s
            .split(|c: char| c == ',' || c == '\n')
            .flat_map(|part| {
                let p = part.trim();
                // `Bash(git:*)` keeps its spaces inside parentheses
                if p.contains('(') {
                    vec![p.to_string()]
                } else {
                    p.split_whitespace().map(str::to_string).collect()
                }
            })
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Null => None,
                other => Some(other.to_string()),
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => vec![],
    }
}

fn str_field(m: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match m.get(*k) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

/// Description for cards: `<example>` blocks removed, whitespace folded, ≤ 300 chars.
pub fn short_description(desc: &str) -> String {
    let mut s = desc.to_string();
    for tag in ["example", "commentary"] {
        loop {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            let Some(a) = s.find(&open) else { break };
            match s[a..].find(&close) {
                Some(rel) => s.replace_range(a..a + rel + close.len(), " "),
                None => s.truncate(a),
            }
        }
    }
    let folded: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded = folded.trim_end_matches([':', ' ']).to_string();
    if folded.chars().count() <= 300 {
        return folded;
    }
    let mut cut: String = folded.chars().take(297).collect();
    if let Some(sp) = cut.rfind(' ') {
        cut.truncate(sp);
    }
    format!("{cut}…")
}

fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let file = file
        .strip_suffix(".md")
        .or_else(|| file.strip_suffix(".json"))
        .or_else(|| file.strip_suffix(".toml"))
        .or_else(|| file.strip_suffix(".yaml"))
        .or_else(|| file.strip_suffix(".yml"))
        .unwrap_or(file);
    file.strip_suffix(".agent")
        .or_else(|| file.strip_suffix(".prompt"))
        .or_else(|| file.strip_suffix(".chatmode"))
        .unwrap_or(file)
        .to_string()
}

fn text_of<'a>(files: &'a RawFiles, path: &str) -> Result<&'a str> {
    let bytes = files
        .get(path)
        .ok_or_else(|| perr(format!("entry `{path}` is not among the files")))?;
    std::str::from_utf8(bytes).map_err(|_| perr(format!("`{path}` is not UTF-8")))
}

fn json_of(files: &RawFiles, path: &str) -> Result<Value> {
    jsonc::parse_value(text_of(files, path)?).map_err(|e| perr(format!("`{path}`: {}", e.message)))
}

fn is_executable_name(p: &str) -> bool {
    p.ends_with(".sh")
        || p.ends_with(".py")
        || p.ends_with(".js") && p.contains("hooks/")
        || p.ends_with(".bash")
        || p.ends_with(".zsh")
}

/// VS Code / Copilot tool ids that mark an "agent" as a Copilot chatmode.
const COPILOT_TOOLS: &[&str] = &[
    "codebase",
    "editFiles",
    "runCommands",
    "search/codebase",
    "githubRepo",
    "problems",
    "usages",
    "terminalLastCommand",
    "terminalSelection",
    "findTestFiles",
    "runTests",
    "testFailure",
    "changes",
    "vscodeAPI",
    "extensions",
    "openSimpleBrowser",
    "new",
    "runNotebooks",
    "searchResults",
];

/// Parses one catalog item into the canonical shape (the catalog contract).
/// `entry_path` is relative to the component folder and must be in `files`.
pub fn parse_raw(kind: ComponentKind, entry_path: &str, files: &RawFiles) -> Result<Component> {
    let total: usize = files.values().map(|b| b.len()).sum();
    if total > MAX_COMPONENT_BYTES {
        return Err(perr(format!("component is {total} bytes, over the cap")));
    }
    let raw_files: Vec<ComponentFile> = {
        let mut v: Vec<ComponentFile> = files
            .iter()
            .map(|(p, b)| ComponentFile {
                path: p.clone(),
                bytes: b.clone(),
                executable: is_executable_name(p),
            })
            .collect();
        v.sort_by_key(|f| if f.path == entry_path { 0 } else { 1 });
        v
    };
    let (name, description, body, origin) = match kind {
        ComponentKind::Agent => parse_agent(entry_path, files)?,
        ComponentKind::Command => parse_command(entry_path, files)?,
        ComponentKind::Rule => parse_rule(entry_path, files)?,
        ComponentKind::Skill => parse_skill(entry_path, files)?,
        ComponentKind::Mcp => parse_mcp(entry_path, files)?,
        ComponentKind::Hook => parse_hook(entry_path, files)?,
        ComponentKind::Setting => parse_setting(entry_path, files)?,
        ComponentKind::Statusline => parse_statusline(entry_path, files)?,
        ComponentKind::Loop => parse_loop(entry_path, files)?,
        ComponentKind::Workflow => parse_workflow(entry_path, files)?,
        ComponentKind::Mod => parse_mod(entry_path, files)?,
        ComponentKind::Plugin => parse_plugin(entry_path, files)?,
        ComponentKind::ProjectTemplate => parse_template(entry_path, files)?,
        ComponentKind::SandboxRecipe => parse_sandbox(entry_path, files)?,
        ComponentKind::Stack => parse_stack(entry_path, files)?,
    };
    let mut c = Component {
        id: format!("local:{}/{}", kind.as_str(), name),
        kind,
        name,
        category: None,
        description,
        source: None,
        license: None,
        author: None,
        sha256: String::new(),
        origin_tool: origin,
        security: None,
        compat: BTreeMap::new(),
        body,
        files: raw_files,
        entry: entry_path.to_string(),
    };
    c.rehash();
    Ok(c)
}

type Parsed = (String, String, ComponentBody, String);

fn parse_agent(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let (mut fm, body) = frontmatter(text);
    let name = str_field(&fm, &["name"]).unwrap_or_else(|| stem(entry));
    let description = str_field(&fm, &["description"]).unwrap_or_default();
    let tools = string_list(fm.get("tools"));
    let origin = if entry.ends_with(".chatmode.md")
        || entry.ends_with(".agent.md")
        || tools.iter().any(|t| COPILOT_TOOLS.contains(&t.as_str()))
    {
        "copilot"
    } else {
        "claude"
    };
    let mut spec = AgentSpec {
        name: name.clone(),
        description: description.clone(),
        prompt: body.to_string(),
        tools,
        disallowed_tools: string_list(
            fm.get("disallowedTools")
                .or(fm.get("disallowed_tools"))
                .or(fm.get("disallowed-tools")),
        ),
        model: str_field(&fm, &["model"]),
        permission_mode: str_field(
            &fm,
            &["permissionMode", "permission_mode", "permission-mode"],
        ),
        max_turns: fm
            .get("maxTurns")
            .or(fm.get("max_turns"))
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .map(|n| n as u32),
        skills: string_list(fm.get("skills")),
        mcp_servers: fm
            .get("mcpServers")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default(),
        hooks: fm.get("hooks").cloned().unwrap_or(Value::Null),
        color: str_field(&fm, &["color"]),
        background: fm.get("background").and_then(|v| v.as_bool()),
        effort: str_field(&fm, &["effort"]),
        isolation: str_field(&fm, &["isolation"]),
        memory: str_field(&fm, &["memory"]),
        initial_prompt: str_field(&fm, &["initialPrompt", "initial_prompt"]),
        extra: Map::new(),
    };
    for k in [
        "name",
        "description",
        "tools",
        "disallowedTools",
        "disallowed_tools",
        "disallowed-tools",
        "model",
        "permissionMode",
        "permission_mode",
        "permission-mode",
        "maxTurns",
        "max_turns",
        "skills",
        "mcpServers",
        "hooks",
        "color",
        "background",
        "effort",
        "isolation",
        "memory",
        "initialPrompt",
        "initial_prompt",
    ] {
        fm.remove(k);
    }
    spec.extra = fm;
    Ok((
        name,
        short_description(&description),
        ComponentBody::Agent(spec),
        origin.into(),
    ))
}

fn parse_command(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let (mut fm, body) = frontmatter(text);
    let name = stem(entry);
    let description =
        str_field(&fm, &["description"]).unwrap_or_else(|| first_heading(body).unwrap_or_default());
    let spec = CommandSpec {
        description: description.clone(),
        argument_hint: str_field(&fm, &["argument-hint", "argument_hint", "argumentHint"]),
        allowed_tools: string_list(
            fm.get("allowed-tools")
                .or(fm.get("allowed_tools"))
                .or(fm.get("allowedTools")),
        ),
        model: str_field(&fm, &["model"]),
        agent: str_field(&fm, &["agent"]),
        body: placeholders::to_canonical(body),
        extra: {
            for k in [
                "description",
                "argument-hint",
                "argument_hint",
                "argumentHint",
                "allowed-tools",
                "allowed_tools",
                "allowedTools",
                "model",
                "agent",
            ] {
                fm.remove(k);
            }
            fm
        },
    };
    let origin = if entry.ends_with(".prompt.md") {
        "copilot"
    } else {
        "claude"
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Command(spec),
        origin.into(),
    ))
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix('#')
                .map(|h| h.trim_start_matches('#').trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

fn parse_rule(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let (fm, body) = frontmatter(text);
    let name = stem(entry);
    let globs = {
        let mut g = string_list(
            fm.get("globs")
                .or(fm.get("paths"))
                .or(fm.get("applyTo"))
                .or(fm.get("fileMatchPattern")),
        );
        g.retain(|x| !x.is_empty());
        g
    };
    let always = fm
        .get("alwaysApply")
        .or(fm.get("always_apply"))
        .and_then(|v| v.as_bool())
        .unwrap_or(globs.is_empty());
    let description =
        str_field(&fm, &["description"]).unwrap_or_else(|| first_heading(body).unwrap_or_default());
    let spec = RuleSpec {
        markdown: body.to_string(),
        description: description.clone(),
        scope: if globs.is_empty() {
            RuleScope::Root
        } else {
            RuleScope::Scoped
        },
        globs,
        always_apply: always,
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Rule(spec),
        "claude".into(),
    ))
}

fn parse_skill(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let (fm, _) = frontmatter(text);
    let folder = entry
        .rsplit_once('/')
        .map(|(d, _)| d.rsplit('/').next().unwrap_or(d).to_string());
    let name = str_field(&fm, &["name"])
        .or(folder.clone())
        .ok_or_else(|| perr("skill has no name"))?;
    let description = str_field(&fm, &["description"]).unwrap_or_default();
    let dir_name = sanitize_name(&name);
    let spec = SkillSpec {
        name: name.clone(),
        description: description.clone(),
        allowed_tools: string_list(fm.get("allowed-tools").or(fm.get("allowed_tools"))),
        dir_name,
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Skill(spec),
        "claude".into(),
    ))
}

/// Lowercase, `[a-z0-9-_.]` only, for file and folder names we create.
pub fn sanitize_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches(['-', '.']).to_string();
    if out.is_empty() {
        "component".into()
    } else {
        out
    }
}

fn parse_mcp(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let v = json_of(files, entry)?;
    let servers_obj = v
        .get("mcpServers")
        .or_else(|| v.get("servers"))
        .or_else(|| v.get("mcp"))
        .or_else(|| v.get("context_servers"))
        .and_then(|x| x.as_object())
        .cloned();
    let mut servers = Vec::new();
    match servers_obj {
        Some(m) => {
            for (name, rec) in m {
                servers.push(mcp_from_record(&name, &rec));
            }
        }
        None => {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| stem(entry));
            servers.push(mcp_from_record(&name, &v));
        }
    }
    if servers.is_empty() {
        return Err(perr("MCP file has no servers"));
    }
    let name = if servers.len() == 1 {
        servers[0].name.clone()
    } else {
        stem(entry)
    };
    let description = servers
        .iter()
        .map(|s| s.description.clone())
        .find(|d| !d.is_empty())
        .unwrap_or_default();
    Ok((
        name,
        short_description(&description),
        ComponentBody::Mcp(McpSpec { servers }),
        "claude".into(),
    ))
}

/// Reads one server record in any dialect (Claude/Cursor/VS Code/OpenCode/Goose/
/// Windsurf/Gemini/Codex-as-JSON) into the canonical record.
pub fn mcp_from_record(name: &str, rec: &Value) -> McpServer {
    let mut extra = rec.as_object().cloned().unwrap_or_default();
    let take = |m: &mut Map<String, Value>, k: &str| m.remove(k);
    let ty = take(&mut extra, "type")
        .or_else(|| take(&mut extra, "transport"))
        .and_then(|v| v.as_str().map(str::to_lowercase));
    let description = take(&mut extra, "description")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    // command: string or array (OpenCode), `cmd` (Goose), `{path,args}` (old Zed)
    let (mut command, mut args): (Option<String>, Vec<String>) = (None, vec![]);
    match take(&mut extra, "command").or_else(|| take(&mut extra, "cmd")) {
        Some(Value::String(s)) => command = Some(s),
        Some(Value::Array(a)) => {
            let parts: Vec<String> = a
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            if let Some((c, rest)) = parts.split_first() {
                command = Some(c.clone());
                args = rest.to_vec();
            }
        }
        Some(Value::Object(o)) => {
            command = o.get("path").and_then(|p| p.as_str()).map(str::to_string);
            args = string_list(o.get("args"));
        }
        _ => {}
    }
    if let Some(a) = take(&mut extra, "args") {
        args.extend(string_list(Some(&a)).into_iter().filter(|_| true));
        if let Value::Array(arr) = &a {
            // keep args with spaces intact
            args = arr
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
    }
    let env_v = take(&mut extra, "env")
        .or_else(|| take(&mut extra, "environment"))
        .or_else(|| take(&mut extra, "envs"));
    let mut env: BTreeMap<String, String> = env_v
        .and_then(|v| v.as_object().cloned())
        .map(|m| {
            m.into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let url = take(&mut extra, "url")
        .or_else(|| take(&mut extra, "httpUrl"))
        .or_else(|| take(&mut extra, "serverUrl"))
        .or_else(|| take(&mut extra, "uri"))
        .and_then(|v| v.as_str().map(str::to_string));
    let mut headers: BTreeMap<String, String> = take(&mut extra, "headers")
        .or_else(|| take(&mut extra, "http_headers"))
        .and_then(|v| v.as_object().cloned())
        .map(|m| {
            m.into_iter()
                .map(|(k, v)| (k, v.as_str().map(str::to_string).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let cwd = take(&mut extra, "cwd").and_then(|v| v.as_str().map(str::to_string));
    let oauth = take(&mut extra, "oauth");
    let bearer = take(&mut extra, "bearer_token_env_var")
        .or_else(|| take(&mut extra, "bearerTokenEnvVar"))
        .and_then(|v| v.as_str().map(str::to_string));
    for noise in ["enabled", "disabled", "name"] {
        extra.remove(noise);
    }
    let transport = match (ty.as_deref(), url.as_deref()) {
        (Some("sse"), _) => McpTransport::Sse,
        (Some(t), _) if t.contains("http") || t == "remote" => McpTransport::Http,
        (_, Some(u)) if command.is_none() => {
            if u.trim_end_matches('/').ends_with("/sse") {
                McpTransport::Sse
            } else {
                McpTransport::Http
            }
        }
        _ => McpTransport::Stdio,
    };
    let mut secrets: Vec<SecretRef> = Vec::new();
    let add_secret = |name: &str, original: Option<String>, secrets: &mut Vec<SecretRef>| {
        if !secrets.iter().any(|s| s.name == name) {
            secrets.push(SecretRef {
                name: name.to_string(),
                description: String::new(),
                original,
            });
        }
    };
    // env values
    for (k, v) in env.iter_mut() {
        let secretish = looks_secret_key(k);
        if let Some((_, orig)) = placeholder_secret(v) {
            add_secret(k, Some(orig), &mut secrets);
            *v = format!("{{{{secret:{k}}}}}");
        } else if secretish && !v.is_empty() {
            add_secret(k, Some(v.clone()), &mut secrets);
            *v = format!("{{{{secret:{k}}}}}");
        }
    }
    // headers, url, args: replace placeholders inside the string
    for v in headers.values_mut() {
        *v = replace_placeholders(v, &mut secrets);
    }
    let url = url.map(|u| replace_placeholders(&u, &mut secrets));
    let args: Vec<String> = args
        .into_iter()
        .map(|a| replace_env_refs(&a, &mut secrets))
        .collect();
    if let Some(b) = bearer {
        headers.insert("Authorization".into(), format!("Bearer {{{{secret:{b}}}}}"));
        add_secret(&b, None, &mut secrets);
    }
    McpServer {
        name: name.to_string(),
        transport,
        command,
        args,
        env,
        cwd,
        url,
        headers,
        oauth,
        secrets,
        description,
        extra,
    }
}

fn looks_secret_key(k: &str) -> bool {
    let u = k.to_ascii_uppercase();
    [
        "TOKEN",
        "API_KEY",
        "APIKEY",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
        "AUTH",
    ]
    .iter()
    .any(|w| u.contains(w))
}

fn upper_snake(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let t = out.trim_matches('_').to_string();
    t.strip_prefix("YOUR_").map(str::to_string).unwrap_or(t)
}

/// A whole-value placeholder: `${VAR}`, `$VAR`, `${env:VAR}`, `${input:x}`,
/// `<YOUR_TOKEN>`, `{{secret:X}}` → (NAME, original).
pub fn placeholder_secret(v: &str) -> Option<(String, String)> {
    let t = v.trim();
    if let Some(inner) = t
        .strip_prefix("{{secret:")
        .and_then(|x| x.strip_suffix("}}"))
    {
        return Some((inner.to_string(), t.to_string()));
    }
    if let Some(inner) = t.strip_prefix("${").and_then(|x| x.strip_suffix('}')) {
        let name = inner
            .strip_prefix("env:")
            .or_else(|| inner.strip_prefix("input:"))
            .unwrap_or(inner);
        return Some((upper_snake(name), t.to_string()));
    }
    if let Some(inner) = t.strip_prefix('$') {
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Some((inner.to_string(), t.to_string()));
        }
    }
    if t.starts_with('<') && t.ends_with('>') && t.len() > 2 && !t.contains(' ') {
        return Some((upper_snake(&t[1..t.len() - 1]), t.to_string()));
    }
    None
}

/// Replaces `${…}` and `<…>` placeholders inside a string with `{{secret:NAME}}`.
fn replace_placeholders(s: &str, secrets: &mut Vec<SecretRef>) -> String {
    let with_env = replace_env_refs(s, secrets);
    let mut out = String::new();
    let mut rest = with_env.as_str();
    while let Some(a) = rest.find('<') {
        let Some(rel) = rest[a..].find('>') else {
            break;
        };
        let token = &rest[a..a + rel + 1];
        out.push_str(&rest[..a]);
        match placeholder_secret(token) {
            Some((name, orig)) if !name.is_empty() => {
                if !secrets.iter().any(|x| x.name == name) {
                    secrets.push(SecretRef {
                        name: name.clone(),
                        description: String::new(),
                        original: Some(orig),
                    });
                }
                out.push_str(&format!("{{{{secret:{name}}}}}"));
            }
            _ => out.push_str(token),
        }
        rest = &rest[a + rel + 1..];
    }
    out.push_str(rest);
    out
}

fn replace_env_refs(s: &str, secrets: &mut Vec<SecretRef>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find("${") {
        let Some(rel) = rest[a..].find('}') else {
            break;
        };
        let token = &rest[a..a + rel + 1];
        out.push_str(&rest[..a]);
        match placeholder_secret(token) {
            Some((name, orig))
                if !name.is_empty()
                    && !token.contains("workspaceFolder")
                    && !token.contains("userHome") =>
            {
                if !secrets.iter().any(|x| x.name == name) {
                    secrets.push(SecretRef {
                        name: name.clone(),
                        description: String::new(),
                        original: Some(orig),
                    });
                }
                out.push_str(&format!("{{{{secret:{name}}}}}"));
            }
            _ => out.push_str(token),
        }
        rest = &rest[a + rel + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads a Claude hook record `{Event: [{matcher, hooks: [handler]}]}`.
pub fn hook_entries(hooks: &Value) -> Vec<HookEntry> {
    let mut out = Vec::new();
    let Some(events) = hooks.as_object() else {
        return out;
    };
    for (event, groups) in events {
        let groups: Vec<Value> = match groups {
            Value::Array(a) => a.clone(),
            Value::String(s) => vec![
                serde_json::json!({"matcher": "*", "hooks": [{"type": "command", "command": s}]}),
            ],
            other => vec![other.clone()],
        };
        for g in groups {
            let matcher = g
                .get("matcher")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .filter(|m| !m.is_empty());
            let handlers = g
                .get("hooks")
                .and_then(|h| h.as_array())
                .cloned()
                .unwrap_or_else(|| {
                    // flat shape (Cursor/Copilot): the group is the handler
                    vec![g.clone()]
                });
            for h in handlers {
                out.push(HookEntry {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    handler: handler_from(&h),
                });
            }
        }
    }
    out
}

fn handler_from(h: &Value) -> HookHandler {
    let mut extra = h.as_object().cloned().unwrap_or_default();
    let kind = extra
        .remove("type")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "command".into());
    let command = extra
        .remove("command")
        .or_else(|| extra.remove("bash"))
        .and_then(|v| v.as_str().map(str::to_string));
    let url = extra
        .remove("url")
        .and_then(|v| v.as_str().map(str::to_string));
    let prompt = extra
        .remove("prompt")
        .and_then(|v| v.as_str().map(str::to_string));
    let timeout = extra
        .remove("timeout")
        .or_else(|| extra.remove("timeoutSec"))
        .and_then(|v| v.as_f64());
    extra.remove("matcher");
    HookHandler {
        kind,
        command,
        url,
        prompt,
        timeout,
        extra,
    }
}

/// Support files: `supportingFiles` entries plus same-basename `.py`/`.sh` next
/// to the hook JSON (the original CLI's probe), both with their destination.
fn support_files(v: &Value, entry: &str, files: &RawFiles, default_dir: &str) -> Vec<SupportFile> {
    let mut out: Vec<SupportFile> = Vec::new();
    if let Some(list) = v.get("supportingFiles").and_then(|x| x.as_array()) {
        for f in list {
            let (Some(src), Some(dst)) = (
                f.get("source").and_then(|s| s.as_str()),
                f.get("destination").and_then(|s| s.as_str()),
            ) else {
                continue;
            };
            out.push(SupportFile {
                source: src.to_string(),
                destination: dst.to_string(),
                executable: f
                    .get("executable")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false),
            });
        }
    }
    let base = stem(entry);
    let dir = entry
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    for ext in ["py", "sh", "js"] {
        let p = format!("{dir}{base}.{ext}");
        if files.contains_key(&p)
            && !out
                .iter()
                .any(|s| s.source == p || s.source.ends_with(&format!("{base}.{ext}")))
        {
            out.push(SupportFile {
                source: p,
                destination: format!("{default_dir}/{base}.{ext}"),
                executable: ext != "js",
            });
        }
    }
    out
}

fn parse_hook(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let v = json_of(files, entry)?;
    let hooks = v.get("hooks").cloned().unwrap_or(Value::Null);
    let entries = hook_entries(&hooks);
    if entries.is_empty() {
        return Err(perr("hook file has no hooks"));
    }
    let description = v
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or_default()
        .to_string();
    let spec = HookSpec {
        entries,
        supporting_files: support_files(&v, entry, files, ".claude/hooks"),
        tags: string_list(v.get("tags")),
    };
    Ok((
        stem(entry),
        short_description(&description),
        ComponentBody::Hook(spec),
        "claude".into(),
    ))
}

fn script_sidecars(command: &str, entry: &str, files: &RawFiles) -> Vec<SupportFile> {
    // `.claude/scripts/<x>.py` referenced by the command and shipped next to the JSON
    let mut out = Vec::new();
    let dir = entry
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    for marker in [".claude/scripts/", ".claude/hooks/"] {
        let mut rest = command;
        while let Some(a) = rest.find(marker) {
            let tail = &rest[a + marker.len()..];
            let end = tail
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(tail.len());
            let file = &tail[..end];
            let src = format!("{dir}{file}");
            if !file.is_empty()
                && files.contains_key(&src)
                && !out.iter().any(|s: &SupportFile| s.source == src)
            {
                out.push(SupportFile {
                    source: src,
                    destination: format!("{marker}{file}"),
                    executable: true,
                });
            }
            rest = &tail[end..];
        }
    }
    out
}

fn parse_setting(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let v = json_of(files, entry)?;
    let mut values = v
        .as_object()
        .cloned()
        .ok_or_else(|| perr("setting is not a JSON object"))?;
    let description = values
        .remove("description")
        .and_then(|d| d.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut supporting = support_files(&v, entry, files, ".claude/scripts");
    if let Some(Value::Object(fm)) = values.remove("files") {
        for (path, meta) in fm {
            let exec = meta
                .get("executable")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);
            supporting.push(SupportFile {
                source: format!("inline:{path}"),
                destination: path,
                executable: exec,
            });
        }
    }
    values.remove("supportingFiles");
    if let Some(cmd) = values
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
    {
        for s in script_sidecars(cmd, entry, files) {
            if !supporting.iter().any(|x| x.source == s.source) {
                supporting.push(s);
            }
        }
    }
    // a same-name sidecar the settings reference is already covered; drop others
    supporting.retain(|s| s.source.starts_with("inline:") || files.contains_key(&s.source));
    Ok((
        stem(entry),
        short_description(&description),
        ComponentBody::Setting(SettingSpec {
            values,
            supporting_files: supporting,
        }),
        "claude".into(),
    ))
}

fn parse_statusline(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let v = json_of(files, entry)?;
    let sl = v
        .get("statusLine")
        .or_else(|| v.get("statusline"))
        .cloned()
        .unwrap_or_else(|| v.clone());
    let command = sl
        .get("command")
        .and_then(|c| c.as_str())
        .ok_or_else(|| perr("statusline has no command"))?
        .to_string();
    let description = v
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or_default()
        .to_string();
    let mut extra = v.as_object().cloned().unwrap_or_default();
    for k in [
        "statusLine",
        "statusline",
        "description",
        "files",
        "supportingFiles",
    ] {
        extra.remove(k);
    }
    let fields = [
        "model.display_name",
        "model.id",
        "workspace.current_dir",
        "workspace.project_dir",
        "cost.total_cost_usd",
        "session_id",
        "transcript_path",
        "output_style.name",
        "version",
    ]
    .iter()
    .filter(|f| {
        let parts: Vec<&str> = f.split('.').collect();
        parts.iter().all(|p| command.contains(p))
            || files.values().any(|b| {
                std::str::from_utf8(b)
                    .map(|t| parts.iter().all(|p| t.contains(p)))
                    .unwrap_or(false)
            })
    })
    .map(|s| s.to_string())
    .collect();
    let mut supporting = script_sidecars(&command, entry, files);
    for s in support_files(&v, entry, files, ".claude/scripts") {
        if !supporting.iter().any(|x| x.source == s.source) {
            supporting.push(s);
        }
    }
    supporting.retain(|s| files.contains_key(&s.source));
    let spec = StatuslineSpec {
        command,
        padding: sl.get("padding").and_then(|p| p.as_i64()),
        supporting_files: supporting,
        stdin_fields: fields,
        extra,
    };
    Ok((
        stem(entry),
        short_description(&description),
        ComponentBody::Statusline(spec),
        "claude".into(),
    ))
}

fn parse_loop(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let (fm, body) = frontmatter(text);
    let name = str_field(&fm, &["name"]).unwrap_or_else(|| stem(entry));
    let description = str_field(&fm, &["description"]).unwrap_or_default();
    let components = string_list(fm.get("components"))
        .into_iter()
        .map(|c| {
            c.trim_end_matches(".md")
                .trim_end_matches(".json")
                .to_string()
        })
        .collect();
    let spec = LoopSpec {
        goal: description.clone(),
        interval: str_field(&fm, &["interval"]),
        stop_condition: str_field(&fm, &["stop-condition", "stop_condition", "stopCondition"]),
        budget: str_field(&fm, &["budget"]),
        components,
        body: body.to_string(),
        tags: string_list(fm.get("tags")),
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Loop(spec),
        "claude".into(),
    ))
}

fn parse_workflow(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let v = if entry.ends_with(".json") {
        jsonc::parse_value(text)?
    } else {
        yaml::parse(text)?
    };
    let name = v
        .get("name")
        .or_else(|| v.get("metadata").and_then(|m| m.get("name")))
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem(entry));
    let description = v
        .get("description")
        .or_else(|| v.get("metadata").and_then(|m| m.get("description")))
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    let steps = v
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .map(|s| WorkflowStep {
                    kind: s
                        .get("type")
                        .or_else(|| s.get("kind"))
                        .and_then(|k| k.as_str())
                        .unwrap_or("prompt")
                        .to_string(),
                    reference: s
                        .get("name")
                        .or_else(|| s.get("ref"))
                        .or_else(|| s.get("component"))
                        .and_then(|k| k.as_str())
                        .map(str::to_string),
                    prompt: s
                        .get("prompt")
                        .or_else(|| s.get("description"))
                        .and_then(|k| k.as_str())
                        .map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    let yaml_text = (!entry.ends_with(".json")).then(|| text.to_string());
    Ok((
        name,
        short_description(&description),
        ComponentBody::Workflow(WorkflowSpec {
            steps,
            yaml: yaml_text,
        }),
        "claude".into(),
    ))
}

fn plugin_manifest(entry: &str, files: &RawFiles) -> Result<(Value, String)> {
    let manifest_path = if entry.ends_with("plugin.json") {
        entry.to_string()
    } else {
        files
            .keys()
            .find(|k| k.ends_with(".claude-plugin/plugin.json"))
            .cloned()
            .ok_or_else(|| perr("no .claude-plugin/plugin.json"))?
    };
    let prefix = manifest_path
        .trim_end_matches("plugin.json")
        .trim_end_matches(".claude-plugin/")
        .to_string();
    Ok((json_of(files, &manifest_path)?, prefix))
}

fn parse_mod(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let (manifest, prefix) = plugin_manifest(entry, files)?;
    let name = manifest
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .ok_or_else(|| perr("plugin.json has no name"))?;
    let hooks_path = format!("{prefix}hooks/hooks.json");
    let modules = files
        .get(&hooks_path)
        .and_then(|_| json_of(files, &hooks_path).ok())
        .map(|h| string_list(h.get("modules")))
        .unwrap_or_default();
    if modules.is_empty() {
        return Err(perr("mod has no hooks/hooks.json modules"));
    }
    let description = manifest
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or_default()
        .to_string();
    let spec = ModSpec {
        dir_name: sanitize_name(&name),
        plugin_name: name.clone(),
        version: manifest
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        modules,
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Mod(spec),
        "claude".into(),
    ))
}

fn parse_plugin(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let (manifest, prefix) = plugin_manifest(entry, files)?;
    let name = manifest
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .ok_or_else(|| perr("plugin.json has no name"))?;
    let mut contents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in files.keys() {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        let top = rel.split('/').next().unwrap_or("");
        let key = match top {
            "agents" | "commands" | "skills" | "hooks" | "output-styles" | "workflows" => {
                top.to_string()
            }
            ".mcp.json" => "mcp".to_string(),
            _ => continue,
        };
        contents.entry(key).or_default().push(rel.to_string());
    }
    let description = manifest
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or_default()
        .to_string();
    let spec = PluginSpec {
        dir_name: sanitize_name(&name),
        name: name.clone(),
        version: manifest
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        manifest,
        contents,
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Plugin(spec),
        "claude".into(),
    ))
}

fn parse_template(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let text = text_of(files, entry)?;
    let v = if entry.ends_with(".json") {
        jsonc::parse_value(text)?
    } else {
        Value::Object(frontmatter(text).0)
    };
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem(entry));
    let description = v
        .get("description")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    let agents_md = if entry.ends_with(".md") {
        Some(frontmatter(text).1.to_string())
    } else {
        v.get("agents_md")
            .and_then(|a| a.as_str())
            .map(str::to_string)
    };
    let spec = TemplateSpec {
        detect: v.get("detect").cloned().unwrap_or(Value::Null),
        components: string_list(v.get("components")),
        agents_md,
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::ProjectTemplate(spec),
        "claude".into(),
    ))
}

fn parse_sandbox(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let provider = ["docker", "e2b", "cloudflare"]
        .iter()
        .find(|p| entry.contains(*p) || files.keys().any(|k| k.contains(*p)))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "docker".into());
    let description = text_of(files, entry)
        .ok()
        .map(|t| first_heading(t).unwrap_or_default())
        .unwrap_or_default();
    Ok((
        format!("{provider}-sandbox"),
        description,
        ComponentBody::SandboxRecipe(SandboxSpec {
            provider,
            entry: Some(entry.to_string()),
        }),
        "claude".into(),
    ))
}

fn parse_stack(entry: &str, files: &RawFiles) -> Result<Parsed> {
    let v = json_of(files, entry)?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem(entry));
    let description = v
        .get("description")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    let spec = StackSpec {
        components: string_list(v.get("components")),
        targets: string_list(v.get("targets")),
        scope: v.get("scope").and_then(|s| s.as_str()).map(str::to_string),
    };
    Ok((
        name,
        short_description(&description),
        ComponentBody::Stack(spec),
        "claude".into(),
    ))
}

/// Parses a catalog index item (contract `catalog-index.json`) with its files,
/// filling id, category, source, license, author and tags from the item.
pub fn parse_item(item: &Value, files: &RawFiles) -> Result<Component> {
    let kind = item
        .get("kind")
        .and_then(|k| k.as_str())
        .and_then(ComponentKind::parse)
        .ok_or_else(|| perr("item has no known kind"))?;
    let entry = item
        .get("entry")
        .and_then(|e| e.as_str())
        .ok_or_else(|| perr("item has no entry"))?;
    let mut c = parse_raw(kind, entry, files)?;
    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
        c.id = id.to_string();
    }
    if let Some(n) = item.get("name").and_then(|v| v.as_str()) {
        if kind != ComponentKind::Mcp || c.name.is_empty() {
            c.name = n.to_string();
        }
    }
    c.category = item
        .get("category")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(d) = item
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|d| !d.is_empty())
    {
        c.description = d.to_string();
    }
    c.license = item
        .get("license")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    c.author = item
        .get("author")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(o) = item.get("origin_tool").and_then(|v| v.as_str()) {
        c.origin_tool = o.to_string();
    }
    if let Some(src) = item.get("source") {
        c.source = serde_json::from_value(src.clone()).ok();
    }
    c.security = item.get("security").cloned().filter(|v| !v.is_null());
    Ok(c)
}

/// Reads a component from disk: a single file (agent/command/rule/loop/hook/
/// setting/mcp) with the sibling files it may reference, or a folder (skill,
/// mod, plugin). Returns the entry path relative to the component root.
pub fn load_path(kind: ComponentKind, path: &Path) -> Result<(String, RawFiles)> {
    let io = |p: &Path, e: std::io::Error| AgentkitError::io("reading", p, &e);
    let mut files = RawFiles::new();
    if path.is_dir() {
        let entry = match kind {
            ComponentKind::Skill => "SKILL.md".to_string(),
            ComponentKind::Mod | ComponentKind::Plugin => ".claude-plugin/plugin.json".to_string(),
            _ => {
                return Err(perr(format!(
                    "{} is a folder; a {kind} is a file",
                    path.display()
                )))
            }
        };
        let mut total = 0usize;
        for e in walkdir::WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            if !e.file_type().is_file() {
                continue;
            }
            let rel = e.path().strip_prefix(path).unwrap_or(e.path());
            let rel_s = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            if rel_s.split('/').any(|p| p == ".git" || p == "node_modules") {
                continue;
            }
            let bytes = std::fs::read(e.path()).map_err(|er| io(e.path(), er))?;
            total += bytes.len();
            if total > MAX_COMPONENT_BYTES {
                return Err(perr("component folder is too big"));
            }
            files.insert(rel_s, bytes);
        }
        if !files.contains_key(&entry) {
            return Err(perr(format!("{} has no {entry}", path.display())));
        }
        return Ok((entry, files));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| perr("bad path"))?;
    files.insert(name.clone(), std::fs::read(path).map_err(|e| io(path, e))?);
    let dir = path.parent().unwrap_or(Path::new("."));
    let base = stem(&name);
    // same-basename scripts and a same-name folder (supportingFiles)
    for ext in ["py", "sh", "js"] {
        let p = dir.join(format!("{base}.{ext}"));
        if p.is_file() {
            files.insert(
                format!("{base}.{ext}"),
                std::fs::read(&p).map_err(|e| io(&p, e))?,
            );
        }
    }
    if matches!(
        kind,
        ComponentKind::Hook | ComponentKind::Setting | ComponentKind::Statusline
    ) {
        if let Ok(v) = jsonc::parse_value(std::str::from_utf8(&files[&name]).unwrap_or("")) {
            let mut sources: Vec<String> = v
                .get("supportingFiles")
                .and_then(|s| s.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|f| {
                            f.get("source").and_then(|s| s.as_str()).map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(cmd) = v
                .get("statusLine")
                .and_then(|s| s.get("command"))
                .and_then(|c| c.as_str())
            {
                for marker in [".claude/scripts/", ".claude/hooks/"] {
                    let mut rest = cmd;
                    while let Some(a) = rest.find(marker) {
                        let tail = &rest[a + marker.len()..];
                        let end = tail
                            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                            .unwrap_or(tail.len());
                        sources.push(tail[..end].to_string());
                        rest = &tail[end..];
                    }
                }
            }
            for s in sources {
                if s.contains("..") {
                    continue;
                }
                let p = dir.join(&s);
                if p.is_file() {
                    files.insert(s.clone(), std::fs::read(&p).map_err(|e| io(&p, e))?);
                }
            }
        }
    }
    Ok((name, files))
}

/// [`load_path`] + [`parse_raw`].
pub fn parse_path(kind: ComponentKind, path: &Path) -> Result<Component> {
    let (entry, files) = load_path(kind, path)?;
    let mut c = parse_raw(kind, &entry, &files)?;
    c.source = Some(SourceRef {
        id: "local".into(),
        path: Some(path.display().to_string()),
        ..Default::default()
    });
    Ok(c)
}

/// Neutral command placeholders and the conversion to/from each dialect.
pub mod placeholders {
    use std::collections::BTreeMap;

    /// Claude dialect → neutral markers. Exact inverse of [`to_claude`].
    pub fn to_canonical(body: &str) -> String {
        let mut out = String::with_capacity(body.len());
        let b = body.as_bytes();
        let mut i = 0;
        let mut in_fence = false;
        let mut line_start = true;
        while i < body.len() {
            if line_start && (body[i..].starts_with("```") || body[i..].starts_with("~~~")) {
                in_fence = !in_fence;
            }
            let c = b[i];
            line_start = c == b'\n';
            if body[i..].starts_with("$ARGUMENTS") {
                out.push_str("{{args}}");
                i += "$ARGUMENTS".len();
                continue;
            }
            if c == b'$'
                && i + 1 < b.len()
                && (b'1'..=b'9').contains(&b[i + 1])
                && (i + 2 >= b.len() || !b[i + 2].is_ascii_digit())
                && !in_fence
            {
                out.push_str(&format!("{{{{arg:{}}}}}", b[i + 1] as char));
                i += 2;
                continue;
            }
            if c == b'!' && body[i..].starts_with("!`") {
                if let Some(rel) = body[i + 2..].find('`') {
                    let cmd = &body[i + 2..i + 2 + rel];
                    if !cmd.contains('\n') && !cmd.contains("}}") {
                        out.push_str(&format!("{{{{shell:{cmd}}}}}"));
                        i += 3 + rel;
                        continue;
                    }
                }
            }
            if c == b'@'
                && !in_fence
                && (i == 0 || b[i - 1].is_ascii_whitespace() || b[i - 1] == b'(')
            {
                let rest = &body[i + 1..];
                let end = rest
                    .find(|ch: char| {
                        ch.is_whitespace()
                            || ch == ')'
                            || ch == ','
                            || ch == '`'
                            || ch == '"'
                            || ch == '\''
                    })
                    .unwrap_or(rest.len());
                let mut path = &rest[..end];
                path = path.trim_end_matches(['.', ':', ';']);
                if !path.is_empty()
                    && (path.contains('/') || path.contains('.'))
                    && !path.contains("}}")
                    && !path.starts_with('{')
                {
                    out.push_str(&format!("{{{{file:{path}}}}}"));
                    i += 1 + path.len();
                    continue;
                }
            }
            let ch = body[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
        out
    }

    /// Neutral markers → Claude dialect.
    pub fn to_claude(body: &str) -> String {
        let map: BTreeMap<String, String> = [
            ("args", "$ARGUMENTS"),
            ("arg", "$N"),
            ("shell", "!`{cmd}`"),
            ("file", "@{path}"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        render(body, &map).0
    }

    /// Neutral markers → a target dialect given its `placeholder_map`
    /// (`args`, `arg` with `N`, `shell` with `{cmd}`, `file` with `{path}`).
    /// Returns the text and what could not be expressed.
    pub fn render(body: &str, map: &BTreeMap<String, String>) -> (String, Vec<String>) {
        let mut out = String::with_capacity(body.len());
        let mut lost: Vec<String> = Vec::new();
        let mut rest = body;
        while let Some(a) = rest.find("{{") {
            out.push_str(&rest[..a]);
            let after = &rest[a + 2..];
            let Some(close) = after.find("}}") else {
                out.push_str(&rest[a..]);
                rest = "";
                break;
            };
            let token = &after[..close];
            let (name, arg) = token
                .split_once(':')
                .map(|(n, v)| (n, Some(v)))
                .unwrap_or((token, None));
            let rendered = match (name, arg) {
                ("args", None) => Some(map.get("args").cloned().unwrap_or_default())
                    .map(|t| (t, "arguments placeholder")),
                ("arg", Some(n)) if n.chars().all(|c| c.is_ascii_digit()) => {
                    map.get("arg").map(|t| {
                        (
                            t.replace("${N}", &format!("${n}")).replace('N', n),
                            "positional arguments",
                        )
                    })
                }
                ("shell", Some(cmd)) => map
                    .get("shell")
                    .map(|t| (t.replace("{cmd}", cmd), "shell injection")),
                ("file", Some(p)) => map
                    .get("file")
                    .map(|t| (t.replace("{path}", p), "file references")),
                _ => None,
            };
            match rendered {
                Some((t, what)) if !t.is_empty() || name == "args" => {
                    if t.is_empty() {
                        push_unique(&mut lost, what);
                    }
                    out.push_str(&t);
                }
                Some((_, what)) => {
                    push_unique(&mut lost, what);
                    out.push_str(&fallback(name, arg));
                }
                None if matches!(name, "args" | "arg" | "shell" | "file") => {
                    push_unique(
                        &mut lost,
                        match name {
                            "arg" => "positional arguments",
                            "shell" => "shell injection",
                            "file" => "file references",
                            _ => "arguments placeholder",
                        },
                    );
                    out.push_str(&fallback(name, arg));
                }
                None => {
                    out.push_str("{{");
                    out.push_str(token);
                    out.push_str("}}");
                }
            }
            rest = &after[close + 2..];
        }
        out.push_str(rest);
        (out, lost)
    }

    fn push_unique(v: &mut Vec<String>, s: &str) {
        if !v.iter().any(|x| x == s) {
            v.push(s.to_string());
        }
    }

    /// Plain-text rendering when the dialect cannot express the marker.
    fn fallback(name: &str, arg: Option<&str>) -> String {
        match (name, arg) {
            ("arg", Some(n)) => format!("<argument {n}>"),
            ("shell", Some(cmd)) => format!("(run `{cmd}` and use its output)"),
            ("file", Some(p)) => format!("`{p}`"),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> RawFiles {
        pairs
            .iter()
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn placeholders_round_trip() {
        let src = "Create **$ARGUMENTS** from $1 and $2.\n- branch: !`git branch --show-current`\nSee @src/main.rs and @AGENTS.md, mail a@b.com\n```\nawk '{print $1}'\n```\n";
        let canon = placeholders::to_canonical(src);
        assert!(canon.contains("{{args}}"));
        assert!(canon.contains("{{arg:1}}"));
        assert!(canon.contains("{{shell:git branch --show-current}}"));
        assert!(canon.contains("{{file:src/main.rs}}"));
        assert!(canon.contains("a@b.com"));
        assert!(canon.contains("awk '{print $1}'"));
        assert_eq!(placeholders::to_claude(&canon), src);
        let gemini: BTreeMap<String, String> = [
            ("args", "{{args}}"),
            ("shell", "!{{cmd}}"),
            ("file", "@{{path}}"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let (g, lost) = placeholders::render(&canon, &gemini);
        assert!(g.contains("{{args}}"));
        assert!(g.contains("!{git branch --show-current}"));
        assert!(g.contains("@{src/main.rs}"));
        assert!(lost.contains(&"positional arguments".to_string()));
    }

    #[test]
    fn agent_with_messy_frontmatter() {
        let f = files(&[(
            "frontend-developer.md",
            "---\nname: frontend-developer\ndescription: \"Use when building.\\n\\n<example>\\nContext: x\\n</example>\"\ntools: Read, Write, Edit, Bash\nmodel: sonnet\ncolor: blue\ncustom: 1\n---\n\nYou are a senior dev.\n",
        )]);
        let c = parse_raw(ComponentKind::Agent, "frontend-developer.md", &f).unwrap();
        assert_eq!(c.name, "frontend-developer");
        assert_eq!(c.description, "Use when building.");
        let ComponentBody::Agent(a) = &c.body else {
            panic!()
        };
        assert_eq!(a.tools, vec!["Read", "Write", "Edit", "Bash"]);
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        assert_eq!(a.extra.get("custom"), Some(&serde_json::json!(1)));
        assert!(a.prompt.contains("senior dev"));
        assert!(!c.sha256.is_empty());
    }

    #[test]
    fn mcp_secrets_become_markers() {
        let f = files(&[(
            "hf.json",
            r#"{"mcpServers":{"huggingface":{"description":"HF","url":"https://huggingface.co/mcp","headers":{"Authorization":"Bearer <YOUR_HF_TOKEN>"}},
               "gh":{"command":"npx","args":["-y","gh"],"env":{"GITHUB_PERSONAL_ACCESS_TOKEN":"<token>","LOG":"debug"}}}}"#,
        )]);
        let c = parse_raw(ComponentKind::Mcp, "hf.json", &f).unwrap();
        let ComponentBody::Mcp(m) = &c.body else {
            panic!()
        };
        let hf = m.servers.iter().find(|s| s.name == "huggingface").unwrap();
        assert_eq!(hf.transport, McpTransport::Http);
        assert_eq!(hf.headers["Authorization"], "Bearer {{secret:HF_TOKEN}}");
        assert_eq!(hf.secrets[0].name, "HF_TOKEN");
        let gh = m.servers.iter().find(|s| s.name == "gh").unwrap();
        assert_eq!(
            gh.env["GITHUB_PERSONAL_ACCESS_TOKEN"],
            "{{secret:GITHUB_PERSONAL_ACCESS_TOKEN}}"
        );
        assert_eq!(gh.env["LOG"], "debug");
    }

    #[test]
    fn hook_with_supporting_files_and_loop() {
        let f = files(&[
            (
                "change-logger.json",
                r#"{"description":"log","supportingFiles":[{"source":"change-logger.py","destination":".claude/hooks/change-logger.py","executable":true}],"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"python3 .claude/hooks/change-logger.py"}]}],"Stop":"echo done"}}"#,
            ),
            ("change-logger.py", "print(1)\n"),
        ]);
        let c = parse_raw(ComponentKind::Hook, "change-logger.json", &f).unwrap();
        let ComponentBody::Hook(h) = &c.body else {
            panic!()
        };
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.supporting_files.len(), 1);
        let l = files(&[("x-loop.md", "---\nname: x-loop\ndescription: Do it\ninterval: 10m\nstop-condition: green\ncomponents: [agent:dev/test-runner.md, hook:testing/x]\n---\n# Run\n")]);
        let c = parse_raw(ComponentKind::Loop, "x-loop.md", &l).unwrap();
        let ComponentBody::Loop(lp) = &c.body else {
            panic!()
        };
        assert_eq!(
            lp.components,
            vec!["agent:dev/test-runner", "hook:testing/x"]
        );
        assert_eq!(lp.interval.as_deref(), Some("10m"));
    }
}
