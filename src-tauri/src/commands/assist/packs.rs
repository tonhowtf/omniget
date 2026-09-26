//! Commands for selective pack import (`assist::packs`): list a pack's
//! catalog (metadata only), plan, apply with explicit choices, roll back,
//! and turn an imported agent into a bot with no tools granted.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use omniget_core::core::assist::db;
use omniget_core::core::assist::packs::{self, ImportPlan, ItemKind, SourceMeta};
use omniget_core::core::skills;

use crate::AppState;

fn to_value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn skills_root() -> Result<PathBuf, String> {
    skills::install::skills_dir().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn assist_packs_catalog(dir: String) -> Result<Value, String> {
    let p = PathBuf::from(&dir);
    let cat = packs::catalog(&p)?;
    Ok(json!({
        "items": cat,
        "license": packs::detect_license(&p),
        "sha": packs::detect_sha(&p),
        "origin": packs::detect_origin(&p).unwrap_or(dir),
    }))
}

#[derive(Debug, Deserialize)]
pub struct PlanInput {
    pub dir: String,
    pub selection: Vec<String>,
    #[serde(default)]
    pub all_of: Vec<ItemKind>,
}

#[tauri::command]
pub async fn assist_packs_plan(input: PlanInput) -> Result<ImportPlan, String> {
    let p = PathBuf::from(&input.dir);
    let meta = SourceMeta {
        origin: packs::detect_origin(&p).unwrap_or_else(|| input.dir.clone()),
        sha: packs::detect_sha(&p),
        license: packs::detect_license(&p),
        attribution: None,
    };
    packs::plan(
        &*db::global()?,
        &p,
        &skills_root()?,
        &input.selection,
        &input.all_of,
        meta,
    )
}

#[tauri::command]
pub async fn assist_packs_apply(
    plan: ImportPlan,
    overwrite: Vec<String>,
) -> Result<packs::ApplyReport, String> {
    let scan = |d: &std::path::Path| skills::scan::scan_dir(d);
    packs::apply(&*db::global()?, &plan, &skills_root()?, &overwrite, &scan)
}

#[tauri::command]
pub async fn assist_packs_items() -> Result<Vec<packs::PackItem>, String> {
    let mut items = packs::items(&*db::global()?)?;
    for i in &mut items {
        // The list never carries bodies; open one on demand.
        i.content = None;
    }
    Ok(items)
}

#[tauri::command]
pub async fn assist_packs_text(kind: ItemKind, name: String) -> Result<Option<String>, String> {
    packs::text_of(&*db::global()?, kind, &name)
}

#[tauri::command]
pub async fn assist_packs_history(item_id: String) -> Result<Vec<Value>, String> {
    packs::history(&*db::global()?, &item_id)
}

#[tauri::command]
pub async fn assist_packs_rollback(item_id: String) -> Result<Value, String> {
    let mut it = packs::rollback(&*db::global()?, &item_id, &skills_root()?)?;
    it.content = None;
    to_value(it)
}

/// An imported agent becomes a bot: its text is the bot's instructions; the
/// tools it asked for are listed, not granted.
#[tauri::command]
pub async fn assist_packs_agent_to_bot(
    state: State<'_, AppState>,
    item_id: String,
    connection_agent_id: String,
) -> Result<Value, String> {
    let db = db::global()?;
    let it = packs::item_by_id(&db, &item_id)?;
    if it.kind != ItemKind::Agent {
        return Err(format!(
            "{}: {} is not an agent",
            packs::ERR_PACK_INPUT,
            it.name
        ));
    }
    let body = it.content.clone().unwrap_or_default();
    let body = match body
        .strip_prefix("---")
        .and_then(|r| r.find("\n---").map(|e| r[e + 4..].to_string()))
    {
        Some(b) => b,
        None => body,
    };
    let instructions = format!(
        "{}\n\n(Imported from {} — {}. Tools it asked for, not granted: {})",
        body.trim(),
        it.origin,
        it.license
            .clone()
            .unwrap_or_else(|| "license unknown".into()),
        if it.capabilities.is_empty() {
            "none".into()
        } else {
            it.capabilities.join(", ")
        }
    );
    crate::commands::assist::bots::assist_bot_create(
        state,
        serde_json::from_value(json!({ "name": it.name, "purpose": "Imported agent", "instructions": instructions, "capabilities": [], "connection_agent_id": connection_agent_id })).map_err(|e| e.to_string())?,
    )
    .await
}
