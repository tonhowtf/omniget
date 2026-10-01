use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncReadExt;

pub const STDERR_TAIL: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct Spawn {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub cwd: Option<PathBuf>,
}

fn search_path() -> Vec<PathBuf> {
    let probe = crate::core::process::std_command("x");
    let from_cmd = probe
        .get_envs()
        .find(|(k, _)| *k == "PATH")
        .and_then(|(_, v)| v.map(|v| v.to_os_string()));
    let raw = from_cmd
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(extra) = std::env::var_os("OMNIGET_CLITOOLS_EXTRA_PATH") {
        out.extend(std::env::split_paths(&extra));
    }
    out.extend(std::env::split_paths(&raw));
    if let Some(home) = dirs::home_dir() {
        for d in [".local/bin", ".opencode/bin", ".bun/bin", ".npm-global/bin"] {
            let p = d.split('/').fold(home.clone(), |acc, part| acc.join(part));
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

pub fn which(bin: &str) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    let as_path = Path::new(bin);
    if as_path.components().count() > 1 {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        Vec::new()
    };
    for dir in search_path() {
        let plain = dir.join(bin);
        if cfg!(windows) {
            for ext in &exts {
                let cand = dir.join(format!("{bin}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
            if plain.is_file() && plain.extension().is_some() {
                return Some(plain);
            }
        } else if is_executable(&plain) {
            return Some(plain);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

pub fn command(spec: &Spawn) -> tokio::process::Command {
    let program = which(&spec.command)
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| spec.command.clone().into());
    let mut cmd = crate::core::process::command(program);
    cmd.args(&spec.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for name in &spec.env_remove {
        cmd.env_remove(name);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

pub fn spawn(spec: &Spawn) -> std::io::Result<tokio::process::Child> {
    let mut cmd = command(spec);
    crate::core::process::spawn_retrying_busy(|| cmd.spawn())
}

pub fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(unix)]
    {
        let pgid = pid as i32;
        if pgid > 1 {
            // SAFETY: plain syscall; a group that is already gone is ESRCH.
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
            }
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                // SAFETY: as above.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            });
        }
    }
    #[cfg(windows)]
    {
        let _ = crate::core::process::std_command("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[derive(Clone, Default)]
pub struct Tail(Arc<Mutex<String>>);

impl Tail {
    pub fn push(&self, chunk: &str) {
        let mut t = self.0.lock().unwrap_or_else(|e| e.into_inner());
        t.push_str(chunk);
        if t.len() > STDERR_TAIL {
            let mut cut = t.len() - STDERR_TAIL;
            while !t.is_char_boundary(cut) {
                cut += 1;
            }
            t.drain(..cut);
        }
    }

    pub fn text(&self) -> String {
        sanitize(&self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub fn drain_into<R: tokio::io::AsyncRead + Unpin + Send + 'static>(mut pipe: R, tail: Tail) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match pipe.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => tail.push(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
}

pub fn sanitize(s: &str) -> String {
    use std::sync::OnceLock;
    static RES: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            (r"(?i)(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}", "$1 ***"),
            (
                r"(?i)(x-api-key|api[_-]?key|token|password|secret)([=:]\s*)\S+",
                "$1$2***",
            ),
            (r"\b(sk|pk|rk)-[A-Za-z0-9_-]{12,}", "$1-***"),
            (
                r"\b(ghp|gho|ghu|ghs|github_pat|xox[abpors])[-_][A-Za-z0-9_-]{8,}",
                "${1}_***",
            ),
            (r"\bAIza[0-9A-Za-z_-]{20,}", "AIza***"),
        ]
        .into_iter()
        .filter_map(|(p, r)| regex::Regex::new(p).ok().map(|re| (re, r)))
        .collect()
    });
    let mut out = s.to_string();
    if let Some(home) = dirs::home_dir() {
        let h = home.to_string_lossy().to_string();
        if h.len() > 1 {
            out = out.replace(&h, "~");
        }
    }
    for (re, rep) in res {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out.trim().to_string()
}
