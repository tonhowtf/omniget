//! Verifiers and the completion verdict.
//!
//! A verifier looks at the artifact as it is now and writes a receipt tied
//! to its digest. The verdict counts only receipts that are still valid for
//! the current criteria version and the current digest. Missing, timed out,
//! incomplete or unknown is never a pass, and a command that exits 0 proves
//! that one check, not the others.
//!
//! What runs where:
//! - `artifact` and `tool_result` checks are pure and run here;
//! - `command` checks run in the mission workspace through the app's shell
//!   tool (the same boundary as the Loop check); this module only decides
//!   whether a criterion may run a command at all ([`command_allowed`]);
//! - `rubric` and `human` criteria never produce a machine pass: a review
//!   records a receipt with its confidence, and the acceptance policy decides
//!   whether that is enough or a person must accept.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Acceptance, Criterion, CriterionKind, MissionReceipt, Origin, Severity};
use crate::core::assist::db::AssistDb;

pub const VERIFIER_VERSION: &str = "2";
/// Largest file an artifact check reads.
pub const ARTIFACT_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Digest of a criterion that has no artifact to look at.
pub const NO_ARTIFACT: &str = "none";

/// A relative path that stays inside the workspace.
pub fn check_rel_path(p: &str) -> Result<(), String> {
    let path = Path::new(p);
    if p.trim().is_empty() || path.is_absolute() {
        return Err(format!("`{p}` must be a path relative to the workspace"));
    }
    for c in path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(format!("`{p}` leaves the workspace")),
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Maximum aggregate bytes copied and hashed for one criterion. Reaching a
/// bound or encountering an inaccessible object means unknown, never identity.
pub const DIGEST_MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const DIGEST_MAX_FILES: usize = 5_000;

/// Digest over effective-open snapshots, not metadata placeholders. No error,
/// missing file, symlink, unsupported object or exhausted bound is a digest.
pub fn artifact_digest(workspace: Option<&Path>, paths: &[String]) -> Option<String> {
    artifact_digest_pinned(workspace, paths, None)
}
pub fn artifact_digest_pinned(
    workspace: Option<&Path>,
    paths: &[String],
    expected: Option<&crate::core::secure_files::Identity>,
) -> Option<String> {
    if paths.is_empty() {
        return Some(NO_ARTIFACT.to_string());
    }
    let root = crate::core::secure_files::Root::open(workspace?, expected).ok()?;
    let mut sorted = paths.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut remaining = DIGEST_MAX_BYTES;
    let mut entries = Vec::new();
    let mut directories = Vec::new();
    for path in sorted {
        check_rel_path(&path).ok()?;
        let snapped = root.snapshot(Path::new(&path), remaining);
        if let Ok(snap) = snapped {
            remaining = remaining.checked_sub(snap.bytes)?;
            entries.push((path, snap.digest));
        } else if snapped
            .as_ref()
            .err()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            // Confirmed absent through the pinned descriptors (ENOENT on a
            // component opened with O_NOFOLLOW): a revision of its own, so
            // a `must_exist: false` receipt can be matched.
            entries.push((path, "absent".to_string()));
        } else {
            if expected.is_some() {
                return None;
            }
            let files = root.entries(Path::new(&path), DIGEST_MAX_FILES).ok()?;
            for (file, directory) in &files {
                let name = file.to_str()?.to_string();
                if *directory {
                    entries.push((format!("directory:{name}"), "directory".to_string()));
                    continue;
                }
                let snap = root.snapshot(file, remaining).ok()?;
                remaining = remaining.checked_sub(snap.bytes)?;
                entries.push((name, snap.digest));
                if entries.len() > DIGEST_MAX_FILES {
                    return None;
                }
            }
            // Include the directory dependency even when empty.
            entries.push((format!("directory:{path}"), "directory".to_string()));
            directories.push((path, files));
        }
        if entries.len() > DIGEST_MAX_FILES {
            return None;
        }
    }
    for (path, before) in directories {
        if root.entries(Path::new(&path), DIGEST_MAX_FILES).ok()? != before {
            return None;
        }
    }
    entries.sort();

    Some(sha256_hex(&serde_json::to_vec(&entries).ok()?))
}

/// Directories skipped anywhere: VCS metadata, dependency trees, bytecode.
const SKIP_ANYWHERE: &[&str] = &[".git", "node_modules", "__pycache__"];
/// Build output skipped only at the workspace root: `src/build/*.rs` is
/// source and must invalidate a command receipt (F-V1).
const SKIP_AT_ROOT: &[&str] = &["target", ".svelte-kit", "build", "dist", ".next"];

fn skipped_dir(name: &[u8], depth: usize) -> bool {
    SKIP_ANYWHERE.iter().any(|s| s.as_bytes() == name)
        || (depth == 0 && SKIP_AT_ROOT.iter().any(|s| s.as_bytes() == name))
}

/// Invalidation-only digest of a whole workspace (a command criterion with
/// no declared artifact). Descriptor-relative walk: every entry is classified
/// with `fstatat(AT_SYMLINK_NOFOLLOW)` and opened with
/// `openat(O_NOFOLLOW|O_NONBLOCK)` and checked again with `fstat` on the
/// descriptor, so a name swapped for a symlink or a FIFO after the listing is
/// never followed nor blocks (F-V2). Symlinks are recorded by their target
/// bytes (never followed), special files by their kind (never opened).
/// Reads are bounded by the bytes actually read. `.git`, `node_modules` and
/// `__pycache__` are skipped at any depth, build output only at the root. Any
/// bound reached or entry that changed kind makes it unknown (`None`).
#[cfg(unix)]
pub fn workspace_digest(root: &Path) -> Option<String> {
    use std::ffi::{CStr, CString};
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    struct Stream(*mut libc::DIR);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    fn open_at(dir: &std::fs::File, name: &CStr, flags: i32) -> Option<std::fs::File> {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        (fd >= 0).then(|| unsafe { std::fs::File::from_raw_fd(fd) })
    }
    fn kind_at(dir: &std::fs::File, name: &CStr) -> Option<u32> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        (r == 0).then_some(st.st_mode as u32 & libc::S_IFMT as u32)
    }
    fn names(dir: &std::fs::File) -> Option<Vec<Vec<u8>>> {
        let scan = open_at(dir, c".", libc::O_RDONLY | libc::O_DIRECTORY)?;
        let fd = scan.into_raw_fd();
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            return None;
        }
        let stream = Stream(stream);
        let mut out = Vec::new();
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0
            };
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0
            };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                    return None;
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_bytes()
                .to_vec();
            if name != b"." && name != b".." {
                out.push(name);
            }
            if out.len() > DIGEST_MAX_FILES * 4 {
                return None;
            }
        }
        out.sort();
        Some(out)
    }
    fn walk(
        dir: &std::fs::File,
        prefix: &str,
        depth: usize,
        remaining: &mut u64,
        entries: &mut Vec<(String, String)>,
    ) -> Option<()> {
        if depth > 64 {
            return None;
        }
        for name in names(dir)? {
            let c_name = CString::new(name.clone()).ok()?;
            let shown = String::from_utf8_lossy(&name);
            let rel = if prefix.is_empty() {
                shown.to_string()
            } else {
                format!("{prefix}/{shown}")
            };
            // Lossless identity for names that are not UTF-8.
            let rel = if std::str::from_utf8(&name).is_ok() {
                rel
            } else {
                format!("{rel}#{}", hex::encode(&name))
            };
            match kind_at(dir, &c_name)? {
                k if k == libc::S_IFLNK as u32 => {
                    let mut buf = vec![0u8; 4096];
                    let n = unsafe {
                        libc::readlinkat(
                            dir.as_raw_fd(),
                            c_name.as_ptr(),
                            buf.as_mut_ptr() as *mut libc::c_char,
                            buf.len(),
                        )
                    };
                    if n < 0 {
                        return None;
                    }
                    buf.truncate(n as usize);
                    entries.push((rel, format!("symlink:{}", hex::encode(&buf))));
                }
                k if k == libc::S_IFDIR as u32 => {
                    if skipped_dir(&name, depth) {
                        continue;
                    }
                    let sub = open_at(dir, &c_name, libc::O_RDONLY | libc::O_DIRECTORY)?;
                    if !sub.metadata().ok()?.is_dir() {
                        return None;
                    }
                    entries.push((rel.clone(), "dir".into()));
                    walk(&sub, &rel, depth + 1, remaining, entries)?;
                }
                k if k == libc::S_IFREG as u32 => {
                    let file = open_at(dir, &c_name, libc::O_RDONLY)?;
                    // The object actually opened must still be a regular file.
                    if !file.metadata().ok()?.is_file() {
                        return None;
                    }
                    let mut bytes = Vec::new();
                    file.take(*remaining + 1).read_to_end(&mut bytes).ok()?;
                    *remaining = remaining.checked_sub(bytes.len() as u64)?;
                    entries.push((rel, sha256_hex(&bytes)));
                }
                // FIFOs, sockets, devices: recorded, never opened.
                _ => entries.push((rel, "special".into())),
            }
            if entries.len() > DIGEST_MAX_FILES * 4 {
                return None;
            }
        }
        Some(())
    }
    let top = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .ok()?;
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut remaining = DIGEST_MAX_BYTES;
    walk(&top, "", 0, &mut remaining, &mut entries)?;
    entries.sort();
    Some(sha256_hex(&serde_json::to_vec(&entries).ok()?))
}

#[cfg(not(unix))]
pub fn workspace_digest(root: &Path) -> Option<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut remaining = DIGEST_MAX_BYTES;
    while let Some((dir, depth)) = stack.pop() {
        for ent in std::fs::read_dir(&dir).ok()? {
            let ent = ent.ok()?;
            let path = ent.path();
            let rel = path.strip_prefix(root).ok()?.to_str()?.to_string();
            let name = ent.file_name();
            let md = std::fs::symlink_metadata(&path).ok()?;
            let ft = md.file_type();
            if ft.is_dir() && skipped_dir(name.to_str()?.as_bytes(), depth) {
                continue;
            }
            if ft.is_symlink() {
                let target = std::fs::read_link(&path).ok()?;
                entries.push((rel, format!("symlink:{}", target.display())));
            } else if ft.is_dir() {
                entries.push((rel, "dir".into()));
                stack.push((path, depth + 1));
            } else if ft.is_file() {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .ok()?
                    .take(remaining + 1)
                    .read_to_end(&mut bytes)
                    .ok()?;
                remaining = remaining.checked_sub(bytes.len() as u64)?;
                entries.push((rel, sha256_hex(&bytes)));
            } else {
                entries.push((rel, "special".into()));
            }
            if entries.len() > DIGEST_MAX_FILES * 4 {
                return None;
            }
        }
    }
    entries.sort();
    Some(sha256_hex(&serde_json::to_vec(&entries).ok()?))
}

/// Digest for one criterion with the mission's workspace. Tool results use
/// their own digest (what they point at in the database).
pub fn digest_for(
    db: Option<&AssistDb>,
    workspace: Option<&Path>,
    c: &Criterion,
    since_ms: i64,
) -> Option<String> {
    match c.kind {
        CriterionKind::ToolResult => tool_result_digest(db?, c, since_ms),
        // A command with no declared artifact depends on the whole workspace:
        // any change there invalidates its receipt.
        CriterionKind::Command if c.artifact_paths().is_empty() && workspace.is_some() => {
            workspace_digest(workspace?)
        }
        _ => artifact_digest(workspace, &c.artifact_paths()),
    }
}

/// Whether a criterion may run its command, and why not.
pub fn command_allowed(c: &Criterion) -> Result<(), String> {
    if c.kind != CriterionKind::Command {
        return Err("not a command criterion".into());
    }
    if !c.origin.may_execute() {
        return Err(match c.origin {
            Origin::Proposed => "proposed by a model: approve it to let it run".into(),
            Origin::Import => "came from an import: imported text never runs as a command".into(),
            _ => "not authorised".into(),
        });
    }
    Ok(())
}

/// What a check found, before it becomes a receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckOutcome {
    /// `pass|fail|unknown`
    pub status: String,
    pub digest: String,
    pub evidence: String,
    pub exit_code: Option<i64>,
    pub artifact_ref: String,
}

impl CheckOutcome {
    fn unknown(why: &str) -> Self {
        Self {
            status: "unknown".into(),
            digest: String::new(),
            evidence: why.to_string(),
            exit_code: None,
            artifact_ref: String::new(),
        }
    }
}

/// Why an artifact path could not be snapshotted as a regular file. It only
/// names the cause for the receipt and the controller; the check itself stays
/// bound to the descriptor-based snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactProblem {
    /// Nothing at that path (or a folder on the way is missing).
    NotFound,
    /// The path is a folder; criteria check files.
    IsDirectory,
    /// Larger than the limit a check reads (bytes found).
    TooLarge(u64),
    /// A symlink, hard link, special file or a file that changed mid-read.
    Unsafe(&'static str),
}

impl ArtifactProblem {
    /// Stable code for machines (`missions_artifacts`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "NOT_FOUND",
            Self::IsDirectory => "IS_DIRECTORY",
            Self::TooLarge(_) | Self::Unsafe(_) => "UNSAFE_OR_OVER_LIMIT",
        }
    }

    /// One plain sentence naming the path.
    pub fn describe(&self, path: &str, max_bytes: u64) -> String {
        match self {
            Self::NotFound => format!("not found: {path} (paths are relative to the mission folder; check the file name and the folder it is in)"),
            Self::IsDirectory => format!("{path} is a folder; criteria check files"),
            Self::TooLarge(n) => format!("{path} is {n} bytes, above the {max_bytes}-byte limit a check reads"),
            Self::Unsafe(why) => format!("{path} {why}"),
        }
    }
}

/// Names why `rel` under `root` could not be snapshotted (`err` is what the
/// snapshot returned). Read-only metadata, never followed through symlinks.
pub fn classify_artifact_error(
    root: &Path,
    rel: &str,
    err: &std::io::Error,
    max_bytes: u64,
) -> ArtifactProblem {
    let missing = |e: &std::io::Error| {
        matches!(
            e.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    };
    if missing(err) {
        return ArtifactProblem::NotFound;
    }
    // A folder on the way that is a symlink is refused, like the snapshot does.
    let mut cur = root.to_path_buf();
    let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        cur.push(part);
        match std::fs::symlink_metadata(&cur) {
            Err(e) if missing(&e) => return ArtifactProblem::NotFound,
            Err(_) => return ArtifactProblem::Unsafe("cannot be inspected"),
            Ok(m) if m.file_type().is_symlink() => {
                return ArtifactProblem::Unsafe(
                    "is or goes through a symbolic link; checks only read regular files",
                )
            }
            Ok(m) if i + 1 < parts.len() => {
                if !m.is_dir() {
                    return ArtifactProblem::NotFound;
                }
            }
            Ok(m) if m.is_dir() => return ArtifactProblem::IsDirectory,
            Ok(m) if !m.is_file() => return ArtifactProblem::Unsafe("is not a regular file"),
            Ok(m) if m.len() > max_bytes => return ArtifactProblem::TooLarge(m.len()),
            Ok(_m) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if _m.nlink() > 1 {
                        return ArtifactProblem::Unsafe("has more than one hard link; checks only read files with a single link");
                    }
                }
                return ArtifactProblem::Unsafe(
                    "changed while it was being read or cannot be read safely",
                );
            }
        }
    }
    ArtifactProblem::Unsafe("cannot be inspected")
}

/// Files elsewhere in the workspace with the same name as a missing path
/// (e.g. written into a subfolder by mistake). Bounded, never follows links.
pub fn same_name_elsewhere(root: &Path, rel: &str) -> Vec<String> {
    let Some(name) = Path::new(rel).file_name().map(|n| n.to_os_string()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            seen += 1;
            if seen > 2_000 || out.len() >= 3 {
                return out;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() && depth < 4 {
                stack.push((p.clone(), depth + 1));
            }
            if ft.is_file() && entry.file_name() == name {
                if let Ok(r) = p.strip_prefix(root) {
                    let r = r.to_string_lossy().replace('\\', "/");
                    if r != rel {
                        out.push(r);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Artifact criterion: `{ path, must_exist?, contains?[], not_contains?[],
/// json_keys?[], max_bytes?, min_bytes? }`.
pub fn check_artifact(workspace: Option<&Path>, c: &Criterion) -> CheckOutcome {
    check_artifact_pinned(workspace, c, None)
}
pub fn check_artifact_pinned(
    workspace: Option<&Path>,
    c: &Criterion,
    expected: Option<&crate::core::secure_files::Identity>,
) -> CheckOutcome {
    let Some(root) = workspace else {
        return CheckOutcome::unknown("the mission has no workspace to read the artifact from");
    };
    let path = c.spec.get("path").and_then(Value::as_str).unwrap_or("");
    if let Err(e) = check_rel_path(path) {
        return CheckOutcome::unknown(&e);
    }
    let safe_root = match crate::core::secure_files::Root::open(root, expected) {
        Ok(root) => root,
        Err(_) => return CheckOutcome::unknown("workspace cannot be opened safely"),
    };
    let must_exist_spec = c
        .spec
        .get("must_exist")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut snapshot = match safe_root.snapshot(Path::new(path), ARTIFACT_MAX_BYTES) {
        Ok(snapshot) => snapshot,
        // Absence confirmed on the descriptor is what `must_exist: false`
        // asks for (F-V3).
        Err(e) if !must_exist_spec && e.kind() == std::io::ErrorKind::NotFound => {
            return match artifact_digest_pinned(Some(root), &c.artifact_paths(), expected) {
                Some(digest) => CheckOutcome {
                    status: "pass".into(),
                    digest,
                    evidence: format!("{path} is absent, as required"),
                    exit_code: None,
                    artifact_ref: path.into(),
                },
                None => CheckOutcome::unknown(
                    "absence could not be confirmed for every artifact dependency",
                ),
            };
        }
        Err(e) => {
            let problem = classify_artifact_error(root, path, &e, ARTIFACT_MAX_BYTES);
            let why = problem.describe(path, ARTIFACT_MAX_BYTES);
            // Missing or a folder is a finding about the work, not about the
            // checker: it fails, bound to the revision seen (absent/folder).
            let fail = match problem {
                ArtifactProblem::NotFound if must_exist_spec => {
                    let elsewhere = same_name_elsewhere(root, path);
                    Some(if elsewhere.is_empty() {
                        why.clone()
                    } else {
                        format!("{why}\na file with that name exists at: {} — the check reads exactly {path}", elsewhere.join(", "))
                    })
                }
                ArtifactProblem::IsDirectory if must_exist_spec => Some(why.clone()),
                ArtifactProblem::IsDirectory => Some(format!("{path} exists but must not")),
                _ => None,
            };
            return match fail {
                Some(evidence) => CheckOutcome {
                    status: "fail".into(),
                    digest: artifact_digest_pinned(Some(root), &c.artifact_paths(), expected)
                        .unwrap_or_default(),
                    evidence,
                    exit_code: None,
                    artifact_ref: path.into(),
                },
                None => CheckOutcome::unknown(&why),
            };
        }
    };
    let mut bytes = Vec::new();
    use std::io::Read;
    if snapshot
        .file
        .by_ref()
        .take(ARTIFACT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 != snapshot.bytes
        || sha256_hex(&bytes) != snapshot.digest
    {
        return CheckOutcome::unknown("artifact snapshot could not be read consistently");
    }
    let must_exist = c
        .spec
        .get("must_exist")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut problems: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    if !must_exist {
        problems.push(format!("{path} exists but must not"));
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_)
            if str_list(&c.spec, "contains").is_empty()
                && str_list(&c.spec, "not_contains").is_empty()
                && str_list(&c.spec, "json_keys").is_empty() =>
        {
            ""
        }
        Err(_) => {
            return CheckOutcome::unknown(
                "artifact is not UTF-8; textual predicates cannot be evaluated",
            )
        }
    };
    {
        {
            if let Some(max) = c.spec.get("max_bytes").and_then(Value::as_u64) {
                if snapshot.bytes > max {
                    problems.push(format!("{path} is {} bytes, max {max}", snapshot.bytes));
                }
            }
            if let Some(min) = c.spec.get("min_bytes").and_then(Value::as_u64) {
                if snapshot.bytes < min {
                    problems.push(format!("{path} is {} bytes, min {min}", snapshot.bytes));
                }
            }
            for needle in str_list(&c.spec, "contains") {
                if text.contains(&needle) {
                    notes.push(format!("contains {needle:?}"));
                } else {
                    problems.push(format!("{path} does not contain {needle:?}"));
                }
            }
            for needle in str_list(&c.spec, "not_contains") {
                if text.contains(&needle) {
                    problems.push(format!("{path} still contains {needle:?}"));
                }
            }
            let keys = str_list(&c.spec, "json_keys");
            if !keys.is_empty() {
                match serde_json::from_str::<Value>(&text) {
                    Err(e) => problems.push(format!("{path} is not JSON: {e}")),
                    Ok(v) => {
                        for k in keys {
                            if pointer(&v, &k).is_none() {
                                problems.push(format!("{path} has no `{k}`"));
                            }
                        }
                    }
                }
            }
        }
    }
    // Bind acceptance to the immutable bytes actually checked. Recomputing
    // only before/after path hashes could accidentally attest an ABA revision.
    let mut manifest = Vec::new();
    let mut remaining = DIGEST_MAX_BYTES;
    for dependency in c.artifact_paths() {
        let (bytes, hash) =
            if dependency == path {
                (snapshot.bytes, snapshot.digest.clone())
            } else {
                match safe_root.snapshot(Path::new(&dependency), remaining) {
                    Ok(other) => (other.bytes, other.digest),
                    Err(_) => return CheckOutcome::unknown(
                        "additional artifact dependency cannot be snapshotted as a regular file",
                    ),
                }
            };
        let Some(left) = remaining.checked_sub(bytes) else {
            return CheckOutcome::unknown("artifact dependencies exceed snapshot budget");
        };
        remaining = left;
        manifest.push((dependency, hash));
    }
    manifest.sort();
    let digest = match serde_json::to_vec(&manifest) {
        Ok(encoded) => sha256_hex(&encoded),
        Err(_) => return CheckOutcome::unknown("artifact manifest cannot be encoded"),
    };
    if artifact_digest_pinned(Some(root), &c.artifact_paths(), expected).as_deref()
        != Some(digest.as_str())
    {
        return CheckOutcome::unknown("artifact revision changed during verification");
    }
    let status = if problems.is_empty() { "pass" } else { "fail" };
    let mut evidence = problems.clone();
    evidence.extend(notes);
    CheckOutcome {
        status: status.into(),
        digest,
        evidence: evidence.join("\n"),
        exit_code: None,
        artifact_ref: path.into(),
    }
}

fn pointer<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if key.starts_with('/') {
        v.pointer(key)
    } else {
        let mut cur = v;
        for part in key.split('.') {
            cur = cur.get(part)?;
        }
        Some(cur)
    }
}

fn str_list(spec: &Value, key: &str) -> Vec<String> {
    spec.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Tool-result digest: what the check points at (the newest reading round
/// of the journey recorded after the mission started, …).
pub fn tool_result_digest(db: &AssistDb, c: &Criterion, since_ms: i64) -> Option<String> {
    match c.spec.get("check").and_then(Value::as_str)? {
        "reading_round" => {
            let journey = c.spec.get("journey_id").and_then(Value::as_str)?;
            let round: Option<String> = db
                .with(|conn| {
                    use rusqlite::OptionalExtension;
                    conn.query_row(
                        "SELECT id FROM reading_rounds WHERE journey_id = ?1 AND created_ms >= ?2 ORDER BY created_ms DESC LIMIT 1",
                        rusqlite::params![journey, since_ms],
                        |r| r.get(0),
                    )
                    .optional()
                })
                .ok()?;
            Some(
                round
                    .map(|r| format!("round:{r}"))
                    .unwrap_or_else(|| "round:none".into()),
            )
        }
        _ => None,
    }
}

/// `tool_result` criterion. Supported checks:
/// - `reading_round { journey_id, min_items?, max_items?, first_round_roles? }`:
///   a round recorded by the reading rules after the mission started. The
///   rules themselves (roles, spoilers, access status, repeats) already
///   refused anything invalid, so a stored round is evidence; this check adds
///   the count and reports each film's access status as recorded (probable
///   stays probable).
pub fn check_tool_result(db: &AssistDb, c: &Criterion, since_ms: i64) -> CheckOutcome {
    let check = c.spec.get("check").and_then(Value::as_str).unwrap_or("");
    match check {
        "reading_round" => check_reading_round(db, c, since_ms),
        other => CheckOutcome::unknown(&format!(
            "tool check `{other}` is not supported; nothing was verified"
        )),
    }
}

fn check_reading_round(db: &AssistDb, c: &Criterion, since_ms: i64) -> CheckOutcome {
    let Some(journey) = c.spec.get("journey_id").and_then(Value::as_str) else {
        return CheckOutcome::unknown("reading_round needs journey_id");
    };
    let digest = tool_result_digest(db, c, since_ms).unwrap_or_else(|| "round:none".into());
    let Some(round_id) = digest
        .strip_prefix("round:")
        .filter(|r| *r != "none")
        .map(str::to_string)
    else {
        return CheckOutcome {
            status: "fail".into(),
            digest,
            evidence: "no reading round was recorded for this journey since the mission started"
                .into(),
            exit_code: None,
            artifact_ref: format!("journey:{journey}"),
        };
    };
    let rows: Vec<(String, String, String, String)> = db
        .with(|conn| {
            let mut st = conn.prepare(
                "SELECT i.role, i.status, COALESCE(m.title, i.movie_id), COALESCE(i.missing_json, '[]') FROM reading_round_items i \
                 LEFT JOIN reading_movies m ON m.id = i.movie_id WHERE i.round_id = ?1 ORDER BY i.position",
            )?;
            let rows = st.query_map(rusqlite::params![round_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        })
        .unwrap_or_default();
    let shortfall: Option<String> = db
        .with(|conn| {
            conn.query_row(
                "SELECT shortfall_reason FROM reading_rounds WHERE id = ?1",
                rusqlite::params![round_id],
                |r| r.get(0),
            )
        })
        .ok()
        .flatten();
    let n = rows.len() as u64;
    let min = c.spec.get("min_items").and_then(Value::as_u64).unwrap_or(1);
    let max = c.spec.get("max_items").and_then(Value::as_u64).unwrap_or(3);
    let mut problems = Vec::new();
    if n < min && shortfall.as_deref().map(str::trim).unwrap_or("").is_empty() {
        problems.push(format!(
            "{n} film(s), at least {min} expected and no shortfall explained"
        ));
    }
    if n > max {
        problems.push(format!("{n} film(s), at most {max}"));
    }
    if c.spec
        .get("first_round_roles")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && n == 3
    {
        let mut roles: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        roles.sort_unstable();
        if roles != ["entry", "shift", "surprise"] {
            problems.push("first round of three needs entry, shift and surprise".into());
        }
    }
    let mut lines: Vec<String> = rows
        .iter()
        .map(|(role, status, title, missing)| {
            let miss: Vec<String> = serde_json::from_str(missing).unwrap_or_default();
            if miss.is_empty() {
                format!("{title} [{role}] access: {status}")
            } else {
                format!(
                    "{title} [{role}] access: {status} (missing: {})",
                    miss.join("; ")
                )
            }
        })
        .collect();
    if let Some(s) = shortfall.filter(|s| !s.trim().is_empty()) {
        lines.push(format!("shortfall: {s}"));
    }
    let partial = n < min;
    let status = if !problems.is_empty() {
        "fail"
    } else if partial {
        lines.push("partial: fewer films than asked, with the reason recorded".into());
        "partial"
    } else {
        "pass"
    };
    let mut evidence = problems;
    evidence.extend(lines);
    CheckOutcome {
        status: status.into(),
        digest,
        evidence: evidence.join("\n"),
        exit_code: None,
        artifact_ref: format!("reading_round:{round_id}"),
    }
}

/// Command outcome as the shell tool reported it. `timed_out` or a missing
/// exit code is `unknown`, never a pass.
pub fn command_outcome(
    exit_code: Option<i64>,
    timed_out: bool,
    output: &str,
    digest: String,
    command: &str,
) -> CheckOutcome {
    let status = match (timed_out || digest.is_empty(), exit_code) {
        (true, _) => "unknown",
        (false, Some(0)) => "pass",
        (false, Some(_)) => "fail",
        (false, None) => "unknown",
    };
    let head = if timed_out {
        "timed out; the check did not finish\n"
    } else if digest.is_empty() {
        "artifact revision unavailable; exit zero does not establish acceptance\n"
    } else {
        ""
    };
    CheckOutcome {
        status: status.into(),
        digest,
        evidence: format!(
            "{head}$ {command}\nexit {}\n{}",
            exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            tail(output, 6000)
        ),
        exit_code,
        artifact_ref: command.to_string(),
    }
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("[…]\n{}", &s[cut..])
}

// ── Verdict ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionStatus {
    pub id: String,
    pub title: String,
    pub kind: CriterionKind,
    pub severity: Severity,
    pub acceptance: Acceptance,
    pub objective: bool,
    /// `pass|fail|unknown|missing|stale|awaiting_human`
    pub status: String,
    pub receipt_id: Option<String>,
    pub detail: String,
    /// Rubric reviews: what the reviewer said about its confidence.
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub passed: bool,
    pub criteria: Vec<CriterionStatus>,
    pub failing: Vec<String>,
    pub missing: Vec<String>,
    pub awaiting_human: Vec<String>,
    pub advisory_failing: Vec<String>,
    /// Required criteria met only in part, with the reason recorded.
    #[serde(default)]
    pub partial: Vec<String>,
    /// `auto` when every required criterion is objective; `human` when a
    /// person's judgement is part of the conclusion.
    pub completion: String,
}

impl Verdict {
    pub fn summary(&self) -> String {
        if self.passed {
            return "every required criterion passed".into();
        }
        let mut parts = Vec::new();
        if !self.failing.is_empty() {
            parts.push(format!("failing: {}", self.failing.join(", ")));
        }
        if !self.missing.is_empty() {
            parts.push(format!("not verified yet: {}", self.missing.join(", ")));
        }
        if !self.awaiting_human.is_empty() {
            parts.push(format!(
                "waiting for your decision: {}",
                self.awaiting_human.join(", ")
            ));
        }
        if !self.partial.is_empty() {
            parts.push(format!("partly met: {}", self.partial.join(", ")));
        }
        parts.join("; ")
    }
}

/// Decides, for the current criteria, which ones pass. `current` gives the
/// digest of each criterion's artifact now (`None` = unknown: a receipt
/// cannot be matched, so it does not count). A quick view without current
/// digests must report unknown, never treat historical receipts as current.
pub fn verdict(
    criteria: &[Criterion],
    receipts: &[MissionReceipt],
    current: &dyn Fn(&Criterion) -> Option<String>,
) -> Verdict {
    let mut v = Verdict {
        completion: "auto".into(),
        ..Default::default()
    };
    for c in criteria {
        if c.severity == Severity::Required
            && (!c.kind.is_objective() || c.acceptance != Acceptance::Auto)
        {
            v.completion = "human".into();
        }
        let now_digest = current(c).filter(|d| !d.is_empty());
        let mine: Vec<&MissionReceipt> = receipts
            .iter()
            .filter(|r| {
                r.criterion_id == c.id
                    && r.criterion_version == c.version
                    && r.invalidated_ms.is_none()
            })
            .collect();
        // Newest valid receipt for the artifact as it is now.
        let latest = mine
            .iter()
            .filter(|r| {
                now_digest
                    .as_ref()
                    .map(|d| d == &r.artifact_digest)
                    .unwrap_or(false)
            })
            .max_by_key(|r| r.created_ms);
        let stale = latest.is_none() && !mine.is_empty();
        let human_receipt = mine
            .iter()
            .filter(|r| r.verifier == "human")
            .filter(|r| {
                now_digest
                    .as_ref()
                    .map(|d| d == &r.artifact_digest)
                    .unwrap_or(false)
            })
            .max_by_key(|r| r.created_ms);
        let (status, receipt, detail, confidence) = if now_digest.is_none() {
            (
                "unknown".to_string(),
                None,
                "current revision unavailable; historical receipts cannot establish acceptance"
                    .into(),
                None,
            )
        } else {
            match c.acceptance {
                Acceptance::Human => match human_receipt {
                    Some(r) => (
                        r.status.clone(),
                        Some(r.id.clone()),
                        r.evidence.clone(),
                        r.confidence.clone(),
                    ),
                    None => {
                        let review = latest.map(|r| {
                            format!(
                                "review: {} ({})",
                                r.status,
                                r.confidence
                                    .clone()
                                    .unwrap_or_else(|| "no confidence".into())
                            )
                        });
                        (
                            "awaiting_human".to_string(),
                            latest.map(|r| r.id.clone()),
                            review.unwrap_or_else(|| "waiting for your decision".into()),
                            latest.and_then(|r| r.confidence.clone()),
                        )
                    }
                },
                _ => match latest {
                    Some(r) => (
                        r.status.clone(),
                        Some(r.id.clone()),
                        r.evidence.clone(),
                        r.confidence.clone(),
                    ),
                    None if stale => (
                        "stale".to_string(),
                        None,
                        "the artifact changed after the last check".into(),
                        None,
                    ),
                    None => ("missing".to_string(), None, "not verified yet".into(), None),
                },
            }
        };
        let label = format!("{} ({})", c.title, c.id);
        if c.severity == Severity::Required {
            match status.as_str() {
                "pass" => {}
                "partial" => v.partial.push(label.clone()),
                "fail" => v.failing.push(label.clone()),
                "awaiting_human" => v.awaiting_human.push(label.clone()),
                _ => v.missing.push(label.clone()),
            }
        } else if status != "pass" && status != "missing" {
            v.advisory_failing.push(label.clone());
        }
        v.criteria.push(CriterionStatus {
            id: c.id.clone(),
            title: c.title.clone(),
            kind: c.kind,
            severity: c.severity,
            acceptance: c.acceptance,
            objective: c.kind.is_objective() && c.acceptance == Acceptance::Auto,
            status,
            receipt_id: receipt,
            detail: super::clip(&detail, 1200),
            confidence,
        });
    }
    let has_required = criteria.iter().any(|c| c.severity == Severity::Required);
    v.passed = has_required
        && v.failing.is_empty()
        && v.missing.is_empty()
        && v.awaiting_human.is_empty()
        && v.partial.is_empty();
    v
}

/// A structured next action for a failing verdict (shown and used as the
/// prompt of the next round).
pub fn next_round_prompt(objective: &str, v: &Verdict) -> String {
    next_round_prompt_for(objective, v, &[])
}

/// [`next_round_prompt`] that also names the exact file each failing
/// artifact criterion reads, so the executor writes it where it is checked.
pub fn next_round_prompt_for(objective: &str, v: &Verdict, criteria: &[Criterion]) -> String {
    let mut lines = vec![
        format!("Objective: {objective}"),
        "The mission is not complete. These required checks do not pass yet:".to_string(),
    ];
    for c in v
        .criteria
        .iter()
        .filter(|c| c.severity == Severity::Required && c.status != "pass")
    {
        lines.push(format!(
            "- {} [{}]: {}",
            c.title,
            c.status,
            super::clip(&c.detail, 1500)
        ));
        let def = criteria
            .iter()
            .find(|d| d.id == c.id && d.kind == CriterionKind::Artifact);
        if let Some(path) = def.and_then(|d| d.spec.get("path")).and_then(Value::as_str) {
            lines.push(format!(
                "  Expected file: `{path}`, exactly this path relative to the mission folder (not inside another subfolder)."
            ));
        }
    }
    lines.push("Fix what the checks report. Saying you are done does not finish the mission; only the checks do.".into());
    lines.join("\n")
}

/// JSON for the UI of one outcome (kept small).
pub fn outcome_json(o: &CheckOutcome) -> Value {
    json!({ "status": o.status, "digest": o.digest, "exit_code": o.exit_code, "evidence": super::clip(&o.evidence, 2000) })
}

#[cfg(all(test, unix))]
mod revision_tests {
    use super::*;
    use std::path::PathBuf;
    fn workspace() -> PathBuf {
        let p = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-verifier-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn criterion() -> Criterion {
        Criterion {
            id: "file".into(),
            version: 1,
            kind: CriterionKind::Artifact,
            severity: Severity::Required,
            title: "file".into(),
            spec: json!({"path":"file"}),
            origin: Origin::User,
            acceptance: Acceptance::Auto,
        }
    }
    #[test]
    fn large_same_size_content_has_distinct_revision() {
        let p = workspace();
        let path = p.join("large");
        let mut data = vec![b'a'; ARTIFACT_MAX_BYTES as usize + 1];
        std::fs::write(&path, &data).unwrap();
        let first = artifact_digest(Some(&p), &["large".into()]).unwrap();
        *data.last_mut().unwrap() = b'b';
        std::fs::write(&path, &data).unwrap();
        assert_ne!(first, artifact_digest(Some(&p), &["large".into()]).unwrap());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn empty_directory_addition_changes_workspace_revision() {
        let p = workspace();
        let before = artifact_digest(Some(&p), &[".".into()]).unwrap();
        std::fs::create_dir(p.join("new-directory")).unwrap();
        assert_ne!(before, artifact_digest(Some(&p), &[".".into()]).unwrap());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn oversized_tree_is_unknown_and_similar_dependency_names_are_hashed() {
        let p = workspace();
        std::fs::write(p.join("node_modules-not-excluded"), b"a").unwrap();
        let before = artifact_digest(Some(&p), &[".".into()]).unwrap();
        std::fs::write(p.join("node_modules-not-excluded"), b"b").unwrap();
        assert_ne!(before, artifact_digest(Some(&p), &[".".into()]).unwrap());
        for i in 0..DIGEST_MAX_FILES {
            std::fs::write(p.join(format!("f{i}")), b"").unwrap();
        }
        assert_eq!(artifact_digest(Some(&p), &[".".into()]), None);
        std::fs::write(p.join("f0"), b"changed").unwrap();
        assert_eq!(artifact_digest(Some(&p), &[".".into()]), None);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn unsafe_unreadable_and_special_objects_cannot_be_receipt_identities() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let p = workspace();
        std::fs::write(p.join("file"), b"ok").unwrap();
        symlink("file", p.join("link")).unwrap();
        assert_eq!(artifact_digest(Some(&p), &["link".into()]), None);
        let fifo = std::ffi::CString::new(p.join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(artifact_digest(Some(&p), &["fifo".into()]), None);
        std::fs::set_permissions(p.join("file"), std::fs::Permissions::from_mode(0o0)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(artifact_digest(Some(&p), &["file".into()]), None);
        }
        std::fs::set_permissions(p.join("file"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn unavailable_revision_never_passes_historical_receipt_or_command() {
        let c = criterion();
        let r = MissionReceipt {
            id: "r".into(),
            mission_id: "m".into(),
            criterion_id: c.id.clone(),
            criterion_version: 1,
            task_id: None,
            run_id: None,
            artifact_ref: "file".into(),
            artifact_digest: "old".into(),
            verifier: "artifact".into(),
            verifier_version: VERIFIER_VERSION.into(),
            status: "pass".into(),
            exit_code: None,
            confidence: None,
            evidence: "old pass".into(),
            created_ms: 1,
            invalidated_ms: None,
            invalidated_reason: None,
        };
        let v = verdict(&[c], &[r], &|_| None);
        assert!(!v.passed);
        assert_eq!(v.criteria[0].status, "unknown");
        assert_eq!(
            command_outcome(Some(0), false, "ok", String::new(), "check").status,
            "unknown"
        );
    }
    #[test]
    fn artifact_check_cannot_treat_unreadable_or_missing_as_empty_success() {
        let p = workspace();
        let c = criterion();
        // Missing is a failure that says so, never a pass nor "unknown".
        let missing = check_artifact(Some(&p), &c);
        assert_eq!(missing.status, "fail");
        assert!(
            missing.evidence.starts_with("not found: file"),
            "{}",
            missing.evidence
        );
        std::fs::write(p.join("file"), b"ok").unwrap();
        assert_eq!(check_artifact(Some(&p), &c).status, "pass");
        std::fs::remove_dir_all(p).unwrap();
    }
    fn artifact(path: &str) -> Criterion {
        Criterion {
            spec: json!({ "path": path, "contains": ["Hello"] }),
            ..criterion()
        }
    }
    #[test]
    fn missing_artifact_fails_with_not_found_and_points_at_a_same_named_file() {
        let p = workspace();
        std::fs::create_dir(p.join("mission")).unwrap();
        std::fs::write(p.join("mission/hello.md"), b"Hello").unwrap();
        let c = artifact("hello.md");
        let o = check_artifact(Some(&p), &c);
        assert_eq!(o.status, "fail");
        assert!(o.evidence.contains("not found: hello.md"), "{}", o.evidence);
        assert!(o.evidence.contains("mission/hello.md"), "{}", o.evidence);
        assert!(
            !o.digest.is_empty(),
            "the failure is bound to the absent revision"
        );
        // Writing it where it is checked changes the revision and passes.
        std::fs::write(p.join("hello.md"), b"Hello").unwrap();
        assert_ne!(
            artifact_digest(Some(&p), &c.artifact_paths()).unwrap(),
            o.digest
        );
        assert_eq!(check_artifact(Some(&p), &c).status, "pass");
        // A missing folder on the way is also "not found".
        let deep = check_artifact(Some(&p), &artifact("nope/hello.md"));
        assert_eq!(deep.status, "fail");
        assert!(
            deep.evidence.starts_with("not found: nope/hello.md"),
            "{}",
            deep.evidence
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn a_folder_at_the_artifact_path_fails_as_a_folder() {
        let p = workspace();
        std::fs::create_dir(p.join("out")).unwrap();
        let o = check_artifact(Some(&p), &artifact("out"));
        assert_eq!(o.status, "fail");
        assert!(
            o.evidence.contains("is a folder; criteria check files"),
            "{}",
            o.evidence
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn unsafe_or_oversized_artifacts_stay_unknown_with_the_precise_reason() {
        let p = workspace();
        std::fs::write(p.join("real"), b"Hello").unwrap();
        std::os::unix::fs::symlink(p.join("real"), p.join("link")).unwrap();
        let o = check_artifact(Some(&p), &artifact("link"));
        assert_eq!(o.status, "unknown");
        assert!(o.evidence.contains("symbolic link"), "{}", o.evidence);
        std::fs::hard_link(p.join("real"), p.join("hard")).unwrap();
        let o = check_artifact(Some(&p), &artifact("hard"));
        assert_eq!(o.status, "unknown");
        assert!(o.evidence.contains("hard link"), "{}", o.evidence);
        std::fs::write(p.join("big"), vec![b'a'; ARTIFACT_MAX_BYTES as usize + 1]).unwrap();
        let o = check_artifact(Some(&p), &artifact("big"));
        assert_eq!(o.status, "unknown");
        assert!(o.evidence.contains("-byte limit"), "{}", o.evidence);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn classify_names_codes_for_the_controller() {
        let p = workspace();
        std::fs::create_dir(p.join("dir")).unwrap();
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "x");
        let gone = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        assert_eq!(
            classify_artifact_error(&p, "a.md", &gone, 10).code(),
            "NOT_FOUND"
        );
        assert_eq!(
            classify_artifact_error(&p, "dir", &denied, 10).code(),
            "IS_DIRECTORY"
        );
        std::fs::write(p.join("big"), vec![0u8; 20]).unwrap();
        assert_eq!(
            classify_artifact_error(&p, "big", &denied, 10),
            ArtifactProblem::TooLarge(20)
        );
        assert_eq!(
            classify_artifact_error(&p, "big", &denied, 10).code(),
            "UNSAFE_OR_OVER_LIMIT"
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn next_round_prompt_names_the_expected_path() {
        let c = artifact("hello.md");
        let v = Verdict {
            passed: false,
            criteria: vec![CriterionStatus {
                id: c.id.clone(),
                title: c.title.clone(),
                kind: c.kind,
                severity: Severity::Required,
                acceptance: Acceptance::Auto,
                objective: true,
                status: "fail".into(),
                receipt_id: None,
                detail: "not found: hello.md".into(),
                confidence: None,
            }],
            failing: vec![c.id.clone()],
            ..Default::default()
        };
        let prompt = next_round_prompt_for("write hello", &v, std::slice::from_ref(&c));
        assert!(prompt.contains("Expected file: `hello.md`"), "{prompt}");
        assert!(!next_round_prompt("write hello", &v).contains("Expected file"));
    }
}
