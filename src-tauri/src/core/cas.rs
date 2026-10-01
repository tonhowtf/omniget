use std::path::{Path, PathBuf};

pub fn object_path(store_root: &Path, sha256: &str) -> Option<PathBuf> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let lower = sha256.to_lowercase();
    let (prefix, rest) = lower.split_at(2);
    Some(store_root.join(prefix).join(rest))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked,
    Copied,
    AlreadyPresent,
}

pub fn ingest(store_root: &Path, source: &Path, sha256: &str) -> std::io::Result<PathBuf> {
    let object = object_path(store_root, sha256)
        .ok_or_else(|| std::io::Error::other(format!("invalid sha256: {sha256:?}")))?;

    if object.exists() {
        return Ok(object);
    }
    if let Some(parent) = object.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(source, &object) {
        Ok(()) => Ok(object),
        Err(_) => {
            std::fs::copy(source, &object)?;
            Ok(object)
        }
    }
}

pub fn materialize(object: &Path, dest: &Path) -> std::io::Result<LinkOutcome> {
    if dest.exists() && same_file(object, dest) {
        return Ok(LinkOutcome::AlreadyPresent);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    match std::fs::hard_link(object, dest) {
        Ok(()) => Ok(LinkOutcome::Linked),
        Err(_) => {
            std::fs::copy(object, dest)?;
            Ok(LinkOutcome::Copied)
        }
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

#[cfg(unix)]
pub fn link_count(object: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(object).ok().map(|m| m.nlink())
}
