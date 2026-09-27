//! The one long-lived `claude` process of a thread: spawn, stdio pumps,
//! exit watch, and a stop that only ever touches what we started.
//!
//! Killing is scoped to the process tree we own: the root is our unreaped
//! `Child` (its pid cannot be reused while we hold it), and descendants are
//! found by walking parent pids from that root, each remembered with its
//! start time (`ps -o lstart`) and re-checked before the hard kill so a
//! recycled pid is never hit. No process-group signals, no kill by name.
//! Windows uses `taskkill /T` on our own pid.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

/// How long a stop waits for the CLI to exit after stdin closes.
pub const GRACE: Duration = Duration::from_secs(3);
const STDERR_TAIL: usize = 8 * 1024;

pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// How the process ended.
#[derive(Debug, Clone)]
pub struct Exit {
    pub code: Option<i32>,
    pub killed: bool,
    pub stderr_tail: String,
}

/// Handle kept by the session. Dropping `stdin` closes the pipe (the CLI then
/// exits by itself); `kill` asks the watcher for a hard kill of the tree.
pub struct ProcHandle {
    pub pid: Option<u32>,
    pub stdin: mpsc::UnboundedSender<String>,
    kill: Option<oneshot::Sender<()>>,
    pub exited: watch::Receiver<bool>,
}

impl ProcHandle {
    /// Close stdin, wait for a graceful exit, then kill what is left of the
    /// tree. The descendants are listed *before* stdin closes: once the root
    /// is gone they are reparented and no longer traceable to it.
    pub async fn shutdown(mut self) {
        let tree = match self.pid {
            Some(pid) => descendants(pid).await,
            None => Vec::new(),
        };
        drop(std::mem::replace(
            &mut self.stdin,
            mpsc::unbounded_channel().0,
        ));
        let mut exited = self.exited.clone();
        let graceful = tokio::time::timeout(GRACE, exited.wait_for(|v| *v))
            .await
            .is_ok();
        if !graceful {
            if let Some(k) = self.kill.take() {
                let _ = k.send(());
            }
            let _ = tokio::time::timeout(GRACE, exited.wait_for(|v| *v)).await;
        }
        kill_listed(&tree).await;
    }

    /// Hard kill now (interrupt watchdog).
    pub fn kill_now(&mut self) {
        if let Some(k) = self.kill.take() {
            let _ = k.send(());
        }
    }
}

/// Spawns the CLI. `on_line` gets every stdout line (in order, on one task);
/// `on_exit` runs once when the process is gone.
pub fn spawn(
    spec: SpawnSpec,
    on_line: impl Fn(String) + Send + 'static,
    on_exit: impl FnOnce(Exit) + Send + 'static,
) -> std::io::Result<ProcHandle> {
    let mut cmd = crate::core::process::command(&spec.program);
    cmd.args(&spec.args);
    for key in &spec.scrub {
        cmd.env_remove(key);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tracing::info!(
        "[claude] spawn {} {} (cwd {:?}, env {:?})",
        spec.program.display(),
        spec.args
            .iter()
            .filter(|a| a.len() < 120)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
        spec.cwd,
        spec.env.keys().collect::<Vec<_>>()
    );
    let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn())?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    // stdin pump: one writer, lines in order.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if let Some(mut pipe) = stdin {
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let mut bytes = line.into_bytes();
                bytes.push(b'\n');
                if pipe.write_all(&bytes).await.is_err() || pipe.flush().await.is_err() {
                    break;
                }
            }
            // Channel closed = stop: dropping the pipe sends EOF.
        });
    }

    // stderr tail, drained so the pipe never blocks the child.
    let (err_tx, err_rx) = oneshot::channel::<String>();
    match stderr {
        Some(mut pipe) => {
            tokio::spawn(async move {
                let mut tail: Vec<u8> = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    match pipe.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            tail.extend_from_slice(&buf[..n]);
                            if tail.len() > STDERR_TAIL * 2 {
                                tail.drain(..tail.len() - STDERR_TAIL);
                            }
                        }
                    }
                }
                if tail.len() > STDERR_TAIL {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
                let _ = err_tx.send(String::from_utf8_lossy(&tail).into_owned());
            });
        }
        None => {
            let _ = err_tx.send(String::new());
        }
    }

    // stdout: the protocol. Lines can be large (tool results), so no cap.
    let (out_done_tx, out_done_rx) = oneshot::channel::<()>();
    match stdout {
        Some(pipe) => {
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        on_line(line);
                    }
                }
                let _ = out_done_tx.send(());
            });
        }
        None => {
            let _ = out_done_tx.send(());
        }
    }

    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    let (exit_tx, exit_rx) = watch::channel(false);
    tokio::spawn(async move {
        let mut killed = false;
        let status = tokio::select! {
            s = child.wait() => s.ok(),
            _ = kill_rx => {
                killed = true;
                if let Some(pid) = pid {
                    let tree = descendants(pid).await;
                    kill_listed(&tree).await;
                }
                let _ = child.start_kill();
                child.wait().await.ok()
            }
        };
        // Let stdout drain so the last lines (the final `result`) are seen
        // before the exit is reported.
        let _ = tokio::time::timeout(Duration::from_secs(2), out_done_rx).await;
        let stderr_tail = tokio::time::timeout(Duration::from_secs(1), err_rx)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let _ = exit_tx.send(true);
        on_exit(Exit {
            code: status.and_then(|s| s.code()),
            killed,
            stderr_tail,
        });
    });

    Ok(ProcHandle {
        pid,
        stdin: tx,
        kill: Some(kill_tx),
        exited: exit_rx,
    })
}

/// One process of our tree: pid + start time as `ps` prints it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub pid: u32,
    pub started: String,
}

/// Parses `ps -A -o pid=,ppid=,lstart=` into `pid → (ppid, lstart)`.
pub fn parse_ps(text: &str) -> HashMap<u32, (u32, String)> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        let started = it.collect::<Vec<_>>().join(" ");
        out.insert(pid, (ppid, started));
    }
    out
}

/// Every descendant of `root` in a `parse_ps` table, deepest first.
pub fn tree_of(root: u32, table: &HashMap<u32, (u32, String)>) -> Vec<Stamped> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (pid, (ppid, _)) in table {
        children.entry(*ppid).or_default().push(*pid);
    }
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(p) = stack.pop() {
        for c in children.get(&p).cloned().unwrap_or_default() {
            if c != root && !order.iter().any(|s: &Stamped| s.pid == c) {
                order.push(Stamped {
                    pid: c,
                    started: table.get(&c).map(|t| t.1.clone()).unwrap_or_default(),
                });
                stack.push(c);
            }
        }
    }
    order.reverse();
    order
}

#[cfg(unix)]
async fn ps_table() -> HashMap<u32, (u32, String)> {
    let out = crate::core::process::command("ps")
        .args(["-A", "-o", "pid=,ppid=,lstart="])
        .env("LC_ALL", "C")
        .output()
        .await;
    match out {
        Ok(o) => parse_ps(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => HashMap::new(),
    }
}

/// Descendants of our child, stamped.
pub async fn descendants(root: u32) -> Vec<Stamped> {
    #[cfg(unix)]
    {
        tree_of(root, &ps_table().await)
    }
    #[cfg(not(unix))]
    {
        // `taskkill /T` walks the tree itself; remember only the root.
        vec![Stamped {
            pid: root,
            started: String::new(),
        }]
    }
}

/// SIGTERM the listed processes, give them a moment, then SIGKILL the ones
/// still alive **with the same start time**.
pub async fn kill_listed(list: &[Stamped]) {
    if list.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        for s in list {
            // SAFETY: plain signal to a single pid we listed from our tree.
            unsafe {
                libc::kill(s.pid as libc::pid_t, libc::SIGTERM);
            }
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let now = ps_table().await;
        for s in list {
            if now.get(&s.pid).map(|t| &t.1) == Some(&s.started) {
                // SAFETY: same pid and same start time as when listed.
                unsafe {
                    libc::kill(s.pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
    #[cfg(windows)]
    {
        for s in list {
            let _ = crate::core::process::command("taskkill")
                .args(["/PID", &s.pid.to_string(), "/T", "/F"])
                .output()
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_is_walked_from_our_root_only() {
        let ps = "  100     1 Tue Sep 22 19:30:00 2026\n  200   100 Tue Sep 22 19:30:01 2026\n  201   200 Tue Sep 22 19:30:02 2026\n  300     1 Tue Sep 22 19:30:03 2026\n  301   300 Tue Sep 22 19:30:04 2026\n";
        let table = parse_ps(ps);
        assert_eq!(table.get(&201).unwrap().0, 200);
        let tree = tree_of(100, &table);
        let pids: Vec<u32> = tree.iter().map(|s| s.pid).collect();
        // Deepest first, and nothing outside our root.
        assert_eq!(pids, vec![201, 200]);
        assert_eq!(tree[0].started, "Tue Sep 22 19:30:02 2026");
        assert!(tree_of(999, &table).is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_closes_stdin_and_kills_the_grandchild() {
        // A fake CLI that echoes stdin and keeps a grandchild alive.
        let dir = std::env::temp_dir().join(format!("omniget-claude-sup-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("fake.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nsleep 300 &\necho started\nwhile read l; do echo \"$l\"; done\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
        let (exit_tx, exit_rx) = oneshot::channel::<Exit>();
        let handle = spawn(
            SpawnSpec {
                program: script.clone(),
                args: vec![],
                env: BTreeMap::new(),
                scrub: vec![],
                cwd: None,
            },
            move |l| {
                let _ = line_tx.send(l);
            },
            move |e| {
                let _ = exit_tx.send(e);
            },
        )
        .unwrap();
        assert_eq!(line_rx.recv().await.as_deref(), Some("started"));
        handle.stdin.send("{\"ping\":1}".into()).unwrap();
        assert_eq!(line_rx.recv().await.as_deref(), Some("{\"ping\":1}"));
        let root = handle.pid.unwrap();
        let tree = descendants(root).await;
        assert!(!tree.is_empty(), "the sleep grandchild must be listed");
        handle.shutdown().await;
        let exit = exit_rx.await.unwrap();
        assert!(!exit.killed, "stdin EOF must end the script gracefully");
        let now = ps_table().await;
        for s in &tree {
            assert_ne!(
                now.get(&s.pid).map(|t| &t.1),
                Some(&s.started),
                "pid {} survived",
                s.pid
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
