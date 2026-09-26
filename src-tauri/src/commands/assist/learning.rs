//! Commands for procedural learning (`assist::learning`): what a bot learned
//! from feedback, its candidates and their evaluations, promotion and
//! rollback, export and forget. Shown in the bot panel, apart from memory.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use omniget_core::core::assist::db;
use omniget_core::core::assist::learning::{self, eval, NewCandidate, NewObservation, Settings};
use omniget_core::core::llm::budget::{Estimate, PoolLimits};
use omniget_core::core::llm::types::TurnEvent;

use crate::llm_manager::LlmManager;
use crate::AppState;

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assist_learning_overview(bot_id: String) -> Result<Value, String> {
    let db = db::global()?;
    let cands = learning::candidates(&db, &bot_id)?;
    let skills: std::collections::BTreeSet<String> =
        cands.iter().map(|c| c.skill.clone()).collect();
    let mut cases = serde_json::Map::new();
    for s in &skills {
        cases.insert(s.clone(), to_value(eval::cases(&db, &bot_id, s)?)?);
    }
    Ok(json!({
        "settings": learning::settings(&db, &bot_id)?,
        "observations": learning::observations(&db, &bot_id, None, 100)?,
        "candidates": cands,
        "active": learning::active_for_bot(&db, &bot_id)?,
        "promotions": learning::promotions(&db, &bot_id)?,
        "cases": cases,
        "running_evals": running_eval_jobs(&bot_id),
    }))
}

#[tauri::command]
pub async fn assist_learning_settings_save(settings: Settings) -> Result<Settings, String> {
    learning::set_settings(&*db::global()?, &settings)
}

/// Feedback typed by the person (explicit source).
#[tauri::command]
pub async fn assist_learning_observe(mut observation: NewObservation) -> Result<Value, String> {
    observation.source = Some(learning::Source::ExplicitFeedback);
    to_value(learning::observe(&*db::global()?, observation)?)
}

#[tauri::command]
pub async fn assist_learning_revoke(
    observation_id: String,
    reason: String,
) -> Result<Vec<String>, String> {
    learning::revoke_observation(&*db::global()?, &observation_id, &reason)
}

#[tauri::command]
pub async fn assist_learning_propose(candidate: NewCandidate) -> Result<Value, String> {
    to_value(learning::propose(&*db::global()?, candidate)?)
}

#[tauri::command]
pub async fn assist_learning_revise(
    candidate_id: String,
    overlay: String,
    reason: String,
    observations: Vec<String>,
) -> Result<Value, String> {
    to_value(learning::revise(
        &*db::global()?,
        &candidate_id,
        &overlay,
        &reason,
        &observations,
    )?)
}

#[tauri::command]
pub async fn assist_learning_candidate(candidate_id: String) -> Result<Value, String> {
    let db = db::global()?;
    Ok(json!({
        "candidate": learning::candidate(&db, &candidate_id)?,
        "versions": learning::versions(&db, &candidate_id)?,
        "runs": eval::runs_of(&db, &candidate_id)?,
    }))
}

#[tauri::command]
pub async fn assist_learning_add_case(case: eval::NewCase) -> Result<Value, String> {
    to_value(eval::add_case(&*db::global()?, case)?)
}

#[tauri::command]
pub async fn assist_learning_delete_case(case_id: String) -> Result<(), String> {
    eval::delete_case(&*db::global()?, &case_id)
}

#[tauri::command]
pub async fn assist_learning_record_fixture(
    case_id: String,
    variant: String,
    output: String,
) -> Result<(), String> {
    eval::record_fixture(&*db::global()?, &case_id, &variant, &output)
}

/// Live outputs: one model call per case and variant, reserved on the bot's
/// own day budget like any other turn (evaluation is not free). Each call
/// runs on a fresh conversation pinned to the variant's exact overlays, so
/// the learning augment injects those and nothing else: the baseline runs
/// with the active version only, the candidate with its version in place of
/// the active one (never both).
struct LiveRunner {
    llm: Arc<LlmManager>,
    bot: String,
}

#[async_trait]
impl eval::OutputRunner for LiveRunner {
    async fn output(
        &self,
        case: &eval::EvalCase,
        overlays: &[learning::ActiveOverlay],
    ) -> Result<String, String> {
        let agent = self
            .llm
            .agent(&self.bot)
            .ok_or_else(|| format!("no bot {}", self.bot))?;
        let prompt = case.input.clone();
        let overlay_len: usize = overlays.iter().map(|o| o.overlay.len()).sum();
        let reservation = self
            .llm
            .budget()
            .reserve(
                &format!("learning:{}", self.bot),
                &PoolLimits {
                    usd: agent.budget.usd_per_day,
                    tokens: None,
                    turns: Some(200),
                    strict_unknown: true,
                },
                Estimate {
                    usd: None,
                    tokens: ((prompt.len() + overlay_len) / 4) as u64 + 2_000,
                },
            )
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        let conv = format!("learn-eval-{}", uuid::Uuid::new_v4().simple());
        // Unpinned on every exit (the guard drops with this call).
        let _pin = learning::pin_exact(&conv, overlays.to_vec());
        let (request, _cancel, mut stream) =
            match self.llm.turn_stream(&conv, &self.bot, &prompt).await {
                Ok(v) => v,
                Err(e) => {
                    self.llm.budget().release(&reservation);
                    return Err(e);
                }
            };
        let mut text = String::new();
        let mut err = None;
        let (mut tin, mut tout, mut usd) = (0u32, 0u32, None);
        while let Some(ev) = stream.next().await {
            match ev {
                TurnEvent::TextDelta { text: t } => text.push_str(&t),
                TurnEvent::Usage { usage } => {
                    tin += usage.billable_input_tokens().min(u32::MAX as u64) as u32;
                    tout += usage.output_tokens;
                    if let Some(c) = usage.cost_usd {
                        usd = Some(usd.unwrap_or(0.0) + c);
                    }
                }
                TurnEvent::Error { error } => {
                    err = Some(format!("{}: {}", error.code, error.message))
                }
                _ => {}
            }
        }
        self.llm.finish_turn(&request, &self.bot);
        self.llm.drop_conversation(&conv);
        self.llm.budget().settle(&reservation, usd, tin, tout);
        match err {
            Some(e) if text.trim().is_empty() => Err(e),
            _ => Ok(text),
        }
    }
    fn label(&self) -> String {
        self.llm.model_label(&self.bot)
    }
}

#[derive(Debug, Deserialize)]
pub struct EvaluateInput {
    pub candidate_id: String,
    /// `offline` (fixtures only, never a call) or `live`.
    pub mode: String,
}

/// Event sent when a background (live) evaluation ends:
/// `{ run_id, candidate_id, bot_id, state: "done"|"failed", error? }`.
pub const EVENT_LEARNING_EVAL: &str = "assist://learning-eval";

/// A live evaluation in the background. It can take many minutes (one model
/// call per case and variant), longer than the webview keeps an IPC request
/// open: Tauri's IPC then re-sends the same command, which used to run the
/// whole evaluation twice. So the command returns a run id at once, the
/// work happens here, and the UI follows it by event or polling.
#[derive(Debug, Clone, Serialize)]
pub struct EvalJob {
    pub run_id: String,
    pub candidate_id: String,
    pub candidate_version: i64,
    pub bot_id: String,
    pub mode: String,
    /// `running|done|failed`
    pub state: String,
    pub error: Option<String>,
    pub started_ms: i64,
}

static EVAL_JOBS: std::sync::Mutex<Vec<EvalJob>> = std::sync::Mutex::new(Vec::new());

/// Registers a live evaluation of `candidate_id@version`, or returns the one
/// already running for it (a duplicate request, a double click, an IPC
/// re-send): the same run id, never a second evaluation. The bool says
/// whether this call created it.
pub fn claim_eval_job(
    candidate_id: &str,
    version: i64,
    bot_id: &str,
    mode: &str,
    now: i64,
) -> (EvalJob, bool) {
    let mut jobs = EVAL_JOBS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(j) = jobs.iter().find(|j| {
        j.candidate_id == candidate_id && j.candidate_version == version && j.state == "running"
    }) {
        return (j.clone(), false);
    }
    // Keep finished jobs for a while so a late status poll still answers.
    jobs.retain(|j| j.state == "running" || now - j.started_ms < 6 * 3_600_000);
    let job = EvalJob {
        run_id: omniget_core::core::assist::new_id(),
        candidate_id: candidate_id.into(),
        candidate_version: version,
        bot_id: bot_id.into(),
        mode: mode.into(),
        state: "running".into(),
        error: None,
        started_ms: now,
    };
    jobs.push(job.clone());
    (job, true)
}

pub fn finish_eval_job(run_id: &str, error: Option<String>) -> Option<EvalJob> {
    let mut jobs = EVAL_JOBS.lock().unwrap_or_else(|e| e.into_inner());
    let j = jobs.iter_mut().find(|j| j.run_id == run_id)?;
    j.state = if error.is_some() { "failed" } else { "done" }.into();
    j.error = error;
    Some(j.clone())
}

pub fn eval_job(run_id: &str) -> Option<EvalJob> {
    EVAL_JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|j| j.run_id == run_id)
        .cloned()
}

fn running_eval_jobs(bot_id: &str) -> Vec<EvalJob> {
    EVAL_JOBS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|j| j.bot_id == bot_id && j.state == "running")
        .cloned()
        .collect()
}

/// Offline: replays fixtures right away and returns
/// `{ run_id, state: "done", run }`. Live: returns `{ run_id, state:
/// "running", already_running }` immediately and evaluates in the
/// background; [`assist_learning_eval_status`] and
/// [`EVENT_LEARNING_EVAL`] tell when it ends.
#[tauri::command]
pub async fn assist_learning_evaluate(
    app: AppHandle,
    state: State<'_, AppState>,
    input: EvaluateInput,
) -> Result<Value, String> {
    let db = db::global()?;
    let c = learning::candidate(&db, &input.candidate_id)?;
    if input.mode != "live" {
        let run = eval::evaluate(&db, &input.candidate_id, eval::Mode::Offline, None).await?;
        return Ok(json!({ "run_id": run.id, "state": "done", "run": run }));
    }
    let (job, created) = claim_eval_job(
        &c.id,
        c.current_version,
        &c.bot_id,
        "live",
        omniget_core::core::assist::now_ms(),
    );
    if created {
        let runner = LiveRunner {
            llm: state.llm.clone(),
            bot: c.bot_id.clone(),
        };
        let job2 = job.clone();
        tauri::async_runtime::spawn(async move {
            let out = eval::evaluate_as(
                &db,
                &job2.candidate_id,
                eval::Mode::Live,
                Some(&runner as &dyn eval::OutputRunner),
                &job2.run_id,
            )
            .await;
            let error = out.err();
            if let Some(e) = &error {
                tracing::warn!("[learning] live evaluation {}: {e}", job2.run_id);
                // The candidate must not stay `evaluating` after a failure.
                if let Ok(cur) = learning::candidate(&db, &job2.candidate_id) {
                    if cur.state == learning::CandidateState::Evaluating
                        && cur.reason.as_deref()
                            == Some(&format!("evaluating (run {})", job2.run_id))
                    {
                        let _ = learning::set_state(
                            &db,
                            &job2.candidate_id,
                            learning::CandidateState::Proposed,
                            e,
                        );
                    }
                }
            }
            let fin = finish_eval_job(&job2.run_id, error.clone());
            let _ = app.emit(
                EVENT_LEARNING_EVAL,
                json!({ "run_id": job2.run_id, "candidate_id": job2.candidate_id, "bot_id": job2.bot_id, "state": fin.map(|j| j.state).unwrap_or_else(|| "done".into()), "error": error }),
            );
        });
    }
    Ok(
        json!({ "run_id": job.run_id, "state": "running", "already_running": !created, "candidate_id": job.candidate_id }),
    )
}

/// State of a background evaluation: `{ state: running|done|failed, run?,
/// error? }`. A run already stored answers `done` even after a restart.
#[tauri::command]
pub async fn assist_learning_eval_status(run_id: String) -> Result<Value, String> {
    let db = db::global()?;
    if let Ok(run) = eval::run(&db, &run_id) {
        return Ok(json!({ "run_id": run_id, "state": "done", "run": run }));
    }
    match eval_job(&run_id) {
        Some(j) => Ok(
            json!({ "run_id": run_id, "state": j.state, "error": j.error, "candidate_id": j.candidate_id }),
        ),
        None => Err(format!(
            "{}: eval run {run_id}",
            learning::ERR_LEARN_NOT_FOUND
        )),
    }
}

#[tauri::command]
pub async fn assist_learning_promote(
    candidate_id: String,
    eval_run_id: String,
) -> Result<Value, String> {
    to_value(learning::promote(
        &*db::global()?,
        &candidate_id,
        &eval_run_id,
        "user",
    )?)
}

#[tauri::command]
pub async fn assist_learning_rollback(
    bot_id: String,
    skill: String,
    scope: Option<String>,
    reason: String,
) -> Result<Value, String> {
    let scope = scope.unwrap_or_else(|| learning::bot_scope(&bot_id));
    to_value(learning::rollback(
        &*db::global()?,
        &bot_id,
        &skill,
        &scope,
        "user",
        &reason,
    )?)
}

#[tauri::command]
pub async fn assist_learning_export(bot_id: String, path: Option<String>) -> Result<Value, String> {
    let v = learning::export(&*db::global()?, &bot_id)?;
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        std::fs::write(
            &p,
            serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        return Ok(json!({ "path": p }));
    }
    Ok(v)
}

#[tauri::command]
pub async fn assist_learning_forget(bot_id: String) -> Result<usize, String> {
    learning::forget(&*db::global()?, &bot_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_live_evaluation_request_returns_the_same_run() {
        // Live demo 2026-09-25: one `invoke` ran the evaluation twice, 333 s
        // apart (the webview re-sent the command after ~5 minutes).
        let cid = format!("cand-{}", omniget_core::core::assist::new_id());
        let (first, created) = claim_eval_job(&cid, 3, "bot", "live", 1_000);
        assert!(created);
        let (again, created2) = claim_eval_job(&cid, 3, "bot", "live", 1_000 + 333_000);
        assert!(!created2, "a duplicate never starts a second evaluation");
        assert_eq!(again.run_id, first.run_id);
        assert_eq!(eval_job(&first.run_id).unwrap().state, "running");
        // Another version is another evaluation.
        let (other, created3) = claim_eval_job(&cid, 4, "bot", "live", 2_000);
        assert!(created3);
        assert_ne!(other.run_id, first.run_id);
        // Once it ends, a new request starts a new run.
        assert_eq!(finish_eval_job(&first.run_id, None).unwrap().state, "done");
        let (next, created4) = claim_eval_job(&cid, 3, "bot", "live", 3_000);
        assert!(created4);
        assert_ne!(next.run_id, first.run_id);
        assert_eq!(
            finish_eval_job(&next.run_id, Some("boom".into()))
                .unwrap()
                .state,
            "failed"
        );
        finish_eval_job(&other.run_id, None);
    }
}
