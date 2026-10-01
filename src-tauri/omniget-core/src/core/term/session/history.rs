use std::io::Write;
use std::path::{Path, PathBuf};

pub const DEFAULT_CAP: usize = 2 * 1024 * 1024;
pub const MIN_CAP: usize = 64 * 1024;

pub struct History {
    cap: usize,
    buf: Vec<u8>,
    path: Option<PathBuf>,
    unwritten: Vec<u8>,
    disk_len: usize,
    rewrite: bool,
}

pub enum DiskJob {
    Append(PathBuf, Vec<u8>),
    Rewrite(PathBuf, Vec<u8>),
}

impl History {
    pub fn new(cap: usize, path: Option<PathBuf>) -> Self {
        let cap = cap.max(MIN_CAP);
        Self {
            cap,
            buf: Vec::new(),
            path,
            unwritten: Vec::new(),
            disk_len: 0,
            rewrite: true,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
        if self.path.is_some() && !self.rewrite {
            self.unwritten.extend_from_slice(data);
        }
        if self.buf.len() > self.cap + self.cap / 4 {
            let mut cut = self.buf.len() - self.cap;
            let window = &self.buf[cut..(cut + 8192).min(self.buf.len())];
            if let Some(nl) = window.iter().position(|&c| c == b'\n') {
                cut += nl + 1;
            }
            self.buf.drain(..cut);
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.unwritten.clear();
        self.rewrite = true;
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn dirty(&self) -> bool {
        self.path.is_some() && (self.rewrite || !self.unwritten.is_empty())
    }

    pub fn take_job(&mut self) -> Option<DiskJob> {
        let path = self.path.clone()?;
        if !self.rewrite && self.disk_len + self.unwritten.len() > self.cap + self.cap / 2 {
            self.rewrite = true;
        }
        if self.rewrite {
            self.rewrite = false;
            self.unwritten.clear();
            self.disk_len = self.buf.len();
            return Some(DiskJob::Rewrite(path, self.buf.clone()));
        }
        if self.unwritten.is_empty() {
            return None;
        }
        let data = std::mem::take(&mut self.unwritten);
        self.disk_len += data.len();
        Some(DiskJob::Append(path, data))
    }
}

impl DiskJob {
    pub fn run(self) {
        let res = match &self {
            DiskJob::Append(path, data) => std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(data)),
            DiskJob::Rewrite(path, data) => rewrite(path, data),
        };
        if let Err(e) = res {
            tracing::debug!("[pty] history write failed: {e}");
        }
    }
}

fn rewrite(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("log.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn dir() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("terminal"))
}

fn session_path(id: &str) -> Option<PathBuf> {
    path_for(id).map(|p| p.with_extension("session"))
}

pub fn session_id_of(payload: &serde_json::Value) -> Option<String> {
    let pick = |v: &serde_json::Value| {
        ["session_id", "sessionId", "thread-id", "thread_id"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_str()).map(str::to_string))
    };
    pick(payload)
        .or_else(|| payload.get("codex").and_then(pick))
        .map(|s| s.trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

pub fn save_agent_session(id: &str, session: &str) {
    let Some(p) = session_path(id) else { return };
    if std::fs::read_to_string(&p).ok().as_deref() == Some(session) {
        return;
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&p, session);
}

pub fn agent_session(id: &str) -> Option<String> {
    let s = std::fs::read_to_string(session_path(id)?).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn forget_agent_session(id: &str) {
    if let Some(p) = session_path(id) {
        let _ = std::fs::remove_file(p);
    }
}

pub fn path_for(id: &str) -> Option<PathBuf> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || id.starts_with('.')
    {
        return None;
    }
    dir().map(|d| d.join(format!("{id}.log")))
}

pub fn read_tail(id: &str, cap: usize) -> Option<Vec<u8>> {
    let data = std::fs::read(path_for(id)?).ok()?;
    if data.len() <= cap {
        return Some(data);
    }
    let mut cut = data.len() - cap;
    if let Some(nl) = data[cut..].iter().take(8192).position(|&c| c == b'\n') {
        cut += nl + 1;
    }
    Some(data[cut..].to_vec())
}
