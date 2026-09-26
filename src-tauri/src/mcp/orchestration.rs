//! External orchestration projection. Authority is selected from local grants;
//! callers cannot provide raw mission origins, roots, rooms or verifier code.
//! External native execution uses pinned file capabilities and bounded grants.
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
    /// Active work minutes (paused/blocked time does not count); default 15.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_minutes: Option<u32>,
    idempotency_key: String,
}
/// Upper bound of an external mission's active work time.
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
        ("workspaces_list","List granted workspace IDs, never host paths.",json!({}),json!([])),
        ("missions_list","Bounded stable ID pagination of missions owned by this principal and bound to external authority.",page_schema,json!([])),
        ("missions_get","Read owned mission/task metadata. Historical state is not a fresh artifact verification.",mission.clone(),json!(["missionId"])),
        ("missions_events","Read owned event metadata by sequence; arbitrary payloads are deliberately omitted.",json!({"missionId":id_schema,"afterSeq":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100,"default":25}}),json!(["missionId"])),
        ("missions_cancel","Cancel an owned mission through the existing domain and stop its live jobs; unknown receipts are never replayed.",control.clone(),json!(["missionId","idempotencyKey"])),
        ("missions_pause","Pause an owned mission through the existing domain and stop its live jobs.",control.clone(),json!(["missionId","idempotencyKey"])),
        ("missions_resume","Resume an owned native mission within its existing grant and remaining budget; unresolved effects must be reconciled first.",control,json!(["missionId","idempotencyKey"])),
        ("missions_artifacts","Current digest and latest verification receipt of each artifact of an owned mission; with the transfer scope, a short-lived artifact link bound to that exact revision (same rules as download artifacts). Optional extra paths inside the granted workspace.",json!({"missionId":id_schema,"paths":{"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":1024}}}),json!(["missionId"])),
        ("missions_diagnostics","Redacted diagnosis of an owned mission: state, block reason with suggested actions, criteria verdict, task errors, job/effect references. No private settings, memory or other clients' data.",mission.clone(),json!(["missionId"])),
        ("approvals_list","Pending permission questions of an owned mission's runs. Read-only: approvals are given by a person in the OmniGet window, never through this API.",json!({"missionId":id_schema}),json!(["missionId"])),
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

/// Both a principal match and a durable external execution binding are needed.
/// Historical reads/cancellation remain available after grant revocation, while
/// creation/resume must revalidate the live grant and execution boundary.
fn owned(db: &AssistDb, p: &Principal, mission: &str) -> Result<(), String> {
    id(mission)?;
    let exists:bool=storage(db.with(|c|c.query_row("SELECT EXISTS(SELECT 1 FROM missions_missions m JOIN external_executions e ON e.mission=m.id AND e.conversation=m.conversation_id JOIN external_grants g ON g.id=e.grant_id WHERE m.id=?1 AND m.principal=?2 AND g.principal=?2)",params![mission,p.id],|r|r.get(0))))?;
    if exists {
        Ok(())
    } else {
        Err("MISSION_NOT_AUTHORIZED".into())
    }
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

/// A CLI executor qualifies only from probed flags; probe before the sync check
/// (the cache is empty after a restart).
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
    // A derived bot's pins are checked by `for_bot` (source and derivative
    // revisions); a directly granted executor by the grant's own pin.
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

/// Same rule as the executor's file tools and external criteria (M1): shape
/// plus the protected names (`.git`, `.ssh`, `.env*`, `.omniget-*`, ...).
fn safe_rel(path: &str) -> Result<(), String> {
    omniget_core::core::assist::external_files::validate_path(path).map_err(|e| {
        if e == "PROTECTED_WORKSPACE_PATH" {
            e
        } else {
            "INVALID_ARTIFACT_PATH".into()
        }
    })
}

/// Per-call ceiling of bytes snapshotted by `missions_artifacts` (M3).
const ARTIFACTS_CALL_BYTES: u64 = 256 * 1024 * 1024;

/// Deterministic precondition of a mission control, checked before the
/// idempotency receipt is reserved, so a predictable refusal is a known
/// `ERR_MISSION_STATE` and never poisons the key (F8).
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

/// After the receipt: a domain state refusal rolled its transaction back, so
/// the outcome is known; the pending receipt is released instead of kept.
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

/// Why a path could not be snapshotted, for the controller: `NOT_FOUND`
/// (write it at exactly that path), `IS_DIRECTORY` or `UNSAFE_OR_OVER_LIMIT`.
fn artifact_unavailable(
    workspace: &std::path::Path,
    path: &str,
    err: &std::io::Error,
    budget: u64,
) -> Value {
    let problem = missions::verify::classify_artifact_error(workspace, path, err, budget);
    json!({"available": false, "reason": problem.code(), "detail": problem.describe(path, budget)})
}

/// Artifacts of an owned mission: the criteria paths plus requested extra
/// paths, each with its digest as it is NOW (descriptor-pinned snapshot) and
/// the newest receipt for that criterion. A transfer link is issued only with
/// the transfer scope and only while the execution grant is live.
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
        // Criteria paths get the same protected-name rule as extra paths
        // (defense if an old or local criterion escaped validation).
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
        json!({"missionId": mission, "pending": pending, "remoteApproval": false, "approveIn": "OmniGet window → LLM → Activity"}),
    )
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
    let db = omniget_core::core::assist::db::global().map_err(|_| "ORCHESTRATION_UNAVAILABLE")?;
    let result = match name {
        "agents_list" | "workspaces_list" => {
            let _: Empty = parse(args)?;
            let grants = authority::list(&db, &p.id).map_err(|_| "EXECUTION_GRANTS_UNAVAILABLE")?;
            // Grants are locally bounded individually. Refuse an unbounded
            // result rather than silently claiming discovery is complete.
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
            // Snapshots, digests and link copies are blocking file work: off
            // the async runtime and behind the shared artifact semaphore (M3).
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
        "approvals_list" => {
            let request: MissionId = parse(args)?;
            owned(&db, p, &request.mission_id)?;
            approvals(app, &db, &request.mission_id)?
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
                // A single executor conversation has one turn at a time.
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
            // Boot reconciliation also drives queued missions if this process
            // exits after the atomic creation and before this notification.
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
            // A finished replay returns its receipt; a pending one stays unknown.
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
            // Existing driver/domain owns state and live-job cancellation.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_dto_rejects_code_origin_paths_and_reviewer_fields() {
        let raw = json!({"grantId":"g","workspaceId":"w","executorId":"b","objective":"work","budgetTokens":100,"idempotencyKey":"key","criteria":[{"path":"README.md","command":"rm -rf /","origin":"user"}],"tasks":[{"title":"work","input":"work"}]});
        assert!(parse::<Create>(raw).is_err());
        assert!(parse::<Control>(
            json!({"missionId":"m","idempotencyKey":"key","principal":"other"})
        )
        .is_err());
        assert!(parse::<Events>(json!({"missionId":"m","afterSeq":-1})).is_err());
        assert!(limit(0).is_err());
        assert!(limit(101).is_err());
    }
    #[test]
    fn reads_require_matching_principal_and_external_binding() {
        use omniget_core::core::assist::db::Migration;
        let dir =
            std::env::temp_dir().join(format!("omniget-orch-{}", uuid::Uuid::new_v4().simple()));
        let db = AssistDb::open_with(&dir.join("assist.db"), &[Migration { module: "orchestration_fixture", version: 1, sql: "
            CREATE TABLE missions_missions(id TEXT,principal TEXT,conversation_id TEXT);
            CREATE TABLE external_executions(mission TEXT,conversation TEXT,grant_id TEXT);
            CREATE TABLE external_grants(id TEXT,principal TEXT);
            INSERT INTO missions_missions VALUES('mission-a','alice','external-mcp-a');
            INSERT INTO missions_missions VALUES('mission-b','bob','external-mcp-b');
            INSERT INTO missions_missions VALUES('local-unbound','alice','local');
            INSERT INTO external_grants VALUES('grant-a','alice'),('grant-b','bob');
            INSERT INTO external_executions VALUES('mission-a','external-mcp-a','grant-a'),('mission-b','external-mcp-b','grant-b');
        " }]).unwrap();
        let alice = Principal {
            id: "alice".into(),
            name: "A".into(),
            scopes: vec![],
        };
        let bob = Principal {
            id: "bob".into(),
            name: "B".into(),
            scopes: vec![],
        };
        assert!(owned(&db, &alice, "mission-a").is_ok());
        assert!(owned(&db, &bob, "mission-a").is_err());
        assert!(owned(&db, &alice, "local-unbound").is_err());
        db.with(|c| {
            c.execute(
                "UPDATE external_grants SET principal='bob' WHERE id='grant-a'",
                [],
            )
        })
        .unwrap();
        assert!(owned(&db, &alice, "mission-a").is_err());
    }
    #[test]
    fn catalog_closed_and_creation_describes_limited_executor() {
        for tool in tools() {
            assert_eq!(tool.input_schema["additionalProperties"], false);
        }
        assert!(tools()
            .iter()
            .find(|t| t.name == "missions_create")
            .unwrap()
            .description
            .contains("SHA-256 of the revision last read"));
        assert!(bounded(json!({"large":"x".repeat(MAX_REPLY)})).is_err());
    }
    /// Regression of audit M1 (`f1_missions_artifacts_path_check_skips_
    /// protected_names`): the controller-facing path check now refuses the
    /// protected names before any snapshot, digest or transfer link.
    #[test]
    fn missions_artifacts_refuses_protected_workspace_names() {
        for p in [
            ".env",
            ".env.local",
            ".git/config",
            ".ssh/id_rsa",
            "a/.aws/credentials",
            ".omniget-private-x/f",
            "id_rsa",
        ] {
            assert_eq!(safe_rel(p).unwrap_err(), "PROTECTED_WORKSPACE_PATH", "{p}");
        }
        for p in ["", "/abs", "../x", "a//b", "a\\b", "./a"] {
            assert_eq!(safe_rel(p).unwrap_err(), "INVALID_ARTIFACT_PATH", "{p:?}");
        }
        assert!(safe_rel("out/report.md").is_ok());
        // Refused before the mission is even looked up (no snapshot/register).
        let db = AssistDb::open_in_memory().unwrap();
        let p = Principal {
            id: "a".into(),
            name: "A".into(),
            scopes: vec!["transfer".into()],
        };
        let req = MissionPaths {
            mission_id: "m".into(),
            paths: vec!["README.md".into(), ".env".into()],
        };
        assert_eq!(
            mission_artifacts(&db, &p, &req).unwrap_err(),
            "PROTECTED_WORKSPACE_PATH"
        );
    }
    #[test]
    fn missions_artifacts_names_why_a_path_is_unavailable() {
        let ws = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "omniget-orch-art-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(ws.join("out")).unwrap();
        std::fs::write(ws.join("big.bin"), vec![0u8; 64]).unwrap();
        let root = omniget_core::core::secure_files::Root::open(&ws, None).unwrap();
        let reason = |path: &str, budget: u64| {
            let err = root
                .snapshot(std::path::Path::new(path), budget)
                .err()
                .expect("snapshot must fail");
            artifact_unavailable(&ws, path, &err, budget)
        };
        let missing = reason("hello.md", 1024);
        assert_eq!(missing["reason"], "NOT_FOUND");
        assert!(missing["detail"]
            .as_str()
            .unwrap()
            .contains("not found: hello.md"));
        assert_eq!(reason("out", 1024)["reason"], "IS_DIRECTORY");
        assert_eq!(reason("big.bin", 8)["reason"], "UNSAFE_OR_OVER_LIMIT");
        assert_eq!(reason("big.bin", 8)["available"], false);
        std::fs::remove_dir_all(ws).unwrap();
    }
    /// Regression of state audit F8: a predictable control refusal is decided
    /// before `policy::reserve`, so it never leaves an unknown receipt.
    #[test]
    fn control_preconditions_are_checked_before_the_receipt() {
        use missions::MissionState as S;
        assert_eq!(
            control_precondition("missions_pause", S::Succeeded).unwrap_err(),
            "ERR_MISSION_STATE"
        );
        assert_eq!(
            control_precondition("missions_cancel", S::Succeeded).unwrap_err(),
            "ERR_MISSION_STATE"
        );
        assert!(control_precondition("missions_cancel", S::Cancelled).is_ok());
        assert!(control_precondition("missions_cancel", S::Running).is_ok());
        assert!(control_precondition("missions_pause", S::Running).is_ok());
        assert_eq!(
            control_precondition("missions_resume", S::Cancelled).unwrap_err(),
            "ERR_MISSION_STATE"
        );
        assert!(control_precondition("missions_resume", S::Paused).is_ok());
        assert!(control_precondition("missions_resume", S::Running).is_ok());
        assert!(control_precondition("missions_create", S::Running).is_err());
    }
}
