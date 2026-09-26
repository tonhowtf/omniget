//! Packaged stdio adapter: forwards to the running desktop's own queue.
//! Credentials come from the client environment, never command-line arguments.
//!
//! Each stdin line is one JSON-RPC message or batch, POSTed as-is to the
//! desktop's `/mcp`: the answer (object or array) is written back unchanged,
//! so a batch behaves exactly as over HTTP. When the desktop does not answer,
//! the adapter itself replies once per request (an array for a batch) and
//! never to notifications or to the client's own responses.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
const MAX_MESSAGE: u64 = 65536;
/// Same bound as the desktop's HTTP batch limit (`mcp::BATCH_MAX`).
const BATCH_MAX: usize = 32;
const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");
#[tokio::main]
async fn main() -> Result<()> {
    let endpoint =
        std::env::var("OMNIGET_MCP_URL").unwrap_or_else(|_| "http://127.0.0.1:47720/mcp".into());
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
    let token = std::env::var("OMNIGET_MCP_TOKEN")
        .context("MCP_CONNECTION_REQUIRED: create a connection in OmniGet")?;
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    run(input, tokio::io::stdout(), url, token).await
}
async fn run<R, W>(mut input: R, mut output: W, url: reqwest::Url, token: String) -> Result<()>
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
        let reply = client
            .post(url.clone())
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", &version)
            .json(&msg)
            .send()
            .await;
        let value = match reply {
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
                let mut value =
                    serde_json::from_slice::<Value>(&bytes).context("INVALID_MCP_RESPONSE")?;
                annotate_health(&msg, &mut value, &version);
                Some(value)
            }
            Ok(response) => local_reply(&msg, Failure::Http(response.status().as_u16()), &version),
            Err(e) => local_reply(
                &msg,
                if e.is_connect() {
                    Failure::NotRunning
                } else {
                    Failure::OutcomeUnknown
                },
                &version,
            ),
        };
        let Some(value) = value else { continue };
        if let Some(v) = value["result"]["protocolVersion"].as_str() {
            version = v.to_string();
        }
        write(&mut output, &value).await?;
    }
    Ok(())
}
/// Why the desktop produced no JSON-RPC answer.
#[derive(Clone, Copy)]
enum Failure {
    NotRunning,
    Http(u16),
    OutcomeUnknown,
}
impl Failure {
    fn message(self) -> &'static str {
        match self {
            Failure::NotRunning => "APP_NOT_RUNNING",
            Failure::Http(401) => "UNAUTHORIZED",
            Failure::Http(403) => "MCP_DISABLED_OR_ACCESS_DENIED",
            Failure::Http(_) => "MCP_HTTP_ERROR",
            Failure::OutcomeUnknown => {
                "TRANSPORT_OUTCOME_UNKNOWN: inspect status before repeating a mutation"
            }
        }
    }
}
fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
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
    if matches!(failure, Failure::NotRunning) && is_health_call(msg) {
        let health = json!({
            "app": "not_running",
            "reachable": false,
            "queue": "unavailable",
            "message": "OmniGet is not running. Open the desktop app and keep the MCP server enabled.",
            "adapter": adapter_info(version, false),
        });
        let text = serde_json::to_string_pretty(&health).unwrap_or_default();
        return Some(
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":text}],"structuredContent":health,"isError":false}}),
        );
    }
    Some(rpc_error(id, -32000, failure.message()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal `/mcp` stand-in: answers each POST with `reply(body)`
    /// (`None` = 202 without body) and records the bodies it received.
    async fn fake_desktop(
        reply: fn(&Value) -> Option<Value>,
    ) -> (reqwest::Url, tokio::sync::mpsc::UnboundedReceiver<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url =
            reqwest::Url::parse(&format!("http://{}/mcp", listener.local_addr().unwrap())).unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    loop {
                        let mut head = Vec::new();
                        let mut b = [0u8; 1];
                        while !head.ends_with(b"\r\n\r\n") {
                            if s.read(&mut b).await.unwrap_or(0) == 0 {
                                return;
                            }
                            head.push(b[0]);
                        }
                        let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        let mut body = vec![0u8; len];
                        s.read_exact(&mut body).await.unwrap();
                        let msg: Value = serde_json::from_slice(&body).unwrap();
                        let _ = tx.send(msg.clone());
                        let response = match reply(&msg) {
                            Some(v) => {
                                let body = serde_json::to_vec(&v).unwrap();
                                let mut r = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n", body.len()).into_bytes();
                                r.extend(body);
                                r
                            }
                            None => b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\n\r\n".to_vec(),
                        };
                        if s.write_all(&response).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (url, rx)
    }

    /// Same answer rules as `mcp::handle_body`: one response per request,
    /// nothing for notifications/responses, an array for a batch.
    fn desktop_rules(msg: &Value) -> Option<Value> {
        let one = |m: &Value| -> Option<Value> {
            let id = request_id(m)?;
            Some(json!({"jsonrpc":"2.0","id":id,"result":{"echo":m["method"]}}))
        };
        match msg.as_array() {
            Some(items) => {
                let out: Vec<Value> = items.iter().filter_map(one).collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            None => one(msg),
        }
    }

    async fn drive(url: reqwest::Url, lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut out = Vec::new();
        run(
            tokio::io::BufReader::new(input.as_bytes()),
            &mut out,
            url,
            "t".into(),
        )
        .await
        .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn closed_port_url() -> reqwest::Url {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        reqwest::Url::parse(&format!("http://{addr}/mcp")).unwrap()
    }

    #[tokio::test]
    async fn d13_stdio_batch_is_forwarded_and_answered_like_http() {
        let (url, mut seen) = fake_desktop(desktop_rules).await;
        let batch = json!([
            {"jsonrpc":"2.0","id":"b1","method":"ping"},
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":"b2","method":"tools/list"}
        ]);
        let only_notifications = json!([{"jsonrpc":"2.0","method":"notifications/x"}]);
        let out = drive(
            url,
            &[
                batch.clone(),
                only_notifications.clone(),
                json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
            ],
        )
        .await;
        // The batch reached the desktop intact, as one POST.
        assert_eq!(seen.recv().await.unwrap(), batch);
        assert_eq!(seen.recv().await.unwrap(), only_notifications);
        assert_eq!(out.len(), 2, "{out:?}");
        let ids: Vec<&Value> = out[0]
            .as_array()
            .expect("batch answer is an array")
            .iter()
            .map(|r| &r["id"])
            .collect();
        assert_eq!(ids, [&json!("b1"), &json!("b2")]);
        assert_eq!(out[1]["id"], 3);
    }

    #[tokio::test]
    async fn d13_batch_rejection_from_desktop_is_relayed() {
        fn reject(_: &Value) -> Option<Value> {
            Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid request: empty batch"}}),
            )
        }
        let (url, _seen) = fake_desktop(reject).await;
        let out = drive(url, &[json!([])]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn d14_offline_never_answers_responses_or_notifications() {
        let out = drive(
            closed_port_url(),
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":5,"result":{}}),
                json!({"jsonrpc":"2.0","id":6,"error":{"code":1,"message":"x"}}),
                json!([
                    {"jsonrpc":"2.0","id":"b1","method":"tools/list"},
                    {"jsonrpc":"2.0","id":7,"result":{}},
                    {"jsonrpc":"2.0","method":"notifications/x"}
                ]),
                json!([{"jsonrpc":"2.0","id":8,"result":{}}]),
            ],
        )
        .await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(
            (&out[0]["id"], &out[0]["error"]["message"]),
            (&json!(1), &json!("APP_NOT_RUNNING"))
        );
        let batch = out[1].as_array().unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(
            (&batch[0]["id"], &batch[0]["error"]["message"]),
            (&json!("b1"), &json!("APP_NOT_RUNNING"))
        );
    }

    #[tokio::test]
    async fn d14_offline_health_reports_unreachable_app_version_and_protocol() {
        let out = drive(
            closed_port_url(),
            &[json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"omniget_health","arguments":{}}})],
        )
        .await;
        let result = &out[0]["result"];
        assert_eq!(result["isError"], false);
        let sc = &result["structuredContent"];
        assert_eq!(
            (&sc["app"], &sc["reachable"]),
            (&json!("not_running"), &json!(false))
        );
        assert_eq!(sc["adapter"]["version"], ADAPTER_VERSION);
        assert_eq!(sc["adapter"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            result["content"][0]["text"],
            serde_json::to_string_pretty(sc).unwrap()
        );
    }

    #[tokio::test]
    async fn d14_online_health_says_reachable() {
        fn health(msg: &Value) -> Option<Value> {
            let sc = json!({"app":"running","queue":"available","remote":"disabled"});
            Some(
                json!({"jsonrpc":"2.0","id":msg["id"],"result":{"content":[{"type":"text","text":sc.to_string()}],"structuredContent":sc,"isError":false}}),
            )
        }
        let (url, _seen) = fake_desktop(health).await;
        let out = drive(url, &[json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"omniget_health","arguments":{}}})]).await;
        let sc = &out[0]["result"]["structuredContent"];
        assert_eq!(
            (&sc["app"], &sc["reachable"], &sc["adapter"]["reachable"]),
            (&json!("running"), &json!(true), &json!(true))
        );
    }

    #[test]
    fn http_failures_map_per_request_and_keep_invalid_messages_visible() {
        let batch = json!([{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","id":2,"method":"initialize"},{"id":3}]);
        let out = local_reply(&batch, Failure::Http(401), "2025-06-18").unwrap();
        let out = out.as_array().unwrap();
        assert_eq!(out[0]["error"]["message"], "UNAUTHORIZED");
        assert_eq!(out[1]["error"]["code"], -32600);
        assert_eq!(
            (&out[2]["id"], &out[2]["error"]["code"]),
            (&Value::Null, &json!(-32600))
        );
        assert_eq!(
            local_reply(&json!([]), Failure::NotRunning, "x").unwrap()["error"]["code"],
            -32600
        );
        assert!(local_reply(
            &json!({"jsonrpc":"2.0","method":"n"}),
            Failure::Http(403),
            "x"
        )
        .is_none());
    }
}
