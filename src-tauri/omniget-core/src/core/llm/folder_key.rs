use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FolderKey(String);

// WHY: NTFS and APFS ignore case by default, so folder keys are case-folded there.
const FOLD_CASE: bool = cfg!(windows) || cfg!(target_os = "macos");

impl FolderKey {
    pub fn of(path: &Path) -> FolderKey {
        let resolved = resolve(path);
        Self::normalize(&resolved.to_string_lossy(), cfg!(windows), FOLD_CASE)
    }

    pub fn normalize(raw: &str, windows: bool, fold_case: bool) -> FolderKey {
        let mut s = raw.to_string();
        if windows {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                s = format!(r"\\{rest}");
            } else if let Some(rest) = s.strip_prefix(r"\\?\") {
                s = rest.to_string();
            }
            s = s.replace('\\', "/");
        }
        let unc = windows && s.starts_with("//");
        let mut out = String::with_capacity(s.len() + 1);
        let mut slash = false;
        for c in s.chars() {
            if c == '/' {
                if slash {
                    continue;
                }
                slash = true;
            } else {
                slash = false;
            }
            out.push(c);
        }
        if unc {
            out.insert(0, '/');
        }
        let drive_root = |o: &str| windows && o.len() == 3 && o.as_bytes()[1] == b':';
        while out.len() > 1 && out.ends_with('/') && !drive_root(&out) {
            out.pop();
        }
        if windows && out.len() == 2 && out.as_bytes()[1] == b':' {
            out.push('/');
        }
        if fold_case {
            out = out.to_lowercase();
        }
        FolderKey(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn overlaps(&self, other: &FolderKey) -> bool {
        contains(&self.0, &other.0) || contains(&other.0, &self.0)
    }
}

impl std::fmt::Display for FolderKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn contains(outer: &str, inner: &str) -> bool {
    inner == outer
        || (inner.starts_with(outer)
            && (outer.ends_with('/') || inner.as_bytes().get(outer.len()) == Some(&b'/')))
}

fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut lexical = PathBuf::new();
    for comp in absolute.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                lexical.pop();
            }
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut existing = lexical.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return lexical,
        }
    }
}
