use std::collections::HashSet;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Fingerprint {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    #[serde(default)]
    pub len: u64,
    pub mtime: i64,
    pub is_dir: bool,
}

#[cfg(unix)]
fn dev_ino(m: &Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

#[cfg(not(unix))]
fn dev_ino(_m: &Metadata) -> (u64, u64) {
    (0, 0)
}

fn mtime_ns(m: &Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

pub fn allocated(m: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        m.len()
    }
}

pub fn is_link(m: &Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

pub fn measure(path: &Path) -> (u64, u64) {
    let mut seen = HashSet::new();
    measure_with(path, &mut seen)
}

pub fn measure_with(path: &Path, seen: &mut HashSet<(u64, u64)>) -> (u64, u64) {
    let Ok(m) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if is_link(&m) {
        return (allocated(&m), 1);
    }
    if !m.is_dir() {
        return (count_once(&m, seen), 1);
    }
    let mut bytes = allocated(&m);
    let mut files = 0;
    for e in walkdir::WalkDir::new(path)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() && !is_link(&m) {
            bytes += allocated(&m);
        } else {
            bytes += count_once(&m, seen);
            files += 1;
        }
    }
    (bytes, files)
}

fn count_once(m: &Metadata, seen: &mut HashSet<(u64, u64)>) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() > 1 && !seen.insert((m.dev(), m.ino())) {
            return 0;
        }
    }
    let _ = seen;
    allocated(m)
}

pub fn fingerprint(path: &Path) -> Result<Fingerprint, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (dev, ino) = dev_ino(&m);
    let is_dir = m.is_dir() && !is_link(&m);
    let size = if is_dir {
        measure(path).0
    } else {
        allocated(&m)
    };
    Ok(Fingerprint {
        dev,
        ino,
        size,
        len: if is_dir { 0 } else { m.len() },
        mtime: mtime_ns(&m),
        is_dir,
    })
}

pub fn unchanged(path: &Path, fp: &Fingerprint) -> Result<(), String> {
    let m = std::fs::symlink_metadata(path).map_err(|_| "GONE: the path no longer exists")?;
    if is_link(&m) {
        return Err("CHANGED: the path is now a link".into());
    }
    let (dev, ino) = dev_ino(&m);
    if (fp.dev, fp.ino) != (0, 0) && (dev, ino) != (fp.dev, fp.ino) {
        return Err("CHANGED: a different file is at this path now".into());
    }
    let is_dir = m.is_dir();
    if is_dir != fp.is_dir {
        return Err("CHANGED: the path changed type".into());
    }
    if !is_dir && (mtime_ns(&m) != fp.mtime || allocated(&m) != fp.size || m.len() != fp.len) {
        return Err("CHANGED: the file was modified after the scan".into());
    }
    Ok(())
}

pub fn remove_tree(path: &Path, contents_only: bool) -> Result<u64, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if is_link(&m) {
        let bytes = allocated(&m);
        remove_link(path, &m)?;
        return Ok(bytes);
    }
    if !m.is_dir() {
        let bytes = allocated(&m);
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(bytes);
    }
    let mut freed = 0u64;
    let mut first_error = None;
    let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    for e in entries.flatten() {
        match remove_tree(&e.path(), false) {
            Ok(b) => freed += b,
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
    }
    if !contents_only {
        match std::fs::remove_dir(path) {
            Ok(()) => freed += allocated(&m),
            Err(e) => {
                first_error.get_or_insert(format!("{}: {e}", path.display()));
            }
        }
    }
    match first_error {
        Some(err) if freed == 0 => Err(err),
        _ => Ok(freed),
    }
}

fn remove_link(path: &Path, _m: &Metadata) -> Result<(), String> {
    #[cfg(windows)]
    {
        if _m.is_dir() {
            return std::fs::remove_dir(path).map_err(|e| format!("{}: {e}", path.display()));
        }
    }
    std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn expand_vars(pattern: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = pattern;
    if let Some(r) = rest.strip_prefix('~') {
        if r.is_empty() || r.starts_with('/') || r.starts_with('\\') {
            out.push_str(&lookup("HOME")?);
            rest = r;
        }
    }
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' {
            let braced = bytes.get(i + 1) == Some(&b'{');
            let start = if braced { i + 2 } else { i + 1 };
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > start {
                out.push_str(&lookup(&rest[start..j])?);
                i = if braced && bytes.get(j) == Some(&b'}') {
                    j + 1
                } else {
                    j
                };
                continue;
            }
        } else if c == b'%' {
            if let Some(end) = rest[i + 1..].find('%') {
                let name = &rest[i + 1..i + 1 + end];
                if !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'(' || b == b')')
                {
                    out.push_str(&lookup(name)?);
                    i += end + 2;
                    continue;
                }
            }
        }
        let ch = rest[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    Some(out)
}

pub fn env_lookup(name: &str) -> Option<String> {
    if name == "HOME" {
        return dirs::home_dir().map(|p| p.to_string_lossy().into_owned());
    }
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn glob(pattern: &str) -> Vec<PathBuf> {
    let norm = pattern.replace('\\', "/");
    let (prefix, rest) = split_literal_prefix(&norm);
    let base = PathBuf::from(if prefix.is_empty() { "/" } else { &prefix });
    if rest.is_empty() {
        return if std::fs::symlink_metadata(&base).is_ok() {
            vec![base]
        } else {
            Vec::new()
        };
    }
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    let mut out = Vec::new();
    walk_glob(&base, &parts, &mut out);
    out
}

fn split_literal_prefix(p: &str) -> (String, String) {
    let mut lit = Vec::new();
    let mut parts = p.split('/').peekable();
    let absolute = p.starts_with('/');
    let mut rest = Vec::new();
    let mut hit = false;
    for part in parts.by_ref() {
        if !hit && !part.contains(['*', '?', '[']) {
            lit.push(part);
        } else {
            hit = true;
            rest.push(part);
        }
    }
    let mut prefix = lit.join("/");
    if absolute && !prefix.starts_with('/') {
        prefix.insert(0, '/');
    }
    if prefix.len() == 2 && prefix.ends_with(':') {
        prefix.push('/');
    }
    (prefix, rest.join("/"))
}

fn walk_glob(dir: &Path, parts: &[&str], out: &mut Vec<PathBuf>) {
    let Some((first, tail)) = parts.split_first() else {
        out.push(dir.to_path_buf());
        return;
    };
    if *first == "**" {
        walk_glob(dir, tail, out);
        for e in read_dirs(dir) {
            walk_glob(&e, parts, out);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !wildcard(first, &name) {
            continue;
        }
        let path = e.path();
        if tail.is_empty() {
            out.push(path);
        } else if let Ok(m) = std::fs::symlink_metadata(&path) {
            if m.is_dir() && !is_link(&m) {
                walk_glob(&path, tail, out);
            }
        }
    }
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter(|e| {
                    std::fs::symlink_metadata(e.path())
                        .map(|m| m.is_dir() && !is_link(&m))
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

pub fn wildcard(pattern: &str, name: &str) -> bool {
    let fold = cfg!(any(windows, target_os = "macos"));
    let (p, n): (Vec<char>, Vec<char>) = if fold {
        (
            pattern.to_lowercase().chars().collect(),
            name.to_lowercase().chars().collect(),
        )
    } else {
        (pattern.chars().collect(), name.chars().collect())
    };
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn age_days(m: &Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_within_a_segment() {
        assert!(wildcard("*.log", "a.log"));
        assert!(wildcard("cache-?", "cache-1"));
        assert!(!wildcard("*.log", "a.txt"));
        assert!(wildcard("*", ""));
    }

    #[test]
    fn vars_expand_without_a_shell() {
        let look = |k: &str| match k {
            "HOME" => Some("/home/u".to_string()),
            "LOCALAPPDATA" => Some("C:/Users/u/AppData/Local".to_string()),
            _ => None,
        };
        assert_eq!(expand_vars("~/.cache", &look).unwrap(), "/home/u/.cache");
        assert_eq!(expand_vars("$HOME/x", &look).unwrap(), "/home/u/x");
        assert_eq!(
            expand_vars("%LOCALAPPDATA%/Temp", &look).unwrap(),
            "C:/Users/u/AppData/Local/Temp"
        );
        assert!(expand_vars("$NOPE/x", &look).is_none());
    }

    #[test]
    fn glob_finds_files_and_skips_linked_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/x.log"), "1").unwrap();
        std::fs::write(root.join("a/y.log"), "1").unwrap();
        let found = glob(&format!("{}/a/**/*.log", root.display()));
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn remove_tree_keeps_the_root_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/f"), vec![0u8; 5000]).unwrap();
        let freed = remove_tree(&root, true).unwrap();
        assert!(freed > 0);
        assert!(root.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_count_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::write(&a, vec![1u8; 64 * 1024]).unwrap();
        std::fs::hard_link(&a, dir.path().join("b")).unwrap();
        let (bytes, files) = measure(dir.path());
        assert_eq!(files, 2);
        let one = allocated(&std::fs::metadata(&a).unwrap());
        assert!(bytes < one * 2, "{bytes} vs {one}");
    }
}
