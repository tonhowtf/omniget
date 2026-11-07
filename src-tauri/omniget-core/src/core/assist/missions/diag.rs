use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const REDACTED: &str = "[redacted]";
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedAction {
    pub id: String,
    pub label: String,
    pub effect: String,
    pub needs_authorization: bool,
}

impl SuggestedAction {
    pub fn new(id: &str, label: &str, effect: &str, needs_authorization: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            effect: effect.into(),
            needs_authorization,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnosis {
    pub code: String,
    pub stage: String,
    pub retryable: bool,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub suggested_actions: Vec<SuggestedAction>,
    #[serde(default)]
    pub detail: Option<String>,
}

impl Diagnosis {
    pub fn new(code: &str, stage: &str, retryable: bool, summary: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            stage: stage.into(),
            retryable,
            summary: summary.into(),
            evidence_refs: Vec::new(),
            correlation_id: None,
            suggested_actions: Vec::new(),
            detail: None,
        }
    }

    pub fn with_refs(mut self, refs: impl IntoIterator<Item = String>) -> Self {
        self.evidence_refs.extend(refs);
        self
    }

    pub fn with_action(mut self, a: SuggestedAction) -> Self {
        self.suggested_actions.push(a);
        self
    }

    pub fn with_correlation(mut self, id: &str) -> Self {
        self.correlation_id = Some(id.to_string());
        self
    }

    pub fn with_detail(mut self, d: &str) -> Self {
        self.detail = Some(super::clip(&redact(d), 4000));
        self
    }

    pub fn effect_unknown(mission: &str, task: &str, detail: &str) -> Self {
        Self::new(
            super::ERR_MISSION_EFFECT_UNKNOWN,
            "resume",
            false,
            "The app stopped while a task was acting. Its outcome is unknown, so it is not repeated on its own.",
        )
        .with_correlation(mission)
        .with_refs([format!("task:{task}")])
        .with_detail(detail)
        .with_action(SuggestedAction::new(
            "unblock_confirmed",
            "It happened",
            "Mark the effect as done and move on without repeating it.",
            false,
        ))
        .with_action(SuggestedAction::new(
            "unblock_retry",
            "It did not happen",
            "Run the task again; the agent is told to check what exists first.",
            false,
        ))
        .with_action(SuggestedAction::new("cancel", "Cancel the mission", "Stop everything that is still pending.", false))
    }

    pub fn criteria_failing(mission: &str, verdict: &super::verify::Verdict, rounds: u32) -> Self {
        Self::new(
            super::ERR_MISSION_CRITERIA,
            "verify",
            true,
            format!(
                "After {rounds} round(s) the required checks still do not pass: {}",
                verdict.summary()
            ),
        )
        .with_correlation(mission)
        .with_refs(
            verdict
                .criteria
                .iter()
                .filter_map(|c| c.receipt_id.clone().map(|r| format!("receipt:{r}"))),
        )
        .with_action(SuggestedAction::new(
            "resume",
            "Try more rounds",
            "Queue the mission again with the same criteria.",
            false,
        ))
        .with_action(SuggestedAction::new(
            "edit_criteria",
            "Adjust the criteria",
            "Open the criteria to change or relax them (a new version).",
            false,
        ))
        .with_action(SuggestedAction::new(
            "cancel",
            "Cancel",
            "Stop the mission and keep what exists.",
            false,
        ))
    }
}

pub fn from_error(raw: &str, stage: &str, correlation: Option<&str>) -> Diagnosis {
    let tokens: Vec<&str> = raw
        .split(|c: char| c == ':' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();
    let code = tokens
        .iter()
        .find(|p| p.starts_with("ERR_"))
        .or_else(|| {
            tokens.iter().rev().find(|t| {
                t.len() >= 6
                    && t.contains('_')
                    && t.chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            })
        })
        .copied()
        .unwrap_or("ERR_UNKNOWN")
        .to_string();
    let (summary, retryable, actions): (&str, bool, Vec<SuggestedAction>) = match code.as_str() {
        "ERR_LLM_BUDGET" | "ERR_MISSION_BUDGET" => (
            "The budget for this work is used up.",
            false,
            vec![
                SuggestedAction::new("raise_budget", "Raise the budget", "Allow more spend for this mission.", true),
                SuggestedAction::new("cancel", "Stop here", "Keep what exists and stop.", false),
            ],
        ),
        "ERR_LLM_NET" | "ERR_LLM_TIMEOUT" => (
            "The model provider did not answer (network down, or the local model server is not running).",
            true,
            vec![
                SuggestedAction::new("open_local", "Check the model server", "Open the local models / accounts page to start or reconnect it.", false),
                SuggestedAction::new("resume", "Try again", "Queue the mission again once the provider answers.", false),
            ],
        ),
        "EXTERNAL_MISSION_BUDGET_EXCEEDED" => (
            "This mission's own token cap is used up; the grant's other missions are not affected.",
            false,
            vec![SuggestedAction::new("raise_budget", "Raise this mission's cap locally", "A person raises the mission's limits in OmniGet (within the grant's ceiling) and resumes it; the controller cannot.", true)],
        ),
        "EXTERNAL_BUDGET_EXCEEDED" | "EXECUTION_GRANT_EXCEEDED" => (
            "The shared token ceiling of this execution grant is used up (all missions of the grant draw from it).",
            false,
            vec![SuggestedAction::new("raise_grant", "Raise the grant locally", "A person grants a new ceiling in OmniGet → Settings → MCP; the controller cannot.", true)],
        ),
        "EXECUTION_NOT_GRANTED" | "EXTERNAL_EXECUTION_DENIED" | "EXECUTOR_REVISION_CHANGED" | "WORKSPACE_CHANGED" => (
            "The local execution grant no longer allows this (revoked, executor changed, or the workspace moved).",
            false,
            vec![SuggestedAction::new("raise_grant", "Review the grant locally", "A person checks the grant in OmniGet → Settings → MCP.", true)],
        ),
        "ERR_TOOL_DENIED" => (
            "A tool the task needed is not allowed for this bot.",
            false,
            vec![SuggestedAction::new("open_bot", "Review the bot's tools", "Open the bot to grant the tool, if you want to.", true)],
        ),
        "ERR_TOOL_TIMEOUT" => (
            "A permission question was not answered in time.",
            true,
            vec![SuggestedAction::new("resume", "Try again", "Run the task again and answer the question.", false)],
        ),
        "ERR_TOOL_UNKNOWN" => (
            "The task asked for a tool that does not exist here (a server may be disconnected).",
            false,
            vec![SuggestedAction::new("open_mcp", "Check connections", "Open the MCP connections to reconnect the server.", false)],
        ),
        "ERR_MISSION_PIN_CHANGED" => (
            "The account, folder or runtime changed since the mission ran; its session cannot be reused.",
            false,
            vec![SuggestedAction::new(
                "allow_replay",
                "Continue with a fresh session",
                "Replay the mission's context into a new session on the current account/folder.",
                true,
            )],
        ),
        "ERR_MISSION_STAGNANT" => (
            "The work stopped making progress: the same actions repeat and nothing changes.",
            false,
            vec![
                SuggestedAction::new("edit_context", "Correct the context", "Add a note or fix what the agent misunderstood, then resume.", false),
                SuggestedAction::new("cancel", "Cancel", "Stop the mission.", false),
            ],
        ),
        "ERR_MISSION_UNSUPPORTED" => (
            "This runtime lacks a capability the mission needs.",
            false,
            vec![SuggestedAction::new("change_bot", "Use another bot", "Pick a bot whose runtime supports it.", false)],
        ),
        _ => (
            "The step failed.",
            true,
            vec![SuggestedAction::new("resume", "Try again", "Queue the mission again.", false)],
        ),
    };
    let mut d = Diagnosis::new(&code, stage, retryable, summary).with_detail(raw);
    d.suggested_actions = actions;
    if let Some(c) = correlation {
        d.correlation_id = Some(c.to_string());
    }
    d
}

fn sensitive_key(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase().replace(['-', '_', ' '], "");
    matches!(
        k.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "password"
            | "passwd"
            | "secret"
            | "clientsecret"
            | "apikey"
            | "xapikey"
            | "key"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "session"
            | "sessionid"
            | "jwt"
            | "signature"
            | "sig"
            | "credential"
            | "credentials"
            | "auth"
            | "policy"
            | "expires"
            | "xamzsignature"
            | "xamzcredential"
            | "xamzsecuritytoken"
            | "googleaccessid"
            | "privatekey"
            | "secretkey"
            | "accesskey"
            | "signingkey"
            | "lsig"
            | "oh"
            | "oe"
            | "hdnea"
            | "hdnts"
            | "hmac"
            | "ip"
    ) || k.starts_with("xamz")
        || k.starts_with("xgoog")
        || k.ends_with("token")
        || k.ends_with("secret")
        || k.ends_with("claim")
        || k.ends_with("privatekey")
        || k.ends_with("password")
}

fn opaque_value(v: &str) -> bool {
    v.len() >= 24
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.~%/+=".contains(&b))
        && v.bytes().any(|b| b.is_ascii_digit())
        && v.bytes().any(|b| b.is_ascii_alphabetic())
}

fn redact_url(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_owned();
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        let _ = parsed.set_username("redacted");
        let _ = parsed.set_password(None);
    }
    if parsed.query().is_some() {
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| {
                let val = if sensitive_key(&k) || opaque_value(&v) || v.contains("hmac=") {
                    REDACTED.to_owned()
                } else {
                    v.into_owned()
                };
                (k.into_owned(), val)
            })
            .collect();
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    if parsed.fragment().map(|f| f.contains('=')).unwrap_or(false) {
        parsed.set_fragment(Some(REDACTED));
    }
    parsed.to_string()
}

fn token_shapes() -> &'static Regex {
    static KEYS: OnceLock<Regex> = OnceLock::new();
    KEYS.get_or_init(|| {
        Regex::new(concat!(
            r"\b(sk-[A-Za-z0-9_-]{16,}|sk-ant-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}",
            r"|xox[abpr]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,}",
            r"|[sr]k_(?:live|test)_[A-Za-z0-9]{10,}|ya29\.[A-Za-z0-9_-]{10,}|glpat-[A-Za-z0-9_-]{16,}",
            r"|npm_[A-Za-z0-9]{20,}|hf_[A-Za-z0-9]{20,}|b3BlbnNzaC1rZXktdjE[A-Za-z0-9+/=]*)"
        ))
        .unwrap()
    })
}

fn pem_block() -> &'static Regex {
    static PEM: OnceLock<Regex> = OnceLock::new();
    PEM.get_or_init(|| {
        Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----(?s:.*?)(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)").unwrap()
    })
}

fn percent_decoded(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut any = false;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                any = true;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    any.then(|| String::from_utf8_lossy(&out).into_owned())
}

pub fn redact(text: &str) -> String {
    let once = redact_once(text);
    match percent_decoded(&once) {
        Some(decoded) => {
            let red = redact_once(&decoded);
            if red != decoded {
                red
            } else {
                once
            }
        }
        None => once,
    }
}

fn redact_once(text: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    static PATH_TOKEN: OnceLock<Regex> = OnceLock::new();
    static HEADER: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    static ASSIGN: OnceLock<Regex> = OnceLock::new();
    static JWT: OnceLock<Regex> = OnceLock::new();
    static COOKIES: OnceLock<Regex> = OnceLock::new();
    static HOME: OnceLock<Regex> = OnceLock::new();
    let urls = URL.get_or_init(|| Regex::new(r#"https?://[^\s<>"'`]+"#).unwrap());
    let path_token = PATH_TOKEN.get_or_init(|| {
        Regex::new(r"(?i)(/bot)\d{3,}:[A-Za-z0-9_-]{20,}|(/webhooks/\d+/)[A-Za-z0-9_-]{20,}|(hooks\.slack\.com/services/)[A-Za-z0-9/]{20,}").unwrap()
    });
    let headers = HEADER.get_or_init(|| {
        Regex::new(r"(?i)\b((?:proxy-)?authorization|set-cookie|cookie|x-api-key|x-csrf-token)\s*[:=][ \t]*(?:\r?\n[ \t]+)?[^\r\n]*").unwrap()
    });
    let bearer = BEARER.get_or_init(|| {
        Regex::new(r"(?i)\b(Bearer|Basic|Token)\s+[A-Za-z0-9._~+/=-]{8,}").unwrap()
    });
    let assign = ASSIGN.get_or_init(|| {
        Regex::new(r#"(?i)(^|[^A-Za-z0-9_-])([A-Za-z0-9_-]*?(?:token|secret|api[_-]?key|access[_-]?key|private[_-]?key|password|passwd|credential|signature|cookie|session[_-]?id|sessionid)[A-Za-z0-9_-]*)\s*["']?\s*[:=]\s*("[^"\r\n]*"|'[^'\r\n]*'|\[redacted\]|[^\s,;\]}&"']+)"#).unwrap()
    });
    let jwt = JWT.get_or_init(|| {
        Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b").unwrap()
    });
    let cookies = COOKIES.get_or_init(|| {
        Regex::new(r"\b(SID|HSID|SSID|APISID|SAPISID|LSID|NID|__Secure-[A-Za-z0-9-]+|__Host-[A-Za-z0-9-]+|csrftoken|ds_user_id|sessionid|auth_token|ct0)=[^;\s,]+").unwrap()
    });
    let home = HOME
        .get_or_init(|| Regex::new(r"(/Users/|/home/|C:\\Users\\|C:/Users/)[^/\\\s]+").unwrap());
    let t = pem_block().replace_all(text, REDACTED);
    let t = path_token.replace_all(&t, |c: &Captures| {
        let keep = c
            .get(1)
            .or(c.get(2))
            .or(c.get(3))
            .map(|m| m.as_str())
            .unwrap_or("");
        format!("{keep}{REDACTED}")
    });
    let t = urls.replace_all(&t, |c: &Captures| redact_url(&c[0]));
    let t = headers.replace_all(&t, |c: &Captures| format!("{}: {REDACTED}", &c[1]));
    let t = bearer.replace_all(&t, |c: &Captures| format!("{} {REDACTED}", &c[1]));
    let t = assign.replace_all(&t, |c: &Captures| {
        if &c[3] == REDACTED {
            c[0].to_string()
        } else {
            format!("{}{}={REDACTED}", &c[1], &c[2])
        }
    });
    let t = jwt.replace_all(&t, REDACTED);
    let t = token_shapes().replace_all(&t, REDACTED);
    let t = cookies.replace_all(&t, |c: &Captures| format!("{}={REDACTED}", &c[1]));
    let t = home.replace_all(&t, |c: &Captures| format!("{}~", &c[1]));
    t.into_owned()
}

pub fn redact_json(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(redact_json).collect()),
        Value::Object(o) => {
            let har_secret = o
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(sensitive_key)
                && o.contains_key("value");
            let mut out = serde_json::Map::new();
            for (i, (k, v)) in o.into_iter().enumerate() {
                let v = if (sensitive_key(&k) || (har_secret && k == "value"))
                    && !v.is_boolean()
                    && !v.is_number()
                    && !v.is_null()
                {
                    Value::String(REDACTED.into())
                } else {
                    redact_json(v)
                };
                let rk = redact(&k);
                let k = if rk != k {
                    format!("{REDACTED}#{i}")
                } else {
                    k
                };
                out.insert(k, v);
            }
            Value::Object(out)
        }
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpHealth {
    pub reachable: bool,
    pub authenticated: bool,
    pub initialized: bool,
    pub tool_capable: bool,
    pub level: String,
    pub usable: bool,
    pub summary: String,
    pub suggested_actions: Vec<SuggestedAction>,
}

pub fn mcp_health(
    ok: bool,
    error_code: Option<&str>,
    error_message: Option<&str>,
    tool_count: usize,
) -> McpHealth {
    let msg = error_message.unwrap_or("");
    let status: Option<u16> = msg
        .split_whitespace()
        .skip_while(|w| *w != "HTTP")
        .nth(1)
        .and_then(|s| s.trim_end_matches(':').parse().ok());
    let mk = |reachable,
              authenticated,
              initialized,
              tool_capable,
              level: &str,
              summary: String,
              actions: Vec<SuggestedAction>| McpHealth {
        reachable,
        authenticated,
        initialized,
        tool_capable,
        level: level.into(),
        usable: tool_capable,
        summary,
        suggested_actions: actions,
    };
    if ok && tool_count > 0 {
        return mk(
            true,
            true,
            true,
            true,
            "ready",
            format!("connected, {tool_count} tool(s) available"),
            vec![],
        );
    }
    if ok {
        return mk(
            true,
            true,
            true,
            false,
            "no_tools",
            "connected, but the server offers no tools; nothing can use it yet".into(),
            vec![SuggestedAction::new(
                "open_mcp",
                "Check the server",
                "Open the server settings.",
                false,
            )],
        );
    }
    match (error_code.unwrap_or(""), status) {
        (_, Some(401)) | (_, Some(403)) => mk(
            true,
            false,
            false,
            false,
            "unauthenticated",
            format!("the server answered HTTP {} — it is reachable but not authorised; its tools are not usable", status.unwrap_or(401)),
            vec![SuggestedAction::new("edit_credentials", "Fix the credential", "Open the server's authentication settings.", true)],
        ),
        (_, Some(s)) if s >= 400 => mk(true, true, false, false, "not_initialized", format!("the server answered HTTP {s}; it did not complete the MCP handshake"), vec![SuggestedAction::new("retry", "Try again", "Run the check again.", false)]),
        ("ERR_MCP_PROTO", _) => mk(true, true, false, false, "not_initialized", "the server answered but not as an MCP server (handshake failed)".into(), vec![SuggestedAction::new("open_mcp", "Check the address", "The URL or command may point at something else.", false)]),
        _ => mk(false, false, false, false, "unreachable", format!("could not reach the server: {}", redact(msg)), vec![SuggestedAction::new("retry", "Try again", "Run the check again.", false)]),
    }
}
