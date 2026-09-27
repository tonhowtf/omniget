//! The largest files under some folders, offered as a plan that moves the
//! chosen ones to the system Trash. Sizes are allocated blocks, so a sparse
//! VM disk shows what it really takes.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::super::fsops;
use super::super::plan::{Op, PlanItem};
use super::super::protect::{self, Guard};
use super::super::{Destination, Risk};

/// Bundles and libraries whose insides are one thing to the person: an app,
/// a photo library. A file inside is never offered on its own.
const PACKAGE_EXT: &[&str] = &[
    "app",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "appex",
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "fcpbundle",
    "logicx",
    "band",
    "imovielibrary",
    "lrlibrary",
    "lrdata",
    "xcarchive",
    "sparsebundle",
    "vmwarevm",
    "pvm",
    "utm",
    "pkg",
    "mpkg",
    "xcodeproj",
    "xcworkspace",
    "playground",
    "rtfd",
    "pages",
    "numbers",
    "key",
    "scriptd",
];

/// Folders of already-trashed files.
const TRASH_DIRS: &[&str] = &[".Trash", ".Trashes", "$Recycle.Bin", "$RECYCLE.BIN"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub bytes: u64,
    pub path: PathBuf,
    pub root: PathBuf,
}

fn ext_lower(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn skip_dir(p: &Path) -> bool {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    if TRASH_DIRS.iter().any(|t| name == *t) {
        return true;
    }
    // ~/.local/share/Trash
    if name == "Trash" && p.parent().is_some_and(|q| q.ends_with(".local/share")) {
        return true;
    }
    PACKAGE_EXT.contains(&ext_lower(p).as_str())
}

/// What kind of file it is, for the list's groups.
pub fn group_of(p: &Path) -> &'static str {
    match ext_lower(p).as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "ts"
        | "m2ts" | "mts" | "3gp" | "vob" | "prores" | "braw" | "r3d" => "video",
        "dmg" | "iso" | "img" | "vmdk" | "vdi" | "vhd" | "vhdx" | "qcow2" | "sparseimage"
        | "toast" | "wim" | "esd" | "ipsw" | "hdd" | "raw" => "disk_image",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "xip"
        | "deb" | "rpm" | "apk" | "aab" | "jar" | "cab" | "msi" | "exe" | "appimage" => "archive",
        _ => "other",
    }
}

#[cfg(unix)]
fn link_key(m: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (m.nlink() > 1).then(|| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn link_key(_m: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// The `limit` largest files of at least `min_bytes` under `roots`, each one
/// already cleared by the protection list.
pub fn find(roots: &[PathBuf], min_bytes: u64, limit: usize) -> (Vec<Found>, u64) {
    let mut heap: BinaryHeap<Reverse<Found>> = BinaryHeap::new();
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut scanned = 0u64;
    let mut last = Instant::now();
    for root in roots {
        let walker = walkdir::WalkDir::new(root)
            .follow_links(false)
            .same_file_system(true)
            .into_iter()
            .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_dir() && skip_dir(e.path())));
        for e in walker.flatten() {
            scanned += 1;
            if scanned.is_multiple_of(4096) && last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                super::super::progress(
                    "large_files",
                    "scan",
                    scanned,
                    0,
                    e.path().display().to_string(),
                );
            }
            if !e.file_type().is_file() {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let bytes = fsops::allocated(&m);
            if bytes < min_bytes {
                continue;
            }
            let full = heap.len() >= limit;
            if full && heap.peek().is_some_and(|Reverse(min)| bytes <= min.bytes) {
                continue;
            }
            if let Some(k) = link_key(&m) {
                if !seen_links.insert(k) {
                    continue;
                }
            }
            let path = e.path().to_path_buf();
            if !seen_paths.insert(path.clone()) {
                continue; // overlapping roots
            }
            if protect::check(&path, Guard::Picked, Some(root)).is_err() {
                continue;
            }
            heap.push(Reverse(Found {
                bytes,
                path,
                root: root.clone(),
            }));
            if heap.len() > limit {
                heap.pop();
            }
        }
    }
    let mut v: Vec<Found> = heap.into_iter().map(|Reverse(f)| f).collect();
    v.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    super::super::progress(
        "large_files",
        "done",
        scanned,
        scanned,
        format!("{} files", v.len()),
    );
    (v, scanned)
}

pub fn item_id(p: &Path) -> String {
    format!("large_files:{}", p.display())
}

/// A plan item for each file (files that vanished since are dropped).
pub fn items(found: &[Found]) -> Vec<(PlanItem, &Found)> {
    found
        .iter()
        .filter_map(|f| {
            let fp = fsops::fingerprint(&f.path).ok()?;
            let label = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string());
            let item = PlanItem {
                id: item_id(&f.path),
                label,
                detail: f.path.parent().map(|p| p.display().to_string()),
                group: group_of(&f.path).to_string(),
                bytes: f.bytes,
                risk: Risk::Reversible,
                admin: false,
                op: Op::RemovePath {
                    path: f.path.clone(),
                    destination: Destination::Trash,
                    fp,
                    root: Some(f.root.clone()),
                    guard: Guard::Picked,
                    contents_only: false,
                },
            };
            Some((item, f))
        })
        .collect()
}

pub fn item_json(it: &PlanItem, f: &Found) -> Value {
    let mtime = std::fs::symlink_metadata(&f.path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    json!({
        "id": it.id,
        "label": it.label,
        "detail": it.detail,
        "path": f.path.display().to_string(),
        "group": it.group,
        "bytes": it.bytes,
        "risk": "reversible",
        "modified": mtime,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::plan::{self, Source};
    use super::super::super::store::Ctx;
    use super::*;

    fn write(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![1u8; n]).unwrap();
    }

    #[test]
    fn largest_first_skipping_packages_and_trash() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_path_buf();
        write(&r.join("movies/a.mp4"), 3_000_000);
        write(&r.join("iso/b.iso"), 2_000_000);
        write(&r.join("c.zip"), 1_500_000);
        write(&r.join("small.txt"), 10_000);
        write(&r.join("Tool.app/Contents/big.bin"), 5_000_000);
        write(&r.join(".Trash/old.mov"), 5_000_000);
        let (found, scanned) = find(std::slice::from_ref(&r), 1_000_000, 2);
        assert!(scanned >= 4);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.mp4", "b.iso"]);
        assert_eq!(group_of(&found[0].path), "video");
        assert_eq!(group_of(&found[1].path), "disk_image");
        assert_eq!(group_of(Path::new("/x/c.ZIP")), "archive");
        assert_eq!(group_of(Path::new("/x/c.psd")), "other");
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_once_and_links_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_path_buf();
        write(&r.join("a.bin"), 2_000_000);
        std::fs::hard_link(r.join("a.bin"), r.join("b.bin")).unwrap();
        let out = tempfile::tempdir().unwrap();
        write(&out.path().join("huge.bin"), 4_000_000);
        std::os::unix::fs::symlink(out.path(), r.join("elsewhere")).unwrap();
        let (found, _) = find(std::slice::from_ref(&r), 1_000_000, 10);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn items_become_a_trash_plan() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().join("files");
        write(&r.join("big.mkv"), 2_000_000);
        let (found, _) = find(std::slice::from_ref(&r), 1_000_000, 10);
        let pairs = items(&found);
        assert_eq!(pairs.len(), 1);
        let (it, f) = &pairs[0];
        assert_eq!(it.id, item_id(&f.path));
        assert_eq!(it.risk, Risk::Reversible);
        match &it.op {
            Op::RemovePath {
                destination,
                guard,
                root,
                contents_only,
                ..
            } => {
                assert_eq!(*destination, Destination::Trash);
                assert_eq!(*guard, Guard::Picked);
                assert_eq!(root.as_deref(), Some(r.as_path()));
                assert!(!contents_only);
            }
            _ => panic!("wrong op"),
        }
        let j = item_json(it, f);
        assert_eq!(j["group"], "video");
        let ctx = Ctx::at(&d.path().join("state"));
        let p = plan::offer(
            &ctx,
            "large_files",
            pairs.into_iter().map(|(i, _)| i).collect(),
            Source::Agent,
        )
        .unwrap();
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.max_risk, Risk::Reversible);
    }
}
