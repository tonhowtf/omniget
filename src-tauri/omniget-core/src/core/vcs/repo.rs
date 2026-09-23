//! Repository discovery and read-only queries: root, branch, remote, porcelain
//! v2 status, ahead/behind, branches and log.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::runner::{redact, Invocation, VcsError, VcsResult};

/// Where git commands run. `Repo` is a normal checkout (main or linked
/// worktree); `Shadow` is a private git dir under the app data dir whose work
/// tree is a folder that has no git of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCtx {
    Repo {
        work_tree: PathBuf,
    },
    Shadow {
        git_dir: PathBuf,
        work_tree: PathBuf,
    },
}

impl GitCtx {
    pub fn work_tree(&self) -> &Path {
        match self {
            GitCtx::Repo { work_tree } | GitCtx::Shadow { work_tree, .. } => work_tree,
        }
    }

    pub fn is_shadow(&self) -> bool {
        matches!(self, GitCtx::Shadow { .. })
    }

    /// A `git` invocation bound to this context.
    pub fn git(&self) -> Invocation {
        match self {
            GitCtx::Repo { work_tree } => Invocation::git().arg("-C").arg(work_tree).cwd(work_tree),
            GitCtx::Shadow { git_dir, work_tree } => Invocation::git()
                .arg("--git-dir")
                .arg(git_dir)
                .arg("--work-tree")
                .arg(work_tree)
                .args(["-c", "core.autocrlf=false"])
                .cwd(work_tree)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE"),
        }
    }

    /// Absolute git dir (per-worktree for a linked worktree).
    pub async fn git_dir(&self) -> VcsResult<PathBuf> {
        match self {
            GitCtx::Shadow { git_dir, .. } => Ok(git_dir.clone()),
            GitCtx::Repo { .. } => {
                let out = self
                    .git()
                    .args(["rev-parse", "--absolute-git-dir"])
                    .run()
                    .await?;
                Ok(PathBuf::from(out.trimmed()))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoPaths {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// A linked worktree (`.git` is a file), not the main checkout.
    pub linked_worktree: bool,
}

/// Finds the repository that contains `path`. `NotRepo` when there is none.
pub async fn discover(path: &Path) -> VcsResult<RepoPaths> {
    if !path.exists() {
        return Err(VcsError::NotFound(format!(
            "{} does not exist",
            path.display()
        )));
    }
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let out = Invocation::git()
        .arg("-C")
        .arg(&dir)
        .cwd(&dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args([
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
        ])
        .unchecked()
        .run()
        .await?;
    if !out.ok() {
        return Err(VcsError::NotRepo {
            path: dir.display().to_string(),
        });
    }
    let text = out.text();
    let mut lines = text.lines();
    let root = lines
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| VcsError::NotRepo {
            path: dir.display().to_string(),
        })?;
    let git_dir = lines
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".git"));
    let common = lines
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| git_dir.clone());
    let common_dir = if common.is_absolute() {
        common
    } else {
        dir.join(common)
    };
    let common_dir = std::fs::canonicalize(&common_dir).unwrap_or(common_dir);
    let git_dir_c = std::fs::canonicalize(&git_dir).unwrap_or(git_dir.clone());
    Ok(RepoPaths {
        linked_worktree: root.join(".git").is_file() && git_dir_c != common_dir,
        root,
        git_dir,
        common_dir,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// Index (staged) state letter, `.` when unchanged.
    pub index: char,
    /// Work tree state letter, `.` when unchanged.
    pub worktree: char,
    /// modified | added | deleted | renamed | copied | typechange | untracked | conflict | ignored
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusBranch {
    pub oid: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Porcelain {
    pub branch: StatusBranch,
    pub files: Vec<StatusFile>,
}

fn kind_of(x: char, y: char) -> &'static str {
    let pick = if x != '.' { x } else { y };
    match pick {
        'M' => "modified",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "typechange",
        _ => "modified",
    }
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse_porcelain_v2(raw: &[u8]) -> Porcelain {
    let mut p = Porcelain::default();
    let recs: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut i = 0;
    while i < recs.len() {
        let rec = String::from_utf8_lossy(recs[i]).into_owned();
        i += 1;
        if rec.is_empty() {
            continue;
        }
        if let Some(h) = rec.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" => p.branch.oid = (v != "(initial)").then(|| v.to_string()),
                "branch.head" => {
                    if v == "(detached)" {
                        p.branch.detached = true;
                    } else {
                        p.branch.head = Some(v.to_string());
                    }
                }
                "branch.upstream" => p.branch.upstream = Some(v.to_string()),
                "branch.ab" => {
                    for part in v.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            p.branch.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            p.branch.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = rec.chars().next().unwrap_or(' ');
        match kind {
            '1' => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if f.len() == 9 {
                    let xy: Vec<char> = f[1].chars().collect();
                    let (x, y) = (
                        xy.first().copied().unwrap_or('.'),
                        xy.get(1).copied().unwrap_or('.'),
                    );
                    p.files.push(StatusFile {
                        path: f[8].to_string(),
                        orig_path: None,
                        index: x,
                        worktree: y,
                        kind: kind_of(x, y).into(),
                    });
                }
            }
            '2' => {
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                let orig = recs.get(i).map(|b| String::from_utf8_lossy(b).into_owned());
                i += 1;
                if f.len() == 10 {
                    let xy: Vec<char> = f[1].chars().collect();
                    let (x, y) = (
                        xy.first().copied().unwrap_or('.'),
                        xy.get(1).copied().unwrap_or('.'),
                    );
                    p.files.push(StatusFile {
                        path: f[9].to_string(),
                        orig_path: orig,
                        index: x,
                        worktree: y,
                        kind: kind_of(x, y).into(),
                    });
                }
            }
            'u' => {
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                if f.len() == 11 {
                    let xy: Vec<char> = f[1].chars().collect();
                    p.files.push(StatusFile {
                        path: f[10].to_string(),
                        orig_path: None,
                        index: xy.first().copied().unwrap_or('U'),
                        worktree: xy.get(1).copied().unwrap_or('U'),
                        kind: "conflict".into(),
                    });
                }
            }
            '?' | '!' => {
                p.files.push(StatusFile {
                    path: rec[2..].to_string(),
                    orig_path: None,
                    index: kind,
                    worktree: kind,
                    kind: if kind == '?' {
                        "untracked".into()
                    } else {
                        "ignored".into()
                    },
                });
            }
            _ => {}
        }
    }
    p
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteInfo {
    pub name: String,
    /// Fetch URL with any credential blanked.
    pub url: String,
    /// github | gitlab | bitbucket | azure | forgejo | unknown
    pub host_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoStatus {
    pub is_repo: bool,
    pub root: Option<PathBuf>,
    pub git_dir: Option<PathBuf>,
    pub common_dir: Option<PathBuf>,
    pub linked_worktree: bool,
    /// Lives under OmniGet's own worktrees dir.
    pub omniget_worktree: bool,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub remote: Option<RemoteInfo>,
    pub dirty: bool,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicts: u32,
    pub files: Vec<StatusFile>,
    /// `gh-merge-base` recorded for the branch (base of an OmniGet worktree).
    pub base_branch: Option<String>,
    /// An `index.lock` is present; status was not taken.
    pub index_locked: bool,
}

impl RepoStatus {
    fn none() -> Self {
        RepoStatus {
            is_repo: false,
            root: None,
            git_dir: None,
            common_dir: None,
            linked_worktree: false,
            omniget_worktree: false,
            branch: None,
            head: None,
            detached: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            remote: None,
            dirty: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            files: Vec::new(),
            base_branch: None,
            index_locked: false,
        }
    }
}

/// Classifies a remote URL by its host.
pub fn host_kind(url: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    let host = lower
        .split("://")
        .nth(1)
        .map(|r| r.split('/').next().unwrap_or(""))
        .unwrap_or_else(|| lower.split(':').next().unwrap_or(""));
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    let has = |n: &str| labels.iter().any(|l| *l == n);
    if host == "github.com"
        || host.ends_with(".github.com")
        || has("github")
        || host.ends_with("ghe.com")
    {
        "github"
    } else if has("gitlab") {
        "gitlab"
    } else if host == "bitbucket.org" {
        "bitbucket"
    } else if host.contains("dev.azure.com") || host.ends_with("visualstudio.com") {
        "azure"
    } else if host == "codeberg.org" || has("forgejo") || has("gitea") {
        "forgejo"
    } else {
        "unknown"
    }
}

/// Primary remote: `origin`, else the first one.
pub async fn primary_remote(ctx: &GitCtx) -> VcsResult<Option<RemoteInfo>> {
    let out = ctx.git().arg("remote").unchecked().run().await?;
    let names: Vec<String> = out
        .text()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let Some(name) = names
        .iter()
        .find(|n| *n == "origin")
        .or_else(|| names.first())
        .cloned()
    else {
        return Ok(None);
    };
    let url = ctx
        .git()
        .args(["remote", "get-url", &name])
        .unchecked()
        .run()
        .await?
        .trimmed();
    let url = redact(&url);
    Ok(Some(RemoteInfo {
        host_kind: host_kind(&url).into(),
        name,
        url,
    }))
}

pub async fn config_get(ctx: &GitCtx, key: &str) -> Option<String> {
    let out = ctx
        .git()
        .args(["config", "--get", key])
        .unchecked()
        .run()
        .await
        .ok()?;
    let v = out.trimmed();
    (out.ok() && !v.is_empty()).then_some(v)
}

/// Full status of the checkout that contains `path`. A path outside any repo
/// answers `is_repo: false` instead of an error.
pub async fn status(path: &Path) -> VcsResult<RepoStatus> {
    let paths = match discover(path).await {
        Ok(p) => p,
        Err(VcsError::NotRepo { .. }) => return Ok(RepoStatus::none()),
        Err(e) => return Err(e),
    };
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let mut st = RepoStatus::none();
    st.is_repo = true;
    st.omniget_worktree = super::worktree::is_omniget_worktree(&paths.root);
    st.linked_worktree = paths.linked_worktree;
    st.root = Some(paths.root.clone());
    st.git_dir = Some(paths.git_dir.clone());
    st.common_dir = Some(paths.common_dir.clone());
    st.remote = primary_remote(&ctx).await?;

    if paths.git_dir.join("index.lock").exists() {
        st.index_locked = true;
        let head = ctx
            .git()
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unchecked()
            .run()
            .await?;
        st.branch = head.ok().then(|| head.trimmed());
        st.detached = st.branch.is_none();
        return Ok(st);
    }

    let out = ctx
        .git()
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
        ])
        .timeout(Some(std::time::Duration::from_secs(60)))
        .run()
        .await?;
    let p = parse_porcelain_v2(&out.stdout);
    st.branch = p.branch.head.clone();
    st.head = p.branch.oid.clone();
    st.detached = p.branch.detached;
    st.upstream = p.branch.upstream.clone();
    st.ahead = p.branch.ahead;
    st.behind = p.branch.behind;
    for f in &p.files {
        match f.kind.as_str() {
            "untracked" => st.untracked += 1,
            "conflict" => st.conflicts += 1,
            "ignored" => {}
            _ => {
                if f.index != '.' {
                    st.staged += 1;
                }
                if f.worktree != '.' {
                    st.unstaged += 1;
                }
            }
        }
    }
    st.dirty = st.staged + st.unstaged + st.untracked + st.conflicts > 0;
    st.files = p.files;
    if let Some(b) = &st.branch {
        st.base_branch = config_get(&ctx, &format!("branch.{b}.gh-merge-base")).await;
    }
    Ok(st)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub oid: String,
    pub upstream: Option<String>,
    pub updated_at: i64,
    /// Checked out in some worktree (path), if any.
    pub worktree: Option<PathBuf>,
}

pub async fn branches(path: &Path) -> VcsResult<Vec<BranchInfo>> {
    let paths = discover(path).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root,
    };
    let out = ctx
        .git()
        .args([
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(objectname)%00%(upstream:short)%00%(committerdate:unix)%00%(HEAD)%00%(worktreepath)",
            "refs/heads",
            "refs/remotes",
        ])
        .run()
        .await?;
    let mut list = Vec::new();
    for line in out.text().lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 5 {
            continue;
        }
        let full = f[0];
        let (name, remote) = if let Some(n) = full.strip_prefix("refs/heads/") {
            (n, false)
        } else if let Some(n) = full.strip_prefix("refs/remotes/") {
            if n.ends_with("/HEAD") {
                continue;
            }
            (n, true)
        } else {
            continue;
        };
        list.push(BranchInfo {
            name: name.to_string(),
            remote,
            current: f[4].trim() == "*",
            oid: f[1].to_string(),
            upstream: (!f[2].is_empty()).then(|| f[2].to_string()),
            updated_at: f[3].parse().unwrap_or(0),
            worktree: f.get(5).filter(|s| !s.is_empty()).map(PathBuf::from),
        });
    }
    list.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then(a.remote.cmp(&b.remote))
            .then(b.updated_at.cmp(&a.updated_at))
    });
    Ok(list)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub oid: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub subject: String,
    pub refs: String,
}

pub async fn log(path: &Path, n: u32, rev: Option<&str>) -> VcsResult<Vec<LogEntry>> {
    let paths = discover(path).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root,
    };
    let head = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .unchecked()
        .run()
        .await?;
    if !head.ok() && rev.is_none() {
        return Ok(Vec::new());
    }
    let mut inv = ctx
        .git()
        .arg("log")
        .arg(format!("-n{}", n.clamp(1, 5000)))
        .arg("--format=%H%x00%h%x00%an%x00%ae%x00%at%x00%D%x00%s%x1e");
    if let Some(r) = rev {
        if r.starts_with('-') {
            return Err(VcsError::Invalid(format!("bad revision {r}")));
        }
        inv = inv.arg(r);
    }
    let out = inv.arg("--").run().await?;
    Ok(out
        .text()
        .split('\u{1e}')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            let f: Vec<&str> = rec.split('\0').collect();
            (f.len() >= 7).then(|| LogEntry {
                oid: f[0].into(),
                short: f[1].into(),
                author: f[2].into(),
                email: f[3].into(),
                at: f[4].parse().unwrap_or(0),
                refs: f[5].into(),
                subject: f[6].trim_end().into(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_v2() {
        let raw = b"# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaa aaa src/a b.rs\0\
2 R. N... 100644 100644 100644 aaa aaa R100 new.rs\0old.rs\0\
? tmp/x.txt\0";
        let p = parse_porcelain_v2(raw);
        assert_eq!(p.branch.head.as_deref(), Some("main"));
        assert_eq!((p.branch.ahead, p.branch.behind), (2, 1));
        assert_eq!(p.files.len(), 3);
        assert_eq!(p.files[0].path, "src/a b.rs");
        assert_eq!(p.files[0].kind, "modified");
        assert_eq!(p.files[1].orig_path.as_deref(), Some("old.rs"));
        assert_eq!(p.files[1].kind, "renamed");
        assert_eq!(p.files[2].kind, "untracked");
    }

    #[test]
    fn hosts() {
        assert_eq!(host_kind("git@github.com:a/b.git"), "github");
        assert_eq!(host_kind("https://***@gitlab.example.com/a/b"), "gitlab");
        assert_eq!(host_kind("https://bitbucket.org/a/b"), "bitbucket");
        assert_eq!(host_kind("/local/path"), "unknown");
    }
}
