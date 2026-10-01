use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::install::SIDECAR;
use super::{SkillError, ERR_SKILL_TOO_BIG};

const MAX_DEPTH: usize = 16;
const MAX_ENTRIES: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    rel: String,
    kind: u8,
    len: u64,
    mtime_ns: u128,
    abs: PathBuf,
}

static CACHE: Mutex<Option<HashMap<PathBuf, (u64, String)>>> = Mutex::new(None);

pub fn dir_hash(dir: &Path) -> Result<String, SkillError> {
    let entries = walk(dir)?;
    let fingerprint = fingerprint(&entries);
    if let Some((fp, hash)) = CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(dir).cloned())
    {
        if fp == fingerprint {
            return Ok(hash);
        }
    }
    let hash = hash_entries(&entries)?;
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(dir.to_path_buf(), (fingerprint, hash.clone()));
    Ok(hash)
}

pub fn dir_hash_uncached(dir: &Path) -> Result<String, SkillError> {
    hash_entries(&walk(dir)?)
}

pub fn short(hash: &str, n: usize) -> &str {
    &hash[..hash.len().min(n)]
}

fn walk(dir: &Path) -> Result<Vec<Entry>, SkillError> {
    let mut out = Vec::new();
    walk_into(dir, dir, 0, &mut out)?;
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn walk_into(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<Entry>,
) -> Result<(), SkillError> {
    if depth > MAX_DEPTH {
        return Err(SkillError::new(
            ERR_SKILL_TOO_BIG,
            format!("{} is nested too deep", dir.display()),
        ));
    }
    let read = std::fs::read_dir(dir)
        .map_err(|e| SkillError::io(&format!("reading {}", dir.display()), &e))?;
    let mut any = false;
    for entry in read {
        let entry = entry.map_err(|e| SkillError::io(&format!("reading {}", dir.display()), &e))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if depth == 0 && rel == SIDECAR {
            continue;
        }
        any = true;
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| SkillError::io(&format!("reading {}", path.display()), &e))?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        if meta.file_type().is_symlink() {
            out.push(Entry {
                rel,
                kind: b'L',
                len: 0,
                mtime_ns,
                abs: path,
            });
        } else if meta.is_dir() {
            walk_into(root, &path, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(Entry {
                rel,
                kind: b'F',
                len: meta.len(),
                mtime_ns,
                abs: path,
            });
        }
        if out.len() > MAX_ENTRIES {
            return Err(SkillError::new(
                ERR_SKILL_TOO_BIG,
                format!("{} has too many files to hash", root.display()),
            ));
        }
    }
    if !any && depth > 0 {
        let rel = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(Entry {
            rel,
            kind: b'D',
            len: 0,
            mtime_ns: 0,
            abs: dir.to_path_buf(),
        });
    }
    Ok(())
}

fn fingerprint(entries: &[Entry]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for e in entries {
        e.rel.hash(&mut h);
        e.kind.hash(&mut h);
        e.len.hash(&mut h);
        e.mtime_ns.hash(&mut h);
    }
    entries.len().hash(&mut h);
    h.finish()
}

fn hash_entries(entries: &[Entry]) -> Result<String, SkillError> {
    let mut hasher = Sha256::new();
    for e in entries {
        hasher.update([e.kind, 0]);
        hasher.update(e.rel.as_bytes());
        hasher.update([0]);
        match e.kind {
            b'F' => {
                let bytes = std::fs::read(&e.abs)
                    .map_err(|err| SkillError::io(&format!("reading {}", e.abs.display()), &err))?;
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            b'L' => {
                let target = std::fs::read_link(&e.abs).unwrap_or_default();
                hasher.update(target.to_string_lossy().as_bytes());
            }
            _ => {}
        }
        hasher.update([0xff]);
    }
    Ok(hex::encode(hasher.finalize()))
}
