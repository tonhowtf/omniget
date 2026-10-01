//! Packaged stdio adapter: forwards to the running desktop's own queue.
//! Credentials come from the client environment, never command-line arguments.
//!
//! Each stdin line is one JSON-RPC message or batch, POSTed as-is to the
//! desktop's `/mcp`: the answer (object or array) is written back unchanged,
//! so a batch behaves exactly as over HTTP. When the desktop does not answer,
//! the adapter itself replies once per request (an array for a batch) and
//! never to notifications or to the client's own responses.
//!
//! Without a token, or while the desktop is closed, the adapter still speaks
//! MCP: `initialize`, `ping` and `tools/list` are answered from the catalog
//! snapshot the desktop's tests keep in sync (`mcp-catalog.json`), and every
//! tool call fails with `TOKEN_MISSING` or `APP_NOT_RUNNING` and how to fix it.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::LazyLock;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
const MAX_MESSAGE: u64 = 65536;
/// Same bound as the desktop's HTTP batch limit (`mcp::BATCH_MAX`).
const BATCH_MAX: usize = 32;
const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:47720/mcp";
/// Revisions, instructions and full tool catalog of the desktop, written and
/// checked by its `adapter_catalog_matches_the_desktop` test.
static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../mcp-catalog.json")).expect("valid mcp-catalog.json")
});
#[tokio::main]
async fn main() -> Result<()> {
    // An empty value counts as unset: MCPB hosts pass blank optional fields.
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let endpoint = env("OMNIGET_MCP_URL").unwrap_or_else(|| DEFAULT_ENDPOINT.into());
    let url = reqwest::Url::parse(&endpoint).context("INVALID_LOCAL_ENDPOINT")?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"))
        || url.path() != "/mcp"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
    {
        bail!("INVALID_LOCAL_ENDPOINT");
    }
    let token = env("OMNIGET_MCP_TOKEN");
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    run(input, tokio::io::stdout(), url, token).await
}
async fn run<R, W>(
    mut input: R,
    mut output: W,
    url: reqwest::Url,
    token: Option<String>,
) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut version = "2025-06-18".to_string();
    loop {
        let mut line = String::new();
        let count = (&mut input)
            .take(MAX_MESSAGE + 1)
            .read_line(&mut line)
            .await?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_MESSAGE {
            bail!("MCP_MESSAGE_TOO_LARGE");
        }
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(_) => {
                write(&mut output,&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})).await?;
                continue;
            }
        };
        // An isolated development instance (OMNIGET_DEV_ENDPOINT) is resolved
        // again on every call; it never falls back to the personal app.
        let target = match std::env::var("OMNIGET_DEV_ENDPOINT") {
            Ok(file) => match development_endpoint(&client, &file).await {
                Ok((dev_url, dev_token)) => Some((dev_url, Some(dev_token))),
                Err(_) => None,
            },
            Err(_) => Some((url.clone(), token.clone())),
        };
        let Some((request_url, request_token)) = target else {
            if let Some(value) = local_reply(&msg, Failure::NotRunning, &version) {
                write(&mut output, &value).await?;
            }
            continue;
        };
        let value = match request_token.as_deref() {
            // Without a token the desktop could only answer 401: stay local.
            None => local_reply(&msg, Failure::TokenMissing, &version),
            Some(token) => {
                let reply = client
                    .post(request_url)
                    .bearer_auth(token)
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", &version)
                    .json(&msg)
                    .send()
                    .await;
                match reply {
                    Ok(response) if response.status() == reqwest::StatusCode::ACCEPTED => continue,
                    Ok(mut response) if response.status().is_success() => {
                        if response.content_length().is_some_and(|n| n > 1024 * 1024) {
                            bail!("MCP_RESPONSE_TOO_LARGE");
                        }
                        let mut bytes = Vec::new();
                        while let Some(chunk) =
                            response.chunk().await.context("MCP_RESPONSE_READ_FAILED")?
                        {
                            if bytes.len() + chunk.len() > 1024 * 1024 {
                                bail!("MCP_RESPONSE_TOO_LARGE");
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        let mut value = serde_json::from_slice::<Value>(&bytes)
                            .context("INVALID_MCP_RESPONSE")?;
                        annotate_health(&msg, &mut value, &version);
                        Some(value)
                    }
                    Ok(response) => {
                        local_reply(&msg, Failure::Http(response.status().as_u16()), &version)
                    }
                    Err(e) => local_reply(
                        &msg,
                        if e.is_connect() {
                            Failure::NotRunning
                        } else {
                            Failure::OutcomeUnknown
                        },
                        &version,
                    ),
                }
            }
        };
        let Some(value) = value else { continue };
        if let Some(v) = value["result"]["protocolVersion"].as_str() {
            version = v.to_string();
        }
        write(&mut output, &value).await?;
    }
    Ok(())
}
/// Re-read an explicit isolated session descriptor on every call. Never fall back
/// to the personal app, and never retry mutations with an unknown outcome.
async fn development_endpoint(
    client: &reqwest::Client,
    file: &str,
) -> Result<(reqwest::Url, String)> {
    let path = std::path::Path::new(file);
    if !path.is_absolute() {
        bail!("INVALID_DEV_ENDPOINT");
    }
    let descriptor: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let state: Value = serde_json::from_slice(&std::fs::read(path.with_file_name("state.json"))?)?;
    if state["phase"] != "ready"
        || state["stale"] != false
        || state["bootId"] != descriptor["bootId"]
    {
        bail!("DEV_REVISION_NOT_READY");
    }
    let url = reqwest::Url::parse(descriptor["url"].as_str().context("INVALID_DEV_ENDPOINT")?)?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/mcp"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("INVALID_DEV_ENDPOINT");
    }
    let token = descriptor["token"]
        .as_str()
        .context("MISSING_DEV_TOKEN")?
        .to_string();
    let status: Value = client.post(url.clone()).bearer_auth(&token)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":"dev-identity","method":"tools/call","params":{"name":"omniget_app_status","arguments":{}}}))
        .send().await?.error_for_status()?.json().await?;
    let identity = &status["result"]["structuredContent"]["devIdentity"];
    if identity["sessionId"] != descriptor["sessionId"]
        || identity["bootId"] != descriptor["bootId"]
        || identity["sessionId"].is_null()
        || identity["bootId"].is_null()
    {
        bail!("DEV_INSTANCE_MISMATCH");
    }
    Ok((url, token))
}
/// Why the desktop produced no JSON-RPC answer.
#[derive(Clone, Copy)]
enum Failure {
    TokenMissing,
    NotRunning,
    Http(u16),
    OutcomeUnknown,
}
impl Failure {
    fn message(self) -> &'static str {
        match self {
            Failure::TokenMissing => {
                "TOKEN_MISSING: OMNIGET_MCP_TOKEN is not set. Open OmniGet, go to LLM → MCP → Your endpoint, turn the server on, create a connection and set its token as OMNIGET_MCP_TOKEN in this MCP server's environment."
            }
            Failure::NotRunning => {
                "APP_NOT_RUNNING: the OmniGet desktop app is not reachable. Open OmniGet and keep the server on in LLM → MCP → Your endpoint; OMNIGET_MCP_URL must match the address shown there."
            }
            Failure::Http(401) => "UNAUTHORIZED",
            Failure::Http(403) => "MCP_DISABLED_OR_ACCESS_DENIED",
            Failure::Http(_) => "MCP_HTTP_ERROR",
            Failure::OutcomeUnknown => {
                "TRANSPORT_OUTCOME_UNKNOWN: inspect status before repeating a mutation"
            }
        }
    }
    /// Machine-readable part of the message, before the first `:`.
    fn code(self) -> &'static str {
        self.message().split(':').next().unwrap_or_default()
    }
    /// Nothing reached the desktop: the adapter answers the handshake itself.
    fn offline(self) -> bool {
        matches!(self, Failure::TokenMissing | Failure::NotRunning)
    }
}
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn rpc_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
/// Request id of a message that expects an answer: a JSON-RPC request with a
/// string or integer id. Notifications and client responses have none.
fn request_id(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?;
    (msg["method"].is_string() && (id.is_string() || id.is_i64() || id.is_u64()))
        .then(|| id.clone())
}
/// A notification or a response the client sent: never answered.
fn silent(msg: &Value) -> bool {
    let Some(obj) = msg.as_object() else {
        return false;
    };
    if obj.get("jsonrpc") != Some(&json!("2.0")) {
        return false;
    }
    let is_response = !obj.contains_key("method")
        && obj.contains_key("id")
        && (obj.contains_key("result") || obj.contains_key("error"));
    let is_notification =
        obj.get("method").is_some_and(Value::is_string) && !obj.contains_key("id");
    is_response || is_notification
}
/// The adapter's own answer when the desktop did not produce one, shaped like
/// the HTTP answer would be: `None` when nothing is owed, an array for a batch.
fn local_reply(msg: &Value, failure: Failure, version: &str) -> Option<Value> {
    let Some(items) = msg.as_array() else {
        return local_single(msg, failure, version, false);
    };
    if items.is_empty() {
        return Some(rpc_error(
            Value::Null,
            -32600,
            "invalid request: empty batch",
        ));
    }
    if items.len() > BATCH_MAX {
        return Some(rpc_error(
            Value::Null,
            -32600,
            "invalid request: a batch holds at most 32 messages",
        ));
    }
    let out: Vec<Value> = items
        .iter()
        .filter_map(|item| local_single(item, failure, version, true))
        .collect();
    (!out.is_empty()).then_some(Value::Array(out))
}
fn local_single(msg: &Value, failure: Failure, version: &str, in_batch: bool) -> Option<Value> {
    if silent(msg) {
        return None;
    }
    let Some(id) = request_id(msg).filter(|_| msg["jsonrpc"] == "2.0") else {
        return Some(rpc_error(Value::Null, -32600, "invalid request"));
    };
    if in_batch && msg["method"] == "initialize" {
        return Some(rpc_error(
            id,
            -32600,
            "initialize cannot be part of a batch",
        ));
    }
    if failure.offline() {
        return Some(offline_single(id, msg, failure, version));
    }
    Some(rpc_error(id, -32000, failure.message()))
}
/// What the desktop would answer, minus anything that needs it: the
/// handshake and the catalog work, a tool call says what is missing.
fn offline_single(id: Value, msg: &Value, failure: Failure, version: &str) -> Value {
    let params = &msg["params"];
    match msg["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let supported = CATALOG["protocolVersions"].as_array();
            let requested = params["protocolVersion"].as_str();
            // Same rule as the desktop: the client's revision when supported,
            // otherwise the newest one.
            let negotiated = supported
                .and_then(|all| all.iter().find(|v| v.as_str() == requested))
                .or_else(|| supported.and_then(|all| all.first()))
                .cloned()
                .unwrap_or_else(|| json!(version));
            let instructions = format!(
                "{} {}",
                CATALOG["instructions"].as_str().unwrap_or_default(),
                failure.message()
            );
            rpc_ok(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "OmniGet", "version": ADAPTER_VERSION },
                    "instructions": instructions,
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": CATALOG["tools"] })),
        "resources/list" => rpc_ok(id, json!({ "resources": [] })),
        "prompts/list" => rpc_ok(id, json!({ "prompts": [] })),
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return rpc_error(id, -32602, "invalid params: `name` is required");
            };
            let known = CATALOG["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|t| t["name"] == name));
            if !known {
                return rpc_error(id, -32602, "unknown tool");
            }
            if matches!(failure, Failure::NotRunning) && name == "omniget_health" {
                return offline_health(id, version);
            }
            // The desktop's error envelope, so output schemas still validate.
            let error = json!({"error":{"code":failure.code(),"message":failure.message()}});
            rpc_ok(
                id,
                json!({"content":[{"type":"text","text":failure.message()}],"structuredContent":error,"isError":true}),
            )
        }
        _ => rpc_error(id, -32000, failure.message()),
    }
}
fn offline_health(id: Value, version: &str) -> Value {
    let health = json!({
        "app": "not_running",
        "reachable": false,
        "queue": "unavailable",
        "message": "OmniGet is not running. Open the desktop app and keep the MCP server enabled.",
        "adapter": adapter_info(version, false),
    });
    let text = serde_json::to_string_pretty(&health).unwrap_or_default();
    rpc_ok(
        id,
        json!({"content":[{"type":"text","text":text}],"structuredContent":health,"isError":false}),
    )
}
fn is_health_call(msg: &Value) -> bool {
    msg["method"] == "tools/call" && msg["params"]["name"] == "omniget_health"
}
fn adapter_info(version: &str, reachable: bool) -> Value {
    json!({"name":"omniget-mcp","version":ADAPTER_VERSION,"protocolVersion":version,"reachable":reachable})
}
/// `omniget_health` through the adapter also says the desktop was reached,
/// and which adapter and protocol revision carried the call.
fn annotate_health(msg: &Value, value: &mut Value, version: &str) {
    if !is_health_call(msg) || value["result"]["isError"] == true {
        return;
    }
    let Some(sc) = value["result"]["structuredContent"].as_object_mut() else {
        return;
    };
    sc.insert("reachable".into(), json!(true));
    sc.insert("adapter".into(), adapter_info(version, true));
    let text =
        serde_json::to_string_pretty(&value["result"]["structuredContent"]).unwrap_or_default();
    value["result"]["content"] = json!([{"type":"text","text":text}]);
}
async fn write<W: AsyncWrite + Unpin>(out: &mut W, v: &Value) -> Result<()> {
    out.write_all(serde_json::to_string(v)?.as_bytes()).await?;
    out.write_all(b"\n").await?;
    out.flush().await?;
    Ok(())
}
