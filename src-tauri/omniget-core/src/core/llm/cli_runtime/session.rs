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

use futures::channel::mpsc;
use futures::stream::BoxStream;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::super::error::{LlmError, ERR_LLM_CANCELLED};
use super::super::types::{FinishReason, TurnEvent};
use super::parse::{classify_error_text, CliSignal, RateSnapshot};
use super::{ERR_CLI_EXIT, ERR_CLI_SPAWN};

/// How long we wait for the child to die after a cancel before giving up on
/// the wait (the kill itself is a signal and is immediate). The gate wants the
/// stream closed in under 200 ms.
const KILL_GRACE_MS: u64 = 150;
/// Only the tail of stderr is kept: a CLI can print megabytes of progress.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Everything needed to run one turn, with no reference to an account: the
/// caller (claude.rs / codex.rs) has already resolved the binary and the env.
#[derive(Debug, Clone)]
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

/// Everything besides the event stream a turn can report, all optional.
#[derive(Default, Clone)]
pub struct TurnHooks {
    pub rate: Option<RateSink>,
    /// The provider's session id (`CliSignal::Session`), once per id.
    pub session: Option<Arc<dyn Fn(String) + Send + Sync>>,
    /// Right after the spawn: pid and our launch id (also in the child's
    /// environment as `OMNIGET_LAUNCH_ID`).
    pub spawned: Option<Arc<dyn Fn(Option<u32>, &str) + Send + Sync>>,
    /// Runs once when the turn task ends, whatever the path (temp files,
    /// tokens).
    pub cleanup: Option<Arc<dyn Fn() + Send + Sync>>,
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

/// Reads one line of at most `max` bytes. `Ok(None)` at EOF; an oversized
/// line comes back as `Some((String::new(), true))` after it is skipped.
async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Option<(String, bool)>> {
    buf.clear();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !oversized {
            if buf.len() + take > max {
                oversized = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        reader.consume(take);
        if done {
            break;
        }
    }
    if oversized {
        return Ok(Some((String::new(), true)));
    }
    while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
        buf.pop();
    }
    Ok(Some((String::from_utf8_lossy(buf).into_owned(), false)))
}

/// Stops the child and every process of its own group (the child is the
/// group leader: `process_group(0)` at spawn). Processes outside the group
/// are never touched. TERM first, KILL after the grace.
async fn kill_tree(child: &mut tokio::process::Child, pgid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pg) = pgid {
        // SAFETY: plain syscall on a process group id we created.
        unsafe {
            libc::killpg(pg as libc::pid_t, libc::SIGTERM);
        }
        let exited = tokio::time::timeout(
            std::time::Duration::from_millis(KILL_GRACE_MS / 2),
            child.wait(),
        )
        .await
        .is_ok();
        unsafe {
            // Descendants may outlive the leader: the group gets KILL anyway.
            libc::killpg(pg as libc::pid_t, libc::SIGKILL);
        }
        if !exited {
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(KILL_GRACE_MS / 2),
                child.wait(),
            )
            .await;
        }
        return;
    }
    let _ = pgid;
    let _ = child.start_kill();
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(KILL_GRACE_MS),
        child.wait(),
    )
    .await;
}

/// [`spawn_turn`] with every hook. The child runs in a process group of its
/// own, so a cancel stops its descendants too and nothing else.
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
    let pid = child.id();
    // With `process_group(0)` the group id is the child's pid.
    let pgid = if cfg!(unix) { pid } else { None };
    if let Some(spawned) = &hooks.spawned {
        spawned(pid, &launch_id);
    }

    let Some(stdout) = child.stdout.take() else {
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
        let mut line_buf: Vec<u8> = Vec::new();
        let mut saw_error = false;
        let mut saw_finish = false;
        let mut cancelled = false;
        let mut sessions_seen: Vec<String> = Vec::new();

        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    cancelled = true;
                    None
                }
                // A read error ends the stream like EOF: the exit status below
                // is what explains the turn.
                line = read_line_bounded(&mut reader, &mut line_buf, MAX_LINE_BYTES) => line.unwrap_or_default(),
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
                        if matches!(event, TurnEvent::Error { .. }) {
                            saw_error = true;
                        }
                        if matches!(event, TurnEvent::Finished { .. }) {
                            saw_finish = true;
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

        if cancelled {
            kill_tree(&mut child, pgid).await;
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
            args: vec![],
            env: BTreeMap::new(),
            scrub: vec![],
            cwd: None,
            stdin: None,
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
                args: vec![],
                env: BTreeMap::new(),
                scrub: vec![],
                cwd: None,
                stdin: None,
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
