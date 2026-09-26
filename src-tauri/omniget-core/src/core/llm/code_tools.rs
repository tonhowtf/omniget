//! Coding tools of the native harness: read, list, glob, grep, edit, write,
//! apply_patch, shell and a plan. Design approved in the 2026-09-18 brainstorm
//! (plan §10): every tool is bound to ONE workspace root, paths never leave it,
//! `.git` is write-protected, outputs are capped, and on macOS the shell runs
//! under a seatbelt profile that allows writes only inside the workspace and
//! denies the network.
//!
//! Lineage, read from source: the read/list/glob/grep/edit shapes and caps
//! follow opencode (`packages/opencode/src/tool/*`), the patch envelope is the
//! Codex / aider "V4A" one with aider's context fuzz ladder (exact, rstrip,
//! strip), and `todo_write` is Codex's `update_plan`.

use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use serde_json::{json, Value};

const READ_LIMIT_LINES: usize = 2000;
const LINE_MAX_CHARS: usize = 2000;
const OUT_MAX: usize = 50 * 1024;
const LIST_MAX: usize = 1000;
const GLOB_MAX: usize = 500;
const GREP_MAX: usize = 100;
const SHELL_TIMEOUT_MS: u64 = 120_000;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".svelte-kit",
];

pub const ERR_NO_WORKSPACE: &str = "ERR_CODE_NO_WORKSPACE";
pub const ERR_OUTSIDE: &str = "ERR_CODE_OUTSIDE_WORKSPACE";
pub const ERR_PROTECTED: &str = "ERR_CODE_PROTECTED_PATH";
pub const ERR_EDIT_NO_MATCH: &str = "ERR_CODE_EDIT_NO_MATCH";
pub const ERR_EDIT_AMBIGUOUS: &str = "ERR_CODE_EDIT_AMBIGUOUS";
pub const ERR_PATCH: &str = "ERR_CODE_PATCH";

/// Names of the tools in this module, for default grants and `TaskKind::Code`.
pub const READ_TOOLS: &[&str] = &["fs_read", "fs_list", "fs_glob", "fs_grep", "todo_write"];
pub const WRITE_TOOLS: &[&str] = &["fs_edit", "fs_write", "fs_apply_patch", "shell_exec"];

static WORKSPACE: RwLock<Option<PathBuf>> = RwLock::new(None);
static PLAN: RwLock<Option<Value>> = RwLock::new(None);
static BY_CONVERSATION: RwLock<Option<std::collections::HashMap<String, PathBuf>>> =
    RwLock::new(None);
static STORE_FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// What a tool call knows about the turn it runs in. The coordinator opens the
/// scope around every brokered call; outside a turn (the embedded MCP server,
/// the CLI) the tools fall back to the process-wide workspace.
pub struct TurnCtx {
    pub conversation: String,
    pub agent: String,
    pub request: String,
    external_ok: std::sync::atomic::AtomicBool,
}

tokio::task_local! {
    static TURN: std::sync::Arc<TurnCtx>;
}

pub async fn scope<F: std::future::Future>(
    conversation: &str,
    agent: &str,
    request: &str,
    fut: F,
) -> F::Output {
    let ctx = std::sync::Arc::new(TurnCtx {
        conversation: conversation.to_string(),
        agent: agent.to_string(),
        request: request.to_string(),
        external_ok: std::sync::atomic::AtomicBool::new(false),
    });
    TURN.scope(ctx, fut).await
}

pub fn current_turn() -> Option<std::sync::Arc<TurnCtx>> {
    TURN.try_with(|c| c.clone()).ok()
}

tokio::task_local! {
    static TOOL_CALL_ID: String;
}

/// Add the broker's real call id without changing the enclosing turn context.
pub async fn scope_tool_call<F: std::future::Future>(id: &str, fut: F) -> F::Output {
    TOOL_CALL_ID.scope(id.to_owned(), fut).await
}

pub fn current_tool_call() -> Option<String> {
    TOOL_CALL_ID.try_with(Clone::clone).ok()
}

/// The broker calls this after the user answered an `external_directory` ask:
/// the next path check of this call may leave the workspace, once.
pub fn allow_external_once() {
    if let Some(ctx) = current_turn() {
        ctx.external_ok
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn take_external() -> bool {
    current_turn()
        .map(|c| {
            c.external_ok
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        })
        .unwrap_or(false)
}

/// The canonical store of "which folder belongs to which conversation".
/// `assist::groups` installs one backed by `assist.db` at boot; without it
/// (tests, tools that never booted the assistant) the legacy in-memory map
/// loaded from `workspaces.json` answers.
pub trait WorkspaceBindings: Send + Sync {
    /// The folder authorised for `conversation`, `None` when it is personal.
    fn get(&self, conversation: &str) -> Option<PathBuf>;
    /// Binds (`Some`, already canonical) or detaches (`None`) one conversation.
    fn set(&self, conversation: &str, path: Option<&Path>) -> Result<(), String>;
    /// Called with the legacy map when `workspaces.json` is loaded, so old
    /// per-conversation folders survive the move (idempotent, never deletes).
    fn import_legacy(&self, _map: &std::collections::HashMap<String, PathBuf>) {}
}

static BINDINGS: RwLock<Option<std::sync::Arc<dyn WorkspaceBindings>>> = RwLock::new(None);

/// Installs the canonical binding store and hands it the legacy map.
pub fn set_bindings(bindings: Option<std::sync::Arc<dyn WorkspaceBindings>>) {
    if let Some(b) = &bindings {
        b.import_legacy(&legacy_bindings());
    }
    *BINDINGS.write().unwrap_or_else(|e| e.into_inner()) = bindings;
}

fn bindings() -> Option<std::sync::Arc<dyn WorkspaceBindings>> {
    BINDINGS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The per-conversation folders read from `workspaces.json` (legacy store).
pub fn legacy_bindings() -> std::collections::HashMap<String, PathBuf> {
    BY_CONVERSATION
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Where the legacy conversation → workspace map is persisted (the app sets
/// it once). With a [`WorkspaceBindings`] installed the file is only read,
/// to import what older versions wrote; it is never deleted.
pub fn set_store_file(path: PathBuf) {
    let loaded: std::collections::HashMap<String, PathBuf> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if let Some(b) = bindings() {
        b.import_legacy(&loaded);
    }
    *BY_CONVERSATION.write().unwrap_or_else(|e| e.into_inner()) = Some(loaded);
    *STORE_FILE.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

fn canonical_dir(p: PathBuf) -> Result<PathBuf, String> {
    let real = p
        .canonicalize()
        .map_err(|e| format!("{ERR_NO_WORKSPACE}: {e}"))?;
    if !real.is_dir() {
        return Err(format!("{ERR_NO_WORKSPACE}: not a directory"));
    }
    Ok(real)
}

/// Bind one conversation to a folder (it becomes a Project conversation).
/// `None` detaches it (personal again). Never touches another conversation
/// nor the process-wide folder.
pub fn set_conversation_workspace(
    conversation: &str,
    path: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let resolved = path.map(canonical_dir).transpose()?;
    if let Some(b) = bindings() {
        b.set(conversation, resolved.as_deref())?;
        return Ok(resolved);
    }
    let mut guard = BY_CONVERSATION.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    match &resolved {
        Some(p) => {
            map.insert(conversation.to_string(), p.clone());
        }
        None => {
            map.remove(conversation);
        }
    }
    if let Some(file) = STORE_FILE.read().unwrap_or_else(|e| e.into_inner()).clone() {
        if let Ok(bytes) = serde_json::to_vec_pretty(&*map) {
            let _ = std::fs::write(file, bytes);
        }
    }
    Ok(resolved)
}

/// The folder authorised for one conversation, or `None` for a personal
/// (projectless) one. There is deliberately NO fallback to the process-wide
/// folder: a personal chat must never inherit the last project somebody
/// opened elsewhere (A04), and two conversations keep their own folders (A05).
pub fn workspace_of(conversation: &str) -> Option<PathBuf> {
    let found = match bindings() {
        Some(b) => b.get(conversation),
        None => BY_CONVERSATION
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(conversation).cloned()),
    };
    found.filter(|p| p.is_dir())
}

/// True when `raw` is a path the tool would resolve outside the workspace.
pub fn is_external(raw: &str) -> bool {
    let Some(root) = workspace() else {
        return false;
    };
    let p = Path::new(raw.trim());
    p.is_absolute() && !p.starts_with(&root)
}

/// The folder every coding tool is confined to. `None` disables them all.
pub fn set_workspace(path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    let resolved = match path {
        Some(p) => {
            let real = p
                .canonicalize()
                .map_err(|e| format!("{ERR_NO_WORKSPACE}: {e}"))?;
            if !real.is_dir() {
                return Err(format!("{ERR_NO_WORKSPACE}: not a directory"));
            }
            Some(real)
        }
        None => None,
    };
    *WORKSPACE.write().unwrap_or_else(|e| e.into_inner()) = resolved.clone();
    Ok(resolved)
}

fn global_workspace() -> Option<PathBuf> {
    WORKSPACE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Inside a turn: the folder of that turn's conversation, or `None` for a
/// personal conversation. Outside a turn (the embedded MCP server, the `omniget`
/// CLI, background checks): the process-wide folder, which only those callers
/// use and which choosing a folder in a conversation never changes.
pub fn workspace() -> Option<PathBuf> {
    match current_turn() {
        Some(ctx) => workspace_of(&ctx.conversation),
        None => global_workspace(),
    }
}

// ── one writer per workspace (B06) ──────────────────────────────────────
//
// Two conversations (or two members of a room) bound to the same folder must
// not interleave writes. Every write tool takes the folder's lock for the
// whole call: `fs_edit` / `fs_write` / `fs_apply_patch` hold it for the
// milliseconds of the write, `shell_exec` for the whole command. The key is
// the canonical folder, so two spellings of one path share one lock.

struct WsLock {
    busy: std::sync::Mutex<bool>,
    cv: std::sync::Condvar,
    notify: tokio::sync::Notify,
}

static WRITE_LOCKS: std::sync::Mutex<
    Option<std::collections::HashMap<PathBuf, std::sync::Arc<WsLock>>>,
> = std::sync::Mutex::new(None);

pub const ERR_WORKSPACE_BUSY: &str = "ERR_CODE_WORKSPACE_BUSY";

fn lock_for(root: &Path) -> std::sync::Arc<WsLock> {
    let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut g = WRITE_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default)
        .entry(key)
        .or_insert_with(|| {
            std::sync::Arc::new(WsLock {
                busy: std::sync::Mutex::new(false),
                cv: std::sync::Condvar::new(),
                notify: tokio::sync::Notify::new(),
            })
        })
        .clone()
}

/// Held while one write tool runs in a folder; dropping it lets the next in.
pub struct WriteGuard(std::sync::Arc<WsLock>);

impl Drop for WriteGuard {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.0.cv.notify_all();
        self.0.notify.notify_waiters();
    }
}

fn try_take(lock: &std::sync::Arc<WsLock>) -> Option<WriteGuard> {
    let mut busy = lock.busy.lock().unwrap_or_else(|e| e.into_inner());
    if *busy {
        return None;
    }
    *busy = true;
    Some(WriteGuard(lock.clone()))
}

/// Waits for the folder's write lock without blocking the runtime.
pub async fn write_lock(root: &Path) -> WriteGuard {
    let lock = lock_for(root);
    loop {
        let notified = lock.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(g) = try_take(&lock) {
            return g;
        }
        notified.await;
    }
}

/// The same lock for the synchronous write tools. On a multi-thread runtime
/// the wait moves off the worker (`block_in_place`); on a current-thread
/// runtime a wait could deadlock the holder, so a busy folder answers
/// `ERR_CODE_WORKSPACE_BUSY` instead of blocking.
pub fn write_lock_blocking(root: &Path) -> Result<WriteGuard, String> {
    let lock = lock_for(root);
    if let Some(g) = try_take(&lock) {
        return Ok(g);
    }
    let wait = || {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(600_000 + 5_000);
        let mut busy = lock.busy.lock().unwrap_or_else(|e| e.into_inner());
        while *busy {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "{ERR_WORKSPACE_BUSY}: another agent is still writing here"
                ));
            }
            busy = lock
                .cv
                .wait_timeout(busy, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *busy = true;
        Ok(WriteGuard(lock.clone()))
    };
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::CurrentThread) => Err(format!(
            "{ERR_WORKSPACE_BUSY}: another agent is writing in this folder; try again"
        )),
        Ok(_) => tokio::task::block_in_place(wait),
        Err(_) => wait(),
    }
}

pub fn current_plan() -> Option<Value> {
    PLAN.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn root() -> Result<PathBuf, String> {
    workspace().ok_or_else(|| format!("{ERR_NO_WORKSPACE}: pick a workspace folder first"))
}

/// Lexical join that can never leave `root`, then a symlink check on whatever
/// already exists along the way.
fn join_inside(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let rel = Path::new(raw.trim());
    let rel = rel.strip_prefix(root).unwrap_or(rel);
    if rel.is_absolute() {
        // The user said yes to an `external_directory` ask for this call.
        if take_external() && !rel.components().any(|c| matches!(c, Component::ParentDir)) {
            return Ok(rel.to_path_buf());
        }
        return Err(format!("{ERR_OUTSIDE}: {raw}"));
    }
    let mut out = root.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            _ => return Err(format!("{ERR_OUTSIDE}: {raw}")),
        }
    }
    let mut probe = out.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    if let Ok(real) = probe.canonicalize() {
        if !real.starts_with(root) {
            return Err(format!(
                "{ERR_OUTSIDE}: {raw} resolves outside the workspace"
            ));
        }
    }
    Ok(out)
}

fn writable(root: &Path, path: &Path) -> Result<(), String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    if rel.components().any(|c| c.as_os_str() == ".git") {
        return Err(format!("{ERR_PROTECTED}: .git is read-only for agents"));
    }
    Ok(())
}

fn rel_str(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn cap(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n… [truncated]");
    }
    s
}

fn arg_str(a: &Value, k: &str) -> String {
    a.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn arg_u(a: &Value, k: &str) -> Option<usize> {
    a.get(k).and_then(Value::as_u64).map(|v| v as usize)
}

// ── read / list / glob / grep ────────────────────────────────────────────

pub fn fs_read(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let path = join_inside(&root, &arg_str(a, "path"))?;
    if path.is_dir() {
        return fs_list(a);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", rel_str(&root, &path)))?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(format!("{}: binary file", rel_str(&root, &path)));
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = arg_u(a, "offset").unwrap_or(1).max(1);
    let limit = arg_u(a, "limit")
        .unwrap_or(READ_LIMIT_LINES)
        .min(READ_LIMIT_LINES);
    let total = text.lines().count();
    let mut out = String::new();
    let mut last = offset - 1;
    for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
        let shown: String = line.chars().take(LINE_MAX_CHARS).collect();
        out.push_str(&format!("{:>6}\t{}\n", i + 1, shown));
        last = i + 1;
        if out.len() > OUT_MAX {
            break;
        }
    }
    if last < total {
        out.push_str(&format!(
            "\n[{total} lines total. Use offset={} to continue]",
            last + 1
        ));
    }
    Ok(json!({ "path": rel_str(&root, &path), "lines": total, "content": out }))
}

fn walk(base: &Path, depth: usize) -> impl Iterator<Item = walkdir::DirEntry> {
    walkdir::WalkDir::new(base)
        .max_depth(depth)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !e
                    .file_name()
                    .to_str()
                    .map(|n| SKIP_DIRS.contains(&n))
                    .unwrap_or(false)
        })
        .filter_map(Result::ok)
}

pub fn fs_list(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let raw = arg_str(a, "path");
    let base = join_inside(&root, if raw.is_empty() { "." } else { &raw })?;
    let depth = arg_u(a, "depth").unwrap_or(2).clamp(1, 8);
    let mut entries = Vec::new();
    for e in walk(&base, depth).skip(1).take(LIST_MAX) {
        let mut s = rel_str(&root, e.path());
        if e.file_type().is_dir() {
            s.push('/');
        }
        entries.push(s);
    }
    Ok(json!({ "path": rel_str(&root, &base), "entries": entries }))
}

fn glob_to_regex(pattern: &str) -> Result<regex::Regex, String> {
    let mut re = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                re.push_str(".*");
                i += 1;
                if chars.get(i + 1) == Some(&'/') {
                    i += 1;
                }
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => re.push('('),
            '}' => re.push(')'),
            ',' => re.push('|'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| e.to_string())
}

pub fn fs_glob(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let raw = arg_str(a, "path");
    let base = join_inside(&root, if raw.is_empty() { "." } else { &raw })?;
    let pattern = arg_str(a, "pattern");
    let re = glob_to_regex(&pattern)?;
    let bare = !pattern.contains('/');
    let mut hits = Vec::new();
    for e in walk(&base, 32).filter(|e| e.file_type().is_file()) {
        let rel = rel_str(&base, e.path());
        let name = e.file_name().to_string_lossy();
        if re.is_match(&rel) || (bare && re.is_match(&name)) {
            hits.push(rel_str(&root, e.path()));
            if hits.len() >= GLOB_MAX {
                break;
            }
        }
    }
    Ok(json!({ "pattern": pattern, "files": hits }))
}

pub fn fs_grep(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let raw = arg_str(a, "path");
    let base = join_inside(&root, if raw.is_empty() { "." } else { &raw })?;
    let pattern = arg_str(a, "pattern");
    let literal = a
        .get("literal_text")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let re = regex::Regex::new(&if literal {
        regex::escape(&pattern)
    } else {
        pattern.clone()
    })
    .map_err(|e| e.to_string())?;
    let include = arg_str(a, "include");
    let include = if include.is_empty() {
        None
    } else {
        Some(glob_to_regex(&include)?)
    };
    let mut matches = Vec::new();
    'files: for e in walk(&base, 32).filter(|e| e.file_type().is_file()) {
        if let Some(inc) = &include {
            if !inc.is_match(&e.file_name().to_string_lossy())
                && !inc.is_match(&rel_str(&base, e.path()))
            {
                continue;
            }
        }
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        if bytes.len() > 2 * 1024 * 1024 || bytes.iter().take(4096).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (n, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let shown: String = line.trim().chars().take(240).collect();
                matches.push(format!("{}:{}: {}", rel_str(&root, e.path()), n + 1, shown));
                if matches.len() >= GREP_MAX {
                    break 'files;
                }
            }
        }
    }
    Ok(json!({ "pattern": pattern, "matches": matches, "truncated": matches.len() >= GREP_MAX }))
}

// ── edit / write ─────────────────────────────────────────────────────────

/// Candidate byte ranges of `old` inside `text`, from the strictest replacer to
/// the most forgiving (opencode's cascade, the subset that needs no fuzzy
/// distance): exact, per-line trimmed, whitespace-normalised, indent-flexible.
fn find_candidates(text: &str, old: &str) -> Vec<(usize, usize)> {
    let exact: Vec<(usize, usize)> = text
        .match_indices(old)
        .map(|(i, m)| (i, i + m.len()))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let old_lines: Vec<&str> = old.trim_matches('\n').lines().collect();
    if old_lines.is_empty() {
        return Vec::new();
    }
    let mut line_spans = Vec::new();
    let mut pos = 0;
    for l in text.split_inclusive('\n') {
        line_spans.push((pos, pos + l.len(), l.trim_end_matches(['\n', '\r'])));
        pos += l.len();
    }
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    for mode in 0..2 {
        let mut found = Vec::new();
        if line_spans.len() >= old_lines.len() {
            for start in 0..=(line_spans.len() - old_lines.len()) {
                let ok = old_lines.iter().enumerate().all(|(k, ol)| {
                    let tl = line_spans[start + k].2;
                    if mode == 0 {
                        tl.trim() == ol.trim()
                    } else {
                        norm(tl) == norm(ol)
                    }
                });
                if ok {
                    let end_line = &line_spans[start + old_lines.len() - 1];
                    let end = end_line.0 + end_line.2.len();
                    found.push((line_spans[start].0, end));
                }
            }
        }
        if !found.is_empty() {
            return found;
        }
    }
    block_anchor(&line_spans, &old_lines)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// opencode's BlockAnchor replacer: first and last line anchor the block, the
/// middle is scored by Levenshtein similarity. A span whose size is out of
/// proportion with `old` is never a candidate (the model would wipe code it
/// never saw).
fn block_anchor(line_spans: &[(usize, usize, &str)], old_lines: &[&str]) -> Vec<(usize, usize)> {
    if old_lines.len() < 3 {
        return Vec::new();
    }
    let first = old_lines[0].trim();
    let last = old_lines[old_lines.len() - 1].trim();
    let slack = (old_lines.len() / 2).max(3);
    let mut scored: Vec<(f64, usize, usize)> = Vec::new();
    for start in 0..line_spans.len() {
        if line_spans[start].2.trim() != first {
            continue;
        }
        for end in (start + 2)..line_spans.len() {
            if line_spans[end].2.trim() != last {
                continue;
            }
            let span_len = end - start + 1;
            if span_len.abs_diff(old_lines.len()) > slack {
                break;
            }
            let mid = (old_lines.len() - 2).min(span_len - 2);
            let mut sim = 0.0;
            for k in 1..=mid {
                let (a, b) = (line_spans[start + k].2.trim(), old_lines[k].trim());
                let max = a.chars().count().max(b.chars().count());
                sim += if max == 0 {
                    1.0
                } else {
                    1.0 - levenshtein(a, b) as f64 / max as f64
                };
            }
            let sim = if mid == 0 { 1.0 } else { sim / mid as f64 };
            scored.push((
                sim,
                line_spans[start].0,
                line_spans[end].0 + line_spans[end].2.len(),
            ));
            break;
        }
    }
    let threshold = if scored.len() == 1 { 0.3 } else { 0.5 };
    scored.retain(|c| c.0 >= threshold);
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.first().map(|c| vec![(c.1, c.2)]).unwrap_or_default()
}

fn short_diff(path: &str, before: &str, after: &str) -> String {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let mut head = 0;
    while head < b.len() && head < a.len() && b[head] == a[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < b.len() - head
        && tail < a.len() - head
        && b[b.len() - 1 - tail] == a[a.len() - 1 - tail]
    {
        tail += 1;
    }
    let mut out = format!("--- {path}\n+++ {path}\n@@ line {} @@\n", head + 1);
    for l in &b[head..b.len() - tail] {
        out.push_str(&format!("-{l}\n"));
    }
    for l in &a[head..a.len() - tail] {
        out.push_str(&format!("+{l}\n"));
    }
    cap(out, 16 * 1024)
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!(
        "{}.omniget-tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn fs_edit(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let _write = write_lock_blocking(&root)?;
    let path = join_inside(&root, &arg_str(a, "path"))?;
    writable(&root, &path)?;
    let old = arg_str(a, "old_string");
    let new = arg_str(a, "new_string");
    let all = a
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rel = rel_str(&root, &path);
    if old.is_empty() {
        if path.exists() {
            return Err(format!(
                "{ERR_EDIT_NO_MATCH}: old_string is empty and {rel} exists"
            ));
        }
        write_atomic(&path, &new)?;
        return Ok(json!({ "path": rel, "created": true, "diff": short_diff(&rel, "", &new) }));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
    let crlf = raw.contains("\r\n");
    let text = if crlf {
        raw.replace("\r\n", "\n")
    } else {
        raw.clone()
    };
    let old_n = old.replace("\r\n", "\n");
    let new_n = new.replace("\r\n", "\n");
    let spans = find_candidates(&text, &old_n);
    if spans.is_empty() {
        return Err(format!("{ERR_EDIT_NO_MATCH}: old_string not found in {rel}; read the file again and copy the exact lines"));
    }
    if spans.len() > 1 && !all {
        return Err(format!(
            "{ERR_EDIT_AMBIGUOUS}: old_string matches {} places in {rel}; add surrounding lines or set replace_all",
            spans.len()
        ));
    }
    let mut after = text.clone();
    for (s, e) in spans.iter().rev() {
        after.replace_range(*s..*e, &new_n);
    }
    let diff = short_diff(&rel, &text, &after);
    let out = if crlf {
        after.replace('\n', "\r\n")
    } else {
        after
    };
    write_atomic(&path, &out)?;
    Ok(json!({ "path": rel, "replacements": spans.len(), "diff": diff }))
}

pub fn fs_write(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let _write = write_lock_blocking(&root)?;
    let path = join_inside(&root, &arg_str(a, "path"))?;
    writable(&root, &path)?;
    let content = arg_str(a, "content");
    let existed = path.exists();
    write_atomic(&path, &content)?;
    Ok(json!({ "path": rel_str(&root, &path), "bytes": content.len(), "created": !existed }))
}

// ── apply_patch (V4A envelope) ───────────────────────────────────────────

fn find_block(lines: &[String], block: &[String], from: usize) -> Option<(usize, u32)> {
    if block.is_empty() {
        return Some((from.min(lines.len()), 0));
    }
    if lines.len() < block.len() {
        return None;
    }
    type Norm = fn(&str) -> String;
    let ladder: [(u32, Norm); 3] = [
        (0, |s| s.to_string()),
        (1, |s| s.trim_end().to_string()),
        (100, |s| s.trim().to_string()),
    ];
    for (fuzz, norm) in ladder {
        for pass_from in [from, 0] {
            for start in pass_from..=(lines.len() - block.len()) {
                if block
                    .iter()
                    .enumerate()
                    .all(|(k, b)| norm(&lines[start + k]) == norm(b))
                {
                    return Some((start, fuzz));
                }
            }
        }
    }
    None
}

pub fn fs_apply_patch(a: &Value) -> Result<Value, String> {
    let root = root()?;
    let _write = write_lock_blocking(&root)?;
    let patch = arg_str(a, "patch");
    let lines: Vec<&str> = patch.lines().collect();
    let begin = lines
        .iter()
        .position(|l| l.trim() == "*** Begin Patch")
        .ok_or_else(|| format!("{ERR_PATCH}: missing '*** Begin Patch'"))?;
    let mut i = begin + 1;
    let mut summary = Vec::new();
    let mut staged: Vec<(PathBuf, Option<String>, Option<PathBuf>)> = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if line.trim() == "*** End Patch" {
            break;
        }
        if let Some(p) = line.strip_prefix("*** Add File: ") {
            let path = join_inside(&root, p)?;
            writable(&root, &path)?;
            i += 1;
            let mut body = String::new();
            while i < lines.len() && !lines[i].starts_with("*** ") {
                body.push_str(lines[i].strip_prefix('+').unwrap_or(lines[i]));
                body.push('\n');
                i += 1;
            }
            summary.push(format!("A {}", rel_str(&root, &path)));
            staged.push((path, Some(body), None));
        } else if let Some(p) = line.strip_prefix("*** Delete File: ") {
            let path = join_inside(&root, p)?;
            writable(&root, &path)?;
            summary.push(format!("D {}", rel_str(&root, &path)));
            staged.push((path, None, None));
            i += 1;
        } else if let Some(p) = line.strip_prefix("*** Update File: ") {
            let path = join_inside(&root, p)?;
            writable(&root, &path)?;
            let rel = rel_str(&root, &path);
            i += 1;
            let mut move_to = None;
            if i < lines.len() {
                if let Some(m) = lines[i].strip_prefix("*** Move to: ") {
                    let dest = join_inside(&root, m)?;
                    writable(&root, &dest)?;
                    move_to = Some(dest);
                    i += 1;
                }
            }
            let raw =
                std::fs::read_to_string(&path).map_err(|e| format!("{ERR_PATCH}: {rel}: {e}"))?;
            let mut file: Vec<String> = raw
                .replace("\r\n", "\n")
                .lines()
                .map(str::to_string)
                .collect();
            let mut cursor = 0usize;
            let mut worst = 0u32;
            while i < lines.len() && !lines[i].starts_with("*** ")
                || (i < lines.len() && lines[i].trim() == "*** End of File")
            {
                if lines[i].trim() == "*** End of File" {
                    i += 1;
                    continue;
                }
                if let Some(anchor) = lines[i].strip_prefix("@@") {
                    let anchor = anchor.trim();
                    if !anchor.is_empty() {
                        if let Some(at) = file.iter().skip(cursor).position(|l| l.trim() == anchor)
                        {
                            cursor += at + 1;
                        } else if let Some(at) = file.iter().position(|l| l.trim() == anchor) {
                            cursor = at + 1;
                        }
                    }
                    i += 1;
                }
                // `kept` pairs a context line of the new block with its index
                // in the old one: a fuzzy match must keep the file's own line.
                let (mut old_block, mut new_block) = (Vec::new(), Vec::<String>::new());
                let mut kept: Vec<(usize, usize)> = Vec::new();
                while i < lines.len()
                    && !lines[i].starts_with("@@")
                    && !lines[i].starts_with("*** ")
                {
                    let l = lines[i];
                    match l.chars().next() {
                        Some('-') => old_block.push(l[1..].to_string()),
                        Some('+') => new_block.push(l[1..].to_string()),
                        Some(' ') => {
                            kept.push((new_block.len(), old_block.len()));
                            old_block.push(l[1..].to_string());
                            new_block.push(l[1..].to_string());
                        }
                        None => {
                            kept.push((new_block.len(), old_block.len()));
                            old_block.push(String::new());
                            new_block.push(String::new());
                        }
                        _ => return Err(format!("{ERR_PATCH}: {rel}: bad hunk line {:?}", l)),
                    }
                    i += 1;
                }
                if old_block.is_empty() && new_block.is_empty() {
                    continue;
                }
                let (at, fuzz) = find_block(&file, &old_block, cursor).ok_or_else(|| {
                    format!(
                        "{ERR_PATCH}: {rel}: context not found:\n{}",
                        old_block.join("\n")
                    )
                })?;
                worst = worst.max(fuzz);
                for (n, o) in &kept {
                    new_block[*n] = file[at + o].clone();
                }
                file.splice(at..at + old_block.len(), new_block.iter().cloned());
                cursor = at + new_block.len();
            }
            let mut body = file.join("\n");
            if raw.ends_with('\n') || raw.is_empty() {
                body.push('\n');
            }
            if raw.contains("\r\n") {
                body = body.replace('\n', "\r\n");
            }
            summary.push(format!(
                "M {rel}{}",
                if worst > 0 {
                    format!(" (fuzz {worst})")
                } else {
                    String::new()
                }
            ));
            staged.push((path, Some(body), move_to));
        } else {
            i += 1;
        }
    }
    if staged.is_empty() {
        return Err(format!("{ERR_PATCH}: the patch changes nothing"));
    }
    // Everything parsed and matched: only now touch the disk.
    for (path, body, move_to) in staged {
        match (body, move_to) {
            (None, _) => std::fs::remove_file(&path).map_err(|e| e.to_string())?,
            (Some(b), Some(dest)) => {
                write_atomic(&dest, &b)?;
                let _ = std::fs::remove_file(&path);
            }
            (Some(b), None) => write_atomic(&path, &b)?,
        }
    }
    Ok(json!({ "applied": summary }))
}

// ── shell ────────────────────────────────────────────────────────────────

/// Seatbelt profile: deny by default, read anywhere, write only inside the
/// workspace and the temp dirs, no network.
#[cfg(target_os = "macos")]
fn seatbelt_profile(workspace: &Path) -> String {
    let ws = workspace.to_string_lossy().replace('"', "\\\"");
    format!(
        r#"(version 1)
(deny default)
(allow process-exec* process-fork signal sysctl-read mach-lookup ipc-posix-shm* file-read* file-ioctl)
(allow file-write* (subpath "{ws}") (subpath "/private/tmp") (subpath "/private/var/folders") (literal "/dev/null") (literal "/dev/tty") (regex #"^/dev/fd/"))
(deny file-write* (subpath "{ws}/.git"))
(deny network*)
"#
    )
}

pub fn sandbox_kind() -> &'static str {
    if cfg!(target_os = "macos") {
        "seatbelt"
    } else {
        "none"
    }
}

pub async fn shell_exec(a: Value) -> Result<Value, String> {
    let root = root()?;
    let command = arg_str(&a, "command");
    if command.trim().is_empty() {
        return Err("command is empty".into());
    }
    let workdir = arg_str(&a, "workdir");
    // Small models invent container paths (`/workspace`, `/app`). A workdir
    // that does not exist anywhere is that; one that exists outside is refused.
    let invented = Path::new(workdir.trim()).is_absolute()
        && !Path::new(workdir.trim()).starts_with(&root)
        && !Path::new(workdir.trim()).exists();
    let cwd = join_inside(
        &root,
        if workdir.is_empty() || invented {
            "."
        } else {
            &workdir
        },
    )?;
    let timeout = a
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(SHELL_TIMEOUT_MS)
        .clamp(1_000, 600_000);

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
        c.arg("-p")
            .arg(seatbelt_profile(&root))
            .arg("/bin/zsh")
            .arg("-lc")
            .arg(&command);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("/bin/sh");
        c.arg("-c").arg(&command);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&command);
        c
    };
    cmd.current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // One writer per folder: a command may write anywhere inside it.
    let _write = write_lock(&root).await;
    let started = std::time::Instant::now();
    let child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let out = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout),
        child.wait_with_output(),
    )
    .await
    {
        Ok(r) => r.map_err(|e| e.to_string())?,
        Err(_) => return Err(format!("ERR_CODE_SHELL_TIMEOUT: killed after {timeout} ms")),
    };
    // Keep the tail: the end of a build log is where the error is.
    let tail = |b: &[u8]| {
        let s = String::from_utf8_lossy(b).to_string();
        if s.len() > OUT_MAX {
            let mut cut = s.len() - OUT_MAX;
            while !s.is_char_boundary(cut) {
                cut += 1;
            }
            format!("[… truncated]\n{}", &s[cut..])
        } else {
            s
        }
    };
    Ok(json!({
        "exit_code": out.status.code(),
        "stdout": tail(&out.stdout),
        "stderr": tail(&out.stderr),
        "ms": started.elapsed().as_millis() as u64,
        "sandbox": sandbox_kind(),
    }))
}

// ── plan ─────────────────────────────────────────────────────────────────

pub fn todo_write(a: &Value) -> Result<Value, String> {
    let plan = a.get("plan").cloned().unwrap_or(Value::Null);
    if !plan.is_array() {
        return Err("plan must be an array of {step, status}".into());
    }
    *PLAN.write().unwrap_or_else(|e| e.into_inner()) = Some(plan.clone());
    Ok(json!({ "plan": plan }))
}

/// What the permission prompt shows for a call: the command, the path, or the
/// head of the patch. Never the whole payload.
pub fn preview(tool: &str, input: &Value) -> String {
    let s = match tool {
        "shell_exec" => arg_str(input, "command"),
        "fs_apply_patch" => arg_str(input, "patch"),
        "fs_write" => format!(
            "{} ({} bytes)",
            arg_str(input, "path"),
            arg_str(input, "content").len()
        ),
        "fs_edit" => format!(
            "{}\n- {}\n+ {}",
            arg_str(input, "path"),
            arg_str(input, "old_string"),
            arg_str(input, "new_string")
        ),
        _ => input.to_string(),
    };
    cap(s, 2000)
}

#[cfg(test)]
mod tool_call_context_tests {
    #[tokio::test]
    async fn real_call_id_is_local_to_its_scope() {
        assert!(super::current_tool_call().is_none());
        super::scope_tool_call("call-real", async {
            assert_eq!(super::current_tool_call().as_deref(), Some("call-real"));
        })
        .await;
        assert!(super::current_tool_call().is_none());
    }
}

#[cfg(test)]
mod conversation_workspace_tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("omniget-ct-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    /// A04: a project opened elsewhere (the process-wide folder, or another
    /// conversation) must not become the cwd of a personal conversation.
    #[tokio::test]
    async fn a_personal_conversation_does_not_inherit_the_last_opened_folder() {
        let project = temp_dir("a04");
        std::fs::write(project.join("secret.txt"), "project data").unwrap();
        // Somebody opened a project: in another conversation and globally
        // (what `llm_workspace_set` used to do on every pick).
        set_conversation_workspace("a04-project-chat", Some(project.clone())).unwrap();
        set_workspace(Some(project.clone())).unwrap();

        assert_eq!(workspace_of("a04-personal-chat"), None);
        let seen = scope("a04-personal-chat", "reader", "r1", async {
            (
                workspace(),
                fs_read(&json!({ "path": "secret.txt" })),
                fs_list(&json!({})),
            )
        })
        .await;
        assert_eq!(seen.0, None, "no fallback to the last cwd");
        assert!(seen.1.unwrap_err().starts_with(ERR_NO_WORKSPACE));
        assert!(seen.2.unwrap_err().starts_with(ERR_NO_WORKSPACE));
        // The project conversation still has its folder.
        let own = scope("a04-project-chat", "coder", "r2", async { workspace() }).await;
        assert_eq!(own, Some(project.clone()));
        // Outside any turn (embedded MCP server, CLI) the global one still answers.
        assert!(workspace().is_some());
    }

    /// A05: two conversations, two folders, each keeps its own.
    #[tokio::test]
    async fn two_conversations_keep_their_own_folders() {
        let a = temp_dir("a05a");
        let b = temp_dir("a05b");
        std::fs::write(a.join("which.txt"), "A").unwrap();
        std::fs::write(b.join("which.txt"), "B").unwrap();
        set_conversation_workspace("a05-one", Some(a.clone())).unwrap();
        set_conversation_workspace("a05-two", Some(b.clone())).unwrap();
        let read = |conv: &'static str| async move {
            scope(conv, "coder", "r", async {
                fs_read(&json!({ "path": "which.txt" })).unwrap()["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .await
        };
        assert!(read("a05-one").await.contains('A'));
        assert!(read("a05-two").await.contains('B'));
        // Re-pointing one does not move the other.
        let c = temp_dir("a05c");
        set_conversation_workspace("a05-one", Some(c.clone())).unwrap();
        assert_eq!(workspace_of("a05-one"), Some(c));
        assert_eq!(workspace_of("a05-two"), Some(b));
        set_conversation_workspace("a05-one", None).unwrap();
        assert_eq!(workspace_of("a05-one"), None);
    }

    /// B06: two writers in one folder run one after the other.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn writes_in_one_folder_are_serialised() {
        let ws = temp_dir("b06");
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(&str, u128)>::new()));
        let t0 = std::time::Instant::now();
        let mut tasks = Vec::new();
        for who in ["first", "second"] {
            let ws = ws.clone();
            let log = log.clone();
            tasks.push(tokio::spawn(async move {
                let _g = write_lock(&ws).await;
                log.lock().unwrap().push((who, t0.elapsed().as_millis()));
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                log.lock().unwrap().push((who, t0.elapsed().as_millis()));
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 4);
        // enter/leave pairs never interleave
        assert_eq!(log[0].0, log[1].0);
        assert_eq!(log[2].0, log[3].0);
        assert!(log[2].1 >= log[1].1);

        // A synchronous write waits for a shell-style holder in the same folder.
        set_conversation_workspace("b06-a", Some(ws.clone())).unwrap();
        set_conversation_workspace("b06-b", Some(ws.clone())).unwrap();
        let holder = write_lock(&ws).await;
        let writer = tokio::spawn(scope("b06-b", "second", "r", async {
            let started = std::time::Instant::now();
            fs_write(&json!({ "path": "out.txt", "content": "late" })).unwrap();
            started.elapsed().as_millis()
        }));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            !ws.join("out.txt").exists(),
            "the writer must wait for the holder"
        );
        drop(holder);
        let waited = writer.await.unwrap();
        assert!(waited >= 150, "waited {waited} ms");
        assert_eq!(std::fs::read_to_string(ws.join("out.txt")).unwrap(), "late");
    }

    #[tokio::test]
    async fn a_busy_folder_on_a_single_thread_runtime_answers_instead_of_deadlocking() {
        let ws = temp_dir("b06-ct");
        let _held = write_lock(&ws).await;
        let err = write_lock_blocking(&ws).err().unwrap();
        assert!(err.starts_with(ERR_WORKSPACE_BUSY));
    }
}
