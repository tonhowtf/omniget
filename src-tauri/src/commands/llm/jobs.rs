//! `llm_job_*`, `llm_loop_*`, `llm_trigger_*`: the UI door of `crate::jobs`.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::jobs::{self, LoopDef, Trigger};

fn to<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_jobs_list(app: AppHandle, limit: Option<u32>) -> Result<Value, String> {
    to(jobs::get(&app)?.list(limit.unwrap_or(100)))
}

#[tauri::command]
pub async fn llm_job_get(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.job(&id))
}

/// `isolation`: `shared | worktree | auto` (absent = `auto`); `base`: the
/// ref a worktree starts from (2.3); `run_mode`: what the job may do without
/// asking (2.7; absent = `approval_required`, the default of a `run`).
#[tauri::command]
pub async fn llm_job_submit(
    app: AppHandle,
    agent_id: String,
    prompt: String,
    workspace: Option<String>,
    isolation: Option<String>,
    base: Option<String>,
    run_mode: Option<String>,
) -> Result<Value, String> {
    let isolation = isolation
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(jobs::Isolation::parse)
        .transpose()?;
    let run_mode = run_mode
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(jobs::RunMode::parse)
        .transpose()?;
    let job = jobs::get(&app)?.submit_as(
        "run", &agent_id, &prompt, workspace, None, isolation, base, run_mode,
    )?;
    crate::llm_audit::job_created(omniget_core::core::llm::audit::Actor::window(), &job);
    to(job)
}

/// Removes a finished job's worktree (2.3): its uncommitted work is saved
/// on the job's branch first, and a branch with work is kept. Returns what
/// happened (`dirty`, `saved`, `preserved_branch`, `branch_deleted`).
#[tauri::command]
pub async fn llm_job_remove_worktree(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.remove_worktree(&id).await?)
}

/// What runs, what waits (position and reason) and the limits: the queue
/// view of the jobs page (the same as `GET /v1/state`).
#[tauri::command]
pub async fn llm_jobs_state(app: AppHandle) -> Result<Value, String> {
    Ok(jobs::get(&app)?.state_view())
}

/// Saves the parallel caps and applies them to the running gate at once.
#[tauri::command]
pub async fn llm_jobs_limits_set(
    app: AppHandle,
    limits: jobs::JobLimitsInput,
) -> Result<Value, String> {
    let mut current = crate::storage::config::load_settings(&app);
    current.llm = jobs::merge_limits(current.llm, &limits);
    crate::storage::config::save_settings(&app, &current).map_err(|e| format!("Save: {e}"))?;
    let j = jobs::get(&app)?;
    j.apply_limits(&current.llm);
    Ok(j.state_view())
}

#[tauri::command]
pub async fn llm_job_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.cancel_by(&id, omniget_core::core::llm::audit::Actor::window())?)
}

#[tauri::command]
pub async fn llm_job_delete(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.delete(&id)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn llm_loops_list(app: AppHandle) -> Result<Value, String> {
    to(jobs::get(&app)?.loops())
}

#[tauri::command]
pub async fn llm_loop_create(app: AppHandle, def: LoopDef) -> Result<Value, String> {
    to(jobs::get(&app)?.loop_create(def)?)
}

#[tauri::command]
pub async fn llm_loop_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.loop_cancel(&id)?)
}

#[tauri::command]
pub async fn llm_loop_delete(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.loop_delete(&id)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn llm_triggers_list(app: AppHandle) -> Result<Value, String> {
    let bridge = jobs::bridge_info(&app);
    let triggers: Vec<Value> = jobs::get(&app)?
        .triggers()
        .iter()
        .map(jobs::trigger_view)
        .collect();
    Ok(json!({ "triggers": triggers, "bridge": bridge }))
}

/// Returns the routine as the list shows it (next run, missed), not the raw
/// row.
#[tauri::command]
pub async fn llm_trigger_save(app: AppHandle, trigger: Trigger) -> Result<Value, String> {
    Ok(jobs::trigger_view(&jobs::get(&app)?.trigger_save(trigger)?))
}

#[tauri::command]
pub async fn llm_trigger_delete(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.trigger_delete(&id)?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn llm_trigger_fire(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.fire(&id, None)?)
}

/// An interrupted job again (a person asked; the agent is told to check
/// what was already done first).
#[tauri::command]
pub async fn llm_job_resume(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.job_resume(&id)?)
}

#[tauri::command]
pub async fn llm_job_mark_done(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.job_mark_done(&id)?)
}

#[tauri::command]
pub async fn llm_job_discard(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.job_discard(&id)?)
}

#[tauri::command]
pub async fn llm_loop_resume(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.loop_resume(&id)?)
}

/// `done: true` marks an interrupted Loop finished, `false` discards it.
#[tauri::command]
pub async fn llm_loop_settle(app: AppHandle, id: String, done: bool) -> Result<Value, String> {
    to(jobs::get(&app)?.loop_settle(&id, done)?)
}

#[tauri::command]
pub async fn llm_trigger_mute(app: AppHandle, id: String, muted: bool) -> Result<Value, String> {
    to(jobs::trigger_view(
        &jobs::get(&app)?.trigger_mute(&id, muted)?,
    ))
}

/// What the job changed: files, totals, why it is unavailable, and the
/// patch (the jobs page; the bridge serves the same as `GET
/// /v1/jobs/{id}/diff`).
#[tauri::command]
pub async fn llm_job_diff(app: AppHandle, id: String) -> Result<Value, String> {
    let j = jobs::get(&app)?;
    let (job, patch, complete) = j.diff(&id).await?;
    let path = jobs::patch_path(&job.id);
    match jobs::diff_view(&job, patch.as_deref(), path.as_deref(), "files", complete) {
        Ok(jobs::DiffBody::Json(mut v)) => {
            v["patch"] = json!(patch.unwrap_or_default());
            Ok(v)
        }
        Ok(jobs::DiffBody::Patch(_)) => Err(format!("{}: unexpected patch", jobs::ERR_JOBS)),
        Err((_, e)) => Err(e),
    }
}

/// The user opened a job: it stops counting as "finished, not seen".
#[tauri::command]
pub async fn llm_job_seen(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.mark_seen(&id)?)
}

/// Why a job is in its state right now: last event age, pending question,
/// deadline left (`omniget agent explain` reads the same through the bridge).
#[tauri::command]
pub async fn llm_job_explain(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?
        .explain(&id)
        .ok_or_else(|| format!("{}: no job {id}", jobs::ERR_JOBS))
}

/// What the window shows, so a notification about a job already on screen
/// is not sent (`llm_attention`). Cheap: called on every navigation.
#[tauri::command]
pub fn llm_ui_focus(path: String, job_id: Option<String>, conversation_id: Option<String>) {
    crate::llm_attention::report_focus(path, job_id, conversation_id);
}

/// The notification texts in the interface language (`$lib/llm-notify-strings`);
/// the backend has no locale of its own, like the tray.
#[tauri::command]
pub fn sync_llm_notify_strings(strings: crate::llm_attention::NotifyStrings) {
    crate::llm_attention::push_strings(strings.clone());
    if let Some(jobs) = jobs::try_get() {
        jobs.attention.set_strings(strings);
    }
}

// ── bridge access tokens (2.1) ─────────────────────────────────────────

/// The scoped tokens (never their secret) and the grace period of the
/// extension key on the agent routes: whether it is on and when that key
/// was last used there, so the page can say "switch this script".
#[tauri::command]
pub async fn llm_bridge_tokens(app: AppHandle) -> Result<Value, String> {
    let settings = crate::storage::config::load_settings(&app);
    let log = crate::bridge_tokens::legacy_log();
    Ok(json!({
        "tokens": jobs::get(&app)?.token_list(),
        "scopes": crate::bridge_tokens::SCOPES,
        "legacy": {
            "allowed": settings.bridge.legacy_agent_routes,
            "last": log.last(),
            "count": log.count(),
        },
    }))
}

/// Mints a user token. The secret is in this answer only: the page offers
/// to copy it and never prints it.
#[tauri::command]
pub async fn llm_bridge_token_mint(
    app: AppHandle,
    name: String,
    scopes: Vec<String>,
    expires_days: Option<u32>,
) -> Result<Value, String> {
    let expires_ms = expires_days
        .filter(|d| *d > 0)
        .map(|d| crate::llm_manager::now_ms() + u64::from(d.min(3650)) * 86_400_000);
    let minted = jobs::get(&app)?.token_mint(&name, &scopes, expires_ms)?;
    // The line names the token by id and scopes, never its secret (2.8).
    omniget_core::core::llm::audit::log(
        omniget_core::core::llm::audit::Actor::window(),
        "token.minted",
        omniget_core::core::llm::audit::Entity::token(&minted.view.id),
        json!({ "name": minted.view.name, "scopes": minted.view.scopes, "expires_ms": minted.view.expires_ms }),
    );
    Ok(json!({ "token": minted.token, "view": minted.view }))
}

#[tauri::command]
pub async fn llm_bridge_token_revoke(app: AppHandle, id: String) -> Result<Value, String> {
    let revoked = jobs::get(&app)?.token_revoke(&id)?;
    if revoked {
        omniget_core::core::llm::audit::log(
            omniget_core::core::llm::audit::Actor::window(),
            "token.revoked",
            omniget_core::core::llm::audit::Entity::token(&id),
            json!({}),
        );
    }
    Ok(json!({ "revoked": revoked }))
}

/// Turns the extension key's grace period on the agent routes on or off.
#[tauri::command]
pub async fn llm_bridge_legacy_set(app: AppHandle, allowed: bool) -> Result<Value, String> {
    let mut settings = crate::storage::config::load_settings(&app);
    settings.bridge.legacy_agent_routes = allowed;
    crate::storage::config::save_settings(&app, &settings).map_err(|e| e.to_string())?;
    crate::bridge_tokens::legacy_flag().store(allowed, std::sync::atomic::Ordering::Relaxed);
    Ok(json!({ "allowed": allowed }))
}

// ── review (3.5) ─────────────────────────────────────────────────────

/// The job's diff as hunks with ids and the patch of each file.
#[tauri::command]
pub async fn llm_job_hunks(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.review_hunks(&id).await
}

/// Puts one hunk back: `hunk_id` and `patch` must be the hunk as it is now,
/// byte for byte (`ERR_HUNK_STALE` otherwise: refresh).
#[tauri::command]
pub async fn llm_job_discard_hunk(
    app: AppHandle,
    id: String,
    hunk_id: String,
    patch: String,
) -> Result<Value, String> {
    jobs::get(&app)?.discard_hunk(&id, &hunk_id, &patch).await
}

#[tauri::command]
pub async fn llm_job_notes_list(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.notes(&id))
}

/// A note on lines `start_line..=end_line` of `path` (new version), or on
/// the whole file when both are absent.
#[tauri::command]
pub async fn llm_job_note_add(
    app: AppHandle,
    id: String,
    path: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
    body: String,
) -> Result<Value, String> {
    to(jobs::get(&app)?.note_add(&id, &path, start_line, end_line, &body)?)
}

#[tauri::command]
pub async fn llm_job_note_delete(
    app: AppHandle,
    id: String,
    note_id: String,
) -> Result<Value, String> {
    jobs::get(&app)?.note_delete(&id, &note_id)?;
    Ok(json!({ "ok": true }))
}

/// Sends the unsent notes to the agent: a follow-up job in the same
/// conversation and worktree. Returns the new job.
#[tauri::command]
pub async fn llm_job_notes_send(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.notes_send(&id).await?)
}

/// What integrating the job's branch would do (target branch, blocker, commits).
#[tauri::command]
pub async fn llm_job_integrate_preview(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.integrate_preview(&id).await
}

/// `strategy`: `merge` (`--no-ff`, default) or `squash`.
#[tauri::command]
pub async fn llm_job_integrate(
    app: AppHandle,
    id: String,
    strategy: Option<String>,
) -> Result<Value, String> {
    let strategy =
        omniget_core::core::vcs::integrate::Strategy::parse(strategy.as_deref().unwrap_or(""))
            .map_err(|e| format!("{}: {e}", jobs::ERR_JOBS))?;
    let (outcome, job) = jobs::get(&app)?.integrate(&id, strategy).await?;
    Ok(json!({ "outcome": outcome, "job": job }))
}

/// Throws the worktree job's work away: worktree and branch.
#[tauri::command]
pub async fn llm_job_discard_worktree(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.discard_work(&id).await
}

/// The walkthrough of the job's diff, on demand (`refresh` skips the cache).
#[tauri::command]
pub async fn llm_job_walkthrough(
    app: AppHandle,
    id: String,
    refresh: Option<bool>,
) -> Result<Value, String> {
    to(jobs::get(&app)?
        .walkthrough(&id, refresh.unwrap_or(false))
        .await?)
}
