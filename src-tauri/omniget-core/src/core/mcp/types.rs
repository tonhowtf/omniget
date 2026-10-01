use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::error::{McpError, ERR_MCP_HTTP, ERR_MCP_SPAWN};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

pub const SECRET_PREFIX: &str = "secret:";
pub const SECRET_NS: &str = "mcp";

pub const CLIENT_NAME: &str = "OmniGet";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default = "empty_schema")]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

impl ToolDef {
    pub fn new(name: impl Into<String>, description: impl Into<String>, schema: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            output_schema: None,
            title: None,
        }
    }

    pub fn schema_is_object(&self) -> bool {
        self.input_schema.get("type").and_then(|t| t.as_str()) == Some("object")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion", default)]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(rename = "serverInfo", default)]
    pub server_info: ServerInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl InitializeResult {
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.get(capability).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Result { id: String, result: Value },
    Failure { id: String, error: RpcError },
    ServerMessage { method: String },
    Unknown,
}

pub fn request(id: i64, method: &str, params: Option<Value>) -> Value {
    let mut m = Map::new();
    m.insert("jsonrpc".into(), json!("2.0"));
    m.insert("id".into(), json!(id));
    m.insert("method".into(), json!(method));
    if let Some(p) = params {
        m.insert("params".into(), p);
    }
    Value::Object(m)
}

pub fn notification(method: &str, params: Option<Value>) -> Value {
    let mut m = Map::new();
    m.insert("jsonrpc".into(), json!("2.0"));
    m.insert("method".into(), json!(method));
    if let Some(p) = params {
        m.insert("params".into(), p);
    }
    Value::Object(m)
}

pub fn initialize_params(client_version: &str) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": CLIENT_NAME, "version": client_version },
    })
}

pub fn id_key(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

pub fn classify(v: &Value) -> Incoming {
    let id = v.get("id").and_then(id_key);
    if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
        return Incoming::ServerMessage {
            method: method.to_string(),
        };
    }
    let Some(id) = id else {
        return Incoming::Unknown;
    };
    if let Some(err) = v.get("error") {
        let error = serde_json::from_value::<RpcError>(err.clone()).unwrap_or(RpcError {
            code: 0,
            message: err.to_string(),
            data: None,
        });
        return Incoming::Failure { id, error };
    }
    match v.get("result") {
        Some(result) => Incoming::Result {
            id,
            result: result.clone(),
        },
        None => Incoming::Unknown,
    }
}

pub fn parse_tools(result: &Value) -> Vec<ToolDef> {
    result
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<ToolDef>(v.clone()).ok())
                .filter(|t| !t.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn is_tool_error(result: &Value) -> bool {
    result
        .get("isError")
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
}

pub fn tool_result_text(result: &Value) -> String {
    let parts: Vec<&str> = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect()
        })
        .unwrap_or_default();
    parts.join("\n")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Transport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub transport: Transport,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

fn enabled_default() -> bool {
    true
}

pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;

impl McpServerConfig {
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1))
    }

    pub fn is_stdio(&self) -> bool {
        matches!(self.transport, Transport::Stdio { .. })
    }

    pub fn secret_id_for(&self, kind: &str, name: &str) -> String {
        secret_id(&self.id, kind, name)
    }

    pub fn extract_secrets(&mut self) -> Vec<(String, String)> {
        let id = self.id.clone();
        let take = |kind: &str,
                    map: &mut BTreeMap<String, String>,
                    out: &mut Vec<(String, String)>| {
            for (name, value) in map.iter_mut() {
                if value.trim().is_empty() || secret_ref(value).is_some() || !name_is_secret(name) {
                    continue;
                }
                let sid = secret_id(&id, kind, name);
                out.push((sid.clone(), std::mem::take(value)));
                *value = format!("{}{}", SECRET_PREFIX, sid);
            }
        };
        let mut out = Vec::new();
        match &mut self.transport {
            Transport::Stdio { env, .. } => take("env", env, &mut out),
            Transport::Http { headers, .. } => take("header", headers, &mut out),
        }
        out
    }

    pub fn secret_ids(&self) -> Vec<String> {
        let values: Vec<&String> = match &self.transport {
            Transport::Stdio { env, .. } => env.values().collect(),
            Transport::Http { headers, .. } => headers.values().collect(),
        };
        values
            .into_iter()
            .filter_map(|v| secret_ref(v).map(|s| s.to_string()))
            .collect()
    }

    pub fn validate(&self) -> Result<(), McpError> {
        if !valid_id(&self.id) {
            return Err(McpError::new(
                ERR_MCP_SPAWN,
                format!(
                    "invalid server id {:?}: use lowercase letters, digits, - and _",
                    self.id
                ),
            ));
        }
        let (kind, map): (&str, &BTreeMap<String, String>) = match &self.transport {
            Transport::Stdio { command, env, .. } => {
                if command.trim().is_empty() {
                    return Err(McpError::new(ERR_MCP_SPAWN, "no command configured"));
                }
                ("env", env)
            }
            Transport::Http { url, headers } => {
                if url.trim().is_empty() {
                    return Err(McpError::new(ERR_MCP_SPAWN, "no URL configured"));
                }
                ("header", headers)
            }
        };
        for (name, value) in map {
            if name_is_secret(name) && !value.trim().is_empty() && secret_ref(value).is_none() {
                return Err(McpError::new(
                    ERR_MCP_SPAWN,
                    format!(
                        "{} {:?} holds a literal secret: it must be a secret:<id> reference",
                        kind, name
                    ),
                ));
            }
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn secret_ref(value: &str) -> Option<&str> {
    value
        .strip_prefix(SECRET_PREFIX)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

pub type SecretLookup =
    std::sync::Arc<dyn Fn(&str) -> Result<Option<String>, String> + Send + Sync>;

pub fn default_lookup() -> SecretLookup {
    std::sync::Arc::new(|id: &str| crate::core::secrets::get(SECRET_NS, id))
}

pub fn secret_id(server_id: &str, kind: &str, name: &str) -> String {
    let slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}-{}-{}", server_id, kind, slug)
}

pub fn name_is_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["token", "secret", "key", "password", "authorization"]
        .iter()
        .any(|word| n.contains(word))
}

fn resolve_value<F>(kind: &str, name: &str, value: &str, lookup: &F) -> Result<String, McpError>
where
    F: Fn(&str) -> Result<Option<String>, String> + ?Sized,
{
    match secret_ref(value) {
        None => Ok(value.to_string()),
        Some(id) => match lookup(id) {
            Ok(Some(secret)) => Ok(secret),
            Ok(None) => Err(McpError::new(
                ERR_MCP_HTTP,
                format!("{} {}: no secret stored under {:?}", kind, name, id),
            )),
            Err(e) => Err(McpError::new(
                ERR_MCP_HTTP,
                format!("{} {}: could not read the secret: {}", kind, name, e),
            )),
        },
    }
}

pub fn resolve_headers_with<F>(
    headers: &BTreeMap<String, String>,
    lookup: F,
) -> Result<Vec<(String, String)>, McpError>
where
    F: Fn(&str) -> Result<Option<String>, String>,
{
    let mut out = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        out.push((name.clone(), resolve_value("header", name, value, &lookup)?));
    }
    Ok(out)
}

pub fn resolve_headers(
    headers: &BTreeMap<String, String>,
    lookup: &SecretLookup,
) -> Result<Vec<(String, String)>, McpError> {
    resolve_headers_with(headers, |id| lookup(id))
}

pub fn resolve_env_with<F>(
    env: &BTreeMap<String, String>,
    lookup: F,
) -> Result<BTreeMap<String, String>, McpError>
where
    F: Fn(&str) -> Result<Option<String>, String>,
{
    let mut out = BTreeMap::new();
    for (name, value) in env {
        out.insert(name.clone(), resolve_value("env", name, value, &lookup)?);
    }
    Ok(out)
}

pub fn resolve_env(
    env: &BTreeMap<String, String>,
    lookup: &SecretLookup,
) -> Result<BTreeMap<String, String>, McpError> {
    resolve_env_with(env, |id| lookup(id))
}

pub fn redacted(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| {
            let hidden = secret_ref(v).is_some()
                || k.eq_ignore_ascii_case("authorization")
                || k.to_ascii_lowercase().contains("token")
                || k.to_ascii_lowercase().contains("api-key");
            (
                k.clone(),
                if hidden { "***".to_string() } else { v.clone() },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_has_the_four_fields() {
        let r = request(7, "tools/list", None);
        assert_eq!(r["jsonrpc"], "2.0");
        assert_eq!(r["id"], 7);
        assert_eq!(r["method"], "tools/list");
        assert!(
            r.get("params").is_none(),
            "no params key when there are none"
        );
    }

    #[test]
    fn a_notification_has_no_id() {
        let n = notification("notifications/initialized", None);
        assert!(n.get("id").is_none());
        assert_eq!(n["method"], "notifications/initialized");
    }

    #[test]
    fn initialize_params_carry_the_protocol_and_client() {
        let p = initialize_params("0.9.1");
        assert_eq!(p["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(p["clientInfo"]["name"], CLIENT_NAME);
        assert_eq!(p["clientInfo"]["version"], "0.9.1");
    }

    #[test]
    fn a_string_id_and_a_number_id_key_the_same_way() {
        assert_eq!(id_key(&json!(3)).unwrap(), "3");
        assert_eq!(id_key(&json!("3")).unwrap(), "3");
        assert_eq!(id_key(&Value::Null), None);
    }

    #[test]
    fn classify_sorts_results_failures_and_server_messages() {
        assert_eq!(
            classify(&json!({ "jsonrpc": "2.0", "id": 1, "result": { "a": 1 } })),
            Incoming::Result {
                id: "1".into(),
                result: json!({ "a": 1 })
            }
        );
        let Incoming::Failure { id, error } = classify(
            &json!({ "jsonrpc": "2.0", "id": "x", "error": { "code": -32601, "message": "nope" } }),
        ) else {
            panic!("should be a failure");
        };
        assert_eq!(id, "x");
        assert_eq!(error.code, -32601);
        assert_eq!(
            classify(&json!({ "jsonrpc": "2.0", "method": "notifications/message" })),
            Incoming::ServerMessage {
                method: "notifications/message".into()
            }
        );
        assert_eq!(classify(&json!({ "hello": 1 })), Incoming::Unknown);
    }

    #[test]
    fn a_server_request_is_not_mistaken_for_a_reply() {
        assert!(matches!(
            classify(&json!({ "jsonrpc": "2.0", "id": 9, "method": "sampling/createMessage" })),
            Incoming::ServerMessage { .. }
        ));
    }

    #[test]
    fn tools_parse_and_bad_entries_are_skipped() {
        let result = json!({ "tools": [
            { "name": "a", "description": "d", "inputSchema": { "type": "object" } },
            { "description": "no name" },
            { "name": "b" },
            "not an object"
        ] });
        let tools = parse_tools(&result);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "a");
        assert_eq!(tools[1].name, "b");
        assert!(tools[1].schema_is_object());
    }

    #[test]
    fn tooldef_round_trips_with_the_servers_field_names() {
        let t = ToolDef::new("x", "does x", json!({ "type": "object" }));
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["inputSchema"]["type"], "object");
        assert!(v.get("outputSchema").is_none(), "no null noise on the wire");
        assert_eq!(serde_json::from_value::<ToolDef>(v).unwrap(), t);
    }

    #[test]
    fn tool_errors_and_text_are_read_from_the_result() {
        let ok = json!({ "content": [{ "type": "text", "text": "hi" }], "isError": false });
        assert!(!is_tool_error(&ok));
        assert_eq!(tool_result_text(&ok), "hi");
        let bad = json!({ "content": [{ "type": "text", "text": "boom" }], "isError": true });
        assert!(is_tool_error(&bad));
        assert_eq!(tool_result_text(&bad), "boom");
        let img = json!({ "content": [{ "type": "image", "data": "..." }] });
        assert_eq!(tool_result_text(&img), "");
    }

    #[test]
    fn config_round_trips_through_json() {
        let cfg = McpServerConfig {
            id: "fs".into(),
            name: "Filesystem".into(),
            transport: Transport::Stdio {
                command: "npx".into(),
                args: vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into(),
                ],
                env: BTreeMap::new(),
                cwd: None,
            },
            enabled: true,
            timeout_ms: None,
        };
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(text.contains("\"kind\":\"stdio\""));
        assert_eq!(serde_json::from_str::<McpServerConfig>(&text).unwrap(), cfg);
    }

    #[test]
    fn a_config_missing_optional_fields_still_loads() {
        let cfg: McpServerConfig = serde_json::from_str(
            r#"{ "id": "h", "transport": { "kind": "http", "url": "https://example.com/mcp" } }"#,
        )
        .unwrap();
        assert!(cfg.enabled, "a server is enabled unless it says otherwise");
        assert_eq!(cfg.timeout(), std::time::Duration::from_millis(60_000));
        assert!(!cfg.is_stdio());
    }

    #[test]
    fn ids_are_slugs() {
        assert!(valid_id("github"));
        assert!(valid_id("my_server-2"));
        assert!(!valid_id(""));
        assert!(!valid_id("Github"));
        assert!(!valid_id("a b"));
        assert!(!valid_id(&"a".repeat(65)));
    }

    #[test]
    fn secret_headers_are_resolved_and_plain_ones_are_not() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "secret:gh-token".into());
        headers.insert("X-Plain".into(), "kept".into());
        let out = resolve_headers_with(&headers, |id| {
            assert_eq!(id, "gh-token");
            Ok(Some("Bearer real".into()))
        })
        .unwrap();
        assert!(out.contains(&("Authorization".into(), "Bearer real".into())));
        assert!(out.contains(&("X-Plain".into(), "kept".into())));
    }

    #[test]
    fn a_missing_secret_fails_the_connection_instead_of_sending_the_literal() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "secret:gone".into());
        let err = resolve_headers_with(&headers, |_| Ok(None)).unwrap_err();
        assert_eq!(err.code, ERR_MCP_HTTP);
        assert!(err.message.contains("gone"), "{}", err.message);
    }

    #[test]
    fn the_config_file_never_holds_the_secret_itself() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "secret:gh-token".into());
        let cfg = McpServerConfig {
            id: "gh".into(),
            name: "GitHub".into(),
            transport: Transport::Http {
                url: "https://api.githubcopilot.com/mcp/".into(),
                headers: headers.clone(),
            },
            enabled: true,
            timeout_ms: None,
        };
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(text.contains("secret:gh-token"));
        assert!(!text.contains("Bearer"), "{}", text);
        let shown = redacted(&headers);
        assert_eq!(shown["Authorization"], "***");
    }

    #[test]
    fn redaction_hides_tokens_even_when_they_are_literal() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "Bearer ghp_real".into());
        headers.insert("X-Api-Key".into(), "abcd".into());
        headers.insert("X-Trace".into(), "on".into());
        let shown = redacted(&headers);
        assert_eq!(shown["Authorization"], "***");
        assert_eq!(shown["X-Api-Key"], "***");
        assert_eq!(shown["X-Trace"], "on");
    }

    #[test]
    fn the_env_of_a_stdio_server_resolves_secrets_too() {
        let mut env = BTreeMap::new();
        env.insert("GITHUB_TOKEN".into(), "secret:gh-env-github_token".into());
        env.insert("NODE_ENV".into(), "production".into());
        let out = resolve_env_with(&env, |id| {
            assert_eq!(id, "gh-env-github_token");
            Ok(Some("ghp_real".into()))
        })
        .unwrap();
        assert_eq!(out["GITHUB_TOKEN"], "ghp_real");
        assert_eq!(out["NODE_ENV"], "production");
    }

    #[test]
    fn a_missing_env_secret_stops_the_spawn() {
        let mut env = BTreeMap::new();
        env.insert("GITHUB_TOKEN".into(), "secret:gone".into());
        let err = resolve_env_with(&env, |_| Ok(None)).unwrap_err();
        assert_eq!(err.code, ERR_MCP_HTTP);
        assert!(err.message.contains("env GITHUB_TOKEN"), "{}", err.message);
    }

    #[test]
    fn secret_looking_names_are_recognised() {
        for name in [
            "Authorization",
            "X-Api-Key",
            "GITHUB_TOKEN",
            "my_secret",
            "PASSWORD",
            "openai_api_key",
        ] {
            assert!(name_is_secret(name), "{}", name);
        }
        for name in ["NODE_ENV", "Content-Type", "X-Trace", "HOME"] {
            assert!(!name_is_secret(name), "{}", name);
        }
    }

    #[test]
    fn extract_secrets_moves_literals_out_of_the_config() {
        let mut cfg = McpServerConfig {
            id: "gh".into(),
            name: "GitHub".into(),
            transport: Transport::Http {
                url: "https://example.com/mcp".into(),
                headers: [
                    (
                        "Authorization".to_string(),
                        "Bearer ghp_literal".to_string(),
                    ),
                    ("X-Trace".to_string(), "on".to_string()),
                ]
                .into_iter()
                .collect(),
            },
            enabled: true,
            timeout_ms: None,
        };
        assert!(cfg.validate().is_err(), "a literal must not be saved");
        let taken = cfg.extract_secrets();
        assert_eq!(
            taken,
            vec![(
                "gh-header-authorization".to_string(),
                "Bearer ghp_literal".to_string()
            )]
        );
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(!text.contains("ghp_literal"), "{}", text);
        assert!(text.contains("secret:gh-header-authorization"));
        assert!(text.contains("\"X-Trace\":\"on\""), "{}", text);
        cfg.validate().unwrap();
        assert!(cfg.extract_secrets().is_empty());
        assert_eq!(
            cfg.secret_ids(),
            vec!["gh-header-authorization".to_string()]
        );
    }

    #[test]
    fn extract_secrets_covers_the_environment_of_a_stdio_server() {
        let mut cfg = McpServerConfig {
            id: "gh".into(),
            name: "GitHub".into(),
            transport: Transport::Stdio {
                command: "npx".into(),
                args: vec![],
                env: [
                    ("GITHUB_TOKEN".to_string(), "ghp_literal".to_string()),
                    ("NODE_ENV".to_string(), "production".to_string()),
                ]
                .into_iter()
                .collect(),
                cwd: None,
            },
            enabled: true,
            timeout_ms: None,
        };
        let taken = cfg.extract_secrets();
        assert_eq!(
            taken,
            vec![("gh-env-github_token".to_string(), "ghp_literal".to_string())]
        );
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(!text.contains("ghp_literal"), "{}", text);
        assert!(text.contains("secret:gh-env-github_token"));
        assert!(text.contains("production"));
        cfg.validate().unwrap();
        assert_eq!(
            cfg.secret_id_for("env", "GITHUB_TOKEN"),
            "gh-env-github_token"
        );
    }

    #[test]
    fn validate_refuses_an_empty_command_or_url_and_a_bad_id() {
        let mut cfg = McpServerConfig {
            id: "ok".into(),
            name: String::new(),
            transport: Transport::Stdio {
                command: "  ".into(),
                args: vec![],
                env: BTreeMap::new(),
                cwd: None,
            },
            enabled: true,
            timeout_ms: None,
        };
        assert_eq!(cfg.validate().unwrap_err().code, ERR_MCP_SPAWN);
        cfg.transport = Transport::Http {
            url: "  ".into(),
            headers: BTreeMap::new(),
        };
        assert_eq!(cfg.validate().unwrap_err().code, ERR_MCP_SPAWN);
        cfg.transport = Transport::Http {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::new(),
        };
        cfg.validate().unwrap();
        cfg.id = "Bad Id".into();
        assert_eq!(cfg.validate().unwrap_err().code, ERR_MCP_SPAWN);
    }

    #[test]
    fn secret_ref_needs_a_non_empty_id() {
        assert_eq!(secret_ref("secret:abc"), Some("abc"));
        assert_eq!(secret_ref("secret: abc "), Some("abc"));
        assert_eq!(secret_ref("secret:"), None);
        assert_eq!(secret_ref("plain"), None);
    }

    #[test]
    fn initialize_result_parses_and_reports_capabilities() {
        let r: InitializeResult = serde_json::from_value(json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "fixture", "version": "1.0.0" }
        }))
        .unwrap();
        assert_eq!(r.protocol_version, "2025-03-26");
        assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&r.protocol_version.as_str()));
        assert!(r.supports("tools"));
        assert!(!r.supports("prompts"));
        assert_eq!(r.server_info.name, "fixture");
    }
}
