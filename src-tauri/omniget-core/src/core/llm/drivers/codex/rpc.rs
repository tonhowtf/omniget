//! JSON-RPC client over the stdio of `codex app-server`.
//!
//! Framing (measured on codex-cli 0.156.0, `fixtures/app-server-*.jsonl`):
//! one JSON object per line, no `"jsonrpc"` field required either way.
//! `{id, method, params}` is a request, `{method, params}` a notification,
//! `{id, result}` / `{id, error: {code, message, data?}}` a response. The server
//! also adds `emittedAtMs` to notifications; unknown keys are ignored.
//!
//! The client is transport-agnostic (any `AsyncRead`/`AsyncWrite`), so the
//! tests drive it through `tokio::io::duplex` with a scripted peer.
//!
//! Rules copied from T3's `effect-codex-app-server` transport (estudo 76,
//! 01-server.md §1.3): sequential integer ids from 1, a pending map of
//! oneshots failed all at once when the stream ends, and at most
//! [`MAX_ACTIVE_SERVER_REQUESTS`] unanswered server requests; beyond that the
//! server gets `-32001` right away instead of an unbounded backlog.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// Unanswered server → client requests allowed at once.
pub const MAX_ACTIVE_SERVER_REQUESTS: usize = 32;

pub const CODE_PARSE_ERROR: i64 = -32700;
pub const CODE_INVALID_REQUEST: i64 = -32600;
pub const CODE_METHOD_NOT_FOUND: i64 = -32601;
pub const CODE_INVALID_PARAMS: i64 = -32602;
pub const CODE_INTERNAL: i64 = -32603;
pub const CODE_OVERLOADED: i64 = -32001;

/// What went wrong with one call.
#[derive(Debug, Clone, PartialEq)]
pub enum RpcError {
    /// The server answered with a JSON-RPC error.
    Remote {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    /// The process is gone (EOF, write failure, or closed on purpose).
    Closed(String),
    /// No answer in time.
    Timeout(String),
    /// The answer did not decode into the expected type.
    Decode(String),
}

impl RpcError {
    pub fn message(&self) -> String {
        match self {
            RpcError::Remote { message, .. } => message.clone(),
            RpcError::Closed(m) | RpcError::Timeout(m) | RpcError::Decode(m) => m.clone(),
        }
    }

    /// The server does not know this method (older or newer CLI). Codex
    /// answers `-32600 "Invalid request: unknown variant …"` rather than the
    /// standard `-32601`, so both are recognised.
    pub fn is_unknown_method(&self) -> bool {
        match self {
            RpcError::Remote { code, message, .. } => {
                *code == CODE_METHOD_NOT_FOUND
                    || (message.contains("unknown variant") && message.contains("expected one of"))
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Remote { code, message, .. } => write!(f, "codex error {code}: {message}"),
            RpcError::Closed(m) => write!(f, "codex app-server closed: {m}"),
            RpcError::Timeout(m) => write!(f, "codex app-server timed out: {m}"),
            RpcError::Decode(m) => write!(f, "codex answer did not decode: {m}"),
        }
    }
}

impl std::error::Error for RpcError {}

/// What the server pushed.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// Must be answered with [`RpcClient::respond`] or
    /// [`RpcClient::respond_error`], exactly once.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The stream ended. Sent once, last.
    Closed {
        reason: String,
    },
}

type Pending = Arc<Mutex<HashMap<i64, (String, oneshot::Sender<Result<Value, RpcError>>)>>>;

/// Cheap to clone; every clone talks to the same process.
#[derive(Clone)]
pub struct RpcClient {
    out: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: Arc<AtomicI64>,
    closed: Arc<AtomicBool>,
    active_server_requests: Arc<AtomicUsize>,
}

/// One classified line of the wire. Pure, so the framing rules are tested
/// without a process.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Value,
        result: Result<Value, RpcError>,
    },
    Invalid(String),
}

pub fn classify_line(line: &str) -> Option<Frame> {
    let line = line.trim_end_matches('\r');
    if line.trim().is_empty() {
        return None;
    }
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(Frame::Invalid(format!("not JSON: {e}"))),
    };
    let Some(obj) = value.as_object() else {
        return Some(Frame::Invalid("not an object".into()));
    };
    let id = obj
        .get("id")
        .filter(|v| v.is_string() || v.is_number())
        .cloned();
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    match (obj.get("method").and_then(Value::as_str), id) {
        (Some(method), Some(id)) => Some(Frame::Request {
            id,
            method: method.to_string(),
            params,
        }),
        (Some(method), None) => Some(Frame::Notification {
            method: method.to_string(),
            params,
        }),
        (None, Some(id)) => {
            if let Some(err) = obj.get("error") {
                let code = err
                    .get("code")
                    .and_then(Value::as_i64)
                    .unwrap_or(CODE_INTERNAL);
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string();
                Some(Frame::Response {
                    id,
                    result: Err(RpcError::Remote {
                        code,
                        message,
                        data: err.get("data").cloned(),
                    }),
                })
            } else {
                Some(Frame::Response {
                    id,
                    result: Ok(obj.get("result").cloned().unwrap_or(Value::Null)),
                })
            }
        }
        (None, None) => Some(Frame::Invalid("neither method nor id".into())),
    }
}

fn id_key(id: &Value) -> Option<i64> {
    id.as_i64()
        .or_else(|| id.as_str().and_then(|s| s.parse::<i64>().ok()))
}

impl RpcClient {
    /// Starts the reader and writer tasks. Everything the server pushes goes
    /// to `incoming`; the last message is always [`Incoming::Closed`].
    pub fn start<R, W>(reader: R, writer: W, incoming: mpsc::UnboundedSender<Incoming>) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));

        // Writer: one line per message, flushed, in order.
        {
            let closed = closed.clone();
            let pending = pending.clone();
            tokio::spawn(async move {
                let mut writer = writer;
                while let Some(line) = out_rx.recv().await {
                    if line.is_empty() {
                        // `close()`: end stdin so the server sees EOF.
                        break;
                    }
                    let ok = writer.write_all(line.as_bytes()).await.is_ok()
                        && writer.write_all(b"\n").await.is_ok()
                        && writer.flush().await.is_ok();
                    if !ok {
                        closed.store(true, Ordering::SeqCst);
                        fail_all(&pending, "write to codex app-server failed");
                        break;
                    }
                }
                let _ = writer.shutdown().await;
            });
        }

        let client = Self {
            out: out_tx,
            pending: pending.clone(),
            next_id: Arc::new(AtomicI64::new(1)),
            closed: closed.clone(),
            active_server_requests: active.clone(),
        };

        // Reader.
        {
            let client = client.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(reader).lines();
                let reason = loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            let Some(frame) = classify_line(&line) else {
                                continue;
                            };
                            match frame {
                                Frame::Response { id, result } => {
                                    let entry = id_key(&id).and_then(|k| {
                                        client.pending.lock().ok().and_then(|mut m| m.remove(&k))
                                    });
                                    match entry {
                                        Some((_, tx)) => {
                                            let _ = tx.send(result);
                                        }
                                        None => {
                                            tracing::debug!("[codex] response for unknown id {id}")
                                        }
                                    }
                                }
                                Frame::Notification { method, params } => {
                                    let _ =
                                        incoming.send(Incoming::Notification { method, params });
                                }
                                Frame::Request { id, method, params } => {
                                    let now = client
                                        .active_server_requests
                                        .fetch_add(1, Ordering::SeqCst);
                                    if now >= MAX_ACTIVE_SERVER_REQUESTS {
                                        client.respond_error(
                                            id,
                                            CODE_OVERLOADED,
                                            "Too many Codex requests are already active.",
                                        );
                                        continue;
                                    }
                                    let _ = incoming.send(Incoming::Request { id, method, params });
                                }
                                Frame::Invalid(why) => {
                                    // Never log the payload: it can hold file contents.
                                    tracing::debug!("[codex] dropped a line: {why}");
                                }
                            }
                        }
                        Ok(None) => break "end of stream".to_string(),
                        Err(e) => break format!("read failed: {e}"),
                    }
                };
                client.closed.store(true, Ordering::SeqCst);
                fail_all(&client.pending, &reason);
                let _ = incoming.send(Incoming::Closed { reason });
            });
        }

        client
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.out.is_closed()
    }

    fn send_line(&self, value: &Value) -> Result<(), RpcError> {
        if self.is_closed() {
            return Err(RpcError::Closed("the process is not running".into()));
        }
        self.out
            .send(value.to_string())
            .map_err(|_| RpcError::Closed("writer stopped".into()))
    }

    /// Sends a request and waits for its answer.
    pub async fn request_value(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut m) = self.pending.lock() {
            m.insert(id, (method.to_string(), tx));
        }
        let mut msg = json!({ "id": id, "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        if let Err(e) = self.send_line(&msg) {
            if let Ok(mut m) = self.pending.lock() {
                m.remove(&id);
            }
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError::Closed(format!(
                "{method}: no answer, the client closed"
            ))),
            Err(_) => {
                if let Ok(mut m) = self.pending.lock() {
                    m.remove(&id);
                }
                Err(RpcError::Timeout(format!(
                    "{method} after {} s",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// Typed request: params encoded, result decoded.
    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
        timeout: Duration,
    ) -> Result<R, RpcError> {
        let params = serde_json::to_value(params)
            .map_err(|e| RpcError::Decode(format!("{method} params: {e}")))?;
        let value = self.request_value(method, Some(params), timeout).await?;
        serde_json::from_value(value).map_err(|e| RpcError::Decode(format!("{method}: {e}")))
    }

    pub fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        let mut msg = json!({ "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        self.send_line(&msg)
    }

    /// Answers a server request.
    pub fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.release_server_request();
        self.send_line(&json!({ "id": id, "result": result }))
    }

    pub fn respond_error(&self, id: Value, code: i64, message: &str) {
        self.release_server_request();
        let _ = self.send_line(&json!({ "id": id, "error": { "code": code, "message": message } }));
    }

    fn release_server_request(&self) {
        let _ = self
            .active_server_requests
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// Stops accepting calls; pending ones fail with `Closed`.
    pub fn close(&self, reason: &str) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let _ = self.out.send(String::new());
        }
        fail_all(&self.pending, reason);
    }
}

fn fail_all(pending: &Pending, reason: &str) {
    let drained: Vec<_> = pending
        .lock()
        .map(|mut m| m.drain().collect())
        .unwrap_or_default();
    for (_, (method, tx)) in drained {
        let _ = tx.send(Err(RpcError::Closed(format!("{method}: {reason}"))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn classify_follows_the_codex_framing() {
        assert_eq!(classify_line("   "), None);
        assert!(matches!(
            classify_line(r#"{"id":1,"result":{"userAgent":"x"}}"#),
            Some(Frame::Response { result: Ok(_), .. })
        ));
        assert!(matches!(
            classify_line("{\"method\":\"turn/started\",\"params\":{},\"emittedAtMs\":1}\r"),
            Some(Frame::Notification { method, .. }) if method == "turn/started"
        ));
        assert!(matches!(
            classify_line(r#"{"id":"0","method":"item/fileChange/requestApproval","params":{}}"#),
            Some(Frame::Request { .. })
        ));
        match classify_line(
            r#"{"error":{"code":-32600,"message":"no active turn to interrupt"},"id":11}"#,
        ) {
            Some(Frame::Response {
                result: Err(RpcError::Remote { code, message, .. }),
                ..
            }) => {
                assert_eq!(code, -32600);
                assert_eq!(message, "no active turn to interrupt");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(classify_line("garbage"), Some(Frame::Invalid(_))));
    }

    #[test]
    fn codex_unknown_method_answer_is_recognised() {
        let e = RpcError::Remote {
            code: -32600,
            message:
                "Invalid request: unknown variant `thread/rollback`, expected one of `initialize`"
                    .into(),
            data: None,
        };
        assert!(e.is_unknown_method());
        let e = RpcError::Remote {
            code: -32600,
            message: "no rollout found for thread id x".into(),
            data: None,
        };
        assert!(!e.is_unknown_method());
    }

    #[tokio::test]
    async fn request_response_notification_and_server_request_round_trip() {
        let (client_io, server_io) = duplex(64 * 1024);
        let (cr, cw) = tokio::io::split(client_io);
        let (sr, mut sw) = tokio::io::split(server_io);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = RpcClient::start(cr, cw, tx);

        let peer = tokio::spawn(async move {
            let mut lines = BufReader::new(sr).lines();
            let first = lines.next_line().await.unwrap().unwrap();
            let v: Value = serde_json::from_str(&first).unwrap();
            assert_eq!(v["method"], "initialize");
            assert_eq!(v["id"], 1);
            sw.write_all(b"{\"method\":\"remoteControl/status/changed\",\"params\":{\"status\":\"disabled\"}}\n")
                .await
                .unwrap();
            sw.write_all(
                format!(
                    "{{\"id\":{},\"result\":{{\"userAgent\":\"omniget/0.156.0\"}}}}\n",
                    v["id"]
                )
                .as_bytes(),
            )
            .await
            .unwrap();
            sw.write_all(b"{\"id\":0,\"method\":\"item/commandExecution/requestApproval\",\"params\":{\"itemId\":\"i\"}}\n")
                .await
                .unwrap();
            let answer = lines.next_line().await.unwrap().unwrap();
            let a: Value = serde_json::from_str(&answer).unwrap();
            assert_eq!(a["id"], 0);
            assert_eq!(a["result"]["decision"], "accept");
            drop(sw);
        });

        let result = client
            .request_value(
                "initialize",
                Some(json!({"clientInfo": {"name": "t", "version": "1"}})),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(result["userAgent"], "omniget/0.156.0");
        match rx.recv().await.unwrap() {
            Incoming::Notification { method, .. } => {
                assert_eq!(method, "remoteControl/status/changed")
            }
            other => panic!("{other:?}"),
        }
        match rx.recv().await.unwrap() {
            Incoming::Request { id, method, .. } => {
                assert_eq!(method, "item/commandExecution/requestApproval");
                client.respond(id, json!({"decision": "accept"})).unwrap();
            }
            other => panic!("{other:?}"),
        }
        peer.await.unwrap();
        loop {
            match rx.recv().await {
                Some(Incoming::Closed { .. }) | None => break,
                _ => {}
            }
        }
        assert!(client.is_closed());
        let err = client
            .request_value("thread/start", None, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Closed(_)));
    }

    #[tokio::test]
    async fn pending_calls_fail_when_the_process_goes_away() {
        let (client_io, server_io) = duplex(1024);
        let (cr, cw) = tokio::io::split(client_io);
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = RpcClient::start(cr, cw, tx);
        let c2 = client.clone();
        let call = tokio::spawn(async move {
            c2.request_value("turn/start", Some(json!({})), Duration::from_secs(10))
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(server_io);
        let err = call.await.unwrap().unwrap_err();
        assert!(matches!(err, RpcError::Closed(_)), "{err:?}");
    }
}
