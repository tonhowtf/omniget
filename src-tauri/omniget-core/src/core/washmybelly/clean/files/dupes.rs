//! Exact duplicates: group by size, then a hash of the first and last 64 KB,
//! then a full xxh3-128 hash. Hashes are cached by (dev, inode, size, mtime)
//! in `hashcache.sqlite`; hardlinks of one inode are one file, not copies.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use xxhash_rust::xxh3::{xxh3_128, Xxh3};

use super::super::super::fsops;
use super::super::super::plan::{self, Op, PlanItem};
use super::super::super::protect::{self, Guard};
use super::super::super::store::Ctx;
use super::super::super::{Destination, Risk};
use super::{dev_ino, mtime_ns, roots, skip_dir};

const EDGE: u64 = 64 * 1024;
const TASK: &str = "find_duplicates";

struct File {
    path: PathBuf,
    root: usize,
    len: u64,
    alloc: u64,
    dev: u64,
    ino: u64,
    mtime: i64,
}

fn collect(roots: &[PathBuf], min: u64) -> Vec<File> {
    let mut out = Vec::new();
    let mut seen_ino = HashSet::new();
    for (ri, root) in roots.iter().enumerate() {
        let mut it = walkdir::WalkDir::new(root).follow_links(false).into_iter();
        while let Some(entry) = it.next() {
            let Ok(e) = entry else { continue };
            let ft = e.file_type();
            if ft.is_dir() {
                if skip_dir(e.path(), root) {
                    it.skip_current_dir();
                }
                continue;
            }
            if !ft.is_file() || e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            if fsops::is_link(&m) || m.len() == 0 || m.len() < min {
                continue;
            }
            let (dev, ino) = dev_ino(&m, e.path());
            // Another name of an inode already seen: the same file, not a copy.
            if !seen_ino.insert((dev, ino)) {
                continue;
            }
            out.push(File {
                path: e.into_path(),
                root: ri,
                len: m.len(),
                alloc: fsops::allocated(&m),
                dev,
                ino,
                mtime: mtime_ns(&m),
            });
            if out.len().is_multiple_of(2000) {
                super::super::super::progress(TASK, "walk", out.len() as u64, 0, "");
            }
        }
    }
    out
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Head and tail; a file up to 128 KB is hashed whole (then it is also the full hash).
fn partial_hash(p: &Path, len: u64) -> Option<u128> {
    let mut f = std::fs::File::open(p).ok()?;
    if len <= 2 * EDGE {
        let mut buf = vec![0u8; len as usize];
        let n = read_full(&mut f, &mut buf).ok()?;
        return (n as u64 == len).then(|| xxh3_128(&buf));
    }
    let mut buf = vec![0u8; 2 * EDGE as usize];
    let (head, tail) = buf.split_at_mut(EDGE as usize);
    if read_full(&mut f, head).ok()? != EDGE as usize {
        return None;
    }
    f.seek(SeekFrom::Start(len - EDGE)).ok()?;
    if read_full(&mut f, tail).ok()? != EDGE as usize {
        return None;
    }
    Some(xxh3_128(&buf))
}

fn full_hash(p: &Path) -> Option<u128> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Xxh3::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match f.read(&mut buf).ok()? {
            0 => break,
            n => h.update(&buf[..n]),
        }
    }
    Some(h.digest128())
}

/// Map in parallel with scoped threads (no rayon in the core).
fn par_map<T: Sync, R: Send + Default + Clone>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let next = AtomicUsize::new(0);
    let mut out = vec![R::default(); items.len()];
    let chunks: Vec<Vec<(usize, R)>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= items.len() {
                            break;
                        }
                        local.push((i, f(&items[i])));
                    }
                    local
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (i, r) in chunks.into_iter().flatten() {
        out[i] = r;
    }
    out
}

// ── Hash cache ───────────────────────────────────────────────────────

struct Cache {
    db: Option<Connection>,
    hits: u64,
    pending: Vec<(u64, u64, u64, i64, &'static str, u128)>,
}

impl Cache {
    fn open(ctx: &Ctx) -> Cache {
        let db = Connection::open(ctx.dir.join("hashcache.sqlite")).ok().filter(|db| {
            db.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS hashes (dev INTEGER NOT NULL, ino INTEGER NOT NULL, size INTEGER NOT NULL, mtime INTEGER NOT NULL, partial TEXT, full TEXT, PRIMARY KEY(dev, ino, size, mtime));",
            )
            .is_ok()
        });
        Cache {
            db,
            hits: 0,
            pending: Vec::new(),
        }
    }

    fn get(&mut self, f: &File, col: &str) -> Option<u128> {
        let db = self.db.as_ref()?;
        let sql =
            format!("SELECT {col} FROM hashes WHERE dev=?1 AND ino=?2 AND size=?3 AND mtime=?4");
        let v: Option<String> = db
            .query_row(
                &sql,
                params![f.dev as i64, f.ino as i64, f.len as i64, f.mtime],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let h = v.and_then(|s| u128::from_str_radix(&s, 16).ok());
        if h.is_some() {
            self.hits += 1;
        }
        h
    }

    fn put(&mut self, f: &File, col: &'static str, h: u128) {
        self.pending.push((f.dev, f.ino, f.len, f.mtime, col, h));
    }

    fn flush(&mut self) {
        let Some(db) = self.db.as_mut() else { return };
        let Ok(tx) = db.transaction() else { return };
        for (dev, ino, size, mtime, col, h) in self.pending.drain(..) {
            let sql = format!(
                "INSERT INTO hashes(dev, ino, size, mtime, {col}) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(dev, ino, size, mtime) DO UPDATE SET {col}=excluded.{col}"
            );
            let _ = tx.execute(
                &sql,
                params![
                    dev as i64,
                    ino as i64,
                    size as i64,
                    mtime,
                    format!("{h:032x}")
                ],
            );
        }
        let _ = tx.commit();
    }
}

/// Hash `files` (indices into `all`) with the cache, computing misses in parallel.
fn hashes(
    all: &[File],
    idx: &[usize],
    cache: &mut Cache,
    col: &'static str,
    stage: &str,
) -> Vec<Option<u128>> {
    let mut out: Vec<Option<u128>> = idx.iter().map(|&i| cache.get(&all[i], col)).collect();
    let missing: Vec<usize> = (0..idx.len()).filter(|&k| out[k].is_none()).collect();
    super::super::super::progress(TASK, stage, 0, missing.len() as u64, "");
    let computed = par_map(&missing, |&k| {
        let f = &all[idx[k]];
        if col == "partial" {
            partial_hash(&f.path, f.len)
        } else {
            full_hash(&f.path)
        }
    });
    for (k, h) in missing.into_iter().zip(computed) {
        if let Some(h) = h {
            cache.put(&all[idx[k]], col, h);
            // Small files: the partial hash is the whole file.
            if col == "partial" && all[idx[k]].len <= 2 * EDGE {
                cache.put(&all[idx[k]], "full", h);
            }
        }
        out[k] = h;
    }
    super::super::super::progress(TASK, stage, idx.len() as u64, idx.len() as u64, "");
    out
}

fn group_by(idx: Vec<usize>, keys: Vec<Option<u128>>, all: &[File]) -> Vec<(u128, Vec<usize>)> {
    let mut m: HashMap<(u64, u128), Vec<usize>> = HashMap::new();
    for (i, k) in idx.into_iter().zip(keys) {
        if let Some(k) = k {
            m.entry((all[i].len, k)).or_default().push(i);
        }
    }
    m.into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((_, k), v)| (k, v))
        .collect()
}

/// Download folders, caches and temp folders hold the copy to let go.
fn in_scratch_place(p: &Path) -> bool {
    // Judge only what is below the home folder (or below the first folder
    // of the volume): `/tmp/...` or `/var/folders/.../T/` as a whole is not
    // what makes one copy more disposable than another.
    let home = dirs::home_dir();
    let rel: &Path = match home.as_deref().and_then(|h| p.strip_prefix(h).ok()) {
        Some(r) => r,
        None => {
            let mut it = p.components();
            it.next();
            it.next();
            it.as_path()
        }
    };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy().to_lowercase();
        matches!(
            s.as_str(),
            "downloads" | "caches" | "cache" | ".cache" | "tmp" | "temp" | "trash" | ".trash"
        )
    })
}

/// Which file of a group to keep: outside Downloads/caches first, then the
/// oldest, then the shortest path.
pub(super) fn pick_keep(paths: &[(&Path, i64)]) -> usize {
    (0..paths.len())
        .min_by_key(|&i| {
            (
                in_scratch_place(paths[i].0),
                paths[i].1,
                paths[i].0.components().count(),
                paths[i].0.to_path_buf(),
            )
        })
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn supports_clone(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid NUL-terminated path and a zeroed out-struct.
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return false;
    }
    let name: Vec<u8> = st
        .f_fstypename
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as u8)
        .collect();
    name == b"apfs"
}

pub fn find_duplicates(ctx: &Ctx, a: &Value) -> Result<Value, String> {
    let (roots, skipped_roots) = roots(a);
    if roots.is_empty() {
        return Err("no folder to search: pass roots that exist".into());
    }
    let min = a["min_kb"].as_u64().unwrap_or(64) * 1024;
    super::super::super::progress(TASK, "start", 0, 0, "");
    let all = collect(&roots, min);
    let scanned = all.len();

    // 1. Size.
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, f) in all.iter().enumerate() {
        by_size.entry(f.len).or_default().push(i);
    }
    let idx: Vec<usize> = by_size
        .into_values()
        .filter(|v| v.len() > 1)
        .flatten()
        .collect();

    // 2. Partial hash.
    let mut cache = Cache::open(ctx);
    let partial = hashes(&all, &idx, &mut cache, "partial", "partial_hash");
    let pmap: HashMap<usize, u128> = idx
        .iter()
        .zip(&partial)
        .filter_map(|(&i, h)| h.map(|h| (i, h)))
        .collect();
    let groups = group_by(idx, partial, &all);

    // 3. Full hash (files up to 128 KB were hashed whole already).
    let (small, big): (Vec<usize>, Vec<usize>) = groups
        .into_iter()
        .flat_map(|(_, v)| v)
        .partition(|&i| all[i].len <= 2 * EDGE);
    let big_keys = hashes(&all, &big, &mut cache, "full", "full_hash");
    cache.flush();
    let mut final_groups = group_by(big, big_keys, &all);
    let small_keys = small.iter().map(|i| pmap.get(i).copied()).collect();
    final_groups.extend(group_by(small, small_keys, &all));

    // Keep one per group; offer the others.
    let mut out_groups = Vec::new();
    let mut trash_items = Vec::new();
    let mut kept_items: Vec<PlanItem> = Vec::new();
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut clone_items: Vec<PlanItem> = Vec::new();
    #[cfg(target_os = "macos")]
    let mut clone_ok: HashMap<u64, bool> = HashMap::new();
    for (key, g) in &final_groups {
        let pairs: Vec<(&Path, i64)> = g
            .iter()
            .map(|&i| (all[i].path.as_path(), all[i].mtime))
            .collect();
        let keep_pos = pick_keep(&pairs);
        let keep = &all[g[keep_pos]];
        let mut wasted = 0u64;
        let mut files = Vec::new();
        for (pos, &i) in g.iter().enumerate() {
            let f = &all[i];
            let is_keep = pos == keep_pos;
            files
                .push(json!({ "path": f.path, "mtime": f.mtime / 1_000_000_000, "keep": is_keep }));
            let root = &roots[f.root];
            if is_keep {
                // The kept copy is a candidate too (unticked), so the screen
                // can switch which file of the group stays.
                if protect::check(&f.path, Guard::Picked, Some(root)).is_ok() {
                    if let Ok(fp) = fsops::fingerprint(&f.path) {
                        kept_items.push(PlanItem {
                            id: format!("duplicates:trash:{}", f.path.display()),
                            label: f
                                .path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            detail: Some(format!("{} · kept copy", f.path.display())),
                            group: "duplicates".into(),
                            bytes: f.alloc,
                            risk: Risk::Reversible,
                            admin: false,
                            op: Op::RemovePath {
                                path: f.path.clone(),
                                destination: Destination::Trash,
                                fp,
                                root: Some(root.clone()),
                                guard: Guard::Picked,
                                contents_only: false,
                            },
                        });
                    }
                }
                continue;
            }
            if protect::check(&f.path, Guard::Picked, Some(root)).is_err() {
                continue;
            }
            let Ok(fp) = fsops::fingerprint(&f.path) else {
                continue;
            };
            wasted += f.alloc;
            let name = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            trash_items.push(PlanItem {
                id: format!("duplicates:trash:{}", f.path.display()),
                label: name.clone(),
                detail: Some(format!(
                    "{} · copy of {}",
                    f.path.display(),
                    keep.path.display()
                )),
                group: "duplicates".into(),
                bytes: f.alloc,
                risk: Risk::Reversible,
                admin: false,
                op: Op::RemovePath {
                    path: f.path.clone(),
                    destination: Destination::Trash,
                    fp,
                    root: Some(root.clone()),
                    guard: Guard::Picked,
                    contents_only: false,
                },
            });
            #[cfg(target_os = "macos")]
            if f.dev == keep.dev
                && *clone_ok
                    .entry(f.dev)
                    .or_insert_with(|| supports_clone(&keep.path))
            {
                clone_items.push(PlanItem {
                    id: format!("duplicates:clone:{}", f.path.display()),
                    label: name,
                    detail: Some(format!(
                        "{} · becomes a space-sharing clone of {}",
                        f.path.display(),
                        keep.path.display()
                    )),
                    group: "duplicates_clone".into(),
                    bytes: f.alloc,
                    risk: Risk::Reversible,
                    admin: false,
                    op: Op::Module {
                        module: "clean".into(),
                        action: "apfs_clone".into(),
                        args: json!({ "keep": keep.path, "replace": f.path, "fp": fp }),
                    },
                });
            }
        }
        out_groups.push(json!({ "hash": format!("{key:032x}"), "size": keep.len, "files": files, "wasted": wasted }));
    }
    out_groups.sort_by(|a, b| b["wasted"].as_u64().cmp(&a["wasted"].as_u64()));
    let total_bytes: u64 = trash_items.iter().map(|i| i.bytes).sum();
    let groups_total = out_groups.len();
    out_groups.truncate(2000);
    super::super::super::progress(TASK, "done", scanned as u64, scanned as u64, "");

    let source = super::super::artifacts::source_of(a);
    plan::register("duplicates", &kept_items);
    let plan_id = if trash_items.is_empty() {
        None
    } else {
        Some(plan::offer(ctx, "duplicates", trash_items, source)?.id)
    };
    // The clone alternative is its own plan over the same kind: pick either
    // the Trash items or the clone items of a file, never both.
    let clone_plan_id = if clone_items.is_empty() {
        None
    } else {
        Some(plan::offer(ctx, "duplicates", clone_items, source)?.id)
    };
    Ok(json!({
        "groups": out_groups,
        "groups_total": groups_total,
        "total_bytes": total_bytes,
        "plan_id": plan_id,
        "clone_plan_id": clone_plan_id,
        "scanned_files": scanned,
        "cache_hits": cache.hits,
        "roots": roots,
        "skipped_roots": skipped_roots,
    }))
}
