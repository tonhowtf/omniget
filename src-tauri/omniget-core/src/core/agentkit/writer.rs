//! Transactional writer (plan §4.4): apply a plan with a backup of every file it
//! touches under `<app_data>/agentkit/backups/<tx>/`, atomic writes, idempotent
//! merges and a lockfile entry per install; `uninstall` removes only what we
//! wrote (merged pieces included, checked by hash), `drift` reports user edits,
//! `restore` rolls a whole transaction back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::convert::{Dedupe, FileAction, PatchOp};
use super::edit::{self, textblock, DocFormat, Editor, Seg};
use super::lock::{self, InstallRecord, Lockfile, UndoOp, WrittenAction, WrittenFile};
use super::plan::{InstallPlan, UnitStatus};
use super::{now_iso, sha256_hex, AgentkitError, Env, Result};

pub(crate) fn value_sha(v: &Value) -> String {
    sha256_hex(serde_json::to_string(v).unwrap_or_default().as_bytes())
}

// ------------------------------------------------------------------ pure merge logic

/// Is `value` already in `arr` under this dedupe rule? `Err` = same identity
/// with another value (a conflict).
pub(crate) fn already_there(
    arr: &[Value],
    value: &Value,
    dedupe: &Dedupe,
) -> std::result::Result<bool, String> {
    match dedupe {
        Dedupe::Equal => Ok(arr.contains(value)),
        Dedupe::Fields { fields } => {
            for e in arr {
                if fields.iter().all(|f| e.get(f) == value.get(f)) {
                    return if e == value {
                        Ok(true)
                    } else {
                        Err(format!(
                            "an entry with the same {} already exists",
                            fields.join("+")
                        ))
                    };
                }
            }
            Ok(false)
        }
        Dedupe::ClaudeHook => {
            let norm = |m: Option<&Value>| {
                m.and_then(|x| x.as_str())
                    .map(|s| if s == "*" { "" } else { s })
                    .unwrap_or("")
                    .to_string()
            };
            let key = |h: &Value| {
                h.get("command")
                    .or_else(|| h.get("url"))
                    .or_else(|| h.get("prompt"))
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let wanted: Vec<Value> = value
                .get("hooks")
                .and_then(|h| h.as_array())
                .map(|a| a.iter().map(key).collect())
                .unwrap_or_default();
            let matcher = norm(value.get("matcher"));
            for g in arr {
                if norm(g.get("matcher")) != matcher {
                    continue;
                }
                let have: Vec<Value> = g
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|a| a.iter().map(key).collect())
                    .unwrap_or_default();
                if !wanted.is_empty() && wanted.iter().all(|w| have.contains(w)) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Dedupe::FlatHook => {
            let cmd = |v: &Value| v.get("command").or_else(|| v.get("bash")).cloned();
            Ok(arr
                .iter()
                .any(|e| cmd(e) == cmd(value) && e.get("matcher") == value.get("matcher")))
        }
    }
}

/// Result of applying ops to one document.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub text: String,
    pub undo: Vec<UndoOp>,
    /// Ops that did nothing (already present) or were skipped, for the report.
    pub notes: Vec<String>,
    /// Undo candidates for pieces that were already there with our exact value
    /// (a block, entry or key another install may own). The writer keeps one
    /// only when another install really holds the same piece, so the piece is
    /// reference-counted and goes out with its last holder.
    pub shared: Vec<UndoOp>,
}

/// Applies merge ops to a document's text. Pure; used by the plan (preview) and
/// by `apply` (for real) so both produce the same bytes.
pub fn apply_ops(format: DocFormat, text: &str, ops: &[PatchOp]) -> Result<Applied> {
    let mut out = Applied {
        text: text.to_string(),
        ..Default::default()
    };
    if format == DocFormat::Markdown {
        for op in ops {
            if let PatchOp::TextBlock { id, content } = op {
                if textblock::content(&out.text, id).as_deref()
                    == Some(content.trim_end_matches('\n'))
                {
                    out.notes.push(format!("block {id} already present"));
                    out.shared.push(UndoOp::TextBlock {
                        id: id.clone(),
                        sha256: sha256_hex(content.trim_end_matches('\n').as_bytes()),
                    });
                    continue;
                }
                let existed = textblock::find(&out.text, id).is_some();
                out.text = textblock::upsert(&out.text, id, content);
                if !existed {
                    out.undo.push(UndoOp::TextBlock {
                        id: id.clone(),
                        sha256: sha256_hex(content.trim_end_matches('\n').as_bytes()),
                    });
                }
            }
        }
        return Ok(out);
    }
    let base = if text.trim().is_empty() {
        format.empty_doc().to_string()
    } else {
        text.to_string()
    };
    let mut doc = edit::open(format, &base)?;
    for op in ops {
        match op {
            PatchOp::Set { path, value, .. } => {
                if doc.get(path).as_ref() == Some(value) {
                    out.notes
                        .push(format!("{} already set", edit::path_display(path)));
                    out.shared.push(UndoOp::Remove {
                        path: path.clone(),
                        prune_to: path.len().saturating_sub(1),
                        sha256: value_sha(value),
                    });
                    continue;
                }
                let o = doc.set(path, value)?;
                match (o.created_at, o.prev) {
                    (Some(i), _) => out.undo.push(UndoOp::Remove {
                        path: path.clone(),
                        prune_to: i,
                        sha256: value_sha(value),
                    }),
                    (None, Some(prev)) => out.undo.push(UndoOp::Restore {
                        path: path.clone(),
                        prev,
                        sha256: value_sha(value),
                    }),
                    (None, None) => {}
                }
            }
            PatchOp::Append {
                path,
                value,
                dedupe,
            } => {
                let arr = match doc.get(path) {
                    Some(Value::Array(a)) => a,
                    Some(Value::Null) | None => vec![],
                    Some(_) => {
                        return Err(AgentkitError::new(
                            "AGENTKIT_EDIT",
                            format!("`{}` is not a list", edit::path_display(path)),
                        ))
                    }
                };
                match already_there(&arr, value, dedupe) {
                    Ok(true) => {
                        out.notes.push(format!(
                            "{} already has this entry",
                            edit::path_display(path)
                        ));
                        if arr.contains(value) {
                            out.shared.push(UndoOp::RemoveItem {
                                path: path.clone(),
                                value: value.clone(),
                                prune_to: path.len(),
                            });
                        }
                        continue;
                    }
                    Err(why) => {
                        out.notes
                            .push(format!("{}: {why}; left as is", edit::path_display(path)));
                        continue;
                    }
                    Ok(false) => {}
                }
                let created = doc.push(path, value)?;
                let prune_to = created.and_then(|c| c.created_at).unwrap_or(path.len());
                out.undo.push(UndoOp::RemoveItem {
                    path: path.clone(),
                    value: value.clone(),
                    prune_to,
                });
            }
            PatchOp::TextBlock { .. } => {
                return Err(AgentkitError::new(
                    "AGENTKIT_EDIT",
                    "text blocks only go into Markdown files",
                ));
            }
        }
    }
    out.text = doc.text().to_string();
    if text.trim().is_empty() && out.undo.is_empty() {
        out.text = text.to_string();
    }
    Ok(out)
}

fn is_empty_container(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Null => true,
        _ => false,
    }
}

fn prune(doc: &mut dyn Editor, path: &[Seg], start_len: usize, prune_to: usize) -> Result<()> {
    let mut k = start_len;
    while k > prune_to && k > 0 {
        let anc = &path[..k];
        match doc.get(anc) {
            Some(v) if is_empty_container(&v) => {
                doc.remove(anc)?;
            }
            _ => break,
        }
        k -= 1;
    }
    Ok(())
}

/// Undoes merged pieces on a text. Returns the new text, whether the document
/// is now empty, and pieces that were not ours anymore (drift).
pub fn undo_ops(
    format: DocFormat,
    text: &str,
    undo: &[UndoOp],
    force: bool,
) -> Result<(String, bool, Vec<String>)> {
    let mut drift = Vec::new();
    if format == DocFormat::Markdown {
        let mut t = text.to_string();
        for u in undo.iter().rev() {
            if let UndoOp::TextBlock { id, sha256 } = u {
                match textblock::content(&t, id) {
                    Some(c) if force || sha256_hex(c.as_bytes()) == *sha256 => {
                        t = textblock::remove(&t, id).unwrap_or(t);
                    }
                    Some(_) => drift.push(format!("block {id} was edited")),
                    None => {}
                }
            }
        }
        let empty = t.trim().is_empty();
        return Ok((t, empty, drift));
    }
    let mut doc = edit::open(format, text)?;
    for u in undo.iter().rev() {
        match u {
            UndoOp::Remove {
                path,
                prune_to,
                sha256,
            } => match doc.get(path) {
                None => {}
                Some(v) if force || value_sha(&v) == *sha256 => {
                    doc.remove(path)?;
                    prune(doc.as_mut(), path, path.len().saturating_sub(1), *prune_to)?;
                }
                Some(_) => drift.push(format!("{} was edited", edit::path_display(path))),
            },
            UndoOp::Restore { path, prev, sha256 } => match doc.get(path) {
                Some(v) if force || value_sha(&v) == *sha256 => {
                    doc.set(path, prev)?;
                }
                Some(_) => drift.push(format!("{} was edited", edit::path_display(path))),
                None => drift.push(format!("{} was removed", edit::path_display(path))),
            },
            UndoOp::RemoveItem {
                path,
                value,
                prune_to,
            } => {
                if !doc.remove_item(path, value)? {
                    // edited or already gone: nothing of ours to take out
                    if let Some(Value::Array(a)) = doc.get(path) {
                        if !a.is_empty() && !force {
                            drift.push(format!(
                                "entry in {} was edited or removed",
                                edit::path_display(path)
                            ));
                        }
                    }
                }
                prune(doc.as_mut(), path, path.len(), *prune_to)?;
            }
            UndoOp::TextBlock { .. } => {}
        }
    }
    let empty = match doc.value() {
        Value::Object(m) => m.is_empty(),
        Value::Null => doc.text().trim().is_empty(),
        _ => false,
    };
    Ok((doc.text().to_string(), empty, drift))
}

/// Pieces of a merged file that no longer match what we wrote (read-only).
pub fn check_undo(format: DocFormat, text: &str, undo: &[UndoOp]) -> Vec<String> {
    let mut out = Vec::new();
    if format == DocFormat::Markdown {
        for u in undo {
            if let UndoOp::TextBlock { id, sha256 } = u {
                match textblock::content(text, id) {
                    Some(c) if sha256_hex(c.as_bytes()) == *sha256 => {}
                    Some(_) => out.push(format!("block {id} was edited")),
                    None => out.push(format!("block {id} is gone")),
                }
            }
        }
        return out;
    }
    let Ok(v) = edit::parse_value(format, text) else {
        return vec!["file no longer parses".into()];
    };
    for u in undo {
        match u {
            UndoOp::Remove { path, sha256, .. } | UndoOp::Restore { path, sha256, .. } => {
                match edit::value_at(&v, path) {
                    Some(x) if value_sha(x) == *sha256 => {}
                    Some(_) => out.push(format!("{} was edited", edit::path_display(path))),
                    None => out.push(format!("{} is gone", edit::path_display(path))),
                }
            }
            UndoOp::RemoveItem { path, value, .. } => {
                let present = edit::value_at(&v, path)
                    .and_then(|a| a.as_array())
                    .map(|a| a.contains(value))
                    .unwrap_or(false);
                if !present {
                    out.push(format!(
                        "entry in {} was edited or removed",
                        edit::path_display(path)
                    ));
                }
            }
            UndoOp::TextBlock { .. } => {}
        }
    }
    out
}

// ------------------------------------------------------------------ transactions

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub path: String,
    pub existed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    /// The path was a symlink to this target (restored as a link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// A folder link: restoring its absence removes the link or the copied folder.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxManifest {
    pub tx: String,
    pub started_at: String,
    pub kind: String,
    pub entries: Vec<BackupEntry>,
}

/// A backup transaction: every file is copied once before its first change.
pub struct Tx {
    pub id: String,
    dir: PathBuf,
    manifest: TxManifest,
}

impl Tx {
    pub fn begin(env: &Env, kind: &str) -> Result<Tx> {
        let id = format!(
            "{}-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let dir = env.agentkit_dir().join("backups").join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| AgentkitError::io("creating", &dir, &e))?;
        Ok(Tx {
            manifest: TxManifest {
                tx: id.clone(),
                started_at: now_iso(),
                kind: kind.to_string(),
                entries: vec![],
            },
            id,
            dir,
        })
    }

    /// Saves the current bytes of `path` (or its absence) once.
    pub fn backup(&mut self, path: &Path) -> Result<()> {
        let key = path.display().to_string();
        if self.manifest.entries.iter().any(|e| e.path == key) {
            return Ok(());
        }
        let n = self.manifest.entries.len();
        match std::fs::read(path) {
            Ok(bytes) => {
                let name = format!("{n:04}");
                let dst = self.dir.join(&name);
                std::fs::write(&dst, bytes)
                    .map_err(|e| AgentkitError::io("backing up", &dst, &e))?;
                self.manifest.entries.push(BackupEntry {
                    path: key,
                    existed: true,
                    backup: Some(name),
                    link: None,
                    folder: false,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.manifest.entries.push(BackupEntry {
                    path: key,
                    existed: false,
                    backup: None,
                    link: None,
                    folder: false,
                });
            }
            Err(e) => return Err(AgentkitError::io("reading", path, &e)),
        }
        self.save()
    }

    /// Records a folder-link path once: absent, or a symlink (its target kept).
    /// A real folder or file there is never replaced by the writer.
    pub fn backup_link(&mut self, path: &Path) -> Result<()> {
        let key = path.display().to_string();
        if self.manifest.entries.iter().any(|e| e.path == key) {
            return Ok(());
        }
        let link = std::fs::symlink_metadata(path)
            .ok()
            .filter(|m| m.file_type().is_symlink())
            .and_then(|_| std::fs::read_link(path).ok())
            .map(|t| t.display().to_string());
        self.manifest.entries.push(BackupEntry {
            path: key,
            existed: link.is_some(),
            backup: None,
            link,
            folder: true,
        });
        self.save()
    }

    fn save(&self) -> Result<()> {
        let p = self.dir.join("manifest.json");
        let bytes = serde_json::to_vec_pretty(&self.manifest).unwrap_or_default();
        std::fs::write(&p, bytes).map_err(|e| AgentkitError::io("writing", &p, &e))
    }
}

/// Writes through a temporary file in the same folder and renames it in place.
pub fn atomic_write(path: &Path, bytes: &[u8], executable: Option<bool>) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        AgentkitError::new("AGENTKIT_IO", format!("{} has no parent", path.display()))
    })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.omniget-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    {
        use std::io::Write;
        let mut f =
            std::fs::File::create(&tmp).map_err(|e| AgentkitError::io("creating", &tmp, &e))?;
        f.write_all(bytes)
            .map_err(|e| AgentkitError::io("writing", &tmp, &e))?;
        let _ = f.sync_all();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = match (executable, std::fs::metadata(path)) {
            (Some(true), _) => Some(0o755),
            (Some(false), Err(_)) => Some(0o644),
            (_, Ok(m)) => Some(m.permissions().mode() & 0o7777),
            (None, Err(_)) => Some(0o644),
        };
        if let Some(mode) = mode {
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
        }
    }
    #[cfg(not(unix))]
    let _ = executable;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AgentkitError::io("replacing", path, &e));
    }
    Ok(())
}

/// Creates the parent folders of `path`, returning the ones that did not exist
/// (outermost first).
pub(crate) fn ensure_parent(path: &Path) -> Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut cur = path.parent();
    while let Some(d) = cur {
        if d.as_os_str().is_empty() || d.exists() {
            break;
        }
        missing.push(d.to_path_buf());
        cur = d.parent();
    }
    missing.reverse();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| AgentkitError::io("creating", p, &e))?;
    }
    Ok(missing)
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AgentkitError::io("reading", path, &e)),
    }
}

fn save_lock(tx: &mut Tx, path: &Path, lock: &Lockfile) -> Result<()> {
    tx.backup(path)?;
    if lock.is_empty() {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| AgentkitError::io("removing", path, &e))?;
        }
        // `.omniget/` of a project goes too when nothing else is in it
        if let Some(dir) = path.parent() {
            if dir
                .file_name()
                .map(|n| n == lock::PROJECT_LOCK_DIR)
                .unwrap_or(false)
            {
                let _ = std::fs::remove_dir(dir);
            }
        }
        return Ok(());
    }
    ensure_parent(path)?;
    atomic_write(path, &lock.to_bytes(), Some(false))
}

// ------------------------------------------------------------------ folder links

thread_local! {
    static FORCE_COPY_LINKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests (this thread only): behave as on Windows without link permission
/// (copy instead of link).
pub fn set_force_copy_links(on: bool) {
    FORCE_COPY_LINKS.with(|c| c.set(on));
}

/// What a correct link to `to` looks like to the plan (compared with [`link_state`]).
pub fn link_bytes(to: &Path) -> Vec<u8> {
    format!("link:{}", to.display()).into_bytes()
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// `rel → sha256` of every file under `root` (`/`-separated).
fn dir_digest(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .flatten()
    {
        if !e.file_type().is_file() {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        if let Ok(b) = std::fs::read(e.path()) {
            out.insert(rel, sha256_hex(&b));
        }
    }
    out
}

/// State of a folder-link path: `link:<target>` for a symlink, the same for a
/// folder whose files equal `to`'s (a copy made where links are not allowed),
/// `dir:<hash>` for another folder, the bytes of a file, `None` when absent.
pub fn link_state(path: &Path, to: &Path) -> Option<Vec<u8>> {
    let md = std::fs::symlink_metadata(path).ok()?;
    if md.file_type().is_symlink() {
        let t = std::fs::read_link(path).ok()?;
        return Some(link_bytes(&t));
    }
    if md.is_dir() {
        let mine = dir_digest(path);
        if !mine.is_empty() && mine == dir_digest(to) {
            return Some(link_bytes(to));
        }
        let json = serde_json::to_string(&mine).unwrap_or_default();
        return Some(format!("dir:{}", sha256_hex(json.as_bytes())).into_bytes());
    }
    std::fs::read(path).ok()
}

/// Removes a symlink (file or folder link on every OS).
fn remove_link(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) => std::fs::remove_dir(path).map_err(|_| AgentkitError::io("removing", path, &e)),
    }
}

/// Copies the folder `from` to `to`; returns the files copied.
fn copy_dir(from: &Path, to: &Path) -> Result<Vec<lock::LinkedFile>> {
    std::fs::create_dir_all(to).map_err(|e| AgentkitError::io("creating", to, &e))?;
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(from)
        .follow_links(true)
        .into_iter()
        .flatten()
    {
        let Ok(rel) = e.path().strip_prefix(from) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dst = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&dst).map_err(|er| AgentkitError::io("creating", &dst, &er))?;
            continue;
        }
        let bytes =
            std::fs::read(e.path()).map_err(|er| AgentkitError::io("reading", e.path(), &er))?;
        #[cfg(unix)]
        let exec = {
            use std::os::unix::fs::PermissionsExt;
            e.metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        };
        #[cfg(not(unix))]
        let exec = false;
        atomic_write(&dst, &bytes, Some(exec))?;
        out.push(lock::LinkedFile {
            rel: rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/"),
            sha256: sha256_hex(&bytes),
        });
    }
    Ok(out)
}

/// Makes `path` a folder symlink to `to`, or a copy of it where links are not
/// allowed. Returns `(copied, files copied)`.
fn make_link(path: &Path, to: &Path) -> Result<(bool, Vec<lock::LinkedFile>)> {
    if !FORCE_COPY_LINKS.with(|c| c.get()) {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(to, path);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(to, path);
        #[cfg(not(any(unix, windows)))]
        let made: std::io::Result<()> = Err(std::io::Error::other("no symlinks"));
        if made.is_ok() {
            return Ok((false, vec![]));
        }
    }
    Ok((true, copy_dir(to, path)?))
}

/// Takes out a link we made (or the files of its copy). Returns drift notes.
fn remove_linked(
    tx: &mut Tx,
    path: &Path,
    to: &str,
    copied: bool,
    files: &[lock::LinkedFile],
    force: bool,
) -> Result<(bool, Vec<String>)> {
    let shown = path.display().to_string();
    if is_symlink(path) {
        let cur = std::fs::read_link(path)
            .map(|t| t.display().to_string())
            .unwrap_or_default();
        if cur != to && !force {
            return Ok((false, vec![format!("{shown} now links to {cur}; kept")]));
        }
        tx.backup_link(path)?;
        remove_link(path)?;
        return Ok((true, vec![]));
    }
    if !(copied && path.is_dir()) {
        return Ok((false, vec![format!("{shown} is no longer our link; kept")]));
    }
    let mut drift = Vec::new();
    for f in files {
        let p = f.rel.split('/').fold(path.to_path_buf(), |p, s| p.join(s));
        let Some(bytes) = read_opt(&p)? else {
            continue;
        };
        if sha256_hex(&bytes) != f.sha256 && !force {
            drift.push(format!("{} was edited; kept", p.display()));
            continue;
        }
        tx.backup(&p)?;
        std::fs::remove_file(&p).map_err(|e| AgentkitError::io("removing", &p, &e))?;
    }
    let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let _ = std::fs::remove_dir(&d);
    }
    Ok((!path.exists(), drift))
}

// ------------------------------------------------------------------ apply

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub tx: String,
    pub installed: Vec<InstallRecord>,
    pub files_written: Vec<String>,
    pub unchanged: Vec<String>,
    pub skipped_units: Vec<String>,
    pub notes: Vec<String>,
}

/// Applies a plan. Fails with `AGENTKIT_PLAN_STALE` when a file changed since the plan.
pub fn apply(env: &Env, plan: &InstallPlan) -> Result<ApplyReport> {
    // stale check first: nothing is touched if a file moved under us
    for f in &plan.files {
        let now = match &f.link_to {
            Some(to) => link_state(&f.path, to).map(|b| sha256_hex(&b)),
            None => read_opt(&f.path)?.map(|b| sha256_hex(&b)),
        };
        if now != f.before_sha {
            return Err(AgentkitError::new(
                "AGENTKIT_PLAN_STALE",
                format!(
                    "{} changed after the plan was made; plan again",
                    f.path.display()
                ),
            ));
        }
    }
    let mut tx = Tx::begin(env, "install")?;
    let mut report = ApplyReport {
        tx: tx.id.clone(),
        ..Default::default()
    };

    // updates: take the old install out first
    for u in &plan.units {
        if u.status == UnitStatus::Update {
            if let Some(old) = &u.replaces {
                let r = uninstall_in(env, &mut tx, old, plan.project_dir.as_deref(), false)?;
                report.notes.extend(r.drift);
            }
        }
    }

    // what other installs already own (after the updates above took theirs out)
    let prior = lock::all_locks(env, plan.project_dir.as_deref());
    let prior_created = |p: &str| prior.iter().any(|(_, l)| l.created_elsewhere(p, ""));
    let prior_pieces = |p: &str| -> Vec<UndoOp> {
        prior
            .iter()
            .flat_map(|(_, l)| l.merged_elsewhere(p, "").cloned().collect::<Vec<_>>())
            .collect()
    };

    let project_str = plan.project_dir.as_ref().map(|p| p.display().to_string());
    let mut records: BTreeMap<String, InstallRecord> = BTreeMap::new();
    for u in &plan.units {
        if !matches!(u.status, UnitStatus::New | UnitStatus::Update) {
            if u.status != UnitStatus::Installed {
                report.skipped_units.push(format!(
                    "{} → {}: {:?}",
                    u.component.name, u.target, u.status
                ));
            }
            continue;
        }
        records.insert(
            u.unit_id.clone(),
            InstallRecord {
                install_id: format!("ak-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
                component: u.component.clone(),
                target: u.target.clone(),
                scope: plan.scope,
                project_dir: project_str.clone(),
                installed_name: u.install_name.clone(),
                compat: u.compat.clone(),
                tx: tx.id.clone(),
                installed_at: now_iso(),
                files: vec![],
                created_dirs: vec![],
            },
        );
    }

    // one pass per path, in plan order
    for fp in &plan.files {
        let path = &fp.path;
        let contributions: Vec<(&str, &super::convert::PlannedFile)> = plan
            .units
            .iter()
            .filter(|u| records.contains_key(&u.unit_id))
            .flat_map(|u| {
                u.files
                    .iter()
                    .filter(|f| &f.path == path)
                    .map(move |f| (u.unit_id.as_str(), f))
            })
            .collect();
        if contributions.is_empty() {
            continue;
        }
        let links: Vec<_> = contributions
            .iter()
            .filter(|(_, f)| f.action == FileAction::Link && f.link_to.is_some())
            .collect();
        if !links.is_empty() {
            let (_, last) = links.last().unwrap();
            let to = last.link_to.clone().unwrap_or_default();
            let key = path.display().to_string();
            let desired = link_bytes(&to);
            let state = link_state(path, &to);
            let (copied, files) = if state.as_deref() == Some(desired.as_slice()) {
                report.unchanged.push(key.clone());
                if !(fp.owned_by_update || prior_created(&key)) {
                    // the same link was already there and is not ours
                    continue;
                }
                let copied = !is_symlink(path);
                let files = if copied {
                    dir_digest(path)
                        .into_iter()
                        .map(|(rel, sha256)| lock::LinkedFile { rel, sha256 })
                        .collect()
                } else {
                    vec![]
                };
                (copied, files)
            } else {
                if state.is_some() && !is_symlink(path) {
                    report.notes.push(format!(
                        "{key} exists and is not a link of ours; left as is"
                    ));
                    continue;
                }
                tx.backup_link(path)?;
                if is_symlink(path) {
                    remove_link(path)?;
                }
                let created = ensure_parent(path)?;
                let made = make_link(path, &to)?;
                report.files_written.push(key.clone());
                for (uid, _) in &links {
                    if let Some(r) = records.get_mut(*uid) {
                        r.created_dirs
                            .extend(created.iter().map(|d| d.display().to_string()));
                    }
                }
                made
            };
            for (uid, _) in &links {
                if let Some(r) = records.get_mut(*uid) {
                    r.files.push(WrittenFile {
                        path: key.clone(),
                        action: WrittenAction::Linked {
                            to: to.display().to_string(),
                            copied,
                            files: files.clone(),
                        },
                    });
                }
            }
            continue;
        }
        let current = read_opt(path)?;
        let existed = current.is_some();
        let writes: Vec<_> = contributions
            .iter()
            .filter(|(_, f)| f.action == FileAction::Write)
            .collect();
        if !writes.is_empty() {
            let (_, last) = writes.last().unwrap();
            let bytes = last.content.clone().unwrap_or_default();
            let sha = sha256_hex(&bytes);
            let same = current.as_deref() == Some(bytes.as_slice());
            if !same {
                tx.backup(path)?;
                let created = ensure_parent(path)?;
                atomic_write(path, &bytes, Some(last.executable))?;
                report.files_written.push(path.display().to_string());
                for (uid, _) in &writes {
                    if let Some(r) = records.get_mut(*uid) {
                        r.created_dirs
                            .extend(created.iter().map(|d| d.display().to_string()));
                    }
                }
            } else {
                report.unchanged.push(path.display().to_string());
            }
            // a file identical to ours but not written by us stays the user's
            // (unless another install of ours wrote it: then it is co-owned)
            let ours = !same
                || !existed
                || fp.owned_by_update
                || prior_created(&fp.path.display().to_string());
            if ours {
                for (uid, f) in &writes {
                    if let Some(r) = records.get_mut(*uid) {
                        r.files.push(WrittenFile {
                            path: path.display().to_string(),
                            action: WrittenAction::Created {
                                sha256: sha.clone(),
                                executable: f.executable,
                            },
                        });
                    }
                }
            }
            continue;
        }
        // merges, possibly from several units
        let format = contributions[0].1.format.unwrap_or(DocFormat::Json);
        let mut text = current
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        let mut changed = false;
        let mut per_unit: Vec<(&str, Vec<UndoOp>)> = Vec::new();
        let mut owners: Vec<UndoOp> = prior_pieces(&path.display().to_string());
        for (uid, f) in &contributions {
            let a = apply_ops(format, &text, &f.ops)?;
            if a.text != text {
                changed = true;
            }
            text = a.text;
            report.notes.extend(a.notes);
            let mut undo = a.undo;
            // a piece already there that another install of ours owns: hold it
            // too, so it leaves with its last holder
            for cand in a.shared {
                if let Some(own) = owners.iter().find(|o| o.same_piece(&cand)) {
                    if !undo.iter().any(|u| u.same_piece(&cand)) {
                        undo.push(own.clone());
                    }
                }
            }
            owners.extend(undo.iter().cloned());
            per_unit.push((uid, undo));
        }
        if changed {
            tx.backup(path)?;
            let created = ensure_parent(path)?;
            atomic_write(path, text.as_bytes(), None)?;
            report.files_written.push(path.display().to_string());
            for (uid, _) in &per_unit {
                if let Some(r) = records.get_mut(*uid) {
                    r.created_dirs
                        .extend(created.iter().map(|d| d.display().to_string()));
                }
            }
        } else {
            report.unchanged.push(path.display().to_string());
        }
        let mut first_creator = !existed;
        for (uid, undo) in per_unit {
            if undo.is_empty() {
                continue;
            }
            if let Some(r) = records.get_mut(uid) {
                r.files.push(WrittenFile {
                    path: path.display().to_string(),
                    action: WrittenAction::Merged {
                        format,
                        created_file: first_creator,
                        undo,
                    },
                });
                first_creator = false;
            }
        }
    }

    // locks
    let (lock_path, is_project) = (
        lock::lock_path(env, plan.scope, plan.project_dir.as_deref())?,
        plan.scope.is_project_bound(),
    );
    let mut lockf = Lockfile::load(&lock_path)?;
    lockf.version = lock::LOCK_VERSION;
    for (_, r) in records {
        if r.files.is_empty() {
            report.notes.push(format!(
                "{} → {}: nothing new to write",
                r.component.name, r.target
            ));
            continue;
        }
        lockf.installs.retain(|x| {
            !(x.component.id == r.component.id
                && x.target == r.target
                && x.scope == r.scope
                && x.project_dir == r.project_dir)
        });
        report.installed.push(r.clone());
        lockf.installs.push(r);
    }
    save_lock(&mut tx, &lock_path, &lockf)?;
    if is_project {
        if let Some(p) = &plan.project_dir {
            let gp = lock::global_lock_path(env);
            let mut g = Lockfile::load(&gp)?;
            let key = p.display().to_string();
            let has = lockf.installs.iter().any(|_| true);
            if has && !g.projects.contains(&key) {
                g.version = lock::LOCK_VERSION;
                g.projects.push(key);
                save_lock(&mut tx, &gp, &g)?;
            }
        }
    }
    Ok(report)
}

// ------------------------------------------------------------------ uninstall / drift / restore

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UninstallReport {
    pub tx: String,
    pub install_id: String,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    pub drift: Vec<String>,
}

fn find_record(
    env: &Env,
    install_id: &str,
    project: Option<&Path>,
) -> Result<(PathBuf, InstallRecord)> {
    for (p, l) in lock::all_locks(env, project) {
        if let Some(r) = l.by_id(install_id) {
            return Ok((p, r.clone()));
        }
    }
    Err(AgentkitError::new(
        "AGENTKIT_NOT_INSTALLED",
        format!("install `{install_id}` is not in any lockfile"),
    ))
}

/// Removes an install: only files and pieces we wrote; edited pieces are kept
/// and reported unless `force`.
pub fn uninstall(
    env: &Env,
    install_id: &str,
    project: Option<&Path>,
    force: bool,
) -> Result<UninstallReport> {
    let mut tx = Tx::begin(env, "uninstall")?;
    let mut r = uninstall_in(env, &mut tx, install_id, project, force)?;
    r.tx = tx.id.clone();
    Ok(r)
}

fn uninstall_in(
    env: &Env,
    tx: &mut Tx,
    install_id: &str,
    project: Option<&Path>,
    force: bool,
) -> Result<UninstallReport> {
    let (lock_path, rec) = find_record(env, install_id, project)?;
    let mut report = UninstallReport {
        install_id: install_id.to_string(),
        tx: tx.id.clone(),
        ..Default::default()
    };
    // every lock, mutable: shared pieces pass to the installs that stay
    let mut locks = lock::all_locks(env, project);
    if !locks.iter().any(|(p, _)| *p == lock_path) {
        locks.push((lock_path.clone(), Lockfile::load(&lock_path)?));
    }
    let mut dirty: Vec<PathBuf> = vec![lock_path.clone()];
    for f in rec.files.iter().rev() {
        let path = PathBuf::from(&f.path);
        if let WrittenAction::Linked { to, copied, files } = &f.action {
            if std::fs::symlink_metadata(&path).is_err() {
                continue;
            }
            if locks
                .iter()
                .any(|(_, l)| l.created_elsewhere(&f.path, install_id))
            {
                report
                    .kept
                    .push(format!("{} (also installed by another component)", f.path));
                continue;
            }
            let (gone, drift) = remove_linked(tx, &path, to, *copied, files, force)?;
            if gone {
                report.removed.push(f.path.clone());
            } else {
                report.kept.push(f.path.clone());
            }
            report.drift.extend(drift);
            continue;
        }
        let Some(bytes) = read_opt(&path)? else {
            continue;
        };
        match &f.action {
            WrittenAction::Created { sha256, .. } => {
                if locks
                    .iter()
                    .any(|(_, l)| l.created_elsewhere(&f.path, install_id))
                {
                    report
                        .kept
                        .push(format!("{} (also installed by another component)", f.path));
                    continue;
                }
                if sha256_hex(&bytes) != *sha256 && !force {
                    report.drift.push(format!("{} was edited; kept", f.path));
                    report.kept.push(f.path.clone());
                    continue;
                }
                tx.backup(&path)?;
                std::fs::remove_file(&path)
                    .map_err(|e| AgentkitError::io("removing", &path, &e))?;
                report.removed.push(f.path.clone());
            }
            WrittenAction::Merged {
                format,
                created_file,
                undo,
            } => {
                // pieces another install still holds stay (reference count)
                let held: Vec<UndoOp> = locks
                    .iter()
                    .flat_map(|(_, l)| {
                        l.merged_elsewhere(&f.path, install_id)
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let mine: Vec<UndoOp> = undo
                    .iter()
                    .filter(|u| !held.iter().any(|h| h.same_piece(u)))
                    .cloned()
                    .collect();
                let text = String::from_utf8_lossy(&bytes).to_string();
                let (new_text, empty, drift) = undo_ops(*format, &text, &mine, force)?;
                report
                    .drift
                    .extend(drift.into_iter().map(|d| format!("{}: {d}", f.path)));
                let heirs = !held.is_empty();
                if *created_file && empty && !heirs {
                    tx.backup(&path)?;
                    std::fs::remove_file(&path)
                        .map_err(|e| AgentkitError::io("removing", &path, &e))?;
                    report.removed.push(f.path.clone());
                } else if new_text != text {
                    tx.backup(&path)?;
                    atomic_write(&path, new_text.as_bytes(), None)?;
                    report.removed.push(format!("{} (our part)", f.path));
                }
                if heirs {
                    for (lp, l) in locks.iter_mut() {
                        if hand_over(l, install_id, &f.path, *created_file, undo)
                            && !dirty.contains(lp)
                        {
                            dirty.push(lp.clone());
                        }
                    }
                }
            }
            WrittenAction::Linked { .. } => {}
        }
    }
    let mut dirs: Vec<PathBuf> = rec.created_dirs.iter().map(PathBuf::from).collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    let mut kept_dirs: Vec<String> = Vec::new();
    for d in dirs {
        // only when empty; a folder shared with other files stays
        if std::fs::read_dir(&d)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&d);
        } else if d.is_dir() {
            kept_dirs.push(d.display().to_string());
        }
    }
    // (k1-convert) a folder we created that still holds another install's files
    // passes to that install, so the last one out removes it (`.gemini/` holding
    // both an agent and a command installed as two units).
    for d in kept_dirs {
        'outer: for (lp, l) in locks.iter_mut() {
            if let Some(heir) = l.installs.iter_mut().find(|x| {
                x.install_id != install_id
                    && x.files.iter().any(|f| Path::new(&f.path).starts_with(&d))
            }) {
                if !heir.created_dirs.contains(&d) {
                    heir.created_dirs.push(d.clone());
                    if !dirty.contains(lp) {
                        dirty.push(lp.clone());
                    }
                }
                break 'outer;
            }
        }
    }
    let mut project_lock_emptied = false;
    for (lp, l) in locks.iter_mut() {
        if *lp == lock_path {
            l.installs.retain(|x| x.install_id != install_id);
            project_lock_emptied = l.installs.is_empty();
        }
    }
    for (lp, l) in &locks {
        if dirty.contains(lp) {
            save_lock(tx, lp, l)?;
        }
    }
    if project_lock_emptied
        && lock_path.ends_with(Path::new(lock::PROJECT_LOCK_DIR).join(lock::PROJECT_LOCK))
    {
        let gp = lock::global_lock_path(env);
        let mut g = Lockfile::load(&gp)?;
        if let Some(p) = rec.project_dir.as_ref() {
            let before = g.projects.len();
            g.projects.retain(|x| x != p);
            if g.projects.len() != before {
                save_lock(tx, &gp, &g)?;
            }
        }
    }
    Ok(report)
}

/// Depth of the shallowest node an undo op created (`prune_to` + 1), and its path.
fn created_root(u: &UndoOp) -> Option<(&[Seg], usize)> {
    match u {
        UndoOp::Remove { path, prune_to, .. } | UndoOp::RemoveItem { path, prune_to, .. } => {
            Some((path.as_slice(), *prune_to))
        }
        _ => None,
    }
}

/// Hands what the leaving install `gone` created in `path` to the installs of
/// `l` that stay there: the "file created by us" flag (so the last one out
/// deletes an emptied file) and the containers it created above their pieces
/// (so the last one out prunes them). Returns whether `l` changed.
fn hand_over(
    l: &mut Lockfile,
    gone: &str,
    path: &str,
    created_file: bool,
    undo: &[UndoOp],
) -> bool {
    let mut changed = false;
    let mut flag_given = !created_file;
    for r in l.installs.iter_mut().filter(|r| r.install_id != gone) {
        for wf in r.files.iter_mut().filter(|w| w.path == path) {
            let WrittenAction::Merged {
                created_file: cf,
                undo: heir_undo,
                ..
            } = &mut wf.action
            else {
                continue;
            };
            if !flag_given {
                if !*cf {
                    *cf = true;
                    changed = true;
                }
                flag_given = true;
            }
            for h in heir_undo.iter_mut() {
                let (q, q_prune) = match h {
                    UndoOp::Remove { path, prune_to, .. }
                    | UndoOp::RemoveItem { path, prune_to, .. } => (path.clone(), prune_to),
                    _ => continue,
                };
                for u in undo {
                    let Some((p, p_prune)) = created_root(u) else {
                        continue;
                    };
                    let common = p.iter().zip(q.iter()).take_while(|(a, b)| a == b).count();
                    if common > p_prune && *q_prune > p_prune {
                        *q_prune = p_prune;
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftItem {
    pub install_id: String,
    pub component: String,
    pub target: String,
    pub path: String,
    /// `missing | modified | piece_changed`.
    pub state: String,
    pub detail: Vec<String>,
}

/// Files/pieces that no longer match what we wrote.
pub fn drift(env: &Env, project: Option<&Path>) -> Result<Vec<DriftItem>> {
    let mut out = Vec::new();
    for (_, l) in lock::all_locks(env, project) {
        for r in &l.installs {
            for f in &r.files {
                let path = PathBuf::from(&f.path);
                let item = |state: &str, detail: Vec<String>| DriftItem {
                    install_id: r.install_id.clone(),
                    component: r.component.name.clone(),
                    target: r.target.clone(),
                    path: f.path.clone(),
                    state: state.to_string(),
                    detail,
                };
                if let WrittenAction::Linked { to, copied, files } = &f.action {
                    if std::fs::symlink_metadata(&path).is_err() {
                        out.push(item("missing", vec![]));
                    } else if is_symlink(&path) {
                        let cur = std::fs::read_link(&path)
                            .map(|t| t.display().to_string())
                            .unwrap_or_default();
                        if cur != *to {
                            out.push(item("modified", vec![format!("links to {cur}")]));
                        }
                    } else if *copied {
                        let now = dir_digest(&path);
                        let d: Vec<String> = files
                            .iter()
                            .filter(|x| now.get(&x.rel) != Some(&x.sha256))
                            .map(|x| format!("{} was edited or removed", x.rel))
                            .collect();
                        if !d.is_empty() {
                            out.push(item("piece_changed", d));
                        }
                    } else {
                        out.push(item("modified", vec!["no longer a link".into()]));
                    }
                    continue;
                }
                let Some(bytes) = read_opt(&path)? else {
                    out.push(item("missing", vec![]));
                    continue;
                };
                match &f.action {
                    WrittenAction::Created { sha256, .. } => {
                        if sha256_hex(&bytes) != *sha256 {
                            out.push(item("modified", vec![]));
                        }
                    }
                    WrittenAction::Merged { format, undo, .. } => {
                        let d = check_undo(*format, &String::from_utf8_lossy(&bytes), undo);
                        if !d.is_empty() {
                            out.push(item("piece_changed", d));
                        }
                    }
                    WrittenAction::Linked { .. } => {}
                }
            }
        }
    }
    Ok(out)
}

/// Every install we know of (global + project).
pub fn installed(env: &Env, project: Option<&Path>) -> Vec<InstallRecord> {
    let mut out = Vec::new();
    for (_, l) in lock::all_locks(env, project) {
        for r in l.installs {
            if !out
                .iter()
                .any(|x: &InstallRecord| x.install_id == r.install_id)
            {
                out.push(r);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreReport {
    pub tx: String,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
}

/// Rolls a transaction back: every file it touched returns to its backup (or is
/// deleted when it did not exist), lockfiles included.
pub fn restore(env: &Env, tx_id: &str) -> Result<RestoreReport> {
    if tx_id.contains('/') || tx_id.contains('\\') || tx_id.contains("..") {
        return Err(AgentkitError::new("AGENTKIT_TX", "bad transaction id"));
    }
    let dir = env.agentkit_dir().join("backups").join(tx_id);
    let mp = dir.join("manifest.json");
    let bytes = std::fs::read(&mp).map_err(|e| AgentkitError::io("reading", &mp, &e))?;
    let m: TxManifest = serde_json::from_slice(&bytes)
        .map_err(|e| AgentkitError::new("AGENTKIT_TX", e.to_string()))?;
    let mut report = RestoreReport {
        tx: tx_id.to_string(),
        ..Default::default()
    };
    for e in m.entries.iter().rev() {
        let path = PathBuf::from(&e.path);
        if e.folder {
            // a folder link: take out what is there now, put the old link back
            if is_symlink(&path) {
                remove_link(&path)?;
                report.removed.push(e.path.clone());
            } else if path.is_dir() {
                std::fs::remove_dir_all(&path)
                    .map_err(|er| AgentkitError::io("removing", &path, &er))?;
                report.removed.push(e.path.clone());
            }
            if let Some(prev) = &e.link {
                ensure_parent(&path)?;
                #[cfg(unix)]
                let _ = std::os::unix::fs::symlink(prev, &path);
                #[cfg(windows)]
                let _ = std::os::windows::fs::symlink_dir(prev, &path);
                report.restored.push(e.path.clone());
            }
            continue;
        }
        match (&e.existed, &e.backup) {
            (true, Some(b)) => {
                let data = std::fs::read(dir.join(b))
                    .map_err(|er| AgentkitError::io("reading", &dir.join(b), &er))?;
                ensure_parent(&path)?;
                atomic_write(&path, &data, None)?;
                report.restored.push(e.path.clone());
            }
            _ => {
                if path.exists() {
                    std::fs::remove_file(&path)
                        .map_err(|er| AgentkitError::io("removing", &path, &er))?;
                    report.removed.push(e.path.clone());
                }
            }
        }
    }
    Ok(report)
}

/// Transactions on disk, newest first.
pub fn transactions(env: &Env) -> Vec<TxManifest> {
    let dir = env.agentkit_dir().join("backups");
    let mut out: Vec<TxManifest> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read(e.path().join("manifest.json")).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect();
    out.sort_by(|a, b| b.tx.cmp(&a.tx));
    out
}

/// Scope of a record, for callers that only have the id.
pub fn record(env: &Env, install_id: &str, project: Option<&Path>) -> Result<InstallRecord> {
    find_record(env, install_id, project).map(|(_, r)| r)
}
