//! JSON-RPC 2.0, newline-delimited, over an ACP agent's stdio. Our own
//! client, no SDK. Requests we send get numeric ids; the agent's requests
//! keep whatever id it chose (numbers, strings, even `0`) and are answered
//! with that exact value. Notifications we send never carry an `id` (Grok CLI
//! silently drops a `session/cancel` framed as a request).
//!
//! Everything the agent sends that is not a response to us (requests and
//! notifications) goes to one ordered channel, the same one the driver uses
//! for its own markers (`$/prompt_done`, `$/exit`), so a prompt's completion
//! is never processed before the updates that preceded it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::proc::{self, Spawn, Tail};
use crate::core::mcp::stdio::LineFramer;

/// One line may carry a whole file (`fs/write_text_file`).
const MAX_LINE: usize = 32 * 1024 * 1024;

/// JSON-RPC error codes the spec names.
pub const CODE_AUTH_REQUIRED: i64 = -32000;
pub const CODE_METHOD_NOT_FOUND: i64 = -32601;
pub const CODE_INVALID_PARAMS: i64 = -32602;
pub const CODE_INTERNAL: i64 = -32603;
pub const CODE_RESOURCE_NOT_FOUND: i64 = -32002;

/// Marker methods the driver injects into the inbound channel.
pub const EXIT: &str = "$/exit";
pub const PROMPT_DONE: &str = "$/prompt_done";

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn from_value(v: &Value) -> Self {
        Self {
            code: v
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(CODE_INTERNAL),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("agent error")
                .to_string(),
            data: v.get("data").cloned().filter(|d| !d.is_null()),
        }
    }

    /// The agent wants `authenticate` first (spec code, or the message says so:
    /// Gemini CLI answers `-32000 "Gemini API key is missing or not configured."`).
    pub fn is_auth_required(&self) -> bool {
        if self.code == CODE_AUTH_REQUIRED {
            return true;
        }
        let m = self.message.to_ascii_lowercase();
        [
            "authenticat",
            "not logged in",
            "login required",
            "log in",
            "sign in",
            "api key",
        ]
        .iter()
        .any(|k| m.contains(k))
    }

    fn to_value(&self) -> Value {
        let mut v = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            v["data"] = d.clone();
        }
        v
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// A request or notification from the agent, or a driver marker.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// `Some` for a request (answer it with [`Connection::respond`]).
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

type Waiter = oneshot::Sender<Result<Value, RpcError>>;
type Pending = Arc<StdMutex<HashMap<i64, Waiter>>>;

pub struct Connection {
    stdin: Mutex<Option<tokio::process::ChildStdin>>,
    child: StdMutex<Option<tokio::process::Child>>,
    pid: Option<u32>,
    pending: Pending,
    next_id: AtomicI64,
    dead: Arc<AtomicBool>,
    stderr: Tail,
    inbound: mpsc::UnboundedSender<Inbound>,
    /// Every line in and out, when protocol logging is on (tests, probes).
    log: Option<Arc<StdMutex<Vec<Value>>>>,
}

impl Connection {
    /// Start the agent and its reader. Inbound traffic goes to `inbound`.
    pub fn spawn(
        spec: &Spawn,
        inbound: mpsc::UnboundedSender<Inbound>,
        log: bool,
    ) -> Result<Arc<Self>, String> {
        let mut child = proc::spawn(spec)
            .map_err(|e| format!("could not start `{}`: {e}", proc::sanitize(&spec.command)))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr_pipe = child.stderr.take();
        let tail = Tail::default();
        if let Some(e) = stderr_pipe {
            proc::drain_into(e, tail.clone());
        }
        let conn = Arc::new(Self {
            stdin: Mutex::new(Some(stdin)),
            pid: child.id(),
            child: StdMutex::new(Some(child)),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            next_id: AtomicI64::new(1),
            dead: Arc::new(AtomicBool::new(false)),
            stderr: tail,
            inbound,
            log: log.then(|| Arc::new(StdMutex::new(Vec::new()))),
        });
        conn.clone().spawn_reader(stdout);
        Ok(conn)
    }

    fn record(&self, dir: &str, msg: &Value) {
        if let Some(log) = &self.log {
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(json!({ "dir": dir, "msg": msg }));
        }
    }

    /// The protocol log (`{dir: "in"|"out", msg}`), when enabled.
    pub fn take_log(&self) -> Vec<Value> {
        self.log
            .as_ref()
            .map(|l| std::mem::take(&mut *l.lock().unwrap_or_else(|e| e.into_inner())))
            .unwrap_or_default()
    }

    fn spawn_reader(self: Arc<Self>, stdout: tokio::process::ChildStdout) {
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut framer = LineFramer::new(MAX_LINE);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => framer.push(&buf[..n], |line| self.on_line(line)),
                }
            }
            self.dead.store(true, Ordering::SeqCst);
            let waiting: Vec<_> = self
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
                .collect();
            let tail = self.stderr.text();
            for (_, w) in waiting {
                let _ = w.send(Err(RpcError::new(
                    CODE_INTERNAL,
                    if tail.is_empty() {
                        "the ACP agent closed its output".to_string()
                    } else {
                        format!("the ACP agent closed its output: {tail}")
                    },
                )));
            }
            let code = {
                let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
                match child {
                    Some(mut c) => {
                        match tokio::time::timeout(Duration::from_secs(2), c.wait()).await {
                            Ok(Ok(st)) => st.code(),
                            _ => None,
                        }
                    }
                    None => None,
                }
            };
            let _ = self.inbound.send(Inbound {
                id: None,
                method: EXIT.into(),
                params: json!({ "code": code, "stderr": tail }),
            });
        });
    }

    fn on_line(&self, line: &[u8]) {
        let Ok(msg) = serde_json::from_slice::<Value>(line) else {
            // Agents sometimes print banners on stdout; keep them for errors.
            self.stderr.push(&String::from_utf8_lossy(line));
            self.stderr.push("\n");
            return;
        };
        self.record("in", &msg);
        if let Some(method) = msg.get("method").and_then(Value::as_str) {
            let _ = self.inbound.send(Inbound {
                id: msg.get("id").cloned().filter(|v| !v.is_null()),
                method: method.to_string(),
                params: msg.get("params").cloned().unwrap_or(Value::Null),
            });
            return;
        }
        let Some(id) = msg.get("id").and_then(Value::as_i64) else {
            return;
        };
        let waiter = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(match msg.get("error") {
                Some(err) if !err.is_null() => Err(RpcError::from_value(err)),
                _ => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            });
        }
    }

    async fn write(&self, msg: &Value) -> Result<(), RpcError> {
        self.record("out", msg);
        let mut line =
            serde_json::to_vec(msg).map_err(|e| RpcError::new(CODE_INTERNAL, e.to_string()))?;
        line.push(b'\n');
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| RpcError::new(CODE_INTERNAL, "the ACP agent is stopped"))?;
        stdin
            .write_all(&line)
            .await
            .map_err(|e| RpcError::new(CODE_INTERNAL, format!("write to agent: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| RpcError::new(CODE_INTERNAL, format!("write to agent: {e}")))
    }

    /// Send a request; the returned future resolves with its response.
    /// Split from the wait so a caller can fire `session/prompt` and return.
    pub async fn send_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<Result<Value, RpcError>>, RpcError> {
        if self.is_dead() {
            return Err(RpcError::new(CODE_INTERNAL, "the ACP agent is not running"));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write(&msg).await {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }
        Ok(rx)
    }

    /// Request + wait, with an optional timeout.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, RpcError> {
        let rx = self.send_request(method, params).await?;
        let waited = match timeout {
            Some(t) => tokio::time::timeout(t, rx).await.map_err(|_| {
                RpcError::new(
                    CODE_INTERNAL,
                    format!("`{method}` got no answer in {} s", t.as_secs()),
                )
            })?,
            None => rx.await,
        };
        waited.unwrap_or_else(|_| Err(RpcError::new(CODE_INTERNAL, "the ACP agent went away")))
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    /// Answer a request of the agent with its own id.
    pub async fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        let msg = match result {
            Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e.to_value() }),
        };
        let _ = self.write(&msg).await;
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr.text()
    }

    /// Push a driver marker into the inbound stream (after everything already
    /// received).
    pub fn inject(&self, method: &str, params: Value) {
        let _ = self.inbound.send(Inbound {
            id: None,
            method: method.to_string(),
            params,
        });
    }

    /// Close stdin and kill the whole process tree.
    pub async fn shutdown(&self) {
        self.stdin.lock().await.take();
        proc::kill_tree(self.pid);
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.start_kill();
        }
        self.dead.store(true, Ordering::SeqCst);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.dead.load(Ordering::SeqCst) {
            proc::kill_tree(self.pid);
        }
    }
}
