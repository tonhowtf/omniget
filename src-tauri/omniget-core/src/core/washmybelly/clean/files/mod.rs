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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn blob(seed: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn write(p: &Path, data: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }

    #[test]
    fn three_copies_one_group_hardlinks_and_lookalikes_are_not_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("files");
        let data = blob(1, 300 * 1024);
        write(&root.join("docs/a.bin"), &data);
        write(&root.join("docs/sub/b.bin"), &data);
        write(&root.join("Downloads/c.bin"), &data);
        // Same size, same head and tail, different middle: only the full hash tells.
        let mut other = data.clone();
        other[150 * 1024] ^= 0xff;
        write(&root.join("docs/lookalike.bin"), &other);
        // Under min_kb: ignored.
        write(&root.join("docs/tiny1"), b"same");
        write(&root.join("docs/tiny2"), b"same");
        #[cfg(unix)]
        {
            // Another name of a.bin and a lone file with a hardlink: one file each.
            fs::hard_link(root.join("docs/a.bin"), root.join("docs/a-link.bin")).unwrap();
            let lone = blob(9, 100 * 1024);
            write(&root.join("x/lone.bin"), &lone);
            fs::hard_link(root.join("x/lone.bin"), root.join("x/lone-link.bin")).unwrap();
        }
        let ctx = Ctx::at(&tmp.path().join("db"));
        let out = find_duplicates(&ctx, &json!({ "roots": [root] })).unwrap();
        let groups = out["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{out:#}");
        let files = groups[0]["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        let kept: Vec<&str> = files
            .iter()
            .filter(|f| f["keep"] == true)
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(!kept[0].contains("Downloads"));
        assert!(out["plan_id"].as_str().unwrap().starts_with("plan_"));
        assert!(out["total_bytes"].as_u64().unwrap() >= 2 * 300 * 1024);
        assert_eq!(out["cache_hits"], 0);

        // Second run: every hash comes from the cache.
        let again = find_duplicates(&ctx, &json!({ "roots": [root] })).unwrap();
        assert!(again["cache_hits"].as_u64().unwrap() >= 4, "{again:#}");
        assert_eq!(again["groups"].as_array().unwrap().len(), 1);
        assert!(ctx.dir.join("hashcache.sqlite").exists());

        // min_kb 0: small files count too (hashed whole in the partial pass).
        let small = find_duplicates(&ctx, &json!({ "roots": [root], "min_kb": 0 })).unwrap();
        assert_eq!(small["groups"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn keep_prefers_outside_downloads_then_oldest() {
        let a = Path::new("/u/Downloads/x");
        let b = Path::new("/u/Documents/x");
        let c = Path::new("/u/Documents/deep/er/x");
        assert_eq!(dupes_pick_keep(&[(a, 1), (b, 5), (c, 3)]), 2);
        assert_eq!(dupes_pick_keep(&[(a, 1), (b, 5)]), 1);
    }

    fn dupes_pick_keep(p: &[(&Path, i64)]) -> usize {
        dupes::pick_keep(p)
    }

    #[test]
    fn nested_empty_folders_offer_the_outermost_and_list_broken_links() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        write(&root.join("a/b/.DS_Store"), b"x");
        write(&root.join("a/Thumbs.db"), b"x");
        write(&root.join("d/keep.txt"), b"x");
        fs::create_dir_all(root.join("d/e/f")).unwrap();
        fs::create_dir_all(root.join(".git/refs/tags")).unwrap();
        fs::create_dir_all(root.join("Game.app/Contents")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("nowhere"), root.join("d/broken")).unwrap();
            fs::create_dir_all(root.join("g")).unwrap();
            std::os::unix::fs::symlink(root.join("d"), root.join("g/ok")).unwrap();
        }
        let ctx = Ctx::at(&tmp.path().join("db"));
        let out = find_empty_folders(&ctx, &json!({ "roots": [root] })).unwrap();
        let ids: Vec<&str> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        let want = [root.join("a"), root.join("d/e")];
        assert_eq!(ids.len(), want.len(), "{ids:#?}");
        for w in want {
            assert!(
                ids.contains(&format!("empty_folders:{}", w.display()).as_str()),
                "{ids:#?}"
            );
        }
        assert!(out["plan_id"].as_str().unwrap().starts_with("plan_"));
        #[cfg(unix)]
        {
            let broken = out["broken_links"].as_array().unwrap();
            assert_eq!(broken.len(), 1);
            assert!(broken[0]["path"].as_str().unwrap().ends_with("d/broken"));
        }
        // An empty root is never offered itself.
        let lone = tmp.path().join("lone");
        fs::create_dir_all(&lone).unwrap();
        let out = find_empty_folders(&ctx, &json!({ "roots": [lone] })).unwrap();
        assert!(out["items"].as_array().unwrap().is_empty());
        assert!(out["plan_id"].is_null());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apfs_clone_swaps_in_place_and_refuses_different_bytes() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = blob(3, 512 * 1024);
        let keep = tmp.path().join("keep.bin");
        let copy = tmp.path().join("copy.bin");
        write(&keep, &data);
        write(&copy, &data);
        let ino_before = fs::metadata(&copy).unwrap().ino();
        let fp = crate::core::washmybelly::fsops::fingerprint(&copy).unwrap();
        let freed = apfs_clone_replace(&keep, &copy, &fp).unwrap();
        assert!(freed >= 512 * 1024);
        assert_eq!(fs::read(&copy).unwrap(), data);
        assert_eq!(fs::read(&keep).unwrap(), data);
        assert_ne!(fs::metadata(&copy).unwrap().ino(), ino_before);
        // No temporary left behind.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);

        // Different bytes of the same size: refused, the copy untouched.
        let mut other = data.clone();
        other[1000] ^= 1;
        let diff = tmp.path().join("diff.bin");
        write(&diff, &other);
        let fp = crate::core::washmybelly::fsops::fingerprint(&diff).unwrap();
        let err = apfs_clone_replace(&keep, &diff, &fp).unwrap_err();
        assert!(err.contains("CHANGED"), "{err}");
        assert_eq!(fs::read(&diff).unwrap(), other);
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 3);

        // Changed after the scan: refused by the fingerprint.
        let stale = crate::core::washmybelly::fsops::fingerprint(&copy).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&copy, &blob(4, 512 * 1024));
        assert!(apfs_clone_replace(&keep, &copy, &stale).is_err());
    }
}
