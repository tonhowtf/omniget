//! C02 projection: an external controller prepares limited derived bots,
//! rooms of its own derived bots and delegated tasks. All authority checks
//! live in `assist::external_config` and run on every call; this module only
//! parses closed DTOs, maps errors to stable codes and bounds replies.
//! Preparing configuration does not run anything: tasks stay `pending` until
//! the mission driver runs them under the external authority.
use super::{policy::Principal, ToolDef};
use omniget_core::core::assist::db::AssistDb;
use omniget_core::core::assist::external_config::{self as ext, BotRoster};
use serde::Deserialize;
use serde_json::{json, Value};

pub const NAMES: &[&str] = &[
    "agents_prepare",
    "agents_derived_list",
    "groups_prepare",
    "groups_get",
    "group_tasks_create",
];

pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RoomId {
    room_id: String,
}

pub fn tools() -> Vec<ToolDef> {
    let id = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9._-]+$"});
    let key = json!({"type":"string","minLength":1,"maxLength":100,"pattern":"^[A-Za-z0-9._-]+$"});
    let specs = vec![
        ("agents_prepare","Prepare (or replay by idempotencyKey) a limited derived bot from an executor in one of this client's local grants. Native only; model and connection come from that executor; tools must be a subset of the grant; no memory, skills or local capabilities. It shares the parent grant's budget and stops working if the grant is revoked or the executor changes.",json!({
            "grantId":id,"sourceExecutorId":id,
            "name":{"type":"string","minLength":1,"maxLength":80},
            "purpose":{"type":"string","maxLength":2000},
            "instructions":{"type":"string","minLength":1,"maxLength":8192},
            "tools":{"type":"array","maxItems":32,"uniqueItems":true,"items":id},
            "idempotencyKey":key
        }),json!(["grantId","sourceExecutorId","name","instructions","idempotencyKey"])),
        ("agents_derived_list","List this client's derived bots with their current validity (ready or invalid with a reason). Never returns private agent settings.",json!({}),json!([])),
        ("groups_prepare","Prepare (or replay) a room whose members are only this client's own derived bots under one grant. Limits are conservative (depth 1, one run at a time, at most 4 delegations, 15 min per task) and never exceed the grant's token ceiling.",json!({
            "grantId":id,"title":{"type":"string","minLength":1,"maxLength":120},
            "memberBotIds":{"type":"array","minItems":1,"maxItems":6,"uniqueItems":true,"items":id},
            "coordinatorId":id,
            "limits":{"type":"object","additionalProperties":false,"properties":{
                "maxDelegations":{"type":"integer","minimum":1,"maximum":4},
                "maxTaskSeconds":{"type":"integer","minimum":5,"maximum":900},
                "maxTokens":{"type":"integer","minimum":1}
            }},
            "idempotencyKey":key
        }),json!(["grantId","title","memberBotIds","coordinatorId","idempotencyKey"])),
        ("groups_get","Read an owned room: members, limits, grant status and this client's delegated tasks with bounded, redacted results.",json!({"roomId":id}),json!(["roomId"])),
        ("group_tasks_create","Delegate one task (idempotent by key) from the room coordinator to another owned member. The task is recorded as pending; it runs only through the mission driver under this client's grant.",json!({
            "roomId":id,"to":id,
            "question":{"type":"string","minLength":1,"maxLength":4096},
            "context":{"type":"string","maxLength":8192},
            "deliverable":{"type":"string","maxLength":2000},
            "limitSeconds":{"type":"integer","minimum":1,"maximum":900},
            "idempotencyKey":key
        }),json!(["roomId","to","question","idempotencyKey"])),
    ];
    specs
        .into_iter()
        .map(|(name, description, properties, required)| ToolDef {
            name,
            description,
            input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        })
        .collect()
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|_| "INVALID_ARGUMENTS".into())
}

/// Domain errors are stable codes already; group-layer messages carry free
/// text (bot names, counts), so they are collapsed to codes here.
fn code(e: String) -> String {
    if e.starts_with("ERR_GROUP_LIMIT") {
        "EXTERNAL_ROOM_LIMIT_REACHED".into()
    } else if e.starts_with("ERR_GROUP_BUSY") {
        "EXTERNAL_BOT_BUSY".into()
    } else if e.starts_with("ERR_GROUP") {
        "EXTERNAL_ROOM_INVALID".into()
    } else if e.starts_with("ERR_ASSIST_DB") || e.contains(' ') {
        "ORCHESTRATION_STORAGE_UNAVAILABLE".into()
    } else {
        e
    }
}

fn clip(v: Option<String>, max: usize) -> Option<String> {
    v.map(|s| {
        crate::core::flight_recorder::redact(&s)
            .chars()
            .take(max)
            .collect()
    })
}

pub fn call(db: &AssistDb, p: &Principal, name: &str, args: Value) -> Result<Value, String> {
    call_with(db, ext::installed_roster().as_deref(), p, name, args)
}

pub(crate) fn call_with(
    db: &AssistDb,
    roster: Option<&dyn BotRoster>,
    p: &Principal,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let roster = || roster.ok_or_else(|| "EXTERNAL_ROSTER_UNAVAILABLE".to_string());
    let value = match name {
        "agents_prepare" => {
            let req: ext::BotRequest = parse(args)?;
            json!(ext::prepare_bot_with(db, roster()?, &p.id, &req).map_err(code)?)
        }
        "agents_derived_list" => {
            let _: Empty = parse(args)?;
            json!({"items": ext::list_bots_with(db, roster().ok(), &p.id).map_err(code)?})
        }
        "groups_prepare" => {
            let req: ext::RoomRequest = parse(args)?;
            json!(ext::prepare_room_with(db, roster()?, &p.id, &req).map_err(code)?)
        }
        "groups_get" => {
            let req: RoomId = parse(args)?;
            let room = ext::room_view(db, &p.id, &req.room_id).map_err(code)?;
            let tasks: Vec<Value> = ext::room_tasks(db, &p.id, &req.room_id)
                .map_err(code)?
                .into_iter()
                .map(|t| {
                    json!({"taskId":t.task_id,"recipient":t.recipient,"state":t.state,"limitMs":t.limit_ms,
                           "createdMs":t.created_ms,"finishedMs":t.finished_ms,
                           "result":clip(t.result,4000),"error":clip(t.error,500)})
                })
                .collect();
            json!({"room":room,"tasks":tasks})
        }
        "group_tasks_create" => {
            let req: ext::TaskRequest = parse(args)?;
            let t = ext::create_task_with(db, roster()?, &p.id, &req).map_err(code)?;
            json!({"taskId":t.task_id,"roomId":t.room_id,"recipient":t.recipient,"state":t.state,"limitMs":t.limit_ms,
                   "execution":"pending_driver"})
        }
        _ => return Err("UNKNOWN_TOOL".into()),
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniget_core::core::assist::authority;
    use omniget_core::core::llm::roster_store::RosterStore;

    fn principal(id: &str) -> Principal {
        Principal {
            id: id.into(),
            name: id.into(),
            scopes: vec![],
        }
    }

    #[test]
    fn catalog_is_closed_and_policy_mapped() {
        for t in tools() {
            assert_eq!(t.input_schema["additionalProperties"], false, "{}", t.name);
            assert!(handles(t.name));
            assert!(super::super::policy::scope(t.name).is_some(), "{}", t.name);
        }
        assert_eq!(tools().len(), NAMES.len());
        assert!(super::super::orchestration::tools()
            .iter()
            .any(|t| t.name == "agents_prepare"));
    }

    #[test]
    fn mcp_flow_prepares_limited_config_and_isolates_principals() {
        // The desktop crate has no tempfile dev-dependency.
        let dir =
            std::env::temp_dir().join(format!("omniget-c02-mcp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let db = AssistDb::open_in_memory().unwrap();
        let roster = RosterStore::at(dir.join("roster.json"));
        let mut exec = roster.get("builder").unwrap();
        exec.id = "exec-a".into();
        roster.create(exec.clone()).unwrap();
        let grant = authority::grant(
            &db,
            authority::Ceiling {
                id: String::new(),
                principal: "A".into(),
                workspace_id: "ws".into(),
                workspace: dir.to_string_lossy().into_owned(),
                workspace_identity: None,
                bots: vec!["exec-a".into()],
                bot_revisions: std::collections::BTreeMap::from([(
                    "exec-a".into(),
                    authority::agent_revision(&exec).unwrap(),
                )]),
                tools: vec!["fs_read".into(), "fs_write".into()],
                max_tokens: 10_000,
                max_usd: None,
            },
        )
        .unwrap();
        let (a, b) = (principal("A"), principal("B"));
        let r: Option<&dyn BotRoster> = Some(&roster);
        let bot = |key: &str, tools: Value| json!({"grantId":grant,"sourceExecutorId":"exec-a","name":key,"instructions":"work","tools":tools,"idempotencyKey":key});
        let lead = call_with(
            &db,
            r,
            &a,
            "agents_prepare",
            bot("lead", json!(["fs_read"])),
        )
        .unwrap();
        let worker = call_with(
            &db,
            r,
            &a,
            "agents_prepare",
            bot("worker", json!(["fs_read", "fs_write"])),
        )
        .unwrap();
        assert_eq!(lead["state"], "ready");
        assert!(lead.get("agent").is_none() && lead.get("systemPrompt").is_none());
        // Escalation through the MCP surface.
        assert_eq!(
            call_with(
                &db,
                r,
                &a,
                "agents_prepare",
                bot("esc", json!(["shell_exec"]))
            )
            .unwrap_err(),
            "EXTERNAL_TOOL_NOT_GRANTED"
        );
        let mut extra = bot("esc2", json!(["fs_read"]));
        extra["capabilities"] = json!(["memory"]);
        assert_eq!(
            call_with(&db, r, &a, "agents_prepare", extra).unwrap_err(),
            "INVALID_ARGUMENTS"
        );
        assert_eq!(
            call_with(
                &db,
                r,
                &b,
                "agents_prepare",
                bot("lead", json!(["fs_read"]))
            )
            .unwrap_err(),
            "EXECUTION_NOT_GRANTED"
        );
        assert_eq!(
            call_with(&db, None, &a, "agents_prepare", bot("x", json!([]))).unwrap_err(),
            "EXTERNAL_ROSTER_UNAVAILABLE"
        );
        let listed = call_with(&db, r, &a, "agents_derived_list", json!({})).unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 2);
        assert!(
            call_with(&db, r, &b, "agents_derived_list", json!({})).unwrap()["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let room = call_with(&db, r, &a, "groups_prepare", json!({"grantId":grant,"title":"t","memberBotIds":[lead["botId"],worker["botId"]],"coordinatorId":lead["botId"],"idempotencyKey":"room"})).unwrap();
        let room_id = room["roomId"].as_str().unwrap().to_string();
        assert_eq!(room["limits"]["max_tokens_per_round"], 10_000);
        let task_args =
            json!({"roomId":room_id,"to":worker["botId"],"question":"q","idempotencyKey":"t1"});
        let t = call_with(&db, r, &a, "group_tasks_create", task_args.clone()).unwrap();
        assert_eq!(t["state"], "pending");
        assert_eq!(
            call_with(&db, r, &a, "group_tasks_create", task_args.clone()).unwrap()["taskId"],
            t["taskId"]
        );
        assert_eq!(
            call_with(&db, r, &b, "group_tasks_create", task_args).unwrap_err(),
            "ROOM_NOT_AUTHORIZED"
        );
        assert_eq!(
            call_with(&db, r, &b, "groups_get", json!({"roomId":room_id})).unwrap_err(),
            "ROOM_NOT_AUTHORIZED"
        );
        let got = call_with(&db, r, &a, "groups_get", json!({"roomId":room_id})).unwrap();
        assert_eq!(got["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(got["room"]["grantActive"], true);
        authority::revoke_principal(&db, "A").unwrap();
        assert_eq!(
            call_with(
                &db,
                r,
                &a,
                "group_tasks_create",
                json!({"roomId":room_id,"to":worker["botId"],"question":"q","idempotencyKey":"t2"})
            )
            .unwrap_err(),
            "EXECUTION_NOT_GRANTED"
        );
        let listed = call_with(&db, r, &a, "agents_derived_list", json!({})).unwrap();
        assert!(listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["state"] == "invalid"));
    }
}
