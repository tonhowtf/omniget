use super::*;

const GOOD: &str = "---
agent: claude-code
run_mode: auto_edits
isolation: worktree
deadline_min: 30
budget: 0.5
checks: [\"true\"]
hooks:
  after_create: echo made
  before_remove: echo bye
include: .env.local
---
Repo {{repo}} on {{branch}}.

{{ task }}
";

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".git")).unwrap();
    std::fs::write(d.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(d.path().join(".omniget")).unwrap();
    d
}

fn write(d: &Path, text: &str) {
    std::fs::write(d.join(FILE), text).unwrap();
}

#[test]
fn a_file_parses_into_defaults_and_a_template() {
    let w = parse(GOOD).unwrap();
    assert_eq!(w.config.agent.as_deref(), Some("claude-code"));
    assert_eq!(w.config.isolation.as_deref(), Some("worktree"));
    assert_eq!(w.config.deadline_min, Some(30));
    assert_eq!(w.config.budget_usd, Some(0.5));
    assert_eq!(w.config.include, vec![".env.local".to_string()]);
    assert_eq!(w.config.hooks.after_create.as_deref(), Some("echo made"));
    assert_eq!(w.sha256, sha256_hex(GOOD.as_bytes()));
    let vars: BTreeMap<&str, String> = [
        ("repo", "/r".to_string()),
        ("branch", "main".to_string()),
        ("task", "fix X".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(w.render(&vars).unwrap(), "Repo /r on main.\n\nfix X");
    // Strict at render time too: a variable without a value is an error.
    let mut partial = vars.clone();
    partial.remove("branch");
    assert!(w.render(&partial).unwrap_err().starts_with(ERR_TEMPLATE));
    assert_eq!(
        w.gated(),
        ["after_create", "before_remove", "check", "run_mode"]
    );
}

#[test]
fn no_front_matter_is_a_template_and_an_empty_body_is_the_task() {
    let w = parse("Please: {{task}}").unwrap();
    assert_eq!(w.config, Config::default());
    let w = parse("---\nagent: omni\n---\n").unwrap();
    assert_eq!(w.template, "{{task}}");
    assert!(w.gated().is_empty());
    // approval_required is no extra power: nothing to approve.
    assert!(parse("---\nrun_mode: approval_required\n---\n")
        .unwrap()
        .gated()
        .is_empty());
}

#[test]
fn every_mistake_is_an_error_that_says_where() {
    let cases = [
        ("---\nagent: a\n---\n{{ tsak }}", "unknown variable `tsak`"),
        (
            "---\nagent: a\n---\nline\n{{task}} {{task.title}}",
            "line 2",
        ),
        ("---\nagent: a\n---\n{{task", "never closed"),
        (
            "---\nagent: a\n---\nno variable at all",
            "must use {{task}}",
        ),
        ("---\nagnet: a\n---\n{{task}}", "unknown field `agnet`"),
        ("---\nagent: a\n{{task}}", "never closed by another"),
        ("---\nrun_mode: yolo\n---\n", "run_mode"),
        ("---\nisolation: sandbox\n---\n", "isolation"),
        ("---\ndeadline_min: 0\n---\n", "deadline_min"),
        ("---\nisolation: shared\nbase: main\n---\n", "base"),
        ("---\ninclude: ../secrets\n---\n", "include"),
        (
            "---\nhooks:\n  after_create: ''\n---\n",
            "hooks.after_create",
        ),
        (
            "---\nhooks:\n  on_start: x\n---\n",
            "unknown field `on_start`",
        ),
    ];
    for (text, needle) in cases {
        let e = parse(text).unwrap_err();
        assert!(e.contains(needle), "{text:?} → {e}");
    }
}

#[test]
fn an_invalid_reload_keeps_the_last_good_version_even_after_a_restart() {
    let d = repo();
    let dbf = d.path().join("wf.db");
    let store = Store::open(&dbf).unwrap();
    assert_eq!(store.status(d.path()).state, "none");
    write(d.path(), GOOD);
    let v1 = store.status(d.path());
    assert_eq!(v1.state, "active");
    let v1_sha = v1.active.as_ref().unwrap().sha256.clone();
    // A broken edit: the error is shown, v1 stays in effect.
    write(
        d.path(),
        &GOOD.replace("{{ task }}", "{{ task }} {{ nope }}"),
    );
    let broken = store.status(d.path());
    assert_eq!(broken.state, "invalid");
    assert!(broken.error.as_deref().unwrap().contains("nope"));
    assert!(broken.from_last_good);
    assert_eq!(broken.active.as_ref().unwrap().sha256, v1_sha);
    assert_ne!(broken.file_sha256.as_deref(), Some(v1_sha.as_str()));
    // After a restart (a new store on the same file) too.
    drop(store);
    let store = Store::open(&dbf).unwrap();
    let again = store.status(d.path());
    assert_eq!(again.active.as_ref().unwrap().sha256, v1_sha);
    // A valid edit replaces it.
    write(
        d.path(),
        &GOOD.replace("deadline_min: 30", "deadline_min: 45"),
    );
    let v2 = store.status(d.path());
    assert_eq!(v2.state, "active");
    assert_eq!(v2.active.unwrap().config.deadline_min, Some(45));
    // Deleting the file ends the contract: no old version comes back.
    std::fs::remove_file(d.path().join(FILE)).unwrap();
    assert_eq!(store.status(d.path()).state, "none");
    write(d.path(), "{{nope}}");
    let orphan = store.status(d.path());
    assert_eq!(orphan.state, "invalid");
    assert!(orphan.active.is_none());
}

#[test]
fn trust_is_per_sha256_and_lost_on_any_change() {
    let d = repo();
    let store = Store::memory();
    write(d.path(), GOOD);
    let st = store.status(d.path());
    assert!(!st.trusted && !st.may_run_commands());
    assert!(st.skipped_reason().contains("not trusted"));
    let sha = st.active.as_ref().unwrap().sha256.clone();
    assert!(store.trust(d.path(), "deadbeef", "t").is_err());
    assert!(store.trust(d.path(), &sha[..4], "t").is_err(), "too short");
    let st = store.trust(d.path(), &sha[..12], "t").unwrap();
    assert!(st.trusted && st.may_run_commands());
    assert_eq!(st.trusted_sha256.as_deref(), Some(sha.as_str()));
    // Opened from a subfolder: same repository, same trust.
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    assert!(store.status(&d.path().join("src")).trusted);
    // One byte changed: not trusted any more, and the log says why.
    write(d.path(), &GOOD.replace("echo made", "echo made!"));
    let st = store.status(d.path());
    assert!(!st.trusted);
    assert!(st.skipped_reason().contains("changed since it was trusted"));
    // An invalid edit keeps the trusted last good version runnable.
    write(d.path(), GOOD);
    assert!(store.status(d.path()).trusted);
    write(d.path(), "{{nope}}");
    let st = store.status(d.path());
    assert!(st.from_last_good && st.trusted);
    let st = store.untrust(d.path()).unwrap();
    assert!(!st.trusted && st.trusted_sha256.is_none());
}

#[test]
fn the_branch_is_read_from_head() {
    let d = repo();
    assert_eq!(checked_out_branch(d.path()).as_deref(), Some("main"));
    std::fs::write(d.path().join(".git/HEAD"), "0123456789abcdef0123\n").unwrap();
    assert_eq!(
        checked_out_branch(d.path()).as_deref(),
        Some("0123456789ab")
    );
    assert_eq!(root_of(&d.path().join(".omniget")), root_of(d.path()));
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_reports_its_exit_and_a_slow_one_is_killed() {
    let d = tempfile::tempdir().unwrap();
    let env = vec![("OMNIGET_HOOK".to_string(), "after_create".to_string())];
    let ok = run_command(
        "after_create",
        "echo $OMNIGET_HOOK > out.txt; echo hi",
        d.path(),
        &env,
        Duration::from_secs(10),
    )
    .await;
    assert!(ok.ok, "{ok:?}");
    assert_eq!(ok.exit_code, Some(0));
    assert_eq!(ok.output.trim(), "hi");
    assert_eq!(
        std::fs::read_to_string(d.path().join("out.txt"))
            .unwrap()
            .trim(),
        "after_create"
    );
    let bad = run_command("check", "exit 3", d.path(), &[], Duration::from_secs(10)).await;
    assert!(!bad.ok);
    assert_eq!(bad.exit_code, Some(3));
    let t0 = Instant::now();
    let slow = run_command(
        "before_remove",
        "sleep 30",
        d.path(),
        &[],
        Duration::from_millis(300),
    )
    .await;
    assert!(slow.timed_out && !slow.ok);
    assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    assert!(slow.note().contains("timed out"));
}
