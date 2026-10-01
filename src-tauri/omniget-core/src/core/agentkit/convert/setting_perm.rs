use serde_json::{Map, Value};

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    pub tool: String,
    pub spec: Option<String>,
}

impl Rule {
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

    pub fn family(&self) -> Family {
        family_of(&self.tool)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Shell,
    Read,
    Search,
    Edit,
    WebFetch,
    WebSearch,
    Agent,
    Skill,
    Todo,
    Mcp,
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PermissionSet {
    pub rules: Vec<(Decision, Rule)>,
    pub mode: Option<Mode>,
    pub additional_dirs: Vec<String>,
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

    pub fn of(&self, d: Decision) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(move |(x, _)| *x == d)
            .map(|(_, r)| r)
    }

    pub fn bare(&self, f: Family) -> Option<Decision> {
        self.rules
            .iter()
            .filter(|(_, r)| r.spec.is_none() && r.family() == f)
            .map(|(d, _)| *d)
            .max()
    }

    pub fn writes_denied(&self) -> bool {
        self.bare(Family::Edit) == Some(Decision::Deny)
    }

    pub fn shell_denied(&self) -> bool {
        self.bare(Family::Shell) == Some(Decision::Deny)
    }
}

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

pub fn shell_tokens(spec: &str) -> Option<Vec<String>> {
    let p = shell_prefix(spec);
    if p.is_empty() || p.contains('*') || p.contains('?') {
        return None;
    }
    Some(p.split_whitespace().map(str::to_string).collect())
}

pub fn domain_of(spec: &str) -> Option<&str> {
    spec.strip_prefix("domain:").map(str::trim)
}

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

pub fn path_glob(spec: &str) -> String {
    let s = spec.trim();
    if let Some(abs) = s.strip_prefix("//") {
        return format!("/{abs}");
    }
    if let Some(rel) = s.strip_prefix("./") {
        return rel.to_string();
    }
    if let Some(rel) = s.strip_prefix('/') {
        return rel.to_string();
    }
    s.to_string()
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Custom,
    Bedrock,
    Vertex,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub kind: ProviderKind,
    pub protocol: Protocol,
    pub base_url: Option<String>,
    pub key_env: Option<String>,
    pub model: Option<String>,
    pub small_model: Option<String>,
}

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

pub const TELEMETRY_ENV: &[&str] = &[
    "DISABLE_TELEMETRY",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
    "CLAUDE_CODE_ENABLE_TELEMETRY",
];

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
        get(key).map(|v| var_ref(&v).unwrap_or_else(|| key.to_string()))
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

pub const MODELLED_KEYS: &[&str] = &[
    "permissions",
    "env",
    "model",
    "statusLine",
    "hooks",
    "apiKeyHelper",
];
