//! Commands for `assist::reading`: the reading panel of a bot and the
//! `/llm/reading` page. Every command acts as the person at the screen
//! (`AssistCtx::user()`); the rules (round validation, access status) are the
//! same backend functions the bot's tools use.

use omniget_core::core::assist::ctx::AssistCtx;
use omniget_core::core::assist::db;
use omniget_core::core::assist::reading::{
    self, rules, skill, JourneyPatch, NewJourney, PrefsPatch,
};
use serde_json::{json, Value};

fn user() -> AssistCtx {
    AssistCtx::user()
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// Journeys of a bot (each in detail), its access preferences and whether
/// the curation skill is installed.
#[tauri::command]
pub async fn assist_reading_overview(bot_id: String) -> Result<Value, String> {
    let db = db::global()?;
    let ctx = user();
    let now = rules::now();
    let journeys = reading::list_journeys(&db, &ctx, Some(&bot_id))?;
    let mut details = Vec::new();
    for j in &journeys {
        details.push(reading::journey_detail(&db, &ctx, &j.id, now)?);
    }
    Ok(json!({
        "journeys": details,
        "prefs": reading::get_prefs(&db, &ctx, &bot_id)?,
        "skill": skill::status().ok(),
        // Whether this bot already has the curation skill linked, so the
        // panel does not keep asking to link it.
        "skill_bound": omniget_core::core::assist::bots::skills::bindings(&db, &bot_id)
            .map(|b| b.iter().any(|x| x.skill == skill::SKILL_NAME))
            .unwrap_or(false),
    }))
}

/// Every journey of every bot, newest first (the `/llm/reading` page).
#[tauri::command]
pub async fn assist_reading_list_all() -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::list_journeys(&db, &user(), None)?)
}

#[tauri::command]
pub async fn assist_reading_journey(journey_id: String) -> Result<Value, String> {
    let db = db::global()?;
    reading::journey_detail(&db, &user(), &journey_id, rules::now())
}

#[tauri::command]
pub async fn assist_reading_start_journey(
    bot_id: String,
    journey: NewJourney,
) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::create_journey(&db, &user(), &bot_id, journey)?)
}

#[tauri::command]
pub async fn assist_reading_update_journey(
    journey_id: String,
    patch: JourneyPatch,
) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::update_journey(&db, &user(), &journey_id, patch)?)
}

#[tauri::command]
pub async fn assist_reading_delete_journey(journey_id: String) -> Result<(), String> {
    let db = db::global()?;
    reading::delete_journey(&db, &user(), &journey_id)
}

/// `origin` is `manual` (typed by the person) or `reader` (read from the
/// Study plugin on the person's click).
#[tauri::command]
pub async fn assist_reading_record_progress(
    journey_id: String,
    kind: String,
    value: String,
    edition: Option<String>,
    origin: Option<String>,
    note: Option<String>,
) -> Result<Value, String> {
    let db = db::global()?;
    let origin = match origin.as_deref() {
        Some("reader") => "reader",
        _ => "manual",
    };
    let out = reading::record_progress(
        &db,
        &user(),
        &journey_id,
        &kind,
        &value,
        edition,
        origin,
        note,
    )?;
    Ok(json!({
        "progress": out.progress,
        "not_comparable": out.not_comparable,
        "direction": out.compared_with.map(|(_, o)| format!("{o:?}").to_lowercase()),
    }))
}

#[tauri::command]
pub async fn assist_reading_note_context(
    journey_id: String,
    text: String,
    hours: Option<i64>,
) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::note_context(
        &db,
        &user(),
        &journey_id,
        &text,
        hours,
    )?)
}

/// planned / watched / abandoned / declined, with an optional short reaction.
#[tauri::command]
pub async fn assist_reading_record_viewing(
    journey_id: String,
    movie_id: String,
    kind: String,
    reaction: Option<String>,
) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::record_viewing(
        &db,
        &user(),
        &journey_id,
        &movie_id,
        &kind,
        reaction,
        Vec::new(),
        "user",
    )?)
}

#[tauri::command]
pub async fn assist_reading_forget_reaction(event_id: String) -> Result<(), String> {
    let db = db::global()?;
    reading::forget_reaction(&db, &user(), &event_id)
}

#[tauri::command]
pub async fn assist_reading_get_prefs(bot_id: String) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::get_prefs(&db, &user(), &bot_id)?)
}

#[tauri::command]
pub async fn assist_reading_set_prefs(bot_id: String, patch: PrefsPatch) -> Result<Value, String> {
    let db = db::global()?;
    to_value(reading::set_prefs(&db, &user(), &bot_id, patch)?)
}

/// Current access status of one film, recomputed from the evidence now.
#[tauri::command]
pub async fn assist_reading_assess(bot_id: String, movie_id: String) -> Result<Value, String> {
    let db = db::global()?;
    let prefs = reading::get_prefs(&db, &user(), &bot_id)?;
    to_value(reading::assess(&db, &movie_id, &prefs, rules::now())?)
}

#[tauri::command]
pub async fn assist_reading_skill_status() -> Result<Value, String> {
    to_value(skill::status()?)
}

/// Installs the embedded curation skill through the app's skill installer
/// (staging + scan). A copy that differs is replaced only with `replace`.
/// When the scan parks it, the result carries `outcome.needs_confirm` for
/// `llm_skills_confirm_install`.
#[tauri::command]
pub async fn assist_reading_install_skill(replace: Option<bool>) -> Result<Value, String> {
    let out = tokio::task::spawn_blocking(move || skill::install(replace.unwrap_or(false)))
        .await
        .map_err(|e| e.to_string())??;
    let pending = out
        .get("outcome")
        .and_then(|o| o.get("needs_confirm"))
        .map(|v| !v.is_null())
        .unwrap_or(false);
    if !pending && out.get("already_installed") == Some(&Value::Bool(false)) {
        let projection = omniget_core::core::assist::bots::skills::projection();
        if let Err(e) = omniget_core::core::assist::bots::skills::skills_changed(
            &projection,
            Some(skill::SKILL_NAME),
        ) {
            tracing::warn!("[reading] re-projecting skills after install: {e}");
        }
    }
    Ok(out)
}
