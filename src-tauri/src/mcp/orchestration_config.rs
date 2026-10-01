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
