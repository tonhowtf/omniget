//! One CLI turn = one child process. Spawn with an isolated environment, read
//! stdout line by line, turn each line into `TurnEvent`, kill on cancel.
//! Owned by f4-cli-runtime.
//!
//! Budget (plan §3 Fase 4): nothing here runs between turns. The process is
//! created inside [`spawn_turn`] and is dead — by exit or by signal — before
//! the stream ends. Switching accounts costs one `BTreeMap` of environment
//! variables, so it is free.

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

/// How long we wait for the child to die after a cancel before giving up on
/// the wait (the kill itself is a signal and is immediate). The gate wants the
/// stream closed in under 200 ms.
const KILL_GRACE_MS: u64 = 150;
/// Only the tail of stderr is kept: a CLI can print megabytes of progress.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Everything needed to run one turn, with no reference to an account: the
/// caller (claude.rs / codex.rs) has already resolved the binary and the env.
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Variables to set, typically just `CLAUDE_CONFIG_DIR`/`CODEX_HOME`.
    pub env: BTreeMap<String, String>,
    /// Variables to remove before launching (API keys and OAuth tokens).
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Written to the child's stdin and then closed. `None` closes it right
    /// away, which is what tells the CLI there is no interactive input.
    pub stdin: Option<String>,
    /// The watchdog (`ERR_CLI_STALLED`): no stdout line for this long kills
    /// the process tree. `None` = the turn may stay silent forever, which is
    /// how a stuck `claude -p` used to hold a job slot until the app closed.
    pub idle_timeout: Option<Duration>,
    /// The same window while a tool the CLI started has not answered
    /// (`ToolCallStart` without `ToolResult`): tests and builds are silent by
    /// nature. `None` falls back to `idle_timeout`.
    pub tool_idle_timeout: Option<Duration>,
    /// No line at all this long after the spawn = `ERR_CLI_NO_START`. `None`
    /// falls back to `idle_timeout`.
    pub start_timeout: Option<Duration>,
}

/// The three windows of the turn watchdog, as the settings give them. Kept
/// apart from the argv on purpose: [`WatchdogConfig::apply`] only touches the
/// timing fields of a [`SpawnSpec`], so a turn launches with exactly the same
/// command line with or without the watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogConfig {
    pub idle: Option<Duration>,
    pub tool_idle: Option<Duration>,
    pub start: Option<Duration>,
}

impl Default for WatchdogConfig {
    /// The shipped defaults (`LlmSettings`): the protection holds even before
    /// the app pushes the user's values.
    fn default() -> Self {
        Self {
            idle: Some(Duration::from_secs(600)),
            tool_idle: Some(Duration::from_secs(1800)),
            start: Some(Duration::from_secs(90)),
        }
    }
}

impl WatchdogConfig {
    /// No watchdog at all (tests, and the old behaviour for comparison).
    pub const OFF: Self = Self {
        idle: None,
        tool_idle: None,
        start: None,
    };

    pub fn apply(&self, spec: &mut SpawnSpec) {
        spec.idle_timeout = self.idle;
        spec.tool_idle_timeout = self.tool_idle;
        spec.start_timeout = self.start;
    }
}

/// Which window ran out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stall {
    /// Not a single line since the spawn.
    NoStart,
    /// Silent with no tool in flight.
    Idle,
    /// Silent while a tool of the CLI had not answered.
    ToolIdle,
}

/// The window that applies right now, and what running out of it means.
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

/// How often a paused clock looks again: often enough that the whole window
/// is still there after the question is answered (at most this much of it is
/// lost), rarely enough to cost nothing while a person takes an hour.
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

/// Turns one line of stdout into signals. Implemented by `claude.rs` and
/// `codex.rs`; a trait so `session.rs` never learns a message format.
pub trait LineParser: Send + Sync {
    fn parse_line(&self, line: &str) -> Vec<CliSignal>;

    /// A CLI that is talked to while the turn runs (`codex app-server`,
    /// JSON-RPC over stdio, 3.6) hands its outbox here, once. `None` (every
    /// one-shot CLI) keeps the old contract: stdin is written and closed.
    fn duplex(&self) -> Option<Duplex> {
        None
    }
}

/// The write half of a conversational turn. Everything else (process tree,
/// watchdog, cancel, secret scrubbing) stays the one path every CLI turn
/// takes, so a JSON-RPC CLI gets none of it by a parallel route.
pub struct Duplex {
    /// Lines to write to the child's stdin, without their newline: the
    /// parser's follow-up requests and the answers to the CLI's questions
    /// (an approval answered an hour later lands here).
    pub outbox: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// On cancel: the line that asks the CLI to stop the turn itself
    /// (`turn/interrupt`), so its history records an interrupted turn
    /// instead of a torn one. `None`, or a turn not yet started, goes
    /// straight to the kill.
    pub interrupt: Option<Arc<dyn Fn() -> Option<String> + Send + Sync>>,
}

/// How long a cancelled duplex turn may take to acknowledge its interrupt
/// before its tree is killed anyway. Cancel stays under 2 s (`81#P7`).
pub const INTERRUPT_GRACE: Duration = Duration::from_millis(800);

/// Where a quota reading goes. The runtime stores it per account so the router
/// can read `Capacity` without spawning anything.
pub type RateSink = Arc<dyn Fn(RateSnapshot) + Send + Sync>;

/// Longest stdout line kept (one JSON event). A longer line is dropped, with
/// a warning, instead of growing the buffer without bound.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Called with the child's pid and our launch id.
pub type SpawnedHook = Arc<dyn Fn(Option<u32>, &str) + Send + Sync>;

/// Everything besides the event stream a turn can report, all optional.
#[derive(Default, Clone)]
pub struct TurnHooks {
    pub rate: Option<RateSink>,
    /// The provider's session id (`CliSignal::Session`), once per id.
    pub session: Option<Arc<dyn Fn(String) + Send + Sync>>,
    /// Right after the spawn: pid and our launch id (also in the child's
    /// environment as `OMNIGET_LAUNCH_ID`).
    pub spawned: Option<SpawnedHook>,
    /// Runs once when the turn task ends, whatever the path (temp files,
    /// tokens).
    pub cleanup: Option<Arc<dyn Fn() + Send + Sync>>,
    /// True while a person is being asked something on behalf of this turn
    /// (`LlmManager::has_pending_ask`). Waiting for a human is not a stall:
    /// the idle clock does not run meanwhile, and starts over from zero once
    /// the answer lands.
    pub paused: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

/// Spawns the CLI and returns the turn stream.
///
/// The stream always ends: on a clean exit, on a non-zero exit
/// (`ERR_CLI_EXIT` carrying the code), on a broken pipe, or on cancel
/// (`ERR_LLM_CANCELLED`, after the child is killed).
pub fn spawn_turn(
    spec: SpawnSpec,
    parser: Arc<dyn LineParser>,
    cancel: CancellationToken,
    rate_sink: Option<RateSink>,
) -> Result<BoxStream<'static, TurnEvent>, LlmError> {
    spawn_turn_with(
        spec,
        parser,
        cancel,
        TurnHooks {
            rate: rate_sink,
            ..Default::default()
        },
    )
}

/// Reads stdout one line at a time, at most `max` bytes per line.
///
/// Cancel-safe, because the watchdog's timer races it in a `select!`: the
/// only await is `fill_buf` (which keeps what it read inside the
/// `BufReader`), and the partial line lives here, not in the future, so a
/// timer that fires mid-line loses nothing.
#[derive(Default)]
struct LineReader {
    buf: Vec<u8>,
    oversized: bool,
}

impl LineReader {
    /// `Ok(None)` at EOF; an oversized line comes back as
    /// `Some((String::new(), true))` after it is skipped.
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

/// Stops the child and every process of its tree, the same way on every
/// OS: [`ProcessTreeGuard::interrupt`](crate::core::process::ProcessTreeGuard::interrupt)
/// (Unix: `SIGTERM` to the group and to every descendant that left it), a
/// short grace for the child to exit, then `terminate` (Unix: freeze and
/// `SIGKILL` the whole tree, escapees included; Windows: close the
/// kill-on-close Job Object, which ends every descendant still in it).
/// Processes outside the tree are never touched.
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

/// Secret values (not names) that must never reach a child's environment,
/// whatever variable carries them: the bridge's extension bearer is
/// registered here when the bridge starts (2.1). A name-based `scrub` misses
/// a key the user exported under another name; this does not.
fn secret_values() -> &'static std::sync::Mutex<Vec<String>> {
    static VALUES: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    VALUES.get_or_init(Default::default)
}

/// See [`secret_values`]. Short values are ignored (they would match noise).
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

/// [`spawn_turn`] with every hook. The child runs in a process tree of its
/// own (Unix process group plus a walk of its descendants, Windows Job
/// Object), so a cancel or a stall stops its descendants too, even those
/// that left the group, and nothing else.
///
/// The watchdog lives in the read loop: a third `select!` arm sleeps until
/// the current window (`start_timeout` before the first line, then
/// `idle_timeout`, or `tool_idle_timeout` while a tool is in flight) runs out
/// since the last stdout line. Every line resets it, whatever the parser
/// makes of it. `hooks.paused` freezes it while a person is being asked.
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
    // Windows: created suspended, so not one instruction runs before the
    // process sits inside its kill-on-close Job Object (resumed right below).
    crate::core::process::suspend_on_spawn(&mut cmd);
    // The prompt travels on stdin, so the argv is safe to log.
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
            // Still suspended and in no job: nothing of it ran, end it here.
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

        // A duplex turn keeps stdin open for the parser's follow-ups and
        // closes it once the turn finished (the server then exits on EOF).
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
                // Dropping closes it: the CLI must not wait for more input.
                drop(pipe);
            }
        }
        let mut outbox_open = duplex.is_some();

        // stderr is drained in parallel so a full pipe can never deadlock the
        // child, and its tail explains a non-zero exit.
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

        // The watchdog's state: when the last line arrived, whether any did,
        // which tools of the CLI are still out, and whether the clock was
        // frozen for a question at the last look.
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
                                // The child closed its stdin: it is on its way out,
                                // and the exit status explains the turn.
                                pipe_open = None;
                            }
                        }
                        (Some(_), None) => {}
                        (None, _) => outbox_open = false,
                    }
                    continue;
                }
                // A read error ends the stream like EOF: the exit status below
                // is what explains the turn.
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
                        // Frozen while a person decides; on the first look
                        // after the answer the whole window starts over.
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
                            // The consumer dropped the stream: same as cancel.
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
            // The turn is over: closing stdin lets a server CLI exit on its
            // own, and the normal exit path below reports it.
            if saw_finish && pipe_open.is_some() {
                pipe_open = None;
                outbox_open = false;
            }
        }

        if let Some(error) = stalled {
            tracing::warn!("[cli] watchdog: {}: {}", error.code, error.message);
            // The tree dies before anyone hears about it: a job that reads
            // the error can hand its slot over at once.
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
            // A duplex CLI is asked to stop the turn first, so what it keeps
            // (Codex's thread history) records an interrupt, not a torn turn.
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
        // A normal end leaves whatever the agent started in the background
        // alone, on every OS: only a cancel or a stall kills the tree.
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

/// The error for a process that died with a non-zero status. The stderr tail
/// still gets classified: a CLI that prints "usage limit reached" and exits 1
/// must reroute, not fail the turn.
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
