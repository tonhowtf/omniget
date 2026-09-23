//! Copy of a project for a sandbox run, and the diff back: the agent works on a
//! copy, the user reviews what changed, and only the files they pick are
//! written back (refused when the original changed in the meantime).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::agentkit::plan::unified_diff;

/// Never copied into the sandbox (dependencies and build output are rebuilt
/// inside; macOS binaries would not run in a Linux container anyway).
pub const SKIP: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    "dist",
    ".turbo",
    ".gradle",
    ".DS_Store",
];

pub const MAX_FILES: usize = 20_000;
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
const DIFF_MAX: usize = 256 * 1024;

/// `relative path (with /) → sha256` of the base the agent started from.
pub type Manifest = BTreeMap<String, String>;

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn rel(root: &Path, p: &Path) -> Option<String> {
    let r = p.strip_prefix(root).ok()?;
    let parts: Vec<String> = r
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    Some(parts.join("/"))
}

fn files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut total = 0u64;
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            e.depth() == 0 || !SKIP.contains(&n.as_ref())
        });
    for e in walker.flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        total += e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(e.into_path());
        if out.len() > MAX_FILES || total > MAX_BYTES {
            return Err(format!(
                "{}: the project is too big to copy into a sandbox ({} files / {} MB so far); use bind mode",
                super::ERR_SANDBOX,
                out.len(),
                total / (1024 * 1024)
            ));
        }
    }
    Ok(out)
}

/// Hashes of every file of `root` (same skip list).
pub fn snapshot(root: &Path) -> Result<Manifest, String> {
    let mut m = Manifest::new();
    for p in files(root)? {
        if let (Some(r), Ok(b)) = (rel(root, &p), std::fs::read(&p)) {
            m.insert(r, sha(&b));
        }
    }
    Ok(m)
}

/// Copies `src` into `dst` (created) and returns the base manifest.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<Manifest, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
    let mut m = Manifest::new();
    for p in files(src)? {
        let Some(r) = rel(src, &p) else { continue };
        let target = dst.join(&r);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
        }
        let bytes = std::fs::read(&p)
            .map_err(|e| format!("{}: {}: {e}", super::ERR_SANDBOX, p.display()))?;
        std::fs::write(&target, &bytes).map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&p) {
                let _ = std::fs::set_permissions(
                    &target,
                    std::fs::Permissions::from_mode(meta.permissions().mode()),
                );
            }
        }
        m.insert(r, sha(&bytes));
    }
    Ok(m)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    /// `added | modified | deleted`
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub binary: bool,
    /// The original file changed since the copy: applying would lose that edit.
    pub conflict: bool,
}

fn text(b: &[u8]) -> Option<&str> {
    if b.contains(&0) {
        return None;
    }
    std::str::from_utf8(b).ok()
}

/// What the agent changed in `work` compared with the `base` it started from.
pub fn changes(original: &Path, work: &Path, base: &Manifest) -> Result<Vec<Change>, String> {
    let now = snapshot(work)?;
    let mut out = Vec::new();
    let mut budget = DIFF_MAX;
    let orig_hash = |r: &str| std::fs::read(original.join(r)).ok().map(|b| sha(&b));
    for (r, h) in &now {
        let status = match base.get(r) {
            None => "added",
            Some(b) if b != h => "modified",
            _ => continue,
        };
        let after = std::fs::read(work.join(r)).unwrap_or_default();
        let before = if status == "modified" {
            std::fs::read(original.join(r)).unwrap_or_default()
        } else {
            vec![]
        };
        let conflict = match status {
            "added" => original.join(r).exists(),
            _ => orig_hash(r).as_ref() != base.get(r),
        };
        let (diff, binary) = match (text(&before), text(&after)) {
            (Some(a), Some(b)) => {
                let d = unified_diff(a, b, r, status == "added");
                if d.len() <= budget {
                    budget -= d.len();
                    (Some(d), false)
                } else {
                    (None, false)
                }
            }
            _ => (None, true),
        };
        out.push(Change {
            path: r.clone(),
            status: status.into(),
            diff,
            binary,
            conflict,
        });
    }
    for r in base.keys() {
        if !now.contains_key(r) {
            let before = std::fs::read(original.join(r)).unwrap_or_default();
            let diff = text(&before).map(|a| unified_diff(a, "", r, false));
            out.push(Change {
                path: r.clone(),
                status: "deleted".into(),
                diff,
                binary: text(&before).is_none(),
                conflict: orig_hash(r).as_ref() != base.get(r),
            });
        }
    }
    Ok(out)
}

/// Writes the chosen changes back into `original`. Conflicting paths are
/// skipped and returned in the second list.
pub fn apply(
    original: &Path,
    work: &Path,
    base: &Manifest,
    paths: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let all = changes(original, work, base)?;
    let mut done = Vec::new();
    let mut skipped = Vec::new();
    for c in all
        .into_iter()
        .filter(|c| paths.is_empty() || paths.contains(&c.path))
    {
        if c.conflict {
            skipped.push(c.path);
            continue;
        }
        if c.path.split('/').any(|seg| seg == ".." || seg.is_empty()) {
            skipped.push(c.path);
            continue;
        }
        let dest = original.join(&c.path);
        match c.status.as_str() {
            "deleted" => {
                let _ = std::fs::remove_file(&dest);
            }
            _ => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
                }
                let bytes = std::fs::read(work.join(&c.path))
                    .map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
                let tmp = dest.with_extension("omniget-tmp");
                std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
                std::fs::rename(&tmp, &dest).map_err(|e| format!("{}: {e}", super::ERR_SANDBOX))?;
            }
        }
        done.push(c.path);
    }
    Ok((done, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_diff_apply_round_trip() {
        let root = std::env::temp_dir().join(format!("sbtree-{}", uuid::Uuid::new_v4().simple()));
        let orig = root.join("orig");
        let work = root.join("work");
        std::fs::create_dir_all(orig.join("src")).unwrap();
        std::fs::create_dir_all(orig.join("node_modules/x")).unwrap();
        std::fs::write(orig.join("src/a.js"), "let a = 1;\n").unwrap();
        std::fs::write(orig.join("src/b.js"), "b\n").unwrap();
        std::fs::write(orig.join("c.txt"), "c\n").unwrap();
        std::fs::write(orig.join("node_modules/x/i.js"), "x").unwrap();
        let base = copy_tree(&orig, &work).unwrap();
        assert!(!work.join("node_modules").exists());
        std::fs::write(work.join("src/a.js"), "let a = 2;\n").unwrap();
        std::fs::write(work.join("new.md"), "hi\n").unwrap();
        std::fs::remove_file(work.join("c.txt")).unwrap();
        std::fs::write(work.join("src/b.js"), "b2\n").unwrap();
        std::fs::write(orig.join("src/b.js"), "user edit\n").unwrap();
        let ch = changes(&orig, &work, &base).unwrap();
        let by = |p: &str| ch.iter().find(|c| c.path == p).unwrap().clone();
        assert_eq!(by("src/a.js").status, "modified");
        assert!(by("src/a.js").diff.unwrap().contains("+let a = 2;"));
        assert_eq!(by("new.md").status, "added");
        assert_eq!(by("c.txt").status, "deleted");
        assert!(by("src/b.js").conflict);
        let (done, skipped) = apply(&orig, &work, &base, &[]).unwrap();
        assert_eq!(skipped, vec!["src/b.js".to_string()]);
        assert_eq!(done.len(), 3);
        assert_eq!(
            std::fs::read_to_string(orig.join("src/a.js")).unwrap(),
            "let a = 2;\n"
        );
        assert!(!orig.join("c.txt").exists());
        assert_eq!(
            std::fs::read_to_string(orig.join("src/b.js")).unwrap(),
            "user edit\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
