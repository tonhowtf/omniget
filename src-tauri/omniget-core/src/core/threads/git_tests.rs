//! Acceptance of the T2 glue against a real temporary repository and the
//! system `git`, with a fake driver (this test) emitting runtime events:
//! thread with a worktree, two turns that write files, checkpoints 0/1/2,
//! the turn-2 diff, "edit from here" back to turn 1 (files + conversation),
//! the branch renamed after the first prompt, archive and re-creation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::engine::ThreadsEngine;
use super::git::{GitHooks, ThreadGit};
use super::model::{Command, CommandEnvelope};
use super::store;
use crate::core::llm::drivers::*;
use crate::core::vcs::diff::DiffOptions;

fn data_dir() -> PathBuf {
    // The same data dir as the vcs tests: the env var is process-global and
    // both suites read it (worktrees root, shadow dir).
    let d = std::env::temp_dir().join("omniget-vcs-tests-data");
    std::fs::create_dir_all(&d).unwrap();
    unsafe { std::env::set_var("OMNIGET_DATA_DIR", &d) };
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "omniget-threads-git-{name}-{}",
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

async fn dispatch(e: &ThreadsEngine, command: Command) -> super::DispatchResult {
    e.dispatch(CommandEnvelope {
        command_id: None,
        command,
    })
    .await
    .unwrap()
}

async fn runtime(e: &ThreadsEngine, thread: &str, turn: &str, kind: RuntimeEventKind) {
    let ev = RuntimeEvent::new("fake", "fake", thread, Some(turn), kind);
    e.dispatch(CommandEnvelope {
        command_id: Some(format!("provider:{}", ev.event_id)),
        command: Command::RuntimeAppend { event: ev },
    })
    .await
    .unwrap();
}

/// The fake driver's turn: the host's `before_turn`, the file work, then the
/// runtime events a real driver would send.
async fn fake_turn(
    e: &Arc<ThreadsEngine>,
    g: &ThreadGit,
    thread: &str,
    text: &str,
    work: impl FnOnce(&Path),
) -> (String, PathBuf) {
    let r = dispatch(
        e,
        Command::TurnStart {
            thread_id: thread.into(),
            turn_id: None,
            message_id: None,
            text: text.into(),
            attachments: vec![],
            model: None,
        },
    )
    .await;
    let (turn_id, ordinal) = r
        .events
        .iter()
        .find_map(|ev| match &ev.event {
            super::DomainEvent::TurnStartRequested {
                turn_id, ordinal, ..
            } => Some((turn_id.clone(), *ordinal)),
            _ => None,
        })
        .unwrap();
    let cwd = g.before_turn(thread, ordinal).await.unwrap().unwrap();
    runtime(
        e,
        thread,
        &turn_id,
        RuntimeEventKind::TurnStarted(TurnStartedPayload {
            model: Some("gpt-4o-mini".into()),
            effort: None,
        }),
    )
    .await;
    work(&cwd);
    runtime(
        e,
        thread,
        &turn_id,
        RuntimeEventKind::ContentDelta(ContentDeltaPayload {
            stream_kind: StreamKind::AssistantText,
            delta: format!("done: {text}"),
            content_index: None,
            summary_index: None,
        }),
    )
    .await;
    runtime(
        e,
        thread,
        &turn_id,
        RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
            state: TurnEndState::Completed,
            stop_reason: None,
            usage: Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 20,
                ..Default::default()
            }),
            total_cost_usd: None,
            error_message: None,
        }),
    )
    .await;
    // The reactor takes the same checkpoint; whoever comes second skips.
    g.after_turn(thread, &turn_id).await.unwrap();
    (turn_id, cwd)
}

async fn eventually<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    for _ in 0..300 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worktree_turn_checkpoints_diff_and_edit_from_here() {
    data_dir();
    let repo_dir = scratch("repo");
    git(&repo_dir, &["init", "-q", "-b", "main"]);
    git(&repo_dir, &["config", "user.name", "Test"]);
    git(&repo_dir, &["config", "user.email", "test@example.com"]);
    git(&repo_dir, &["config", "commit.gpgsign", "false"]);
    write(&repo_dir.join("a.txt"), b"one\n");
    git(&repo_dir, &["add", "-A"]);
    git(&repo_dir, &["commit", "-q", "-m", "init"]);

    let db = scratch("db").join("threads.db");
    let engine = ThreadsEngine::open(&db).unwrap();
    let g = ThreadGit::new(engine.clone(), GitHooks::default());
    tokio::spawn(g.clone().run(engine.subscribe()));

    // thread.create by folder, with a worktree.
    let created = dispatch(
        &engine,
        serde_json::from_value(serde_json::json!({
            "type": "thread.create",
            "threadId": "t1",
            "projectPath": repo_dir.to_string_lossy(),
            "worktree": true,
            "instanceId": "fake",
            "driver": "fake",
        }))
        .unwrap(),
    )
    .await;
    assert!(created
        .events
        .iter()
        .any(|e| matches!(e.event, super::DomainEvent::ProjectCreated { .. })));

    // Turn 1: a new file and an edit.
    let (_t1, wt) = fake_turn(&engine, &g, "t1", "Add a hello file", |cwd| {
        write(&cwd.join("hello.txt"), b"hello\n");
        write(&cwd.join("a.txt"), b"one\ntwo\n");
    })
    .await;
    assert_ne!(wt, repo_dir, "the turn ran in the worktree");
    assert!(crate::core::vcs::worktree::is_omniget_worktree(&wt));
    let row = engine
        .read(|c| store::thread_row(c, "t1"))
        .unwrap()
        .unwrap();
    assert_eq!(row.worktree_state.as_deref(), Some("ready"));
    assert_eq!(row.worktree_path.as_deref(), Some(&*wt.to_string_lossy()));

    // Turn 2: edit the new file, add another.
    let (t2, _) = fake_turn(&engine, &g, "t1", "Say world too", |cwd| {
        write(&cwd.join("hello.txt"), b"hello\nworld\n");
        write(&cwd.join("b.txt"), b"bee\n");
    })
    .await;

    // Checkpoints 0 (baseline), 1 and 2, with the numstat of each turn.
    let cps = engine.read(|c| store::checkpoints(c, "t1")).unwrap();
    assert_eq!(
        cps.iter().map(|c| c.turn_count).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let mut t2_files: Vec<(String, String, u32, u32)> = cps[2]
        .files
        .iter()
        .map(|f| (f.path.clone(), f.status.clone(), f.additions, f.deletions))
        .collect();
    t2_files.sort();
    assert_eq!(
        t2_files,
        [
            ("b.txt".to_string(), "added".to_string(), 1, 0),
            ("hello.txt".to_string(), "modified".to_string(), 1, 0),
        ]
    );
    assert_eq!(cps[2].turn_id.as_deref(), Some(t2.as_str()));
    // The page of turns carries it too.
    let page = engine.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(page.turns[1].checkpoint.as_ref().unwrap().additions, 2);
    // Every finished turn has a usage row.
    assert!(page.turns.iter().all(|t| t.usage.is_some()));

    // The diff of turn 2, with patches.
    let d = g
        .diff("t1", Some(2), &DiffOptions::default())
        .await
        .unwrap();
    let hello = d.files.iter().find(|f| f.path == "hello.txt").unwrap();
    assert!(hello.patch.contains("+world"), "{}", hello.patch);
    assert_eq!((d.additions, d.deletions), (2, 0));
    // Whole thread: baseline → live tree.
    let total = g.diff("t1", None, &DiffOptions::default()).await.unwrap();
    assert_eq!(total.files.len(), 3);

    // The temp branch got a name from the first prompt (heuristic: no model).
    let branch = eventually("branch rename", || {
        engine
            .read(|c| store::thread_row(c, "t1"))
            .unwrap()
            .and_then(|r| r.branch)
            .filter(|b| !crate::core::vcs::worktree::is_temp_branch(b))
    })
    .await;
    assert_eq!(branch, "omniget/add-hello-file");
    assert_eq!(git(&wt, &["branch", "--show-current"]), branch);

    // "Edit from here" at turn 1, files included.
    let out = g.revert_to_turn("t1", 1, true).await.unwrap();
    assert_eq!(out.prompt.as_deref(), Some("Say world too"));
    let mut restored = out.restored_files.clone();
    restored.sort();
    assert_eq!(restored, ["b.txt", "hello.txt"]);
    assert_eq!(std::fs::read(wt.join("hello.txt")).unwrap(), b"hello\n");
    assert_eq!(std::fs::read(wt.join("a.txt")).unwrap(), b"one\ntwo\n");
    assert!(!wt.join("b.txt").exists());
    // The conversation is cut after turn 1.
    let page = engine.turns_page_blocking("t1", None, 10).unwrap();
    assert_eq!(page.turns.len(), 1);
    assert!(page.turns[0]
        .messages
        .iter()
        .all(|m| !m.text.contains("world")));
    let cps = engine.read(|c| store::checkpoints(c, "t1")).unwrap();
    assert_eq!(cps.iter().map(|c| c.turn_count).collect::<Vec<_>>(), [0, 1]);
    // The user's checkout never moved.
    assert!(!repo_dir.join("hello.txt").exists());
    assert_eq!(git(&repo_dir, &["branch", "--show-current"]), "main");

    // Only the conversation this time: files stay as they are.
    write(&wt.join("c.txt"), b"sea\n");
    let (_t2b, _) = fake_turn(&engine, &g, "t1", "Another try", |_| {}).await;
    let out = g.revert_to_turn("t1", 1, false).await.unwrap();
    assert!(out.restored_files.is_empty());
    assert!(wt.join("c.txt").exists());

    // Archive: the folder goes, unsaved work lives in a checkpoint; the next
    // turn re-creates the worktree and puts it back.
    dispatch(
        &engine,
        Command::Archive {
            thread_id: "t1".into(),
        },
    )
    .await;
    eventually("worktree removal", || {
        let r = engine
            .read(|c| store::thread_row(c, "t1"))
            .unwrap()
            .unwrap();
        (r.worktree_state.as_deref() == Some("removed")).then_some(())
    })
    .await;
    assert!(!wt.exists());
    let (_t3, wt2) = fake_turn(&engine, &g, "t1", "Back again", |_| {}).await;
    assert!(wt2.is_dir());
    assert_eq!(std::fs::read(wt2.join("c.txt")).unwrap(), b"sea\n");
    assert_eq!(std::fs::read(wt2.join("hello.txt")).unwrap(), b"hello\n");

    // Delete: worktree and refs go.
    dispatch(
        &engine,
        Command::ThreadDelete {
            thread_id: "t1".into(),
        },
    )
    .await;
    eventually("worktree deletion", || (!wt2.exists()).then_some(())).await;
    let refs = git(&repo_dir, &["for-each-ref", "refs/omniget/checkpoints"]);
    assert!(refs.is_empty(), "{refs}");
}
