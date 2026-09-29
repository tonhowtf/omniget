//! The repository's hooks against real repositories, real worktrees and
//! the system `git` (BN5 `workflow`: untrusted hooks never run, trusted ones
//! run once, a failing `before_remove` keeps the worktree).

use std::path::{Path, PathBuf};

use super::*;
use omniget_core::core::vcs::job_worktree;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "omniget-workflow-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(d).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
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
}

/// A repository with one commit, `.env.local` ignored, and a workflow whose
/// hooks append to files outside it (so a run is counted, never guessed).
fn repo(marks: &Path) -> PathBuf {
    let r = scratch("repo");
    git(&r, &["init", "-q", "-b", "main"]);
    git(&r, &["config", "commit.gpgsign", "false"]);
    std::fs::write(r.join("a.txt"), "one\n").unwrap();
    std::fs::write(r.join(".gitignore"), ".env.local\n").unwrap();
    std::fs::write(r.join(".env.local"), "KEY=1\n").unwrap();
    git(&r, &["add", "a.txt", ".gitignore"]);
    git(&r, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(r.join(".omniget")).unwrap();
    let after = marks.join("after.log");
    let before = marks.join("before.log");
    std::fs::write(
        r.join(workflow::FILE),
        format!(
            "---\nagent: claude-code\nisolation: worktree\ninclude: .env.local\nhooks:\n  after_create: echo \"$OMNIGET_JOB_ID\" >> '{}'\n  before_remove: echo \"$OMNIGET_JOB_ID\" >> '{}' && test -f \"$OMNIGET_WORKTREE/allow-remove\"\n---\n{{{{task}}}}\n",
            after.display(),
            before.display()
        ),
    )
    .unwrap();
    r
}

fn lines(p: &Path) -> usize {
    std::fs::read_to_string(p)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// A job started in `repo` with its workflow, in a real worktree.
async fn job_in(store: &Store, repo: &Path) -> Job {
    let mut spec = StartSpec {
        prompt: "fix X".into(),
        workspace: Some(repo.to_string_lossy().into_owned()),
        ..Default::default()
    };
    apply(Some(store), &mut spec).unwrap();
    let id = format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let wt = job_worktree::create(repo, &id, None).await.unwrap();
    Job {
        id,
        kind: "run".into(),
        agent_id: spec.agent_id,
        prompt: spec.prompt,
        workspace: spec.workspace,
        state: "running".into(),
        isolation: jobs::Isolation::Worktree,
        worktree_path: Some(wt.path.to_string_lossy().into_owned()),
        branch: Some(wt.branch),
        base_oid: Some(wt.base_oid),
        origin: Some(json!({ "workflow": spec.applied_workflow })),
        ..Default::default()
    }
}

#[tokio::test]
async fn untrusted_hooks_never_run_and_trusted_ones_run_once() {
    let env = crate::secrets::test_env::EnvGuard::acquire();
    let data = scratch("data");
    env.set("OMNIGET_DATA_DIR", Some(data.to_str().unwrap()));
    let marks = scratch("marks");
    let (after, before) = (marks.join("after.log"), marks.join("before.log"));
    let repo = repo(&marks);
    let store = Store::memory();

    // ── untrusted: nothing of the repository runs, and the log says why ──
    let mut untrusted = job_in(&store, &repo).await;
    let wt = PathBuf::from(untrusted.worktree_path.clone().unwrap());
    for _ in 0..3 {
        after_create(Some(&store), &mut untrusted, &repo, &wt)
            .await
            .unwrap();
    }
    assert_eq!(lines(&after), 0, "an untrusted after_create ran");
    assert!(
        untrusted.log.contains("after_create hook skipped")
            && untrusted.log.contains("not trusted"),
        "{}",
        untrusted.log
    );
    // The include files come along without trust (the user's own files).
    assert_eq!(
        std::fs::read_to_string(wt.join(".env.local")).unwrap(),
        "KEY=1\n"
    );
    let out = crate::jobs::remove_job_worktree_with(Some(&store), &untrusted)
        .await
        .unwrap();
    assert_eq!(lines(&before), 0, "an untrusted before_remove ran");
    assert!(
        out.hook_note
            .as_deref()
            .is_some_and(|n| n.contains("before_remove hook skipped")),
        "{out:?}"
    );
    assert!(!wt.exists(), "the untrusted removal went ahead");
    let untrusted_runs = lines(&after) + lines(&before);

    // ── trusted: after_create once per worktree ──
    let sha = store.status(&repo).active.unwrap().sha256;
    store.trust(&repo, &sha, "test").unwrap();
    let mut trusted = job_in(&store, &repo).await;
    let wt = PathBuf::from(trusted.worktree_path.clone().unwrap());
    after_create(Some(&store), &mut trusted, &repo, &wt)
        .await
        .unwrap();
    assert_eq!(lines(&after), 1);
    assert_eq!(
        std::fs::read_to_string(&after).unwrap().trim(),
        trusted.id,
        "the hook sees its job"
    );
    let runs = trusted.origin.as_ref().unwrap()["workflow"]["runs"].clone();
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!(runs[0]["kind"], "after_create");

    // ── before_remove fails: the worktree stays ──
    let blocked = crate::jobs::remove_job_worktree_with(Some(&store), &trusted).await;
    let blocked_err = blocked.unwrap_err();
    assert!(blocked_err.contains(workflow::ERR_HOOK), "{blocked_err}");
    assert!(wt.is_dir(), "a failed before_remove must keep the worktree");
    assert_eq!(lines(&before), 1);
    // ── it passes: removed, the hook ran once more ──
    std::fs::write(wt.join("allow-remove"), "").unwrap();
    let out = crate::jobs::remove_job_worktree_with(Some(&store), &trusted)
        .await
        .unwrap();
    assert!(!wt.exists());
    assert_eq!(lines(&before), 2);
    assert!(
        crate::jobs::removal_note(&out).contains("✓ before_remove"),
        "{out:?}"
    );

    // ── the file changes: trust is gone, hooks stop again ──
    let text = std::fs::read_to_string(repo.join(workflow::FILE)).unwrap();
    std::fs::write(
        repo.join(workflow::FILE),
        text.replace("isolation: worktree", "isolation: auto"),
    )
    .unwrap();
    let mut changed = job_in(&store, &repo).await;
    let wt = PathBuf::from(changed.worktree_path.clone().unwrap());
    after_create(Some(&store), &mut changed, &repo, &wt)
        .await
        .unwrap();
    assert_eq!(
        lines(&after),
        1,
        "a changed file ran without a new approval"
    );
    assert!(changed.log.contains("changed since it was trusted"));
    let _ = crate::jobs::remove_job_worktree_with(Some(&store), &changed).await;

    for (metric, value) in [
        ("untrusted_hook_runs", json!(untrusted_runs)),
        ("trusted_after_create_runs", json!(1)),
        ("before_remove_failure_blocks_removal", json!(true)),
        ("changed_file_hook_runs", json!(lines(&after) - 1)),
    ] {
        crate::bridge_tokens::bn5::emit(
            json!({"scenario": "workflow", "metric": metric, "value": value}),
        );
    }
    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&marks);
    let _ = std::fs::remove_dir_all(&data);
}

/// A failed `after_create` fails the start: the error names the hook.
#[tokio::test]
async fn a_failing_after_create_is_an_error() {
    let env = crate::secrets::test_env::EnvGuard::acquire();
    let data = scratch("data");
    env.set("OMNIGET_DATA_DIR", Some(data.to_str().unwrap()));
    let marks = scratch("marks");
    let repo = repo(&marks);
    std::fs::write(
        repo.join(workflow::FILE),
        "---\nhooks:\n  after_create: exit 9\n---\n{{task}}\n",
    )
    .unwrap();
    let store = Store::memory();
    let sha = store.status(&repo).active.unwrap().sha256;
    store.trust(&repo, &sha, "test").unwrap();
    let mut job = job_in(&store, &repo).await;
    let wt = PathBuf::from(job.worktree_path.clone().unwrap());
    let e = after_create(Some(&store), &mut job, &repo, &wt)
        .await
        .unwrap_err();
    assert!(
        e.contains(workflow::ERR_HOOK) && e.contains("exit 9"),
        "{e}"
    );
    let _ = job_worktree::remove(&repo, &wt, job.branch.as_deref().unwrap(), None).await;
    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&marks);
    let _ = std::fs::remove_dir_all(&data);
}

/// The checks run only trusted, and each lands in the proof.
#[tokio::test]
async fn checks_run_only_when_trusted_and_land_in_the_proof() {
    let d = scratch("checks");
    std::fs::create_dir_all(d.join(".git")).unwrap();
    std::fs::create_dir_all(d.join(".omniget")).unwrap();
    std::fs::write(
        d.join(workflow::FILE),
        "---\nchecks: [\"true\", \"exit 4\"]\n---\n{{task}}\n",
    )
    .unwrap();
    let store = Store::memory();
    let mut spec = StartSpec {
        prompt: "fix X".into(),
        workspace: Some(d.to_string_lossy().into_owned()),
        ..Default::default()
    };
    apply(Some(&store), &mut spec).unwrap();
    let base = Job {
        id: "j1".into(),
        state: "review".into(),
        workspace: spec.workspace.clone(),
        origin: Some(json!({ "workflow": spec.applied_workflow })),
        proof: Some(json!({"checks": []})),
        ..Default::default()
    };
    let mut job = base.clone();
    run_checks(Some(&store), &mut job).await;
    assert!(job.proof.as_ref().unwrap()["checks"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(job.proof.as_ref().unwrap()["checks_skipped"]
        .as_str()
        .unwrap()
        .contains("not trusted"));
    let sha = store.status(&d).active.unwrap().sha256;
    store.trust(&d, &sha, "t").unwrap();
    let mut job = base.clone();
    run_checks(Some(&store), &mut job).await;
    let checks = job.proof.as_ref().unwrap()["checks"].clone();
    assert_eq!(checks[0]["ok"], true);
    assert_eq!(
        (checks[1]["ok"].clone(), checks[1]["exit_code"].clone()),
        (json!(false), json!(4))
    );
    assert_eq!(verdict(&job), "checks_failed");
    let _ = std::fs::remove_dir_all(&d);
}
