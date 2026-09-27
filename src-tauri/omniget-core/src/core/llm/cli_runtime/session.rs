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
}

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
    for (key, value) in &spec.env {
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

        if let Some(mut pipe) = stdin.take() {
            if let Some(text) = &spec.stdin {
                let _ = pipe.write_all(text.as_bytes()).await;
            }
            // Dropping closes it: the CLI must not wait for more input.
            drop(pipe);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::time::Instant;

    use super::super::parse::{QuotaSource, RateStatus, RateWindow};

    /// A parser that turns every line into one text delta, so the shell
    /// scripts below can drive the machinery without any real CLI.
    struct EchoParser;

    impl LineParser for EchoParser {
        fn parse_line(&self, line: &str) -> Vec<CliSignal> {
            if let Some(rest) = line.strip_prefix("RATE ") {
                let used: f32 = rest.trim().parse().unwrap_or(0.0);
                return vec![CliSignal::Rate(RateSnapshot {
                    status: RateStatus::Allowed,
                    window_5h: Some(RateWindow {
                        used,
                        resets_at: None,
                    }),
                    window_7d: None,
                    source: QuotaSource::Real,
                })];
            }
            // `TOOL <id>` / `RESULT <id>` stand for Claude's `tool_use` and
            // `tool_result`, so the watchdog's tool window can be driven.
            if let Some(id) = line.strip_prefix("TOOL ") {
                return vec![CliSignal::Event(TurnEvent::ToolCallStart {
                    id: id.trim().into(),
                    name: "Bash".into(),
                })];
            }
            if let Some(id) = line.strip_prefix("RESULT ") {
                return vec![CliSignal::Event(TurnEvent::ToolResult {
                    id: id.trim().into(),
                    content: String::new(),
                    is_error: false,
                })];
            }
            vec![CliSignal::Event(TurnEvent::TextDelta {
                text: line.to_string(),
            })]
        }
    }

    /// Writes an executable shell script and returns its path. This is the
    /// "fake CLI" the gate asks for: no network, no real binary.
    #[cfg(unix)]
    fn fake_cli(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path =
            std::env::temp_dir().join(format!("omniget-fake-cli-{name}-{}.sh", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        f.write_all(body.as_bytes()).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn spec(program: PathBuf) -> SpawnSpec {
        SpawnSpec {
            program,
            ..SpawnSpec::default()
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_clean_run_streams_every_line_and_finishes() {
        let cli = fake_cli("ok", "echo one\necho two\nexit 0\n");
        let stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            CancellationToken::new(),
            None,
        )
        .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let texts: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["one", "two"]);
        assert!(matches!(
            events.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Stop
            })
        ));
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_non_zero_exit_is_err_cli_exit_with_the_code() {
        let cli = fake_cli("boom", "echo hi\necho 'bad thing' 1>&2\nexit 42\n");
        let stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            CancellationToken::new(),
            None,
        )
        .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let error = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("a non-zero exit must surface");
        assert_eq!(error.code, ERR_CLI_EXIT);
        assert!(error.message.contains("42"), "{}", error.message);
        assert!(error.message.contains("bad thing"), "{}", error.message);
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_rate_message_on_stderr_reroutes_instead_of_failing() {
        let cli = fake_cli(
            "rate",
            "echo 'Claude AI usage limit reached' 1>&2\nexit 1\n",
        );
        let stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            CancellationToken::new(),
            None,
        )
        .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let error = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("an error event");
        assert_eq!(error.code, super::super::ERR_CLI_RATE);
        assert!(error.retryable);
        assert!(super::super::super::router::is_reroutable(&error.code));
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_the_child_in_under_200ms() {
        // A CLI that would print for a minute; we cancel after the first line.
        let cli = fake_cli("slow", "echo first\nsleep 60\necho never\n");
        let cancel = CancellationToken::new();
        let mut stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            cancel.clone(),
            None,
        )
        .unwrap();
        let first = stream.next().await.expect("the first line");
        assert!(matches!(first, TurnEvent::TextDelta { .. }));

        let started = Instant::now();
        cancel.cancel();
        let rest: Vec<TurnEvent> = stream.collect().await;
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() <= 200,
            "cancel took {} ms",
            elapsed.as_millis()
        );
        let error = rest
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("a cancel error");
        assert_eq!(error.code, ERR_LLM_CANCELLED);
        assert!(matches!(
            rest.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Cancelled
            })
        ));
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_child_sees_the_isolated_env_and_not_the_scrubbed_one() {
        let cli = fake_cli(
            "env",
            "echo \"dir=$CLAUDE_CONFIG_DIR\"\necho \"key=${ANTHROPIC_API_KEY:-gone}\"\n",
        );
        std::env::set_var("ANTHROPIC_API_KEY", "sk-should-not-reach-the-cli");
        let mut s = spec(cli.clone());
        s.env
            .insert("CLAUDE_CONFIG_DIR".into(), "/tmp/profile-a".into());
        s.scrub = vec!["ANTHROPIC_API_KEY".into()];
        let stream = spawn_turn(s, Arc::new(EchoParser), CancellationToken::new(), None).unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(text.contains("dir=/tmp/profile-a"), "{text}");
        assert!(text.contains("key=gone"), "{text}");
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_reaches_the_child_and_is_closed() {
        let cli = fake_cli("stdin", "cat\n");
        let mut s = spec(cli.clone());
        s.stdin = Some("hello from omniget\n".into());
        let stream = spawn_turn(s, Arc::new(EchoParser), CancellationToken::new(), None).unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        assert!(events.iter().any(|e| matches!(
            e,
            TurnEvent::TextDelta { text } if text == "hello from omniget"
        )));
        let _ = std::fs::remove_file(cli);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rate_signals_reach_the_sink_and_never_the_stream() {
        let cli = fake_cli("sink", "echo 'RATE 0.75'\necho done\n");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let sink: RateSink = Arc::new(move |snap: RateSnapshot| {
            sink_seen.lock().unwrap().push(snap);
        });
        let stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            CancellationToken::new(),
            Some(sink),
        )
        .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let snaps = seen.lock().unwrap();
        assert_eq!(snaps.len(), 1);
        assert!((snaps[0].window_5h.unwrap().used - 0.75).abs() < 1e-6);
        assert!(!events.iter().any(|e| matches!(
            e,
            TurnEvent::TextDelta { text } if text.starts_with("RATE")
        )));
        let _ = std::fs::remove_file(cli);
    }

    #[tokio::test]
    async fn a_missing_binary_is_err_cli_spawn() {
        let err = spawn_turn(
            SpawnSpec {
                program: PathBuf::from("/nonexistent/omniget-no-such-cli"),
                ..SpawnSpec::default()
            },
            Arc::new(EchoParser),
            CancellationToken::new(),
            None,
        )
        .err()
        .expect("spawning a missing binary must fail");
        assert_eq!(err.code, ERR_CLI_SPAWN);
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A12: cancelling a run whose CLI has a grandchild stops both; a process
    /// outside the group (spawned here, not by the turn) survives; no
    /// completion is reported.
    #[cfg(unix)]
    #[tokio::test]
    async fn a12_cancel_kills_our_tree_and_spares_a_foreign_process() {
        let dir = std::env::temp_dir().join(format!("omniget-a12-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        // The CLI starts a grandchild (sleep) in the background, prints, waits.
        let cli = fake_cli(
            "a12",
            &format!(
                "sleep 120 &\necho $! > {}\necho started\nwait\n",
                pidfile.display()
            ),
        );
        // A process that is not ours: its own group, spawned by the test.
        let mut foreign = std::process::Command::new("sleep")
            .arg("120")
            .spawn()
            .unwrap();
        let foreign_pid = foreign.id() as i32;

        let spawned: Arc<std::sync::Mutex<Option<u32>>> = Arc::default();
        let s2 = spawned.clone();
        let cleaned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c2 = cleaned.clone();
        let cancel = CancellationToken::new();
        let mut stream = spawn_turn_with(
            spec(cli.clone()),
            Arc::new(EchoParser),
            cancel.clone(),
            TurnHooks {
                spawned: Some(Arc::new(move |pid, launch| {
                    assert!(!launch.is_empty());
                    *s2.lock().unwrap() = pid;
                })),
                cleanup: Some(Arc::new(move || {
                    c2.store(true, std::sync::atomic::Ordering::SeqCst);
                })),
                ..Default::default()
            },
        )
        .unwrap();
        let first = stream.next().await.expect("started");
        assert!(matches!(first, TurnEvent::TextDelta { .. }));
        let child_pid = spawned.lock().unwrap().expect("pid") as i32;
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(child_pid) && alive(grandchild) && alive(foreign_pid));

        cancel.cancel();
        let rest: Vec<TurnEvent> = stream.collect().await;
        assert!(matches!(
            rest.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Cancelled
            })
        ));
        assert!(!rest.iter().any(|e| matches!(
            e,
            TurnEvent::Finished {
                reason: FinishReason::Stop
            }
        )));
        // Give the kernel a moment to reap.
        for _ in 0..50 {
            if !alive(grandchild) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!alive(grandchild), "the grandchild must die with the run");
        assert!(alive(foreign_pid), "a foreign process must survive");
        assert!(cleaned.load(std::sync::atomic::Ordering::SeqCst));
        let _ = foreign.kill();
        let _ = foreign.wait();
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_oversized_line_is_dropped_not_buffered_forever() {
        let big = MAX_LINE_BYTES + 10;
        let cli = fake_cli(
            "bigline",
            &format!("head -c {big} /dev/zero | tr '\\0' 'a'\necho\necho after\n"),
        );
        let stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            CancellationToken::new(),
            None,
        )
        .unwrap();
        let events: Vec<TurnEvent> = stream.collect().await;
        let texts: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["after"]);
        let _ = std::fs::remove_file(cli);
    }

    // ── watchdog ────────────────────────────────────────────────────────

    #[cfg(unix)]
    fn watched(
        program: PathBuf,
        idle_ms: u64,
        tool_ms: Option<u64>,
        start_ms: Option<u64>,
    ) -> SpawnSpec {
        SpawnSpec {
            program,
            idle_timeout: Some(Duration::from_millis(idle_ms)),
            tool_idle_timeout: tool_ms.map(Duration::from_millis),
            start_timeout: start_ms.map(Duration::from_millis),
            ..SpawnSpec::default()
        }
    }

    /// Runs a turn to its end and returns the events with the time each
    /// arrived, measured from the spawn.
    #[cfg(unix)]
    async fn run_timed(spec: SpawnSpec, hooks: TurnHooks) -> Vec<(Duration, TurnEvent)> {
        let started = Instant::now();
        let mut stream =
            spawn_turn_with(spec, Arc::new(EchoParser), CancellationToken::new(), hooks).unwrap();
        let mut out = Vec::new();
        while let Some(event) = stream.next().await {
            out.push((started.elapsed(), event));
        }
        out
    }

    /// Upper bound for "the kill came": these tests spawn real processes, and
    /// on a loaded machine (a parallel `cargo test`, CI) a `sleep` or an
    /// `echo` can run seconds late. What they pin is the lower bound (never
    /// killed inside the window); how close to the window the kill lands is
    /// BN1's job (`scripts/llm/watchdog_eval.py`), which runs alone.
    #[cfg(unix)]
    const LATE: Duration = Duration::from_secs(10);

    /// The error of the turn, timed from the last event before it (the line
    /// that started the silence): a loaded machine can take a while to run
    /// the fake's first `echo`, and that is not the watchdog's window.
    #[cfg(unix)]
    fn error_of(events: &[(Duration, TurnEvent)]) -> Option<(Duration, LlmError)> {
        let i = events
            .iter()
            .position(|(_, e)| matches!(e, TurnEvent::Error { .. }))?;
        let TurnEvent::Error { error } = &events[i].1 else {
            return None;
        };
        let since = if i == 0 {
            Duration::ZERO
        } else {
            events[i - 1].0
        };
        Some((events[i].0 - since, error.clone()))
    }

    /// D3: a `claude -p` that prints its init and then hangs used to hold a
    /// job slot until the app closed. Now the idle window kills it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_silent_cli_is_killed_after_the_idle_window() {
        let cli = fake_cli("stall", "echo first\nsleep 30\necho never\n");
        let events = run_timed(
            watched(cli.clone(), 1000, None, Some(5000)),
            TurnHooks::default(),
        )
        .await;
        let (at, error) = error_of(&events).expect("a stall error");
        assert_eq!(error.code, ERR_CLI_STALLED, "{}", error.message);
        assert!(!error.retryable, "a stall is never rerouted");
        assert!(
            at >= Duration::from_millis(1000) && at <= Duration::from_millis(1000) + LATE,
            "killed at {at:?}, the window is 1 s"
        );
        assert!(matches!(
            events.last(),
            Some((
                _,
                TurnEvent::Finished {
                    reason: FinishReason::Other
                }
            ))
        ));
        assert!(!events
            .iter()
            .any(|(_, e)| matches!(e, TurnEvent::TextDelta { text } if text == "never")));
        let _ = std::fs::remove_file(cli);
    }

    /// Nothing at all after the spawn: the prompt never took.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cli_that_never_prints_fails_with_no_start() {
        let cli = fake_cli("nostart", "sleep 30\n");
        let events = run_timed(
            watched(cli.clone(), 10_000, None, Some(1000)),
            TurnHooks::default(),
        )
        .await;
        let (at, error) = error_of(&events).expect("a no-start error");
        assert_eq!(error.code, ERR_CLI_NO_START, "{}", error.message);
        assert!(at <= Duration::from_millis(1000) + LATE, "took {at:?}");
        let _ = std::fs::remove_file(cli);
    }

    /// A question to a person freezes the clock (82#P4): 3 windows of
    /// silence while paused do not kill, and after the answer the whole
    /// window is available again before the stall.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_paused_clock_does_not_kill_and_restarts_on_resume() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cli = fake_cli("paused", "echo first\nsleep 30\n");
        let asking = Arc::new(AtomicBool::new(true));
        let probe = asking.clone();
        let hooks = TurnHooks {
            paused: Some(Arc::new(move || probe.load(Ordering::SeqCst))),
            ..Default::default()
        };
        let mut stream = spawn_turn_with(
            watched(cli.clone(), 1000, None, Some(5000)),
            Arc::new(EchoParser),
            CancellationToken::new(),
            hooks,
        )
        .unwrap();
        let first = stream.next().await.expect("first line");
        assert!(matches!(first, TurnEvent::TextDelta { .. }));
        // The question stays open for 3 windows after the last line.
        let since_line = Instant::now();
        let flag = asking.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(3000)).await;
            flag.store(false, Ordering::SeqCst);
        });
        let rest: Vec<TurnEvent> = stream.collect().await;
        let at = since_line.elapsed();
        let error = rest
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("stalls once the answer is in");
        assert_eq!(error.code, ERR_CLI_STALLED);
        // Answered at 3 s; the clock restarts at the first look after it
        // (poll = 250 ms), so the kill lands in [3.75 s, ~4.5 s]: never in
        // the paused stretch, and the whole window is there after it.
        assert!(
            at >= Duration::from_millis(3700) && at <= Duration::from_millis(4000) + LATE,
            "killed {at:?} after the last line"
        );
        let _ = std::fs::remove_file(cli);
    }

    /// `tool_use` without `tool_result` gets the longer window; once the
    /// result is in, the short one applies again.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_tool_in_flight_uses_the_longer_window() {
        let cli = fake_cli(
            "toolwin",
            "echo 'TOOL t1'\nsleep 2\necho 'RESULT t1'\nsleep 30\n",
        );
        let events = run_timed(
            watched(cli.clone(), 1000, Some(5000), Some(5000)),
            TurnHooks::default(),
        )
        .await;
        assert!(
            events
                .iter()
                .any(|(_, e)| matches!(e, TurnEvent::ToolResult { .. })),
            "2 s of silence with a tool in flight must not kill (window 5 s)"
        );
        let (at, error) = error_of(&events).expect("stalls after the result");
        assert_eq!(error.code, ERR_CLI_STALLED);
        assert!(
            !error.message.contains("tool"),
            "back to the plain window: {}",
            error.message
        );
        assert!(
            at >= Duration::from_millis(1000) && at <= Duration::from_millis(1000) + LATE,
            "killed {at:?} after the result"
        );
        let _ = std::fs::remove_file(cli);
    }

    /// The tool window itself also runs out, and says it was a tool.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_tool_that_never_answers_is_a_tool_stall() {
        let cli = fake_cli("toolstall", "echo 'TOOL t1'\nsleep 30\n");
        let events = run_timed(
            watched(cli.clone(), 500, Some(1500), Some(5000)),
            TurnHooks::default(),
        )
        .await;
        let (at, error) = error_of(&events).expect("a stall");
        assert_eq!(error.code, ERR_CLI_STALLED);
        assert!(error.message.contains("tool"), "{}", error.message);
        assert!(
            at >= Duration::from_millis(1500) && at <= Duration::from_millis(1500) + LATE,
            "killed {at:?} after the tool_use"
        );
        let _ = std::fs::remove_file(cli);
    }

    /// Any line resets the clock: a CLI that talks every 0.5 s for 6 s
    /// under a 3 s window finishes normally (the window is 6× the gap so a
    /// loaded machine that runs a `sleep 0.5` late cannot fake a stall).
    #[cfg(unix)]
    #[tokio::test]
    async fn every_line_resets_the_clock() {
        let cli = fake_cli(
            "ticks",
            "for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo tick; sleep 0.5; done\nexit 0\n",
        );
        let events = run_timed(
            watched(cli.clone(), 3000, None, Some(5000)),
            TurnHooks::default(),
        )
        .await;
        assert!(error_of(&events).is_none(), "{events:?}");
        assert!(matches!(
            events.last(),
            Some((
                _,
                TurnEvent::Finished {
                    reason: FinishReason::Stop
                }
            ))
        ));
        let _ = std::fs::remove_file(cli);
    }

    /// The stall kills the whole tree, like a cancel (A12).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stall_kills_the_grandchild_too() {
        let dir = std::env::temp_dir().join(format!("omniget-stall-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let cli = fake_cli(
            "stalltree",
            &format!(
                "sleep 120 &\necho $! > {}\necho started\nwait\n",
                pidfile.display()
            ),
        );
        let events = run_timed(
            watched(cli.clone(), 800, None, Some(5000)),
            TurnHooks::default(),
        )
        .await;
        assert_eq!(error_of(&events).expect("stall").1.code, ERR_CLI_STALLED);
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let killed_at = Instant::now();
        while alive(grandchild) && killed_at.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !alive(grandchild),
            "the grandchild outlived the stall by 1 s"
        );
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── descendants that leave the process group ───────────────────────
    //
    // `setsid()` takes a process out of the CLI's group, so `killpg` never
    // reaches it: an MCP server, a `nohup`, a daemonizing tool. macOS ships
    // no `setsid` binary, so the escape is a small Python script (the BN1
    // fake needs `python3` anyway).

    /// Writes the escapee: it calls `setsid()`, with `--child` starts a
    /// `sleep` of its own (which stays in the new session), writes
    /// "<own pid> [<sleep pid>]" to `argv[1]` atomically and sleeps.
    #[cfg(unix)]
    fn escapee_script(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("escape.py");
        std::fs::write(
            &path,
            "import os, subprocess, sys, time\n\
             os.setsid()\n\
             kids = [subprocess.Popen(['sleep', '120']).pid] if '--child' in sys.argv else []\n\
             tmp = sys.argv[1] + '.tmp'\n\
             with open(tmp, 'w') as f:\n    f.write(' '.join(str(p) for p in [os.getpid()] + kids))\n\
             os.rename(tmp, sys.argv[1])\n\
             time.sleep(120)\n",
        )
        .unwrap();
        path
    }

    /// Shell lines that wait until `pidfile` exists, so the pids are known
    /// before the CLI prints its first line.
    #[cfg(unix)]
    fn await_pidfile(pidfile: &std::path::Path) -> String {
        format!(
            "while [ ! -s {0} ]; do sleep 0.02; done\n",
            pidfile.display()
        )
    }

    #[cfg(unix)]
    fn read_pids(pidfile: &std::path::Path) -> Vec<i32> {
        std::fs::read_to_string(pidfile)
            .unwrap()
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect()
    }

    /// Waits up to 1 s for every pid to be gone and returns the survivors.
    #[cfg(unix)]
    async fn survivors_after_1s(pids: &[i32]) -> Vec<i32> {
        let since = Instant::now();
        while pids.iter().any(|&p| alive(p)) && since.elapsed() < Duration::from_secs(1) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let left: Vec<i32> = pids.iter().copied().filter(|&p| alive(p)).collect();
        for &p in &left {
            // Never leave a two-minute sleeper behind a failed assertion.
            unsafe {
                libc::kill(p, libc::SIGKILL);
            }
        }
        left
    }

    /// A tree whose grandchild left the group (`setsid`): `pidfile` gets
    /// its pid (and, with `--child`, its own child's) before "started".
    #[cfg(unix)]
    fn escaping_cli(
        name: &str,
        dir: &std::path::Path,
        pidfile: &std::path::Path,
        deep: bool,
    ) -> PathBuf {
        let escape = escapee_script(dir);
        let launch = if deep {
            // child (group leader) → sh (same group) → python (new session)
            // → sleep (python's session): three levels, two of them outside.
            format!(
                "sh -c 'python3 {} {} --child & wait' &\n",
                escape.display(),
                pidfile.display()
            )
        } else {
            format!("python3 {} {} &\n", escape.display(), pidfile.display())
        };
        fake_cli(
            name,
            &format!("{launch}{}echo started\nwait\n", await_pidfile(pidfile)),
        )
    }

    /// The watchdog's kill reaches a grandchild that called `setsid()`.
    /// With `killpg` alone it survived the stall.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stall_kills_a_grandchild_that_left_the_group() {
        let dir =
            std::env::temp_dir().join(format!("omniget-escape-stall-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("escapee.pid");
        let cli = escaping_cli("escstall", &dir, &pidfile, false);
        let events = run_timed(
            watched(cli.clone(), 800, None, Some(5000)),
            TurnHooks::default(),
        )
        .await;
        assert_eq!(error_of(&events).expect("stall").1.code, ERR_CLI_STALLED);
        let pids = read_pids(&pidfile);
        assert_eq!(
            survivors_after_1s(&pids).await,
            Vec::<i32>::new(),
            "the escapee outlived the stall"
        );
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Cancelling a turn reaches the same escapee.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_a_grandchild_that_left_the_group() {
        let dir =
            std::env::temp_dir().join(format!("omniget-escape-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("escapee.pid");
        let cli = escaping_cli("esccancel", &dir, &pidfile, false);
        let cancel = CancellationToken::new();
        let mut stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            cancel.clone(),
            None,
        )
        .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(TurnEvent::TextDelta { .. })
        ));
        let pids = read_pids(&pidfile);
        assert!(pids.iter().all(|&p| alive(p)));
        cancel.cancel();
        let rest: Vec<TurnEvent> = stream.collect().await;
        assert!(matches!(
            rest.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Cancelled
            })
        ));
        assert_eq!(
            survivors_after_1s(&pids).await,
            Vec::<i32>::new(),
            "the escapee outlived the cancel"
        );
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// child → grandchild → great-grandchild in a new session → its own
    /// child: every level dies with the stall.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_deep_chain_with_a_new_session_dies_whole() {
        let dir =
            std::env::temp_dir().join(format!("omniget-escape-deep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("escapee.pid");
        let cli = escaping_cli("escdeep", &dir, &pidfile, true);
        let events = run_timed(
            watched(cli.clone(), 800, None, Some(5000)),
            TurnHooks::default(),
        )
        .await;
        assert_eq!(error_of(&events).expect("stall").1.code, ERR_CLI_STALLED);
        let pids = read_pids(&pidfile);
        assert_eq!(pids.len(), 2, "python and its sleep");
        assert_eq!(
            survivors_after_1s(&pids).await,
            Vec::<i32>::new(),
            "part of the chain survived"
        );
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The walk follows parent links only: a process in a session of its
    /// own that the test (not the turn) started is never touched, even
    /// while the turn's own escapee is being killed.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_tree_kill_spares_a_process_that_is_not_a_descendant() {
        let dir =
            std::env::temp_dir().join(format!("omniget-escape-foreign-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let foreign_file = dir.join("foreign.pid");
        let mut foreign = std::process::Command::new("python3")
            .arg(escapee_script(&dir))
            .arg(&foreign_file)
            .spawn()
            .unwrap();
        let pidfile = dir.join("escapee.pid");
        let cli = escaping_cli("escforeign", &dir, &pidfile, false);
        let cancel = CancellationToken::new();
        let mut stream = spawn_turn(
            spec(cli.clone()),
            Arc::new(EchoParser),
            cancel.clone(),
            None,
        )
        .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(TurnEvent::TextDelta { .. })
        ));
        let since = Instant::now();
        while !foreign_file.exists() && since.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let stranger = read_pids(&foreign_file)[0];
        cancel.cancel();
        let _: Vec<TurnEvent> = stream.collect().await;
        assert_eq!(
            survivors_after_1s(&read_pids(&pidfile)).await,
            Vec::<i32>::new()
        );
        assert!(alive(stranger), "a process outside the tree was killed");
        let _ = foreign.kill();
        let _ = foreign.wait();
        let _ = std::fs::remove_file(cli);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The watchdog is timing only: applying it never touches the argv, the
    /// environment or the prompt, so the missions baseline (B0) measured
    /// with the same argv still holds.
    #[test]
    fn the_watchdog_leaves_the_argv_alone() {
        let mut spec = SpawnSpec {
            program: PathBuf::from("/usr/bin/claude"),
            args: vec!["-p".into(), "--output-format".into(), "stream-json".into()],
            env: BTreeMap::from([("CLAUDE_CONFIG_DIR".to_string(), "/x".to_string())]),
            scrub: vec!["ANTHROPIC_API_KEY".into()],
            cwd: Some(PathBuf::from("/w")),
            stdin: Some("User: oi".into()),
            ..SpawnSpec::default()
        };
        let before = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            spec.program, spec.args, spec.env, spec.scrub, spec.cwd, spec.stdin
        );
        WatchdogConfig::default().apply(&mut spec);
        let after = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            spec.program, spec.args, spec.env, spec.scrub, spec.cwd, spec.stdin
        );
        assert_eq!(before, after);
        assert_eq!(spec.idle_timeout, Some(Duration::from_secs(600)));
        assert_eq!(spec.tool_idle_timeout, Some(Duration::from_secs(1800)));
        assert_eq!(spec.start_timeout, Some(Duration::from_secs(90)));
        WatchdogConfig::OFF.apply(&mut spec);
        assert_eq!(spec.idle_timeout, None);
    }

    #[test]
    fn the_window_follows_the_turn() {
        let spec = SpawnSpec {
            idle_timeout: Some(Duration::from_secs(10)),
            tool_idle_timeout: Some(Duration::from_secs(30)),
            start_timeout: Some(Duration::from_secs(5)),
            ..SpawnSpec::default()
        };
        assert_eq!(
            watch_window(&spec, false, false),
            Some((Duration::from_secs(5), Stall::NoStart))
        );
        assert_eq!(
            watch_window(&spec, true, false),
            Some((Duration::from_secs(10), Stall::Idle))
        );
        assert_eq!(
            watch_window(&spec, true, true),
            Some((Duration::from_secs(30), Stall::ToolIdle))
        );
        let only_idle = SpawnSpec {
            idle_timeout: Some(Duration::from_secs(10)),
            ..SpawnSpec::default()
        };
        assert_eq!(
            watch_window(&only_idle, false, false),
            Some((Duration::from_secs(10), Stall::NoStart))
        );
        assert_eq!(
            watch_window(&only_idle, true, true),
            Some((Duration::from_secs(10), Stall::ToolIdle))
        );
        assert_eq!(watch_window(&SpawnSpec::default(), true, false), None);
        assert_eq!(pause_poll(Duration::from_secs(600)), Duration::from_secs(1));
        assert_eq!(
            pause_poll(Duration::from_millis(1000)),
            Duration::from_millis(250)
        );
    }

    // ── BN1 (scripts/llm/watchdog_eval.py) ─────────────────────────────
    //
    // End to end at the turn level: `scripts/llm/fake_claude.py` as the CLI,
    // the real `ClaudeParser`, real pipes, the real kill of the tree. The
    // windows are scaled (idle 600 s → 2 s, tool 1800 s → 12 s, start 90 s →
    // 2 s); the goals are the roadmap's, in the same proportion.

    const BN1_IDLE_MS: u64 = 2_000;
    const BN1_TOOL_MS: u64 = 12_000;
    const BN1_START_MS: u64 = 2_000;

    /// One BN1 measurement to `$OMNIGET_BENCH_OUT`; nothing in a plain run.
    fn bench(metric: &str, value: serde_json::Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{}",
                    serde_json::json!({ "bench": "bn1", "metric": metric, "value": value, "level": "e2e-turn" })
                );
            }
        }
    }

    fn fake_claude() -> Option<PathBuf> {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/llm/fake_claude.py");
        let python = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .ok()?;
        (python.status.success() && script.is_file()).then_some(script)
    }

    /// A turn of the fake in `mode`, silent for `hold_s` where it goes quiet.
    fn bn1_spec(script: &std::path::Path, mode: &str, hold_s: f64) -> SpawnSpec {
        SpawnSpec {
            program: PathBuf::from("python3"),
            // The real CLI flags would follow; the fake ignores them.
            args: vec![
                script.display().to_string(),
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
            env: BTreeMap::from([
                ("FAKE_CLAUDE_MODE".to_string(), mode.to_string()),
                ("FAKE_CLAUDE_HOLD_S".to_string(), hold_s.to_string()),
            ]),
            stdin: Some("User: run the tests".into()),
            idle_timeout: Some(Duration::from_millis(BN1_IDLE_MS)),
            tool_idle_timeout: Some(Duration::from_millis(BN1_TOOL_MS)),
            start_timeout: Some(Duration::from_millis(BN1_START_MS)),
            ..SpawnSpec::default()
        }
    }

    /// The turn's events with their arrival time since the spawn.
    async fn bn1_run(spec: SpawnSpec, hooks: TurnHooks) -> Vec<(Duration, TurnEvent)> {
        let started = Instant::now();
        let mut stream = spawn_turn_with(
            spec,
            Arc::new(super::super::claude::ClaudeParser::new(true)),
            CancellationToken::new(),
            hooks,
        )
        .unwrap();
        let mut out = Vec::new();
        while let Some(event) = stream.next().await {
            out.push((started.elapsed(), event));
        }
        out
    }

    fn bn1_error(events: &[(Duration, TurnEvent)]) -> Option<(Duration, LlmError)> {
        let i = events
            .iter()
            .position(|(_, e)| matches!(e, TurnEvent::Error { .. }))?;
        let TurnEvent::Error { error } = &events[i].1 else {
            return None;
        };
        let since = if i == 0 {
            Duration::ZERO
        } else {
            events[i - 1].0
        };
        Some((events[i].0 - since, error.clone()))
    }

    fn bn1_finished_ok(events: &[(Duration, TurnEvent)]) -> bool {
        bn1_error(events).is_none()
            && events.iter().any(|(_, e)| {
                matches!(
                    e,
                    TurnEvent::Finished {
                        reason: FinishReason::Stop
                    }
                )
            })
    }

    /// Silent after `init`: killed within idle + 2 s (herdr's goal, 79#P1).
    #[tokio::test]
    async fn bn1_stall_after_init() {
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let events = bn1_run(
            bn1_spec(&fake, "stall-after-init", 3600.0),
            TurnHooks::default(),
        )
        .await;
        let (after_last, error) = bn1_error(&events).expect("the stall must end the turn");
        assert_eq!(error.code, ERR_CLI_STALLED);
        let limit = Duration::from_millis(BN1_IDLE_MS + 2_000);
        bench(
            "stall_after_init_kill_s",
            serde_json::json!(after_last.as_secs_f64()),
        );
        bench(
            "stall_after_init_idle_s",
            serde_json::json!(BN1_IDLE_MS as f64 / 1000.0),
        );
        bench("stall_after_init_code", serde_json::json!(error.code));
        // The stream ends right after the error: the job's runner returns
        // and its slot guard drops.
        let closed = events.last().unwrap().0
            - events
                .iter()
                .rev()
                .find(|(_, e)| matches!(e, TurnEvent::Error { .. }))
                .unwrap()
                .0;
        bench(
            "stream_closed_after_error_ms",
            serde_json::json!(closed.as_millis() as u64),
        );
        assert!(
            after_last <= limit,
            "killed {after_last:?} after the last line"
        );
        assert!(matches!(
            events.last(),
            Some((
                _,
                TurnEvent::Finished {
                    reason: FinishReason::Other
                }
            ))
        ));
    }

    /// Nothing printed at all: `ERR_CLI_NO_START` within start + 2 s.
    #[tokio::test]
    async fn bn1_silent_start() {
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let events = bn1_run(
            bn1_spec(&fake, "silent-start", 3600.0),
            TurnHooks::default(),
        )
        .await;
        let (at, error) = bn1_error(&events).expect("no-start must end the turn");
        bench("silent_start_code", serde_json::json!(error.code));
        bench("silent_start_kill_s", serde_json::json!(at.as_secs_f64()));
        assert_eq!(error.code, ERR_CLI_NO_START);
        assert!(at <= Duration::from_millis(BN1_START_MS + 2_000), "{at:?}");
    }

    /// A tool silent for 2x the idle window (≙ 20 min under 10 min) inside
    /// the tool window (≙ 30 min) is not killed: no false positive.
    #[tokio::test]
    async fn bn1_tool_long() {
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let hold = 2.0 * BN1_IDLE_MS as f64 / 1000.0;
        let events = bn1_run(bn1_spec(&fake, "tool-long", hold), TurnHooks::default()).await;
        let ok = bn1_finished_ok(&events);
        bench("tool_long_survived", serde_json::json!(ok));
        bench("tool_long_silence_s", serde_json::json!(hold));
        assert!(ok, "{:?}", bn1_error(&events));
        assert!(events
            .iter()
            .any(|(_, e)| matches!(e, TurnEvent::ToolResult { .. })));
    }

    /// A question to a person pending for 2.5x the idle window (≙ 15 min+
    /// under 10 min, 79#P1): the real broker's `has_pending` freezes the
    /// clock, so the turn is not killed and ends normally after the answer.
    #[tokio::test]
    async fn bn1_ask_pending() {
        use crate::core::llm::broker::{ToolBroker, ToolExecutor};
        use crate::core::omni::bus::Bus;
        struct Nothing;
        #[async_trait::async_trait]
        impl ToolExecutor for Nothing {
            async fn execute(&self, _: &str, _: serde_json::Value) -> Result<String, LlmError> {
                Ok(String::new())
            }
        }
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let broker = Arc::new(
            ToolBroker::new(vec![], Arc::new(Nothing), Arc::new(Bus::new()))
                .with_ask_timeout(Duration::from_secs(600)),
        );
        let pending_for = Duration::from_millis(BN1_IDLE_MS * 5 / 2);
        // The question, as Claude's permission prompt tool raises it.
        let b = broker.clone();
        tokio::spawn(async move {
            b.ask_user(
                "claude-code",
                "req-bn1",
                "toolu_fake_1",
                "cli:Bash",
                "sleep 300",
            )
            .await
        });
        let b = broker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(pending_for).await;
            while !b.answer("toolu_fake_1", true) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        while !broker.has_pending("req-bn1") {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let probe = broker.clone();
        let hooks = TurnHooks {
            paused: Some(Arc::new(move || probe.has_pending("req-bn1"))),
            ..Default::default()
        };
        // The fake stays silent a little longer than the question is open,
        // like a CLI that runs the tool once it is allowed.
        let hold = pending_for.as_secs_f64() + 0.5;
        let mut spec = bn1_spec(&fake, "ask-pending", hold);
        // Only the plain window: the pause alone must carry the turn.
        spec.tool_idle_timeout = None;
        let events = bn1_run(spec, hooks).await;
        let ok = bn1_finished_ok(&events);
        bench("ask_pending_survived", serde_json::json!(ok));
        bench(
            "ask_pending_s",
            serde_json::json!(pending_for.as_secs_f64()),
        );
        assert!(ok, "{:?}", bn1_error(&events));
    }

    /// The tree dies with the stall: the grandchild (a `sleep` the fake
    /// started, like a git or an MCP server) is gone within 1 s (Foco 1).
    #[cfg(unix)]
    #[tokio::test]
    async fn bn1_grandchild() {
        bn1_grandchild_in("grandchild", "grandchild").await;
    }

    /// The same for a grandchild that called `setsid()` (an MCP server, a
    /// `nohup`): outside the CLI's group, found by the walk of the tree.
    #[cfg(unix)]
    #[tokio::test]
    async fn bn1_grandchild_setsid() {
        bn1_grandchild_in("grandchild-setsid", "grandchild_setsid").await;
    }

    #[cfg(unix)]
    async fn bn1_grandchild_in(mode: &str, metric: &str) {
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let dir = std::env::temp_dir().join(format!("omniget-bn1-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let mut spec = bn1_spec(&fake, mode, 3600.0);
        spec.env
            .insert("FAKE_CLAUDE_PIDFILE".into(), pidfile.display().to_string());
        let events = bn1_run(spec, TurnHooks::default()).await;
        let killed = Instant::now();
        assert_eq!(bn1_error(&events).expect("stall").1.code, ERR_CLI_STALLED);
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        while alive(grandchild) && killed.elapsed() < Duration::from_secs(2) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let gone_after = killed.elapsed();
        let survivors = u32::from(alive(grandchild));
        if survivors > 0 {
            // Never leave an hour-long sleeper behind a failed benchmark.
            unsafe {
                libc::kill(grandchild, libc::SIGKILL);
            }
        }
        bench(&format!("{metric}_survivors"), serde_json::json!(survivors));
        bench(
            &format!("{metric}_gone_after_ms"),
            serde_json::json!(gone_after.as_millis() as u64),
        );
        assert_eq!(survivors, 0, "the grandchild outlived the stall");
        assert!(gone_after <= Duration::from_secs(1), "{gone_after:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A healthy turn is untouched by the watchdog.
    #[tokio::test]
    async fn bn1_ok() {
        let Some(fake) = fake_claude() else {
            return eprintln!("python3 missing: skipped");
        };
        let events = bn1_run(bn1_spec(&fake, "ok", 0.0), TurnHooks::default()).await;
        let ok = bn1_finished_ok(&events);
        bench("ok_survived", serde_json::json!(ok));
        assert!(ok);
        let text: String = events
            .iter()
            .filter_map(|(_, e)| match e {
                TurnEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("ok"), "{text}");
    }

    #[test]
    fn exit_error_keeps_the_code_and_the_stderr_tail() {
        let e = exit_error(Some(2), "line a\nline b\n");
        assert_eq!(e.code, ERR_CLI_EXIT);
        assert!(e.message.contains('2'));
        assert!(e.message.contains("line b"));
        assert!(!e.retryable);
        let signalled = exit_error(None, "");
        assert!(signalled.message.contains("signal"));
        assert!(signalled.message.contains("no stderr"));
    }
}
