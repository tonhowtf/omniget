//! Undo by shadow git: one bare-ish git dir per workspace under the app data
//! dir, never inside the user's repo. Before the first writing tool of a turn
//! the whole work tree is staged into the shadow index and `write-tree` gives a
//! tree id; undo stages the present and `read-tree --reset -u` back to that
//! tree. The user's own `.git` is only borrowed through `alternates`, so no
//! object is copied twice and nothing of theirs is touched. Shape read from
//! opencode (`snapshot/index.ts`) and cline checkpoints.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::folder_key::FolderKey;

pub const ERR_SNAPSHOT: &str = "ERR_CODE_SNAPSHOT";
pub const ERR_NOTHING_TO_UNDO: &str = "ERR_CODE_NOTHING_TO_UNDO";
pub const ERR_UNDO_SHARED: &str = "ERR_UNDO_SHARED";
pub const ERR_UNDO_CONFLICT: &str = "ERR_UNDO_CONFLICT";
/// A job's diff cannot be told: some turn of it took no snapshot. Never
/// answered with an empty diff (78#P2).
pub const ERR_DIFF_UNAVAILABLE: &str = "ERR_DIFF_UNAVAILABLE";

static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_root(path: PathBuf) {
    *ROOT.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub request: String,
    pub tree: String,
    pub at_ms: u64,
    /// Messages the conversation held when this turn began, so undo can take
    /// the turn's bubbles back too. `None` in entries written before 0.10.0.
    #[serde(default)]
    pub messages_before: Option<usize>,
    /// Tree right after the turn ended, so the turn's own diff can be shown
    /// later even when newer turns changed the folder again.
    #[serde(default)]
    pub after: Option<String>,
}

/// One file a turn changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// `A` added, `M` modified, `D` deleted, `R` renamed.
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    /// No line counts: git sees binary content.
    #[serde(default)]
    pub binary: bool,
}

/// The real diff of one turn, from the shadow snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnDiff {
    pub files: Vec<FileChange>,
    /// Unified diff, clipped to [`DIFF_MAX_BYTES`].
    pub patch: String,
    pub clipped: bool,
    /// False while the turn has not ended (the diff is against the folder
    /// as it is now).
    pub complete: bool,
}

pub const DIFF_MAX_BYTES: usize = 256 * 1024;
/// Most files a job diff lists; the totals still count every file.
pub const DIFF_MAX_FILES: usize = 1000;

/// What a whole job changed in its folder (1.7, shared mode): the tree
/// before its first turn against the tree after its last one, from the
/// shadow snapshots, so nothing of the user's repo is read or touched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct JobDiff {
    pub files: Vec<FileChange>,
    pub additions: u64,
    pub deletions: u64,
    /// Where the caller stored [`Self::patch`] (`<llm dir>/jobs/<id>.patch`).
    pub patch_path: Option<PathBuf>,
    /// The patch was cut at [`DIFF_MAX_BYTES`] or the list at
    /// [`DIFF_MAX_FILES`].
    pub clipped: bool,
    /// Commit of the job's tip; worktree mode only (2.4), `None` here.
    pub head_oid: Option<String>,
    /// False when the last turn never recorded its end: the diff is against
    /// the folder as it is now.
    pub complete: bool,
    /// Unified diff, text only (binary files are one `Binary files` line).
    #[serde(skip)]
    pub patch: String,
}

/// Turn marks of this process: request id → messages the conversation held
/// when the turn began. The Coordinator sets one per turn; `before_write`
/// copies it into the entry it pushes.
static MARKS: RwLock<Option<std::collections::HashMap<String, usize>>> = RwLock::new(None);

/// Why the last snapshot of a workspace failed (no `git`, a read-only disk).
/// The chip shows it: an undo that cannot happen must not look available.
static FAILURES: RwLock<Option<std::collections::HashMap<PathBuf, String>>> = RwLock::new(None);

pub fn mark_turn(request: &str, messages_before: usize) {
    let mut marks = MARKS.write().unwrap_or_else(|e| e.into_inner());
    let map = marks.get_or_insert_with(Default::default);
    if map.len() > 256 {
        map.clear();
    }
    map.insert(request.to_string(), messages_before);
}

fn mark_of(request: &str) -> Option<usize> {
    MARKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(request).copied())
}

fn note_failure(workspace: &Path, error: Option<&str>) {
    let mut all = FAILURES.write().unwrap_or_else(|e| e.into_inner());
    let map = all.get_or_insert_with(Default::default);
    match error {
        Some(e) => {
            map.insert(workspace.to_path_buf(), e.to_string());
        }
        None => {
            map.remove(workspace);
        }
    }
}

/// `git` resolvable on the PATH. Checked without spawning anything.
pub fn git_available() -> bool {
    let exe = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
        .unwrap_or(false)
}

/// What stands between this workspace and an undo, if anything.
pub fn problem(workspace: &Path) -> Option<String> {
    if !git_available() {
        return Some(format!(
            "{ERR_SNAPSHOT}: git was not found on the PATH, so turns cannot be undone"
        ));
    }
    FAILURES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(workspace).cloned())
}

/// What `undo` put back.
#[derive(Debug, Clone, Serialize)]
pub struct Undone {
    pub files: Vec<String>,
    pub messages_before: Option<usize>,
}

fn git_dir(workspace: &Path) -> Result<PathBuf, String> {
    let root = ROOT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| format!("{ERR_SNAPSHOT}: no snapshot dir"))?;
    let mut hasher = Sha256::new();
    hasher.update(workspace.to_string_lossy().as_bytes());
    let hash: String = hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(root.join(hash))
}

async fn git(dir: &Path, workspace: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_io(dir, workspace, args, None, None).await?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// `git` with bytes in and out: `stdin` is written and closed, `index`
/// points `GIT_INDEX_FILE` at a private index (the undo builds its target
/// tree there without touching the shadow index).
async fn git_io(
    dir: &Path,
    workspace: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    index: Option<&Path>,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(dir)
        .arg("--work-tree")
        .arg(workspace)
        .args(["-c", "core.autocrlf=false", "-c", "core.quotepath=false"])
        .args(args)
        .current_dir(workspace)
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{ERR_SNAPSHOT}: git: {e}"))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes)
            .await
            .map_err(|e| format!("{ERR_SNAPSHOT}: git: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("{ERR_SNAPSHOT}: git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{ERR_SNAPSHOT}: git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

async fn ensure(workspace: &Path) -> Result<PathBuf, String> {
    let dir = git_dir(workspace)?;
    if !dir.join("HEAD").exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_SNAPSHOT}: {e}"))?;
        git(&dir, workspace, &["init", "-q"]).await?;
        let real_objects = workspace.join(".git").join("objects");
        if real_objects.is_dir() {
            let info = dir.join("objects").join("info");
            let _ = std::fs::create_dir_all(&info);
            let _ = std::fs::write(
                info.join("alternates"),
                format!("{}\n", real_objects.display()),
            );
        }
        let info = dir.join("info");
        let _ = std::fs::create_dir_all(&info);
        let _ = std::fs::write(
            info.join("exclude"),
            "node_modules/\ntarget/\ndist/\nbuild/\n.svelte-kit/\n*.omniget-tmp\n",
        );
    }
    Ok(dir)
}

async fn stage_tree(dir: &Path, workspace: &Path) -> Result<String, String> {
    git(dir, workspace, &["add", "-A", "."]).await?;
    git(dir, workspace, &["write-tree"]).await
}

fn stack_file(dir: &Path, conversation: &str) -> PathBuf {
    let safe: String = conversation
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("omniget-undo-{safe}.json"))
}

fn load(dir: &Path, conversation: &str) -> Vec<Entry> {
    std::fs::read(stack_file(dir, conversation))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn store(dir: &Path, conversation: &str, stack: &[Entry]) {
    if let Ok(bytes) = serde_json::to_vec(stack) {
        let _ = std::fs::write(stack_file(dir, conversation), bytes);
    }
}

/// Called before a writing tool runs. A failure is remembered for the UI.
pub async fn before_write(
    workspace: &Path,
    conversation: &str,
    request: &str,
) -> Result<(), String> {
    let out = before_write_inner(workspace, conversation, request).await;
    note_failure(workspace, out.as_ref().err().map(String::as_str));
    out
}

/// The snapshot itself. One snapshot per turn: a second write of
/// the same `request` is a no-op.
async fn before_write_inner(
    workspace: &Path,
    conversation: &str,
    request: &str,
) -> Result<(), String> {
    let dir = ensure(workspace).await?;
    let mut stack = load(&dir, conversation);
    if stack.last().map(|e| e.request == request).unwrap_or(false) {
        return Ok(());
    }
    let tree = stage_tree(&dir, workspace).await?;
    stack.push(Entry {
        request: request.to_string(),
        messages_before: mark_of(request),
        after: None,
        tree,
        at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    });
    if stack.len() > 50 {
        let cut = stack.len() - 50;
        stack.drain(..cut);
    }
    store(&dir, conversation, &stack);
    Ok(())
}

/// Records the tree after a turn that took a snapshot. A turn that wrote
/// nothing has no entry and costs nothing here.
pub async fn after_turn(workspace: &Path, conversation: &str, request: &str) -> Result<(), String> {
    let dir = git_dir(workspace)?;
    if !dir.join("HEAD").exists() {
        return Ok(());
    }
    let mut stack = load(&dir, conversation);
    let Some(pos) = stack.iter().rposition(|e| e.request == request) else {
        return Ok(());
    };
    let tree = stage_tree(&dir, workspace).await?;
    stack[pos].after = Some(tree);
    store(&dir, conversation, &stack);
    Ok(())
}

/// The files and patch one turn changed: its snapshot tree against the tree
/// taken when it ended (or the folder now, while it runs).
pub async fn turn_diff(
    workspace: &Path,
    conversation: &str,
    request: &str,
) -> Result<Option<TurnDiff>, String> {
    let dir = git_dir(workspace)?;
    if !dir.join("HEAD").exists() {
        return Ok(None);
    }
    let stack = load(&dir, conversation);
    let Some(entry) = stack.iter().rev().find(|e| e.request == request).cloned() else {
        return Ok(None);
    };
    let (after, complete) = match entry.after.clone() {
        Some(t) => (t, true),
        None => (stage_tree(&dir, workspace).await?, false),
    };
    let status = git(
        &dir,
        workspace,
        &["diff", "--name-status", "-M", &entry.tree, &after],
    )
    .await?;
    let numstat = git(
        &dir,
        workspace,
        &["diff", "--numstat", "-M", &entry.tree, &after],
    )
    .await?;
    let mut files: Vec<FileChange> = status
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let mut parts = l.split('\t');
            let st = parts.next()?.chars().next()?.to_string();
            let path = parts.last()?.to_string();
            Some(FileChange {
                path,
                status: st,
                additions: None,
                deletions: None,
                binary: false,
            })
        })
        .collect();
    for l in numstat.lines() {
        let cols: Vec<&str> = l.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let path = cols[cols.len() - 1];
        if let Some(f) = files
            .iter_mut()
            .find(|f| f.path == path || path.ends_with(&f.path))
        {
            f.additions = cols[0].parse().ok();
            f.deletions = cols[1].parse().ok();
            f.binary = cols[0] == "-" && cols[1] == "-";
        }
    }
    let mut patch = git(&dir, workspace, &["diff", "-M", &entry.tree, &after]).await?;
    let clipped = patch.len() > DIFF_MAX_BYTES;
    if clipped {
        let mut end = DIFF_MAX_BYTES;
        while !patch.is_char_boundary(end) {
            end -= 1;
        }
        patch.truncate(end);
    }
    Ok(Some(TurnDiff {
        files,
        patch,
        clipped,
        complete,
    }))
}

/// The whole diff of a job whose turns are `requests` (its request ids, in
/// any order) in `conversation`: the `tree` of its earliest snapshot against
/// the `after` of its latest one. `Err(ERR_DIFF_UNAVAILABLE…)` when the
/// folder has no snapshots or any of the turns took none — never an empty
/// diff standing for "unknown". A job with snapshots that changed nothing is
/// `Ok` with no files. Renames are a delete plus an add (the patch applies
/// with `git apply` and reads like `diff -ruN`).
pub async fn job_diff(
    workspace: &Path,
    conversation: &str,
    requests: &[String],
) -> Result<JobDiff, String> {
    let unavailable = |why: String| format!("{ERR_DIFF_UNAVAILABLE}: {why}");
    let mut wanted: Vec<&str> = requests
        .iter()
        .map(String::as_str)
        .filter(|r| !r.is_empty())
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return Err(unavailable("the job never started a turn".into()));
    }
    let dir = git_dir(workspace).map_err(unavailable)?;
    if !dir.join("HEAD").exists() {
        return Err(unavailable(format!(
            "no snapshot was ever taken of {}",
            workspace.display()
        )));
    }
    let stack = load(&dir, conversation);
    let found: Vec<usize> = wanted
        .iter()
        .filter_map(|r| stack.iter().rposition(|e| e.request == *r))
        .collect();
    if found.len() < wanted.len() {
        return Err(unavailable(if found.is_empty() {
            "no turn of this job took a snapshot".into()
        } else {
            format!(
                "{} of {} turns of this job took no snapshot",
                wanted.len() - found.len(),
                wanted.len()
            )
        }));
    }
    let (first, last) = (
        found.iter().copied().min().unwrap_or(0),
        found.iter().copied().max().unwrap_or(0),
    );
    let base = stack[first].tree.clone();
    let (tip, complete) = match stack[last].after.clone() {
        Some(tree) => (tree, true),
        None => (stage_tree_private(&dir, workspace).await?, false),
    };
    let mut diff = diff_trees(&dir, workspace, &base, &tip).await?;
    diff.complete = complete;
    Ok(diff)
}

/// The folder as a tree, staged in a copy of the shadow index (its stat
/// cache keeps it fast) that is removed afterwards. For a turn still
/// running: the shared index belongs to `before_write`/`after_turn`, and a
/// reader holding its lock could make the turn's own `after_turn` fail.
async fn stage_tree_private(dir: &Path, workspace: &Path) -> Result<String, String> {
    let index = dir.join(format!("omniget-jobdiff-{}.index", uuid::Uuid::new_v4().simple()));
    // Git replaces the index by rename, so a copy is always a whole file.
    if let Err(e) = std::fs::copy(dir.join("index"), &index) {
        tracing::debug!("[snapshot] no index to copy ({e}): staging from scratch");
    }
    let out = async {
        git_io(dir, workspace, &["add", "-A", "."], None, Some(&index)).await?;
        let tree = git_io(dir, workspace, &["write-tree"], None, Some(&index)).await?;
        Ok::<String, String>(String::from_utf8_lossy(&tree).trim().to_string())
    }
    .await;
    let _ = std::fs::remove_file(&index);
    out
}

/// Files, totals and the text patch between two trees of the shadow git,
/// from one `git diff` (each spawn costs tens of ms; BN4 asks < 200 ms at 50
/// files). The user's git config must not change the result (a
/// `diff.noprefix`, a histogram algorithm, colour), so every such option is
/// pinned.
async fn diff_trees(dir: &Path, workspace: &Path, from: &str, to: &str) -> Result<JobDiff, String> {
    let out = git_io(
        dir,
        workspace,
        &[
            "diff",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--diff-algorithm=myers",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--raw",
            "--numstat",
            "--patch",
            "-z",
            from,
            to,
        ],
        None,
        None,
    )
    .await?;
    // `--raw --numstat -z` records, an empty record, then the patch. No
    // record is empty (paths cannot hold NUL), so the first NUL NUL is the
    // boundary.
    let (records, patch) = match out.windows(2).position(|w| w == [0, 0]) {
        Some(i) => (&out[..i], &out[i + 2..]),
        None => (&out[..], &out[out.len()..]),
    };
    let mut files: Vec<FileChange> = Vec::new();
    let (mut additions, mut deletions) = (0u64, 0u64);
    let mut counts = std::collections::HashMap::new();
    let mut fields = records.split(|b| *b == 0);
    while let Some(field) = fields.next() {
        if field.first() == Some(&b':') {
            // `:<mode> <mode> <oid> <oid> <status>`, then the path.
            let status = field.rsplit(|b| *b == b' ').next().unwrap_or(b"M");
            let Some(path) = fields.next() else { break };
            files.push(FileChange {
                path: String::from_utf8_lossy(path).to_string(),
                status: String::from_utf8_lossy(&status[..status.len().min(1)]).to_string(),
                additions: None,
                deletions: None,
                binary: false,
            });
        } else if !field.is_empty() {
            // `added TAB deleted TAB path`; `-` `-` for binary.
            let rec = String::from_utf8_lossy(field);
            let mut cols = rec.splitn(3, '\t');
            if let (Some(a), Some(d), Some(path)) = (cols.next(), cols.next(), cols.next()) {
                let (a, d) = (a.parse::<u64>().ok(), d.parse::<u64>().ok());
                additions += a.unwrap_or(0);
                deletions += d.unwrap_or(0);
                counts.insert(path.to_string(), (a, d));
            }
        }
    }
    for f in &mut files {
        if let Some((a, d)) = counts.get(&f.path) {
            f.additions = *a;
            f.deletions = *d;
            f.binary = a.is_none() && d.is_none();
        }
    }
    let mut clipped = files.len() > DIFF_MAX_FILES;
    files.truncate(DIFF_MAX_FILES);
    let mut patch = String::from_utf8_lossy(patch).to_string();
    if patch.len() > DIFF_MAX_BYTES {
        // Cut at the end of the last whole line that fits.
        let end = patch[..DIFF_MAX_BYTES]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        patch.truncate(end);
        clipped = true;
    }
    Ok(JobDiff {
        files,
        additions,
        deletions,
        patch_path: None,
        clipped,
        head_oid: None,
        complete: true,
        patch,
    })
}

pub fn depth(workspace: &Path, conversation: &str) -> usize {
    git_dir(workspace)
        .map(|d| load(&d, conversation).len())
        .unwrap_or(0)
}

/// The live folder, if any, that shares files with `workspace`: the same
/// folder, one inside it, or one it is inside (t3code's
/// `isRestoreWorkspaceIsolated`, MIT, ported as an idea). Compared through
/// [`FolderKey`], so a symlink, `\\?\` or another letter case is the same
/// folder.
pub fn shared_with(workspace: &Path, live: &[PathBuf]) -> Option<PathBuf> {
    let key = FolderKey::of(workspace);
    live.iter()
        .find(|p| FolderKey::of(p).overlaps(&key))
        .cloned()
}

/// Puts back what the last writing turn of this conversation changed.
///
/// Refused, with nothing touched:
/// - `ERR_UNDO_SHARED` while another conversation or job works in a folder
///   that overlaps this one (`live`: their folders; the caller leaves this
///   conversation out). Its files are in the same tree, and restoring now
///   would erase work in progress (W5).
/// - `ERR_UNDO_CONFLICT` when a file this turn changed was changed again
///   after it (by another agent or by hand).
///
/// When the turn's end is known (`after`), only the paths the turn itself
/// changed go back: work that other jobs did later in the same folder keeps
/// every byte, even when they have finished. An entry without `after` (the
/// turn never ended) falls back to putting the whole tree back.
pub async fn undo(workspace: &Path, conversation: &str, live: &[PathBuf]) -> Result<Undone, String> {
    if let Some(other) = shared_with(workspace, live) {
        return Err(format!(
            "{ERR_UNDO_SHARED}: another conversation or job is working in {} right now, which shares files with this folder; restoring them could erase its work. Revert the conversation only, or undo when it ends",
            other.display()
        ));
    }
    let dir = ensure(workspace).await?;
    let mut stack = load(&dir, conversation);
    let entry = stack.last().cloned().ok_or_else(|| {
        format!("{ERR_NOTHING_TO_UNDO}: no turn of this conversation wrote files")
    })?;
    let now = stage_tree(&dir, workspace).await?;
    let changed = match &entry.after {
        Some(after) => {
            let own = changed_paths(&dir, workspace, &entry.tree, after).await?;
            let since = changed_paths(&dir, workspace, after, &now).await?;
            let clash: Vec<&str> = own
                .iter()
                .filter(|p| since.contains(p))
                .map(String::as_str)
                .collect();
            if !clash.is_empty() {
                return Err(format!(
                    "{ERR_UNDO_CONFLICT}: changed again after this turn, so undoing would overwrite that: {}",
                    clash.join(", ")
                ));
            }
            if !own.is_empty() {
                let target = graft(&dir, workspace, &now, &entry.tree, &own).await?;
                git(&dir, workspace, &["read-tree", "--reset", "-u", &target]).await?;
            }
            own
        }
        None => {
            let changed = changed_paths(&dir, workspace, &entry.tree, &now).await?;
            git(
                &dir,
                workspace,
                &["read-tree", "--reset", "-u", &entry.tree],
            )
            .await?;
            changed
        }
    };
    stack.pop();
    store(&dir, conversation, &stack);
    Ok(Undone {
        files: changed,
        messages_before: entry.messages_before,
    })
}

/// "Revert the conversation only": drops the last entry (its bubbles go,
/// the caller truncates them) and leaves every file as it is.
pub async fn forget_last(workspace: &Path, conversation: &str) -> Result<Undone, String> {
    let dir = git_dir(workspace)?;
    let mut stack = if dir.join("HEAD").exists() {
        load(&dir, conversation)
    } else {
        Vec::new()
    };
    let entry = stack.pop().ok_or_else(|| {
        format!("{ERR_NOTHING_TO_UNDO}: no turn of this conversation wrote files")
    })?;
    store(&dir, conversation, &stack);
    Ok(Undone {
        files: Vec::new(),
        messages_before: entry.messages_before,
    })
}

/// Paths that differ between two trees (renames as delete + add).
async fn changed_paths(
    dir: &Path,
    workspace: &Path,
    from: &str,
    to: &str,
) -> Result<Vec<String>, String> {
    let out = git_io(
        dir,
        workspace,
        &["diff", "--name-only", "-z", "--no-renames", from, to],
        None,
        None,
    )
    .await?;
    Ok(out
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).to_string())
        .collect())
}

/// The tree `now` with `paths` as they were in `from` (absent there =
/// removed), built in a private index so the shadow index stays at `now` and
/// `read-tree --reset -u` then touches only those paths on disk.
async fn graft(
    dir: &Path,
    workspace: &Path,
    now: &str,
    from: &str,
    paths: &[String],
) -> Result<String, String> {
    let index = dir.join(format!("omniget-undo-{}.index", uuid::Uuid::new_v4().simple()));
    let out = async {
        git_io(dir, workspace, &["read-tree", now], None, Some(&index)).await?;
        let listing =
            git_io(dir, workspace, &["ls-tree", "-r", "-z", "--full-tree", from], None, None)
                .await?;
        let wanted: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        let mut info: Vec<u8> = Vec::new();
        let mut present = std::collections::HashSet::new();
        for rec in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let rec = String::from_utf8_lossy(rec);
            if let Some((meta, path)) = rec.split_once('\t') {
                if wanted.contains(path) {
                    info.extend_from_slice(format!("{meta}\t{path}\0").as_bytes());
                    present.insert(path.to_string());
                }
            }
        }
        for path in paths.iter().filter(|p| !present.contains(*p)) {
            // Mode 0 removes the path from the index.
            info.extend_from_slice(
                format!("0 0000000000000000000000000000000000000000\t{path}\0").as_bytes(),
            );
        }
        git_io(
            dir,
            workspace,
            &["update-index", "-z", "--index-info"],
            Some(&info),
            Some(&index),
        )
        .await?;
        let tree = git_io(dir, workspace, &["write-tree"], None, Some(&index)).await?;
        Ok::<String, String>(String::from_utf8_lossy(&tree).trim().to_string())
    }
    .await;
    let _ = std::fs::remove_file(&index);
    out
}

#[cfg(test)]
pub(crate) mod undo_tests {
    use super::*;

    /// One shadow root for every test of this crate: `ROOT` is global, and
    /// two tests pointing it at different places would lose each other's
    /// stacks.
    pub(crate) fn test_root() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            set_root(std::env::temp_dir().join(format!("omniget-snap-tests-{}", std::process::id())));
        });
    }

    pub(crate) fn folder(name: &str) -> PathBuf {
        test_root();
        let dir = std::env::temp_dir().join(format!("omniget-undo-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// One writing turn, the way the Coordinator brackets a CLI turn.
    async fn turn(ws: &Path, conv: &str, req: &str, write: impl FnOnce()) {
        before_write(ws, conv, req).await.unwrap();
        write();
        after_turn(ws, conv, req).await.unwrap();
    }

    fn put(ws: &Path, name: &str, text: &str) {
        std::fs::write(ws.join(name), text).unwrap();
    }

    fn read(ws: &Path, name: &str) -> Option<Vec<u8>> {
        std::fs::read(ws.join(name)).ok()
    }

    #[tokio::test]
    async fn undo_refuses_while_another_live_job_uses_the_folder() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("live");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        put(&ws, "keep.txt", "base\n");
        turn(&ws, "conv-a", "r-a", || put(&ws, "a.txt", "from A\n")).await;
        let parent = ws.parent().unwrap().to_path_buf();
        for live in [ws.clone(), ws.join("sub"), parent] {
            let err = undo(&ws, "conv-a", std::slice::from_ref(&live)).await.unwrap_err();
            assert!(err.starts_with(ERR_UNDO_SHARED), "{live:?}: {err}");
            assert_eq!(read(&ws, "a.txt").as_deref(), Some(&b"from A\n"[..]), "nothing touched");
        }
        // A sibling folder is not shared: the undo goes through.
        let sibling = folder("live-sibling");
        let out = undo(&ws, "conv-a", &[sibling]).await.unwrap();
        assert_eq!(out.files, vec!["a.txt".to_string()]);
        assert!(read(&ws, "a.txt").is_none());
    }

    /// W5: B wrote after A (A was done, then B ran and finished). Undoing A
    /// puts A's file back and leaves every byte of B's work.
    #[tokio::test]
    async fn undo_after_another_job_finished_restores_only_its_own_files() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("after");
        put(&ws, "shared.txt", "base\n");
        turn(&ws, "conv-a", "r-a", || {
            put(&ws, "a.txt", "A\n");
            put(&ws, "shared.txt", "base\nA edit\n");
        })
        .await;
        turn(&ws, "conv-b", "r-b", || {
            put(&ws, "b.txt", "B\n");
            std::fs::create_dir_all(ws.join("b-dir")).unwrap();
            put(&ws, "b-dir/deep.txt", "B deep\n");
        })
        .await;
        let b_before = (read(&ws, "b.txt"), read(&ws, "b-dir/deep.txt"));
        let out = undo(&ws, "conv-a", &[]).await.unwrap();
        let mut files = out.files.clone();
        files.sort();
        assert_eq!(files, vec!["a.txt".to_string(), "shared.txt".to_string()]);
        assert!(read(&ws, "a.txt").is_none());
        assert_eq!(read(&ws, "shared.txt").as_deref(), Some(&b"base\n"[..]));
        assert_eq!((read(&ws, "b.txt"), read(&ws, "b-dir/deep.txt")), b_before, "0 bytes of B changed");
    }

    #[tokio::test]
    async fn undo_refuses_when_its_file_changed_after_the_turn() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("conflict");
        turn(&ws, "conv-a", "r-a", || put(&ws, "same.txt", "A\n")).await;
        turn(&ws, "conv-b", "r-b", || put(&ws, "same.txt", "A\nB\n")).await;
        let err = undo(&ws, "conv-a", &[]).await.unwrap_err();
        assert!(err.starts_with(ERR_UNDO_CONFLICT), "{err}");
        assert!(err.contains("same.txt"), "{err}");
        assert_eq!(read(&ws, "same.txt").as_deref(), Some(&b"A\nB\n"[..]));
        assert_eq!(depth(&ws, "conv-a"), 1, "the entry stays for a later decision");
    }

    #[tokio::test]
    async fn forget_last_keeps_the_files() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("forget");
        mark_turn("r-f", 3);
        turn(&ws, "conv-f", "r-f", || put(&ws, "f.txt", "F\n")).await;
        let out = forget_last(&ws, "conv-f").await.unwrap();
        assert!(out.files.is_empty());
        assert_eq!(out.messages_before, Some(3));
        assert_eq!(read(&ws, "f.txt").as_deref(), Some(&b"F\n"[..]));
        assert_eq!(depth(&ws, "conv-f"), 0);
        assert!(forget_last(&ws, "conv-f").await.unwrap_err().starts_with(ERR_NOTHING_TO_UNDO));
    }

    /// A turn that never ended (no tree after it) keeps the old behaviour:
    /// the whole folder goes back, still behind the live guard.
    #[tokio::test]
    async fn an_entry_without_after_still_undoes_the_whole_tree() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("noafter");
        before_write(&ws, "conv-n", "r-n").await.unwrap();
        put(&ws, "n.txt", "N\n");
        let out = undo(&ws, "conv-n", &[]).await.unwrap();
        assert_eq!(out.files, vec!["n.txt".to_string()]);
        assert!(read(&ws, "n.txt").is_none());
    }
}

#[cfg(test)]
mod job_diff_tests {
    use super::undo_tests::folder;
    use super::*;

    async fn turn(ws: &Path, conv: &str, req: &str, write: impl FnOnce()) {
        before_write(ws, conv, req).await.unwrap();
        write();
        after_turn(ws, conv, req).await.unwrap();
    }

    fn put(ws: &Path, name: &str, bytes: &[u8]) {
        if let Some(parent) = ws.join(name).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(ws.join(name), bytes).unwrap();
    }

    fn reqs(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn by_path(d: &JobDiff) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            d.files.iter().map(|f| (f.path.clone(), f.status.clone())).collect();
        v.sort();
        v
    }

    /// Two turns of one job (a resumed job): the base is before the first,
    /// the tip after the last.
    #[tokio::test]
    async fn a_job_diff_spans_its_first_and_last_turn() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-span");
        put(&ws, "keep.txt", b"base\n");
        turn(&ws, "conv-j", "r1", || put(&ws, "a.txt", b"one\n")).await;
        turn(&ws, "conv-j", "r2", || {
            put(&ws, "a.txt", b"one\ntwo\n");
            put(&ws, "dir/b.txt", b"b\n");
        })
        .await;
        let d = job_diff(&ws, "conv-j", &reqs(&["r2", "r1"])).await.unwrap();
        assert_eq!(
            by_path(&d),
            vec![("a.txt".into(), "A".into()), ("dir/b.txt".into(), "A".into())]
        );
        assert_eq!((d.additions, d.deletions), (3, 0));
        assert!(d.complete && !d.clipped && d.head_oid.is_none());
        assert!(d.patch.contains("+++ b/dir/b.txt"), "{}", d.patch);
        // The second turn alone: a.txt changed, b.txt added.
        let d2 = job_diff(&ws, "conv-j", &reqs(&["r2"])).await.unwrap();
        assert_eq!(
            by_path(&d2),
            vec![("a.txt".into(), "M".into()), ("dir/b.txt".into(), "A".into())]
        );
        assert_eq!((d2.additions, d2.deletions), (2, 0));
    }

    #[tokio::test]
    async fn a_job_without_a_snapshot_is_unavailable_never_empty() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        // No shadow git for the folder at all.
        let bare = folder("jd-bare");
        let err = job_diff(&bare, "conv-x", &reqs(&["r1"])).await.unwrap_err();
        assert!(err.starts_with(ERR_DIFF_UNAVAILABLE), "{err}");
        // A shadow git, but not a turn of this job.
        let ws = folder("jd-missing");
        turn(&ws, "conv-j", "r1", || put(&ws, "a.txt", b"a\n")).await;
        let err = job_diff(&ws, "conv-j", &reqs(&["r9"])).await.unwrap_err();
        assert!(err.starts_with(ERR_DIFF_UNAVAILABLE), "{err}");
        // One turn with a snapshot, one without: not a partial answer.
        let err = job_diff(&ws, "conv-j", &reqs(&["r1", "r9"])).await.unwrap_err();
        assert!(err.starts_with(ERR_DIFF_UNAVAILABLE) && err.contains("1 of 2"), "{err}");
        let err = job_diff(&ws, "conv-j", &[]).await.unwrap_err();
        assert!(err.starts_with(ERR_DIFF_UNAVAILABLE), "{err}");
    }

    #[tokio::test]
    async fn a_job_that_changed_nothing_is_available_and_empty() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-read");
        put(&ws, "a.txt", b"a\n");
        turn(&ws, "conv-r", "r1", || {}).await;
        let d = job_diff(&ws, "conv-r", &reqs(&["r1"])).await.unwrap();
        assert!(d.files.is_empty() && d.patch.is_empty());
        assert_eq!((d.additions, d.deletions), (0, 0));
        assert!(d.complete);
    }

    #[tokio::test]
    async fn binary_files_are_counted_as_binary() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-bin");
        turn(&ws, "conv-b", "r1", || {
            put(&ws, "__pycache__/m.pyc", &[0u8, 159, 146, 150, 0, 1, 2, 3]);
            put(&ws, "t.txt", b"text\n");
        })
        .await;
        let d = job_diff(&ws, "conv-b", &reqs(&["r1"])).await.unwrap();
        let bin = d.files.iter().find(|f| f.path == "__pycache__/m.pyc").unwrap();
        assert!(bin.binary && bin.additions.is_none() && bin.deletions.is_none(), "{bin:?}");
        let txt = d.files.iter().find(|f| f.path == "t.txt").unwrap();
        assert!(!txt.binary && txt.additions == Some(1));
        assert_eq!(d.additions, 1, "binary files add no lines");
        assert!(d.patch.contains("Binary files"), "{}", d.patch);
    }

    #[tokio::test]
    async fn a_big_patch_is_clipped_at_256_kb_on_a_line() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-big");
        let big: String = (0..20_000).map(|i| format!("line number {i:08}\n")).collect();
        assert!(big.len() > DIFF_MAX_BYTES);
        turn(&ws, "conv-g", "r1", || put(&ws, "big.txt", big.as_bytes())).await;
        let d = job_diff(&ws, "conv-g", &reqs(&["r1"])).await.unwrap();
        assert!(d.clipped);
        assert!(d.patch.len() <= DIFF_MAX_BYTES && d.patch.ends_with('\n'));
        assert_eq!(d.additions, 20_000, "the totals are never clipped");
    }

    /// A running job (its turn has not recorded its end) is diffed against
    /// the folder now, through a private index: the shared shadow index,
    /// which the Coordinator's `after_turn` writes, is left as it was.
    #[tokio::test]
    async fn a_running_job_is_diffed_without_touching_the_shared_index() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-live");
        put(&ws, "a.txt", b"a\n");
        before_write(&ws, "conv-l", "r1").await.unwrap();
        put(&ws, "b.txt", b"b\n");
        let dir = git_dir(&ws).unwrap();
        let index = std::fs::read(dir.join("index")).unwrap();
        let d = job_diff(&ws, "conv-l", &reqs(&["r1"])).await.unwrap();
        assert!(!d.complete);
        assert_eq!(by_path(&d), vec![("b.txt".into(), "A".into())]);
        assert_eq!(std::fs::read(dir.join("index")).unwrap(), index, "shared index untouched");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("omniget-jobdiff-"))
            .collect();
        assert!(leftovers.is_empty(), "the private index is removed");
    }

    #[tokio::test]
    async fn adds_and_deletes_are_plain_a_and_d() {
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        let ws = folder("jd-mv");
        put(&ws, "old.txt", b"same content\nsecond line\n");
        turn(&ws, "conv-m", "r1", || {
            std::fs::rename(ws.join("old.txt"), ws.join("new.txt")).unwrap();
        })
        .await;
        let d = job_diff(&ws, "conv-m", &reqs(&["r1"])).await.unwrap();
        assert_eq!(
            by_path(&d),
            vec![("new.txt".into(), "A".into()), ("old.txt".into(), "D".into())]
        );
        assert_eq!((d.additions, d.deletions), (2, 2));
        assert!(d.patch.contains("deleted file mode") && d.patch.contains("new file mode"));
    }
}

// ── BN4 (scripts/llm/diff_eval.py) ─────────────────────────────────────
//
// Level: `scripts/llm/fake_claude.py --mode apply` spawned through the real
// `spawn_turn_with` and read by the real `ClaudeParser`, bracketed by the
// real snapshot the way the Coordinator brackets a CLI turn (`before_write`
// before, `after_turn` after), then the real `job_diff`. The fake edits the
// folder from its own process, like an agent's shell (nothing goes through
// OmniGet's tools). Not in it: `Jobs::run_job` and the Tauri app (no window
// here); `run_job` calls `job_diff` with the same request ids after the
// stream ends. With `BN4_FIXTURES` the script's fixtures are used (each dir:
// `seed/`, `changes.json`; this writes `work/`, `job.patch`, `job.json`);
// without it a small built-in set runs, so `cargo test` checks something.
#[cfg(test)]
mod bn4 {
    use super::undo_tests::test_root;
    use super::*;
    use crate::core::llm::cli_runtime::claude::ClaudeParser;
    use crate::core::llm::cli_runtime::session::{spawn_turn_with, SpawnSpec, TurnHooks};
    use crate::core::llm::types::{FinishReason, TurnEvent};
    use futures::StreamExt;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio_util::sync::CancellationToken;

    fn bench(metric: &str, value: serde_json::Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(
                    f,
                    "{}",
                    serde_json::json!({ "bench": "bn4", "metric": metric, "value": value, "level": "turn+snapshot+job_diff" })
                );
            }
        }
    }

    fn fake() -> Option<PathBuf> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_claude.py");
        let python = std::process::Command::new("python3").arg("--version").output().ok()?;
        (python.status.success() && script.is_file()).then_some(script)
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let (src, dst) = (e.path(), to.join(e.file_name()));
            if src.is_dir() {
                copy_dir(&src, &dst);
            } else {
                std::fs::copy(&src, &dst).unwrap();
            }
        }
    }

    /// One CLI turn of a job the way the Coordinator runs it; true when the
    /// stream ended with `Finished{Stop}`.
    async fn cli_turn(fake: &Path, ws: &Path, conv: &str, req: &str, env: &[(&str, String)]) -> bool {
        before_write(ws, conv, req).await.unwrap();
        let spec = SpawnSpec {
            program: PathBuf::from("python3"),
            args: vec![
                fake.display().to_string(),
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
            env: env.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            cwd: Some(ws.to_path_buf()),
            stdin: Some("User: do the task".into()),
            ..SpawnSpec::default()
        };
        let mut stream = spawn_turn_with(
            spec,
            Arc::new(ClaudeParser::new(true)),
            CancellationToken::new(),
            TurnHooks::default(),
        )
        .unwrap();
        let mut ok = false;
        while let Some(ev) = stream.next().await {
            if matches!(ev, TurnEvent::Finished { reason: FinishReason::Stop }) {
                ok = true;
            }
        }
        after_turn(ws, conv, req).await.unwrap();
        ok
    }

    /// Built-in fixtures when the script did not provide any: 50 files
    /// changed out of 60 (40 edited, 5 added, 5 deleted) and a binary.
    fn builtin(root: &Path) {
        let f50 = root.join("f50");
        let mut write = serde_json::Map::new();
        let mut delete = Vec::new();
        for i in 0..60 {
            let body: String = (0..40).map(|l| format!("file {i} line {l}\n")).collect();
            let rel = format!("src/m{:02}/f{i:02}.txt", i % 6);
            let seed = f50.join("seed").join(&rel);
            std::fs::create_dir_all(seed.parent().unwrap()).unwrap();
            std::fs::write(&seed, &body).unwrap();
            if i < 40 {
                write.insert(rel, body.replace(" line 7\n", " line 7 (edited)\n").into());
            } else if i < 45 {
                delete.push(serde_json::Value::from(rel));
            }
        }
        for i in 0..5 {
            write.insert(format!("new/n{i}.txt"), format!("new file {i}\n").into());
        }
        std::fs::write(
            f50.join("changes.json"),
            serde_json::json!({"write": write, "delete": delete}).to_string(),
        )
        .unwrap();
        let bin = root.join("bin");
        std::fs::create_dir_all(bin.join("seed")).unwrap();
        std::fs::write(bin.join("seed/a.txt"), "a\n").unwrap();
        std::fs::write(
            bin.join("changes.json"),
            serde_json::json!({"write": {"a.txt": "a\nb\n"}, "write_b64": {"x.bin": "AAECAwQAAQID"}}).to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn bn4_job_diff_matches_the_folder() {
        let Some(fake) = fake() else {
            return eprintln!("python3 or fake_claude.py missing: skipped");
        };
        if !git_available() {
            return eprintln!("git missing: skipped");
        }
        test_root();
        let reps: usize = std::env::var("BN4_TIMING_REPS").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
        let root = match std::env::var("BN4_FIXTURES") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                let dir = std::env::temp_dir().join(format!("omniget-bn4-{}", uuid::Uuid::new_v4()));
                builtin(&dir);
                dir
            }
        };
        let mut names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| e.path().join("seed").is_dir() && e.path().join("changes.json").is_file())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no fixtures in {}", root.display());
        let mut writing_jobs = 0;
        let mut with_files = 0;
        for name in &names {
            let dir = root.join(name);
            let work = dir.join("work");
            let _ = std::fs::remove_dir_all(&work);
            copy_dir(&dir.join("seed"), &work);
            let work = work.canonicalize().unwrap();
            let (conv, req) = (format!("bn4-{name}"), format!("bn4-{name}-{}", uuid::Uuid::new_v4().simple()));
            let changes = dir.join("changes.json").display().to_string();
            let ok = cli_turn(
                &fake,
                &work,
                &conv,
                &req,
                &[("FAKE_CLAUDE_MODE", "apply".into()), ("FAKE_CLAUDE_CHANGES", changes)],
            )
            .await;
            let requests = vec![req.clone()];
            let mut times = Vec::new();
            let mut diff = None;
            for _ in 0..reps.max(1) {
                let t = Instant::now();
                diff = Some(job_diff(&work, &conv, &requests).await.unwrap());
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let d = diff.unwrap();
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (p50, max) = (times[times.len() / 2], times[times.len() - 1]);
            std::fs::write(dir.join("job.patch"), &d.patch).unwrap();
            std::fs::write(
                dir.join("job.json"),
                serde_json::json!({
                    "turn_ok": ok, "files": d.files, "additions": d.additions, "deletions": d.deletions,
                    "clipped": d.clipped, "complete": d.complete, "job_diff_ms_p50": p50, "job_diff_ms_max": max,
                })
                .to_string(),
            )
            .unwrap();
            writing_jobs += 1;
            with_files += usize::from(!d.files.is_empty());
            bench(&format!("{name}_turn_ok"), ok.into());
            bench(&format!("{name}_files"), d.files.len().into());
            bench(&format!("{name}_job_diff_ms_p50"), p50.into());
            bench(&format!("{name}_job_diff_ms_max"), max.into());
            assert!(ok, "{name}: the fake turn did not finish");
            assert!(!d.files.is_empty(), "{name}: a writing job has files");
            assert!(d.complete, "{name}: the turn recorded its end");
        }
        bench("writing_jobs", writing_jobs.into());
        bench("writing_jobs_with_files", with_files.into());

        // Controls. A job that wrote nothing (its turn took a snapshot):
        // available and empty. A job whose folder has no snapshot:
        // unavailable, never an empty diff.
        let read = std::env::temp_dir().join(format!("omniget-bn4-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&read).unwrap();
        std::fs::write(read.join("a.txt"), "a\n").unwrap();
        let read = read.canonicalize().unwrap();
        let ok = cli_turn(&fake, &read, "bn4-read", "bn4-read-req", &[("FAKE_CLAUDE_MODE", "ok".into())]).await;
        let d = job_diff(&read, "bn4-read", &["bn4-read-req".to_string()]).await.unwrap();
        bench("control_readonly_available_files", serde_json::json!({"turn_ok": ok, "files": d.files.len()}));
        assert!(d.files.is_empty());
        let bare = std::env::temp_dir().join(format!("omniget-bn4-bare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&bare).unwrap();
        let err = job_diff(&bare, "bn4-bare", &["r".to_string()]).await.unwrap_err();
        bench("control_nosnap", err.split(':').next().unwrap_or("").into());
        assert!(err.starts_with(ERR_DIFF_UNAVAILABLE));
    }
}
