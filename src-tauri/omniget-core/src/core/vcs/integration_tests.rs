//! End-to-end run against a real temporary repository with the system `git`:
//! worktree, three checkpoints, the turn-2 diff, restore of turn 1 byte for
//! byte, and the user's branch/index/HEAD untouched. Plus the shadow path for
//! a folder without git.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::diff::DiffOptions;
use super::*;

fn data_dir() -> PathBuf {
    // One data dir for every test of this module (the env var is global).
    let d = std::env::temp_dir().join("omniget-vcs-tests-data");
    std::fs::create_dir_all(&d).unwrap();
    unsafe { std::env::set_var("OMNIGET_DATA_DIR", &d) };
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "omniget-vcs-{name}-{}",
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

#[tokio::test]
async fn worktree_checkpoints_diff_restore() {
    data_dir();
    let repo_dir = scratch("repo");
    git(&repo_dir, &["init", "-q", "-b", "main"]);
    git(&repo_dir, &["config", "user.name", "Test"]);
    git(&repo_dir, &["config", "user.email", "test@example.com"]);
    git(&repo_dir, &["config", "commit.gpgsign", "false"]);
    write(&repo_dir.join("a.txt"), b"one\n");
    write(&repo_dir.join("b.txt"), b"bee\r\nwith crlf\r\n");
    write(&repo_dir.join(".gitignore"), b"ignored.log\n");
    git(&repo_dir, &["add", "-A"]);
    git(&repo_dir, &["commit", "-q", "-m", "init"]);
    // The user has work of their own: one staged edit and one untracked file.
    write(&repo_dir.join("a.txt"), b"user staged\n");
    git(&repo_dir, &["add", "a.txt"]);
    write(&repo_dir.join("scratch.txt"), b"user untracked\n");
    let index_before = std::fs::read(repo_dir.join(".git").join("index")).unwrap();
    let head_before = git(&repo_dir, &["rev-parse", "HEAD"]);

    let st = repo::status(&repo_dir).await.unwrap();
    assert!(st.is_repo && st.dirty);
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!((st.staged, st.untracked), (1, 1));

    let thread = "thread/ABC 1";
    let req = worktree::WorktreeRequest {
        repo: repo_dir.clone(),
        thread: thread.into(),
        ..Default::default()
    };
    let snaps = std::sync::Mutex::new(Vec::new());
    let info = worktree::create(&req, Arc::new(AtomicBool::new(false)), |s| {
        snaps.lock().unwrap().push(s.clone())
    })
    .await
    .unwrap();
    let wt = info.path.clone();
    assert!(worktree::is_omniget_worktree(&wt));
    assert!(worktree::is_temp_branch(&info.branch));
    assert_eq!(snaps.lock().unwrap().last().unwrap().phase, "done");
    assert_eq!(std::fs::read(wt.join("a.txt")).unwrap(), b"one\n");

    // turn 0 baseline, then three turns of changes.
    checkpoint::capture(&wt, thread, 0).await.unwrap();
    write(&wt.join("a.txt"), b"one\ntwo\n");
    write(&wt.join("dir/c.txt"), b"see\n");
    write(&wt.join("ignored.log"), b"log 1\n");
    checkpoint::capture(&wt, thread, 1).await.unwrap();

    write(&wt.join("a.txt"), b"one\ntwo\nthree\n");
    std::fs::remove_file(wt.join("b.txt")).unwrap();
    write(&wt.join("img.bin"), &[0u8, 159, 146, 150, 0, 1, 2, 255]);
    checkpoint::capture(&wt, thread, 2).await.unwrap();

    write(&wt.join("dir/c.txt"), b"see\nsaw\n");
    write(&wt.join("dir/new/d.txt"), b"dee\n");
    write(&wt.join("ignored.log"), b"log 3\n");
    checkpoint::capture(&wt, thread, 3).await.unwrap();

    let list = checkpoint::list(&wt, thread).await.unwrap();
    assert_eq!(
        list.iter().map(|c| c.turn).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );

    let d2 = checkpoint::diff_turn(&wt, thread, 2, &DiffOptions::default())
        .await
        .unwrap();
    let find = |p: &str| {
        d2.files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("{p} missing in {:?}", d2.files))
    };
    assert_eq!(d2.files.len(), 3, "{:?}", d2.files);
    let a = find("a.txt");
    assert_eq!(
        (a.status.as_str(), a.additions, a.deletions),
        ("modified", 1, 0)
    );
    assert!(a.patch.contains("+three"));
    let b = find("b.txt");
    assert_eq!((b.status.as_str(), b.deletions), ("deleted", 2));
    let img = find("img.bin");
    assert!(img.binary);
    assert_eq!(img.status, "added");
    assert!(!d2.truncated);

    let full = checkpoint::diff_full(&wt, thread, None, &DiffOptions::default())
        .await
        .unwrap();
    assert!(full.files.iter().any(|f| f.path == "dir/new/d.txt"));
    assert!(!full.files.iter().any(|f| f.path == "ignored.log"));

    // Restore turn 1: exact bytes back, later files gone, ignored untouched.
    let r = checkpoint::restore(&wt, thread, 1, false, true)
        .await
        .unwrap();
    assert!(!r.files.is_empty());
    assert_eq!(r.dropped_later, 2);
    assert_eq!(std::fs::read(wt.join("a.txt")).unwrap(), b"one\ntwo\n");
    assert_eq!(
        std::fs::read(wt.join("b.txt")).unwrap(),
        b"bee\r\nwith crlf\r\n"
    );
    assert_eq!(std::fs::read(wt.join("dir/c.txt")).unwrap(), b"see\n");
    assert!(!wt.join("img.bin").exists());
    assert!(!wt.join("dir/new").exists());
    assert_eq!(std::fs::read(wt.join("ignored.log")).unwrap(), b"log 3\n");
    assert_eq!(checkpoint::list(&wt, thread).await.unwrap().len(), 2);

    // Outside a worktree, restore needs confirmation.
    let refused = checkpoint::restore(&repo_dir, thread, 0, false, false)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), runner::ERR_UNSAFE);

    // The user's checkout did not move.
    assert_eq!(
        std::fs::read(repo_dir.join(".git").join("index")).unwrap(),
        index_before
    );
    assert_eq!(git(&repo_dir, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(&repo_dir, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(
        std::fs::read(repo_dir.join("a.txt")).unwrap(),
        b"user staged\n"
    );
    assert_eq!(
        git(&repo_dir, &["diff", "--cached", "--name-only"]),
        "a.txt"
    );
    assert!(repo_dir.join("scratch.txt").exists());
    assert_eq!(git(&repo_dir, &["stash", "list"]), "");

    // Commit suggestion, rename, commit inside the worktree, then remove it.
    let s = actions::suggest(&wt).await.unwrap();
    assert!(s.files.iter().any(|f| f.path == "a.txt"));
    let renamed = worktree::rename_branch(&wt, "Fix the Thing").await.unwrap();
    assert_eq!(renamed, "fix-the-thing");
    let c = actions::commit(&wt, Some("Turn one\n\nbody"), None)
        .await
        .unwrap();
    assert_eq!(c.status, "created");
    assert_eq!(c.branch.as_deref(), Some("fix-the-thing"));
    assert_eq!(
        actions::commit(&wt, Some("again"), None)
            .await
            .unwrap()
            .status,
        "skipped_no_changes"
    );

    // Push to a local bare remote: published under its own name with -u.
    let remote = scratch("remote");
    git(&remote, &["init", "-q", "--bare"]);
    git(
        &repo_dir,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let pushed = actions::push(&wt).await.unwrap();
    assert_eq!(
        (pushed.status.as_str(), pushed.set_upstream),
        ("pushed", true)
    );
    assert_eq!(
        git(&remote, &["rev-parse", "refs/heads/fix-the-thing"]),
        c.sha.clone().unwrap()
    );
    assert_eq!(
        actions::push(&wt).await.unwrap().status,
        "skipped_up_to_date"
    );
    let h = actions::host(&wt).await.unwrap();
    assert_eq!(
        (h.kind.as_str(), h.reason.as_deref()),
        ("unknown", Some("provider-unsupported"))
    );
    assert!(!git(&remote, &["for-each-ref", "refs/omniget"]).contains("omniget"));
    let wts = worktree::list(&repo_dir).await.unwrap();
    assert!(wts
        .iter()
        .any(|w| w.omniget && w.thread.as_deref() == Some(thread)));
    let rm = worktree::remove(&wt, true, true, Some(thread))
        .await
        .unwrap();
    assert!(rm.removed);
    assert_eq!(rm.branch_deleted.as_deref(), Some("fix-the-thing"));
    assert!(!wt.exists());
    assert!(git(&repo_dir, &["for-each-ref", "refs/omniget"]).is_empty());
    assert_eq!(
        std::fs::read(repo_dir.join(".git").join("index")).unwrap(),
        index_before
    );

    let _ = std::fs::remove_dir_all(&repo_dir);
    let _ = std::fs::remove_dir_all(&remote);
}

#[tokio::test]
async fn shadow_checkpoints_for_plain_folder() {
    data_dir();
    let dir = scratch("plain");
    write(&dir.join("notes.txt"), b"v0\n");
    checkpoint::capture(&dir, "t", 0).await.unwrap();
    write(&dir.join("notes.txt"), b"v1\n");
    write(&dir.join("extra.txt"), b"x\n");
    let cp = checkpoint::capture(&dir, "t", 1).await.unwrap();
    assert!(cp.shadow);
    assert!(!dir.join(".git").exists());
    let d = checkpoint::diff_turn(&dir, "t", 1, &DiffOptions::default())
        .await
        .unwrap();
    assert_eq!(d.files.len(), 2);
    assert_eq!(
        checkpoint::restore(&dir, "t", 0, false, false)
            .await
            .unwrap_err()
            .code(),
        runner::ERR_UNSAFE
    );
    checkpoint::restore(&dir, "t", 0, true, false)
        .await
        .unwrap();
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"v0\n");
    assert!(!dir.join("extra.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn worktree_setup_script_and_cancel() {
    data_dir();
    let repo_dir = scratch("setup");
    git(&repo_dir, &["init", "-q", "-b", "main"]);
    write(
        &repo_dir.join(".omniget/project.json"),
        br#"{"scripts":[{"name":"deps","command":"echo hello-setup; touch setup.out","runOnWorktreeCreate":true}]}"#,
    );
    write(&repo_dir.join("x.txt"), b"x\n");
    git(&repo_dir, &["add", "-A"]);
    git(&repo_dir, &["commit", "-q", "-m", "init"]);

    let req = worktree::WorktreeRequest {
        repo: repo_dir.clone(),
        thread: "setup-thread".into(),
        run_setup: true,
        ..Default::default()
    };
    let last = std::sync::Mutex::new(None);
    let info = worktree::create(&req, Arc::new(AtomicBool::new(false)), |s| {
        *last.lock().unwrap() = Some(s.clone())
    })
    .await
    .unwrap();
    let snap = last.lock().unwrap().clone().unwrap();
    let stage = snap.stages.iter().find(|s| s.id == "setup-script").unwrap();
    assert_eq!(stage.state, "done", "{stage:?}");
    assert!(
        stage.tail.iter().any(|l| l.contains("hello-setup")),
        "{stage:?}"
    );
    assert!(info.path.join("setup.out").exists());
    // A second create for the same thread reuses it.
    assert!(
        worktree::create(&req, Arc::new(AtomicBool::new(false)), |_| {})
            .await
            .unwrap()
            .reused
    );
    worktree::remove(&info.path, true, true, None)
        .await
        .unwrap();

    // Cancelled before checkout: nothing is left behind.
    let req2 = worktree::WorktreeRequest {
        repo: repo_dir.clone(),
        thread: "cancel-thread".into(),
        ..Default::default()
    };
    let err = worktree::create(&req2, Arc::new(AtomicBool::new(true)), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), runner::ERR_CANCELLED);
    assert!(!worktree::worktree_path_for(&repo_dir, "cancel-thread")
        .unwrap()
        .exists());
    assert_eq!(git(&repo_dir, &["branch", "--list", "omniget/*"]), "");
    let _ = std::fs::remove_dir_all(&repo_dir);
}
