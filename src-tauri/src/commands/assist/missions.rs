//! Commands for missions (`assist::missions` + the driver in `crate::missions`):
//! create from the Chat or the Activity screen, follow tasks, criteria,
//! receipts, budget and checkpoints, correct the context, accept or reject a
//! subjective criterion, resume, pause, cancel, export a redacted
//! diagnostic bundle, and apply the presets.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use omniget_core::core::assist::missions::{
    self, checkpoint, diag, verify, Criterion, MissionState, NewMission, NewReceipt, NewTask,
};
use omniget_core::core::assist::packs::presets;
use omniget_core::core::assist::{db, groups, now_ms};
use omniget_core::core::llm::code_tools;

use crate::llm_manager::sanitize_id;
use crate::missions::driver;
use crate::AppState;

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct MissionRow {
    #[serde(flatten)]
    pub mission: missions::Mission,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub verdict: String,
    /// How many required criteria are in each state, counted from the same
    /// verdict the detail view counts (the English `verdict` summary joins
    /// titles with commas, so a title with a comma was counted twice).
    pub verdict_counts: VerdictCounts,
    pub completion: String,
    pub driving: bool,
}

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct VerdictCounts {
    pub passed: bool,
    pub failing: usize,
    pub missing: usize,
    pub awaiting_human: usize,
    pub partial: usize,
}

impl VerdictCounts {
    pub fn of(v: &verify::Verdict) -> Self {
        Self {
            passed: v.passed,
            failing: v.failing.len(),
            missing: v.missing.len(),
            awaiting_human: v.awaiting_human.len(),
            partial: v.partial.len(),
        }
    }
}

#[tauri::command]
pub async fn assist_mission_list(
    app: AppHandle,
    filter: Option<missions::ListFilter>,
) -> Result<Vec<MissionRow>, String> {
    let d = driver(&app)?;
    let db = d.db();
    let mut out = Vec::new();
    for m in missions::list(db, &filter.unwrap_or_default())? {
        let ts = missions::tasks(db, &m.id)?;
        let crit = missions::criteria(db, &m.id, Some(m.criteria_version))?;
        let recs = missions::receipts(db, &m.id)?;
        // The artifacts as they are now, like `missions::detail`: a list that
        // trusts stored receipts alone disagrees with the detail view.
        let ws = m.workspace.clone().map(std::path::PathBuf::from);
        let since = m.created_ms;
        let dflt = |c: &Criterion| verify::digest_for(Some(db), ws.as_deref(), c, since);
        let v = verify::verdict(&crit, &recs, &missions::with_revision(db, &m.id, &dflt));
        out.push(MissionRow {
            tasks_total: ts.len(),
            tasks_done: ts
                .iter()
                .filter(|t| t.state == missions::TaskState::Done)
                .count(),
            verdict: v.summary(),
            verdict_counts: VerdictCounts::of(&v),
            completion: v.completion.clone(),
            driving: d.is_driving(&m.id),
            mission: m,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn assist_mission_get(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let detail = missions::detail(d.db(), &id)?;
    let checkpoints = checkpoint::list(d.db(), &id)?;
    let budget = omniget_core::core::assist::missions::budget_pool(&id);
    let pool = app_state_budget(&app).map(|b| b.pool(&budget));
    let mut v = to_value(&detail)?;
    v["driving"] = json!(d.is_driving(&id));
    v["checkpoints"] = to_value(
        checkpoints
            .iter()
            .map(|c| json!({ "seq": c.seq, "reason": c.reason, "created_ms": c.created_ms }))
            .collect::<Vec<_>>(),
    )?;
    v["pool"] = to_value(pool)?;
    v["context_preview"] = json!(detail
        .last_checkpoint
        .as_ref()
        .map(|c| checkpoint::rebuild_context(c, 4000)));
    Ok(v)
}

fn app_state_budget(
    app: &AppHandle,
) -> Option<std::sync::Arc<omniget_core::core::llm::budget::BudgetStore>> {
    use tauri::Manager;
    Some(app.state::<AppState>().llm.budget())
}

/// Creates (and, with `start`, queues) a mission.
#[tauri::command]
pub async fn assist_mission_create(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: NewMission,
) -> Result<Value, String> {
    if let Some(b) = draft.bot_id.as_deref() {
        if state.llm.agent(b).is_none() {
            return Err(format!("{}: no bot `{b}`", missions::ERR_MISSION_INPUT));
        }
    }
    let d = driver(&app)?;
    let start = draft.start;
    let detail = missions::create(d.db(), draft)?;
    if start {
        d.start(&detail.mission.id);
    }
    d.emit(&detail.mission.id);
    to_value(detail)
}

#[derive(Debug, Deserialize)]
pub struct FromChat {
    pub conversation_id: String,
    pub bot_id: String,
    pub objective: String,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    #[serde(default)]
    pub budget: missions::MissionBudget,
    #[serde(default)]
    pub start: bool,
}

/// From the Chat: the request becomes the objective; the conversation's
/// project folder (if any) becomes the workspace; the turns run in the same
/// conversation, so the chat shows them.
#[tauri::command]
pub async fn assist_mission_from_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    input: FromChat,
) -> Result<Value, String> {
    if state.llm.agent(&input.bot_id).is_none() {
        return Err(format!(
            "{}: no bot `{}`",
            missions::ERR_MISSION_INPUT,
            input.bot_id
        ));
    }
    let workspace = code_tools::workspace_of(&sanitize_id(&input.conversation_id))
        .map(|p| p.to_string_lossy().to_string());
    let room_id = groups::store::room_of(&input.conversation_id).map(str::to_string);
    let d = driver(&app)?;
    let detail = missions::create(
        d.db(),
        NewMission {
            objective: input.objective,
            bot_id: Some(input.bot_id),
            room_id,
            conversation_id: Some(input.conversation_id.clone()),
            auth_origin: format!("chat:{}", input.conversation_id),
            criteria: input.criteria,
            budget: input.budget,
            workspace,
            start: input.start,
            ..Default::default()
        },
    )?;
    if input.start {
        d.start(&detail.mission.id);
    }
    d.emit(&detail.mission.id);
    to_value(detail)
}

#[tauri::command]
pub async fn assist_mission_start(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::get(d.db(), &id)?;
    if m.state == MissionState::Draft {
        missions::transition(
            d.db(),
            &id,
            MissionState::Queued,
            "started by the user",
            None,
        )?;
    } else if !m.state.is_active() {
        missions::resume(d.db(), &id, "started by the user")?;
    }
    d.start(&id);
    d.emit(&id);
    to_value(missions::get(d.db(), &id)?)
}

#[tauri::command]
pub async fn assist_mission_pause(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let out = missions::pause(d.db(), &id)?;
    d.stop_jobs(&out.live_jobs);
    d.emit(&id);
    to_value(out)
}

#[tauri::command]
pub async fn assist_mission_resume(
    app: AppHandle,
    id: String,
    note: Option<String>,
) -> Result<Value, String> {
    let d = driver(&app)?;
    if let Some(n) = note.as_deref().filter(|n| !n.trim().is_empty()) {
        checkpoint::add_note(d.db(), &id, n)?;
    }
    let m = missions::resume(d.db(), &id, "resumed by the user")?;
    d.start(&id);
    d.emit(&id);
    to_value(m)
}

#[tauri::command]
pub async fn assist_mission_cancel(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let out = missions::cancel(d.db(), &id, "cancelled by the user")?;
    d.stop_jobs(&out.live_jobs);
    d.emit(&id);
    to_value(out)
}

#[tauri::command]
pub async fn assist_mission_delete(app: AppHandle, id: String) -> Result<(), String> {
    let d = driver(&app)?;
    missions::delete(d.db(), &id)
}

/// The person corrects or adds context; it reaches every later task.
#[tauri::command]
pub async fn assist_mission_note(app: AppHandle, id: String, text: String) -> Result<(), String> {
    let d = driver(&app)?;
    checkpoint::add_note(d.db(), &id, &text)?;
    d.emit(&id);
    Ok(())
}

#[tauri::command]
pub async fn assist_mission_revise_criteria(
    app: AppHandle,
    id: String,
    criteria: Vec<Criterion>,
    reason: String,
) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::revise_criteria(d.db(), &id, &criteria, &reason)?;
    d.emit(&id);
    to_value(m)
}

/// A person accepts or rejects a criterion (rubric or human). The receipt is
/// tied to the artifact as it is now.
#[tauri::command]
pub async fn assist_mission_accept(
    app: AppHandle,
    id: String,
    criterion_id: String,
    accept: bool,
    note: Option<String>,
) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::get(d.db(), &id)?;
    let crit = missions::criteria(d.db(), &id, None)?;
    let c = crit.iter().find(|c| c.id == criterion_id).ok_or_else(|| {
        format!(
            "{}: no criterion {criterion_id}",
            missions::ERR_MISSION_INPUT
        )
    })?;
    let digest = verify::digest_for(
        Some(d.db()),
        m.workspace.as_deref().map(std::path::Path::new),
        c,
        m.created_ms,
    )
    .unwrap_or_default();
    missions::record_receipt(
        d.db(),
        NewReceipt {
            mission_id: id.clone(),
            criterion_id: c.id.clone(),
            criterion_version: c.version,
            artifact_digest: digest,
            verifier: "human".into(),
            verifier_version: "1".into(),
            status: if accept { "pass" } else { "fail" }.into(),
            evidence: note.unwrap_or_default(),
            ..Default::default()
        },
    )?;
    let ws = m.workspace.clone();
    let db2 = d.db().clone();
    let since = m.created_ms;
    let dig = move |c: &Criterion| {
        verify::digest_for(
            Some(&db2),
            ws.as_deref().map(std::path::Path::new),
            c,
            since,
        )
    };
    let cur = missions::get(d.db(), &id)?;
    let out = if matches!(cur.state, MissionState::Verifying | MissionState::Running) {
        let (m2, v) = missions::complete(d.db(), &id, &dig)?;
        if m2.state == MissionState::Running && !accept {
            // Rejected: the next round works on it.
            missions::add_task(
                d.db(),
                &id,
                NewTask {
                    title: "Rework after your review".into(),
                    input: verify::next_round_prompt_for(
                        &m2.objective,
                        &v,
                        &missions::criteria(d.db(), &id, None).unwrap_or_default(),
                    ),
                    bot_id: m2.bot_id.clone(),
                    ..Default::default()
                },
            )?;
            d.start(&id);
        }
        json!({ "mission": m2, "verdict": v })
    } else {
        json!({ "mission": cur })
    };
    d.emit(&id);
    Ok(out)
}

#[tauri::command]
pub async fn assist_mission_unblock_task(
    app: AppHandle,
    id: String,
    task_id: String,
    confirmed: Option<bool>,
    note: Option<String>,
) -> Result<Value, String> {
    let d = driver(&app)?;
    let t = missions::unblock_task(d.db(), &task_id, confirmed, note.as_deref().unwrap_or(""))?;
    let m = missions::get(d.db(), &id)?;
    if m.state == MissionState::Blocked {
        missions::resume(d.db(), &id, "task unblocked by the user")?;
        d.start(&id);
    }
    d.emit(&id);
    to_value(t)
}

#[tauri::command]
pub async fn assist_mission_allow_replay(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::allow_replay(d.db(), &id)?;
    if m.state == MissionState::Blocked {
        missions::resume(d.db(), &id, "replay allowed by the user")?;
        d.start(&id);
    }
    d.emit(&id);
    to_value(missions::get(d.db(), &id)?)
}

/// A person raises (or sets) a mission's limits, e.g. more work minutes after
/// the time limit blocked it; fields left out keep their value. An external
/// mission stays inside its execution grant's token ceiling. With `resume`, a
/// blocked mission continues at once.
#[tauri::command]
pub async fn assist_mission_revise_budget(
    app: AppHandle,
    id: String,
    budget: missions::MissionBudget,
    resume: Option<bool>,
) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::get(d.db(), &id)?;
    let next = missions::MissionBudget {
        usd: budget.usd.or(m.budget.usd),
        tokens: budget.tokens.or(m.budget.tokens),
        turns: budget.turns.or(m.budget.turns),
        max_minutes: budget.max_minutes.or(m.budget.max_minutes),
    };
    if omniget_core::core::assist::authority::external(&m.conversation_id) {
        let bot = m.bot_id.as_deref().ok_or("EXECUTOR_REQUIRED")?;
        let ceiling =
            omniget_core::core::assist::authority::resolve(d.db(), &m.conversation_id, bot)?;
        if next.tokens.is_some_and(|t| t > ceiling.max_tokens) {
            return Err("EXECUTION_GRANT_EXCEEDED: tokens above the grant's ceiling".into());
        }
    }
    let m = missions::revise_budget(d.db(), &id, &next)?;
    if resume.unwrap_or(false)
        && matches!(
            m.state,
            MissionState::Blocked | MissionState::Partial | MissionState::Paused
        )
    {
        // The task the limit stopped must run again, or the mission re-blocks.
        missions::reopen_budget_blocked(d.db(), &id)?;
        missions::resume(d.db(), &id, "limits revised by the user")?;
        d.start(&id);
    }
    d.emit(&id);
    to_value(missions::get(d.db(), &id)?)
}

/// Trusted desktop revocation of ONE bot an MCP client derived with
/// `agents_prepare`: it is retired (still listed, read-only) and every
/// later authority check for it fails; the grant and its other bots stay.
#[tauri::command]
pub async fn assist_mcp_derived_bot_revoke(bot: String) -> Result<(), String> {
    let db = omniget_core::core::assist::db::global()?;
    omniget_core::core::assist::external_config::retire_derived(&db, &bot)
}

/// Missions whose re-check runs in the background right now.
static VERIFYING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Marks `id` as being re-checked; false when a re-check already runs (a
/// double click or the webview's IPC re-send of a long request).
pub fn claim_verify(id: &str) -> bool {
    let mut v = VERIFYING.lock().unwrap_or_else(|e| e.into_inner());
    if v.iter().any(|x| x == id) {
        return false;
    }
    v.push(id.to_string());
    true
}

pub fn release_verify(id: &str) {
    VERIFYING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|x| x != id);
}

/// Runs the verifiers now (after the person changed something by hand).
/// Command checks and reviews can take many minutes, longer than the webview
/// keeps an IPC request open (it would re-send the command and verify
/// twice), so this returns at once with `{ mission, started, already_running }`
/// and verifies in the background; the result arrives as `assist://mission`
/// events like any other change.
#[tauri::command]
pub async fn assist_mission_verify(app: AppHandle, id: String) -> Result<Value, String> {
    let d = driver(&app)?;
    let m = missions::get(d.db(), &id)?;
    if m.state.is_final() {
        return Err(format!(
            "{}: mission is {}",
            missions::ERR_MISSION_STATE,
            m.state.as_str()
        ));
    }
    if !claim_verify(&id) {
        return Ok(json!({ "mission": m, "started": false, "already_running": true }));
    }
    let d2 = d.clone();
    let id2 = id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = verify_now(&d2, &id2, &m).await {
            tracing::warn!("[missions] re-check {id2}: {e}");
            let _ = missions::append_event(
                d2.db(),
                &id2,
                missions::NewEvent::new(
                    "verify_failed",
                    json!({ "error": missions::clip_pub(&e, 600) }),
                ),
            );
        }
        release_verify(&id2);
        d2.emit(&id2);
    });
    Ok(json!({ "mission": missions::get(d.db(), &id)?, "started": true, "already_running": false }))
}

async fn verify_now(
    d: &std::sync::Arc<crate::missions::Driver>,
    id: &str,
    m: &missions::Mission,
) -> Result<(), String> {
    let id = id.to_string();
    if matches!(
        m.state,
        MissionState::Blocked | MissionState::Partial | MissionState::Failed | MissionState::Paused
    ) {
        missions::resume(d.db(), &id, "re-check requested")?;
        missions::transition(d.db(), &id, MissionState::Running, "re-check", None)?;
    } else if m.state == MissionState::Queued || m.state == MissionState::Draft {
        if m.state == MissionState::Draft {
            missions::transition(d.db(), &id, MissionState::Queued, "re-check", None)?;
        }
        missions::transition(d.db(), &id, MissionState::Running, "re-check", None)?;
    }
    // Automated checks are work, also for a mission that was waiting for a
    // person (its clock was stopped); `complete` stops it again if needed.
    missions::start_clock(d.db(), &id)?;
    d.verify_all(&id).await?;
    let ws = m.workspace.clone();
    let db2 = d.db().clone();
    let since = m.created_ms;
    let dig = move |c: &Criterion| {
        verify::digest_for(
            Some(&db2),
            ws.as_deref().map(std::path::Path::new),
            c,
            since,
        )
    };
    let (m2, v) = missions::complete(d.db(), &id, &dig)?;
    if m2.state == MissionState::Running {
        missions::block(
            d.db(),
            &id,
            MissionState::Blocked,
            &diag::Diagnosis::criteria_failing(&id, &v, 0),
        )?;
    }
    Ok(())
}

/// Redacted diagnostic bundle; written to `path` when given.
#[tauri::command]
pub async fn assist_mission_diagnostics(
    app: AppHandle,
    id: String,
    path: Option<String>,
) -> Result<Value, String> {
    let d = driver(&app)?;
    let detail = missions::detail(d.db(), &id)?;
    let jobs = crate::jobs::get(&app)?;
    let job_refs: Vec<Value> = detail
        .tasks
        .iter()
        .flat_map(|t| t.job_ids.iter().filter_map(|j| jobs.job(j)).map(|j| json!({ "task": t.id, "job": j.id, "state": j.state, "error": j.error, "usage": j.usage, "request_id": j.request_id })))
        .collect();
    let bundle = diag::bundle(
        &detail,
        json!({ "jobs": job_refs, "app_version": env!("CARGO_PKG_VERSION") }),
    )?;
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        std::fs::write(
            &p,
            serde_json::to_vec_pretty(&bundle).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{}: {e}", missions::ERR_MISSION))?;
        return Ok(json!({ "path": p }));
    }
    Ok(bundle)
}

/// Redacted, paginated log of a task's jobs (tool calls, errors).
#[tauri::command]
pub async fn assist_mission_log(
    app: AppHandle,
    id: String,
    task_id: String,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<diag::LogPage, String> {
    let d = driver(&app)?;
    let t = missions::task(d.db(), &task_id)?;
    if t.mission_id != id {
        return Err(format!(
            "{}: task {task_id} is not in mission {id}",
            missions::ERR_MISSION_INPUT
        ));
    }
    let jobs = crate::jobs::get(&app)?;
    let mut text = String::new();
    for j in &t.job_ids {
        if let Some(job) = jobs.job(j) {
            text.push_str(&format!("── job {} ({})\n{}", job.id, job.state, job.log));
            if let Some(e) = job.error {
                text.push_str(&format!("! {e}\n"));
            }
        }
    }
    Ok(diag::paginate(
        &text,
        offset.unwrap_or(0),
        limit.unwrap_or(200),
    ))
}

#[tauri::command]
pub async fn assist_mission_presets() -> Result<Value, String> {
    to_value(presets::all())
}

#[derive(Debug, Deserialize)]
pub struct ApplyPreset {
    pub preset_id: String,
    /// A roster agent whose model and runtime the new bots start with.
    pub connection_agent_id: String,
    /// Create the group room too (off = the coordinator bot alone).
    #[serde(default)]
    pub with_group: bool,
}

/// Creates the preset's bots (and room), binding skills that are installed.
#[tauri::command]
pub async fn assist_mission_apply_preset(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ApplyPreset,
) -> Result<Value, String> {
    let p = presets::get(&input.preset_id).ok_or_else(|| {
        format!(
            "{}: no preset {}",
            missions::ERR_MISSION_INPUT,
            input.preset_id
        )
    })?;
    let installed: Vec<String> = omniget_core::core::skills::install::list()
        .into_iter()
        .map(|m| m.name)
        .collect();
    let mut created: Vec<(String, String)> = Vec::new();
    let bots: Vec<&presets::PresetBot> = if input.with_group {
        p.bots.iter().collect()
    } else {
        p.bots.iter().take(1).collect()
    };
    for b in bots {
        let skills: Vec<String> = b
            .skill
            .iter()
            .filter(|s| installed.contains(s))
            .cloned()
            .collect();
        let mut instructions = b.instructions.clone();
        if let Some(a) = &b.attribution {
            instructions.push_str(&format!("\n\n({a})"));
        }
        let view = crate::commands::assist::bots::assist_bot_create(
            state.clone(),
            serde_json::from_value(json!({
                "name": b.name,
                "purpose": b.role,
                "instructions": instructions,
                "capabilities": b.capabilities,
                "skills": skills,
                "connection_agent_id": input.connection_agent_id,
            }))
            .map_err(|e| e.to_string())?,
        )
        .await?;
        let id = view["agent"]["id"].as_str().unwrap_or_default().to_string();
        created.push((b.key.clone(), id));
    }
    let mut room = Value::Null;
    if let (true, Some(g)) = (input.with_group, &p.group) {
        let members: Vec<groups::store::Member> = created
            .iter()
            .map(|(k, id)| groups::store::Member {
                bot: id.clone(),
                role: p
                    .bots
                    .iter()
                    .find(|b| &b.key == k)
                    .map(|b| b.role.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let coord = created
            .iter()
            .find(|(k, _)| k == &g.coordinator)
            .map(|(_, id)| id.clone());
        let draft = groups::store::RoomDraft {
            title: g.title.clone(),
            members,
            coordinator: coord.clone(),
            default_bot: coord,
            limits: Some(g.limits.clone()),
            ..Default::default()
        };
        room = to_value(groups::store::create_room(&*db::global()?, &draft)?)?;
    }
    let _ = app;
    Ok(
        json!({ "preset": p.id, "bots": created.iter().map(|(k, id)| json!({ "key": k, "bot_id": id })).collect::<Vec<_>>(), "room": room, "criteria": p.criteria, "workspace": p.workspace }),
    )
}

/// The tasks of a group preset mission: specialists first, the coordinator
/// delivers. With one bot, one task.
#[tauri::command]
pub async fn assist_mission_group_plan(
    app: AppHandle,
    room_id: String,
    objective: String,
) -> Result<Vec<NewTask>, String> {
    let d = driver(&app)?;
    let room = groups::store::get_room(d.db(), &room_id)?;
    let coord = room
        .coordinator
        .clone()
        .or(room.default_bot.clone())
        .ok_or_else(|| {
            format!(
                "{}: the room has no coordinator",
                missions::ERR_MISSION_INPUT
            )
        })?;
    let mut tasks = Vec::new();
    let mut deps = Vec::new();
    for (i, m) in room.members.iter().filter(|m| m.bot != coord).enumerate() {
        let key = format!("s{}", i + 1);
        tasks.push(NewTask { key: key.clone(), title: format!("{} — {}", m.role, objective.chars().take(60).collect::<String>()), input: format!("As {}, do your part for: {objective}\nReturn only your findings; the coordinator delivers.", m.role), bot_id: Some(m.bot.clone()), ..Default::default() });
        deps.push(key);
    }
    tasks.push(NewTask {
        key: "deliver".into(),
        title: "Deliver".into(),
        input: format!("Combine what the specialists found and deliver: {objective}"),
        deps,
        bot_id: Some(coord),
        ..Default::default()
    });
    let _ = now_ms();
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_counts_are_the_verdict_counts_even_with_commas_in_titles() {
        // Live demo 2026-09-25: the row said "2 waiting" for one criterion
        // titled "…3 tópicos, cita a biblioteca e o domingo".
        let v = verify::Verdict {
            passed: false,
            criteria: vec![],
            failing: vec![],
            missing: vec!["a, b (c2)".into()],
            awaiting_human: vec![
                "O resumo tem no máximo 3 tópicos, cita a biblioteca e o domingo (c1)".into(),
            ],
            advisory_failing: vec![],
            partial: vec![],
            completion: "human".into(),
        };
        assert_eq!(
            VerdictCounts::of(&v),
            VerdictCounts {
                passed: false,
                failing: 0,
                missing: 1,
                awaiting_human: 1,
                partial: 0
            }
        );
    }
}
