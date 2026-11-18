use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::agentkit::plan::unified_diff;

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

pub fn snapshot(root: &Path) -> Result<Manifest, String> {
    let mut m = Manifest::new();
    for p in files(root)? {
        if let (Some(r), Ok(b)) = (rel(root, &p), std::fs::read(&p)) {
            m.insert(r, sha(&b));
        }
    }
    Ok(m)
}

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
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub binary: bool,
    pub conflict: bool,
}

fn text(b: &[u8]) -> Option<&str> {
    if b.contains(&0) {
        return None;
    }
    std::str::from_utf8(b).ok()
}

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
