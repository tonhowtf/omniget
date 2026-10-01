use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

pub const GRACE: Duration = Duration::from_secs(3);
const STDERR_TAIL: usize = 8 * 1024;

pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub scrub: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Exit {
    pub code: Option<i32>,
    pub killed: bool,
    pub stderr_tail: String,
}

pub struct ProcHandle {
    pub pid: Option<u32>,
    pub stdin: mpsc::UnboundedSender<String>,
    kill: Option<oneshot::Sender<()>>,
    pub exited: watch::Receiver<bool>,
}

impl ProcHandle {
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

    pub fn kill_now(&mut self) {
        if let Some(k) = self.kill.take() {
            let _ = k.send(());
        }
    }
}

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
        });
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub pid: u32,
    pub started: String,
}

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

pub async fn descendants(root: u32) -> Vec<Stamped> {
    #[cfg(unix)]
    {
        tree_of(root, &ps_table().await)
    }
    #[cfg(not(unix))]
    {
        vec![Stamped {
            pid: root,
            started: String::new(),
        }]
    }
}

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
