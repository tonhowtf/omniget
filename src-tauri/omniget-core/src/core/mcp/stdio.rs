use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::error::McpError;
use super::types::{classify, id_key, notification, request, resolve_env, Incoming, SecretLookup};

pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

const STDERR_TAIL_LINES: usize = 20;

#[derive(Debug)]
pub struct LineFramer {
    buf: Vec<u8>,
    max: usize,
    dropping: bool,
    dropped: usize,
}

impl LineFramer {
    pub fn new(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            max,
            dropping: false,
            dropped: 0,
        }
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn push(&mut self, bytes: &[u8], mut on_line: impl FnMut(&[u8])) {
        let mut rest = bytes;
        while let Some(nl) = rest.iter().position(|b| *b == b'\n') {
            let (head, tail) = rest.split_at(nl);
            if self.dropping {
                self.dropping = false;
                self.buf.clear();
            } else if self.buf.len() + head.len() > self.max {
                self.dropped += 1;
                self.buf.clear();
            } else if self.buf.is_empty() {
                on_line(trim_cr(head));
            } else {
                self.buf.extend_from_slice(head);
                let line = std::mem::take(&mut self.buf);
                on_line(trim_cr(&line));
            }
            rest = &tail[1..];
        }
        if rest.is_empty() || self.dropping {
            return;
        }
        if self.buf.len() + rest.len() > self.max {
            self.dropped += 1;
            self.dropping = true;
            self.buf.clear();
        } else {
            self.buf.extend_from_slice(rest);
        }
    }

    pub fn finish(&mut self, mut on_line: impl FnMut(&[u8])) {
        if self.dropping {
            self.dropping = false;
        } else if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            on_line(trim_cr(&line));
        }
        self.buf.clear();
    }
}

fn trim_cr(line: &[u8]) -> &[u8] {
    match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    }
}

type Pending = Arc<StdMutex<HashMap<String, oneshot::Sender<Result<Value, McpError>>>>>;

pub struct StdioTransport {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending: Pending,
    next_id: AtomicI64,
    stderr_tail: Arc<StdMutex<VecDeque<String>>>,
    dead: Arc<AtomicBool>,
    tasks: StdMutex<Vec<JoinHandle<()>>>,
}

impl StdioTransport {
    pub async fn connect(
        command: &str,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
        cwd: Option<&str>,
        lookup: &SecretLookup,
    ) -> Result<Self, McpError> {
        if command.trim().is_empty() {
            return Err(McpError::spawn("no command configured"));
        }
        let env = resolve_env(env, lookup)?;
        let mut cmd = crate::core::process::command(command);
        cmd.args(args);
        for (k, v) in &env {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd.filter(|d| !d.trim().is_empty()) {
            cmd.current_dir(dir);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| McpError::spawn(format!("could not start {:?}: {}", command, e)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::spawn("child has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::spawn("child has no stdout"))?;
        let stderr = child.stderr.take();

        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let stderr_tail = Arc::new(StdMutex::new(VecDeque::new()));
        let dead = Arc::new(AtomicBool::new(false));

        let mut tasks = Vec::new();
        if let Some(stderr) = stderr {
            let tail = stderr_tail.clone();
            tasks.push(tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut framer = LineFramer::new(64 * 1024);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => framer.push(&buf[..n], |line| {
                            if let Ok(mut t) = tail.lock() {
                                if t.len() == STDERR_TAIL_LINES {
                                    t.pop_front();
                                }
                                t.push_back(String::from_utf8_lossy(line).into_owned());
                            }
                        }),
                    }
                }
            }));
        }

        {
            let pending = pending.clone();
            let dead = dead.clone();
            let tail = stderr_tail.clone();
            tasks.push(tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut framer = LineFramer::new(MAX_LINE_BYTES);
                let mut buf = vec![0u8; 16 * 1024];
                let mut dropped = 0usize;
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => framer.push(&buf[..n], |line| dispatch(line, &pending)),
                    }
                    if framer.dropped() > dropped {
                        dropped = framer.dropped();
                        fail_all(
                            &pending,
                            McpError::proto(format!(
                                "the MCP server sent a message over the {} byte ceiling: dropped",
                                MAX_LINE_BYTES
                            )),
                        );
                    }
                }
                framer.finish(|line| dispatch(line, &pending));
                dead.store(true, Ordering::SeqCst);
                let tail = tail_text(&tail);
                fail_all(
                    &pending,
                    McpError::proto(format!(
                        "the MCP server closed its output before answering{}",
                        tail
                    )),
                );
            }));
        }

        Ok(Self {
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            pending,
            next_id: AtomicI64::new(1),
            stderr_tail,
            dead,
            tasks: StdMutex::new(tasks),
        })
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    pub fn stderr_tail(&self) -> String {
        tail_text(&self.stderr_tail)
    }

    async fn write_line(&self, msg: &Value) -> Result<(), McpError> {
        let mut line = serde_json::to_vec(msg)
            .map_err(|e| McpError::proto(format!("could not encode the request: {}", e)))?;
        line.push(b'\n');
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| McpError::proto("the connection to the MCP server is closed"))?;
        stdin.write_all(&line).await.map_err(|e| {
            McpError::proto(format!(
                "could not write to the MCP server: {}{}",
                e,
                self.stderr_tail()
            ))
        })?;
        stdin
            .flush()
            .await
            .map_err(|e| McpError::proto(format!("could not flush to the MCP server: {}", e)))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        if self.is_dead() {
            return Err(McpError::proto("the MCP server is not running"));
        }
        self.write_line(&notification(method, params)).await
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value, McpError> {
        if self.is_dead() {
            return Err(McpError::proto(format!(
                "the MCP server is not running{}",
                self.stderr_tail()
            )));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let key = id_key(&Value::from(id)).unwrap_or_else(|| id.to_string());
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(key.clone(), tx);
        }
        if let Err(e) = self.write_line(&request(id, method, params)).await {
            self.forget(&key);
            return Err(e);
        }
        let cancelled = async {
            match cancel {
                Some(c) => c.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            received = rx => match received {
                Ok(result) => result,
                Err(_) => Err(McpError::proto(format!(
                    "the MCP server stopped answering{}",
                    self.stderr_tail()
                ))),
            },
            _ = tokio::time::sleep(timeout) => {
                self.forget(&key);
                Err(McpError::timeout(format!(
                    "{} took longer than {} ms",
                    method,
                    timeout.as_millis()
                )))
            }
            _ = cancelled => {
                self.forget(&key);
                Err(McpError::cancelled())
            }
        }
    }

    fn forget(&self, key: &str) {
        if let Ok(mut map) = self.pending.lock() {
            map.remove(key);
        }
    }

    pub async fn close(&self) {
        {
            let mut guard = self.stdin.lock().await;
            *guard = None;
        }
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
        }
        self.dead.store(true, Ordering::SeqCst);
        fail_all(
            &self.pending,
            McpError::proto("the connection to the MCP server was closed"),
        );
        if let Ok(mut tasks) = self.tasks.lock() {
            for t in tasks.drain(..) {
                t.abort();
            }
        }
    }
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioTransport")
            .field("dead", &self.is_dead())
            .finish()
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            for t in tasks.drain(..) {
                t.abort();
            }
        }
    }
}

fn tail_text(tail: &Arc<StdMutex<VecDeque<String>>>) -> String {
    let lines: Vec<String> = tail
        .lock()
        .map(|t| t.iter().cloned().collect())
        .unwrap_or_default();
    let joined = lines.join("\n");
    let joined = joined.trim();
    if joined.is_empty() {
        String::new()
    } else {
        format!(" (stderr: {})", super::error::clip(joined, 500))
    }
}

fn dispatch(line: &[u8], pending: &Pending) {
    if line.iter().all(|b| b.is_ascii_whitespace()) {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let (key, outcome) = match classify(&value) {
        Incoming::Result { id, result } => (id, Ok(result)),
        Incoming::Failure { id, error } => (
            id,
            Err(McpError::proto(format!(
                "JSON-RPC error {}: {}",
                error.code, error.message
            ))),
        ),
        Incoming::ServerMessage { .. } | Incoming::Unknown => return,
    };
    let sender = pending.lock().ok().and_then(|mut m| m.remove(&key));
    if let Some(tx) = sender {
        let _ = tx.send(outcome);
    }
}

fn fail_all(pending: &Pending, err: McpError) {
    let waiting: Vec<_> = pending
        .lock()
        .map(|mut m| m.drain().map(|(_, tx)| tx).collect())
        .unwrap_or_default();
    for tx in waiting {
        let _ = tx.send(Err(err.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn framed(chunks: &[&[u8]], max: usize) -> (Vec<String>, usize) {
        let mut f = LineFramer::new(max);
        let mut out = Vec::new();
        for c in chunks {
            f.push(c, |l| out.push(String::from_utf8_lossy(l).into_owned()));
        }
        f.finish(|l| out.push(String::from_utf8_lossy(l).into_owned()));
        (out, f.dropped())
    }

    #[test]
    fn framing_splits_on_newlines_and_survives_chunk_boundaries() {
        let (lines, dropped) = framed(&[b"{\"a\":1}\n{\"b\"", b":2}\n"], MAX_LINE_BYTES);
        assert_eq!(lines, ["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn framing_strips_the_cr_of_crlf() {
        let (lines, _) = framed(&[b"a\r\nb\n"], MAX_LINE_BYTES);
        assert_eq!(lines, ["a", "b"]);
    }

    #[test]
    fn framing_flushes_a_last_line_without_a_newline() {
        let (lines, _) = framed(&[b"a\nb"], MAX_LINE_BYTES);
        assert_eq!(lines, ["a", "b"]);
    }

    #[test]
    fn framing_drops_an_oversized_line_and_keeps_going() {
        let big = vec![b'x'; 100];
        let (lines, dropped) = framed(&[b"ok\n", &big, b"\ngood\n"], 10);
        assert_eq!(lines, ["ok", "good"]);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn framing_caps_a_line_that_never_ends() {
        let chunk = vec![b'x'; 64];
        let mut f = LineFramer::new(100);
        let mut out = Vec::new();
        for _ in 0..50 {
            f.push(&chunk, |l| out.push(l.len()));
        }
        assert!(out.is_empty());
        assert_eq!(f.dropped(), 1);
        assert!(f.buf.len() <= 100, "buffered {}", f.buf.len());
    }

    #[test]
    fn dispatch_wakes_the_waiting_call_by_id() {
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert("4".into(), tx);
        dispatch(
            br#"{"jsonrpc":"2.0","id":4,"result":{"ok":true}}"#,
            &pending,
        );
        assert_eq!(rx.blocking_recv().unwrap().unwrap(), json!({ "ok": true }));
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn dispatch_matches_a_string_id_against_a_numeric_request() {
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert("4".into(), tx);
        dispatch(br#"{"jsonrpc":"2.0","id":"4","result":{}}"#, &pending);
        assert!(rx.blocking_recv().unwrap().is_ok());
    }

    #[test]
    fn dispatch_turns_a_json_rpc_error_into_err_mcp_proto() {
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().unwrap().insert("1".into(), tx);
        dispatch(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#,
            &pending,
        );
        let err = rx.blocking_recv().unwrap().unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_PROTO);
        assert!(err.message.contains("-32601"), "{}", err.message);
    }

    #[test]
    fn dispatch_ignores_noise_on_stdout() {
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, mut rx) = oneshot::channel::<Result<Value, McpError>>();
        pending.lock().unwrap().insert("1".into(), tx);
        dispatch(b"npm WARN using --force", &pending);
        dispatch(b"", &pending);
        dispatch(
            br#"{"jsonrpc":"2.0","method":"notifications/message"}"#,
            &pending,
        );
        dispatch(br#"{"jsonrpc":"2.0","id":99,"result":{}}"#, &pending);
        assert!(rx.try_recv().is_err(), "nothing should have answered it");
        assert_eq!(pending.lock().unwrap().len(), 1);
    }

    #[test]
    fn fail_all_hands_the_same_error_to_everyone_waiting() {
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        pending.lock().unwrap().insert("1".into(), tx1);
        pending.lock().unwrap().insert("2".into(), tx2);
        fail_all(&pending, McpError::proto("gone"));
        assert_eq!(rx1.blocking_recv().unwrap().unwrap_err().message, "gone");
        assert_eq!(rx2.blocking_recv().unwrap().unwrap_err().message, "gone");
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn the_stderr_tail_keeps_only_the_last_lines() {
        let tail: Arc<StdMutex<VecDeque<String>>> = Arc::new(StdMutex::new(VecDeque::new()));
        assert_eq!(tail_text(&tail), "");
        if let Ok(mut t) = tail.lock() {
            t.push_back("boom".into());
        }
        assert!(tail_text(&tail).contains("boom"));
    }
}
