use super::{
    policy::{self, Principal},
    ToolDef,
};
use omniget_core::core::assist::{authority, db::AssistDb, missions};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

const MAX_REPLY: usize = 64 * 1024;
fn execution_enabled() -> bool {
    cfg!(target_os = "macos")
}
fn default_limit() -> u16 {
    25
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Page {
    #[serde(default)]
    after_id: String,
    #[serde(default = "default_limit")]
    limit: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MissionId {
    mission_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Events {
    mission_id: String,
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_limit")]
    limit: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Control {
    mission_id: String,
    idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct JobDiffArgs {
    job_id: String,
    #[serde(default)]
    format: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApprovalArgs {
    approval_id: String,
    decision: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ArtifactCriterion {
    path: String,
    #[serde(default)]
    contains: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Task {
    title: String,
    input: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Create {
    grant_id: String,
    workspace_id: String,
    executor_id: String,
    objective: String,
    criteria: Vec<ArtifactCriterion>,
    tasks: Vec<Task>,
    budget_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_minutes: Option<u32>,
    idempotency_key: String,
}
const MAX_MISSION_MINUTES: u32 = 240;
fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS".into())
}
fn id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Err("INVALID_ARGUMENTS".into())
    } else {
        Ok(())
    }
}
fn limit(n: u16) -> Result<usize, String> {
    if (1..=100).contains(&n) {
        Ok(n as usize)
    } else {
        Err("INVALID_ARGUMENTS".into())
    }
}
fn text(value: &str, max: usize) -> String {
    crate::core::flight_recorder::redact(value)
        .chars()
        .take(max)
        .collect()
}
fn bounded(value: Value) -> Result<Value, String> {
    if serde_json::to_vec(&value)
        .map_err(|_| "RESPONSE_ENCODING")?
        .len()
        > MAX_REPLY
    {
        Err("RESPONSE_LIMIT_EXCEEDED".into())
    } else {
        Ok(value)
    }
}
fn job_ids(encoded: &str) -> Value {
    if encoded.len() > 16384 {
        return json!({"ids":[],"truncated":true});
    }
    let all: Vec<String> = serde_json::from_str(encoded).unwrap_or_default();
    let truncated = all.len() > 4;
    let ids: Vec<String> = all
        .into_iter()
        .rev()
        .take(4)
        .filter(|v| id(v).is_ok())
        .collect();
    json!({"ids":ids,"truncated":truncated})
}
fn storage<T>(result: Result<T, String>) -> Result<T, String> {
    result.map_err(|_| "ORCHESTRATION_STORAGE_UNAVAILABLE".into())
}

pub fn tools() -> Vec<ToolDef> {
    let id_schema = json!({"type":"string","minLength":1,"maxLength":128});
    let page_schema = json!({"afterId":{"type":"string","maxLength":128},"limit":{"type":"integer","minimum":1,"maximum":100,"default":25}});
    let mission = json!({"missionId":id_schema});
    let control = json!({"missionId":id_schema,"idempotencyKey":{"type":"string","minLength":1,"maxLength":100}});
    let specs=vec![
        ("agents_list","List only executor IDs selected in this client's current local execution grants; no private agent settings.",json!({}),json!([])),
        ("workspaces_list","Execution grants of this connection: the workspace IDs it may run missions in, never host paths. Not the canvas workspaces: workspace_list lists those.",json!({}),json!([])),
        ("missions_list","Bounded stable ID pagination of missions owned by this principal and bound to external authority.",page_schema,json!([])),
        ("missions_get","Read owned mission/task metadata. Historical state is not a fresh artifact verification.",mission.clone(),json!(["missionId"])),
        ("missions_events","Read owned event metadata by sequence; arbitrary payloads are deliberately omitted.",json!({"missionId":id_schema,"afterSeq":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100,"default":25}}),json!(["missionId"])),
        ("missions_cancel","Cancel an owned mission through the existing domain and stop its live jobs; unknown receipts are never replayed.",control.clone(),json!(["missionId","idempotencyKey"])),
        ("missions_pause","Pause an owned mission through the existing domain and stop its live jobs.",control.clone(),json!(["missionId","idempotencyKey"])),
        ("missions_resume","Resume an owned native mission within its existing grant and remaining budget; unresolved effects must be reconciled first.",control,json!(["missionId","idempotencyKey"])),
        ("missions_artifacts","Current digest and latest verification receipt of each artifact of an owned mission; with the transfer scope, a short-lived artifact link bound to that exact revision (same rules as download artifacts). Optional extra paths inside the granted workspace.",json!({"missionId":id_schema,"paths":{"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":1024}}}),json!(["missionId"])),
        ("missions_diagnostics","Redacted diagnosis of an owned mission: state, block reason with suggested actions, criteria verdict, task errors, job/effect references. No private settings, memory or other clients' data.",mission.clone(),json!(["missionId"])),
        ("job_diff","What a job of an owned mission changed in its folder, from the host's own snapshots (no agent tokens): files with status and +/- (binary files marked), totals, and with format=patch the unified diff (redacted, cut at 32 KB). A job whose turns took no snapshot answers available=false with the reason, never an empty diff. No host paths.",json!({"jobId":id_schema,"format":{"type":"string","enum":["files","patch"],"default":"files"}}),json!(["jobId"])),
        ("approvals_list","Pending permission questions of an owned mission's runs. Answering one needs approval_respond, a separate grant the person gives in OmniGet (off by default).",json!({"missionId":id_schema}),json!(["missionId"])),
        ("approval_respond","Answer one pending permission question of an owned mission's run, as the person would in the OmniGet window: accept (this call), accept_session (this tool for the rest of the turn), decline, or cancel (decline and stop the turn). Exactly one answer applies; a second one is ERR_NO_ASK. accept_always stays a desktop decision. Needs the approvals_respond grant; an agent session can never answer.",json!({"approvalId":{"type":"string","minLength":1,"maxLength":64},"decision":{"type":"string","enum":["accept","accept_session","decline","cancel"]}}),json!(["approvalId","decision"])),
        ("agent_run","Start one of the person's agents (roster id) on a task as a background job in OmniGet, and return at once with its jobId and stateSeq. When workspace is in a repository with .omniget/workflow.md, the fields you leave out (agentId, runMode, isolation, base, deadline, budget, model, effort) come from that file and the task goes through its prompt template; workflow=false starts without it. commandId makes it idempotent: the same id again returns the same job; the same id with other fields is ERR_COMMAND_CONFLICT. workspace is an absolute folder (a git repository gets its own worktree when another job is writing there). Permission questions of the job wait for an answer (approval_respond, with that grant, or the OmniGet window). A job never starts another job. Needs the agents_run grant; you only see the jobs you started.",json!({
            "commandId":{"type":"string","minLength":1,"maxLength":128},"agentId":{"type":"string","minLength":1,"maxLength":128},
            "prompt":{"type":"string","minLength":1,"maxLength":65536},"task":{"type":"string","minLength":1,"maxLength":65536,"description":"Same as prompt; give one."},
            "workspace":{"type":"string","maxLength":4096},"title":{"type":"string","maxLength":200},"purpose":{"type":"string","maxLength":1000},
            "model":{"type":"string","maxLength":100},"effort":{"type":"string","enum":["low","medium","high","xhigh","max"]},
            "runMode":{"type":"string","maxLength":32},"isolation":{"type":"string","enum":["shared","worktree","auto"]},"base":{"type":"string","maxLength":256},
            "budgetUsd":{"type":"number","exclusiveMinimum":0,"maximum":1000},"deadlineMin":{"type":"integer","minimum":1,"maximum":1440},
            "resumeOf":{"type":"string","maxLength":64,"description":"A finished job of yours whose conversation this one continues."},
            "autoResume":{"type":"boolean","description":"Start again by itself (with --resume and a check-before-repeating note) if the app closes or the agent goes silent, when it ran in its own worktree or wrote nothing. Omitted = the person's setting."},
            "workflow":{"type":"boolean","description":"false = ignore the repository's .omniget/workflow.md."}
        }),json!(["commandId"])),
        ("job_get","Read one job you started (or a job of a mission you own): state and stateSeq, title, the agent's OUTCOME, result (cut), error, what changed (counts), and pendingApprovals with the approvalId to answer. With afterSeq, changed says whether the state moved since.",json!({"jobId":id_schema,"afterSeq":{"type":"integer","minimum":0}}),json!(["jobId"])),
        ("job_wait","Wait (up to timeoutMs, max 25000) until one of your jobs reaches one of the until states (any state change when empty; \"terminal\" = every end state) with a stateSeq above afterSeq, so an old state never ends the wait. A timeout is the error ERR_WAIT_TIMEOUT: call again. A job that ended elsewhere is ERR_WAIT_UNREACHABLE.",json!({"jobId":id_schema,"until":{"type":"array","maxItems":10,"items":{"type":"string","enum":["queued","waiting_workspace","running","waiting_approval","done","review","done_unreported","failed","cancelled","interrupted","terminal"]}},"afterSeq":{"type":"integer","minimum":0},"timeoutMs":{"type":"integer","minimum":1,"maximum":25000}}),json!(["jobId"])),
        ("job_cancel","Stop one of your jobs (queued or running). commandId makes it idempotent. Needs the agents_run grant.",json!({"jobId":id_schema,"commandId":{"type":"string","minLength":1,"maxLength":128}}),json!(["jobId","commandId"])),
        ("job_report","Everything you need to report one of your jobs back to the person, in one call: verdict (succeeded, needs_review, unreported, checks_failed, failed_by_agent, failed, cancelled, running, waiting_approval), the agent's OUTCOME, result (cut), error, the host's proof (base→head, checks with exit codes, tokens, cost, time), the diff summary (files with +/-), what the repository's .omniget/workflow.md filled in or held back, and what to do next. No host paths. Needs the agents_run grant.",json!({"jobId":id_schema,"afterSeq":{"type":"integer","minimum":0}}),json!(["jobId"])),
        ("jobs_state","Your last 25 jobs (id, state, stateSeq, title) and how many are still active.",json!({}),json!([])),
        ("missions_create","Create a bounded native mission using a local execution grant. File tools create new files or edit existing ones only against the SHA-256 of the revision last read; shell and arbitrary executors are unavailable.",json!({
            "grantId":id_schema,"workspaceId":id_schema,"executorId":id_schema,"objective":{"type":"string","minLength":1,"maxLength":8192},"budgetTokens":{"type":"integer","minimum":1},"maxMinutes":{"type":"integer","minimum":1,"maximum":240,"description":"Active work minutes before the mission blocks for a person (default 15)."},"idempotencyKey":{"type":"string","minLength":1,"maxLength":100},
            "criteria":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"object","additionalProperties":false,"properties":{"path":{"type":"string","minLength":1,"maxLength":1024},"contains":{"type":"array","maxItems":8,"items":{"type":"string","minLength":1,"maxLength":512}}},"required":["path"]}},
            "tasks":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"object","additionalProperties":false,"properties":{"title":{"type":"string","minLength":1,"maxLength":160},"input":{"type":"string","minLength":1,"maxLength":4096}},"required":["title","input"]}}
        }),json!(["grantId","workspaceId","executorId","objective","budgetTokens","idempotencyKey","criteria","tasks"]))
    ];
    let mut all:Vec<ToolDef>=specs.into_iter().map(|(name,description,properties,required)|ToolDef{name,description,input_schema:json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})}).collect();
    all.extend(super::orchestration_config::tools());
    all
}

fn owned(db: &AssistDb, p: &Principal, mission: &str) -> Result<(), String> {
    id(mission)?;
    let exists:bool=storage(db.with(|c|c.query_row("SELECT EXISTS(SELECT 1 FROM missions_missions m JOIN external_executions e ON e.mission=m.id AND e.conversation=m.conversation_id JOIN external_grants g ON g.id=e.grant_id WHERE m.id=?1 AND m.principal=?2 AND g.principal=?2)",params![mission,p.id],|r|r.get(0))))?;
    if exists {
        Ok(())
    } else {
        Err("MISSION_NOT_AUTHORIZED".into())
    }
}
fn owned_job(db: &AssistDb, p: &Principal, job: &str) -> Result<(), String> {
    id(job)?;
    let pattern = format!("%\"{job}\"%");
    let rows: Vec<(String, String)> = storage(db.with(|c| {
        let mut s = c.prepare(
            "SELECT mission_id, job_ids FROM missions_tasks WHERE job_ids LIKE ?1 LIMIT 50",
        )?;
        let rows = s.query_map([&pattern], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }))?;
    let mut missions: Vec<String> = rows
        .into_iter()
        .filter(|(_, ids)| {
            serde_json::from_str::<Vec<String>>(ids).is_ok_and(|ids| ids.iter().any(|i| i == job))
        })
        .map(|(m, _)| m)
        .collect();
    missions.dedup();
    if missions.iter().any(|m| owned(db, p, m).is_ok()) {
        Ok(())
    } else {
        Err("JOB_NOT_AUTHORIZED".into())
    }
}
const JOB_DIFF_FILES: usize = 200;
const JOB_DIFF_PATCH: usize = 32 * 1024;
pub(crate) fn job_diff_reply(
    job: &crate::jobs::Job,
    patch: Option<&str>,
    complete: bool,
    with_patch: bool,
) -> Value {
    let (reason, detail) = match (&job.diff_files, job.diff_unavailable.as_deref()) {
        (_, Some(u)) => match u.split_once(": ") {
            Some((code, detail)) => (Some(code.to_string()), Some(text(detail, 300))),
            None => (Some(u.to_string()), None),
        },
        (Some(_), None) => (None, None),
        (None, None) => (Some("no_snapshot".to_string()), None),
    };
    let files = job.diff_files.clone().unwrap_or_default();
    let listed: Vec<Value> = files
        .iter()
        .take(JOB_DIFF_FILES)
        .map(|f| json!({"path": text(&f.path, 512), "status": f.status, "additions": f.additions, "deletions": f.deletions, "binary": f.binary}))
        .collect();
    let mut out = json!({
        "jobId": job.id, "state": job.state, "available": reason.is_none(), "reason": reason, "detail": detail,
        "files": listed, "fileCount": files.len(), "filesTruncated": files.len() > JOB_DIFF_FILES,
        "additions": job.additions.unwrap_or(0), "deletions": job.deletions.unwrap_or(0),
        "clipped": job.diff_clipped, "complete": complete, "headOid": job.head_oid,
    });
    if with_patch && reason.is_none() {
        let full = patch.unwrap_or("");
        out["patch"] = json!(text(full, JOB_DIFF_PATCH));
        out["patchClipped"] = json!(full.chars().count() > JOB_DIFF_PATCH);
    }
    out
}
fn summary(db: &AssistDb, p: &Principal, mission: &str) -> Result<Value, String> {
    owned(db, p, mission)?;
    let result=storage(db.with(|c|c.query_row("SELECT id,state,objective,criteria_version,revision,created_ms,updated_ms,events_dropped FROM missions_missions WHERE id=?1 AND principal=?2",params![mission,p.id],|r|{
        let objective:String=r.get(2)?;
        Ok(json!({"missionId":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"objective":text(&objective,2000),"criteriaVersion":r.get::<_,i64>(3)?,"revision":r.get::<_,i64>(4)?,"createdMs":r.get::<_,i64>(5)?,"updatedMs":r.get::<_,i64>(6)?,"eventsDropped":r.get::<_,i64>(7)?,"verification":"historical_state_only"}))
    })))?;
    Ok(result)
}
fn proposal(db: &AssistDb, p: &Principal, draft: &Create) -> Result<(), String> {
    for v in [
        &draft.grant_id,
        &draft.workspace_id,
        &draft.executor_id,
        &draft.idempotency_key,
    ] {
        id(v)?;
    }
    if draft.idempotency_key.len() > 100
        || draft.objective.trim().is_empty()
        || draft.objective.len() > 8192
        || draft.budget_tokens == 0
        || draft
            .max_minutes
            .is_some_and(|m| !(1..=MAX_MISSION_MINUTES).contains(&m))
        || !(1..=16).contains(&draft.criteria.len())
        || !(1..=8).contains(&draft.tasks.len())
    {
        return Err("INVALID_ARGUMENTS".into());
    }
    for c in &draft.criteria {
        if c.path.is_empty()
            || c.path.len() > 1024
            || c.path.starts_with('/')
            || c.path.contains('\\')
            || c.path.contains('\0')
            || c.path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
            || c.contains.len() > 8
            || c.contains.iter().any(|s| s.is_empty() || s.len() > 512)
        {
            return Err("INVALID_ARTIFACT_CRITERION".into());
        }
    }
    for t in &draft.tasks {
        if t.title.trim().is_empty()
            || t.title.len() > 160
            || t.input.trim().is_empty()
            || t.input.len() > 4096
        {
            return Err("INVALID_TASK".into());
        }
    }
    let ceiling =
        authority::get(db, &draft.grant_id, &p.id).map_err(|_| "EXECUTION_NOT_GRANTED")?;
    if ceiling.workspace_id != draft.workspace_id
        || (!ceiling.bots.contains(&draft.executor_id)
            && authority::for_bot(db, &ceiling, &draft.executor_id).is_err())
        || draft.budget_tokens > ceiling.max_tokens
    {
        return Err("EXECUTION_GRANT_EXCEEDED".into());
    }
    Ok(())
}

async fn probe_executor(app: &AppHandle, bot: &str) {
    let agent = app.state::<crate::AppState>().llm.agent(bot);
    if let Some(a) = agent {
        if !matches!(
            a.runtime,
            omniget_core::core::llm::agent::RuntimeKind::Native
        ) {
            let _ = omniget_core::core::llm::caps::probe(&a.runtime).await;
        }
    }
}
fn validate_executor(app: &AppHandle, grant: &authority::Ceiling, bot: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let agent = state.llm.agent(bot).ok_or("EXECUTOR_UNAVAILABLE")?;
    authority::external_runtime(&agent)?;
    if grant.bots.iter().any(|b| b == bot) {
        if grant.bot_revisions.get(bot) != Some(&authority::agent_revision(&agent)?) {
            return Err("EXECUTOR_REVISION_CHANGED".into());
        }
    } else {
        let db =
            omniget_core::core::assist::db::global().map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
        authority::for_bot(&db, grant, bot).map_err(|_| "EXECUTOR_REVISION_CHANGED")?;
    }
    let identity = grant
        .workspace_identity
        .as_ref()
        .ok_or("WORKSPACE_IDENTITY_REQUIRED")?;
    omniget_core::core::secure_files::Root::open(
        std::path::Path::new(&grant.workspace),
        Some(identity),
    )
    .map_err(|_| "WORKSPACE_CHANGED")?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MissionPaths {
    mission_id: String,
    #[serde(default)]
    paths: Vec<String>,
}

fn safe_rel(path: &str) -> Result<(), String> {
    omniget_core::core::assist::external_files::validate_path(path).map_err(|e| {
        if e == "PROTECTED_WORKSPACE_PATH" {
            e
        } else {
            "INVALID_ARTIFACT_PATH".into()
        }
    })
}

const ARTIFACTS_CALL_BYTES: u64 = 256 * 1024 * 1024;

fn control_precondition(name: &str, state: missions::MissionState) -> Result<(), String> {
    use missions::MissionState as S;
    let ok = match name {
        "missions_cancel" => state == S::Cancelled || !state.is_final(),
        "missions_pause" => state.can_go(S::Paused),
        "missions_resume" => state.is_active() || state.can_go(S::Queued),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("ERR_MISSION_STATE".into())
    }
}

fn control_refused(p: &Principal, name: &str, key: &str, e: String) -> String {
    if e.starts_with("ERR_MISSION_STATE") {
        let _ = policy::release(p, name, key);
        "ERR_MISSION_STATE".into()
    } else {
        "MISSION_CONTROL_OUTCOME_UNKNOWN".into()
    }
}

fn mission_grant(db: &AssistDb, mission: &str) -> Result<String, String> {
    storage(db.with(|c| {
        c.query_row(
            "SELECT grant_id FROM external_executions WHERE mission=?1 LIMIT 1",
            [mission],
            |r| r.get(0),
        )
    }))
}

fn artifact_unavailable(
    workspace: &std::path::Path,
    path: &str,
    err: &std::io::Error,
    budget: u64,
) -> Value {
    let problem = missions::verify::classify_artifact_error(workspace, path, err, budget);
    json!({"available": false, "reason": problem.code(), "detail": problem.describe(path, budget)})
}

fn mission_artifacts(
    db: &AssistDb,
    p: &Principal,
    request: &MissionPaths,
) -> Result<Value, String> {
    for extra in &request.paths {
        safe_rel(extra)?;
    }
    let m = missions::get(db, &request.mission_id).map_err(|_| "MISSION_NOT_AUTHORIZED")?;
    let grant_id = mission_grant(db, &m.id)?;
    let ceiling = authority::get(db, &grant_id, &p.id).ok();
    let criteria =
        missions::criteria(db, &m.id, None).map_err(|_| "ORCHESTRATION_STORAGE_UNAVAILABLE")?;
    let receipts =
        missions::receipts(db, &m.id).map_err(|_| "ORCHESTRATION_STORAGE_UNAVAILABLE")?;
    let mut paths: Vec<(Option<String>, String)> = criteria
        .iter()
        .filter_map(|c| {
            c.spec
                .get("path")
                .and_then(Value::as_str)
                .map(|path| (Some(c.id.clone()), path.to_string()))
        })
        .collect();
    for extra in &request.paths {
        if !paths.iter().any(|(_, x)| x == extra) {
            paths.push((None, extra.clone()));
        }
    }
    let workspace = m.workspace.clone().ok_or("MISSION_HAS_NO_WORKSPACE")?;
    let root = ceiling
        .as_ref()
        .and_then(|c| c.workspace_identity.as_ref().map(|i| (c, i)))
        .and_then(|(c, i)| {
            omniget_core::core::secure_files::Root::open(
                std::path::Path::new(&c.workspace),
                Some(i),
            )
            .ok()
            .map(|r| (c, i.clone(), r))
        });
    let mut items = Vec::new();
    let mut budget = ARTIFACTS_CALL_BYTES;
    for (criterion, path) in paths {
        let receipt = criterion.as_ref().and_then(|cid| {
            receipts
                .iter()
                .filter(|r| &r.criterion_id == cid && r.invalidated_ms.is_none())
                .max_by_key(|r| r.created_ms)
        });
        let mut item = json!({
            "path": text(&path, 1024),
            "criterionId": criterion,
            "receipt": receipt.map(|r| json!({"receiptId": r.id, "status": r.status, "artifactDigest": r.artifact_digest, "artifactDigestScope": "sha256 of the criterion manifest: sorted JSON list of [path, file sha256]; compare with matchesReceipt, not with the file sha256", "verifier": r.verifier, "createdMs": r.created_ms})),
        });
        if let Err(reason) = safe_rel(&path) {
            item["current"] = json!({"available": false, "reason": reason});
            items.push(item);
            continue;
        }
        if budget == 0 {
            item["current"] = json!({"available": false, "reason": "ARTIFACTS_CALL_BYTE_LIMIT"});
            items.push(item);
            continue;
        }
        match &root {
            None => {
                item["current"] = json!({"available": false, "reason": "EXECUTION_GRANT_REVOKED_OR_WORKSPACE_CHANGED"});
            }
            Some((ceiling, identity, root)) => {
                match root.snapshot(std::path::Path::new(&path), budget) {
                    Err(e) => {
                        item["current"] = artifact_unavailable(
                            std::path::Path::new(&ceiling.workspace),
                            &path,
                            &e,
                            budget,
                        )
                    }
                    Ok(snap) => {
                        budget = budget.saturating_sub(snap.bytes.max(1));
                        let criterion_digest = missions::verify::artifact_digest(
                            Some(std::path::Path::new(&ceiling.workspace)),
                            std::slice::from_ref(&path),
                        );
                        item["current"] = json!({"available": true, "sha256": snap.digest, "bytes": snap.bytes,
                        "matchesReceipt": receipt.map(|r| Some(&r.artifact_digest) == criterion_digest.as_ref())});
                        if p.scopes.iter().any(|s| s == "transfer") {
                            let link = super::artifacts::exec_root(
                                &p.id,
                                &ceiling.id,
                                std::path::Path::new(&workspace),
                                identity,
                            )
                            .and_then(|_| {
                                super::artifacts::register(
                                    p,
                                    &std::path::Path::new(&ceiling.workspace).join(&path),
                                )
                            });
                            item["transfer"] = match link {
                                Ok(a) => {
                                    json!({"artifactId": a.artifact_id, "sha256": a.digest, "bytes": a.bytes, "expiresAt": a.expires_at, "downloadPath": a.download_path})
                                }
                                Err(e) => json!({"error": e}),
                            };
                        } else {
                            item["transfer"] = json!({"error": "TRANSFER_NOT_GRANTED"});
                        }
                    }
                }
            }
        }
        items.push(item);
    }
    Ok(
        json!({"missionId": m.id, "state": m.state, "items": items, "note": "digests are computed now; a receipt proves only the revision it names"}),
    )
}

fn mission_diagnostics(app: &AppHandle, db: &AssistDb, mission: &str) -> Result<Value, String> {
    let d = missions::detail(db, mission).map_err(|_| "ORCHESTRATION_STORAGE_UNAVAILABLE")?;
    let jobs = crate::jobs::get(app).ok();
    let tasks: Vec<Value> = d.tasks.iter().take(24).map(|t| {
        let last_job = t.job_ids.last().and_then(|j| jobs.as_ref().and_then(|x| x.job(j)));
        json!({"taskId": t.id, "title": text(&t.title, 160), "state": t.state, "attempts": t.attempts,
            "error": t.error.as_deref().map(|e| text(e, 600)),
            "lastJob": last_job.map(|j| json!({"jobId": j.id, "state": j.state, "runId": j.request_id, "error": j.error.as_deref().map(|e| text(e, 400))}))})
    }).collect();
    let effects: Vec<Value> = d
        .effects
        .iter()
        .take(24)
        .map(|x| json!({"key": text(&x.key, 200), "kind": x.kind, "state": x.state}))
        .collect();
    let block = d.mission.block.clone().map(missions::diag::redact_json);
    Ok(json!({
        "missionId": d.mission.id, "state": d.mission.state, "revision": d.mission.revision,
        "block": block, "verdict": d.verdict.summary(), "completion": d.verdict.completion,
        "criteria": d.verdict.criteria.iter().map(|c| json!({"id": c.id, "status": c.status, "severity": c.severity, "objective": c.objective})).collect::<Vec<_>>(),
        "tasks": tasks, "effects": effects,
        "spent": {"tokens": d.mission.spent.tokens, "turns": d.mission.spent.turns, "unknownCostCalls": d.mission.spent.unknown_cost_calls, "usdKnown": d.mission.spent.usd_known},
        "evidenceGaps": if d.verdict.passed { json!([]) } else { json!(d.verdict.missing) },
    }))
}

fn approvals(app: &AppHandle, db: &AssistDb, mission: &str) -> Result<Value, String> {
    let tasks = missions::tasks(db, mission).map_err(|_| "ORCHESTRATION_STORAGE_UNAVAILABLE")?;
    let jobs = crate::jobs::get(app).map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
    let runs: std::collections::HashSet<String> = tasks
        .iter()
        .flat_map(|t| t.job_ids.iter())
        .filter_map(|j| jobs.job(j).and_then(|x| x.request_id))
        .collect();
    let reg = omniget_core::core::assist::runs::active().ok_or("ORCHESTRATION_UNAVAILABLE")?;
    let pending: Vec<Value> = reg.permissions(None, true).into_iter().filter(|q| runs.contains(&q.run_id)).take(25).map(|q| json!({
        "requestId": q.id, "runId": q.run_id, "action": text(&q.action, 120), "scope": text(&q.scope, 200),
        "preview": text(&q.preview, 400), "deadlineMs": q.deadline_ms,
    })).collect();
    Ok(
        json!({"missionId": mission, "pending": pending, "remoteApproval": false, "approveIn": "the approval card in the OmniGet window, or approval_respond with its grant"}),
    )
}

fn approval_respond(
    app: &AppHandle,
    db: &AssistDb,
    p: &Principal,
    request: &ApprovalArgs,
) -> Result<Value, String> {
    if p.id.starts_with(crate::mcp::SESSION_PREFIX) {
        return Err("APPROVAL_NOT_AUTHORIZED".into());
    }
    if !matches!(
        request.decision.as_str(),
        "accept" | "accept_session" | "decline" | "cancel"
    ) {
        return Err("INVALID_ARGUMENTS".into());
    }
    let door = crate::approvals::from_app(app).map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
    let job = door
        .registry
        .permission(&request.approval_id)
        .and_then(|q| (door.job_of)(&q.run_id))
        .ok_or("APPROVAL_NOT_AUTHORIZED")?;
    owned_job(db, p, &job).map_err(|_| "APPROVAL_NOT_AUTHORIZED")?;
    let actor = omniget_core::core::assist::runs::Actor::new("mcp", &format!("mcp:{}", p.id));
    let v = crate::approvals::respond(&door, &request.approval_id, &request.decision, &actor)
        .map_err(|r| r.code.to_string())?;
    Ok(json!({
        "approvalId": v["id"], "state": v["state"], "decision": v["decision"], "jobId": v["job_id"],
    }))
}

fn started_job(app: &AppHandle, p: &Principal, name: &str, args: &Value) -> Option<String> {
    let jobs = crate::jobs::get(app).ok()?;
    let job = if name == "job_diff" {
        args["jobId"].as_str()?.to_string()
    } else {
        let door = crate::approvals::from_app(app).ok()?;
        let q = door.registry.permission(args["approvalId"].as_str()?)?;
        (door.job_of)(&q.run_id)?
    };
    let owner = format!("mcp:{}", p.id);
    jobs.job(&job)
        .filter(|j| j.owner.as_deref() == Some(owner.as_str()))
        .map(|j| j.id)
}

pub async fn call(
    app: &AppHandle,
    p: &Principal,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    policy::active(p)?;
    if !policy::allowed(p, name) {
        return Err("TOOL_NOT_AUTHORIZED".into());
    }
    if !tools().iter().any(|t| t.name == name) {
        return Err("UNKNOWN_TOOL".into());
    }
    if serde_json::to_vec(&args)
        .map_err(|_| "INVALID_ARGUMENTS")?
        .len()
        > 64 * 1024
    {
        return Err("ARGUMENT_LIMIT_EXCEEDED".into());
    }
    if crate::job_commands::handles(name) {
        let cmds = crate::job_commands::from_app(app).map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
        return crate::job_commands::mcp_call(&cmds, p, name, args)
            .await
            .and_then(bounded);
    }
    let db = omniget_core::core::assist::db::global().map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
    if matches!(name, "job_diff" | "approval_respond") && started_job(app, p, name, &args).is_some()
    {
        let cmds = crate::job_commands::from_app(app).map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
        return crate::job_commands::mcp_call(&cmds, p, name, args)
            .await
            .and_then(bounded);
    }
    let result = match name {
        "agents_list" | "workspaces_list" => {
            let _: Empty = parse(args)?;
            let grants = authority::list(&db, &p.id).map_err(|_| "EXECUTION_GRANTS_UNAVAILABLE")?;
            if grants.len() > 100 {
                return Err("GRANT_LIST_LIMIT_EXCEEDED".into());
            }
            let rows:Vec<Value>=grants.into_iter().map(|g|if name=="agents_list"{json!({"grantId":g.id,"executorIds":g.bots,"maxTokens":g.max_tokens,"executionEnabled":execution_enabled()})}else{json!({"grantId":g.id,"workspaceId":g.workspace_id})}).collect();
            json!({"items":rows,"executionEnabled":execution_enabled()})
        }
        "missions_list" => {
            let page: Page = parse(args)?;
            let n = limit(page.limit)?;
            if !page.after_id.is_empty() {
                id(&page.after_id)?;
            }
            let mut ids:Vec<String>=storage(db.with(|c|{let mut s=c.prepare("SELECT DISTINCT m.id FROM missions_missions m JOIN external_executions e ON e.mission=m.id AND e.conversation=m.conversation_id JOIN external_grants g ON g.id=e.grant_id WHERE m.principal=?1 AND g.principal=?1 AND m.id>?2 ORDER BY m.id LIMIT ?3")?;let rows=s.query_map(params![p.id,page.after_id,n+1],|r|r.get(0))?;rows.collect()}))?;
            let more = ids.len() > n;
            ids.truncate(n);
            let next = ids.last().cloned().unwrap_or(page.after_id);
            let items: Result<Vec<_>, _> = ids.iter().map(|id| summary(&db, p, id)).collect();
            json!({"items":items?,"nextCursor":next,"hasMore":more})
        }
        "missions_get" => {
            let request: MissionId = parse(args)?;
            let mut value = summary(&db, p, &request.mission_id)?;
            let mut tasks:Vec<Value>=storage(db.with(|c|{let mut s=c.prepare("SELECT id,state,attempts,updated_ms,job_ids FROM missions_tasks WHERE mission_id=?1 ORDER BY position,id LIMIT 21")?;let rows=s.query_map([&request.mission_id],|r|Ok(json!({"taskId":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"attempts":r.get::<_,i64>(2)?,"updatedMs":r.get::<_,i64>(3)?,"jobs":job_ids(&r.get::<_,String>(4)?)})))?;rows.collect()}))?;
            if let Ok(d) = missions::detail(&db, &request.mission_id) {
                value["criteria"] = json!(d.verdict.criteria.iter().map(|c| json!({"id": c.id, "title": text(&c.title, 160), "status": c.status, "severity": c.severity, "receiptId": c.receipt_id})).collect::<Vec<_>>());
                value["verdict"] = json!(d.verdict.summary());
                value["block"] = d
                    .mission
                    .block
                    .clone()
                    .map(missions::diag::redact_json)
                    .unwrap_or(Value::Null);
            }
            value["tasksTruncated"] = json!(tasks.len() > 20);
            tasks.truncate(20);
            value["tasks"] = json!(tasks);
            value
        }
        "missions_events" => {
            let request: Events = parse(args)?;
            let n = limit(request.limit)?;
            if request.after_seq > i64::MAX as u64 {
                return Err("INVALID_ARGUMENTS".into());
            }
            owned(&db, p, &request.mission_id)?;
            let mut events:Vec<Value>=storage(db.with(|c|{let mut s=c.prepare("SELECT event_id,seq,kind,revision,ts_ms FROM missions_events WHERE mission_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;let rows=s.query_map(params![request.mission_id,request.after_seq,n+1],|r|{let kind:String=r.get(2)?;Ok(json!({"eventId":r.get::<_,String>(0)?,"seq":r.get::<_,i64>(1)?,"kind":text(&kind,80),"revision":r.get::<_,i64>(3)?,"timestampMs":r.get::<_,i64>(4)?,"payloadOmitted":true}))})?;rows.collect()}))?;
            let more = events.len() > n;
            events.truncate(n);
            let next = events
                .last()
                .and_then(|e| e["seq"].as_u64())
                .unwrap_or(request.after_seq);
            json!({"events":events,"nextCursor":next,"hasMore":more})
        }
        "missions_artifacts" => {
            let request: MissionPaths = parse(args)?;
            owned(&db, p, &request.mission_id)?;
            let permit = super::artifacts::admit()?;
            let (db, p) = (db.clone(), p.clone());
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                mission_artifacts(&db, &p, &request)
            })
            .await
            .map_err(|_| "ORCHESTRATION_UNAVAILABLE")??
        }
        "missions_diagnostics" => {
            let request: MissionId = parse(args)?;
            owned(&db, p, &request.mission_id)?;
            mission_diagnostics(app, &db, &request.mission_id)?
        }
        "job_diff" => {
            let request: JobDiffArgs = parse(args)?;
            let with_patch = match request.format.as_deref().unwrap_or("files") {
                "files" => false,
                "patch" => true,
                _ => return Err("INVALID_ARGUMENTS".into()),
            };
            owned_job(&db, p, &request.job_id)?;
            let jobs = crate::jobs::get(app).map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
            let (job, patch, complete) = jobs
                .diff(&request.job_id)
                .await
                .map_err(|_| "JOB_NOT_AUTHORIZED")?;
            job_diff_reply(&job, patch.as_deref(), complete, with_patch)
        }
        "approvals_list" => {
            let request: MissionId = parse(args)?;
            owned(&db, p, &request.mission_id)?;
            let mut v = approvals(app, &db, &request.mission_id)?;
            v["remoteApproval"] = json!(policy::allowed(p, "approval_respond"));
            v
        }
        "approval_respond" => {
            let request: ApprovalArgs = parse(args)?;
            approval_respond(app, &db, p, &request)?
        }
        "missions_create" => {
            let request: Create = parse(args)?;
            proposal(&db, p, &request)?;
            if !execution_enabled() {
                return Err("EXECUTION_ISOLATION_REQUIRED".into());
            }
            let ceiling = authority::get(&db, &request.grant_id, &p.id)?;
            probe_executor(app, &request.executor_id).await;
            validate_executor(app, &ceiling, &request.executor_id)?;
            let new = missions::NewMission {
                objective: request.objective,
                bot_id: Some(request.executor_id),
                workspace: Some(ceiling.workspace.clone()),
                start: true,
                budget: missions::MissionBudget {
                    tokens: Some(request.budget_tokens),
                    turns: Some(32),
                    max_minutes: Some(request.max_minutes.unwrap_or(15)),
                    ..Default::default()
                },
                criteria: request
                    .criteria
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| missions::Criterion {
                        id: format!("artifact-{i}"),
                        version: 1,
                        kind: missions::CriterionKind::Artifact,
                        severity: missions::Severity::Required,
                        title: format!("Artifact {}", i + 1),
                        spec: json!({"path":c.path,"contains":c.contains,"must_exist":true}),
                        origin: missions::Origin::Proposed,
                        acceptance: missions::Acceptance::Auto,
                    })
                    .collect(),
                tasks: request
                    .tasks
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| missions::NewTask {
                        key: format!("task-{i}"),
                        title: t.title,
                        input: t.input,
                        deps: if i == 0 {
                            vec![]
                        } else {
                            vec![format!("task-{}", i - 1)]
                        },
                        max_attempts: Some(1),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            policy::active(p)?;
            let driver = crate::missions::driver(app).map_err(|_| "MISSION_DRIVER_UNAVAILABLE")?;
            let detail = missions::create_external(
                &db,
                new,
                &p.id,
                &request.grant_id,
                &request.idempotency_key,
            )?;
            driver.start(&detail.mission.id);
            summary(&db, p, &detail.mission.id)?
        }
        "missions_resume" => {
            let request: Control = parse(args.clone())?;
            id(&request.idempotency_key)?;
            if request.idempotency_key.len() > 100 {
                return Err("INVALID_ARGUMENTS".into());
            }
            owned(&db, p, &request.mission_id)?;
            if !execution_enabled() {
                return Err("EXECUTION_ISOLATION_REQUIRED".into());
            }
            if let Some(receipt) = policy::receipt(p, name, &request.idempotency_key, &args)? {
                return bounded(receipt);
            }
            let mission = missions::get(&db, &request.mission_id)?;
            control_precondition(name, mission.state)?;
            let bot = mission.bot_id.as_deref().ok_or("EXECUTOR_REQUIRED")?;
            let ceiling = authority::resolve(&db, &mission.conversation_id, bot)?;
            probe_executor(app, bot).await;
            validate_executor(app, &ceiling, bot)?;
            let unresolved:bool=storage(db.with(|c|c.query_row("SELECT EXISTS(SELECT 1 FROM missions_effects WHERE mission_id=?1 AND state IN ('pending','running','unknown'))",[&request.mission_id],|r|r.get(0))))?;
            if unresolved {
                return Err("MISSION_EFFECT_RECONCILIATION_REQUIRED".into());
            }
            if let Some(receipt) = policy::reserve(p, name, &request.idempotency_key, &args)? {
                return bounded(receipt);
            }
            policy::active(p)?;
            let driver = crate::missions::driver(app).map_err(|_| "MISSION_DRIVER_UNAVAILABLE")?;
            missions::resume(&db, &request.mission_id, "resumed by external owner")
                .map_err(|e| control_refused(p, name, &request.idempotency_key, e))?;
            driver.start(&request.mission_id);
            let value = summary(&db, p, &request.mission_id)?;
            policy::finish(p, name, &request.idempotency_key, &value)?;
            value
        }
        "missions_cancel" | "missions_pause" => {
            let request: Control = parse(args.clone())?;
            id(&request.idempotency_key)?;
            if request.idempotency_key.len() > 100 {
                return Err("INVALID_ARGUMENTS".into());
            }
            owned(&db, p, &request.mission_id)?;
            if let Some(receipt) = policy::receipt(p, name, &request.idempotency_key, &args)? {
                return bounded(receipt);
            }
            let mission =
                missions::get(&db, &request.mission_id).map_err(|_| "MISSION_NOT_AUTHORIZED")?;
            control_precondition(name, mission.state)?;
            if let Some(receipt) = policy::reserve(p, name, &request.idempotency_key, &args)? {
                return bounded(receipt);
            }
            policy::active(p)?;
            let driver = crate::missions::driver(app).map_err(|_| "MISSION_DRIVER_UNAVAILABLE")?;
            let outcome = if name == "missions_cancel" {
                missions::cancel(&db, &request.mission_id, "cancelled by external owner")
            } else {
                missions::pause(&db, &request.mission_id)
            }
            .map_err(|e| control_refused(p, name, &request.idempotency_key, e))?;
            driver.stop_jobs(&outcome.live_jobs);
            driver.emit(&request.mission_id);
            let value = summary(&db, p, &request.mission_id)?;
            policy::finish(p, name, &request.idempotency_key, &value)?;
            value
        }
        n if super::orchestration_config::handles(n) => {
            super::orchestration_config::call(&db, p, n, args)?
        }
        _ => return Err("UNKNOWN_TOOL".into()),
    };
    policy::active(p)?;
    bounded(result)
}
