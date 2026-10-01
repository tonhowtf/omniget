use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use omniget_core::core::llm::workflow::{self, CommandRun, Store};

use crate::job_commands::{CmdError, StartSpec};
use crate::jobs::{self, Job};

pub const DEFAULT_AGENT: &str = "omni";
const PROMPT_MAX: usize = 64 * 1024;

pub fn store() -> Option<Arc<Store>> {
    workflow::active()
}

pub fn boot() {
    let Some(dir) = omniget_core::core::llm::roster_store::llm_dir() else {
        return;
    };
    if let Err(e) = workflow::init(&dir.join(workflow::DB_FILE)) {
        tracing::warn!("[workflow] cannot open the workflow store: {e}");
    }
}

fn first_line(s: &str, max: usize) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(max)
        .collect()
}

pub fn apply(store: Option<&Store>, spec: &mut StartSpec) -> Result<(), CmdError> {
    if spec.workflow == Some(false) || spec.resume_of.is_some() {
        return Ok(());
    }
    let (Some(store), Some(ws)) = (store, spec.workspace.clone()) else {
        return Ok(());
    };
    let st = store.status(Path::new(&ws));
    if st.state == "none" {
        return Ok(());
    }
    let Some(wf) = st.active.clone() else {
        return Err(CmdError::new(
            400,
            workflow::ERR_INVALID,
            format!(
                "{}: {} (fix the file, or start without it: `workflow: false`, `--no-workflow`)",
                st.path,
                st.error.as_deref().unwrap_or("invalid")
            ),
        ));
    };
    let c = &wf.config;
    let mut filled: Vec<&str> = Vec::new();
    let mut held: Vec<&str> = Vec::new();
    if spec.agent_id.trim().is_empty() {
        if let Some(a) = &c.agent {
            spec.agent_id = a.clone();
            filled.push("agent");
        }
    }
    if spec.run_mode.is_none() {
        if let Some(m) = &c.run_mode {
            if st.trusted || !wf.run_mode_needs_trust() {
                spec.run_mode = Some(m.clone());
                filled.push("run_mode");
            } else {
                held.push("run_mode");
            }
        }
    }
    if spec.isolation.is_none() {
        if let Some(i) = &c.isolation {
            spec.isolation = Some(i.clone());
            filled.push("isolation");
        }
    }
    if spec.base.is_none() && spec.isolation.as_deref() != Some("shared") {
        if let Some(b) = &c.base {
            spec.base = Some(b.clone());
            filled.push("base");
        }
    }
    if spec.deadline_min.is_none() {
        if let Some(d) = c.deadline_min {
            spec.deadline_min = Some(d);
            filled.push("deadline_min");
        }
    }
    if spec.budget_usd.is_none() {
        if let Some(b) = c.budget_usd {
            spec.budget_usd = Some(b);
            filled.push("budget_usd");
        }
    }
    if spec.model.is_none() {
        if let Some(m) = &c.model {
            spec.model = Some(m.clone());
            filled.push("model");
        }
    }
    if spec.effort.is_none() {
        if let Some(e) = &c.effort {
            spec.effort = Some(e.clone());
            filled.push("effort");
        }
    }
    let task = spec.prompt.clone();
    let root = Path::new(&st.root);
    let vars: BTreeMap<&str, String> = [
        ("task", task.clone()),
        (
            "title",
            spec.title.clone().unwrap_or_else(|| first_line(&task, 80)),
        ),
        (
            "agent",
            if spec.agent_id.trim().is_empty() {
                DEFAULT_AGENT.to_string()
            } else {
                spec.agent_id.clone()
            },
        ),
        ("workspace", ws.clone()),
        ("repo", st.root.clone()),
        (
            "branch",
            spec.base
                .clone()
                .or_else(|| workflow::checked_out_branch(root))
                .unwrap_or_default(),
        ),
        ("base", spec.base.clone().unwrap_or_default()),
        (
            "isolation",
            spec.isolation.clone().unwrap_or_else(|| "auto".into()),
        ),
        (
            "run_mode",
            spec.run_mode
                .clone()
                .unwrap_or_else(|| "approval_required".into()),
        ),
        ("files", c.include.join("\n")),
    ]
    .into_iter()
    .collect();
    let rendered = wf
        .render(&vars)
        .map_err(|e| CmdError::new(400, workflow::ERR_TEMPLATE, e))?;
    if rendered.len() > PROMPT_MAX {
        return Err(CmdError::new(
            400,
            crate::job_commands::ERR_COMMAND_INVALID,
            "the prompt is over 64 KB after the workflow's template",
        ));
    }
    spec.prompt = rendered;
    let untrusted_parts = !st.trusted && !wf.gated().is_empty();
    spec.applied_workflow = Some(json!({
        "path": st.path,
        "root": st.root,
        "sha256": wf.sha256,
        "trusted": st.trusted,
        "from_last_good": st.from_last_good,
        "error": st.error,
        "filled": filled,
        "held": held,
        "skipped_reason": untrusted_parts.then(|| st.skipped_reason()),
        "task": task,
        "runs": [],
    }));
    Ok(())
}

pub fn default_agent(spec: &mut StartSpec) {
    if spec.agent_id.trim().is_empty() {
        spec.agent_id = DEFAULT_AGENT.to_string();
    }
}

fn root_of_job(job: &Job) -> Option<String> {
    job.origin.as_ref()?["workflow"]["root"]
        .as_str()
        .map(String::from)
}

fn hook_env(job: &Job, repo: &Path, wt: &Path, kind: &str) -> Vec<(String, String)> {
    vec![
        ("OMNIGET_HOOK".into(), kind.into()),
        ("OMNIGET_JOB_ID".into(), job.id.clone()),
        ("OMNIGET_REPO".into(), repo.to_string_lossy().into_owned()),
        ("OMNIGET_WORKTREE".into(), wt.to_string_lossy().into_owned()),
        (
            "OMNIGET_BRANCH".into(),
            job.branch.clone().unwrap_or_default(),
        ),
    ]
}

fn record_run(job: &mut Job, run: &CommandRun) {
    job.log.push_str(&run.note());
    if let Some(runs) = job
        .origin
        .as_mut()
        .and_then(|o| o["workflow"]["runs"].as_array_mut())
    {
        runs.push(json!(run));
    }
}

pub async fn after_create(
    store: Option<&Store>,
    job: &mut Job,
    repo: &Path,
    wt_root: &Path,
) -> Result<(), String> {
    let (Some(store), Some(_)) = (store, root_of_job(job)) else {
        return Ok(());
    };
    let st = store.status(repo);
    let Some(wf) = st.active.as_ref() else {
        return Ok(());
    };
    let copied =
        omniget_core::core::vcs::job_worktree::copy_included(repo, wt_root, &wf.config.include)
            .await;
    if !copied.is_empty() {
        job.log.push_str(&format!(
            "⎇ copied from the workflow's include: {}\n",
            copied.join(", ")
        ));
    }
    let Some(cmd) = wf.config.hooks.after_create.clone() else {
        return Ok(());
    };
    if !st.may_run_commands() {
        job.log.push_str(&format!(
            "⚠ after_create hook skipped: {}\n",
            st.skipped_reason()
        ));
        return Ok(());
    }
    let run = workflow::run_command(
        "after_create",
        &cmd,
        wt_root,
        &hook_env(job, repo, wt_root, "after_create"),
        workflow::HOOK_TIMEOUT,
    )
    .await;
    record_run(job, &run);
    if run.ok {
        Ok(())
    } else {
        Err(format!(
            "{}: the after_create hook failed ({}): {}",
            workflow::ERR_HOOK,
            run.note().trim(),
            run.output.trim()
        ))
    }
}

pub async fn before_remove(
    store: Option<&Store>,
    job: &Job,
    repo: &Path,
    wt_root: &Path,
) -> Result<String, String> {
    let (Some(store), Some(_)) = (store, root_of_job(job)) else {
        return Ok(String::new());
    };
    if !wt_root.is_dir() {
        return Ok(String::new());
    }
    let st = store.status(repo);
    let Some(cmd) = st
        .active
        .as_ref()
        .and_then(|w| w.config.hooks.before_remove.clone())
    else {
        return Ok(String::new());
    };
    if !st.may_run_commands() {
        return Ok(format!(
            "⚠ before_remove hook skipped: {}\n",
            st.skipped_reason()
        ));
    }
    let run = workflow::run_command(
        "before_remove",
        &cmd,
        wt_root,
        &hook_env(job, repo, wt_root, "before_remove"),
        workflow::HOOK_TIMEOUT,
    )
    .await;
    if run.ok {
        Ok(run.note())
    } else {
        Err(format!(
            "{}: the before_remove hook failed, the worktree stays ({}): {}",
            workflow::ERR_HOOK,
            run.note().trim(),
            run.output.trim()
        ))
    }
}

pub async fn run_checks(store: Option<&Store>, job: &mut Job) {
    let (Some(store), Some(root)) = (store, root_of_job(job)) else {
        return;
    };
    if !matches!(job.state.as_str(), "done" | "review" | "done_unreported") {
        return;
    }
    let st = store.status(Path::new(&root));
    let checks = st
        .active
        .as_ref()
        .map(|w| w.config.checks.clone())
        .unwrap_or_default();
    if checks.is_empty() {
        return;
    }
    let Some(dir) = jobs::effective_workspace(job) else {
        return;
    };
    if !st.may_run_commands() {
        let why = st.skipped_reason();
        job.log.push_str(&format!("⚠ checks skipped: {why}\n"));
        if let Some(p) = job.proof.as_mut() {
            p["checks_skipped"] = json!(why);
        }
        return;
    }
    let env = hook_env(job, Path::new(&root), Path::new(&dir), "check");
    for c in checks {
        let run =
            workflow::run_command("check", &c, Path::new(&dir), &env, workflow::CHECK_TIMEOUT)
                .await;
        let out = format!(
            "exit {}\n{}",
            run.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            run.output
        );
        jobs::push_check(job.proof.get_or_insert_with(|| json!({})), &c, run.ok, &out);
        record_run(job, &run);
    }
}

pub fn verdict(job: &Job) -> &'static str {
    let checks_failed = job
        .proof
        .as_ref()
        .and_then(|p| p["checks"].as_array())
        .is_some_and(|c| c.iter().any(|c| c["ok"] == json!(false)));
    let said_failed = job
        .outcome
        .as_ref()
        .is_some_and(|o| o["status"] == json!("failed"));
    match job.state.as_str() {
        "waiting_approval" => "waiting_approval",
        s if jobs::is_active(s) => "running",
        "failed" | "interrupted" => "failed",
        "cancelled" => "cancelled",
        _ if said_failed => "failed_by_agent",
        _ if checks_failed => "checks_failed",
        "review" => "needs_review",
        "done_unreported" => "unreported",
        _ => "succeeded",
    }
}

fn clip(s: &str, n: usize) -> String {
    crate::core::flight_recorder::redact(s)
        .chars()
        .take(n)
        .collect()
}

pub fn report(job: &Job, host_paths: bool) -> Value {
    let proof = job.proof.clone().unwrap_or(Value::Null);
    let short = |v: &Value| v.as_str().map(|s| s.chars().take(12).collect::<String>());
    let files: Vec<Value> = job
        .diff_files
        .as_deref()
        .unwrap_or_default()
        .iter()
        .take(50)
        .map(|f| json!({"path": f.path, "status": f.status, "additions": f.additions, "deletions": f.deletions, "binary": f.binary}))
        .collect();
    let mut wf = job
        .origin
        .as_ref()
        .map(|o| o["workflow"].clone())
        .filter(|w| !w.is_null());
    if let Some(w) = wf.as_mut().and_then(Value::as_object_mut) {
        w.remove("task");
        if !host_paths {
            w.remove("root");
            w.remove("path");
        }
    }
    let next = match verdict(job) {
        "waiting_approval" => "tell the person; answer only when they say so (approval_respond / omniget agent approve)",
        "running" => "wait (job_wait / omniget agent jobs <id>)",
        "needs_review" => "the job changed files: show the diff (job_diff / omniget agent diff <id>) before relying on it",
        "unreported" => "the agent ended without an OUTCOME line: read the result before relying on it",
        "checks_failed" => "the repository's checks failed after the job: say so",
        "failed" | "failed_by_agent" | "cancelled" => "say it failed, with the error or the agent's summary",
        _ => "report the outcome in two or three sentences",
    };
    let mut v = json!({
        "jobId": job.id,
        "state": job.state,
        "stateSeq": job.state_seq,
        "verdict": verdict(job),
        "title": job.title,
        "agentId": job.agent_id,
        "runMode": job.run_mode,
        "isolation": job.isolation.as_str(),
        "outcome": job.outcome,
        "result": job.result.as_deref().map(|r| clip(r, 2_000)),
        "jobError": job.error.as_deref().map(|e| clip(e, 1_000)),
        "failureReason": job.failure_reason,
        "diff": {
            "available": job.diff_files.is_some() && job.diff_unavailable.is_none(),
            "reason": job.diff_unavailable,
            "filesChanged": job.diff_files.as_ref().map(|f| f.len()),
            "additions": job.additions,
            "deletions": job.deletions,
            "clipped": job.diff_clipped,
            "files": files,
        },
        "proof": {
            "generatedBy": proof["generated_by"],
            "mode": proof["mode"],
            "branch": job.branch,
            "base": short(&proof["base"]),
            "head": short(&proof["head"]),
            "checks": proof["checks"],
            "checksSkipped": proof["checks_skipped"],
            "tokens": proof["tokens"],
            "costUsd": proof["cost_usd"],
            "durationMs": proof["duration_ms"],
        },
        "workflow": wf,
        "next": next,
    });
    if host_paths {
        v["proof"]["stamp"] = proof["stamp"].clone();
        v["workspace"] = json!(job.workspace);
        v["worktree"] = json!(job.worktree_path);
    }
    v
}

pub fn status_view(store: &Store, folder: &Path) -> Value {
    let st = store.status(folder);
    let mut v = json!(st);
    v["skipped_reason"] = json!((!st.trusted && !st.gated.is_empty()).then(|| st.skipped_reason()));
    v["variables"] = json!(workflow::VARIABLES
        .iter()
        .map(|(n, d)| json!({"name": n, "about": d}))
        .collect::<Vec<_>>());
    v
}

fn folder(raw: &str) -> Result<&Path, String> {
    let p = Path::new(raw);
    if raw.trim().is_empty() || !p.is_absolute() || !p.is_dir() {
        return Err("`workspace` is an absolute path to a folder".into());
    }
    Ok(p)
}

pub fn status_of(raw: &str) -> Result<Value, String> {
    let store = store().ok_or("the workflow store is not open")?;
    Ok(status_view(&store, folder(raw)?))
}

pub fn trust(
    raw: &str,
    sha256: &str,
    revoke: bool,
    actor: omniget_core::core::llm::audit::Actor,
) -> Result<Value, String> {
    let store = store().ok_or("the workflow store is not open")?;
    let dir = folder(raw)?;
    let by = format!("{}:{}", actor.channel, actor.id);
    let st = if revoke {
        store.untrust(dir)?
    } else {
        store.trust(dir, sha256, &by)?
    };
    omniget_core::core::llm::audit::log(
        actor,
        if revoke {
            "workflow.untrusted"
        } else {
            "workflow.trusted"
        },
        omniget_core::core::llm::audit::Entity::workflow(&st.root),
        json!({ "sha256": st.trusted_sha256, "commands": st.commands }),
    );
    Ok(status_view(&store, dir))
}
