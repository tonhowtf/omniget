use std::path::{Path, PathBuf};

use super::store::Ctx;

pub fn move_in(ctx: &Ctx, receipt_id: &str, index: usize, path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{}: no file name", path.display()))?;
    let slot = ctx
        .quarantine_dir()
        .join(receipt_id)
        .join(index.to_string());
    std::fs::create_dir_all(&slot).map_err(|e| e.to_string())?;
    let dest = slot.join(name);
    std::fs::rename(path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(dest)
}

pub fn restore(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(format!("EXISTS: {} is in use again", to.display()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(from, to).map_err(|e| format!("{}: {e}", from.display()))
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let dev = |p: &Path| {
            let mut cur = Some(p);
            while let Some(c) = cur {
                if let Ok(m) = std::fs::metadata(c) {
                    return Some(m.dev());
                }
                cur = c.parent();
            }
            None
        };
        dev(a).is_some() && dev(a) == dev(b)
    }
    #[cfg(not(unix))]
    {
        let drive = |p: &Path| {
            p.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        };
        drive(a) == drive(b)
    }
}

pub fn purge_expired(ctx: &Ctx, days: u32) -> u64 {
    let Ok(entries) = std::fs::read_dir(ctx.quarantine_dir()) else {
        return 0;
    };
    let max_age = std::time::Duration::from_secs(u64::from(days) * 86_400);
    let mut freed = 0;
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old {
            freed += super::fsops::remove_tree(&e.path(), false).unwrap_or(0);
        }
    }
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_goes_in_and_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("wmb"));
        let f = dir.path().join("keep.txt");
        std::fs::write(&f, "x").unwrap();
        let q = move_in(&ctx, "r1", 0, &f).unwrap();
        assert!(!f.exists() && q.exists());
        restore(&q, &f).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x");
    }

    #[test]
    fn restore_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::at(&dir.path().join("wmb"));
        let f = dir.path().join("a");
        std::fs::write(&f, "old").unwrap();
        let q = move_in(&ctx, "r1", 0, &f).unwrap();
        std::fs::write(&f, "new").unwrap();
        assert!(restore(&q, &f).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
    }
}
