use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::LazyLock;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
const MAX_MESSAGE: u64 = 32 * 1024 * 1024;
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const BATCH_MAX: usize = 32;
const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:47720/mcp";
static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../mcp-catalog.json")).expect("valid mcp-catalog.json")
});
#[tokio::main]
async fn main() -> Result<()> {
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
        .connect_timeout(std::time::Duration::from_secs(10))
        // Idle limit between bytes, not a total: approvals and canvas waits
        // legitimately take up to 5 minutes, a hung app must not stall forever.
        .read_timeout(std::time::Duration::from_secs(330))
        .build()?;
    let mut version = "2025-06-18".to_string();
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_MESSAGE + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_MESSAGE {
            if line.last() != Some(&b'\n') {
                discard_line(&mut input).await?;
            }
            write(
                &mut output,
                &rpc_error(Value::Null, -32600, "MCP_MESSAGE_TOO_LARGE"),
            )
            .await?;
            continue;
        }
        let Ok(line) = String::from_utf8(line) else {
            write(
                &mut output,
                &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            )
            .await?;
            continue;
        };
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
                    Ok(response) if response.status().is_success() => {
                        match read_response(response).await {
                            Ok(mut value) => {
                                annotate_health(&msg, &mut value, &version);
                                Some(value)
                            }
                            Err(message) => Some(rpc_error(
                                msg.get("id").cloned().unwrap_or(Value::Null),
                                -32603,
                                message,
                            )),
                        }
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
async fn discard_line<R: AsyncBufRead + Unpin>(input: &mut R) -> Result<()> {
    loop {
        let buf = input.fill_buf().await?;
        if buf.is_empty() {
            return Ok(());
        }
        let (used, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        input.consume(used);
        if done {
            return Ok(());
        }
    }
}
async fn read_response(mut response: reqwest::Response) -> Result<Value, &'static str> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err("MCP_RESPONSE_TOO_LARGE");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "TRANSPORT_OUTCOME_UNKNOWN: MCP_RESPONSE_READ_FAILED")?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("MCP_RESPONSE_TOO_LARGE");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "INVALID_MCP_RESPONSE")
}
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
    fn code(self) -> &'static str {
        self.message().split(':').next().unwrap_or_default()
    }
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
fn request_id(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?;
    (msg["method"].is_string() && (id.is_string() || id.is_i64() || id.is_u64()))
        .then(|| id.clone())
}
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
fn offline_single(id: Value, msg: &Value, failure: Failure, version: &str) -> Value {
    let params = &msg["params"];
    match msg["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let supported = CATALOG["protocolVersions"].as_array();
            let requested = params["protocolVersion"].as_str();
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
fn annotate_health(msg: &Value, value: &mut Value, version: &str) {
    if !is_health_call(msg) || value.get("error").is_some() || value["result"]["isError"] == true {
        return;
    }
    // get_mut, not IndexMut: indexing would insert `result` into an error reply.
    let Some(sc) = value
        .get_mut("result")
        .and_then(|r| r.get_mut("structuredContent"))
        .and_then(Value::as_object_mut)
    else {
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn health_annotation_leaves_error_replies_alone() {
        let msg = json!({"method":"tools/call","params":{"name":"omniget_health"}});
        let mut reply = json!({"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"x"}});
        annotate_health(&msg, &mut reply, "1");
        assert!(reply.get("result").is_none());
    }
    #[tokio::test]
    async fn oversized_line_replies_error_and_loop_continues() {
        let mut data = vec![b'x'; MAX_MESSAGE as usize + 10];
        data.extend_from_slice(b"\nnot json\n");
        let mut out = Vec::new();
        std::env::remove_var("OMNIGET_DEV_ENDPOINT");
        let url = reqwest::Url::parse(DEFAULT_ENDPOINT).unwrap();
        run(&data[..], &mut out, url, None).await.unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["message"], "MCP_MESSAGE_TOO_LARGE");
        assert_eq!(lines[1]["error"]["code"], -32700);
    }
}
