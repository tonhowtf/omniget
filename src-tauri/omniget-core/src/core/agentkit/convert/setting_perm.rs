//! Canonical permission model (plan §4.3 "Settings e permissões").
//!
//! Every setting component carries Claude's schema (`permissions.allow/ask/deny`
//! with `Tool(pattern)` rules, `defaultMode`, `env`, `model` …). This module reads
//! that into a tool-neutral [`PermissionSet`] plus a [`Provider`] (alternative
//! model endpoint from `env`), and offers the small pure helpers each tool's
//! writer needs (shell prefixes, domains, MCP ids, glob → regex). No I/O.

use serde_json::{Map, Value};

/// What a rule does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }
}

/// One `Tool` or `Tool(spec)` rule in Claude's syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    /// Claude tool name (`Bash`, `Read`, `Edit`, `mcp__github__get_issue`, `*`).
    pub tool: String,
    /// Text between the parentheses, if any.
    pub spec: Option<String>,
}

impl Rule {
    /// `Bash(git push *)` → `Rule{tool: Bash, spec: "git push *"}`.
    pub fn parse(s: &str) -> Option<Rule> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        match s.find('(') {
            Some(i) if s.ends_with(')') => {
                let tool = s[..i].trim().to_string();
                let spec = s[i + 1..s.len() - 1].trim().to_string();
                if tool.is_empty() {
                    return None;
                }
                Some(Rule {
                    tool,
                    spec: (!matches!(spec.as_str(), "" | "*" | "**" | "**/*" | "./**" | "./**/*"))
                        .then_some(spec),
                })
            }
            _ => Some(Rule {
                tool: s.to_string(),
                spec: None,
            }),
        }
    }

    pub fn to_claude(&self) -> String {
        match &self.spec {
            Some(s) => format!("{}({s})", self.tool),
            None => self.tool.clone(),
        }
    }

    /// Tool family, merging Claude's aliases.
    pub fn family(&self) -> Family {
        family_of(&self.tool)
    }
}

/// Coarse tool families every tool understands in some way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Shell,
    Read,
    /// Glob / Grep / LS: reading the tree.
    Search,
    Edit,
    WebFetch,
    WebSearch,
    Agent,
    Skill,
    Todo,
    Mcp,
    /// `*`: every tool.
    All,
    Other,
}

pub fn family_of(tool: &str) -> Family {
    match tool {
        "Bash" | "Shell" | "BashOutput" | "KillBash" => Family::Shell,
        "Read" | "ReadFile" | "NotebookRead" => Family::Read,
        "Glob" | "Grep" | "LS" | "FindFiles" | "SearchFiles" | "ListFiles" => Family::Search,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "EditFile" | "WriteFile" => Family::Edit,
        "WebFetch" => Family::WebFetch,
        "WebSearch" => Family::WebSearch,
        "Agent" | "Task" => Family::Agent,
        "Skill" => Family::Skill,
        "TodoWrite" => Family::Todo,
        "*" => Family::All,
        t if t.starts_with("mcp__") => Family::Mcp,
        _ => Family::Other,
    }
}

/// Claude permission modes (`permissions.defaultMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Default,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
    Bypass,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "default" | "manual" => Mode::Default,
            "acceptEdits" => Mode::AcceptEdits,
            "plan" => Mode::Plan,
            "auto" => Mode::Auto,
            "dontAsk" => Mode::DontAsk,
            "bypassPermissions" => Mode::Bypass,
            _ => return None,
        })
    }

    pub fn as_claude(self) -> &'static str {
        match self {
            Mode::Default => "default",
            Mode::AcceptEdits => "acceptEdits",
            Mode::Plan => "plan",
            Mode::Auto => "auto",
            Mode::DontAsk => "dontAsk",
            Mode::Bypass => "bypassPermissions",
        }
    }
}

/// `permissions` of a Claude settings fragment, tool-neutral.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PermissionSet {
    pub rules: Vec<(Decision, Rule)>,
    pub mode: Option<Mode>,
    pub additional_dirs: Vec<String>,
    /// Keys of `permissions` we do not model (`disableBypassPermissionsMode` …).
    pub other: Vec<String>,
}

impl PermissionSet {
    pub fn from_values(values: &Map<String, Value>) -> PermissionSet {
        let mut out = PermissionSet::default();
        let Some(Value::Object(p)) = values.get("permissions") else {
            return out;
        };
        for (k, v) in p {
            let decision = match k.as_str() {
                "allow" => Some(Decision::Allow),
                "ask" => Some(Decision::Ask),
                "deny" => Some(Decision::Deny),
                _ => None,
            };
            match (decision, v) {
                (Some(d), Value::Array(items)) => {
                    for it in items {
                        if let Some(r) = it.as_str().and_then(Rule::parse) {
                            if !out.rules.contains(&(d, r.clone())) {
                                out.rules.push((d, r));
                            }
                        }
                    }
                }
                (None, Value::String(m)) if k == "defaultMode" => out.mode = Mode::parse(m),
                (None, Value::Array(dirs)) if k == "additionalDirectories" => {
                    out.additional_dirs = dirs
                        .iter()
                        .filter_map(|d| d.as_str().map(str::to_string))
                        .collect();
                }
                _ => out.other.push(k.clone()),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.mode.is_none() && self.additional_dirs.is_empty()
    }

    /// Rules of one decision, in order.
    pub fn of(&self, d: Decision) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(move |(x, _)| *x == d)
            .map(|(_, r)| r)
    }

    /// Decision for a bare rule of this family (no pattern), strongest wins.
    pub fn bare(&self, f: Family) -> Option<Decision> {
        self.rules
            .iter()
            .filter(|(_, r)| r.spec.is_none() && r.family() == f)
            .map(|(d, _)| *d)
            .max()
    }

    /// Edits and writes are denied outright (read-only profile).
    pub fn writes_denied(&self) -> bool {
        self.bare(Family::Edit) == Some(Decision::Deny)
    }

    /// Shell denied outright.
    pub fn shell_denied(&self) -> bool {
        self.bare(Family::Shell) == Some(Decision::Deny)
    }

    /// Network tools denied outright (WebFetch, and WebSearch when mentioned).
    pub fn network_denied(&self) -> bool {
        self.bare(Family::WebFetch) == Some(Decision::Deny)
            && !matches!(self.bare(Family::WebSearch), Some(Decision::Allow))
    }
}

/// `Bash(git push *)` → `git push`; `Bash(npm run test:*)` → `npm run test`.
pub fn shell_prefix(spec: &str) -> String {
    let mut s = spec.trim().to_string();
    loop {
        let before = s.clone();
        for suf in [" *", ":*", "*", " "] {
            if let Some(x) = s.strip_suffix(suf) {
                s = x.to_string();
            }
        }
        if s == before {
            break;
        }
    }
    s.trim().to_string()
}

/// Prefix as argv tokens, `None` when a wildcard sits inside it.
pub fn shell_tokens(spec: &str) -> Option<Vec<String>> {
    let p = shell_prefix(spec);
    if p.is_empty() || p.contains('*') || p.contains('?') {
        return None;
    }
    Some(p.split_whitespace().map(str::to_string).collect())
}

/// `domain:example.com` → `example.com`.
pub fn domain_of(spec: &str) -> Option<&str> {
    spec.strip_prefix("domain:").map(str::trim)
}

/// `mcp__srv__tool` → (`srv`, Some(`tool`)); `mcp__srv` or `mcp__srv__*` →
/// (`srv`, None); `mcp__*` → None (every server).
pub fn mcp_parts(tool: &str) -> Option<(String, Option<String>)> {
    let rest = tool.strip_prefix("mcp__")?;
    if rest == "*" || rest.is_empty() {
        return None;
    }
    match rest.split_once("__") {
        Some((s, t)) if t != "*" && !t.is_empty() => Some((s.to_string(), Some(t.to_string()))),
        Some((s, _)) => Some((s.to_string(), None)),
        None => Some((rest.trim_end_matches('*').to_string(), None)),
    }
}

/// Claude path spec → plain glob (`./src/**` → `src/**`, `//abs` → `/abs`).
pub fn path_glob(spec: &str) -> String {
    let s = spec.trim();
    if let Some(abs) = s.strip_prefix("//") {
        return format!("/{abs}");
    }
    if let Some(rel) = s.strip_prefix("./") {
        return rel.to_string();
    }
    if let Some(rel) = s.strip_prefix('/') {
        // `/x` is project-root relative in Claude
        return rel.to_string();
    }
    s.to_string()
}

/// Glob → regex body (no anchors): `**` any path, `*` within a segment, `?` one char.
pub fn glob_to_regex(glob: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push_str(".*");
                i += 1;
                if chars.get(i + 1) == Some(&'/') {
                    i += 1;
                }
            }
            '*' => out.push_str("[^/\"]*"),
            '?' => out.push_str("[^/\"]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '[' | ']' | '{' | '}' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

// ------------------------------------------------------------------ provider

/// Wire protocol of an alternative endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

/// Where the provider comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// Base URL + key (GLM, MiniMax, a proxy, a local server).
    Custom,
    Bedrock,
    Vertex,
}

/// An alternative model endpoint read from `env` (+ `apiKeyHelper`).
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub kind: ProviderKind,
    pub protocol: Protocol,
    pub base_url: Option<String>,
    /// Environment variable that holds the key (never the key itself).
    pub key_env: Option<String>,
    pub model: Option<String>,
    pub small_model: Option<String>,
}

/// Env keys a [`Provider`] consumes (they are not reported as losses on their own).
pub const PROVIDER_ENV: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "OPENAI_BASE_URL",
    "OPENAI_API_KEY",
    "OPENAI_MODEL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLOUD_ML_REGION",
    "ANTHROPIC_VERTEX_PROJECT_ID",
];

/// Telemetry switches, `true` when telemetry is turned off.
pub fn telemetry_off(env: &Map<String, Value>) -> Option<bool> {
    let truthy = |k: &str| {
        env.get(k).map(|v| match v {
            Value::String(s) => !matches!(s.as_str(), "" | "0" | "false"),
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_i64() != Some(0),
            _ => false,
        })
    };
    if truthy("DISABLE_TELEMETRY") == Some(true)
        || truthy("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC") == Some(true)
    {
        return Some(true);
    }
    match truthy("CLAUDE_CODE_ENABLE_TELEMETRY") {
        Some(true) => Some(false),
        Some(false) => Some(true),
        None => None,
    }
}

/// Env keys that only steer Claude Code's telemetry (consumed by [`telemetry_off`]).
pub const TELEMETRY_ENV: &[&str] = &[
    "DISABLE_TELEMETRY",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_ENABLE_TELEMETRY",
];

/// Name of the variable referenced by `{{secret:X}}`, `${X}`, `$X`, `{env:X}`.
pub fn var_ref(s: &str) -> Option<String> {
    let s = s.trim();
    let valid = |v: &str| {
        !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && !v.starts_with(|c: char| c.is_ascii_digit())
    };
    for (pre, suf) in [
        ("{{secret:", "}}"),
        ("${env:", "}"),
        ("${", "}"),
        ("{env:", "}"),
    ] {
        if let Some(v) = s.strip_prefix(pre).and_then(|x| x.strip_suffix(suf)) {
            if valid(v) {
                return Some(v.to_string());
            }
        }
    }
    // `$X` anywhere (apiKeyHelper = `printf %s "$X"`)
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '$' {
            let mut j = i + 1;
            let brace = bytes.get(j) == Some(&'{');
            if brace {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == '_') {
                j += 1;
            }
            let v: String = bytes[start..j].iter().collect();
            if valid(&v) {
                return Some(v);
            }
        }
        i += 1;
    }
    None
}

/// Reads the provider from `env` and `apiKeyHelper`.
pub fn provider_from(values: &Map<String, Value>) -> Option<Provider> {
    let env = values.get("env").and_then(|e| e.as_object());
    let get = |k: &str| {
        env.and_then(|e| e.get(k))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    let helper_var = values
        .get("apiKeyHelper")
        .and_then(|h| h.as_str())
        .and_then(var_ref);
    let key_from = |key: &str| -> Option<String> {
        match get(key) {
            // a reference keeps the variable name; a literal/placeholder means
            // "put the key in this very variable"
            Some(v) => Some(var_ref(&v).unwrap_or_else(|| key.to_string())),
            None => None,
        }
    };
    if get("CLAUDE_CODE_USE_BEDROCK").is_some() {
        return Some(Provider {
            kind: ProviderKind::Bedrock,
            protocol: Protocol::Anthropic,
            base_url: None,
            key_env: get("AWS_BEARER_TOKEN_BEDROCK").map(|_| "AWS_BEARER_TOKEN_BEDROCK".into()),
            model: get("ANTHROPIC_MODEL"),
            small_model: get("ANTHROPIC_SMALL_FAST_MODEL"),
        });
    }
    if get("CLAUDE_CODE_USE_VERTEX").is_some() {
        return Some(Provider {
            kind: ProviderKind::Vertex,
            protocol: Protocol::Anthropic,
            base_url: None,
            key_env: None,
            model: get("ANTHROPIC_MODEL"),
            small_model: get("ANTHROPIC_SMALL_FAST_MODEL"),
        });
    }
    if let Some(url) = get("ANTHROPIC_BASE_URL") {
        return Some(Provider {
            kind: ProviderKind::Custom,
            protocol: Protocol::Anthropic,
            base_url: Some(url),
            key_env: helper_var
                .or_else(|| key_from("ANTHROPIC_AUTH_TOKEN"))
                .or_else(|| key_from("ANTHROPIC_API_KEY")),
            model: get("ANTHROPIC_MODEL").or_else(|| get("ANTHROPIC_DEFAULT_SONNET_MODEL")),
            small_model: get("ANTHROPIC_SMALL_FAST_MODEL")
                .or_else(|| get("ANTHROPIC_DEFAULT_HAIKU_MODEL")),
        });
    }
    if let Some(url) = get("OPENAI_BASE_URL") {
        return Some(Provider {
            kind: ProviderKind::Custom,
            protocol: Protocol::OpenAi,
            base_url: Some(url),
            key_env: key_from("OPENAI_API_KEY").or(helper_var),
            model: get("OPENAI_MODEL"),
            small_model: None,
        });
    }
    None
}

/// Provider id derived from the base URL host (`api.z.ai` → `z-ai`).
pub fn provider_id(p: &Provider) -> String {
    match p.kind {
        ProviderKind::Bedrock => return "amazon-bedrock".into(),
        ProviderKind::Vertex => return "google-vertex-anthropic".into(),
        ProviderKind::Custom => {}
    }
    let host = p
        .base_url
        .as_deref()
        .and_then(|u| u.split("://").nth(1))
        .and_then(|r| r.split(['/', ':']).next())
        .unwrap_or("custom");
    let parts: Vec<&str> = host.split('.').filter(|s| !s.is_empty()).collect();
    let core: Vec<&str> = if parts.len() > 2 && matches!(parts[0], "api" | "www") {
        parts[1..].to_vec()
    } else {
        parts
    };
    let id: String = core
        .join("-")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if id.is_empty() {
        "custom".into()
    } else {
        format!("omniget-{id}")
    }
}

/// Top-level Claude keys the neutral model understands; every other key is
/// Claude-only (spinner, output style, announcements …).
pub const MODELLED_KEYS: &[&str] = &[
    "permissions",
    "env",
    "model",
    "statusLine",
    "hooks",
    "apiKeyHelper",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_parse_and_classify() {
        let v = json!({"permissions": {
            "allow": ["Read(**/*)", "Glob", "Grep", "LS", "Bash(git *)"],
            "ask": ["Bash(git push *)"],
            "deny": ["Edit", "Write", "MultiEdit", "Bash", "WebFetch", "Read(./.env)"],
            "defaultMode": "plan", "additionalDirectories": ["../lib"]
        }});
        let p = PermissionSet::from_values(v.as_object().unwrap());
        assert_eq!(p.mode, Some(Mode::Plan));
        assert!(p.writes_denied() && p.shell_denied() && p.network_denied());
        assert_eq!(
            p.bare(Family::Read),
            Some(Decision::Allow),
            "Read(**/*) is bare"
        );
        assert_eq!(p.of(Decision::Ask).count(), 1);
        assert_eq!(p.additional_dirs, vec!["../lib".to_string()]);
        assert_eq!(Rule::parse("Read(**/*)").unwrap().spec, None);
    }

    #[test]
    fn shell_and_ids() {
        assert_eq!(shell_prefix("git push *"), "git push");
        assert_eq!(shell_prefix("npm run test:*"), "npm run test");
        assert_eq!(shell_tokens("rm -rf *").unwrap(), vec!["rm", "-rf"]);
        assert!(shell_tokens("git * main").is_none());
        assert_eq!(
            mcp_parts("mcp__github__get_issue"),
            Some(("github".into(), Some("get_issue".into())))
        );
        assert_eq!(mcp_parts("mcp__github__*"), Some(("github".into(), None)));
        assert_eq!(mcp_parts("mcp__*"), None);
        assert_eq!(path_glob("./.env"), ".env");
        assert_eq!(path_glob("//etc/x"), "/etc/x");
        assert_eq!(glob_to_regex("src/**/*.ts"), "src/.*[^/\"]*\\.ts");
    }

    #[test]
    fn provider_from_env() {
        let glm = json!({"env": {
            "ANTHROPIC_BASE_URL": "https://api.z.ai/api/anthropic",
            "ANTHROPIC_AUTH_TOKEN": "YOUR-API-KEY",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "glm-4.7"
        }});
        let p = provider_from(glm.as_object().unwrap()).unwrap();
        assert_eq!(p.protocol, Protocol::Anthropic);
        assert_eq!(p.key_env.as_deref(), Some("ANTHROPIC_AUTH_TOKEN"));
        assert_eq!(p.model.as_deref(), Some("glm-4.7"));
        assert_eq!(provider_id(&p), "omniget-z-ai");
        let helper = json!({"env": {"ANTHROPIC_BASE_URL": "http://localhost:4000"},
            "apiKeyHelper": "printf %s \"$MY_PROXY_KEY\""});
        let p = provider_from(helper.as_object().unwrap()).unwrap();
        assert_eq!(p.key_env.as_deref(), Some("MY_PROXY_KEY"));
        assert_eq!(var_ref("{{secret:GH_TOKEN}}").as_deref(), Some("GH_TOKEN"));
        assert_eq!(var_ref("your-key"), None);
    }
}
