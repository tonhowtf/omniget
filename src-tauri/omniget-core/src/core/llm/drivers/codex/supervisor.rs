//! The `codex app-server` process: spawn, stderr tail, exit watch, and a
//! stop that kills only what OmniGet started (the app-server and its own
//! descendants: the shells and MCP servers Codex launched), never a process
//! group or anything else on the machine.
//!
//! Restart policy (the driver applies it): a process that dies on its own is
//! restarted on the next call that needs it, resuming the same Codex thread
//! (`thread/resume` with the cursor), at most [`RESTART_LIMIT`] times per
//! [`RESTART_WINDOW`]. Nothing is restarted while no one asks: an idle thread
//! costs no process.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{oneshot, watch};

use super::super::{DriverError, ERR_DRIVER_FAILED, ERR_DRIVER_UNAVAILABLE};
use super::launch::LaunchSpec;

pub const RESTART_LIMIT: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(5 * 60);
/// After SIGTERM, how long the tree gets before the hard kill.
const KILL_GRACE: Duration = Duration::from_secs(2);
const STDERR_TAIL_LINES: usize = 20;

/// Crash restarts allowed in a sliding window.
#[derive(Debug, Default)]
pub struct RestartBudget {
    times: VecDeque<Instant>,
}

impl RestartBudget {
    /// Records a restart at `now` if the budget allows it.
    pub fn allow(&mut self, now: Instant) -> bool {
        while let Some(first) = self.times.front() {
            if now.duration_since(*first) > RESTART_WINDOW {
                self.times.pop_front();
            } else {
                break;
            }
        }
        if self.times.len() >= RESTART_LIMIT {
            return false;
        }
        self.times.push_back(now);
        true
    }
}

/// Descendants of `root` in a `(pid, ppid)` table, deepest first, so a kill
/// in this order never lets a child be re-parented before it is reached.
pub fn descendants(table: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut levels: Vec<Vec<u32>> = Vec::new();
    let mut frontier = vec![root];
    let mut seen = std::collections::HashSet::new();
    seen.insert(root);
    while !frontier.is_empty() {
        let next: Vec<u32> = table
            .iter()
            .filter(|(pid, ppid)| frontier.contains(ppid) && seen.insert(*pid))
            .map(|(pid, _)| *pid)
            .collect();
        if next.is_empty() {
            break;
        }
        levels.push(next.clone());
        frontier = next;
    }
    levels.into_iter().rev().flatten().collect()
}

/// Parses `ps -A -o pid= -o ppid=`.
pub fn parse_ps(text: &str) -> Vec<(u32, u32)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

#[cfg(unix)]
fn process_table() -> Vec<(u32, u32)> {
    crate::core::process::std_command("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Asks the tree rooted at `pid` to stop: descendants first, then the root.
#[cfg(unix)]
fn terminate_tree(pid: u32) {
    let table = process_table();
    for child in descendants(&table, pid) {
        // SAFETY: plain kill(2) on a pid we just read as our descendant.
        unsafe {
            libc::kill(child as libc::pid_t, libc::SIGTERM);
        }
    }
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(unix)]
fn kill_tree_hard(pid: u32) {
    let table = process_table();
    for child in descendants(&table, pid) {
        unsafe {
            libc::kill(child as libc::pid_t, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
fn terminate_tree(pid: u32) {
    // `/T` walks the tree of this pid only.
    let _ = crate::core::process::std_command("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(windows)]
fn kill_tree_hard(pid: u32) {
    terminate_tree(pid);
}

#[cfg(not(any(unix, windows)))]
fn terminate_tree(_pid: u32) {}
#[cfg(not(any(unix, windows)))]
fn kill_tree_hard(_pid: u32) {}

/// Drops ANSI escapes and Codex's own tracing lines
/// (`2026-09-22T22:36:39Z ERROR codex_api::…: …`), which duplicate the
/// `error` notifications. What is left is a real crash message.
pub fn stderr_line_of_interest(line: &str) -> Option<String> {
    let mut clean = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        clean.push(c);
    }
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    let b = clean.as_bytes();
    let tracing = b.len() > 20
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T';
    if tracing {
        return None;
    }
    Some(clean.to_string())
}

/// A live app-server process owned by one session.
pub struct ProcessHandle {
    pub pid: Option<u32>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    /// `None` while running; `Some(code)` once it exited.
    pub exit: watch::Receiver<Option<Option<i32>>>,
    pub stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl ProcessHandle {
    /// Stops the tree (idempotent).
    pub fn stop(&self) {
        if let Some(tx) = self.kill.lock().ok().and_then(|mut k| k.take()) {
            let _ = tx.send(());
        }
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    /// Waits a moment for the exit code (after the stdout EOF).
    pub async fn exit_code(&self, wait: Duration) -> Option<i32> {
        let mut rx = self.exit.clone();
        if let Some(code) = *rx.borrow() {
            return code;
        }
        let _ = tokio::time::timeout(wait, rx.changed()).await;
        let code = *rx.borrow();
        code.flatten()
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The pipes of one app-server.
pub struct Connection {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    pub process: Option<ProcessHandle>,
}

/// Opens an app-server. The real one spawns the CLI; tests plug a scripted
/// peer over `tokio::io::duplex`.
#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self, spec: &LaunchSpec) -> Result<Connection, DriverError>;
}

/// Spawns `codex app-server` for real.
pub struct ProcessConnector;

async fn resolve_program(spec: &LaunchSpec) -> Result<std::path::PathBuf, DriverError> {
    let bare = spec.program.components().count() == 1;
    if !bare {
        return Ok(spec.program.clone());
    }
    let name = spec.program.to_string_lossy().to_string();
    crate::core::dependencies::find_tool(&name)
        .await
        .ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_UNAVAILABLE,
                format!("the Codex CLI (`{name}`) is not installed or not on PATH"),
            )
        })
}

#[async_trait]
impl Connector for ProcessConnector {
    async fn connect(&self, spec: &LaunchSpec) -> Result<Connection, DriverError> {
        let program = resolve_program(spec).await?;
        let mut cmd = crate::core::process::command(&program);
        cmd.args(&spec.args);
        for key in &spec.scrub {
            cmd.env_remove(key);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(dir) = spec.cwd.as_ref().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        tracing::info!(
            "[codex] spawn {} {} (CODEX_HOME {})",
            program.display(),
            spec.args.join(" "),
            spec.codex_home
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "default".into())
        );
        let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn()).map_err(|e| {
            DriverError::new(
                ERR_DRIVER_FAILED,
                format!("cannot start {}: {e}", program.display()),
            )
        })?;
        let pid = child.id();
        let stdout = child.stdout.take().ok_or_else(|| {
            DriverError::new(ERR_DRIVER_FAILED, "codex app-server gave no stdout")
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| DriverError::new(ERR_DRIVER_FAILED, "codex app-server gave no stdin"))?;
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(l) = stderr_line_of_interest(&line) {
                        tracing::debug!(
                            "[codex] stderr: {}",
                            l.chars().take(300).collect::<String>()
                        );
                        if let Ok(mut t) = tail.lock() {
                            if t.len() >= STDERR_TAIL_LINES {
                                t.pop_front();
                            }
                            t.push_back(l);
                        }
                    }
                }
            });
        }
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = watch::channel::<Option<Option<i32>>>(None);
        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let _ = exit_tx.send(Some(status.ok().and_then(|s| s.code())));
                }
                _ = kill_rx => {
                    if let Some(pid) = pid {
                        terminate_tree(pid);
                    }
                    let status = match tokio::time::timeout(KILL_GRACE, child.wait()).await {
                        Ok(s) => s.ok(),
                        Err(_) => {
                            if let Some(pid) = pid {
                                kill_tree_hard(pid);
                            }
                            let _ = child.kill().await;
                            child.wait().await.ok()
                        }
                    };
                    let _ = exit_tx.send(Some(status.and_then(|s| s.code())));
                }
            }
        });
        Ok(Connection {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            process: Some(ProcessHandle {
                pid,
                kill: Mutex::new(Some(kill_tx)),
                exit: exit_rx,
                stderr_tail: tail,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_are_listed_deepest_first_and_only_under_the_root() {
        // 100 = app-server; 101, 102 its children; 103 grandchild; 200 unrelated.
        let table = vec![
            (100, 1),
            (101, 100),
            (102, 100),
            (103, 101),
            (200, 1),
            (201, 200),
        ];
        let d = descendants(&table, 100);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0], 103, "grandchild first");
        assert!(d.contains(&101) && d.contains(&102));
        assert!(!d.contains(&200) && !d.contains(&201) && !d.contains(&100));
        // A cycle in a stale table never loops.
        let table = vec![(5, 6), (6, 5)];
        assert_eq!(descendants(&table, 5), vec![6]);
    }

    #[test]
    fn ps_output_parses() {
        let t = parse_ps("  1     0\n 345   1\nbad line\n 346 345\n");
        assert_eq!(t, vec![(1, 0), (345, 1), (346, 345)]);
    }

    #[test]
    fn restart_budget_slides() {
        let mut b = RestartBudget::default();
        let t0 = Instant::now();
        assert!(b.allow(t0));
        assert!(b.allow(t0 + Duration::from_secs(1)));
        assert!(b.allow(t0 + Duration::from_secs(2)));
        assert!(!b.allow(t0 + Duration::from_secs(3)));
        assert!(b.allow(t0 + RESTART_WINDOW + Duration::from_secs(2)));
    }

    #[test]
    fn stderr_filter_drops_tracing_and_ansi() {
        let traced = "\u{1b}[2m2026-09-22T22:36:39.427977Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m \u{1b}[2mcodex_api::endpoint::responses_websocket\u{1b}[0m\u{1b}[2m:\u{1b}[0m failed to connect to websocket: HTTP error: 401 Unauthorized";
        assert_eq!(stderr_line_of_interest(traced), None);
        assert_eq!(
            stderr_line_of_interest("thread 'main' panicked at src/main.rs:1"),
            Some("thread 'main' panicked at src/main.rs:1".into())
        );
        assert_eq!(stderr_line_of_interest("   "), None);
    }
}
