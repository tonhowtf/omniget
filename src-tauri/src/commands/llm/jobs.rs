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
    let blank = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let mut spec = crate::job_commands::StartSpec {
        agent_id: agent_id.trim().to_string(),
        prompt,
        workspace: blank(workspace),
        isolation: blank(isolation),
        base: blank(base),
        run_mode: blank(run_mode),
        ..Default::default()
    };
    if let Some(i) = &spec.isolation {
        jobs::Isolation::parse(i)?;
    }
    if let Some(m) = &spec.run_mode {
        jobs::RunMode::parse(m)?;
    }
    crate::job_workflow::apply(crate::job_workflow::store().as_deref(), &mut spec)
        .map_err(|e| e.text())?;
    crate::job_workflow::default_agent(&mut spec);
    let job = jobs::get(&app)?.submit_window(&spec)?;
    crate::llm_audit::job_created(omniget_core::core::llm::audit::Actor::window(), &job);
    to(job)
}

#[tauri::command]
pub async fn llm_workflow_status(folder: String) -> Result<Value, String> {
    crate::job_workflow::status_of(&folder)
}

#[tauri::command]
pub async fn llm_workflow_trust(
    folder: String,
    sha256: Option<String>,
    revoke: Option<bool>,
) -> Result<Value, String> {
    crate::job_workflow::trust(
        &folder,
        sha256.as_deref().unwrap_or(""),
        revoke.unwrap_or(false),
        omniget_core::core::llm::audit::Actor::window(),
    )
}

#[tauri::command]
pub async fn llm_job_remove_worktree(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.remove_worktree(&id).await?)
}

#[tauri::command]
pub async fn llm_jobs_state(app: AppHandle) -> Result<Value, String> {
    Ok(jobs::get(&app)?.state_view())
}

#[tauri::command]
pub async fn llm_jobs_limits_set(
    app: AppHandle,
    limits: jobs::JobLimitsInput,
) -> Result<Value, String> {
    let j = jobs::get(&app)?;
    crate::storage::config::update_settings(&app, |current| {
        current.llm = jobs::merge_limits(current.llm.clone(), &limits);
    })
    .map_err(|e| format!("Save: {e}"))?;
    crate::storage::settings_effects::settle(&[
        crate::storage::settings_effects::EffectId::LlmJobs,
    ]);
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

#[tauri::command]
pub async fn llm_job_seen(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.mark_seen(&id)?)
}

#[tauri::command]
pub fn llm_ui_focus(path: String, job_id: Option<String>, conversation_id: Option<String>) {
    crate::llm_attention::report_focus(path, job_id, conversation_id);
}

#[tauri::command]
pub fn sync_llm_notify_strings(strings: crate::llm_attention::NotifyStrings) {
    crate::llm_attention::push_strings(strings.clone());
    if let Some(jobs) = jobs::try_get() {
        jobs.attention.set_strings(strings);
    }
}

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

#[tauri::command]
pub async fn llm_bridge_legacy_set(app: AppHandle, allowed: bool) -> Result<Value, String> {
    crate::storage::config::update_settings(&app, |s| s.bridge.legacy_agent_routes = allowed)
        .map_err(|e| e.to_string())?;
    crate::bridge_tokens::legacy_flag().store(allowed, std::sync::atomic::Ordering::Relaxed);
    Ok(json!({ "allowed": allowed }))
}

#[tauri::command]
pub async fn llm_job_hunks(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.review_hunks(&id).await
}

#[tauri::command]
pub async fn llm_job_discard_hunk(
    app: AppHandle,
    id: String,
    hunk_id: String,
    patch: String,
) -> Result<Value, String> {
    let r = jobs::get(&app)?.discard_hunk(&id, &hunk_id, &patch).await?;
    crate::llm_audit::job_reviewed(
        omniget_core::core::llm::audit::Actor::window(),
        "job.hunk_discarded",
        &id,
        json!({ "hunk_id": hunk_id }),
    );
    Ok(r)
}

#[tauri::command]
pub async fn llm_job_notes_list(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.notes(&id))
}

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

#[tauri::command]
pub async fn llm_job_notes_send(app: AppHandle, id: String) -> Result<Value, String> {
    to(jobs::get(&app)?.notes_send(&id).await?)
}

#[tauri::command]
pub async fn llm_job_integrate_preview(app: AppHandle, id: String) -> Result<Value, String> {
    jobs::get(&app)?.integrate_preview(&id).await
}

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
    crate::llm_audit::job_reviewed(
        omniget_core::core::llm::audit::Actor::window(),
        "job.integrated",
        &id,
        json!({ "strategy": strategy, "status": outcome.status }),
    );
    Ok(json!({ "outcome": outcome, "job": job }))
}

#[tauri::command]
pub async fn llm_job_discard_worktree(app: AppHandle, id: String) -> Result<Value, String> {
    let r = jobs::get(&app)?.discard_work(&id).await?;
    crate::llm_audit::job_reviewed(
        omniget_core::core::llm::audit::Actor::window(),
        "job.discarded",
        &id,
        json!({}),
    );
    Ok(r)
}

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
