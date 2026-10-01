//! Exact duplicates, empty folders and broken links (F2), native in the
//! mould of czkawka (<https://github.com/qarmin/czkawka>, MIT): size, then a
//! partial hash, then a full xxh3 hash with a cache, hardlinks filtered.
//! On macOS a duplicate can also become an APFS clone (idea from MacSai,
//! <https://github.com/iliyami/MacSai>, BSD-3-Clause).

mod clone;
mod dupes;
mod empty;

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::protect::{self, Guard};
use super::super::store::Ctx;
use super::super::tools::blocking;

pub use clone::apfs_clone_replace;
pub use dupes::find_duplicates;
pub use empty::find_empty_folders;

pub async fn tool_duplicates(a: Value) -> Result<Value, String> {
    blocking(move || find_duplicates(&Ctx::app()?, &a)).await
}

pub async fn tool_empty_folders(a: Value) -> Result<Value, String> {
    blocking(move || find_empty_folders(&Ctx::app()?, &a)).await
}

/// Folder names that are a single thing to the person (an app, a photo
/// library, a project package): never looked inside.
const BUNDLES: &[&str] = &[
    "app",
    "framework",
    "bundle",
    "plugin",
    "appex",
    "kext",
    "xpc",
    "photoslibrary",
    "photolibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "fcpbundle",
    "logicx",
    "band",
    "xcarchive",
    "xcodeproj",
    "xcworkspace",
    "dSYM",
    "pkg",
    "mpkg",
    "rtfd",
    "sparsebundle",
];

/// Whether the walk must not enter `dir` (below its root): hidden folders,
/// bundles, `node_modules`, app data, and trees nothing may touch.
fn skip_dir(dir: &Path, root: &Path) -> bool {
    if dir == root {
        return false;
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    if name.starts_with('.') || name == "node_modules" {
        return true;
    }
    if let Some(ext) = dir.extension().and_then(|e| e.to_str()) {
        if BUNDLES.iter().any(|b| b.eq_ignore_ascii_case(ext)) {
            return true;
        }
    }
    if let Some(home) = dirs::home_dir() {
        for d in ["Library", "AppData", "Applications"] {
            if dir == home.join(d) {
                return true;
            }
        }
    }
    for d in [
        "/Applications",
        "/Library",
        "/System",
        "C:\\Windows",
        "C:\\Program Files",
        "C:\\Program Files (x86)",
        "C:\\ProgramData",
    ] {
        if dir == Path::new(d) {
            return true;
        }
    }
    // A child of a tree nothing may enter is refused by the guard.
    protect::check(&dir.join(".wmb-probe"), Guard::Picked, None).is_err()
}

fn roots(a: &Value) -> (Vec<PathBuf>, Vec<String>) {
    super::artifacts::roots_from(a, Vec::new)
}

#[cfg(unix)]
fn dev_ino(m: &std::fs::Metadata, _p: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (m.dev(), m.ino())
}

/// No inode on Windows through std: the path stands in for it (no hardlink
/// filter there, and the cache keys by path).
#[cfg(not(unix))]
fn dev_ino(_m: &std::fs::Metadata, p: &Path) -> (u64, u64) {
    (
        0,
        xxhash_rust::xxh3::xxh3_64(p.to_string_lossy().as_bytes()),
    )
}

fn mtime_ns(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
