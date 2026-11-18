use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::folder_key::FolderKey;

pub const ERR_SNAPSHOT: &str = "ERR_CODE_SNAPSHOT";
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
    #[serde(default)]
    pub messages_before: Option<usize>,
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    #[serde(default)]
    pub binary: bool,
}

pub const DIFF_MAX_BYTES: usize = 256 * 1024;
pub const DIFF_MAX_FILES: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct JobDiff {
    pub files: Vec<FileChange>,
    pub additions: u64,
    pub deletions: u64,
    pub patch_path: Option<PathBuf>,
    pub clipped: bool,
    pub head_oid: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub tip: Option<String>,
    pub complete: bool,
    #[serde(skip)]
    pub patch: String,
}

static MARKS: RwLock<Option<std::collections::HashMap<String, usize>>> = RwLock::new(None);

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
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{ERR_SNAPSHOT}: git: {e}"))?;
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

pub async fn before_write(
    workspace: &Path,
    conversation: &str,
    request: &str,
) -> Result<(), String> {
    let out = before_write_inner(workspace, conversation, request).await;
    note_failure(workspace, out.as_ref().err().map(String::as_str));
    out
}

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

async fn stage_tree_private(dir: &Path, workspace: &Path) -> Result<String, String> {
    let index = dir.join(format!(
        "omniget-jobdiff-{}.index",
        uuid::Uuid::new_v4().simple()
    ));
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
        base: Some(from.to_string()),
        tip: Some(to.to_string()),
        complete: true,
        patch,
    })
}

pub async fn worktree_diff(worktree: &Path, base_oid: &str) -> Result<JobDiff, String> {
    use crate::core::vcs::checkpoint;
    use crate::core::vcs::repo::GitCtx;
    let unavailable = |why: String| format!("{ERR_DIFF_UNAVAILABLE}: {why}");
    if !worktree.join(".git").exists() {
        return Err(unavailable(format!(
            "the worktree {} is gone",
            worktree.display()
        )));
    }
    if base_oid.is_empty() || base_oid.starts_with('-') {
        return Err(unavailable("the job has no base commit".into()));
    }
    let ctx = GitCtx::Repo {
        work_tree: worktree.to_path_buf(),
    };
    let both = ctx
        .git()
        .args(["rev-parse", "--absolute-git-dir", "HEAD^{commit}"])
        .run()
        .await
        .map_err(|e| unavailable(e.to_string()))?
        .text();
    let mut lines = both.lines();
    let (Some(dir), Some(head)) = (lines.next(), lines.next()) else {
        return Err(unavailable("git did not name the worktree's HEAD".into()));
    };
    let (dir, head) = (
        std::path::PathBuf::from(dir.trim()),
        head.trim().to_string(),
    );
    let merge_base = async {
        ctx.git()
            .args(["merge-base", base_oid, &head])
            .run()
            .await
            .map(|o| o.trimmed())
            .map_err(|e| unavailable(format!("no common history with the base: {e}")))
    };
    let staged = async {
        checkpoint::live_tree_at(&ctx, &dir)
            .await
            .map_err(|e| unavailable(e.to_string()))
    };
    let (base, tree) = tokio::join!(merge_base, staged);
    let (base, tree) = (base?, tree?);
    let mut diff = diff_trees(&dir, worktree, &base, &tree).await?;
    diff.head_oid = Some(head);
    diff.base = Some(base);
    diff.tip = Some(tree);
    Ok(diff)
}

pub fn shadow_ctx(workspace: &Path) -> Result<crate::core::vcs::repo::GitCtx, String> {
    let dir = git_dir(workspace)?;
    if !dir.join("HEAD").exists() {
        return Err(format!(
            "{ERR_DIFF_UNAVAILABLE}: no snapshot was ever taken of {}",
            workspace.display()
        ));
    }
    Ok(crate::core::vcs::repo::GitCtx::Shadow {
        git_dir: dir,
        work_tree: workspace.to_path_buf(),
    })
}

pub async fn live_diff(workspace: &Path, base: &str) -> Result<JobDiff, String> {
    let ctx = shadow_ctx(workspace)?;
    let crate::core::vcs::repo::GitCtx::Shadow { git_dir: dir, .. } = ctx else {
        unreachable!("shadow_ctx returns a shadow context");
    };
    if base.is_empty() || base.starts_with('-') {
        return Err(format!("{ERR_DIFF_UNAVAILABLE}: no base tree"));
    }
    let tip = stage_tree_private(&dir, workspace).await?;
    diff_trees(&dir, workspace, base, &tip).await
}

pub fn shared_with(workspace: &Path, live: &[PathBuf]) -> Option<PathBuf> {
    let key = FolderKey::of(workspace);
    live.iter()
        .find(|p| FolderKey::of(p).overlaps(&key))
        .cloned()
}
