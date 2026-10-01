use omniget_core::core::company::{self as co, rpc};
use serde_json::{json, Value};

use super::policy::Principal;
use super::ToolDef;

pub fn tools() -> Vec<ToolDef> {
    let schema = json!({"type": "object", "additionalProperties": false, "required": ["method"], "properties": {
        "method": {"type": "string", "description": "area.verb, e.g. company.list, issue.list, issue.create, issue.comment, staff.wake"},
        "args": {"type": "object", "description": "Arguments of the method, e.g. {\"company\": \"<id>\", \"issue\": {\"title\": \"…\"}}"}
    }});
    vec![
        ToolDef {
            name: "company_query",
            description: "Read the companies of agents in OmniGet: company.list, company.dashboard, staff.chart, issue.list, issue.get, issue.search, approval.list, budget.status, cost.summary, run.list, routine.list, audit.list, inbox.list.",
            input_schema: schema.clone(),
        },
        ToolDef {
            name: "company_command",
            description: "Act in a company of agents: issue.create, issue.update, issue.status, issue.comment, issue.doc_put, issue.relate, staff.wake, staff.hire, goal.create, project.create, routine.create, approval.request. Approvals, budgets and firing are decided by the owner in the OmniGet window.",
            input_schema: schema,
        },
    ]
}

pub async fn call(p: &Principal, name: &str, a: Value) -> Result<Value, String> {
    let method = a
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let args = a.get("args").cloned().unwrap_or_else(|| json!({}));
    let write = name == "company_command";
    if !write && !rpc::is_read_only(&method) {
        return Err("TOOL_NOT_AUTHORIZED: use company_command for writes".into());
    }
    let channel = format!("mcp:{}", p.name);
    let echo = method.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = co::store()?;
        let out = rpc::dispatch_external(&st, &method, &args, &channel, write);
        if out.is_ok() && write {
            if let Some(e) = crate::company::get() {
                e.poke();
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|result| json!({"method": echo, "result": result}))
}
