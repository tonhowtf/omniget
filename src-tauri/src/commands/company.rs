//! Window commands of the companies of agents. One entry point for the whole
//! domain (`company_rpc`, same methods as the bridge, the CLI and the MCP),
//! plus what only the running engine knows.

use omniget_core::core::company::{self as co, rpc, Actor};
use serde_json::{json, Value};

#[tauri::command]
pub async fn company_rpc(method: String, args: Option<Value>) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| json!({}));
    tauri::async_runtime::spawn_blocking(move || {
        let st = co::store()?;
        let out = rpc::dispatch(&st, &method, &args, &Actor::user(), true);
        if out.is_ok() && !rpc::is_read_only(&method) {
            if let Some(e) = crate::company::get() {
                e.poke();
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn company_run_stop(run_id: String) -> Result<(), String> {
    match crate::company::get() {
        Some(e) => e.stop_run(&run_id),
        None => Err(format!("{}: the engine is not running", co::ERR_COMPANY)),
    }
}

#[tauri::command]
pub async fn company_engine_status() -> Result<Value, String> {
    Ok(match crate::company::get() {
        Some(e) => {
            json!({"running": true, "live_runs": e.live_count(), "max": crate::company::GLOBAL_MAX})
        }
        None => json!({"running": false}),
    })
}

/// Roster agents a hire can copy (runtime and model), for the hire form.
#[tauri::command]
pub async fn company_runtime_templates(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri::Manager;
    let llm = app.state::<crate::AppState>().llm.clone();
    Ok(json!(llm
        .roster()
        .into_iter()
        .filter(|a| !a.id.starts_with("co-"))
        .map(|a| json!({"id": a.id, "name": a.name, "runtime": a.runtime, "model": llm.model_label(&a.id)}))
        .collect::<Vec<_>>()))
}

/// Hires with a fresh roster agent copied from `template_agent_id` (its
/// runtime and model), or with an existing roster agent (`staff.agent_id`).
#[tauri::command]
pub async fn company_hire(
    company: String,
    staff: Value,
    template_agent_id: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = co::store()?;
        let mut new: co::org::NewStaff = serde_json::from_value(staff)
            .map_err(|e| format!("{}: {e}", co::ERR_COMPANY_INVALID))?;
        if new.agent_id.trim().is_empty() || template_agent_id.is_some() {
            let c = st.company_get(&company)?;
            let slug: String = new
                .name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(20)
                .collect();
            new.agent_id = format!(
                "co-{}-{}-{}",
                c.issue_prefix.to_lowercase(),
                slug,
                &co::new_id()[..4]
            );
            co::toolset::provision(&co::toolset::ProvisionRequest {
                agent_id: new.agent_id.clone(),
                name: new.name.clone(),
                role: new.role.clone(),
                instructions: new.instructions.clone(),
                template_agent_id,
                agent_def: None,
            })?;
        }
        let s = st.staff_hire(&company, new, &Actor::user())?;
        if let Some(e) = crate::company::get() {
            e.poke();
        }
        serde_json::to_value(s).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
