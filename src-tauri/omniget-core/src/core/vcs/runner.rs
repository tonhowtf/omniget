//! One way to run `git`, `gh` and `glab`: the system binary through
//! `core::process`, never interactive, with a timeout and a bounded output.
//! Stderr stays out of the error text beyond a short tail (it can carry a
//! remote URL with a token in it), and every failure has a code the front can
//! switch on.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const ERR_GIT_MISSING: &str = "ERR_VCS_GIT_MISSING";
pub const ERR_SPAWN: &str = "ERR_VCS_SPAWN";
pub const ERR_TIMEOUT: &str = "ERR_VCS_TIMEOUT";
pub const ERR_EXIT: &str = "ERR_VCS_EXIT";
pub const ERR_OUTPUT_LIMIT: &str = "ERR_VCS_OUTPUT_LIMIT";
pub const ERR_NOT_REPO: &str = "ERR_VCS_NOT_REPO";
pub const ERR_INVALID: &str = "ERR_VCS_INVALID";
pub const ERR_UNSAFE: &str = "ERR_VCS_UNSAFE";
pub const ERR_NOT_FOUND: &str = "ERR_VCS_NOT_FOUND";
pub const ERR_UNAVAILABLE: &str = "ERR_VCS_UNAVAILABLE";
pub const ERR_CANCELLED: &str = "ERR_VCS_CANCELLED";
pub const ERR_IO: &str = "ERR_VCS_IO";

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_OUTPUT: usize = 16 * 1024 * 1024;

/// Typed failure of a VCS operation. `Display` gives the `"CODE: message"`
/// form the Tauri commands return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsError {
    /// The program is not on the PATH.
    Missing {
        program: String,
    },
    Spawn {
        program: String,
        message: String,
    },
    Timeout {
        program: String,
        op: String,
        secs: u64,
    },
    OutputLimit {
        program: String,
        op: String,
        limit: usize,
    },
    /// Non-zero exit. `stderr` is the last few lines, trimmed.
    Exit {
        program: String,
        op: String,
        code: Option<i32>,
        stderr: String,
    },
    NotRepo {
        path: String,
    },
    Invalid(String),
    /// Refused because it could touch something that is not ours.
    Unsafe(String),
    NotFound(String),
    Unavailable(String),
    Cancelled,
    Io(String),
}

impl VcsError {
    pub fn code(&self) -> &'static str {
        match self {
            VcsError::Missing { .. } => ERR_GIT_MISSING,
            VcsError::Spawn { .. } => ERR_SPAWN,
            VcsError::Timeout { .. } => ERR_TIMEOUT,
            VcsError::OutputLimit { .. } => ERR_OUTPUT_LIMIT,
            VcsError::Exit { .. } => ERR_EXIT,
            VcsError::NotRepo { .. } => ERR_NOT_REPO,
            VcsError::Invalid(_) => ERR_INVALID,
            VcsError::Unsafe(_) => ERR_UNSAFE,
            VcsError::NotFound(_) => ERR_NOT_FOUND,
            VcsError::Unavailable(_) => ERR_UNAVAILABLE,
            VcsError::Cancelled => ERR_CANCELLED,
            VcsError::Io(_) => ERR_IO,
        }
    }

    pub fn exit_stderr(&self) -> Option<&str> {
        match self {
            VcsError::Exit { stderr, .. } => Some(stderr),
            _ => None,
        }
    }
}

impl std::fmt::Display for VcsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.code();
        match self {
            VcsError::Missing { program } => {
                write!(f, "{code}: {program} was not found on the PATH")
            }
            VcsError::Spawn { program, message } => {
                write!(f, "{code}: could not start {program}: {message}")
            }
            VcsError::Timeout { program, op, secs } => {
                write!(f, "{code}: {program} {op} took longer than {secs}s")
            }
            VcsError::OutputLimit { program, op, limit } => {
                write!(f, "{code}: {program} {op} wrote more than {limit} bytes")
            }
            VcsError::Exit {
                program,
                op,
                code: exit,
                stderr,
            } => {
                let exit = exit
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into());
                if stderr.is_empty() {
                    write!(f, "{code}: {program} {op} failed (exit {exit})")
                } else {
                    write!(f, "{code}: {program} {op} failed (exit {exit}): {stderr}")
                }
            }
            VcsError::NotRepo { path } => {
                write!(f, "{code}: {path} is not inside a git repository")
            }
            VcsError::Invalid(m)
            | VcsError::Unsafe(m)
            | VcsError::NotFound(m)
            | VcsError::Unavailable(m)
            | VcsError::Io(m) => write!(f, "{code}: {m}"),
            VcsError::Cancelled => write!(f, "{code}: cancelled"),
        }
    }
}

impl std::error::Error for VcsError {}

impl From<VcsError> for String {
    fn from(e: VcsError) -> String {
        e.to_string()
    }
}

impl From<std::io::Error> for VcsError {
    fn from(e: std::io::Error) -> Self {
        VcsError::Io(e.to_string())
    }
}

pub type VcsResult<T> = Result<T, VcsError>;

/// Raw result of a finished process.
#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: Option<i32>,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn trimmed(&self) -> String {
        self.text().trim().to_string()
    }
    pub fn err_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// One invocation. Built with the helpers, run with [`Invocation::run`] or
/// [`Invocation::run_streaming`].
#[derive(Debug, Clone)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, Option<OsString>)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
    pub max_output: usize,
    /// When false a non-zero exit is returned as `Ok(Output)`.
    pub check: bool,
}

/// Env that keeps every git/ssh/credential helper from asking anything.
pub fn quiet_env() -> Vec<(OsString, Option<OsString>)> {
    [
        ("GIT_TERMINAL_PROMPT", Some("0")),
        ("GCM_INTERACTIVE", Some("never")),
        ("GIT_ASKPASS", Some("")),
        ("SSH_ASKPASS", Some("")),
        ("SSH_ASKPASS_REQUIRE", Some("never")),
        ("GIT_EDITOR", Some(":")),
        ("GIT_SEQUENCE_EDITOR", Some(":")),
        ("GIT_PAGER", Some("cat")),
        ("PAGER", Some("cat")),
        ("GH_PROMPT_DISABLED", Some("1")),
        ("GH_NO_UPDATE_NOTIFIER", Some("1")),
        ("GLAB_NO_PROMPT", Some("1")),
        ("NO_COLOR", Some("1")),
        ("LC_ALL", Some("C")),
        ("LANG", Some("C")),
    ]
    .into_iter()
    .map(|(k, v)| (OsString::from(k), v.map(OsString::from)))
    .collect()
}

impl Invocation {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_string(),
            args: Vec::new(),
            cwd: None,
            env: quiet_env(),
            stdin: None,
            timeout: Some(DEFAULT_TIMEOUT),
            max_output: DEFAULT_MAX_OUTPUT,
            check: true,
        }
    }

    /// `git` with the config every call wants: no pager, raw paths, no
    /// fsmonitor daemon spawned on the user's behalf.
    pub fn git() -> Self {
        Self::new("git").args([
            "-c",
            "core.quotepath=false",
            "-c",
            "core.pager=cat",
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
        ])
    }

    pub fn arg(mut self, a: impl AsRef<OsStr>) -> Self {
        self.args.push(a.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(it.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    pub fn cwd(mut self, p: impl AsRef<Path>) -> Self {
        self.cwd = Some(p.as_ref().to_path_buf());
        self
    }

    pub fn env(mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> Self {
        self.env
            .push((k.as_ref().to_os_string(), Some(v.as_ref().to_os_string())));
        self
    }

    pub fn env_remove(mut self, k: impl AsRef<OsStr>) -> Self {
        self.env.push((k.as_ref().to_os_string(), None));
        self
    }

    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }

    pub fn timeout(mut self, d: Option<Duration>) -> Self {
        self.timeout = d;
        self
    }

    pub fn max_output(mut self, n: usize) -> Self {
        self.max_output = n;
        self
    }

    pub fn unchecked(mut self) -> Self {
        self.check = false;
        self
    }

    fn op_label(&self) -> String {
        // First argument that is not a `-c key=value` pair.
        let mut it = self.args.iter();
        while let Some(a) = it.next() {
            let s = a.to_string_lossy();
            if s == "-c" || s == "-C" || s == "--git-dir" || s == "--work-tree" {
                it.next();
                continue;
            }
            if s.starts_with('-') {
                continue;
            }
            return s.into_owned();
        }
        String::new()
    }

    fn build(&self) -> tokio::process::Command {
        let mut cmd = crate::core::process::command(&self.program);
        cmd.args(&self.args);
        if let Some(c) = &self.cwd {
            cmd.current_dir(c);
        }
        for (k, v) in &self.env {
            match v {
                Some(v) => {
                    cmd.env(k, v);
                }
                None => {
                    cmd.env_remove(k);
                }
            }
        }
        cmd.stdin(if self.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd
    }

    fn spawn(&self) -> VcsResult<tokio::process::Child> {
        let mut cmd = self.build();
        crate::core::process::spawn_retrying_busy(|| cmd.spawn()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                VcsError::Missing {
                    program: self.program.clone(),
                }
            } else {
                VcsError::Spawn {
                    program: self.program.clone(),
                    message: e.to_string(),
                }
            }
        })
    }

    /// Runs to completion and collects stdout/stderr.
    pub async fn run(&self) -> VcsResult<Output> {
        self.run_streaming(|_| {}).await
    }

    /// Like [`run`](Self::run), but every stderr line (split on `\n` and `\r`,
    /// so progress meters arrive as they redraw) goes to `on_line` as it comes.
    pub async fn run_streaming(&self, mut on_line: impl FnMut(&str) + Send) -> VcsResult<Output> {
        let mut child = self.spawn()?;
        let op = self.op_label();
        if let Some(data) = self.stdin.clone() {
            if let Some(mut stdin) = child.stdin.take() {
                tokio::spawn(async move {
                    let _ = stdin.write_all(&data).await;
                    let _ = stdin.shutdown().await;
                });
            }
        }
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let limit = self.max_output;
        let program = self.program.clone();

        let work = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            // Heap buffers: keeps this future small (they nest deep in debug).
            let mut ob = vec![0u8; 16384];
            let mut eb = vec![0u8; 4096];
            let mut line = Vec::new();
            let mut out_open = true;
            let mut err_open = true;
            while out_open || err_open {
                tokio::select! {
                    r = stdout.read(&mut ob), if out_open => {
                        match r {
                            Ok(0) | Err(_) => out_open = false,
                            Ok(n) => {
                                if out.len() + n > limit {
                                    return Err(VcsError::OutputLimit { program: program.clone(), op: op.clone(), limit });
                                }
                                out.extend_from_slice(&ob[..n]);
                            }
                        }
                    }
                    r = stderr.read(&mut eb), if err_open => {
                        match r {
                            Ok(0) | Err(_) => err_open = false,
                            Ok(n) => {
                                for &b in &eb[..n] {
                                    if b == b'\n' || b == b'\r' {
                                        if !line.is_empty() {
                                            on_line(&String::from_utf8_lossy(&line));
                                            line.clear();
                                        }
                                    } else {
                                        line.push(b);
                                    }
                                }
                                if err.len() < 256 * 1024 {
                                    err.extend_from_slice(&eb[..n]);
                                }
                            }
                        }
                    }
                }
            }
            if !line.is_empty() {
                on_line(&String::from_utf8_lossy(&line));
            }
            let status = child.wait().await.map_err(VcsError::from)?;
            Ok(Output {
                stdout: out,
                stderr: err,
                code: status.code(),
            })
        };

        let work = Box::pin(work);
        let output = match self.timeout {
            Some(t) => match tokio::time::timeout(t, work).await {
                Ok(r) => r?,
                Err(_) => {
                    return Err(VcsError::Timeout {
                        program: self.program.clone(),
                        op,
                        secs: t.as_secs(),
                    })
                }
            },
            None => work.await?,
        };
        if self.check && !output.ok() {
            return Err(VcsError::Exit {
                program: self.program.clone(),
                op,
                code: output.code,
                stderr: tail(&output.err_text(), 6, 600),
            });
        }
        Ok(output)
    }
}

/// Last `lines` non-empty lines of `text`, capped at `max` chars, with any
/// `scheme://user:secret@` credential blanked.
pub fn tail(text: &str, lines: usize, max: usize) -> String {
    let kept: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = kept.len().saturating_sub(lines);
    let mut s = redact(&kept[start..].join("\n"));
    if s.chars().count() > max {
        s = s.chars().skip(s.chars().count() - max).collect();
    }
    s
}

/// Blanks the userinfo part of URLs (`https://user:token@host` → `https://***@host`).
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/' || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        if let Some(at) = authority.rfind('@') {
            out.push_str("***");
            out.push_str(&authority[at..]);
        } else {
            out.push_str(authority);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Program resolvable on the (enhanced) PATH, checked without spawning.
pub fn which(program: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{program}.exe"),
            format!("{program}.cmd"),
            program.to_string(),
        ]
    } else {
        vec![program.to_string()]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(bin) = crate::core::paths::app_data_dir() {
        dirs.insert(0, bin.join("bin"));
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
        if let Some(h) = dirs::home_dir() {
            dirs.push(h.join(".local").join("bin"));
        }
    }
    for d in dirs {
        for n in &names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials() {
        assert_eq!(
            redact("fatal: https://me:ghp_x@github.com/a/b.git denied"),
            "fatal: https://***@github.com/a/b.git denied"
        );
        assert_eq!(redact("ssh://git@host/x"), "ssh://***@host/x");
        assert_eq!(redact("no url here"), "no url here");
    }
}
