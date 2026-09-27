//! Disk usage tree. The idea comes from dust (bootandy/dust, Apache-2.0):
//! a parallel walk, allocated blocks, a hardlink counted once by
//! (device, inode), no link followed and no other volume entered. The code
//! is our own.
//!
//! The whole tree stays in memory for ten minutes per root, so a drill-down
//! into any folder under it answers without a new scan. To keep that tree
//! small, each folder keeps its subfolders and only its largest files; the
//! other files fold into a per-folder remainder.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::super::fsops;

/// Files kept by name per folder; the rest fold into `rest_*`.
const KEEP_FILES: usize = 24;
/// Folder entries with their metadata.
type Entries = Vec<(OsString, Metadata)>;
/// Levels listed on the calling thread before the rest splits into jobs.
const SPLIT_DEPTH: usize = 2;
/// Children shown per folder in the output, whatever the fraction.
const MAX_CHILDREN: usize = 120;
/// A scan answers drill-downs for this long.
const CACHE_TTL: Duration = Duration::from_secs(600);
/// Scans kept at once (each can hold a whole home).
const CACHE_MAX: usize = 3;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
    pub is_dir: bool,
    pub children: Vec<Node>,
    /// Small files folded while scanning.
    pub rest_bytes: u64,
    pub rest_files: u64,
}

impl Node {
    fn dir(name: String, own: u64) -> Node {
        Node {
            name,
            bytes: own,
            files: 0,
            is_dir: true,
            children: Vec::new(),
            rest_bytes: 0,
            rest_files: 0,
        }
    }

    fn file(name: String, bytes: u64) -> Node {
        Node {
            name,
            bytes,
            files: 1,
            is_dir: false,
            children: Vec::new(),
            rest_bytes: 0,
            rest_files: 0,
        }
    }

    fn add(&mut self, child: Node) {
        self.bytes += child.bytes;
        self.files += child.files;
        self.children.push(child);
    }

    /// Keep the largest files by name; fold the others.
    fn fold_files(&mut self, mut files: Vec<Node>) {
        files.sort_by_key(|f| std::cmp::Reverse(f.bytes));
        for (i, f) in files.into_iter().enumerate() {
            if i < KEEP_FILES {
                self.children.push(f);
            } else {
                self.rest_bytes += f.bytes;
                self.rest_files += 1;
            }
        }
    }

    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.is_dir && c.name == name)
    }
}

/// A finished scan.
#[derive(Debug)]
pub struct Scan {
    pub root: PathBuf,
    pub tree: Node,
    pub entries: u64,
    pub denied: u64,
    pub errors: u64,
    pub scanned_at: i64,
    at: Instant,
}

struct Walk {
    #[cfg_attr(not(unix), allow(dead_code))]
    dev: u64,
    #[cfg_attr(not(unix), allow(dead_code))]
    seen: Mutex<HashSet<(u64, u64)>>,
    entries: AtomicU64,
    denied: AtomicU64,
    errors: AtomicU64,
    last_tick: Mutex<Instant>,
}

#[cfg(unix)]
fn dev_of(m: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.dev()
}

#[cfg(not(unix))]
fn dev_of(_m: &Metadata) -> u64 {
    0
}

impl Walk {
    fn new(root_meta: &Metadata) -> Walk {
        Walk {
            dev: dev_of(root_meta),
            seen: Mutex::new(HashSet::new()),
            entries: AtomicU64::new(0),
            denied: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_tick: Mutex::new(Instant::now()),
        }
    }

    /// Same volume as the root. Windows has no stable device id in std;
    /// there mount points are reparse points, which are never entered.
    fn same_volume(&self, m: &Metadata) -> bool {
        #[cfg(unix)]
        {
            dev_of(m) == self.dev
        }
        #[cfg(not(unix))]
        {
            let _ = m;
            true
        }
    }

    /// Allocated bytes of a file, or `None` for a hardlink already counted.
    fn file_bytes(&self, m: &Metadata) -> Option<u64> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if m.nlink() > 1 {
                let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
                if !seen.insert((m.dev(), m.ino())) {
                    return None;
                }
            }
        }
        Some(fsops::allocated(m))
    }

    fn tick(&self, path: &Path) {
        let n = self.entries.fetch_add(1, Ordering::Relaxed) + 1;
        if !n.is_multiple_of(2048) {
            return;
        }
        if let Ok(mut last) = self.last_tick.try_lock() {
            if last.elapsed() >= Duration::from_millis(250) {
                *last = Instant::now();
                super::super::progress("disk_usage", "scan", n, 0, path.display().to_string());
            }
        }
    }

    fn count_error(&self, e: &std::io::Error) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            self.denied.fetch_add(1, Ordering::Relaxed);
        } else {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The entries of a folder, links and other volumes already dropped.
    /// Returns (subfolders, files) with their metadata.
    fn list(&self, path: &Path) -> (Entries, Entries) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let rd = match std::fs::read_dir(path) {
            Ok(rd) => rd,
            Err(e) => {
                self.count_error(&e);
                return (dirs, files);
            }
        };
        for entry in rd {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    self.count_error(&e);
                    continue;
                }
            };
            self.tick(path);
            // `DirEntry::metadata` does not follow links.
            let m = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    self.count_error(&e);
                    continue;
                }
            };
            if fsops::is_link(&m) {
                continue;
            }
            if m.is_dir() {
                if self.same_volume(&m) {
                    dirs.push((entry.file_name(), m));
                }
            } else {
                files.push((entry.file_name(), m));
            }
        }
        (dirs, files)
    }

    fn files_into(&self, node: &mut Node, files: Vec<(OsString, Metadata)>) {
        let mut kept = Vec::with_capacity(files.len());
        for (name, m) in files {
            if let Some(b) = self.file_bytes(&m) {
                node.bytes += b;
                node.files += 1;
                kept.push(Node::file(name.to_string_lossy().into_owned(), b));
            }
        }
        node.fold_files(kept);
    }

    /// Plain recursive walk of one subtree, on the calling thread.
    fn walk(&self, path: &Path, name: String, own: &Metadata) -> Node {
        let mut node = Node::dir(name, fsops::allocated(own));
        let (dirs, files) = self.list(path);
        self.files_into(&mut node, files);
        for (n, m) in dirs {
            let child = self.walk(&path.join(&n), n.to_string_lossy().into_owned(), &m);
            node.add(child);
        }
        node
    }
}

/// A folder listed on the calling thread, whose deep subfolders are jobs.
struct Skel {
    node: Node,
    subs: Vec<Sub>,
}

enum Sub {
    Skel(Skel),
    Job(usize),
}

type Job = (PathBuf, String, Metadata);

fn skeleton(
    w: &Walk,
    path: &Path,
    name: String,
    own: &Metadata,
    depth: usize,
    jobs: &mut Vec<Job>,
) -> Skel {
    let mut node = Node::dir(name, fsops::allocated(own));
    let (dirs, files) = w.list(path);
    w.files_into(&mut node, files);
    let mut subs = Vec::with_capacity(dirs.len());
    for (n, m) in dirs {
        let p = path.join(&n);
        let n = n.to_string_lossy().into_owned();
        if depth + 1 < SPLIT_DEPTH {
            subs.push(Sub::Skel(skeleton(w, &p, n, &m, depth + 1, jobs)));
        } else {
            jobs.push((p, n, m));
            subs.push(Sub::Job(jobs.len() - 1));
        }
    }
    Skel { node, subs }
}

fn assemble(skel: Skel, done: &mut [Option<Node>]) -> Node {
    let mut node = skel.node;
    for s in skel.subs {
        let child = match s {
            Sub::Skel(k) => assemble(k, done),
            Sub::Job(i) => match done[i].take() {
                Some(n) => n,
                None => continue,
            },
        };
        node.add(child);
    }
    node
}

fn display_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Walk `root` with a small pool of threads (no rayon in the core).
pub fn scan(root: &Path) -> Result<Scan, String> {
    let meta = std::fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if !meta.is_dir() || fsops::is_link(&meta) {
        return Err(format!("NOT_A_FOLDER: {}", root.display()));
    }
    let started = Instant::now();
    let w = Walk::new(&meta);
    super::super::progress("disk_usage", "scan", 0, 0, root.display().to_string());
    let mut jobs: Vec<Job> = Vec::new();
    let skel = skeleton(&w, root, display_name(root), &meta, 0, &mut jobs);

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8)
        .min(jobs.len().max(1));
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<Node>>> = (0..jobs.len()).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((p, n, m)) = jobs.get(i) else { break };
                let node = w.walk(p, n.clone(), m);
                *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(node);
            });
        }
    });
    let mut done: Vec<Option<Node>> = results
        .into_iter()
        .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect();
    let tree = assemble(skel, &mut done);
    let entries = w.entries.load(Ordering::Relaxed);
    super::super::progress(
        "disk_usage",
        "done",
        entries,
        entries,
        format!("{} in {} ms", root.display(), started.elapsed().as_millis()),
    );
    Ok(Scan {
        root: root.to_path_buf(),
        tree,
        entries,
        denied: w.denied.load(Ordering::Relaxed),
        errors: w.errors.load(Ordering::Relaxed),
        scanned_at: super::super::store::now(),
        at: Instant::now(),
    })
}

// ── Cache ────────────────────────────────────────────────────────────

static CACHE: Mutex<Vec<Arc<Scan>>> = Mutex::new(Vec::new());

/// A fresh-enough scan whose root contains `path`.
pub fn cached_for(path: &Path) -> Option<Arc<Scan>> {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|s| s.at.elapsed() < CACHE_TTL);
    c.iter()
        .filter(|s| path.starts_with(&s.root))
        // The deepest root that holds the path: its tree is the most recent
        // look at that folder.
        .max_by_key(|s| s.root.components().count())
        .cloned()
}

pub fn remember(scan: Scan) -> Arc<Scan> {
    let scan = Arc::new(scan);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|s| s.root != scan.root && s.at.elapsed() < CACHE_TTL);
    c.push(scan.clone());
    while c.len() > CACHE_MAX {
        c.remove(0);
    }
    scan
}

/// The folder at `path` inside a scan, if the scan holds it.
pub fn find<'a>(scan: &'a Scan, path: &Path) -> Option<&'a Node> {
    let rel = path.strip_prefix(&scan.root).ok()?;
    let mut node = &scan.tree;
    for c in rel.components() {
        match c {
            Component::Normal(n) => node = node.child(&n.to_string_lossy())?,
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(node)
}

// ── Output ───────────────────────────────────────────────────────────

/// JSON of `node` (at `path`), `depth` levels deep, children smaller than
/// `threshold` bytes folded into one `other` node per folder.
pub fn view(node: &Node, path: &Path, depth: u32, threshold: u64) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(node.name));
    o.insert("path".into(), json!(path.display().to_string()));
    o.insert("bytes".into(), json!(node.bytes));
    o.insert("files".into(), json!(node.files));
    o.insert(
        "kind".into(),
        json!(if node.is_dir { "dir" } else { "file" }),
    );
    if !node.is_dir {
        return Value::Object(o);
    }
    let expandable = !node.children.is_empty() || node.rest_files > 0;
    o.insert("expandable".into(), json!(expandable));
    if depth == 0 {
        o.insert("children".into(), json!([]));
        return Value::Object(o);
    }
    let mut kids: Vec<&Node> = node.children.iter().collect();
    kids.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    let mut out = Vec::new();
    let (mut other_bytes, mut other_files, mut other_count) =
        (node.rest_bytes, node.rest_files, node.rest_files);
    for k in kids {
        if out.len() < MAX_CHILDREN && k.bytes >= threshold && (k.bytes > 0 || threshold == 0) {
            out.push(view(k, &path.join(&k.name), depth - 1, threshold));
        } else {
            other_bytes += k.bytes;
            other_files += k.files;
            other_count += 1;
        }
    }
    if other_count > 0 {
        out.push(json!({ "name": "other", "kind": "other", "bytes": other_bytes, "files": other_files, "count": other_count }));
    }
    o.insert("children".into(), Value::Array(out));
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![7u8; n]).unwrap();
    }

    fn alloc(p: &Path) -> u64 {
        fsops::allocated(&std::fs::symlink_metadata(p).unwrap())
    }

    fn kid<'a>(v: &'a Value, name: &str) -> &'a Value {
        v["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {v}"))
    }

    #[test]
    fn sums_allocated_and_splits_into_jobs() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(&r.join("a/b/c/deep.bin"), 300_000);
        write(&r.join("a/x.bin"), 100_000);
        write(&r.join("top.bin"), 50_000);
        let s = scan(r).unwrap();
        let files = alloc(&r.join("a/b/c/deep.bin"))
            + alloc(&r.join("a/x.bin"))
            + alloc(&r.join("top.bin"));
        assert_eq!(s.tree.files, 3);
        assert!(s.tree.bytes >= files, "{} < {files}", s.tree.bytes);
        let a = s.tree.child("a").unwrap();
        let c = a.child("b").unwrap().child("c").unwrap();
        assert_eq!(c.files, 1);
        assert!(c.bytes >= alloc(&r.join("a/b/c/deep.bin")));
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_counts_once_and_symlink_is_not_followed() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(&r.join("one/big.bin"), 400_000);
        std::fs::create_dir_all(r.join("two")).unwrap();
        std::fs::hard_link(r.join("one/big.bin"), r.join("two/same.bin")).unwrap();
        // A link to a big tree outside the root must not be counted.
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("huge.bin"), 900_000);
        std::os::unix::fs::symlink(outside.path(), r.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("huge.bin"), r.join("file-link")).unwrap();
        let s = scan(r).unwrap();
        let big = alloc(&r.join("one/big.bin"));
        assert_eq!(s.tree.files, 1, "hardlink counted twice or link followed");
        assert!(s.tree.bytes < big * 2, "{} vs {big}", s.tree.bytes);
        assert!(s.tree.bytes < big + 900_000);
        assert!(s.tree.child("link").is_none());
        let one = s.tree.child("one").unwrap().files;
        let two = s.tree.child("two").unwrap().files;
        assert_eq!(one + two, 1);
    }

    #[test]
    fn cut_by_depth_and_fraction_with_other() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(&r.join("big/inner/deeper/f.bin"), 800_000);
        for i in 0..30 {
            write(&r.join(format!("small/s{i}.bin")), 4096);
        }
        write(&r.join("tiny.txt"), 10);
        let s = scan(r).unwrap();
        let root = &s.tree;

        let v = view(root, r, 1, (root.bytes as f64 * 0.05) as u64);
        let big = kid(&v, "big");
        assert_eq!(big["kind"], "dir");
        assert_eq!(big["expandable"], true);
        assert!(
            big["children"].as_array().unwrap().is_empty(),
            "depth 1 lists no grandchildren"
        );
        let other = kid(&v, "other");
        assert_eq!(other["kind"], "other");
        assert!(other["count"].as_u64().unwrap() >= 1);
        let listed: u64 = v["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["bytes"].as_u64().unwrap())
            .sum();
        // Children plus the folder's own blocks make the total.
        assert!(listed <= v["bytes"].as_u64().unwrap());

        let v3 = view(root, r, 3, 0);
        let deeper = kid(kid(kid(&v3, "big"), "inner"), "deeper");
        assert!(deeper["children"].as_array().unwrap().is_empty());
        assert_eq!(
            kid(&v3, "small")["children"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["kind"] == "file")
                .count(),
            24
        );
        assert_eq!(
            kid(kid(&v3, "small"), "other")["count"],
            6,
            "files past KEEP_FILES fold into other"
        );
    }

    #[test]
    fn drill_down_from_cache() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_path_buf();
        write(&r.join("a/b/f.bin"), 10_000);
        let s = remember(scan(&r).unwrap());
        let hit = cached_for(&r.join("a/b")).unwrap();
        assert!(Arc::ptr_eq(&hit, &s));
        let b = find(&hit, &r.join("a/b")).unwrap();
        assert_eq!(b.files, 1);
        assert!(find(&hit, &r.join("a/nope")).is_none());
        assert!(cached_for(Path::new("/definitely/not/scanned")).is_none());
    }

    #[test]
    fn refuses_a_file() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("f"), 1);
        assert!(scan(&d.path().join("f"))
            .unwrap_err()
            .starts_with("NOT_A_FOLDER"));
    }
}
