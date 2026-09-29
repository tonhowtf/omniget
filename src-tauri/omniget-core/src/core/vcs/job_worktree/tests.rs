//! Job worktrees against real temporary repositories and the system `git`.

use std::path::{Path, PathBuf};

use super::*;

/// The data dir the git tests share, held for the whole test: the env var
/// is process-wide (`test_env`).
fn data_dir() -> crate::core::test_env::Shared {
    crate::core::test_env::shared_data_dir()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "omniget-jobwt-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(d).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(p: &Path, bytes: &[u8]) {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(p, bytes).unwrap();
}

fn job_id() -> String {
    format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

/// A repository with one commit: `a.txt`, `b.txt`, a `.gitignore`.
fn repo(name: &str, gitignore: &str) -> PathBuf {
    let r = scratch(name);
    git(&r, &["init", "-q", "-b", "main"]);
    git(&r, &["config", "user.name", "Test"]);
    git(&r, &["config", "user.email", "test@example.com"]);
    git(&r, &["config", "commit.gpgsign", "false"]);
    write(&r.join("a.txt"), b"one\n");
    write(&r.join("b.txt"), b"bee\n");
    write(&r.join(".gitignore"), gitignore.as_bytes());
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-q", "-m", "init"]);
    r
}

fn worktree_count(repo: &Path) -> usize {
    git(repo, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count()
}

#[test]
fn include_lines_are_literal_repo_paths() {
    let got = parse_worktree_include(
        "# secrets\n.env\n./config/local.json\nconfig/local.json\n\n*.log\n!keep\n../escape\n/abs\n.git/config\nC:/x\ndir/\nsub\\file\n",
    );
    assert_eq!(got, vec![".env", "config/local.json", "dir", "sub/file"]);
}

/// Foco de revisão 3: the job's worktree starts with what the user has not
/// committed (staged, unstaged, untracked), and the user's index, HEAD and
/// files are byte for byte what they were.
#[tokio::test]
async fn the_users_uncommitted_work_is_carried_without_touching_their_index() {
    let _data = data_dir();
    let r = repo("carry", "*.log\n");
    write(&r.join("a.txt"), b"user staged\n");
    git(&r, &["add", "a.txt"]);
    write(&r.join("b.txt"), b"user unstaged\n");
    write(&r.join("new.txt"), b"user untracked\n");
    write(&r.join("debug.log"), b"ignored\n");
    let index_before = std::fs::read(r.join(".git/index")).unwrap();
    let head_before = git(&r, &["rev-parse", "HEAD"]);
    let status_before = git(&r, &["status", "--porcelain"]);

    let id = job_id();
    let wt = create(&r, &id, None).await.unwrap();

    assert_eq!(std::fs::read(r.join(".git/index")).unwrap(), index_before);
    assert_eq!(git(&r, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(&r, &["status", "--porcelain"]), status_before);
    assert_eq!(std::fs::read(r.join("b.txt")).unwrap(), b"user unstaged\n");

    assert_eq!(wt.branch, branch_of(&id));
    assert_eq!(wt.start_oid, head_before);
    assert_ne!(wt.base_oid, wt.start_oid, "a commit carries the work");
    let mut carried = wt.carried.clone();
    carried.sort();
    assert_eq!(carried, vec!["a.txt", "b.txt", "new.txt"]);
    assert_eq!(
        std::fs::read(wt.path.join("a.txt")).unwrap(),
        b"user staged\n"
    );
    assert_eq!(
        std::fs::read(wt.path.join("b.txt")).unwrap(),
        b"user unstaged\n"
    );
    assert_eq!(
        std::fs::read(wt.path.join("new.txt")).unwrap(),
        b"user untracked\n"
    );
    assert!(
        !wt.path.join("debug.log").exists(),
        "ignored files stay out"
    );
    // The job starts clean: its own diff will hold only what it does.
    assert_eq!(git(&wt.path, &["status", "--porcelain"]), "");
    assert_eq!(git(&wt.path, &["rev-parse", "HEAD"]), wt.base_oid);
    assert!(worktree::is_omniget_worktree(&wt.path));

    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
    assert_eq!(std::fs::read(r.join(".git/index")).unwrap(), index_before);
    assert_eq!(git(&r, &["status", "--porcelain"]), status_before);
}

#[tokio::test]
async fn a_named_base_carries_nothing() {
    let _data = data_dir();
    let r = repo("base", "");
    let first = git(&r, &["rev-parse", "HEAD"]);
    write(&r.join("a.txt"), b"two\n");
    git(&r, &["commit", "-qam", "two"]);
    write(&r.join("a.txt"), b"dirty\n");
    let id = job_id();
    let wt = create(&r, &id, Some(&first)).await.unwrap();
    assert_eq!(wt.base_oid, first);
    assert!(wt.carried.is_empty());
    assert_eq!(std::fs::read(wt.path.join("a.txt")).unwrap(), b"one\n");
    assert!(create(&r, &job_id(), Some("--upload-pack=x"))
        .await
        .is_err());
    assert!(create(&r, &job_id(), Some("no-such-ref")).await.is_err());
    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
}

#[tokio::test]
async fn ignored_deps_are_linked_or_copied_and_never_show_in_the_diff() {
    let _data = data_dir();
    let r = repo("deps", "node_modules\n.env\ntarget\n");
    write(
        &r.join("node_modules/dep/index.js"),
        b"module.exports = 1\n",
    );
    write(&r.join(".env"), b"SECRET=1\n");
    write(&r.join("target/debug/x"), b"bin\n");
    write(
        &r.join(".worktreeinclude"),
        b".env\ntarget\nb.txt\nmissing.txt\n",
    );
    git(&r, &["add", ".worktreeinclude"]);
    git(&r, &["commit", "-qm", "include"]);
    let wt = create(&r, &job_id(), None).await.unwrap();
    assert_eq!(wt.shared, vec!["node_modules"]);
    let nm = wt.path.join("node_modules");
    assert!(nm.symlink_metadata().unwrap().file_type().is_symlink() || cfg!(windows));
    assert!(nm.join("dep/index.js").is_file());
    // `.env` and `target` are ignored and listed: copied. `b.txt` is tracked
    // (already there), `missing.txt` does not exist: skipped.
    assert_eq!(wt.copied, vec![".env", "target"]);
    assert_eq!(std::fs::read(wt.path.join(".env")).unwrap(), b"SECRET=1\n");
    assert_eq!(git(&wt.path, &["status", "--porcelain"]), "");
    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
    // The link went, the shared folder did not.
    assert!(r.join("node_modules/dep/index.js").is_file());
}

#[tokio::test]
async fn a_directory_only_pattern_does_not_leak_the_link_into_the_diff() {
    let _data = data_dir();
    let r = repo("deps-slash", "node_modules/\n");
    write(&r.join("node_modules/dep/index.js"), b"1\n");
    let wt = create(&r, &job_id(), None).await.unwrap();
    // Whatever git decides about a link under `node_modules/`, the job's
    // status stays clean.
    assert_eq!(git(&wt.path, &["status", "--porcelain"]), "");
    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
}

/// Removal: the preflight lists the job's files, they are saved on the
/// branch, the folder and the admin entry go, and the branch with work is
/// preserved and reported. A job that did nothing leaves no branch.
#[tokio::test]
async fn removal_saves_the_work_preserves_the_branch_and_leaves_no_orphan() {
    let _data = data_dir();
    let r = repo("remove", "");
    let busy = create(&r, &job_id(), None).await.unwrap();
    let idle = create(&r, &job_id(), None).await.unwrap();
    assert_eq!(worktree_count(&r), 3);
    write(&busy.path.join("a.txt"), b"job edit\n");
    write(&busy.path.join("job.txt"), b"job file\n");

    let out = remove(&r, &busy.path, &busy.branch, Some(&busy.base_oid))
        .await
        .unwrap();
    let mut dirty = out.dirty.clone();
    dirty.sort();
    assert_eq!(dirty, vec!["a.txt", "job.txt"]);
    let saved = out.saved.clone().expect("the work was committed");
    assert_eq!(out.preserved_branch.as_deref(), Some(busy.branch.as_str()));
    assert_eq!(git(&r, &["rev-parse", &busy.branch]), saved);
    assert_eq!(git(&r, &["show", &format!("{saved}:job.txt")]), "job file");
    assert!(!busy.path.exists());

    let out = remove(&r, &idle.path, &idle.branch, Some(&idle.base_oid))
        .await
        .unwrap();
    assert!(out.dirty.is_empty() && out.saved.is_none());
    assert_eq!(out.branch_deleted.as_deref(), Some(idle.branch.as_str()));
    assert_eq!(worktree_count(&r), 1, "git worktree list has no orphan");
    // The user's checkout never moved.
    assert_eq!(std::fs::read(r.join("a.txt")).unwrap(), b"one\n");
    assert_eq!(git(&r, &["status", "--porcelain"]), "");
    // A folder that is not ours is refused.
    assert!(remove(&r, &r, "main", None).await.is_err());
}

#[tokio::test]
async fn a_vanished_worktree_is_recreated_from_its_branch() {
    let _data = data_dir();
    let r = repo("ensure", "");
    let wt = create(&r, &job_id(), None).await.unwrap();
    write(&wt.path.join("work.txt"), b"round one\n");
    save_work(&wt.path, "round one").await.unwrap().unwrap();
    assert_eq!(git(&wt.path, &["status", "--porcelain"]), "");
    std::fs::remove_dir_all(&wt.path).unwrap();
    assert!(ensure(&r, &wt.path, &wt.branch).await.unwrap());
    assert_eq!(
        std::fs::read(wt.path.join("work.txt")).unwrap(),
        b"round one\n"
    );
    assert!(!ensure(&r, &wt.path, &wt.branch).await.unwrap());
    let listed: Vec<String> = list_job_worktrees().into_iter().map(|(id, _)| id).collect();
    assert!(listed.iter().any(|id| wt.branch.ends_with(id.as_str())));
    assert_eq!(
        branch_at(&wt.path).await.as_deref(),
        Some(wt.branch.as_str())
    );
    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
    assert!(ensure(&r, &wt.path, "omniget/job-gone").await.is_err());
}

/// 2.4: the job's diff is `merge-base(base, HEAD)..worktree`: the user's
/// carried work is not in it, uncommitted files are (private index), and
/// once the work is committed on the branch the diff is the same with
/// `head_oid` naming that commit.
#[tokio::test]
async fn the_worktree_diff_is_the_jobs_own_work_committed_or_not() {
    use crate::core::llm::snapshot::worktree_diff;
    let _data = data_dir();
    let r = repo("wtdiff", "");
    write(&r.join("b.txt"), b"user wip\n");
    let wt = create(&r, &job_id(), None).await.unwrap();
    write(&wt.path.join("a.txt"), b"one\njob\n");
    write(&wt.path.join("new.txt"), b"added by the job\n");
    let index = PathBuf::from(git(&wt.path, &["rev-parse", "--absolute-git-dir"])).join("index");
    let index_before = std::fs::read(&index).unwrap();

    let live = worktree_diff(&wt.path, &wt.base_oid).await.unwrap();
    let paths: Vec<(&str, &str)> = live
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status.as_str()))
        .collect();
    assert_eq!(paths, vec![("a.txt", "M"), ("new.txt", "A")]);
    assert_eq!((live.additions, live.deletions), (2, 0));
    assert_eq!(live.base.as_deref(), Some(wt.base_oid.as_str()));
    assert_eq!(live.head_oid.as_deref(), Some(wt.base_oid.as_str()));
    assert!(live.patch.contains("+job"));
    // The worktree's own index was not used (nothing staged there).
    assert_eq!(std::fs::read(&index).unwrap(), index_before);
    assert!(git(&wt.path, &["diff", "--cached", "--name-only"]).is_empty());

    let commit = save_work(&wt.path, "job work").await.unwrap().unwrap();
    let done = worktree_diff(&wt.path, &wt.base_oid).await.unwrap();
    assert_eq!(done.head_oid.as_deref(), Some(commit.as_str()));
    assert_eq!(done.files, live.files);
    assert_eq!(done.patch, live.patch);
    assert_eq!(done.tip, live.tip, "same tree, committed or not");

    // No worktree or no base: unavailable, never an empty diff.
    assert!(worktree_diff(&r.join("nope"), &wt.base_oid)
        .await
        .unwrap_err()
        .starts_with("ERR_DIFF_UNAVAILABLE"));
    assert!(worktree_diff(&wt.path, "")
        .await
        .unwrap_err()
        .starts_with("ERR_DIFF_UNAVAILABLE"));
    remove(&r, &wt.path, &wt.branch, Some(&wt.base_oid))
        .await
        .unwrap();
}
