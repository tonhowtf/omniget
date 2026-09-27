//! The client side of ACP that touches the machine: `fs/read_text_file`,
//! `fs/write_text_file` (confined to the thread's workspace) and
//! `terminal/*` (our own processes, output capped by `outputByteLimit`).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use super::proc::{self, Spawn};
use super::rpc::{RpcError, CODE_INTERNAL, CODE_INVALID_PARAMS, CODE_RESOURCE_NOT_FOUND};

/// Largest file `fs/read_text_file` returns.
const MAX_READ: u64 = 16 * 1024 * 1024;
/// Default output kept per terminal when the agent sets no limit.
const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;

fn lexical_clean(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// Resolve `raw` (absolute per spec; relative is taken from `root`) and make
/// sure it stays inside `root` after symlinks: the nearest existing ancestor
/// is canonicalized, so a link pointing out of the workspace is refused.
pub fn contained(root: &Path, raw: &str) -> Result<PathBuf, RpcError> {
    let outside = || {
        RpcError::new(
            CODE_INVALID_PARAMS,
            format!("path is outside the workspace: {raw}"),
        )
    };
    if raw.trim().is_empty() {
        return Err(RpcError::new(CODE_INVALID_PARAMS, "empty path"));
    }
    let p = PathBuf::from(raw);
    let joined = if p.is_absolute() { p } else { root.join(p) };
    let clean = lexical_clean(&joined).ok_or_else(outside)?;
    let root_real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    // Walk up to the first ancestor that exists, canonicalize it, re-append.
    let mut existing = clean.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return Err(outside()),
        }
    }
    let mut real = std::fs::canonicalize(&existing).map_err(|_| outside())?;
    for name in rest.into_iter().rev() {
        real.push(name);
    }
    if real.starts_with(&root_real) {
        Ok(real)
    } else {
        Err(outside())
    }
}

/// `fs/read_text_file {path, line?, limit?}` (1-based line).
pub fn read_text_file(root: &Path, params: &Value) -> Result<Value, RpcError> {
    let raw = params.get("path").and_then(Value::as_str).unwrap_or("");
    let path = contained(root, raw)?;
    let meta = std::fs::metadata(&path)
        .map_err(|e| RpcError::new(CODE_RESOURCE_NOT_FOUND, format!("{raw}: {e}")))?;
    if meta.len() > MAX_READ {
        return Err(RpcError::new(
            CODE_INVALID_PARAMS,
            format!("{raw} is larger than {} MiB", MAX_READ / 1024 / 1024),
        ));
    }
    let bytes =
        std::fs::read(&path).map_err(|e| RpcError::new(CODE_INTERNAL, format!("{raw}: {e}")))?;
    let text = String::from_utf8_lossy(&bytes);
    let line = params
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let content = if line == 1 && limit.is_none() {
        text.into_owned()
    } else {
        let mut out = String::new();
        let lines = text.split_inclusive('\n').skip(line - 1);
        match limit {
            Some(n) => lines.take(n).for_each(|l| out.push_str(l)),
            None => lines.for_each(|l| out.push_str(l)),
        }
        out
    };
    Ok(json!({ "content": content }))
}

/// `fs/write_text_file {path, content}`; creates parent folders.
pub fn write_text_file(root: &Path, params: &Value) -> Result<Value, RpcError> {
    let raw = params.get("path").and_then(Value::as_str).unwrap_or("");
    let path = contained(root, raw)?;
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(CODE_INVALID_PARAMS, "missing content"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| RpcError::new(CODE_INTERNAL, e.to_string()))?;
    }
    // Write-then-rename so a crash never leaves half a file.
    let tmp = path.with_extension(format!(
        "{}omniget-acp.tmp",
        path.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    std::fs::write(&tmp, content).map_err(|e| RpcError::new(CODE_INTERNAL, e.to_string()))?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(RpcError::new(CODE_INTERNAL, e.to_string()));
    }
    Ok(Value::Null)
}

// ── Terminals ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExitStatus {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

impl ExitStatus {
    fn to_value(&self) -> Value {
        json!({ "exitCode": self.exit_code, "signal": self.signal })
    }
}

/// Output buffer that keeps the *end* when over the limit (spec: truncate
/// from the beginning, at a character boundary).
#[derive(Debug, Default)]
pub struct OutputBuf {
    text: String,
    limit: usize,
    truncated: bool,
}

impl OutputBuf {
    pub fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            truncated: false,
        }
    }

    pub fn push(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        if self.text.len() > self.limit {
            let mut cut = self.text.len() - self.limit;
            while cut < self.text.len() && !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
            self.truncated = true;
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

struct Term {
    output: Arc<StdMutex<OutputBuf>>,
    exit: watch::Receiver<Option<ExitStatus>>,
    pid: Option<u32>,
}

/// Called with `(terminal_id, chunk)` as output arrives.
pub type OutputSink = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// The terminals of one session.
pub struct Terminals {
    root: PathBuf,
    env: Vec<(String, String)>,
    list: StdMutex<HashMap<String, Term>>,
    sink: OutputSink,
}

fn env_list(params: &Value) -> Vec<(String, String)> {
    params
        .get("env")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            Some((
                e.get("name")?.as_str()?.to_string(),
                e.get("value")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// `command` + `args` as the process to start. With no args and a command
/// line that has spaces (several agents send `"ls -la"`), a shell runs it.
pub fn argv(command: &str, args: &[String]) -> (String, Vec<String>) {
    if args.is_empty() && command.trim().contains(char::is_whitespace) {
        if cfg!(windows) {
            ("cmd".into(), vec!["/C".into(), command.to_string()])
        } else {
            ("sh".into(), vec!["-c".into(), command.to_string()])
        }
    } else {
        (command.to_string(), args.to_vec())
    }
}

impl Terminals {
    pub fn new(root: PathBuf, env: Vec<(String, String)>, sink: OutputSink) -> Arc<Self> {
        Arc::new(Self {
            root,
            env,
            list: StdMutex::new(HashMap::new()),
            sink,
        })
    }

    /// `terminal/create` → `{terminalId}`; returns right away.
    pub fn create(self: &Arc<Self>, params: &Value) -> Result<Value, RpcError> {
        let command = params
            .get("command")
            .and_then(Value::as_str)
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| RpcError::new(CODE_INVALID_PARAMS, "missing command"))?;
        let args: Vec<String> = params
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let cwd = match params.get("cwd").and_then(Value::as_str) {
            Some(c) if !c.is_empty() => contained(&self.root, c)?,
            _ => self.root.clone(),
        };
        let limit = params
            .get("outputByteLimit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_OUTPUT_LIMIT)
            .max(1);
        let (program, argv) = argv(command, &args);
        let mut env = self.env.clone();
        env.extend(env_list(params));
        let spec = Spawn {
            command: program,
            args: argv,
            env,
            env_remove: Vec::new(),
            cwd: Some(cwd),
        };
        let mut cmd = proc::command(&spec);
        cmd.stdin(std::process::Stdio::null());
        let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn()).map_err(|e| {
            RpcError::new(CODE_INTERNAL, format!("could not start `{command}`: {e}"))
        })?;
        let id = format!("term-{}", uuid::Uuid::new_v4().simple());
        let output = Arc::new(StdMutex::new(OutputBuf::new(limit)));
        let (tx, rx) = watch::channel(None);
        let pid = child.id();
        let mut readers = Vec::new();
        for pipe in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let output = output.clone();
            let sink = self.sink.clone();
            let tid = id.clone();
            readers.push(tokio::spawn(async move {
                let mut pipe = pipe;
                let mut buf = vec![0u8; 8192];
                loop {
                    match pipe.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                            output
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(&chunk);
                            sink(&tid, &chunk);
                        }
                    }
                }
            }));
        }
        tokio::spawn(async move {
            let status = child.wait().await;
            for r in readers {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), r).await;
            }
            let exit = match status {
                Ok(st) => ExitStatus {
                    exit_code: st.code(),
                    signal: signal_of(&st),
                },
                Err(_) => ExitStatus::default(),
            };
            let _ = tx.send(Some(exit));
        });
        self.list.lock().unwrap_or_else(|e| e.into_inner()).insert(
            id.clone(),
            Term {
                output,
                exit: rx,
                pid,
            },
        );
        Ok(json!({ "terminalId": id }))
    }

    fn with<T>(&self, params: &Value, f: impl FnOnce(&Term) -> T) -> Result<T, RpcError> {
        let id = params
            .get("terminalId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let t = list.get(id).ok_or_else(|| {
            RpcError::new(CODE_RESOURCE_NOT_FOUND, format!("unknown terminal {id}"))
        })?;
        Ok(f(t))
    }

    /// `terminal/output` → `{output, truncated, exitStatus?}`.
    pub fn output(&self, params: &Value) -> Result<Value, RpcError> {
        self.with(params, |t| {
            let out = t.output.lock().unwrap_or_else(|e| e.into_inner());
            let exit = t.exit.borrow().clone();
            let mut v = json!({ "output": out.text(), "truncated": out.truncated() });
            if let Some(e) = exit {
                v["exitStatus"] = e.to_value();
            }
            v
        })
    }

    /// `terminal/wait_for_exit` → `{exitCode, signal}` (awaits).
    pub async fn wait_for_exit(&self, params: &Value) -> Result<Value, RpcError> {
        let mut rx = self.with(params, |t| t.exit.clone())?;
        loop {
            if let Some(e) = rx.borrow().clone() {
                return Ok(e.to_value());
            }
            if rx.changed().await.is_err() {
                return Ok(ExitStatus::default().to_value());
            }
        }
    }

    /// `terminal/kill`: stop the command, keep the terminal readable.
    pub fn kill(&self, params: &Value) -> Result<Value, RpcError> {
        self.with(params, |t| {
            if t.exit.borrow().is_none() {
                proc::kill_tree(t.pid);
            }
        })?;
        Ok(Value::Null)
    }

    /// `terminal/release`: kill if running and forget it.
    pub fn release(&self, params: &Value) -> Result<Value, RpcError> {
        self.kill(params)?;
        let id = params
            .get("terminalId")
            .and_then(Value::as_str)
            .unwrap_or("");
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        Ok(Value::Null)
    }

    /// Session ending: kill every terminal still running.
    pub fn release_all(&self) {
        let list: Vec<Term> = self
            .list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, t)| t)
            .collect();
        for t in list {
            if t.exit.borrow().is_none() {
                proc::kill_tree(t.pid);
            }
        }
    }
}

#[cfg(unix)]
fn signal_of(st: &std::process::ExitStatus) -> Option<String> {
    use std::os::unix::process::ExitStatusExt;
    st.signal().map(|s| match s {
        1 => "SIGHUP".to_string(),
        2 => "SIGINT".to_string(),
        9 => "SIGKILL".to_string(),
        15 => "SIGTERM".to_string(),
        n => format!("SIG{n}"),
    })
}

#[cfg(not(unix))]
fn signal_of(_: &std::process::ExitStatus) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-acp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(d).unwrap()
    }

    #[test]
    fn paths_stay_inside_the_workspace() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "1\n2\n3\n").unwrap();
        assert!(contained(&root, root.join("a.txt").to_str().unwrap()).is_ok());
        assert!(contained(&root, root.join("new/deep/b.txt").to_str().unwrap()).is_ok());
        assert!(contained(&root, "/etc/passwd").is_err());
        assert!(contained(&root, root.join("../x").to_str().unwrap()).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", root.join("escape")).unwrap();
            assert!(contained(&root, root.join("escape/x.txt").to_str().unwrap()).is_err());
        }
        let r = read_text_file(
            &root,
            &json!({ "path": root.join("a.txt"), "line": 2, "limit": 1 }),
        )
        .unwrap();
        assert_eq!(r["content"], "2\n");
        write_text_file(
            &root,
            &json!({ "path": root.join("sub/c.txt"), "content": "hi" }),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("sub/c.txt")).unwrap(),
            "hi"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn output_buffer_keeps_the_tail() {
        let mut b = OutputBuf::new(5);
        b.push("abc");
        b.push("déf");
        assert!(b.truncated());
        assert!(b.text().len() <= 5);
        assert!(b.text().ends_with("éf"));
    }

    #[test]
    fn command_lines_without_args_go_through_a_shell() {
        let (p, a) = argv("ls -la", &[]);
        assert_eq!(a.last().unwrap(), "ls -la");
        assert!(p == "sh" || p == "cmd");
        let (p, a) = argv("git", &["status".into()]);
        assert_eq!((p.as_str(), a.len()), ("git", 1));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_runs_and_reports_exit() {
        let root = tmp();
        let seen = Arc::new(StdMutex::new(String::new()));
        let s2 = seen.clone();
        let terms = Terminals::new(
            root.clone(),
            Vec::new(),
            Arc::new(move |_id, chunk| s2.lock().unwrap().push_str(chunk)),
        );
        let created = terms
            .create(
                &json!({ "sessionId": "s", "command": "sh", "args": ["-c", "echo hi; exit 3"] }),
            )
            .unwrap();
        let p = json!({ "sessionId": "s", "terminalId": created["terminalId"] });
        let exit = terms.wait_for_exit(&p).await.unwrap();
        assert_eq!(exit["exitCode"], 3);
        let out = terms.output(&p).unwrap();
        assert_eq!(out["output"], "hi\n");
        assert_eq!(out["exitStatus"]["exitCode"], 3);
        assert_eq!(seen.lock().unwrap().as_str(), "hi\n");
        terms.release(&p).unwrap();
        assert!(terms.output(&p).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
