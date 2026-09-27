//! Bounded transcript of one terminal: the last `cap` bytes in memory, mirrored
//! to `<app_data>/terminal/<id>.log` with coalesced writes.
//!
//! The file is append-only between rewrites. When appending would push it past
//! one and a half caps, it is rewritten from the in-memory tail (temp file and
//! rename), so the disk never holds much more than the cap and a crash never
//! leaves a half-written log.

use std::io::Write;
use std::path::{Path, PathBuf};

pub const DEFAULT_CAP: usize = 2 * 1024 * 1024;
/// Smallest cap accepted: below this a single screenful does not fit.
pub const MIN_CAP: usize = 64 * 1024;

pub struct History {
    cap: usize,
    buf: Vec<u8>,
    path: Option<PathBuf>,
    /// Bytes appended since the last flush.
    unwritten: Vec<u8>,
    /// Size of the file on disk as far as this process knows.
    disk_len: usize,
    /// The in-memory tail was trimmed since the last flush in a way that the
    /// file must be rewritten to match.
    rewrite: bool,
}

/// Work to do on disk, taken under the session lock and run outside it.
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
        // Trim with slack so trimming is not a memmove per read.
        if self.buf.len() > self.cap + self.cap / 4 {
            let mut cut = self.buf.len() - self.cap;
            // Start on a line boundary so replay never begins inside an escape
            // sequence or a UTF-8 character.
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

/// Directory holding the transcripts.
pub fn dir() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join("terminal"))
}

/// Transcript path for a session id, if the id is safe as a file name.
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

/// Reads the last `cap` bytes of a transcript left on disk (a session from a
/// previous run of the app).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_to_cap_on_a_line_boundary() {
        let mut h = History::new(MIN_CAP, None);
        let line = b"0123456789abcdef0123456789abcdef0123456789abcdef012345678901234\n";
        for _ in 0..5000 {
            h.push(line);
        }
        let b = h.bytes();
        assert!(b.len() <= MIN_CAP + MIN_CAP / 4);
        assert_eq!(b.len() % line.len(), 0);
        assert!(b.starts_with(b"0123"));
    }

    #[test]
    fn disk_jobs_append_then_rewrite() {
        let mut h = History::new(MIN_CAP, Some(PathBuf::from("x.log")));
        h.push(b"abc");
        assert!(matches!(h.take_job(), Some(DiskJob::Rewrite(_, d)) if d == b"abc"));
        h.push(b"def");
        assert!(matches!(h.take_job(), Some(DiskJob::Append(_, d)) if d == b"def"));
        assert!(h.take_job().is_none());
        h.push(&vec![b'x'; MIN_CAP * 2]);
        assert!(matches!(h.take_job(), Some(DiskJob::Rewrite(_, _))));
    }
}
