//! Duplicados (estudo 38, o método do Czkawka em versão enxuta): agrupa por
//! tamanho, depois pelo hash dos primeiros 64 KB e só então pelo hash inteiro.
//! Roda em `spawn_blocking`; nunca apaga sozinho.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize)]
pub struct DupesOptions {
    pub dirs: Vec<String>,
    #[serde(default = "default_min")]
    pub min_size: u64,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_true")]
    pub skip_hidden: bool,
}

fn default_min() -> u64 {
    64 * 1024
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct DupeFile {
    pub path: String,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupeGroup {
    pub size: u64,
    pub hash: String,
    pub files: Vec<DupeFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DupesResult {
    pub scanned: u64,
    pub groups: Vec<DupeGroup>,
    pub wasted_bytes: u64,
}

fn walk(root: &Path, skip_hidden: bool, out: &mut Vec<(PathBuf, u64)>, seen: &mut u64) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if skip_hidden && name.starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                *seen += 1;
                out.push((p, meta.len()));
            }
        }
    }
}

fn hash_prefix(path: &Path, limit: usize) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut read_total = 0usize;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        read_total += n;
        if limit > 0 && read_total >= limit {
            break;
        }
    }
    Some(hex::encode(h.finalize()))
}

pub fn scan(opts: &DupesOptions, progress: &super::ProgressFn) -> DupesResult {
    let id = "dupes";
    let mut files = Vec::new();
    let mut scanned = 0u64;
    for d in &opts.dirs {
        walk(Path::new(d), opts.skip_hidden, &mut files, &mut scanned);
    }
    let exts: Vec<String> = opts.extensions.iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for (p, size) in files {
        if size < opts.min_size {
            continue;
        }
        if !exts.is_empty() {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if !exts.contains(&ext) {
                continue;
            }
        }
        by_size.entry(size).or_default().push(p);
    }
    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    let total = candidates.iter().map(|(_, v)| v.len() as u64).sum::<u64>();
    let mut done = 0u64;
    let mut groups = Vec::new();
    for (size, paths) in candidates {
        let mut by_prefix: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for p in paths {
            done += 1;
            if done.is_multiple_of(20) {
                super::report(progress, id, "hash", done, Some(total), None);
            }
            if let Some(h) = hash_prefix(&p, 64 * 1024) {
                by_prefix.entry(h).or_default().push(p);
            }
        }
        for (_, paths) in by_prefix.into_iter().filter(|(_, v)| v.len() > 1) {
            let mut by_full: HashMap<String, Vec<PathBuf>> = HashMap::new();
            for p in paths {
                if let Some(h) = hash_prefix(&p, 0) {
                    by_full.entry(h).or_default().push(p);
                }
            }
            for (hash, paths) in by_full.into_iter().filter(|(_, v)| v.len() > 1) {
                let files = paths
                    .into_iter()
                    .map(|p| DupeFile {
                        modified: std::fs::metadata(&p)
                            .and_then(|m| m.modified())
                            .ok()
                            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
                        path: p.to_string_lossy().to_string(),
                    })
                    .collect::<Vec<_>>();
                groups.push(DupeGroup { size, hash, files });
            }
        }
    }
    groups.sort_by_key(|b| std::cmp::Reverse(b.size * (b.files.len() as u64 - 1)));
    let wasted = groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
    super::report(progress, id, "done", total, Some(total), None);
    DupesResult { scanned, groups, wasted_bytes: wasted }
}

pub fn delete(paths: &[String]) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for p in paths {
        match std::fs::remove_file(p) {
            Ok(()) => ok.push(p.clone()),
            Err(_) => failed.push(p.clone()),
        }
    }
    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_identical_files() {
        let dir = std::env::temp_dir().join(format!("omniget-dupes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let big = vec![7u8; 70 * 1024];
        std::fs::write(dir.join("a.bin"), &big).unwrap();
        std::fs::write(dir.join("sub").join("b.bin"), &big).unwrap();
        std::fs::write(dir.join("c.bin"), vec![1u8; 70 * 1024]).unwrap();
        let opts = DupesOptions { dirs: vec![dir.to_string_lossy().to_string()], min_size: 1024, extensions: vec![], skip_hidden: true };
        let r = scan(&opts, &super::super::noop_progress());
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].files.len(), 2);
        assert_eq!(r.wasted_bytes, 70 * 1024);
        let _ = std::fs::remove_dir_all(dir);
    }
}
