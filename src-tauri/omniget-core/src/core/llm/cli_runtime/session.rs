use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::super::error::{LlmError, ERR_LLM_CANCELLED};
use super::super::types::{FinishReason, TurnEvent};
use super::parse::{classify_error_text, CliSignal, RateSnapshot};
use super::{ERR_CLI_EXIT, ERR_CLI_NO_START, ERR_CLI_SPAWN, ERR_CLI_STALLED};

const KILL_GRACE_MS: u64 = 150;
const STDERR_TAIL_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<String>,
    pub idle_timeout: Option<Duration>,
    pub tool_idle_timeout: Option<Duration>,
    pub start_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogConfig {
    pub idle: Option<Duration>,
    pub tool_idle: Option<Duration>,
    pub start: Option<Duration>,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            idle: Some(Duration::from_secs(600)),
            tool_idle: Some(Duration::from_secs(1800)),
            start: Some(Duration::from_secs(90)),
        }
    }
}

impl WatchdogConfig {
    pub fn apply(&self, spec: &mut SpawnSpec) {
        spec.idle_timeout = self.idle;
        spec.tool_idle_timeout = self.tool_idle;
        spec.start_timeout = self.start;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stall {
    NoStart,
    Idle,
    ToolIdle,
}

fn watch_window(
    spec: &SpawnSpec,
    saw_line: bool,
    tool_in_flight: bool,
) -> Option<(Duration, Stall)> {
    if !saw_line {
        return spec
            .start_timeout
            .or(spec.idle_timeout)
            .map(|d| (d, Stall::NoStart));
    }
    if tool_in_flight {
        return spec
            .tool_idle_timeout
            .or(spec.idle_timeout)
            .map(|d| (d, Stall::ToolIdle));
    }
    spec.idle_timeout.map(|d| (d, Stall::Idle))
}

fn pause_poll(window: Duration) -> Duration {
    (window / 4).clamp(Duration::from_millis(10), Duration::from_secs(1))
}

fn stall_error(kind: Stall, window: Duration) -> LlmError {
    let secs = window.as_secs_f64();
    match kind {
        Stall::NoStart => LlmError::new(
            ERR_CLI_NO_START,
            format!(
                "the CLI printed nothing in the first {secs:.0} s after it started; its process tree was stopped"
            ),
        ),
        Stall::Idle => LlmError::new(
            ERR_CLI_STALLED,
            format!("the CLI printed nothing for {secs:.0} s; its process tree was stopped"),
        ),
        Stall::ToolIdle => LlmError::new(
            ERR_CLI_STALLED,
            format!(
                "a tool of the CLI gave no answer and nothing was printed for {secs:.0} s; its process tree was stopped"
            ),
        ),
    }
}

pub trait LineParser: Send + Sync {
    fn parse_line(&self, line: &str) -> Vec<CliSignal>;

    fn duplex(&self) -> Option<Duplex> {
        None
    }
}

pub struct Duplex {
    pub outbox: tokio::sync::mpsc::UnboundedReceiver<String>,
    pub interrupt: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
}

pub const INTERRUPT_GRACE: Duration = Duration::from_millis(800);

pub type RateSink = Arc<dyn Fn(RateSnapshot) + Send + Sync>;

pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

pub type SpawnedHook = Arc<dyn Fn(Option<u32>, &str) + Send + Sync>;

#[derive(Default, Clone)]
pub struct TurnHooks {
    pub rate: Option<RateSink>,
    pub session: Option<Arc<dyn Fn(String) + Send + Sync>>,
    pub spawned: Option<SpawnedHook>,
    pub cleanup: Option<Arc<dyn Fn() + Send + Sync>>,
    pub paused: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

#[derive(Default)]
struct LineReader {
    buf: Vec<u8>,
    oversized: bool,
}

impl LineReader {
    async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(
        &mut self,
        reader: &mut R,
        max: usize,
    ) -> std::io::Result<Option<(String, bool)>> {
        loop {
            let chunk = reader.fill_buf().await?;
            if chunk.is_empty() {
                if self.buf.is_empty() && !self.oversized {
                    return Ok(None);
                }
                break;
            }
            let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
                Some(i) => (i + 1, true),
                None => (chunk.len(), false),
            };
            if !self.oversized {
                if self.buf.len() + take > max {
                    self.oversized = true;
                    self.buf.clear();
                } else {
                    self.buf.extend_from_slice(&chunk[..take]);
                }
            }
            reader.consume(take);
            if done {
                break;
            }
        }
        if std::mem::take(&mut self.oversized) {
            self.buf.clear();
            return Ok(Some((String::new(), true)));
        }
        let mut line = std::mem::take(&mut self.buf);
        while matches!(line.last(), Some(b'\n') | Some(b'\r')) {
            line.pop();
        }
        Ok(Some((String::from_utf8_lossy(&line).into_owned(), false)))
    }
}

async fn kill_tree(
    child: &mut tokio::process::Child,
    tree: &mut crate::core::process::ProcessTreeGuard,
) {
    let mut grace = std::time::Duration::from_millis(KILL_GRACE_MS);
    if tree.interrupt() {
        grace /= 2;
        let _ = tokio::time::timeout(grace, child.wait()).await;
    }
    tree.terminate();
    let _ = child.start_kill();
    let _ = tokio::time::timeout(grace, child.wait()).await;
}

fn secret_values() -> &'static std::sync::Mutex<Vec<String>> {
    static VALUES: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    VALUES.get_or_init(Default::default)
}

pub fn register_secret_value(value: &str) {
    let value = value.trim();
    if value.len() < 16 {
        return;
    }
    let mut values = secret_values().lock().unwrap_or_else(|e| e.into_inner());
    if !values.iter().any(|v| v == value) {
        values.push(value.to_string());
    }
}

fn holds_secret(value: &str, secrets: &[String]) -> bool {
    secrets.iter().any(|s| value.contains(s.as_str()))
}

pub fn spawn_turn_with(
    spec: SpawnSpec,
    parser: Arc<dyn LineParser>,
    cancel: CancellationToken,
    hooks: TurnHooks,
) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
    let launch_id = uuid::Uuid::new_v4().to_string();
    let mut cmd = crate::core::process::command(&spec.program);
    cmd.args(&spec.args);
    for key in &spec.scrub {
        cmd.env_remove(key);
    }
    let secrets = secret_values()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if !secrets.is_empty() {
        for (key, value) in std::env::vars_os() {
            if value.to_str().is_some_and(|v| holds_secret(v, &secrets)) {
                cmd.env_remove(key);
            }
        }
    }
    for (key, value) in &spec.env {
        if holds_secret(value, &secrets) {
            tracing::warn!("[cli] {key} carries a registered secret: not passed to the child");
            continue;
        }
        cmd.env(key, value);
    }
    cmd.env("OMNIGET_LAUNCH_ID", &launch_id);
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    crate::core::process::suspend_on_spawn(&mut cmd);
    tracing::info!(
        "[cli] spawn {} {} (cwd {:?}, env {:?})",
        spec.program.display(),
        spec.args
            .iter()
            .filter(|a| a.len() < 80)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
        spec.cwd,
        spec.env.keys().collect::<Vec<_>>()
    );
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match crate::core::process::spawn_retrying_busy(|| cmd.spawn()) {
        Ok(c) => c,
        Err(e) => {
            if let Some(cleanup) = &hooks.cleanup {
                cleanup();
            }
            return Err(LlmError::new(
                ERR_CLI_SPAWN,
                format!("cannot start {}: {e}", spec.program.display()),
            ));
        }
    };
    let mut tree = match crate::core::process::ProcessTreeGuard::new(&child) {
        Ok(tree) => tree,
        Err(e) => {
            let _ = child.start_kill();
            if let Some(cleanup) = &hooks.cleanup {
                cleanup();
            }
            return Err(LlmError::new(
                ERR_CLI_SPAWN,
                format!("cannot contain {}: {e}", spec.program.display()),
            ));
        }
    };
    let pid = child.id();
    if let Some(spawned) = &hooks.spawned {
        spawned(pid, &launch_id);
    }

    let Some(stdout) = child.stdout.take() else {
        tree.terminate();
        if let Some(cleanup) = &hooks.cleanup {
            cleanup();
        }
        return Err(LlmError::new(
            ERR_CLI_SPAWN,
            "the CLI gave no stdout".to_string(),
        ));
    };
    let stderr = child.stderr.take();
    let mut stdin = child.stdin.take();

    let (mut tx, rx) = mpsc::channel::<TurnEvent>(64);

    tokio::spawn(async move {
        struct Cleanup(Option<Arc<dyn Fn() + Send + Sync>>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(f) = self.0.take() {
                    f();
                }
            }
        }
        let _cleanup = Cleanup(hooks.cleanup.clone());

        let mut duplex = parser.duplex();
        let mut pipe_open: Option<tokio::process::ChildStdin> = None;
        if let Some(mut pipe) = stdin.take() {
            if let Some(text) = &spec.stdin {
                let _ = pipe.write_all(text.as_bytes()).await;
            }
            if duplex.is_some() {
                let _ = pipe.flush().await;
                pipe_open = Some(pipe);
            } else {
                drop(pipe);
            }
        }
        let mut outbox_open = duplex.is_some();

        let stderr_task = stderr.map(|mut pipe| {
            tokio::spawn(async move {
                let mut tail: Vec<u8> = Vec::new();
                let mut chunk = vec![0u8; 8192];
                loop {
                    match pipe.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            tail.extend_from_slice(&chunk[..n]);
                            if tail.len() > STDERR_TAIL_BYTES * 2 {
                                tail = tail.split_off(tail.len() - STDERR_TAIL_BYTES);
                            }
                        }
                    }
                }
                if tail.len() > STDERR_TAIL_BYTES {
                    tail = tail.split_off(tail.len() - STDERR_TAIL_BYTES);
                }
                String::from_utf8_lossy(&tail).into_owned()
            })
        });

        let mut reader = BufReader::new(stdout);
        let mut lines = LineReader::default();
        let mut saw_error = false;
        let mut saw_finish = false;
        let mut cancelled = false;
        let mut sessions_seen: Vec<String> = Vec::new();

        let mut last_activity = tokio::time::Instant::now();
        let mut saw_line = false;
        let mut in_flight: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut was_paused = false;
        let mut stalled: Option<LlmError> = None;

        loop {
            let window = watch_window(&spec, saw_line, !in_flight.is_empty());
            let wake = window.map(|(w, _)| {
                if was_paused {
                    tokio::time::Instant::now() + pause_poll(w)
                } else {
                    last_activity + w
                }
            });
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    cancelled = true;
                    None
                }
                out = async {
                    match duplex.as_mut() {
                        Some(d) => d.outbox.recv().await,
                        None => std::future::pending::<Option<String>>().await,
                    }
                }, if outbox_open => {
                    match (out, pipe_open.as_mut()) {
                        (Some(line), Some(pipe)) => {
                            let mut bytes = line.into_bytes();
                            bytes.push(b'\n');
                            if pipe.write_all(&bytes).await.is_err() || pipe.flush().await.is_err() {
                                pipe_open = None;
                            }
                        }
                        (Some(_), None) => {}
                        (None, _) => outbox_open = false,
                    }
                    continue;
                }
                line = lines.next_line(&mut reader, MAX_LINE_BYTES) => {
                    let line = line.unwrap_or_default();
                    if line.is_some() {
                        last_activity = tokio::time::Instant::now();
                        saw_line = true;
                    }
                    line
                }
                _ = tokio::time::sleep_until(wake.unwrap_or_else(tokio::time::Instant::now)), if wake.is_some() => {
                    let paused = hooks.paused.as_ref().map(|p| p()).unwrap_or(false);
                    if paused || was_paused {
                        was_paused = paused;
                        last_activity = tokio::time::Instant::now();
                        continue;
                    }
                    let Some((w, kind)) = window else { continue };
                    if tokio::time::Instant::now() < last_activity + w {
                        continue;
                    }
                    stalled = Some(stall_error(kind, w));
                    None
                }
            };
            let Some((line, oversized)) = next else { break };
            if oversized {
                tracing::warn!("[cli] dropped a stdout line over {MAX_LINE_BYTES} bytes");
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            for signal in parser.parse_line(&line) {
                match signal {
                    CliSignal::Event(event) => {
                        match &event {
                            TurnEvent::Error { .. } => saw_error = true,
                            TurnEvent::Finished { .. } => saw_finish = true,
                            TurnEvent::ToolCallStart { id, .. } => {
                                in_flight.insert(id.clone());
                            }
                            TurnEvent::ToolResult { id, .. } => {
                                in_flight.remove(id);
                            }
                            _ => {}
                        }
                        if tx.send(event).await.is_err() {
                            cancelled = true;
                            break;
                        }
                    }
                    CliSignal::Rate(snapshot) => {
                        if let Some(sink) = &hooks.rate {
                            sink(snapshot);
                        }
                    }
                    CliSignal::Session(id) => {
                        tracing::debug!("[cli] session {id}");
                        if !sessions_seen.contains(&id) {
                            sessions_seen.push(id.clone());
                            if let Some(sink) = &hooks.session {
                                sink(id);
                            }
                        }
                    }
                    CliSignal::Ignored => {}
                }
            }
            if cancelled {
                break;
            }
            if saw_finish && pipe_open.is_some() {
                pipe_open = None;
                outbox_open = false;
            }
        }

        if let Some(error) = stalled {
            tracing::warn!("[cli] watchdog: {}: {}", error.code, error.message);
            kill_tree(&mut child, &mut tree).await;
            let _ = tx.send(TurnEvent::Error { error }).await;
            let _ = tx
                .send(TurnEvent::Finished {
                    reason: FinishReason::Other,
                })
                .await;
            return;
        }

        if cancelled {
            let interrupt = duplex
                .as_ref()
                .and_then(|d| d.interrupt.as_ref())
                .and_then(|f| f());
            if let (Some(line), Some(pipe)) = (interrupt, pipe_open.as_mut()) {
                let mut bytes = line.into_bytes();
                bytes.push(b'\n');
                if pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok() {
                    let _ = tokio::time::timeout(INTERRUPT_GRACE, async {
                        while let Ok(Some((line, _))) =
                            lines.next_line(&mut reader, MAX_LINE_BYTES).await
                        {
                            if line.contains("\"turn/completed\"") {
                                break;
                            }
                        }
                    })
                    .await;
                }
            }
            drop(pipe_open.take());
            kill_tree(&mut child, &mut tree).await;
            let _ = tx
                .send(TurnEvent::Error {
                    error: LlmError::new(ERR_LLM_CANCELLED, "turn cancelled"),
                })
                .await;
            let _ = tx
                .send(TurnEvent::Finished {
                    reason: FinishReason::Cancelled,
                })
                .await;
            return;
        }

        let status = child.wait().await;
        tree.release();
        let stderr_text = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };

        match status {
            Ok(status) if status.success() => {
                if !saw_finish && !saw_error {
                    let _ = tx
                        .send(TurnEvent::Finished {
                            reason: FinishReason::Stop,
                        })
                        .await;
                }
            }
            Ok(status) => {
                if !saw_error {
                    let _ = tx
                        .send(TurnEvent::Error {
                            error: exit_error(status.code(), &stderr_text),
                        })
                        .await;
                    let _ = tx
                        .send(TurnEvent::Finished {
                            reason: FinishReason::Other,
                        })
                        .await;
                } else if !saw_finish {
                    let _ = tx
                        .send(TurnEvent::Finished {
                            reason: FinishReason::Other,
                        })
                        .await;
                }
            }
            Err(e) => {
                let _ = tx
                    .send(TurnEvent::Error {
                        error: LlmError::new(ERR_CLI_SPAWN, format!("wait failed: {e}")),
                    })
                    .await;
            }
        }
    });

    Ok(rx.boxed())
}

pub fn exit_error(code: Option<i32>, stderr: &str) -> LlmError {
    let all: Vec<&str> = stderr.lines().collect();
    let tail = all[all.len().saturating_sub(4)..].join(" / ");
    let shown = if tail.trim().is_empty() {
        "no stderr".to_string()
    } else {
        tail.chars().take(400).collect()
    };
    let code_text = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".to_string());
    let classified = classify_error_text(stderr);
    let mut error = LlmError::new(
        classified.unwrap_or(ERR_CLI_EXIT),
        format!("the CLI exited with {code_text}: {shown}"),
    );
    error.retryable = classified.is_some();
    error
}
